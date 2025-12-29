use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

pub struct MeanReversionStrategy {
    trade_count: u64,
    last_spread: f64,
    prices: Vec<f64>,
}

impl MeanReversionStrategy {
    pub fn new() -> Self {
        Self { 
            trade_count: 0,
            last_spread: 0.0,
            prices: Vec::with_capacity(100),
        }
    }
}

#[async_trait]
impl TradingStrategy for MeanReversionStrategy {
    fn name(&self) -> &str {
        "MeanReversion"
    }

    fn get_features(&self) -> Vec<(String, String)> {
        let mean: f64 = if self.prices.is_empty() { 0.0 } else { self.prices.iter().sum::<f64>() / self.prices.len() as f64 };
        let std_dev: f64 = if self.prices.len() < 2 { 0.0 } else { (self.prices.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / self.prices.len() as f64).sqrt() };
        
        vec![
            ("Mean".to_string(), format!("{:.2}", mean)),
            ("StdDev".to_string(), format!("{:.4}", std_dev)),
            ("Spread".to_string(), format!("{:.4}", self.last_spread)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let trade_price = trade.price.parse::<f64>().unwrap_or(0.0);
        let trade_qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.handle_trade_data(trade.symbol, trade_price, trade_qty, trade.event_time, state).await
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity> {
        let trade_price = trade.price.parse::<f64>().unwrap_or(0.0);
        let trade_qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.handle_trade_data(trade.symbol, trade_price, trade_qty, trade.event_time, state).await
    }

    async fn process_orderbook(&mut self, orderbook: OrderBook, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        
        if !orderbook.bids.is_empty() && !orderbook.asks.is_empty() {
            let best_bid = orderbook.bids[0].price;
            let best_ask = orderbook.asks[0].price;
            let mid_price = (best_bid + best_ask) / 2.0;
            self.last_spread = best_ask - best_bid;

            let mut write_guard = state.write().await;
            if write_guard.state_machine.get_state() == SystemState::Booting {
                write_guard.state_machine.transition_to(SystemState::Accumulating);
            }
            
            // Just push data for chart
            let strat_lat = write_guard.metrics.get_strategy_stats().p50;
            let exec_lat = write_guard.metrics.get_execution_stats().p50;
            write_guard.push_data_point(mid_price, 0.0, None, strat_lat, exec_lat, self.last_spread);
        }

        state.read().await.metrics.record_strategy_latency(start.elapsed());
        Vec::new()
    }
}

impl MeanReversionStrategy {
    async fn handle_trade_data(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        
        // Add price to history
        self.prices.push(price);
        if self.prices.len() > 20 {
            self.prices.remove(0);
        }

        // 1. Update State Machine
        {
            let mut write_guard = state.write().await;
            let current = write_guard.state_machine.get_state();
            if current == SystemState::Booting {
                write_guard.state_machine.transition_to(SystemState::Accumulating);
            } else if current == SystemState::Accumulating && write_guard.state_machine.is_stable() {
                write_guard.state_machine.transition_to(SystemState::Trading);
            }
        }

        // 2. Opportunity Generation (Mean Reversion Logic)
        let mut opportunities = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && self.prices.len() >= 10 {
            let mean: f64 = self.prices.iter().sum::<f64>() / self.prices.len() as f64;
            let std_dev: f64 = (self.prices.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / self.prices.len() as f64).sqrt();
            
            // Buy if price is significantly below mean
            if price < mean - 1.0 * std_dev {
                opportunities.push(Opportunity {
                    id: format!("mr_buy_{}", self.trade_count),
                    signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.3,
                    reason: format!("Price is {:.2} below mean", mean - price),
                    timestamp: ts,
                });
            }
            
            // Sell if price is significantly above mean
            if price > mean + 1.0 * std_dev {
                opportunities.push(Opportunity {
                    id: format!("mr_sell_{}", self.trade_count),
                    signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.8,
                    risk_score: 0.3,
                    reason: format!("Price is {:.2} above mean", price - mean),
                    timestamp: ts,
                });
            }
        }

        // 3. Record history
        {
            let mut write_guard = state.write().await;
            let action = opportunities.first().map(|o| match &o.signal {
                Signal::Buy { .. } => "Buy".to_string(),
                Signal::Sell { .. } => "Sell".to_string(),
                _ => "Cancel".to_string(),
            });
            
            let strat_lat = write_guard.metrics.get_strategy_stats().p50;
            let exec_lat = write_guard.metrics.get_execution_stats().p50;
            write_guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, self.last_spread, ts);
        }

        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opportunities
    }
}
