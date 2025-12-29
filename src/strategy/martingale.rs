use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// Martingale strategy - doubles down on losses (high risk)
pub struct MartingaleStrategy {
    trade_count: u64,
    last_spread: f64,
    last_trade_price: f64,
    position_size: f64,
    in_position: bool,
    consecutive_losses: u32,
}

impl MartingaleStrategy {
    pub fn new() -> Self {
        Self {
            trade_count: 0,
            last_spread: 0.0,
            last_trade_price: 0.0,
            position_size: 0.0001,
            in_position: false,
            consecutive_losses: 0,
        }
    }
}

#[async_trait]
impl TradingStrategy for MartingaleStrategy {
    fn name(&self) -> &str { "Martingale" }
    
    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Losses".to_string(), self.consecutive_losses.to_string()),
            ("Next Size".to_string(), format!("{:.4}", self.position_size * (2.0_f64).powi(self.consecutive_losses.min(5) as i32))),
            ("In Position".to_string(), self.in_position.to_string()),
        ]
    }
    
    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse().unwrap_or(0.0);
        let qty = trade.qty.parse().unwrap_or(0.0);
        self.handle_trade(trade.symbol, price, qty, trade.event_time, state).await
    }
    
    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse().unwrap_or(0.0);
        let qty = trade.qty.parse().unwrap_or(0.0);
        self.handle_trade(trade.symbol, price, qty, trade.event_time, state).await
    }
    
    async fn process_orderbook(&mut self, ob: OrderBook, _state: SharedState) -> Vec<Opportunity> {
        if !ob.bids.is_empty() && !ob.asks.is_empty() {
            self.last_spread = ob.asks[0].price - ob.bids[0].price;
        }
        Vec::new()
    }
}

impl MartingaleStrategy {
    async fn handle_trade(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        
        {
            let mut guard = state.write().await;
            if guard.state_machine.get_state() == SystemState::Booting {
                guard.state_machine.transition_to(SystemState::Accumulating);
            } else if guard.state_machine.get_state() == SystemState::Accumulating && guard.state_machine.is_stable() {
                guard.state_machine.transition_to(SystemState::Trading);
            }
        }
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading {
            // Enter position every 100 trades
            if !self.in_position && self.trade_count % 100 == 0 {
                self.in_position = true;
                self.last_trade_price = price;
                let size = self.position_size * (2.0_f64).powi(self.consecutive_losses.min(5) as i32);
                
                opps.push(Opportunity {
                    id: format!("mart_buy_{}", self.trade_count),
                    signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: size },
                    score: 0.6,
                    risk_score: 0.6,
                    reason: format!("Martingale entry (size={:.4}, losses={})", size, self.consecutive_losses),
                    timestamp: ts,
                });
            }
            
            // Exit on TP or SL
            if self.in_position {
                let pnl_pct = (price - self.last_trade_price) / self.last_trade_price * 100.0;
                
                if pnl_pct > 0.2 {  // Take profit
                    self.in_position = false;
                    self.consecutive_losses = 0;
                    self.position_size = 0.0001;  // Reset size
                    opps.push(Opportunity {
                        id: format!("mart_sell_tp_{}", self.trade_count),
                        signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.7,
                        risk_score: 0.2,
                        reason: format!("Take profit: {:.2}%", pnl_pct),
                        timestamp: ts,
                    });
                } else if pnl_pct < -0.2 {  // Stop loss - will double next position
                    self.in_position = false;
                    self.consecutive_losses += 1;
                    opps.push(Opportunity {
                        id: format!("mart_sell_sl_{}", self.trade_count),
                        signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.5,
                        risk_score: 0.5,
                        reason: format!("Stop loss: {:.2}%, next will double", pnl_pct),
                        timestamp: ts,
                    });
                }
            }
        }
        
        {
            let mut guard = state.write().await;
            let action = opps.first().map(|o| match &o.signal {
                Signal::Buy { .. } => "Buy".to_string(),
                Signal::Sell { .. } => "Sell".to_string(),
                _ => "Hold".to_string(),
            });
            let strat_lat = guard.metrics.get_strategy_stats().p50;
            let exec_lat = guard.metrics.get_execution_stats().p50;
            guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, self.last_spread, ts);
        }
        
        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opps
    }
}
