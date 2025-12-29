use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Triple EMA (TRIX) Strategy
pub struct TRIXStrategy {
    prices: VecDeque<f64>,
    ema1: f64,
    ema2: f64,
    ema3: f64,
    prev_trix: f64,
    period: usize,
    last_signal_time: u64,
}

impl TRIXStrategy {
    pub fn new() -> Self {
        Self {
            prices: VecDeque::with_capacity(100),
            ema1: 0.0,
            ema2: 0.0,
            ema3: 0.0,
            prev_trix: 0.0,
            period: 15,
            last_signal_time: 0,
        }
    }

    fn update_ema(&mut self, price: f64) -> f64 {
        let k = 2.0 / (self.period as f64 + 1.0);
        
        if self.ema1 == 0.0 {
            self.ema1 = price;
            self.ema2 = price;
            self.ema3 = price;
        } else {
            self.ema1 = price * k + self.ema1 * (1.0 - k);
            self.ema2 = self.ema1 * k + self.ema2 * (1.0 - k);
            self.ema3 = self.ema2 * k + self.ema3 * (1.0 - k);
        }
        
        self.ema3
    }
}

#[async_trait]
impl TradingStrategy for TRIXStrategy {
    fn name(&self) -> &str { "TRIXStrategy" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("EMA3".to_string(), format!("{:.2}", self.ema3)),
            ("Prev TRIX".to_string(), format!("{:.4}", self.prev_trix)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        let prev_ema3 = self.ema3;
        self.update_ema(price);
        
        // TRIX = (EMA3 - Prev EMA3) / Prev EMA3
        let trix = if prev_ema3 > 0.0 { (self.ema3 - prev_ema3) / prev_ema3 * 100.0 } else { 0.0 };
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 60000 {
            if self.prev_trix < 0.0 && trix > 0.0 {
                opps.push(Opportunity {
                    id: format!("trix_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.75,
                    risk_score: 0.4,
                    reason: "TRIX Bullish Crossover".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if self.prev_trix > 0.0 && trix < 0.0 {
                opps.push(Opportunity {
                    id: format!("trix_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.75,
                    risk_score: 0.4,
                    reason: "TRIX Bearish Crossover".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            }
        }
        
        self.prev_trix = trix;
        
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
