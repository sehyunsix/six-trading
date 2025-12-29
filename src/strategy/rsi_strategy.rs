use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// RSI-based trading strategy
pub struct RSIStrategy {
    prices: Vec<f64>,
    trade_count: u64,
    last_spread: f64,
    rsi_period: usize,
}

impl RSIStrategy {
    pub fn new() -> Self {
        Self {
            prices: Vec::with_capacity(100),
            trade_count: 0,
            last_spread: 0.0,
            rsi_period: 14,
        }
    }
    
    fn calculate_rsi(&self) -> Option<f64> {
        if self.prices.len() < self.rsi_period + 1 {
            return None;
        }
        
        let changes: Vec<f64> = self.prices.windows(2)
            .map(|w| w[1] - w[0])
            .collect();
        
        let recent = &changes[changes.len().saturating_sub(self.rsi_period)..];
        
        let gains: f64 = recent.iter().filter(|&&x| x > 0.0).sum();
        let losses: f64 = recent.iter().filter(|&&x| x < 0.0).map(|x| x.abs()).sum();
        
        if losses == 0.0 {
            return Some(100.0);
        }
        
        let rs = gains / losses;
        Some(100.0 - (100.0 / (1.0 + rs)))
    }
}

#[async_trait]
impl TradingStrategy for RSIStrategy {
    fn name(&self) -> &str { "RSIStrategy" }
    
    fn get_features(&self) -> Vec<(String, String)> {
        let rsi = self.calculate_rsi().unwrap_or(50.0);
        vec![
            ("RSI".to_string(), format!("{:.1}", rsi)),
            ("Spread".to_string(), format!("{:.4}", self.last_spread)),
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
    
    async fn process_orderbook(&mut self, ob: OrderBook, state: SharedState) -> Vec<Opportunity> {
        if !ob.bids.is_empty() && !ob.asks.is_empty() {
            self.last_spread = ob.asks[0].price - ob.bids[0].price;
        }
        Vec::new()
    }
}

impl RSIStrategy {
    async fn handle_trade(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        self.prices.push(price);
        if self.prices.len() > 50 { self.prices.remove(0); }
        
        // State transitions
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
            if let Some(rsi) = self.calculate_rsi() {
                // Oversold - Buy
                if rsi < 30.0 {
                    opps.push(Opportunity {
                        id: format!("rsi_buy_{}", self.trade_count),
                        signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.8,
                        risk_score: 0.3,
                        reason: format!("RSI={:.1} (oversold)", rsi),
                        timestamp: ts,
                    });
                }
                // Overbought - Sell
                if rsi > 70.0 {
                    opps.push(Opportunity {
                        id: format!("rsi_sell_{}", self.trade_count),
                        signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.8,
                        risk_score: 0.3,
                        reason: format!("RSI={:.1} (overbought)", rsi),
                        timestamp: ts,
                    });
                }
            }
        }
        
        // Record history
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
