use super::{Signal, TradingStrategy};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use log::info;
use std::time::Instant;

pub struct PaperTrader {
    trade_count: u64,
    last_price: Option<f64>,
    last_spread: f64,
}

impl PaperTrader {
    pub fn new() -> Self {
        Self { 
            trade_count: 0,
            last_price: None,
            last_spread: 0.0,
        }
    }
}

#[async_trait]
impl TradingStrategy for PaperTrader {
    fn name(&self) -> &str {
        "PaperTrader"
    }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Spread".to_string(), format!("{:.4}", self.last_spread)),
            ("Trade Count".to_string(), self.trade_count.to_string()),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<super::Opportunity> {
        let trade_price = trade.price.parse::<f64>().unwrap_or(0.0);
        let trade_qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.handle_trade_data(trade.symbol, trade_price, trade_qty, trade.event_time, state).await
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<super::Opportunity> {
        let trade_price = trade.price.parse::<f64>().unwrap_or(0.0);
        let trade_qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.handle_trade_data(trade.symbol, trade_price, trade_qty, trade.event_time, state).await
    }

    async fn process_orderbook(&mut self, orderbook: OrderBook, state: SharedState) -> Vec<super::Opportunity> {
        let start = Instant::now();
        
        // 1. Calculate Scores
        let mut spread_score = 0.0;
        let mut imbalance_score = 0.0;
        let mut mid_price = 0.0;
        let mut volume = 0.0;
        let mut spread = 0.0;

        if !orderbook.bids.is_empty() && !orderbook.asks.is_empty() {
            let best_bid = orderbook.bids[0].price;
            let best_ask = orderbook.asks[0].price;
            mid_price = (best_bid + best_ask) / 2.0;
            volume = (orderbook.bids[0].qty + orderbook.asks[0].qty) / 2.0;
            spread = best_ask - best_bid;
            self.last_spread = spread;

            spread_score = (best_ask - best_bid) / mid_price * 1000.0; // Scaled
            imbalance_score = (orderbook.bids[0].qty - orderbook.asks[0].qty) / (orderbook.bids[0].qty + orderbook.asks[0].qty);
        }

        // 2. Update State Machine
        {
            let mut write_guard = state.write().await;
            if write_guard.state_machine.get_state() == SystemState::Booting {
                info!("OrderBook received: Transitioning Booting -> Accumulating");
                write_guard.state_machine.transition_to(SystemState::Accumulating);
            }

            // Update inferred probabilities
            write_guard.state_machine.update_inferred_probabilities(spread_score, imbalance_score, 0.0);
            
            // Pushing mid price to history
            if mid_price > 0.0 {
                let strat_lat = write_guard.metrics.get_strategy_stats().p50;
                let exec_lat = write_guard.metrics.get_execution_stats().p50;
                write_guard.push_data_point(mid_price, volume, None, strat_lat, exec_lat, spread);
            }
        }

        state.read().await.metrics.record_strategy_latency(start.elapsed());
        Vec::new()
    }
}

impl PaperTrader {
    async fn handle_trade_data(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<super::Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        
        let mut volatility_score = 0.0;
        // 1. Update State Machine
        {
            let mut write_guard = state.write().await;
            
            let current = write_guard.state_machine.get_state();
            if current == SystemState::Booting {
                info!("Market Data received: Transitioning Booting -> Accumulating");
                write_guard.state_machine.transition_to(SystemState::Accumulating);
            } else if current == SystemState::Accumulating {
                 if write_guard.state_machine.is_stable() {
                     write_guard.state_machine.transition_to(SystemState::Trading);
                 }
            }

            if let Some(lp) = self.last_price {
                volatility_score = (price - lp).abs() / lp * 1000.0;
            }
            self.last_price = Some(price);
            write_guard.state_machine.update_inferred_probabilities(0.01, 0.0, volatility_score); 
        }

        // 2. Opportunity Generation
        let mut opportunities = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading {
            // High Confidence Buy Opportunity (Mock)
            if self.trade_count % 5 == 0 {
                opportunities.push(super::Opportunity {
                    id: format!("buy_{}", self.trade_count),
                    signal: Signal::Buy { symbol: symbol.clone(), price: Some(price * 0.999), quantity: 0.001 },
                    score: 0.85,
                    risk_score: 0.2,
                    reason: "Strong momentum detected with low volatility".to_string(),
                    timestamp: ts,
                });
            }
            
            // Scalp Sell Opportunity (Mock)
            if self.trade_count % 8 == 0 {
                opportunities.push(super::Opportunity {
                    id: format!("sell_{}", self.trade_count),
                    signal: Signal::Sell { symbol: symbol.clone(), price: Some(price * 1.001), quantity: 0.001 },
                    score: 0.65,
                    risk_score: 0.4,
                    reason: "Local resistance breakout attempt".to_string(),
                    timestamp: ts,
                });
            }
        }

        // 3. Record history (Take the best opportunity for display if exists)
        {
            let mut write_guard = state.write().await;
            let action = opportunities.first().map(|o| match &o.signal {
                Signal::Buy { .. } => "Buy".to_string(),
                Signal::Sell { .. } => "Sell".to_string(),
                _ => "Cancel".to_string(),
            });
            
            let strat_lat = write_guard.metrics.get_strategy_stats().p50;
            let exec_lat = write_guard.metrics.get_execution_stats().p50;
            let spread = self.last_spread;
            
            write_guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, spread, ts);
        }

        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opportunities
    }
}
