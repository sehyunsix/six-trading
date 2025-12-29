use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// Trend Following strategy using EMA crossover
pub struct TrendFollower {
    prices: Vec<f64>,
    trade_count: u64,
    last_spread: f64,
    in_position: bool,
}

impl TrendFollower {
    pub fn new() -> Self {
        Self {
            prices: Vec::with_capacity(100),
            trade_count: 0,
            last_spread: 0.0,
            in_position: false,
        }
    }
    
    fn ema(&self, period: usize) -> f64 {
        if self.prices.len() < period { return 0.0; }
        let k = 2.0 / (period as f64 + 1.0);
        let recent = &self.prices[self.prices.len().saturating_sub(period)..];
        let mut ema = recent[0];
        for &p in &recent[1..] {
            ema = p * k + ema * (1.0 - k);
        }
        ema
    }
}

#[async_trait]
impl TradingStrategy for TrendFollower {
    fn name(&self) -> &str { "TrendFollower" }
    
    fn get_features(&self) -> Vec<(String, String)> {
        let ema5 = self.ema(5);
        let ema12 = self.ema(12);
        vec![
            ("EMA5".to_string(), format!("{:.2}", ema5)),
            ("EMA12".to_string(), format!("{:.2}", ema12)),
            ("Position".to_string(), self.in_position.to_string()),
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

impl TrendFollower {
    async fn handle_trade(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        self.prices.push(price);
        if self.prices.len() > 50 { self.prices.remove(0); }
        
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
        
        if current_state == SystemState::Trading && self.prices.len() >= 20 {
            let ema_short = self.ema(5);
            let ema_long = self.ema(12);
            
            // Golden cross - buy
            if ema_short > ema_long * 1.001 && !self.in_position {
                self.in_position = true;
                opps.push(Opportunity {
                    id: format!("trend_buy_{}", self.trade_count),
                    signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.75,
                    risk_score: 0.35,
                    reason: format!("EMA5={:.2} > EMA12={:.2} (golden cross)", ema_short, ema_long),
                    timestamp: ts,
                });
            }
            
            // Death cross - sell
            if ema_short < ema_long * 0.999 && self.in_position {
                self.in_position = false;
                opps.push(Opportunity {
                    id: format!("trend_sell_{}", self.trade_count),
                    signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.75,
                    risk_score: 0.35,
                    reason: format!("EMA5={:.2} < EMA12={:.2} (death cross)", ema_short, ema_long),
                    timestamp: ts,
                });
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
