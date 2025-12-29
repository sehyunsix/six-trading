use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Fibonacci Reversion Strategy
pub struct FibonacciReversion {
    prices: VecDeque<f64>,
    period: usize,
    last_signal_time: u64,
}

impl FibonacciReversion {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(100),
            period: 50,
            last_signal_time: 0,
        }
    }

    fn calculate_levels(&self) -> Option<(f64, f64, Vec<f64>)> {
        if self.prices.len() < self.period { return None; }
        let recent = self.prices.iter().rev().take(self.period);
        let high = recent.clone().cloned().fold(f64::NEG_INFINITY, f64::max);
        let low = recent.cloned().fold(f64::INFINITY, f64::min);
        let range = high - low;
        
        let mut levels = Vec::new();
        for &ratio in &[0.236, 0.382, 0.5, 0.618, 0.786] {
            levels.push(high - range * ratio);
        }
        Some((high, low, levels))
    }
}

#[async_trait]
impl TradingStrategy for FibonacciReversion {
    fn name(&self) -> &str { "FibonacciReversion" }

    fn get_features(&self) -> Vec<(String, String)> {
        let levels = self.calculate_levels();
        vec![
            ("High".to_string(), levels.as_ref().map(|l| format!("{:.2}", l.0)).unwrap_or("0.0".to_string())),
            ("Low".to_string(), levels.as_ref().map(|l| format!("{:.2}", l.1)).unwrap_or("0.0".to_string())),
            ("Fib 0.618".to_string(), levels.as_ref().map(|l| format!("{:.2}", l.2[3])).unwrap_or("0.0".to_string())),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 100 { self.prices.pop_front(); }
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if let Some((_, low, levels)) = self.calculate_levels() {
            let fib_618 = levels[3];
            if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 120000 {
                // Buy near 61.8% retracement from bottom
                if (price - fib_618).abs() / price < 0.001 && price > low {
                    opps.push(Opportunity {
                        id: format!("fib_buy_{}", trade.event_time),
                        signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.8,
                        risk_score: 0.3,
                        reason: format!("Fib 0.618 Retracement support: {:.2}", fib_618),
                        timestamp: trade.event_time,
                    });
                     self.last_signal_time = trade.event_time;
                }
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        self.prices.push_back(price);
        if self.prices.len() > 100 { self.prices.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
