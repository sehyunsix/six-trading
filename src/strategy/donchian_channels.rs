use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Donchian Channels Strategy
pub struct DonchianChannels {
    prices: VecDeque<f64>,
    period: usize,
    upper: f64,
    lower: f64,
    last_signal_time: u64,
}

impl DonchianChannels {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(50),
            period: 20,
            upper: 0.0,
            lower: f64::MAX,
            last_signal_time: 0,
        }
    }

    fn update_channels(&mut self) {
        if self.prices.len() < self.period { return; }
        
        let recent = self.prices.iter().rev().take(self.period);
        self.upper = recent.clone().cloned().fold(f64::NEG_INFINITY, f64::max);
        self.lower = recent.cloned().fold(f64::INFINITY, f64::min);
    }
}

#[async_trait]
impl TradingStrategy for DonchianChannels {
    fn name(&self) -> &str { "DonchianChannels" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Upper".to_string(), format!("{:.2}", self.upper)),
            ("Lower".to_string(), format!("{:.2}", self.lower)),
            ("Mid".to_string(), format!("{:.2}", (self.upper + self.lower) / 2.0)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 50 { self.prices.pop_front(); }
        
        self.update_channels();
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 {
            if price >= self.upper {
                opps.push(Opportunity {
                    id: format!("donchian_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.85,
                    risk_score: 0.35,
                    reason: format!("Donchian Upper Breakout: {:.2}", price),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if price <= self.lower {
                opps.push(Opportunity {
                    id: format!("donchian_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.85,
                    risk_score: 0.4,
                    reason: format!("Donchian Lower Breakdown: {:.2}", price),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        self.prices.push_back(price);
        if self.prices.len() > 50 { self.prices.pop_front(); }
        self.update_channels();
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
