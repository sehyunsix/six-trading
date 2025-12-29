use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// Volatility breakout strategy
pub struct VolatilityBreakout {
    prices: Vec<f64>,
    trade_count: u64,
    last_spread: f64,
    in_position: bool,
    entry_price: f64,
}

impl VolatilityBreakout {
    pub fn new() -> Self {
        Self {
            prices: Vec::with_capacity(50),
            trade_count: 0,
            last_spread: 0.0,
            in_position: false,
            entry_price: 0.0,
        }
    }
    
    fn get_range(&self) -> (f64, f64) {
        if self.prices.len() < 10 {
            return (0.0, 0.0);
        }
        let recent = &self.prices[self.prices.len().saturating_sub(10)..];
        let high = recent.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let low = recent.iter().cloned().fold(f64::INFINITY, f64::min);
        (high, low)
    }
}

#[async_trait]
impl TradingStrategy for VolatilityBreakout {
    fn name(&self) -> &str { "VolatilityBreakout" }
    
    fn get_features(&self) -> Vec<(String, String)> {
        let (high, low) = self.get_range();
        vec![
            ("Range High".to_string(), format!("{:.2}", high)),
            ("Range Low".to_string(), format!("{:.2}", low)),
            ("Volatility".to_string(), format!("{:.2}", high - low)),
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

impl VolatilityBreakout {
    async fn handle_trade(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        self.prices.push(price);
        if self.prices.len() > 30 { self.prices.remove(0); }
        
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
            let (high, low) = self.get_range();
            let range = high - low;
            
            if range > 0.0 {
                // Breakout above range
                if !self.in_position && price > high + range * 0.01 {
                    self.in_position = true;
                    self.entry_price = price;
                    opps.push(Opportunity {
                        id: format!("vb_buy_{}", self.trade_count),
                        signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.7,
                        risk_score: 0.4,
                        reason: format!("Breakout above {:.2} (+1% range)", high),
                        timestamp: ts,
                    });
                }
                
                // Take profit or stop loss
                if self.in_position {
                    let pnl_pct = (price - self.entry_price) / self.entry_price * 100.0;
                    if pnl_pct > 0.2 || pnl_pct < -0.1 {
                        self.in_position = false;
                        opps.push(Opportunity {
                            id: format!("vb_sell_{}", self.trade_count),
                            signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                            score: 0.7,
                            risk_score: 0.3,
                            reason: format!("Exit: PnL={:.2}%", pnl_pct),
                            timestamp: ts,
                        });
                    }
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
