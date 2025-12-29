use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Chaikin Money Flow (CMF) Strategy
pub struct ChaikinMoneyFlow {
    prices: VecDeque<f64>,
    volumes: VecDeque<f64>,
    period: usize,
    last_cmf: f64,
    last_signal_time: u64,
}

impl ChaikinMoneyFlow {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(50),
            volumes: VecDeque::with_capacity(50),
            period: 21,
            last_cmf: 0.0,
            last_signal_time: 0,
        }
    }

    fn calculate_cmf(&self) -> f64 {
        if self.prices.len() < self.period { return 0.0; }
        
        let mut money_flow_volume_sum = 0.0;
        let mut volume_sum = 0.0;
        
        for i in (self.prices.len() - self.period)..self.prices.len() {
            let price = self.prices[i];
            let volume = self.volumes[i];
            
            // Simplified MFM (since we don't have high/low easily here, we use price vs prev price)
            let prev_price = if i > 0 { self.prices[i-1] } else { price };
            let mfm = if price > prev_price { 1.0 } else if price < prev_price { -1.0 } else { 0.0 };
            
            money_flow_volume_sum += mfm * volume;
            volume_sum += volume;
        }
        
        if volume_sum == 0.0 { 0.0 } else { money_flow_volume_sum / volume_sum }
    }
}

#[async_trait]
impl TradingStrategy for ChaikinMoneyFlow {
    fn name(&self) -> &str { "ChaikinMoneyFlow" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("CMF".to_string(), format!("{:.3}", self.last_cmf)),
            ("Volume Sum".to_string(), format!("{:.0}", self.volumes.iter().sum::<f64>())),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        self.volumes.push_back(qty);
        if self.prices.len() > 50 { self.prices.pop_front(); self.volumes.pop_front(); }
        
        self.last_cmf = self.calculate_cmf();
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 {
            if self.last_cmf > 0.1 {
                opps.push(Opportunity {
                    id: format!("cmf_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.7,
                    risk_score: 0.3,
                    reason: format!("CMF Bullish Accumulation: {:.3}", self.last_cmf),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if self.last_cmf < -0.1 {
                opps.push(Opportunity {
                    id: format!("cmf_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.7,
                    risk_score: 0.3,
                    reason: format!("CMF Bearish Distribution: {:.3}", self.last_cmf),
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
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.prices.push_back(price); self.volumes.push_back(qty);
        if self.prices.len() > 50 { self.prices.pop_front(); self.volumes.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
