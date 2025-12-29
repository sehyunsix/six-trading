use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};

/// Heikin-Ashi Trend Strategy
pub struct HeikinAshiTrend {
    prev_ha_open: f64,
    prev_ha_close: f64,
    is_bullish: bool,
    last_signal_time: u64,
}

impl HeikinAshiTrend {
    pub fn new() -> Self {
        Self {
            prev_ha_open: 0.0,
            prev_ha_close: 0.0,
            is_bullish: false,
            last_signal_time: 0,
        }
    }

    fn update_ha(&mut self, open: f64, high: f64, low: f64, close: f64) -> (f64, f64) {
        if self.prev_ha_open == 0.0 {
            self.prev_ha_open = open;
            self.prev_ha_close = close;
            return (open, close);
        }

        let ha_close = (open + high + low + close) / 4.0;
        let ha_open = (self.prev_ha_open + self.prev_ha_close) / 2.0;
        
        self.prev_ha_open = ha_open;
        self.prev_ha_close = ha_close;
        (ha_open, ha_close)
    }
}

#[async_trait]
impl TradingStrategy for HeikinAshiTrend {
    fn name(&self) -> &str { "HeikinAshiTrend" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("HA Open".to_string(), format!("{:.2}", self.prev_ha_open)),
            ("HA Close".to_string(), format!("{:.2}", self.prev_ha_close)),
            ("Trend".to_string(), if self.prev_ha_close > self.prev_ha_open { "Bullish" } else { "Bearish" }.to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        let (ha_open, ha_close) = self.update_ha(price, price, price, price); // Simplified HA
        let current_bullish = ha_close > ha_open;
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 30000 {
            if !self.is_bullish && current_bullish {
                opps.push(Opportunity {
                    id: format!("ha_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.7,
                    risk_score: 0.3,
                    reason: "Heikin-Ashi Bullish Flip".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if self.is_bullish && !current_bullish {
                opps.push(Opportunity {
                    id: format!("ha_sell_{}", trade.event_time),
                    signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.7,
                    risk_score: 0.3,
                    reason: "Heikin-Ashi Bearish Flip".to_string(),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            }
        }
        
        self.is_bullish = current_bullish;
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        self.update_ha(price, price, price, price);
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
