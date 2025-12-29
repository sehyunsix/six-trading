use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;

/// Scalper Strategy - High frequency small profit trades
pub struct ScalperStrategy {
    tick_history: VecDeque<f64>,
    last_signal_time: u64,
    position_open: bool,
    entry_price: f64,
}

impl ScalperStrategy {
    pub fn new() -> Self {
        Self {
            tick_history: VecDeque::with_capacity(20),
            last_signal_time: 0,
            position_open: false,
            entry_price: 0.0,
        }
    }
}

#[async_trait]
impl TradingStrategy for ScalperStrategy {
    fn name(&self) -> &str { "ScalperStrategy" }

    fn get_features(&self) -> Vec<(String, String)> {
        let mut trend = 0.0;
        if self.tick_history.len() >= 10 {
            let recent: Vec<f64> = self.tick_history.iter().rev().take(5).copied().collect();
            let older: Vec<f64> = self.tick_history.iter().rev().skip(5).take(5).copied().collect();
            let recent_avg = recent.iter().sum::<f64>() / 5.0;
            let older_avg = older.iter().sum::<f64>() / 5.0;
            trend = (recent_avg - older_avg) / older_avg * 10000.0;
        }
        vec![
            ("Trend (bps)".to_string(), format!("{:.1}", trend)),
            ("In Position".to_string(), self.position_open.to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        self.tick_history.push_back(price);
        if self.tick_history.len() > 20 { self.tick_history.pop_front(); }
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && self.tick_history.len() >= 10 {
            let recent: Vec<f64> = self.tick_history.iter().rev().take(5).copied().collect();
            let older: Vec<f64> = self.tick_history.iter().rev().skip(5).take(5).copied().collect();
            
            let recent_avg = recent.iter().sum::<f64>() / 5.0;
            let older_avg = older.iter().sum::<f64>() / 5.0;
            let micro_trend = (recent_avg - older_avg) / older_avg * 10000.0; // basis points
            
            if !self.position_open && micro_trend > 1.0 && trade.event_time - self.last_signal_time > 5000 {
                self.position_open = true;
                self.entry_price = price;
                opps.push(Opportunity {
                    id: format!("scalp_buy_{}", trade.event_time),
                    signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.0005 },
                    score: (micro_trend / 10.0).min(0.8),
                    risk_score: 0.5,
                    reason: format!("Micro uptrend: {:.1} bps", micro_trend),
                    timestamp: trade.event_time,
                });
                self.last_signal_time = trade.event_time;
            } else if self.position_open {
                let pnl_bps = (price - self.entry_price) / self.entry_price * 10000.0;
                if pnl_bps > 5.0 || pnl_bps < -3.0 {
                    self.position_open = false;
                    opps.push(Opportunity {
                        id: format!("scalp_sell_{}", trade.event_time),
                        signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.0005 },
                        score: 0.7,
                        risk_score: 0.3,
                        reason: format!("Scalp exit: {:.1} bps P&L", pnl_bps),
                        timestamp: trade.event_time,
                    });
                }
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        self.tick_history.push_back(price);
        if self.tick_history.len() > 20 { self.tick_history.pop_front(); }
        Vec::new()
    }

    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
