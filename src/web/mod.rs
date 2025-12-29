use axum::{
    extract::{State, Query},
    routing::get,
    Json, Router,
    response::sse::{Event, KeepAlive, Sse},
};
use serde::{Serialize, Deserialize};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, broadcast};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use sqlx::{Pool, Postgres};
use std::convert::Infallible;
use crate::market_data::downloader::MarketType;

use crate::state_machine::{StateMachine, SystemState};
use crate::metrics::{SystemMetrics, LatencyStats};
use crate::database::repository;
use crate::strategy::{TradingStrategy, Signal};
#[allow(unused_imports)]
use crate::strategy::TradingStrategy as _;
use crate::execution::Executor;

// Global broadcast channel for SSE progress events
lazy_static::lazy_static! {
    pub static ref PROGRESS_TX: broadcast::Sender<ProgressEvent> = {
        let (tx, _) = broadcast::channel(100);
        tx
    };
}

#[derive(Serialize, Clone, Debug)]
pub struct ProgressEvent {
    pub symbol: String,
    pub strategy_name: String,
    pub progress_pct: u32,
    pub status: String,  // "running", "completed", "error"
    pub features: std::collections::HashMap<String, String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DataPoint {
    pub timestamp: u64,
    pub price: f64,
    pub volume: f64,
    pub state: SystemState,
    pub action: Option<String>,
    pub strategy_latency: u64, // microseconds
    pub execution_latency: u64, // microseconds
    pub spread: f64,
    pub equity: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct PortfolioSnapshot {
    pub timestamp: u64,
    pub total_value_usd: f64,
}

// Shared Application State
pub struct AppState {
    pub state_machine: StateMachine,
    pub metrics: SystemMetrics,
    pub history: VecDeque<DataPoint>,
    pub max_history: usize,
    pub run_mode: String,
    pub strategy_name: String,
    pub db_pool: Pool<Postgres>,
    pub symbol: String,
    pub available_markets: Vec<String>,
    pub current_opportunities: Vec<crate::strategy::Opportunity>,
    pub selected_opportunity_id: Option<String>,
    pub total_trades: u64,
    pub win_trades: u64,
    pub loss_trades: u64,
    pub realized_pnl: f64,
    pub last_update_ts: u64,
    pub risk_report: crate::strategy::RiskReport,
    pub executor: Arc<dyn crate::execution::Executor>,
    pub portfolio_history: VecDeque<PortfolioSnapshot>,
    pub last_portfolio_snapshot_ts: u64,
    pub is_trading: bool,
    pub initial_balance: f64,
    pub available_strategies: Vec<String>,
    pub data_quality_score: f64,
    pub sample_rate: usize,  // Only record 1 in N data points
    pub data_point_counter: usize,
    pub market_sender: mpsc::Sender<crate::market_data::MarketEvent>,
    pub current_features: std::collections::HashMap<String, String>,
}

impl AppState {
    pub fn new(
        run_mode: String, 
        strategy_name: String, 
        db_pool: Pool<Postgres>, 
        symbol: String,
        executor: Arc<dyn crate::execution::Executor>,
        market_sender: mpsc::Sender<crate::market_data::MarketEvent>
    ) -> Self {
        let available_markets = vec!["BTCUSDT".to_string(), "ETHUSDT".to_string(), "BNBUSDT".to_string(), "SOLUSDT".to_string()];
        Self {
            state_machine: StateMachine::new(),
            metrics: SystemMetrics::new(),
            history: VecDeque::with_capacity(1000),
            max_history: 1000,
            run_mode,
            strategy_name,
            db_pool,
            symbol,
            available_markets,
            current_opportunities: Vec::new(),
            selected_opportunity_id: None,
            total_trades: 0,
            win_trades: 0,
            loss_trades: 0,
            realized_pnl: 0.0,
            last_update_ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            risk_report: crate::strategy::RiskReport {
                total_risk: 0.0,
                leverage_risk: 0.0,
                drawdown_warning: false,
                recommended_max_size: 0.1,
            },
            executor,
            portfolio_history: VecDeque::with_capacity(500),
            last_portfolio_snapshot_ts: 0,
            is_trading: false,
            initial_balance: 10000.0, // Default for simulation, will be updated from balance
            available_strategies: crate::strategy::StrategyFactory::get_available_strategies(),
            data_quality_score: 100.0,
            sample_rate: 1,  // Default: record every data point
            data_point_counter: 0,
            market_sender,
            current_features: std::collections::HashMap::new(),
        }
    }
    
    /// Add a portfolio value snapshot
    pub fn push_portfolio_snapshot(&mut self, total_value_usd: f64) {
        let snapshot = PortfolioSnapshot {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            total_value_usd,
        };
        self.portfolio_history.push_back(snapshot);
        if self.portfolio_history.len() > 500 {
            self.portfolio_history.pop_front();
        }
    }

    pub fn push_data_point(
        &mut self, 
        price: f64, 
        volume: f64, 
        action: Option<String>,
        strat_lat: u64,
        exec_lat: u64,
        spread: f64
    ) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.push_data_point_at(price, volume, action, strat_lat, exec_lat, spread, ts * 1000);
    }

    pub fn push_data_point_at(
        &mut self, 
        price: f64, 
        volume: f64, 
        action: Option<String>,
        strat_lat: u64,
        exec_lat: u64,
        spread: f64,
        ts_ms: u64
    ) {
        let equity = self.initial_balance + self.realized_pnl;
        let dp = DataPoint {
            timestamp: ts_ms, // Use full millisecond precision
            price,
            volume,
            state: self.state_machine.get_state(),
            action,
            strategy_latency: strat_lat,
            execution_latency: exec_lat,
            spread,
            equity,
        };
        
        // Only record data point if we're at a sampling interval
        self.data_point_counter += 1;
        if self.sample_rate <= 1 || self.data_point_counter % self.sample_rate == 0 {
            self.history.push_back(dp);
            if self.history.len() > self.max_history {
                self.history.pop_front();
            }
        }
    }

    pub fn clear_all_data(&mut self) {
        self.history.clear();
        self.total_trades = 0;
        self.win_trades = 0;
        self.loss_trades = 0;
        self.realized_pnl = 0.0;
        self.state_machine.transition_to(SystemState::Booting);
    }
}

pub type SharedState = Arc<RwLock<AppState>>;


#[derive(Serialize)]
struct StatusResponse {
    state: SystemState,
    strategy_metrics: LatencyStats,
    execution_metrics: LatencyStats,
    run_mode: String,
    strategy_name: String,
    features: std::collections::HashMap<String, String>,
    transition_probabilities: Vec<Vec<f64>>,
    inferred_probabilities: Vec<Vec<f64>>,
    wallet: WalletInfo,
    positions: Vec<crate::execution::PositionInfo>,
    symbol: String,
    available_markets: Vec<String>,
    opportunities: Vec<crate::strategy::Opportunity>,
    selected_opportunity_id: Option<String>,
    total_trades: u64,
    win_trades: u64,
    loss_trades: u64,
    win_rate: f64,
    realized_pnl: f64,
    last_update_ts: u64,
    risk_report: crate::strategy::RiskReport,
    portfolio_history: Vec<PortfolioSnapshot>,
    trade_stats: crate::execution::TradeStats,
    is_trading: bool,
    yield_pct: f64,
    available_strategies: Vec<String>,
    data_quality_score: f64,
}

#[derive(Deserialize)]
struct BacktestRequest {
    strategies: Vec<String>,
    symbols: Vec<String>, // Format: "SPOT:BTCUSDT" or "FUTURES:BTCUSDT"
    start_ts: Option<u64>,
    end_ts: Option<u64>,
    #[serde(default)]
    fast_mode: bool,
}

#[derive(Serialize)]
struct StrategyReport {
    symbol: String,
    strategy_name: String,
    history: Vec<DataPoint>,
    features: std::collections::HashMap<String, String>,
    total_trades: u64,
    win_rate: f64,
    yield_pct: f64,
    realized_pnl: f64,
    max_drawdown: f64,
    profit_factor: f64,
    avg_win: f64,
    avg_loss: f64,
    sharpe_ratio: f64,
    total_fees: f64,
}

#[derive(Serialize)]
struct BacktestReport {
    reports: Vec<StrategyReport>,
    initial_capital: f64,
}

#[derive(Serialize, Default, Clone)]
struct WalletInfo {
    usdt: f64,
    btc: f64,
    all_balances: Vec<CoinBalance>,
}

#[derive(Serialize, Clone)]
struct CoinBalance {
    coin: String,
    amount: f64,
}

async fn get_status(State(state): State<SharedState>) -> Json<StatusResponse> {
    let read_guard = state.read().await;
    let strategy_stats = read_guard.metrics.get_strategy_stats();
    let execution_stats = read_guard.metrics.get_execution_stats();
    
    // Fetch real-time wallet data
    let balances = read_guard.executor.get_balances().await.unwrap_or_default();
    let usdt = balances.iter().find(|(k, _)| k == "USDT").map(|(_, v)| *v).unwrap_or(0.0);
    let btc = balances.iter().find(|(k, _)| k == "BTC").map(|(_, v)| *v).unwrap_or(0.0);
    
    // Convert all balances to CoinBalance struct
    let all_balances: Vec<CoinBalance> = balances.iter()
        .map(|(coin, amount)| CoinBalance { coin: coin.clone(), amount: *amount })
        .collect();
    
    let wallet = WalletInfo { usdt, btc, all_balances };
    
    // Fetch real-time positions
    let positions = read_guard.executor.get_positions().await.unwrap_or_default();
    
    // Fetch trade statistics from Binance
    let trade_stats = read_guard.executor.get_trade_stats(&read_guard.symbol).await.unwrap_or_default();

    // Calculate win rate
    let win_rate = if read_guard.total_trades > 0 {
        (read_guard.win_trades as f64 / read_guard.total_trades as f64) * 100.0
    } else {
        0.0
    };

    // Calculate yield
    let total_value = if !read_guard.portfolio_history.is_empty() {
        read_guard.portfolio_history.back().unwrap().total_value_usd
    } else {
        read_guard.initial_balance
    };
    let yield_pct = ((total_value - read_guard.initial_balance) / read_guard.initial_balance) * 100.0;

    Json(StatusResponse {
        state: read_guard.state_machine.get_state(),
        strategy_metrics: strategy_stats,
        execution_metrics: execution_stats,
        run_mode: read_guard.run_mode.clone(),
        strategy_name: read_guard.strategy_name.clone(),
        features: read_guard.current_features.clone(),
        transition_probabilities: read_guard.state_machine.get_transition_probabilities(),
        inferred_probabilities: read_guard.state_machine.get_inferred_probabilities(),
        wallet,
        positions,
        symbol: read_guard.symbol.clone(),
        available_markets: read_guard.available_markets.clone(),
        opportunities: read_guard.current_opportunities.clone(),
        selected_opportunity_id: read_guard.selected_opportunity_id.clone(),
        total_trades: read_guard.total_trades,
        win_trades: read_guard.win_trades,
        loss_trades: read_guard.loss_trades,
        win_rate,
        realized_pnl: read_guard.realized_pnl,
        last_update_ts: read_guard.last_update_ts,
        risk_report: read_guard.risk_report.clone(),
        portfolio_history: read_guard.portfolio_history.iter().cloned().collect(),
        trade_stats,
        is_trading: read_guard.is_trading,
        yield_pct,
        available_strategies: read_guard.available_strategies.clone(),
        data_quality_score: read_guard.data_quality_score,
    })
}

#[derive(Deserialize)]
struct HistoryQuery {
    interval: Option<String>, // "1m", "1h" or None for raw
}

async fn get_history(
    State(state): State<SharedState>,
    Query(query): Query<HistoryQuery>
) -> Json<Vec<DataPoint>> {
    let read_guard = state.read().await;
    
    if let Some(interval) = query.interval {
        let db_interval = match interval.as_str() {
            "1h" => "hour",
            _ => "minute",
        };
        
        match repository::get_aggregated_trades(&read_guard.db_pool, &read_guard.symbol, "SPOT", db_interval).await {
            Ok(agg_data) => {
                log::info!("Fetched {} aggregated data points for interval: {}", agg_data.len(), interval);
                let dps = agg_data.into_iter().map(|d| DataPoint {
                    timestamp: d.timestamp as u64,
                    price: d.price,
                    volume: d.volume,
                    state: SystemState::Trading, // Simplified for aggregated view
                    action: None,
                    strategy_latency: 0,
                    execution_latency: 0,
                    spread: 0.0,
                    equity: 0.0,
                }).collect();
                return Json(dps);
            }
            Err(e) => {
                log::error!("Aggregated history fetch failed: {}", e);
            }
        }
    }
    
    Json(read_guard.history.iter().cloned().collect())
}

// Simple embedded HTML dashboard
async fn get_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("dashboard.html"))
}

