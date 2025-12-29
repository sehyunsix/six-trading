mod execution;
mod market_data;
mod strategy;
mod state_machine;
mod metrics;
mod web;
mod database;

use dotenv::dotenv;
use log::{info, error};
use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;

use execution::{ExecutionManager, Executor};
use market_data::{MarketDataManager, MarketEvent, backtest::BacktestDataManager, DataFilter};
use strategy::{PaperTrader, MeanReversionStrategy, TradingStrategy};
use web::{AppState, start_server};

fn main() {
    dotenv().ok();
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| {
            use std::io::Write;
            let ts = buf.timestamp();
            writeln!(
                buf,
                "[{} {} {}] {}",
                ts,
                record.level(),
                record.target(),
                record.args()
            )
        })
        .init();

    info!("Starting Binance Trading System (Phase 2)...");

    // 1. Initial configuration
    let run_mode = std::env::var("RUN_MODE").unwrap_or_else(|_| "live".to_string());
    let is_simulation = run_mode == "backtest";
    let symbol = "BTCUSDT".to_string();

    // 2. Initialize blocking components early (outside tokio)
    let execution_manager = ExecutionManager::new(is_simulation);
    let executor = Arc::new(execution_manager);

    // 3. Create the multi-thread Runtime and LEAK IT
    // This is the definitive fix for "Cannot drop a runtime" panics with binance-rs.
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let rt = Box::leak(Box::new(rt));
    let handle = rt.handle();

    // 4. Start everything in the runtime context
    handle.spawn(async move {
        info!("Initializing async components...");
        
        // ... (Database, Shared State, Channels, Web Server as before) ...
        let pool = database::establish_connection().await;
        if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
            error!("Database migration failed: {}", e);
        }

        let mut strategy: Box<dyn TradingStrategy> = Box::new(PaperTrader::new());
        let strategy_name = strategy.name().to_string();
        let (tx, mut rx) = mpsc::channel(100);
        let shared_state = Arc::new(RwLock::new(AppState::new(
            run_mode.clone(), 
            strategy_name, 
            pool.clone(),
            symbol.clone(),
            executor.clone(),
            tx.clone()
        )));

        let web_state = shared_state.clone();
        tokio::spawn(async move {
            start_server(3000, web_state).await;
        });

        // Ensure 6 hours of historical data for backtesting
        let downloader_pool = pool.clone();
        let downloader_symbol = symbol.clone();
        tokio::spawn(async move {
            let downloader = market_data::HistoricalDownloader::new(downloader_pool);
            if let Err(e) = downloader.ensure_data(&downloader_symbol, market_data::downloader::MarketType::Spot, 6).await {
                error!("Historical data download failed: {}", e);
            }
        });

        if is_simulation {
            info!("RUNNING IN BACKTEST MODE");
            let pool_clone = pool.clone();
            let symbol_clone = symbol.clone();
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let backtest = BacktestDataManager::new(symbol_clone, tx_clone, pool_clone);
                if let Err(e) = backtest.run_backtest().await {
                    error!("Backtest failed: {}", e);
                }
            });
        } else {
            info!("RUNNING IN LIVE MODE");
            let market_data = MarketDataManager::new(symbol.clone(), tx.clone());
            market_data.connect().await;
            Box::leak(Box::new(market_data));
        }

        // Background Cleanup Task (runs once an hour)
        let cleanup_pool = pool.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                match database::repository::cleanup_old_data(&cleanup_pool, 24).await {
                    Ok(affected) => info!("Cleaned up {} old records from database", affected),
                    Err(e) => error!("Database cleanup failed: {}", e),
                }
            }
        });

        info!("System core initialized. Processing events...");
        
        // Initialize initial balance for yield calculation
        {
            let balances = executor.get_balances().await.unwrap_or_default();
            let usdt = balances.iter().find(|(k, _)| k == "USDT").map(|(_, v)| *v).unwrap_or(0.0);
            let btc = balances.iter().find(|(k, _)| k == "BTC").map(|(_, v)| *v).unwrap_or(0.0);
            let btc_price = 88000.0; // Same estimate used in snapshots
            let starting_value = usdt + btc * btc_price;
            let mut write_guard = shared_state.write().await;
            write_guard.initial_balance = starting_value;
            info!("Initial portfolio value set to ${:.2} for yield tracking", starting_value);
        }

        let mut event_count = 0;
        let mut data_filter = DataFilter::new(0.05); // 5% outlier threshold

        // Main Processing Loop
        while let Some(event) = rx.recv().await {
            // Check for data quality
            if !data_filter.should_process(&event) {
                let mut write_guard = shared_state.write().await;
                write_guard.data_quality_score = data_filter.get_quality_score();
                continue;
            }
            
            // Periodically update data quality score even if no filtering happens
            if event_count % 100 == 0 {
                let mut write_guard = shared_state.write().await;
                write_guard.data_quality_score = data_filter.get_quality_score();
            }

            // Check for strategy change
            {
                let current_name = shared_state.read().await.strategy_name.clone();
                if strategy.name() != current_name {
                    info!("Swapping strategy from {} to {}", strategy.name(), current_name);
                    strategy = match current_name.as_str() {
                        "MeanReversion" => Box::new(MeanReversionStrategy::new()),
                        _ => Box::new(PaperTrader::new()),
                    };
                }
            }

            event_count += 1;
            if event_count % 100 == 0 {
                info!("Main Loop Heartbeat: Received {} events so far.", event_count);
            }

            let opportunities = match event {
                MarketEvent::Trade(ref trade) => {
                    let pool_clone = pool.clone();
                    let trade_clone = trade.clone();
                    tokio::spawn(async move {
                        let _ = database::repository::save_trade(&pool_clone, &trade_clone, "SPOT").await;
                    });
                    strategy.process_trade(trade.clone(), shared_state.clone()).await
                }
                MarketEvent::AggrTrade(ref agg) => {
                    let pool_clone = pool.clone();
                    let agg_clone = agg.clone();
                    tokio::spawn(async move {
                        let _ = database::repository::save_aggr_trade(&pool_clone, &agg_clone, "SPOT").await;
                    });
                    strategy.process_aggr_trade(agg.clone(), shared_state.clone()).await
                }
                MarketEvent::OrderBook(ref book) => {
                    let pool_clone = pool.clone();
                    let book_clone = book.clone();
                    let symbol_clone = symbol.clone();
                    tokio::spawn(async move {
                        let _ = database::repository::save_order_book(&pool_clone, &symbol_clone, &book_clone, "SPOT").await;
                    });
                    strategy.process_orderbook(book.clone(), shared_state.clone()).await
                }
                MarketEvent::DepthUpdate(_) => Vec::new(),
            };

            // Record portfolio value snapshot for chart (every 5 seconds)
            let now_ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let should_snapshot = {
                let read_guard = shared_state.read().await;
                now_ts - read_guard.last_portfolio_snapshot_ts >= 5
            };

            if should_snapshot {
                let (usdt, btc) = {
                    let write_guard = shared_state.write().await;
                    let balances = write_guard.executor.get_balances().await.unwrap_or_default();
                    let usdt = balances.iter().find(|(k, _)| k == "USDT").map(|(_, v)| *v).unwrap_or(0.0);
                    let btc = balances.iter().find(|(k, _)| k == "BTC").map(|(_, v)| *v).unwrap_or(0.0);
                    (usdt, btc)
                };
                
                // Estimate total value using approximate BTC price (will improve with market data)
                let btc_price = 88000.0;
                let total_value = usdt + btc * btc_price;
                
                let mut write_guard = shared_state.write().await;
                write_guard.push_portfolio_snapshot(total_value);
                write_guard.last_portfolio_snapshot_ts = now_ts;
            }

            // Check if trading is allowed before processing opportunities
            let is_trading = shared_state.read().await.is_trading;

            if !opportunities.is_empty() && is_trading {
                info!("Strategy generated {} opportunities", opportunities.len());
                let mut write_guard = shared_state.write().await;
                let (processed_opps, risk_report) = strategy::RiskManager::analyze_opportunities(&opportunities, &write_guard);
                
                write_guard.current_opportunities = processed_opps.clone();
                write_guard.risk_report = risk_report;
                write_guard.last_update_ts = now_ts;

                if let Some(ref sig) = strategy::RiskManager::select_best_trade(&processed_opps) {
                    // Find the ID of the selected trade
                    let selected_id = processed_opps.iter()
                        .find(|o| format!("{:?}", o.signal) == format!("{:?}", sig))
                        .map(|o| o.id.clone());
                    
                    info!("RiskManager selected trade: {:?}", selected_id);
                    write_guard.selected_opportunity_id = selected_id;
                    write_guard.total_trades += 1;

                    let executor_clone = executor.clone();
                    let shared_state_clone = shared_state.clone();
                    let sig_clone = sig.clone();
                    tokio::spawn(async move {
                        let start_exec = std::time::Instant::now();
                        match executor_clone.execute(sig_clone).await {
                            Ok(pnl) => {
                                let mut write_guard = shared_state_clone.write().await;
                                write_guard.realized_pnl += pnl;
                                write_guard.metrics.record_execution_latency(start_exec.elapsed());
                            }
                            Err(e) => error!("Execution error: {}", e),
                        }
                    });
                }
            }
        }
    });

    // 5. Keep the main thread alive indefinitely
    info!("Main thread parked. Press Ctrl+C to stop.");
    loop {
        std::thread::park();
    }
}
