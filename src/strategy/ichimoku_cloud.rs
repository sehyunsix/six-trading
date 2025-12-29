use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Ichimoku Cloud Strategy (Simplified)
pub struct IchimokuCloud {
    prices: VecDeque<f64>,
    tenkan_period: usize,
    kijun_period: usize,
    tenkan: f64,
    kijun: f64,
    last_signal_time: u64,
}

impl IchimokuCloud {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(100),
            tenkan_period: 9,
            kijun_period: 26,
            tenkan: 0.0,
            kijun: 0.0,
            last_signal_time: 0,
        }
    }

    fn calculate_n_period_mid(&self, n: usize) -> f64 {
        if self.prices.len() < n { return 0.0; }
        let recent = self.prices.iter().rev().take(n);
        let high = recent.clone().cloned().fold(f64::NEG_INFINITY, f64::max);
        let low = recent.cloned().fold(f64::INFINITY, f64::min);
        (high + low) / 2.0
    }
}

#[async_trait]
impl TradingStrategy for IchimokuCloud {
    fn name(&self) -> &str { "IchimokuCloud" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Tenkan".to_string(), format!("{:.2}", self.tenkan)),
            ("Kijun".to_string(), format!("{:.2}", self.kijun)),
            ("TK Gap".to_string(), format!("{:.2}", self.tenkan - self.kijun)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 100 { self.prices.pop_front(); }
        
        let prev_tenkan = self.tenkan;
        let prev_kijun = self.kijun;
        self.tenkan = self.calculate_n_period_mid(self.tenkan_period);
        self.kijun = self.calculate_n_period_mid(self.kijun_period);
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 && prev_kijun > 0.0 {
            // Tenkan crosses Kijun from below
            if prev_tenkan <= prev_kijun && self.tenkan > self.kijun {
                opps.push(Opportunity {
                    id: format!("ichimoku_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.3,
                    reason: "Tenkan-Kijun Bullish Cross".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if prev_tenkan >= prev_kijun && self.tenkan < self.kijun {
                opps.push(Opportunity {
                    id: format!("ichimoku_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.3,
                    reason: "Tenkan-Kijun Bearish Cross".to_string(),
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
        if self.prices.len() > 100 { self.prices.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