async fn get_backtest_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("backtest.html"))
}

#[derive(Deserialize)]
struct ChangeSymbolQuery {
    symbol: String,
}

#[derive(Deserialize)]
struct SelectStrategyQuery {
    strategy: String,
}

async fn select_strategy(
    State(state): State<SharedState>,
    Json(payload): Json<SelectStrategyQuery>
) -> Json<serde_json::Value> {
    let mut write_guard = state.write().await;
    write_guard.strategy_name = payload.strategy.clone();
    log::info!("Strategy selection changed to: {}", payload.strategy);
    Json(serde_json::json!({ "status": "success", "strategy": payload.strategy }))
}

async fn change_symbol(
    State(state): State<SharedState>,
    Json(payload): Json<ChangeSymbolQuery>
) -> Json<serde_json::Value> {
    let mut write_guard = state.write().await;
    write_guard.symbol = payload.symbol.clone();
    log::info!("Symbol changed to: {}", payload.symbol);
    Json(serde_json::json!({ "status": "success", "symbol": payload.symbol }))
}

async fn get_data_range_api(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>
) -> Json<serde_json::Value> {
    let (pool, symbol) = {
        let read_guard = state.read().await;
        let s = params.get("symbol").cloned().unwrap_or_else(|| read_guard.symbol.clone());
        (read_guard.db_pool.clone(), s)
    };
    let market_type = params.get("market_type").map(|s| s.as_str()).unwrap_or("SPOT");

    match repository::get_data_range(&pool, &symbol, market_type).await {
        Ok((min, max)) => Json(serde_json::json!({
            "min": min,
            "max": max
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() }))
    }
}

