use super::{Signal, TradingStrategy, Opportunity};
use crate::web::SharedState;
use crate::state_machine::SystemState;
use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};

/// Grid Trading Strategy - Buy low, sell high with price grids
pub struct GridTrading {
    grid_size: f64,      // % between grid levels
    grid_levels: Vec<f64>,
    base_price: f64,
    positions: Vec<(f64, f64)>,  // (entry_price, qty)
    last_signal_time: u64,
}

impl GridTrading {
    pub fn new() -> Self {
        Self {
            grid_size: 0.05,  // 0.05% grid spacing
            grid_levels: Vec::new(),
            base_price: 0.0,
            positions: Vec::new(),
            last_signal_time: 0,
        }
    }

    fn setup_grid(&mut self, price: f64) {
        self.base_price = price;
        self.grid_levels.clear();
        for i in -5..=5 {
            self.grid_levels.push(price * (1.0 + (i as f64) * self.grid_size / 100.0));
        }
    }

    fn find_grid_level(&self, price: f64) -> Option<(usize, f64)> {
        for (i, &level) in self.grid_levels.iter().enumerate() {
            if (price - level).abs() / level < 0.0005 {
                return Some((i, level));
            }
        }
        None
    }
}

#[async_trait]
impl TradingStrategy for GridTrading {
    fn name(&self) -> &str { "GridTrading" }

    fn get_features(&self) -> Vec<(String, String)> {
        vec![
            ("Base Price".to_string(), format!("{:.2}", self.base_price)),
            ("Positions".to_string(), self.positions.len().to_string()),
            ("Grid Size".to_string(), format!("{:.1}%", self.grid_size)),
        ]
    }

    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity> {
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        let qty = trade.qty.parse::<f64>().unwrap_or(0.0);
        
        if self.base_price == 0.0 { self.setup_grid(price); }
        
        let mut opps = Vec::new();
        let current_state = state.read().await.state_machine.get_state();
        
        if current_state == SystemState::Trading && trade.event_time - self.last_signal_time > 10000 {
            if let Some((level_idx, level_price)) = self.find_grid_level(price) {
                let mid_level = self.grid_levels.len() / 2;
                
                if level_idx < mid_level && self.positions.len() < 5 {
                    // Below base - accumulate
                    opps.push(Opportunity {
                        id: format!("grid_buy_{}", trade.event_time),
                        signal: Signal::Buy { symbol: trade.symbol.clone(), price: Some(price), quantity: 0.0005 },
                        score: 0.65,
                        risk_score: 0.3,
                        reason: format!("Grid buy at level {} ({:.2})", level_idx, level_price),
                        timestamp: trade.event_time,
                    });
                    self.positions.push((price, 0.0005));
                    self.last_signal_time = trade.event_time;
                } else if level_idx > mid_level && !self.positions.is_empty() {
                    // Above base - take profit
                    if let Some((entry, entry_qty)) = self.positions.pop() {
                        let pnl_pct = (price - entry) / entry * 100.0;
                        opps.push(Opportunity {
                            id: format!("grid_sell_{}", trade.event_time),
                            signal: Signal::Sell { symbol: trade.symbol.clone(), price: Some(price), quantity: entry_qty },
                            score: 0.7,
                            risk_score: 0.25,
                            reason: format!("Grid sell +{:.2}% profit", pnl_pct),
                            timestamp: trade.event_time,
                        });
                        self.last_signal_time = trade.event_time;
                    }
                }
            }
        }
        
        { let mut w = state.write().await; w.push_data_point_at(price, qty, opps.first().map(|o| match &o.signal { Signal::Buy{..} => "Buy", Signal::Sell{..} => "Sell", _ => "Cancel" }.to_string()), 0, 0, 0.0, trade.event_time); }
        opps
    }

    async fn process_aggr_trade(&mut self, _: AggrTradesEvent, _: SharedState) -> Vec<Opportunity> { Vec::new() }
    async fn process_orderbook(&mut self, _: OrderBook, _: SharedState) -> Vec<Opportunity> { Vec::new() }
}
