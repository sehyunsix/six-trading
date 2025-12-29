//! Binance Worker - Actor Pattern Implementation
//! 
//! This module solves the runtime conflict between tokio and binance-rs by
//! running all blocking Binance API calls in a dedicated OS thread.
//! 
//! Architecture:
//! ```
//! [Async Code] ---(Command Channel)---> [Worker Thread] ---(Binance API)---> [Binance Server]
//!      ^                                       |
//!      |_______(Response Channel)______________|
//! ```
//! 
//! The worker thread is completely isolated from the tokio runtime,
//! eliminating the "Cannot drop a runtime" panic.

use binance::account::Account;
use binance::api::Binance;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use log::{info, error, warn};

/// Commands that can be sent to the Binance worker
#[derive(Debug)]
pub enum BinanceCommand {
    MarketBuy { 
        symbol: String, 
        quantity: f64,
        response_tx: tokio::sync::oneshot::Sender<BinanceResponse>,
    },
    MarketSell { 
        symbol: String, 
        quantity: f64,
        response_tx: tokio::sync::oneshot::Sender<BinanceResponse>,
    },
    CancelOrder {
        symbol: String,
        order_id: u64,
        response_tx: tokio::sync::oneshot::Sender<BinanceResponse>,
    },
    GetAccount {
        response_tx: tokio::sync::oneshot::Sender<BinanceResponse>,
    },
    GetTradeHistory {
        symbol: String,
        limit: u16,
        response_tx: tokio::sync::oneshot::Sender<BinanceResponse>,
    },
    Shutdown,
}

/// Individual trade info
#[derive(Debug, Clone)]
pub struct TradeInfo {
    pub id: u64,
    pub price: f64,
    pub qty: f64,
    pub commission: f64,
    pub commission_asset: String,
    pub is_buyer: bool,
    pub time: u64,
}

/// Responses from the Binance worker
#[derive(Debug, Clone)]
pub enum BinanceResponse {
    OrderSuccess { order_id: u64, symbol: String, qty: f64 },
    OrderFailed { error: String },
    AccountInfo { balances: Vec<(String, f64)> },
    TradeHistory { trades: Vec<TradeInfo> },
    Cancelled,
    Failed { error: String },
}

/// The Binance Worker - runs in an isolated thread
pub struct BinanceWorker {
    command_tx: mpsc::Sender<BinanceCommand>,
    _handle: JoinHandle<()>,
}

impl BinanceWorker {
    /// Creates a new Binance worker with the given API credentials.
    /// Spawns a dedicated thread that will handle all API calls.
    pub fn new(api_key: String, secret_key: String) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<BinanceCommand>();
        