#[derive(Deserialize)]
struct DownloadDataRequest {
    symbol: String,
    #[serde(default)]
    market_type: Option<String>,
    start_ts: u64,
    end_ts: u64,
}

async fn download_data_api(
    State(state): State<SharedState>,
    Json(payload): Json<DownloadDataRequest>
) -> Json<serde_json::Value> {
    let db_pool = {
        let read_guard = state.read().await;
        read_guard.db_pool.clone()
    };
    
    log::info!("Manual data download requested: {} ({:?}) from {} to {}", payload.symbol, payload.market_type, payload.start_ts, payload.end_ts);
    
    let downloader = crate::market_data::HistoricalDownloader::new(db_pool);
    
    let market_type = match payload.market_type.as_deref() {
        Some("FUTURES") | Some("futures") => crate::market_data::downloader::MarketType::Futures,
        _ => crate::market_data::downloader::MarketType::Spot,
    };
    
    match downloader.fetch_and_save_range_public(&payload.symbol, market_type, payload.start_ts, payload.end_ts).await {
        Ok(_) => {
            let duration_hours = (payload.end_ts - payload.start_ts) as f64 / 3600000.0;
            Json(serde_json::json!({
                "success": true,
                "message": format!("Downloaded {:.1} hours of data for {}", duration_hours, payload.symbol)
            }))
        }
        Err(e) => {
            log::error!("Download failed: {}", e);
            Json(serde_json::json!({
                "success": false,
                "error": e.to_string()
            }))
        }
    }
}

