use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// MACD Crossover Strategy
pub struct MACDCrossover {
    prices: VecDeque<f64>,
    ema12: f64,
    ema26: f64,
    signal_line: f64,
    prev_histogram: f64,
    last_signal_time: u64,
}

impl MACDCrossover {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(30),
            ema12: 0.0,
            ema26: 0.0,
            signal_line: 0.0,
            prev_histogram: 0.0,
            last_signal_time: 0,
        }
    }

    fn update_ema(&mut self, price: f64) {
        let k12 = 2.0 / 13.0;
        let k26 = 2.0 / 27.0;
        let k9 = 2.0 / 10.0;
        
        if self.ema12 == 0.0 { self.ema12 = price; self.ema26 = price; }
        
        self.ema12 = price * k12 + self.ema12 * (1.0 - k12);
        self.ema26 = price * k26 + self.ema26 * (1.0 - k26);
        
        let macd_line = self.ema12 - self.ema26;
        self.signal_line = macd_line * k9 + self.signal_line * (1.0 - k9);
    }
}

#[async_trait]
impl TradingStrategy for MACDCrossover {
    fn name(&self) -> &str { "MACDCrossover" }

    fn get_features(&self) -> Vec<(String, String)> {
        let macd = self.ema12 - self.ema26;
        let hist = macd - self.signal_line;
        vec![
            ("MACD Line".to_string(), format!("{:.4}", macd)),
            ("Signal Line".to_string(), format!("{:.4}", self.signal_line)),
            ("Histogram".to_string(), format!("{:.4}", hist)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push_back(price);
        if self.prices.len() > 30 { self.prices.pop_front(); }
        
        let prev_hist = self.prev_histogram;
        self.update_ema(price);
        let histogram = (self.ema12 - self.ema26) - self.signal_line;
        self.prev_histogram = histogram;
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && 
           self.prices.len() >= 26 && 
           trade.event_time - self.last_signal_time > 45000 {
            
            // Bullish crossover
            if prev_hist < 0.0 && histogram > 0.0 {
                opps.push(Opportunity {
                    id: format!("macd_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.75,
                    risk_score: 0.35,
                    reason: "MACD bullish crossover".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            }
            // Bearish crossover
            else if prev_hist > 0.0 && histogram < 0.0 {
                opps.push(Opportunity {
                    id: format!("macd_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.7,
                    risk_score: 0.4,
                    reason: "MACD bearish crossover".to_string(),
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
        self.update_ema(price);
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
