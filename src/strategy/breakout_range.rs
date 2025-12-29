use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Breakout Range Strategy - Trades breakouts from consolidation ranges
pub struct BreakoutRangeStrategy {
    prices: VecDeque<f64>,
    range_high: f64,
    range_low: f64,
    consolidation_periods: usize,
    last_signal_time: u64,
}

impl BreakoutRangeStrategy {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(50),
            range_high: 0.0,
            range_low: f64::MAX,
            consolidation_periods: 0,
            last_signal_time: 0,
        }
    }

    fn update_range(&mut self) {
        if self.prices.len() < 20 { return; }
        let recent: Vec<f64> = self.prices.iter().rev().take(20).copied().collect();
        self.range_high = recent.iter().fold(0.0_f64, |a, &b| a.max(b));
        self.range_low = recent.iter().fold(f64::MAX, |a, &b| a.min(b));
        
        let range_pct = (self.range_high - self.range_low) / self.range_low * 100.0;
        if range_pct < 0.2 { self.consolidation_periods += 1; } else { self.consolidation_periods = 0; }
    }
}

#[async_trait]
impl TradingStrategy for BreakoutRangeStrategy {
    fn name(&self) -> &str { "BreakoutRange" }

    fn get_features(&self) -> Vec<(String, String)> {
        let range_pct = if self.range_low > 0.0 { (self.range_high - self.range_low) / self.range_low * 100.0 } else { 0.0 };
        vec![
            ("Range High".to_string(), format!("{:.2}", self.range_high)),
            ("Range Low".to_string(), format!("{:.2}", self.range_low)),
            ("Range %".to_string(), format!("{:.2}%", range_pct)),
            ("Consolidation".to_string(), self.consolidation_periods.to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 50 { self.prices.pop_front(); }
        self.update_range();
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && 
           self.consolidation_periods >= 3 && 
           trade.event_time - self.last_signal_time > 60000 {
            
            if price > self.range_high * 1.0001 {
                opps.push(Opportunity {
                    id: format!("breakout_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.85,
                    risk_score: 0.4,
                    reason: format!("Bullish breakout after {} periods consolidation", self.consolidation_periods),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
                self.consolidation_periods = 0;
            } else if price < self.range_low * 0.9999 {
                opps.push(Opportunity {
                    id: format!("breakout_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.45,
                    reason: format!("Bearish breakdown after {} periods consolidation", self.consolidation_periods),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
                self.consolidation_periods = 0;
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        self.prices.push_back(price);
        if self.prices.len() > 50 { self.prices.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
