//! Binance Futures Worker - Actor Pattern Implementation
//! 
//! This module handles Futures trading API calls in a dedicated thread,
//! similar to the Spot trading BinanceWorker.

use binance::futures::account::FuturesAccount;
use binance::api::Binance;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use log::{info, error, warn};

/// Margin type for positions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarginType {
    Cross,
    Isolated,
}

impl std::fmt::Display for MarginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarginType::Cross => write!(f, "CROSSED"),
            MarginType::Isolated => write!(f, "ISOLATED"),
        }
    }
}

/// Position side for Hedge mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionSide {
    Both,  // One-way mode
    Long,
    Short,
}

impl std::fmt::Display for PositionSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PositionSide::Both => write!(f, "BOTH"),
            PositionSide::Long => write!(f, "LONG"),
            PositionSide::Short => write!(f, "SHORT"),
        }
    }
}

/// Commands for Futures worker
#[derive(Debug)]
pub enum FuturesCommand {
    MarketBuy { 
        symbol: String, 
        quantity: f64,
        response_tx: tokio::sync::oneshot::Sender<FuturesResponse>,
    },
    MarketSell { 
        symbol: String, 
        quantity: f64,
        response_tx: tokio::sync::oneshot::Sender<FuturesResponse>,
    },
    SetLeverage {
        symbol: String,
        leverage: u8,
        response_tx: tokio::sync::oneshot::Sender<FuturesResponse>,
    },
    SetMarginType {
        symbol: String,
        margin_type: MarginType,
        response_tx: tokio::sync::oneshot::Sender<FuturesResponse>,
    },
    GetAccount {
        response_tx: tokio::sync::oneshot::Sender<FuturesResponse>,
    },
    GetPositions {
        response_tx: tokio::sync::oneshot::Sender<FuturesResponse>,
    },
    Shutdown,
}

/// Futures position info
#[derive(Debug, Clone)]
pub struct FuturesPosition {
    pub symbol: String,
    pub position_amt: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
    pub leverage: u32,
    pub margin_type: String,
    pub position_side: String,
}

/// Futures account balance
#[derive(Debug, Clone)]
pub struct FuturesBalance {
    pub asset: String,
    pub wallet_balance: f64,
    pub unrealized_pnl: f64,
    pub margin_balance: f64,
    pub available_balance: f64,
}

/// Responses from Futures worker
#[derive(Debug, Clone)]
pub enum FuturesResponse {
    OrderSuccess { order_id: u64, symbol: String, qty: f64 },
    OrderFailed { error: String },
    LeverageSet { symbol: String, leverage: u8 },
    MarginTypeSet { symbol: String, margin_type: String },
    AccountInfo { balances: Vec<FuturesBalance> },
    Positions { positions: Vec<FuturesPosition> },
    Failed { error: String },
}

/// The Futures Worker - runs in an isolated thread
pub struct FuturesWorker {
    command_tx: mpsc::Sender<FuturesCommand>,
    _handle: JoinHandle<()>,
}

impl FuturesWorker {
    /// Creates a new Futures worker with the given API credentials
    pub fn new(api_key: String, secret_key: String) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<FuturesCommand>();
        