// SSE endpoint for real-time backtest progress
async fn sse_progress_handler() -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = PROGRESS_TX.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|result| {
            match result {
                Ok(event) => {
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    Some(Ok(Event::default().data(json)))
                }
                Err(_) => None,
            }
        });
    
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn execute_isolated_backtest(
    State(state): State<SharedState>,
    Json(payload): Json<BacktestRequest>
) -> Json<BacktestReport> {
    let db_pool = {
        let read_guard = state.read().await;
        read_guard.db_pool.clone()
    };

    log::info!("Executing COMBINATORIAL backtest for symbols {:?} with strategies: {:?}", payload.symbols, payload.strategies);

    let start_ts = payload.start_ts.unwrap_or(0);
    let end_ts = payload.end_ts.unwrap_or(u64::MAX);

    let mut symbol_handles = Vec::new();
    let symbols = payload.symbols.clone();
    let strategies = payload.strategies.clone();
    let fast_mode = payload.fast_mode;

    for symbol_spec in symbols {
        let db_pool_inner = db_pool.clone();
        let strategies_inner = strategies.clone();
        
        let handle = tokio::spawn(async move {
            let parts: Vec<&str> = symbol_spec.split(':').collect();
            if parts.len() != 2 {
                log::error!("Invalid symbol format: {}", symbol_spec);
                return Vec::new();
            }
            
            let market_type = match parts[0].to_uppercase().as_str() {
                "FUTURES" => crate::market_data::downloader::MarketType::Futures,
                _ => crate::market_data::downloader::MarketType::Spot,
            };
            let symbol = parts[1].to_string();

            // 0. Download missing data for this symbol
            {
                let downloader = crate::market_data::HistoricalDownloader::new(db_pool_inner.clone());
                if let Err(e) = downloader.ensure_data_range(&symbol, market_type, start_ts, end_ts).await {
                    log::error!("Failed to download historical data for {}: {}", symbol, e);
                }
            }

            // 1. Load historical trades for this symbol
            let trades = repository::get_historical_trades_range(
                &db_pool_inner, 
                &symbol,
                market_type.as_str(),
                Some(start_ts), 
                Some(end_ts)
            ).await.unwrap_or_default();
            
            if trades.is_empty() {
                log::warn!("No trades found for {} ({}) in requested range", symbol, market_type.as_str());
                return Vec::new();
            }

            log::info!("Loaded {} trades for backtesting {}", trades.len(), symbol);
            let trades_arc = std::sync::Arc::new(trades);
            let mut strat_handles = Vec::new();

            for strat_name in strategies_inner {
                let trades_clone = trades_arc.clone();
                let db_pool_clone = db_pool_inner.clone();
                let symbol_clone = symbol.clone();
                let strat_name_clone = strat_name.clone();

                let strat_handle = tokio::spawn(async move {
                    log::info!("[{} | {}] Starting backtest...", symbol_clone, strat_name_clone);
                    
                    let executor = std::sync::Arc::new(crate::execution::ExecutionManager::new(true));
                    let (dummy_tx, _) = mpsc::channel(1);
                    let backtest_state = std::sync::Arc::new(RwLock::new(AppState::new(
                        "backtest".to_string(),
                        strat_name_clone.clone(),
                        db_pool_clone,
                        symbol_clone.clone(),
                        executor.clone(),
                        dummy_tx
                    )));

                    {
                        let mut write_guard = backtest_state.write().await;
                        write_guard.clear_all_data();
                        write_guard.max_history = 10_000;
                        write_guard.state_machine.transition_to(crate::state_machine::SystemState::Trading);
                        write_guard.is_trading = true;
                    }

                    let mut strategy = match crate::strategy::StrategyFactory::create_strategy(&strat_name_clone) {
                        Some(s) => s,
                        None => return None,
                    };

                    let mut trade_pnls = Vec::new();
                    let mut peak_pnl = 0.0;
                    let mut max_drawdown = 0.0;
                    let mut gross_profit = 0.0;
                    let mut gross_loss = 0.0;
                    let mut total_fees = 0.0;

                    let total_trades_count = trades_clone.len();
                    let progress_interval = (total_trades_count / 10).max(1);
                    let sample_rate = (total_trades_count / 2000).max(1);
                    let fast_skip = if fast_mode { 10 } else { 1 };
                    
                    {
                        let mut write_guard = backtest_state.write().await;
                        write_guard.sample_rate = sample_rate;
                    }

                    for (idx, trade) in trades_clone.iter().enumerate() {
                        if fast_mode && idx % fast_skip != 0 {
                            continue;
                        }
                        
                        let current_features: std::collections::HashMap<String, String> = strategy.get_features().into_iter().collect();

                        if idx > 0 && idx % progress_interval == 0 {
                            let progress_pct = (idx as f64 / total_trades_count as f64 * 100.0) as u32;
                            let _ = PROGRESS_TX.send(ProgressEvent {
                                symbol: symbol_clone.clone(),
                                strategy_name: strat_name_clone.clone(),
                                progress_pct,
                                status: "running".to_string(),
                                features: current_features.clone(),
                            });
                        }

                        let opps = strategy.process_trade(trade.clone(), backtest_state.clone()).await;
                        
                        {
                            let mut write_guard = backtest_state.write().await;
                            write_guard.current_features = current_features;
                        }
                        
                        for opp in opps {
                            let price = trade.price.parse::<f64>().unwrap_or(0.0);
                            let fee = match &opp.signal {
                                Signal::Buy { quantity, .. } => price * quantity * 0.001,
                                Signal::Sell { quantity, .. } => price * quantity * 0.001,
                                _ => 0.0,
                            };
                            total_fees += fee;

                            let pnl = executor.execute(opp.signal).await.unwrap_or(0.0);
                            
                            {
                                let mut write_guard = backtest_state.write().await;
                                write_guard.total_trades += 1;
                                write_guard.realized_pnl += pnl - fee;
                                
                                if pnl > 0.0 {
                                    write_guard.win_trades += 1;
                                    trade_pnls.push(pnl - fee);
                                    gross_profit += pnl;
                                } else if pnl < 0.0 {
                                    write_guard.loss_trades += 1;
                                    trade_pnls.push(pnl - fee);
                                    gross_loss += pnl.abs();
                                }
                            }

                            let current_total_pnl = backtest_state.read().await.realized_pnl;
                            if current_total_pnl > peak_pnl { peak_pnl = current_total_pnl; }
                            let drawdown = peak_pnl - current_total_pnl;
                            if drawdown > max_drawdown { max_drawdown = drawdown; }
                        }
                    }
                    
                    let final_features: std::collections::HashMap<String, String> = strategy.get_features().into_iter().collect();
                    let _ = PROGRESS_TX.send(ProgressEvent {
                        symbol: symbol_clone.clone(),
                        strategy_name: strat_name_clone.clone(),
                        progress_pct: 100,
                        status: "completed".to_string(),
                        features: final_features.clone(),
                    });

                    let report_guard = backtest_state.read().await;
                    let win_rate = if report_guard.total_trades > 0 {
                        (report_guard.win_trades as f64 / report_guard.total_trades as f64) * 100.0
                    } else { 0.0 };
                    
                    let yield_pct = (report_guard.realized_pnl / report_guard.initial_balance) * 100.0;
                    let profit_factor = if gross_loss > 0.0 { gross_profit / gross_loss } else { 0.0 };
                    let avg_win = if report_guard.win_trades > 0 { gross_profit / report_guard.win_trades as f64 } else { 0.0 };
                    let avg_loss = if report_guard.loss_trades > 0 { gross_loss / report_guard.loss_trades as f64 } else { 0.0 };

                    let sharpe_ratio = if !trade_pnls.is_empty() {
                        let mean = trade_pnls.iter().sum::<f64>() / trade_pnls.len() as f64;
                        let variance = trade_pnls.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / trade_pnls.len() as f64;
                        if variance > 0.0 { mean / variance.sqrt() } else { 0.0 }
                    } else { 0.0 };

                    Some(StrategyReport {
                        symbol: symbol_clone,
                        strategy_name: strat_name_clone,
                        history: report_guard.history.iter().cloned().collect(),
                        features: final_features,
                        total_trades: report_guard.total_trades,
                        win_rate,
                        yield_pct,
                        realized_pnl: report_guard.realized_pnl,
                        max_drawdown,
                        profit_factor,
                        avg_win,
                        avg_loss,
                        sharpe_ratio,
                        total_fees,
                    })
                });
                strat_handles.push(strat_handle);
            }

            let strat_results = futures::future::join_all(strat_handles).await;
            strat_results.into_iter().filter_map(|r| r.ok().flatten()).collect::<Vec<StrategyReport>>()
        });
        symbol_handles.push(handle);
    }

    let symbol_results = futures::future::join_all(symbol_handles).await;
    let strategy_reports: Vec<StrategyReport> = symbol_results
        .into_iter()
        .filter_map(|r| r.ok())
        .flatten()
        .collect();

    log::info!("Combinatorial backtest completed with {} results", strategy_reports.len());

    Json(BacktestReport {
        reports: strategy_reports,
        initial_capital: 10000.0,
    })
}

