use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};

/// Buy and Hold Strategy - Buys once and stays in position
pub struct BuyAndHold {
    has_bought: bool,
}

impl BuyAndHold {
    pub fn new() -> Self {
        Self { has_bought: false }
    }
}

#[async_trait]
impl TradingStrategy for BuyAndHold {
    fn name(&self) -> &str { "BuyAndHold" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Bought".to_string(), self.has_bought.to_string()),
            ("Strategy".to_string(), "Passive".to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        let mut opps = Vec::new();
        if !self.has_bought {
            opps.push(Opportunity {
                id: format!("buy_hold_{}", trade.event_time),
                signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.1 }, // Buy 0.1 BTC
                score: 1.0,
                risk_score: 0.0,
                reason: "Initial Buy and Hold purchase".to_string(),
                timestamp: trade.event_time,
            });
            self.has_bought = true;
        }

        {
            let mut w = state.write().await;
            w.push_data_point_at(price, qty, opps.first().map(|_| "Buy".to_string()), 0, 0, 0.0, trade.event_time);
        }
        
        opps
    }

    async fn process_aggr_trade(&mut self, _trade: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> {
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> {
        Vec::new()
    }
}