        let handle = thread::Builder::new()
            .name("binance-futures-worker".to_string())
            .spawn(move || {
                info!("Binance Futures Worker thread started");
                
                // Create the Futures account client INSIDE this thread
                let account = FuturesAccount::new(Some(api_key), Some(secret_key));
                
                loop {
                    match command_rx.recv() {
                        Ok(FuturesCommand::Shutdown) => {
                            info!("Futures Worker shutting down");
                            break;
                        }
                        Ok(FuturesCommand::MarketBuy { symbol, quantity, response_tx }) => {
                            info!("Futures Worker: MARKET BUY {} x {}", quantity, symbol);
                            let response = match account.market_buy(&symbol, quantity) {
                                Ok(answer) => {
                                    info!("Futures Order {} placed", answer.order_id);
                                    FuturesResponse::OrderSuccess { 
                                        order_id: answer.order_id, 
                                        symbol: symbol.clone(),
                                        qty: quantity,
                                    }
                                }
                                Err(e) => {
                                    error!("Futures Buy failed: {:?}", e);
                                    FuturesResponse::OrderFailed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(FuturesCommand::MarketSell { symbol, quantity, response_tx }) => {
                            info!("Futures Worker: MARKET SELL {} x {}", quantity, symbol);
                            let response = match account.market_sell(&symbol, quantity) {
                                Ok(answer) => {
                                    info!("Futures Order {} placed", answer.order_id);
                                    FuturesResponse::OrderSuccess { 
                                        order_id: answer.order_id, 
                                        symbol: symbol.clone(),
                                        qty: quantity,
                                    }
                                }
                                Err(e) => {
                                    error!("Futures Sell failed: {:?}", e);
                                    FuturesResponse::OrderFailed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(FuturesCommand::SetLeverage { symbol, leverage, response_tx }) => {
                            info!("Futures Worker: Setting leverage {}x for {}", leverage, symbol);
                            let response = match account.change_initial_leverage(&symbol, leverage) {
                                Ok(_) => {
                                    info!("Leverage set to {}x", leverage);
                                    FuturesResponse::LeverageSet { symbol, leverage }
                                }
                                Err(e) => {
                                    error!("Set leverage failed: {:?}", e);
                                    FuturesResponse::Failed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(FuturesCommand::SetMarginType { symbol, margin_type, response_tx }) => {
                            info!("Futures Worker: Setting margin type {:?} for {}", margin_type, symbol);
                            let isolated = margin_type == MarginType::Isolated;
                            let margin_str = margin_type.to_string();
                            let response = match account.change_margin_type(&symbol, isolated) {
                                Ok(_) => {
                                    info!("Margin type set to {:?}", margin_type);
                                    FuturesResponse::MarginTypeSet { symbol, margin_type: margin_str }
                                }
                                Err(e) => {
                                    // Margin type change can fail if already set
                                    warn!("Margin type change: {:?}", e);
                                    FuturesResponse::MarginTypeSet { symbol, margin_type: margin_str }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(FuturesCommand::GetAccount { response_tx }) => {
                            let response = match account.account_balance() {
                                Ok(balances) => {
                                    let filtered: Vec<FuturesBalance> = balances
                                        .iter()
                                        .filter(|b| b.balance > 0.0)
                                        .map(|b| FuturesBalance {
                                            asset: b.asset.clone(),
                                            wallet_balance: b.balance,
                                            unrealized_pnl: b.cross_unrealized_pnl,
                                            margin_balance: b.cross_wallet_balance, // Estimation
                                            available_balance: b.balance,
                                        })
                                        .collect();
                                    FuturesResponse::AccountInfo { balances: filtered }
                                }
                                Err(e) => {
                                    warn!("Failed to get futures account: {:?}", e);
                                    FuturesResponse::Failed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Ok(FuturesCommand::GetPositions { response_tx }) => {
                            let response = match account.account_information() {
                                Ok(info) => {
                                    let positions: Vec<FuturesPosition> = info.positions
                                        .iter()
                                        .filter(|p| p.position_amount.abs() > 0.0)
                                        .map(|p| FuturesPosition {
                                            symbol: p.symbol.clone(),
                                            position_amt: p.position_amount,
                                            entry_price: p.entry_price,
                                            unrealized_pnl: p.unrealized_profit,
                                            leverage: p.leverage.parse().unwrap_or(1),
                                            margin_type: if p.isolated { "ISOLATED".to_string() } else { "CROSS".to_string() },
                                            position_side: p.position_side.clone(),
                                        })
                                        .collect();
                                    FuturesResponse::Positions { positions }
                                }
                                Err(e) => {
                                    warn!("Failed to get positions: {:?}", e);
                                    FuturesResponse::Failed { error: format!("{:?}", e) }
                                }
                            };
                            let _ = response_tx.send(response);
                        }
                        Err(_) => {
                            info!("Futures Worker: command channel closed");
                            break;
                        }
                    }
                }
                
                info!("Futures Worker thread exited cleanly");
            })
            .expect("Failed to spawn Futures worker thread");
        
        Self {
            command_tx,
            _handle: handle,
        }
    }
    
    /// Places a market buy order asynchronously
    pub async fn market_buy(&self, symbol: String, quantity: f64) -> Result<u64, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(FuturesCommand::MarketBuy { symbol, quantity, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(FuturesResponse::OrderSuccess { order_id, .. }) => Ok(order_id),
            Ok(FuturesResponse::OrderFailed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Places a market sell order asynchronously
    pub async fn market_sell(&self, symbol: String, quantity: f64) -> Result<u64, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(FuturesCommand::MarketSell { symbol, quantity, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(FuturesResponse::OrderSuccess { order_id, .. }) => Ok(order_id),
            Ok(FuturesResponse::OrderFailed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Sets leverage for a symbol
    pub async fn set_leverage(&self, symbol: String, leverage: u8) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(FuturesCommand::SetLeverage { symbol, leverage, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(FuturesResponse::LeverageSet { .. }) => Ok(()),
            Ok(FuturesResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Sets margin type for a symbol
    pub async fn set_margin_type(&self, symbol: String, margin_type: MarginType) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(FuturesCommand::SetMarginType { symbol, margin_type, response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(FuturesResponse::MarginTypeSet { .. }) => Ok(()),
            Ok(FuturesResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Gets Futures account balances
    pub async fn get_account(&self) -> Result<Vec<FuturesBalance>, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(FuturesCommand::GetAccount { response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(FuturesResponse::AccountInfo { balances }) => Ok(balances),
            Ok(FuturesResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
    
    /// Gets open Futures positions
    pub async fn get_positions(&self) -> Result<Vec<FuturesPosition>, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        self.command_tx
            .send(FuturesCommand::GetPositions { response_tx: tx })
            .map_err(|e| format!("Failed to send command: {}", e))?;
        
        match rx.await {
            Ok(FuturesResponse::Positions { positions }) => Ok(positions),
            Ok(FuturesResponse::Failed { error }) => Err(error),
            Ok(_) => Err("Unexpected response".to_string()),
            Err(e) => Err(format!("Response channel error: {}", e)),
        }
    }
}

// Allow FuturesWorker to be shared across threads
unsafe impl Send for FuturesWorker {}
unsafe impl Sync for FuturesWorker {}