        // Spawn the worker thread - completely isolated from tokio
        let handle = thread::Builder::new()
            .name("binance-api-worker".to_string())
            .spawn(move || {
                info!("Binance Worker thread started");
                
                // Create the Binance account client INSIDE this thread
                let account = Account::new(Some(api_key), Some(secret_key));
                
                // Process commands until shutdown
                loop {
                    match command_rx.recv() {
                        Ok(BinanceCommand::Shutdown) => {
                            info!("Binance Worker shutting down");
                            break;
                        }
                        Ok(BinanceCommand::MarketBuy { symbol, quantity, response_tx }) => {
                            info!("Worker: Executing MARKET BUY {} x {}", quantity, symbol);
                            let response = match account.market_buy(&symbol, quantity) {
                                Ok(answer) => {
                                    info!("Order {} placed successfully", answer.order_id);
                                    BinanceResponse::OrderSuccess { 
                                        order_id: answer.order_id, 
                                        symbol: symbol.clone(),
                                        qty: quantity,
                                    }
                                }
                                Err(e) => {
                                    error!("Buy order failed: {:?}", e);
                                    BinanceResponse::OrderFailed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(BinanceCommand::MarketSell { symbol, quantity, response_tx }) => {
                            info!("Worker: Executing MARKET SELL {} x {}", quantity, symbol);
                            let response = match account.market_sell(&symbol, quantity) {
                                Ok(answer) => {
                                    info!("Order {} placed successfully", answer.order_id);
                                    BinanceResponse::OrderSuccess { 
                                        order_id: answer.order_id, 
                                        symbol: symbol.clone(),
                                        qty: quantity,
                                    }
                                }
                                Err(e) => {
                                    error!("Sell order failed: {:?}", e);
                                    BinanceResponse::OrderFailed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(BinanceCommand::CancelOrder { symbol, order_id, response_tx }) => {
                            info!("Worker: Cancelling order {} for {}", order_id, symbol);
                            let response = match account.cancel_order(&symbol, order_id) {
                                Ok(_) => {
                                    info!("Order cancelled successfully");
                                    BinanceResponse::Cancelled
                                }
                                Err(e) => {
                                    error!("Cancel failed: {:?}", e);
                                    BinanceResponse::Failed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(BinanceCommand::GetAccount { response_tx }) => {
                            let response = match account.get_account() {
                                Ok(info) => {
                                    let balances: Vec<(String, f64)> = info.balances
                                        .iter()
                                        .filter(|b| {
                                            let free = b.free.parse::<f64>().unwrap_or(0.0);
                                            let locked = b.locked.parse::<f64>().unwrap_or(0.0);
                                            free > 0.0 || locked > 0.0
                                        })
                                        .map(|b| {
                                            let free = b.free.parse::<f64>().unwrap_or(0.0);
                                            let locked = b.locked.parse::<f64>().unwrap_or(0.0);
                                            (b.asset.clone(), free + locked)
                                        })
                                        .collect();
                                    BinanceResponse::AccountInfo { balances }
                                }
                                Err(e) => {
                                    warn!("Failed to fetch account: {:?}", e);
                                    BinanceResponse::Failed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(BinanceCommand::GetTradeHistory { symbol, limit, response_tx }) => {
                            info!("Worker: Fetching trade history for {}", symbol);
                            let response = match account.trade_history(&symbol) {
                                Ok(trades) => {
                                    let trade_infos: Vec<TradeInfo> = trades
                                        .iter()
                                        .take(limit as usize)
                                        .map(|t| TradeInfo {
                                            id: t.id,
                                            price: t.price,
                                            qty: t.qty,
                                            commission: t.commission.parse::<f64>().unwrap_or(0.0),
                                            commission_asset: t.commission_asset.clone(),
                                            is_buyer: t.is_buyer,
                                            time: t.time,
                                        })
                                        .collect();
                                    info!("Retrieved {} trades", trade_infos.len());
                                    BinanceResponse::TradeHistory { trades: trade_infos }
                                }
                                Err(e) => {
                                    warn!("Failed to fetch trade history: {:?}", e);
                                    BinanceResponse::Failed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Err(_) => {
                            // Channel closed, exit the loop
                            info!("Binance Worker: command channel closed");
                            break;
                        }
                    }
                }
                
                info!("Binance Worker thread exited cleanly");
            })
            .expect("Failed to spawn Binance worker thread");
        
        Self {
            command_tx,
            _handle: handle,
        }
    }
    
    /// Places a market buy order asynchronously
    pub async fn market_buy(&self, symbol: String, quantity: f64) -> Result<u64, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(BinanceCommand::MarketBuy { symbol, quantity, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(BinanceResponse::OrderSuccess { order_id, .. }) => Ok(order_id),
            Ok(BinanceResponse::OrderFailed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Places a market sell order asynchronously
    pub async fn market_sell(&self, symbol: String, quantity: f64) -> Result<u64, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(BinanceCommand::MarketSell { symbol, quantity, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(BinanceResponse::OrderSuccess { order_id, .. }) => Ok(order_id),
            Ok(BinanceResponse::OrderFailed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Cancels an order asynchronously
    pub async fn cancel_order(&self, symbol: String, order_id: u64) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(BinanceCommand::CancelOrder { symbol, order_id, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(BinanceResponse::Cancelled) => Ok(()),
            Ok(BinanceResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Gets account information asynchronously
    pub async fn get_account(&self) -> Result<Vec<(String, f64)>, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(BinanceCommand::GetAccount { response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(BinanceResponse::AccountInfo { balances }) => Ok(balances),
            Ok(BinanceResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Gets trade history asynchronously
    pub async fn get_trade_history(&self, symbol: String, limit: u16) -> Result<Vec<TradeInfo>, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(BinanceCommand::GetTradeHistory { symbol, limit, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(BinanceResponse::TradeHistory { trades }) => Ok(trades),
            Ok(BinanceResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
}

// Allow BinanceWorker to be shared across threads
unsafe impl Send for BinanceWorker {}
unsafe impl Sync for BinanceWorker {}
