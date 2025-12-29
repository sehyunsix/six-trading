use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;
use std::time::Instant;

/// Momentum Breakout Strategy
/// Captures trending markets using price velocity and volatility analysis
pub struct MomentumBreakout {
    price_history: VecDeque<f64>,
    volume_history: VecDeque<f64>,
    window_size: usize,
    last_signal_time: u64,
    signal_cooldown_ms: u64,
    atr: f64,
}

impl MomentumBreakout {
    pub fn new() -> Self {
        Self {
            price_history: VecDeque::with_capacity(50),
            volume_history: VecDeque::with_capacity(50),
            window_size: 20,
            last_signal_time: 0,
            signal_cooldown_ms: 60000, // 1 minute cooldown
            atr: 0.0,
        }
    }

    /// Calculate Average True Range for risk management
    fn calculate_atr(&mut self) -> f64 {
        if self.price_history.len() < 2 {
            return 0.0;
        }

        let mut ranges = Vec::new();
        for i in 1..self.price_history.len() {
            let high_low = (self.price_history[i] - self.price_history[i-1]).abs();
            ranges.push(high_low);
        }

        if ranges.is_empty() {
            return 0.0;
        }

        ranges.iter().sum::<f64>() / ranges.len() as f64
    }

    /// Calculate momentum score based on price velocity
    fn calculate_momentum(&self) -> f64 {
        if self.price_history.len() < self.window_size {
            return 0.0;
        }

        let recent = &self.price_history;
        let old_price = recent[recent.len() - self.window_size];
        let current_price = recent[recent.len() - 1];
        
        (current_price - old_price) / old_price * 100.0
    }

    /// Calculate volume surge factor
    fn calculate_volume_surge(&self) -> f64 {
        if self.volume_history.len() < 10 {
            return 1.0;
        }

        let recent_vol: f64 = self.volume_history.iter().rev().take(3).sum();
        let avg_vol: f64 = self.volume_history.iter().sum::<f64>() / self.volume_history.len() as f64;

        if avg_vol == 0.0 {
            return 1.0;
        }

        (recent_vol / 3.0) / avg_vol
    }
}

#[async_trait]
impl TradingStrategy for MomentumBreakout {
    fn name(&self) -> &str {
        "MomentumBreakout"
    }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Momentum".to_string(), format!("{:.2}%", self.calculate_momentum())),
            ("ATR".to_string(), format!("{:.2}", self.atr)),
            ("Vol Surge".to_string(), format!("{:.2}x", self.calculate_volume_surge())),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.handle_market_data(trade.symbol, price, qty, trade.event_time, state).await
    }

    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        self.handle_market_data(trade.symbol, price, qty, trade.event_time, state).await
    }

    async fn process_orderbook(&mut self, orderbook: OrderBook, state: SharedState) -> Vec<Opportunity> {
        if orderbook.bids.is_empty() || orderbook.asks.is_empty() {
            return Vec::new();
        }

        let mid_price = (orderbook.bids[0].price + orderbook.asks[0].price) / 2.0;
        let volume = (orderbook.bids[0].qty + orderbook.asks[0].qty) / 2.0;
        let spread = orderbook.asks[0].price - orderbook.bids[0].price;

        {
            let mut write_guard = state.write().await;
            if mid_price > 0.0 {
                let strat_lat = write_guard.metrics.get_strategy_stats().p50;
                let exec_lat = write_guard.metrics.get_execution_stats().p50;
                write_guard.push_data_point(mid_price, volume, None, strat_lat, exec_lat, spread);
            }
        }

        Vec::new()
    }
}

impl MomentumBreakout {
    async fn handle_market_data(
        &mut self, 
        symbol: String, 
        price: f64, 
        qty: f64, 
        ts: u64, 
        state: SharedState
    ) -> Vec<Opportunity> {
        let start = Instant::now();

        // Update price and volume history
        self.price_history.push_back(price);
        self.volume_history.push_back(qty);

        if self.price_history.len() > 50 {
            self.price_history.pop_front();
        }
        if self.volume_history.len() > 50 {
            self.volume_history.pop_front();
        }

        // Calculate metrics
        self.atr = self.calculate_atr();
        let momentum = self.calculate_momentum();
        let volume_surge = self.calculate_volume_surge();

        let mut opportunities = Vec::new();
        let current_state = state.read().await.state_machine.get_state();

        // Generate signals only in Trading state with cooldown
        if current_state == SystemState::Trading && 
           ts - self.last_signal_time > self.signal_cooldown_ms &&
           self.price_history.len() >= self.window_size {

            // Bullish breakout: Strong positive momentum + volume surge
            if momentum > 0.2 && volume_surge > 1.1 {
                let stop_loss_distance = self.atr * 2.0;
                let position_size = 0.001 * (1.0 / (self.atr.max(0.0001) / price)); // ATR-based sizing

                opportunities.push(Opportunity {
                    id: format!("momentum_buy_{}", ts),
                    signal: Signal::Buy {
                        symbol: symbol.clone(),
                        price: Some(price * 1.0001), // Slight premium for market entry
                        quantity: position_size.min(0.01).max(0.0001),
                    },
                    score: (momentum / 2.0 + volume_surge / 3.0).min(0.95),
                    risk_score: (self.atr / price * 100.0).min(1.0),
                    reason: format!("Momentum breakout: {:.2}% velocity, {:.1}x volume", momentum, volume_surge),
                    timestamp: ts,
                });

                self.last_signal_time = ts;
            }

            // Bearish reversal: Negative momentum after uptrend
            if momentum < -0.3 && self.price_history.len() > 10 {
                let recent_high = self.price_history.iter().rev().take(10).fold(0.0_f64, |a: f64, &b| a.max(b));
                if price < recent_high * 0.998 {
                    opportunities.push(Opportunity {
                        id: format!("momentum_sell_{}", ts),
                        signal: Signal::Sell {
                            symbol: symbol.clone(),
                            price: Some(price * 0.9999),
                            quantity: 0.001,
                        },
                        score: (momentum.abs() / 2.0).min(0.75),
                        risk_score: 0.3,
                        reason: format!("Momentum reversal detected: {:.2}% decline", momentum),
                        timestamp: ts,
                    });

                    self.last_signal_time = ts;
                }
            }
        }

        // Record metrics
        {
            let mut write_guard = state.write().await;
            let action = opportunities.first().map(|o| match &o.signal {
                Signal::Buy { .. } => "Buy".to_string(),
                Signal::Sell { .. } => "Sell".to_string(),
                _ => "Cancel".to_string(),
            });

            let strat_lat = write_guard.metrics.get_strategy_stats().p50;
            let exec_lat = write_guard.metrics.get_execution_stats().p50;
            write_guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, self.atr, ts);
        }

        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opportunities
    }
}
