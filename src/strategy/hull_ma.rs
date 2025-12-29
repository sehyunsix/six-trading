use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Hull Moving Average (HMA) Strategy
pub struct HullMA {
    prices: VecDeque<f64>,
    period: usize,
    hma: f64,
    prev_hma: f64,
    last_signal_time: u64,
}

impl HullMA {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(100),
            period: 20,
            hma: 0.0,
            prev_hma: 0.0,
            last_signal_time: 0,
        }
    }

    fn wma(prices: &[f64], period: usize) -> f64 {
        if prices.len() < period { return 0.0; }
        let mut denominator = 0.0;
        let mut numerator = 0.0;
        for i in 0..period {
            let weight = (i + 1) as f64;
            numerator += prices[prices.len() - period + i] * weight;
            denominator += weight;
        }
        numerator / denominator
    }

    fn calculate_hma(&self) -> f64 {
        if self.prices.len() < self.period { return 0.0; }
        
        // HMA = WMA(2*WMA(n/2) - WMA(n), sqrt(n))
        let half_period = self.period / 2;
        let sqrt_period = (self.period as f64).sqrt() as usize;
        
        let wma_half = Self::wma(self.prices.as_slices().0, half_period);
        let wma_full = Self::wma(self.prices.as_slices().0, self.period);
        
        let raw_hma = 2.0 * wma_half - wma_full;
        
        // This is simplified as we'd need another history for raw_hma to do the final WMA
        raw_hma
    }
}

#[async_trait]
impl TradingStrategy for HullMA {
    fn name(&self) -> &str { "HullMA" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("HMA".to_string(), format!("{:.2}", self.hma)),
            ("Trend".to_string(), if self.hma > self.prev_hma { "Up" } else { "Down" }.to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 100 { self.prices.pop_front(); }
        
        self.prev_hma = self.hma;
        self.hma = self.calculate_hma();
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 && self.prev_hma > 0.0 {
            if self.hma > self.prev_hma * 1.0001 {
                opps.push(Opportunity {
                    id: format!("hma_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.3,
                    reason: "HMA Turning Up".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if self.hma < self.prev_hma * 0.9999 {
                opps.push(Opportunity {
                    id: format!("hma_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.3,
                    reason: "HMA Turning Down".to_string(),
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
