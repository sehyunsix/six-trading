use async_trait::async_trait;
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use serde::{Deserialize, Serialize};
use crate::web::SharedState;

pub mod factory;
pub mod logger;
pub mod mean_reversion;
pub mod momentum_breakout;
pub mod adaptive_mean_reversion;
pub mod vwap_strategy;
pub mod scalper_strategy;
pub mod breakout_range;
pub mod macd_crossover;
pub mod grid_trading;
pub mod risk;
pub mod rsi_strategy;
pub mod trend_follower;
pub mod dca_strategy;
pub mod volatility_breakout;
pub mod swing_trader;
pub mod martingale;
pub mod parabolic_sar;
pub mod stochastic_oscillator;
pub mod bb_squeeze;
pub mod chaikin_money_flow;
pub mod trix_strategy;
pub mod donchian_channels;
pub mod hull_ma;
pub mod fibonacci_reversion;
pub mod ichimoku_cloud;
pub mod heikin_ashi;
pub mod buy_hold;

pub use logger::PaperTrader;
pub use mean_reversion::MeanReversionStrategy;
pub use momentum_breakout::MomentumBreakout;
pub use adaptive_mean_reversion::AdaptiveMeanReversion;
pub use vwap_strategy::VWAPStrategy;
pub use scalper_strategy::ScalperStrategy;
pub use breakout_range::BreakoutRangeStrategy;
pub use macd_crossover::MACDCrossover;
pub use grid_trading::GridTrading;
pub use rsi_strategy::RSIStrategy;
pub use trend_follower::TrendFollower;
pub use dca_strategy::DCAStrategy;
pub use volatility_breakout::VolatilityBreakout;
pub use swing_trader::SwingTrader;
pub use martingale::MartingaleStrategy;
pub use parabolic_sar::ParabolicSAR;
pub use stochastic_oscillator::StochasticOscillator;
pub use bb_squeeze::BBSqueeze;
pub use chaikin_money_flow::ChaikinMoneyFlow;
pub use trix_strategy::TRIXStrategy;
pub use donchian_channels::DonchianChannels;
pub use hull_ma::HullMA;
pub use fibonacci_reversion::FibonacciReversion;
pub use ichimoku_cloud::IchimokuCloud;
pub use heikin_ashi::HeikinAshiTrend;
pub use buy_hold::BuyAndHold;
pub use risk::RiskManager;
pub use factory::StrategyFactory;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Signal {
    Buy {
        symbol: String,
        price: Option<f64>,
        quantity: f64,
    },
    Sell {
        symbol: String,
        price: Option<f64>,
        quantity: f64,
    },
    Cancel {
        symbol: String,
        order_id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: String,
    pub signal: Signal,
    pub score: f64,         // 0.0 to 1.0
    pub risk_score: f64,    // 0.0 to 1.0
    pub reason: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskReport {
    pub total_risk: f64,
    pub leverage_risk: f64,
    pub drawdown_warning: bool,
    pub recommended_max_size: f64,
}

#[async_trait]
pub trait TradingStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn get_features(&self) -> Vec<(String, String)>;
    async fn process_trade(&mut self, trade: TradeEvent, state: SharedState) -> Vec<Opportunity>;
    async fn process_aggr_trade(&mut self, trade: AggrTradesEvent, state: SharedState) -> Vec<Opportunity>;
    async fn process_orderbook(&mut self, orderbook: OrderBook, state: SharedState) -> Vec<Opportunity>;
}