async fn start_trading(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let mut write_guard = state.write().await;
    write_guard.is_trading = true;
    log::info!("Trading STARTED by user request");
    Json(serde_json::json!({ "status": "success", "is_trading": true }))
}

async fn stop_trading(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let mut write_guard = state.write().await;
    write_guard.is_trading = false;
    log::info!("Trading STOPPED by user request");
    Json(serde_json::json!({ "status": "success", "is_trading": false }))
}

pub async fn start_server(port: u16, state: SharedState) {
    let app = Router::new()
        .route("/api/status", get(get_status))
        .route("/api/history", get(get_history))
        .route("/api/data_range", get(get_data_range_api))
        .route("/api/change_symbol", axum::routing::post(change_symbol))
        .route("/api/select_strategy", axum::routing::post(select_strategy))
        .route("/api/backtest/progress", get(sse_progress_handler))
        .route("/api/backtest/execute", axum::routing::post(execute_isolated_backtest))
        .route("/api/download_data", axum::routing::post(download_data_api))
        .route("/api/start_trading", axum::routing::post(start_trading))
        .route("/api/stop_trading", axum::routing::post(stop_trading))
        .route("/", get(get_dashboard))
        .route("/backtest", get(get_backtest_dashboard))
        .with_state(state);

    
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    log::info!("Web Dashboard running at http://localhost:{}", port);
    
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
