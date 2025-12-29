use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// Dollar Cost Averaging - buys at regular intervals
pub struct DCAStrategy {
    trade_count: u64,
    last_spread: f64,
    buy_interval: u64,  // Buy every N trades
}

impl DCAStrategy {
    pub fn new() -> Self {
        Self {
            trade_count: 0,
            last_spread: 0.0,
            buy_interval: 50,  // Buy every 50 trades for more activity
        }
    }
}

#[async_trait]
impl TradingStrategy for DCAStrategy {
    fn name(&self) -> &str { "DCAStrategy" }
    
    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Interval".to_string(), self.buy_interval.to_string()),
            ("Total Trades".to_string(), self.trade_count.to_string()),
            ("Next Buy In".to_string(), (self.buy_interval - (self.trade_count % self.buy_interval)).to_string()),
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

impl DCAStrategy {
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
        
        // Simple DCA: buy at regular intervals
        if current_state == SystemState::Trading && self.trade_count % self.buy_interval == 0 {
            opps.push(Opportunity {
                id: format!("dca_buy_{}", self.trade_count),
                signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: 0.0001 },
                score: 0.6,
                risk_score: 0.2,
                reason: format!("DCA interval #{}", self.trade_count / self.buy_interval),
                timestamp: ts,
            });
        }
        
        {
            let mut guard = state.write().await;
            let action = opps.first().map(|_| "Buy".to_string());
            let strat_lat = guard.metrics.get_strategy_stats().p50;
            let exec_lat = guard.metrics.get_execution_stats().p50;
            guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, self.last_spread, ts);
        }
        
        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opps
    }
}
