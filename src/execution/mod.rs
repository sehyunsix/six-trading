//! Execution Module - Clean separation of concerns
//! 
//! This module handles trade execution with multiple modes:
//! 1. Simulation Mode: In-memory tracking for testing
//! 2. Live Spot Mode: Uses BinanceWorker for Spot API calls
//! 3. Live Futures Mode: Uses FuturesWorker for Futures API calls

mod binance_worker;
mod futures_worker;

use binance_worker::BinanceWorker;
// Re-exports for other modules

use serde::{Serialize, Deserialize};
use crate::strategy::Signal;
use log::{info, warn, error};
use async_trait::async_trait;
use std::env;
use std::sync::Arc;
#[derive(Serialize, Clone, Debug, Deserialize, Default)]
pub struct TradeStats {
    pub total_trades: u64,
    pub buy_trades: u64,
    pub sell_trades: u64,
    pub total_volume: f64,
    pub total_commission: f64,
    pub commission_asset: String,
}

#[derive(Serialize, Clone, Debug, Deserialize)]
pub struct PositionInfo {
    pub symbol: String,
    pub amount: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
    pub market_type: String,
    pub side: String,
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn execute(&self, signal: Signal) -> Result<f64, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_balances(&self) -> Result<Vec<(String, f64)>, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_positions(&self) -> Result<Vec<PositionInfo>, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_trade_stats(&self, symbol: &str) -> Result<TradeStats, Box<dyn std::error::Error + Send + Sync>>;
}

pub struct ExecutionManager {
    worker: Option<Arc<BinanceWorker>>,
    is_simulation: bool,
    // In-memory tracking for simulation mode
    sim_balances: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, f64>>>,
    sim_positions: std::sync::Arc<tokio::sync::Mutex<Vec<PositionInfo>>>,
}

impl ExecutionManager {
    pub fn new(is_simulation: bool) -> Self {
        let api_key = env::var("BINANCE_API_KEY").ok();
        let secret_key = env::var("BINANCE_API_SECRET").ok();

        let (worker, use_simulation) = if is_simulation {
            info!("Running in SIMULATION mode (backtest)");
            (None, true)
        } else if let (Some(key), Some(secret)) = (api_key, secret_key) {
            info!("Binance API credentials found. Initializing LIVE trading mode.");
            warn!("REAL MONEY will be used for trades!");
            
            // Create the isolated worker thread
            let worker = BinanceWorker::new(key, secret);
            (Some(Arc::new(worker)), false)
        } else {
            warn!("Binance API credentials NOT found. Using PAPER TRADING mode.");
            (None, true)
        };

        let mut balances = std::collections::HashMap::new();
        balances.insert("USDT".to_string(), 10000.0);
        balances.insert("BTC".to_string(), 0.0);

        Self {
            worker,
            is_simulation: use_simulation,
            sim_balances: std::sync::Arc::new(tokio::sync::Mutex::new(balances)),
            sim_positions: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
    
    /// Truncates quantity to Binance's required precision (5 decimal places for BTC)
    fn truncate_qty(qty: f64, decimals: u32) -> f64 {
        let factor = 10_f64.powi(decimals as i32);
        (qty * factor).floor() / factor
    }
}

#[async_trait]
impl Executor for ExecutionManager {
    async fn execute(&self, signal: Signal) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        if !self.is_simulation {
            info!("Executor.execute called.");
        }
        
        // === SIMULATION MODE ===
        if self.is_simulation {
            let mut realized_pnl = 0.0;
            match signal {
                Signal::Buy { symbol, price, quantity } => {
                    // info!("SIMULATION: Buying {} x {} @ {:?}", quantity, symbol, price);
                    let mut bal = self.sim_balances.lock().await;
                    let mut pos = self.sim_positions.lock().await;
                    
                    let usdt = bal.get_mut("USDT").unwrap();
                    let est_price = price.unwrap_or(0.0);
                    if est_price == 0.0 {
                        warn!("SIMULATION: Buy signal received with 0 or missing price. Skipping.");
                        return Ok(0.0);
                    }
                    let fee = quantity * est_price * 0.001;
                    let cost = quantity * est_price + fee;
                    
                    if *usdt >= cost {
                        *usdt -= cost;
                        *bal.entry("BTC".to_string()).or_insert(0.0) += quantity;
                        
                        if let Some(p) = pos.iter_mut().find(|p| p.symbol == symbol) {
                            let total_cost = p.amount * p.entry_price + cost;
                            p.amount += quantity;
                            p.entry_price = total_cost / p.amount;
                        } else {
                            pos.push(PositionInfo {
                                symbol: symbol.clone(),
                                amount: quantity,
                                entry_price: cost / quantity, // Entry price inclusive of fee
                                unrealized_pnl: 0.0,
                                market_type: "Spot".to_string(),
                                side: "Long".to_string(),
                            });
                        }
                    }
                }
                Signal::Sell { symbol, price, quantity } => {
                    // info!("SIMULATION: Selling {} x {} @ {:?}", quantity, symbol, price);
                    let mut bal = self.sim_balances.lock().await;
                    let mut pos = self.sim_positions.lock().await;
                    
                    let btc = bal.get_mut("BTC").unwrap();
                    if *btc >= quantity {
                        *btc -= quantity;
                        let est_price = price.unwrap_or(0.0);
                        if est_price == 0.0 {
                            warn!("SIMULATION: Sell signal received with 0 or missing price. Skipping.");
                            return Ok(0.0);
                        }
                        let revenue = quantity * est_price;
                        let fee = revenue * 0.001;
                        *bal.get_mut("USDT").unwrap() += revenue - fee;
                        
                        if let Some(idx) = pos.iter().position(|p| p.symbol == symbol) {
                            let buy_price = pos[idx].entry_price;
                            // Realized PnL = (Revenue - Fee) - (Buy Cost)
                            realized_pnl = (revenue - fee) - (buy_price * quantity);
                            
                            pos[idx].amount -= quantity;
                            if pos[idx].amount <= 0.000001 {
                                pos.remove(idx);
                            }
                        }
                    }
                }
                Signal::Cancel { .. } => {}
            }
            return Ok(realized_pnl);
        }

        // === LIVE MODE (Using Worker Thread) ===
        if let Some(worker) = &self.worker {
            // First, fetch current balances to check if we can afford the trade
            let balances = match worker.get_account().await {
                Ok(b) => b,
                Err(e) => {
                    error!("Failed to fetch balances: {}", e);
                    return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)));
                }
            };
            
