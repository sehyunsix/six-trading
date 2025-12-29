use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::collections::VecDeque;
use std::time::Instant;

/// Adaptive Mean Reversion Strategy
/// Enhanced mean reversion with dynamic Bollinger Bands and RSI confirmation
pub struct AdaptiveMeanReversion {
    price_history: VecDeque<f64>,
    bb_period: usize,
    bb_std_dev: f64,
    rsi_period: usize,
    last_signal_time: u64,
    signal_cooldown_ms: u64,
    recent_volatility: f64,
}

impl AdaptiveMeanReversion {
    pub fn new() -> Self {
        Self {
            price_history: VecDeque::with_capacity(50),
            bb_period: 20,
            bb_std_dev: 2.0,
            rsi_period: 14,
            last_signal_time: 0,
            signal_cooldown_ms: 45000, // 45 seconds
            recent_volatility: 0.0,
        }
    }

    /// Calculate Bollinger Bands
    fn calculate_bollinger_bands(&self) -> Option<(f64, f64, f64)> {
        if self.price_history.len() < self.bb_period {
            return None;
        }

        let recent_prices: Vec<f64> = self.price_history.iter()
            .rev()
            .take(self.bb_period)
            .copied()
            .collect();

        let sma = recent_prices.iter().sum::<f64>() / recent_prices.len() as f64;
        
        let variance = recent_prices.iter()
            .map(|p| (p - sma).powi(2))
            .sum::<f64>() / recent_prices.len() as f64;
        
        let std_dev = variance.sqrt();
        
        let upper_band = sma + (std_dev * self.bb_std_dev);
        let lower_band = sma - (std_dev * self.bb_std_dev);

        Some((lower_band, sma, upper_band))
    }

    /// Calculate RSI
    fn calculate_rsi(&self) -> f64 {
        if self.price_history.len() < self.rsi_period + 1 {
            return 50.0; // Neutral
        }

        let prices: Vec<f64> = self.price_history.iter()
            .rev()
            .take(self.rsi_period + 1)
            .copied()
            .collect();

        let mut gains = 0.0;
        let mut losses = 0.0;

        for i in 1..prices.len() {
            let change = prices[i-1] - prices[i];
            if change > 0.0 {
                gains += change;
            } else {
                losses += change.abs();
            }
        }

        let avg_gain = gains / self.rsi_period as f64;
        let avg_loss = losses / self.rsi_period as f64;

        if avg_loss == 0.0 {
            return 100.0;
        }

        let rs = avg_gain / avg_loss;
        100.0 - (100.0 / (1.0 + rs))
    }

    /// Calculate recent volatility for dynamic stop-loss
    fn calculate_volatility(&mut self) -> f64 {
        if self.price_history.len() < 10 {
            return 0.0;
        }

        let recent: Vec<f64> = self.price_history.iter().rev().take(10).copied().collect();
        let mean = recent.iter().sum::<f64>() / recent.len() as f64;
        
        let variance = recent.iter()
            .map(|p| (p - mean).powi(2))
            .sum::<f64>() / recent.len() as f64;

        variance.sqrt() / mean * 100.0
    }
}

#[async_trait]
impl TradingStrategy for AdaptiveMeanReversion {
    fn name(&self) -> &str {
        "AdaptiveMeanReversion"
    }

    fn get_features(&self) -> Vec<(String, String)> {
        let rsi = self.calculate_rsi();
        let (lower, sma, upper) = self.calculate_bollinger_bands().unwrap_or((0.0, 0.0, 0.0));
        vec![
            ("RSI".to_string(), format!("{:.1}", rsi)),
            ("Volatility".to_string(), format!("{:.2}%", self.recent_volatility)),
            ("SMA".to_string(), format!("{:.2}", sma)),
            ("BB Upper".to_string(), format!("{:.2}", upper)),
            ("BB Lower".to_string(), format!("{:.2}", lower)),
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

impl AdaptiveMeanReversion {
    async fn handle_market_data(
        &mut self,
        symbol: String,
        price: f64,
        qty: f64,
        ts: u64,
        state: SharedState
    ) -> Vec<Opportunity> {
        let start = Instant::now();

        // Update history
        self.price_history.push_back(price);
        if self.price_history.len() > 50 {
            self.price_history.pop_front();
        }

        // Calculate indicators
        self.recent_volatility = self.calculate_volatility();
        let rsi = self.calculate_rsi();
        
        let mut opportunities = Vec::new();
        let current_state = state.read().await.state_machine.get_state();

        // Generate signals with Bollinger Bands and RSI confirmation
        if current_state == SystemState::Trading &&
           ts - self.last_signal_time > self.signal_cooldown_ms {
            
            if let Some((lower_band, sma, upper_band)) = self.calculate_bollinger_bands() {
                let distance_to_mean = (price - sma).abs() / sma * 100.0;
                
                // Oversold + RSI confirmation -> Buy
                if price < lower_band && rsi < 40.0 {
                    // Scale position based on distance from mean
                    let position_multiplier = (distance_to_mean / 0.3).min(2.0);
                    let position_size = 0.001 * position_multiplier;

                    opportunities.push(Opportunity {
                        id: format!("mean_rev_buy_{}", ts),
                        signal: Signal::Buy {
                            symbol: symbol.clone(),
                            price: Some(price * 1.0001),
                            quantity: position_size.min(0.01).max(0.0001),
                        },
                        score: ((35.0 - rsi) / 35.0 * 0.5 + distance_to_mean / 2.0).min(0.90),
                        risk_score: (self.recent_volatility / 5.0).min(0.6),
                        reason: format!("Oversold: RSI {:.1}, {:.2}% below mean", rsi, distance_to_mean),
                        timestamp: ts,
                    });

                    self.last_signal_time = ts;
                }

                // Overbought + RSI confirmation -> Sell
                if price > upper_band && rsi > 60.0 {
                    opportunities.push(Opportunity {
                        id: format!("mean_rev_sell_{}", ts),
                        signal: Signal::Sell {
                            symbol: symbol.clone(),
                            price: Some(price * 0.9999),
                            quantity: 0.001,
                        },
                        score: ((rsi - 60.0) / 40.0 * 0.5 + distance_to_mean / 2.0).min(0.85),
                        risk_score: 0.4,
                        reason: format!("Overbought: RSI {:.1}, {:.2}% above mean", rsi, distance_to_mean),
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
            write_guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, self.recent_volatility, ts);
        }

        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opportunities
    }
}
