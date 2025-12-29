use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// Parabolic SAR Strategy
pub struct ParabolicSAR {
    sar: f64,
    ep: f64,      // Extreme Point
    af: f64,      // Acceleration Factor
    af_init: f64,
    af_max: f64,
    is_long: bool,
    prices: Vec<f64>,
    last_spread: f64,
}

impl ParabolicSAR {
    pub fn new() -> Self {
        Self {
            sar: 0.0,
            ep: 0.0,
            af: 0.02,
            af_init: 0.02,
            af_max: 0.2,
            is_long: true,
            prices: Vec::with_capacity(50),
            last_spread: 0.0,
        }
    }

    fn update_sar(&mut self, high: f64, low: f64) {
        if self.sar == 0.0 {
            self.sar = low;
            self.ep = high;
            return;
        }

        let prev_sar = self.sar;
        self.sar = prev_sar + self.af * (self.ep - prev_sar);

        if self.is_long {
            if low < self.sar {
                self.is_long = false;
                self.sar = self.ep;
                self.ep = low;
                self.af = self.af_init;
            } else {
                if high > self.ep {
                    self.ep = high;
                    self.af = (self.af + self.af_init).min(self.af_max);
                }
            }
        } else {
            if high > self.sar {
                self.is_long = true;
                self.sar = self.ep;
                self.ep = high;
                self.af = self.af_init;
            } else {
                if low < self.ep {
                    self.ep = low;
                    self.af = (self.af + self.af_init).min(self.af_max);
                }
            }
        }
    }
}

#[async_trait]
impl TradingStrategy for ParabolicSAR {
    fn name(&self) -> &str { "ParabolicSAR" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("SAR".to_string(), format!("{:.2}", self.sar)),
            ("Trend".to_string(), if self.is_long { "Bullish" } else { "Bearish" }.to_string()),
            ("AF".to_string(), format!("{:.3}", self.af)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.prices.push(price);
        if self.prices.len() > 5 { self.prices.remove(0); }
        
        // Simplified: use current price as high/low for update
        self.update_sar(price, price);
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading {
            if self.is_long && price > self.sar {
                 // SAR signal bullish
            } else if !self.is_long && price < self.sar {
                 // SAR signal bearish
            }
            
            // Generate a trade on trend flip
            if self.is_long && price > self.sar * 1.001 {
                opps.push(Opportunity {
                    id: format!("sar_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.7,
                    risk_score: 0.4,
                    reason: "SAR Bullish flip".to_string(),
                    timestamp: trade.event_time,
                });
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        self.update_sar(price, price);
        Vec::new()
    }

    async fn process_orderbook(&mut self, ob: OrderBook, _: SharedState) -> Vec<Opportunity> {
        if !ob.bids.is_empty() && !ob.asks.is_empty() {
            self.last_spread = ob.asks[0].price - ob.bids[0].price;
        }
        Vec::new()
    }
}
