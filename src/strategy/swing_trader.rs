use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use std::time::Instant;

/// Swing trading strategy - captures larger moves
pub struct SwingTrader {
    prices: Vec<f64>,
    trade_count: u64,
    last_spread: f64,
    position: i8,  // -1 short, 0 flat, 1 long
    entry_price: f64,
}

impl SwingTrader {
    pub fn new() -> Self {
        Self {
            prices: Vec::with_capacity(100),
            trade_count: 0,
            last_spread: 0.0,
            position: 0,
            entry_price: 0.0,
        }
    }
    
    fn get_momentum(&self) -> f64 {
        if self.prices.len() < 20 { return 0.0; }
        let now = self.prices[self.prices.len() - 1];
        let old = self.prices[self.prices.len() - 20];
        (now - old) / old * 100.0
    }
}

#[async_trait]
impl TradingStrategy for SwingTrader {
    fn name(&self) -> &str { "SwingTrader" }
    
    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Momentum".to_string(), format!("{:.2}%", self.get_momentum())),
            ("Position".to_string(), match self.position { 1 => "Long", -1 => "Short", _ => "Flat" }.to_string()),
            ("PnL (Active)".to_string(), if self.position != 0 { format!("{:.2}%", 0.0) } else { "N/A".to_string() }), // PnL is dynamic, maybe add it to state?
        ]
    }
    
    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse().unwrap_or(0.0);
        let qty = trade.qty.parse().unwrap_or(0.0);
        self.handle_trade(trade.symbol, price, qty, trade.event_time, state).await
    }
    
    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse().unwrap_or(0.0);
        let qty = trade.qty.parse().unwrap_or(0.0);
        self.handle_trade(trade.symbol, price, qty, trade.event_time, state).await
    }
    
    async fn process_orderbook(&mut self, ob: OrderBook, _state: SharedState) -> Vec<Opportunity> {
        if !ob.bids.is_empty() && !ob.asks.is_empty() {
            self.last_spread = ob.asks[0].price - ob.bids[0].price;
        }
        Vec::new()
    }
}

impl SwingTrader {
    async fn handle_trade(&mut self, symbol: String, price: f64, qty: f64, ts: u64, state: SharedState) -> Vec<Opportunity> {
        let start = Instant::now();
        self.trade_count += 1;
        self.prices.push(price);
        if self.prices.len() > 50 { self.prices.remove(0); }
        
        {
            let mut guard = state.write().await;
            if guard.state_machine.get_state() == SystemState::Booting {
                guard.state_machine.transition_to(SystemState::Accumulating);
            } else if guard.state_machine.get_state() == SystemState::Accumulating && guard.state_machine.is_stable() {
                guard.state_machine.transition_to(SystemState::Trading);
            }
        }
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading {
            let momentum = self.get_momentum();
            
            // Enter long on strong upward momentum
            if self.position == 0 && momentum > 0.1 {
                self.position = 1;
                self.entry_price = price;
                opps.push(Opportunity {
                    id: format!("swing_buy_{}", self.trade_count),
                    signal: Signal::Buy { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                    score: 0.75,
                    risk_score: 0.35,
                    reason: format!("Strong momentum: +{:.2}%", momentum),
                    timestamp: ts,
                });
            }
            
            // Exit on reversal or profit target
            if self.position == 1 {
                let pnl_pct = (price - self.entry_price) / self.entry_price * 100.0;
                if momentum < -0.2 || pnl_pct > 1.0 || pnl_pct < -0.5 {
                    self.position = 0;
                    opps.push(Opportunity {
                        id: format!("swing_sell_{}", self.trade_count),
                        signal: Signal::Sell { symbol: symbol.clone(), price: Some(price), quantity: 0.001 },
                        score: 0.75,
                        risk_score: 0.3,
                        reason: format!("Exit: PnL={:.2}%, mom={:.2}%", pnl_pct, momentum),
                        timestamp: ts,
                    });
                }
            }
        }
        
        {
            let mut guard = state.write().await;
            let action = opps.first().map(|o| match &o.signal {
                Signal::Buy { .. } => "Buy".to_string(),
                Signal::Sell { .. } => "Sell".to_string(),
                _ => "Hold".to_string(),
            });
            let strat_lat = guard.metrics.get_strategy_stats().p50;
            let exec_lat = guard.metrics.get_execution_stats().p50;
            guard.push_data_point_at(price, qty, action, strat_lat, exec_lat, self.last_spread, ts);
        }
        
        state.read().await.metrics.record_strategy_latency(start.elapsed());
        opps
    }
}