            let usdt_balance = balances.iter().find(|(a, _)| a == "USDT").map(|(_, v)| *v).unwrap_or(0.0);
            let btc_balance = balances.iter().find(|(a, _)| a == "BTC").map(|(_, v)| *v).unwrap_or(0.0);
            
            info!("Current balances: USDT={:.2}, BTC={:.6}", usdt_balance, btc_balance);
            
            match signal {
                Signal::Buy { symbol, price, quantity } => {
                    // Check if we have enough USDT (estimate with current price)
                    let est_price = price.unwrap_or(90000.0);
                    let required_usdt = quantity * est_price * 1.001; // 0.1% buffer for fees
                    
                    if usdt_balance < required_usdt {
                        // Calculate max affordable quantity
                        let max_qty = Self::truncate_qty((usdt_balance * 0.995) / est_price, 5);
                        let order_value = max_qty * est_price;
                        
                        // Check minimum notional value ($5 for BTCUSDT)
                        if order_value < 5.0 {
                            warn!("Order value (${:.2}) below minimum notional ($5). Skipping buy.", order_value);
                            return Ok(0.0);
                        }
                        if max_qty < 0.00001 {
                            warn!("Insufficient USDT balance ({:.2}). Skipping buy.", usdt_balance);
                            return Ok(0.0);
                        }
                        info!("Adjusting quantity from {} to {:.5} based on available balance", quantity, max_qty);
                        info!("LIVE: Sending MARKET BUY {:.5} x {} to worker", max_qty, symbol);
                        match worker.market_buy(symbol, max_qty).await {
                            Ok(order_id) => info!("Order {} executed successfully!", order_id),
                            Err(e) => {
                                error!("Order failed: {}", e);
                                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)));
                            }
                        }
                    } else {
                        let qty = Self::truncate_qty(quantity, 5);
                        let order_value = qty * est_price;
                        
                        // Check minimum notional value ($5 for BTCUSDT)
                        if order_value < 5.0 {
                            warn!("Order value (${:.2}) below minimum notional ($5). Skipping buy.", order_value);
                            return Ok(0.0);
                        }
                        
                        info!("LIVE: Sending MARKET BUY {:.5} x {} to worker", qty, symbol);
                        match worker.market_buy(symbol, qty).await {
                            Ok(order_id) => info!("Order {} executed successfully!", order_id),
                            Err(e) => {
                                error!("Order failed: {}", e);
                                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)));
                            }
                        }
                    }
                }
                Signal::Sell { symbol, price, quantity } => {
                    let est_price = price.unwrap_or(90000.0);
                    
                    // Check if we have enough BTC
                    if btc_balance < quantity {
                        if btc_balance < 0.00001 {
                            warn!("Insufficient BTC balance ({:.6}). Skipping sell.", btc_balance);
                            return Ok(0.0);
                        }
                        let sell_qty = Self::truncate_qty(btc_balance, 5);
                        let order_value = sell_qty * est_price;
                        
                        // Check minimum notional value ($5 for BTCUSDT)
                        if order_value < 5.0 {
                            warn!("Order value (${:.2}) below minimum notional ($5). Skipping sell.", order_value);
                            return Ok(0.0);
                        }
                        
                        info!("Adjusting sell quantity from {} to {:.5} based on available balance", quantity, btc_balance);
                        info!("LIVE: Sending MARKET SELL {:.5} x {} to worker", sell_qty, symbol);
                        match worker.market_sell(symbol, sell_qty).await {
                            Ok(order_id) => info!("Order {} executed successfully!", order_id),
                            Err(e) => {
                                error!("Order failed: {}", e);
                                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)));
                            }
                        }
                    } else {
                        let sell_qty = Self::truncate_qty(quantity, 5);
                        let order_value = sell_qty * est_price;
                        
                        // Check minimum notional value ($5 for BTCUSDT)
                        if order_value < 5.0 {
                            warn!("Order value (${:.2}) below minimum notional ($5). Skipping sell.", order_value);
                            return Ok(0.0);
                        }
                        
                        info!("LIVE: Sending MARKET SELL {:.5} x {} to worker", sell_qty, symbol);
                        match worker.market_sell(symbol, sell_qty).await {
                            Ok(order_id) => info!("Order {} executed successfully!", order_id),
                            Err(e) => {
                                error!("Order failed: {}", e);
                                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)));
                            }
                        }
                    }
                }
                Signal::Cancel { symbol, order_id } => {
                    info!("LIVE: Cancelling order {} for {}", order_id, symbol);
                    if let Err(e) = worker.cancel_order(symbol, order_id).await {
                        error!("Cancel failed: {}", e);
                    }
                }
            }
        }

        Ok(0.0)
    }

    async fn get_balances(&self) -> Result<Vec<(String, f64)>, Box<dyn std::error::Error + Send + Sync>> {
        // Simulation mode - return simulated balances
        if self.is_simulation {
            let bal = self.sim_balances.lock().await;
            return Ok(bal.iter().map(|(k, v)| (k.clone(), *v)).collect());
        }
        
        // Live mode - fetch from Binance via worker
        if let Some(worker) = &self.worker {
            match worker.get_account().await {
                Ok(balances) => return Ok(balances),
                Err(e) => {
                    error!("Failed to get balances: {}", e);
                }
            }
        }
        
        Ok(vec![])
    }

    async fn get_positions(&self) -> Result<Vec<PositionInfo>, Box<dyn std::error::Error + Send + Sync>> {
        if self.is_simulation {
            return Ok(self.sim_positions.lock().await.clone());
        }
        // For Spot trading, positions are derived from balances
        // (Full implementation would track order history)
        Ok(vec![])
    }
    
    async fn get_trade_stats(&self, symbol: &str) -> Result<TradeStats, Box<dyn std::error::Error + Send + Sync>> {
        if self.is_simulation {
            // Return default stats for simulation mode
            return Ok(TradeStats::default());
        }
        
        // Live mode - fetch trade history from Binance
        if let Some(worker) = &self.worker {
            match worker.get_trade_history(symbol.to_string(), 100).await {
                Ok(trades) => {
                    let mut stats = TradeStats::default();
                    stats.total_trades = trades.len() as u64;
                    
                    // Get commission asset from first trade
                    if let Some(first_trade) = trades.first() {
                        stats.commission_asset = first_trade.commission_asset.clone();
                    }
                    
                    for trade in &trades {
                        if trade.is_buyer {
                            stats.buy_trades += 1;
                        } else {
                            stats.sell_trades += 1;
                        }
                        stats.total_volume += trade.price * trade.qty;
                        stats.total_commission += trade.commission;
                    }
                    
                    info!("Trade stats for {}: {} trades ({} buys, {} sells), Volume: ${:.2}, Commission: {:.6} {}",
                        symbol, stats.total_trades, stats.buy_trades, stats.sell_trades, 
                        stats.total_volume, stats.total_commission, stats.commission_asset);
                    
                    return Ok(stats);
                }
                Err(e) => {
                    error!("Failed to get trade history: {}", e);
                }
            }
        }
        
        Ok(TradeStats::default())
    }
}
