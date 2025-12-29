use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// VWAP Strategy - Volume Weighted Average Price
/// Trades reversions to VWAP as institutional benchmark
pub struct VWAPStrategy {
    prices: VecDeque<f64>,
    volumes: VecDeque<f64>,
    vwap: f64,
    last_signal_time: u64,
}

impl VWAPStrategy {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(100),
            volumes: VecDeque::with_capacity(100),
            vwap: 0.0,
            last_signal_time: 0,
        }
    }

    fn calculate_vwap(&self) -> f64 {
        if self.prices.is_empty() { return 0.0; }
        let pv_sum: f64 = self.prices.iter().zip(self.volumes.iter()).map(|(p, v)| p * v).sum();
        let v_sum: f64 = self.volumes.iter().sum();
        if v_sum == 0.0 { 0.0 } else { pv_sum / v_sum }
    }
}

#[async_trait]
impl TradingStrategy for VWAPStrategy {
    fn name(&self) -> &str { "VWAPStrategy" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("VWAP".to_string(), format!("{:.2}", self.vwap)),
            ("Samples".to_string(), self.prices.len().to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        self.volumes.push_back(qty);
        if self.prices.len() > 100 { self.prices.pop_front(); self.volumes.pop_front(); }
        
        self.vwap = self.calculate_vwap();
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 30000 && self.vwap > 0.0 {
            let deviation = (price - self.vwap) / self.vwap * 100.0;
            
            if deviation < -0.1 {
                opps.push(Opportunity {
                    id: format!("vwap_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: (deviation.abs() / 0.5).min(0.9),
                    risk_score: 0.3,
                    reason: format!("Below VWAP by {:.3}%", deviation.abs()),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if deviation > 0.1 {
                opps.push(Opportunity {
                    id: format!("vwap_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: (deviation / 0.5).min(0.85),
                    risk_score: 0.35,
                    reason: format!("Above VWAP by {:.3}%", deviation),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.prices.push_back(price); self.volumes.push_back(qty);
        if self.prices.len() > 100 { self.prices.pop_front(); self.volumes.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
