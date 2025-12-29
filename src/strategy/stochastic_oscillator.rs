use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Stochastic Oscillator Strategy
pub struct StochasticOscillator {
    prices: VecDeque<f64>,
    k_period: usize,
    d_period: usize,
    k_values: VecDeque<f64>,
    last_signal_time: u64,
}

impl StochasticOscillator {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(50),
            k_period: 14,
            d_period: 3,
            k_values: VecDeque::with_capacity(10),
            last_signal_time: 0,
        }
    }

    fn calculate_stochastic(&self) -> Option<(f64, f64)> {
        if self.prices.len() < self.k_period { return None; }
        
        let recent = self.prices.iter().rev().take(self.k_period);
        let high = recent.clone().cloned().fold(f64::NEG_INFINITY, f64::max);
        let low = recent.cloned().fold(f64::INFINITY, f64::min);
        let current = *self.prices.back()?;
        
        if high == low { return Some((50.0, 50.0)); }
        
        let k = (current - low) / (high - low) * 100.0;
        
        if self.k_values.len() < self.d_period { return Some((k, 50.0)); }
        
        let d = self.k_values.iter().sum::<f64>() / self.k_values.len() as f64;
        Some((k, d))
    }
}

#[async_trait]
impl TradingStrategy for StochasticOscillator {
    fn name(&self) -> &str { "StochasticOscillator" }

    fn get_features(&self) -> Vec<(String, String)> {
        let (k, d) = self.calculate_stochastic().unwrap_or((0.0, 0.0));
        vec![
            ("%K".to_string(), format!("{:.1}", k)),
            ("%D".to_string(), format!("{:.1}", d)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 50 { self.prices.pop_front(); }
        
        let mut opps = Vec::new();
        if let Some((k, _)) = self.calculate_stochastic() {
            self.k_values.push_back(k);
            if self.k_values.len() > self.d_period { self.k_values.pop_front(); }
            
            let current_state = state.read().await.state_machine.get_state();
            if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 {
                if k < 20.0 {
                    opps.push(Opportunity {
                        id: format!("stoch_buy_{}", trade.event_time),
                        signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.75,
                        risk_score: 0.3,
                        reason: format!("Stochastic Oversold: %K={:.1}", k),
                        timestamp: trade.event_time,
                    });
                    self.last_signal_time = trade.event_time;
                } else if k > 80.0 {
                    opps.push(Opportunity {
                        id: format!("stoch_sell_{}", trade.event_time),
                        signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.75,
                        risk_score: 0.3,
                        reason: format!("Stochastic Overbought: %K={:.1}", k),
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
        if self.prices.len() > 50 { self.prices.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
