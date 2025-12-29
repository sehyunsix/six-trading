use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Bollinger Band Squeeze Strategy
pub struct BBSqueeze {
    prices: VecDeque<f64>,
    period: usize,
    std_dev: f64,
    kc_mult: f64,  // Keltner Channel multiplier
    last_signal_time: u64,
}

impl BBSqueeze {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(50),
            period: 20,
            std_dev: 2.0,
            kc_mult: 1.5,
            last_signal_time: 0,
        }
    }

    fn calculate_metrics(&self) -> Option<(f64, f64, f64, bool)> {
        if self.prices.len() < self.period { return None; }
        
        let recent: Vec<f64> = self.prices.iter().rev().take(self.period).copied().collect();
        let sma = recent.iter().sum::<f64>() / self.period as f64;
        let variance = recent.iter().map(|p| (p - sma).powi(2)).sum::<f64>() / self.period as f64;
        let stdev = variance.sqrt();
        
        let bb_upper = sma + stdev * self.std_dev;
        let bb_lower = sma - stdev * self.std_dev;
        
        // Simplified Keltner Channel
        let atr = stdev; // Mock ATR
        let kc_upper = sma + atr * self.kc_mult;
        let kc_lower = sma - atr * self.kc_mult;
        
        // Squeeze if BB is inside KC
        let squeeze = bb_upper < kc_upper && bb_lower > kc_lower;
        Some((bb_upper, bb_lower, sma, squeeze))
    }
}

#[async_trait]
impl TradingStrategy for BBSqueeze {
    fn name(&self) -> &str { "BBSqueeze" }

    fn get_features(&self) -> Vec<(String, String)> {
        let metrics = self.calculate_metrics();
        vec![
            ("Squeeze".to_string(), metrics.map(|m| m.3.to_string()).unwrap_or("False".to_string())),
            ("BB Width".to_string(), metrics.map(|m| format!("{:.2}", m.0 - m.1)).unwrap_or("0.0".to_string())),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 50 { self.prices.pop_front(); }
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if let Some((upper, lower, sma, squeeze)) = self.calculate_metrics() {
            if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 {
                // If squeeze is releasing
                if !squeeze && price > upper {
                    opps.push(Opportunity {
                        id: format!("bb_squeeze_buy_{}", trade.event_time),
                        signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.8,
                        risk_score: 0.4,
                        reason: "BB Squeeze release bullish".to_string(),
                        timestamp: trade.event_time,
                    });
                    self.last_signal_time = trade.event_time;
                } else if !squeeze && price < lower {
                    opps.push(Opportunity {
                        id: format!("bb_squeeze_sell_{}", trade.event_time),
                        signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.8,
                        risk_score: 0.4,
                        reason: "BB Squeeze release bearish".to_string(),
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
