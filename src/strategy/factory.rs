use super::{
    TradingStrategy, MeanReversionStrategy, PaperTrader, MomentumBreakout, 
    AdaptiveMeanReversion, VWAPStrategy, ScalperStrategy, BreakoutRangeStrategy,
    MACDCrossover, GridTrading,
    RSIStrategy, TrendFollower, DCAStrategy,
    VolatilityBreakout, SwingTrader, MartingaleStrategy,
    ParabolicSAR, StochasticOscillator, BBSqueeze, ChaikinMoneyFlow,
    TRIXStrategy, DonchianChannels, HullMA, FibonacciReversion,
    IchimokuCloud, HeikinAshiTrend, BuyAndHold
};

pub struct StrategyFactory;

impl StrategyFactory {
    pub fn create_strategy(name: &str) -> Option<Box<dyn TradingStrategy>> {
        match name {
            "MeanReversion" => Some(Box::new(MeanReversionStrategy::new())),
            "PaperTrader" => Some(Box::new(PaperTrader::new())),
            "MomentumBreakout" => Some(Box::new(MomentumBreakout::new())),
            "AdaptiveMeanReversion" => Some(Box::new(AdaptiveMeanReversion::new())),
            "VWAPStrategy" => Some(Box::new(VWAPStrategy::new())),
            "ScalperStrategy" => Some(Box::new(ScalperStrategy::new())),
            "BreakoutRange" => Some(Box::new(BreakoutRangeStrategy::new())),
            "MACDCrossover" => Some(Box::new(MACDCrossover::new())),
            "GridTrading" => Some(Box::new(GridTrading::new())),
            "RSIStrategy" => Some(Box::new(RSIStrategy::new())),
            "TrendFollower" => Some(Box::new(TrendFollower::new())),
            "DCAStrategy" => Some(Box::new(DCAStrategy::new())),
            "VolatilityBreakout" => Some(Box::new(VolatilityBreakout::new())),
            "SwingTrader" => Some(Box::new(SwingTrader::new())),
            "Martingale" => Some(Box::new(MartingaleStrategy::new())),
            "ParabolicSAR" => Some(Box::new(ParabolicSAR::new())),
            "StochasticOscillator" => Some(Box::new(StochasticOscillator::new())),
            "BBSqueeze" => Some(Box::new(BBSqueeze::new())),
            "ChaikinMoneyFlow" => Some(Box::new(ChaikinMoneyFlow::new())),
            "TRIXStrategy" => Some(Box::new(TRIXStrategy::new())),
            "DonchianChannels" => Some(Box::new(DonchianChannels::new())),
            "HullMA" => Some(Box::new(HullMA::new())),
            "FibonacciReversion" => Some(Box::new(FibonacciReversion::new())),
            "IchimokuCloud" => Some(Box::new(IchimokuCloud::new())),
            "HeikinAshiTrend" => Some(Box::new(HeikinAshiTrend::new())),
            "BuyAndHold" => Some(Box::new(BuyAndHold::new())),
            _ => None,
        }
    }

    pub fn get_available_strategies() -> Vec<String> {
        vec![
            "MeanReversion".to_string(),
            "PaperTrader".to_string(),
            "MomentumBreakout".to_string(),
            "AdaptiveMeanReversion".to_string(),
            "VWAPStrategy".to_string(),
            "ScalperStrategy".to_string(),
            "BreakoutRange".to_string(),
            "MACDCrossover".to_string(),
            "GridTrading".to_string(),
            "RSIStrategy".to_string(),
            "TrendFollower".to_string(),
            "DCAStrategy".to_string(),
            "VolatilityBreakout".to_string(),
            "SwingTrader".to_string(),
            "Martingale".to_string(),
            "ParabolicSAR".to_string(),
            "StochasticOscillator".to_string(),
            "BBSqueeze".to_string(),
            "ChaikinMoneyFlow".to_string(),
            "TRIXStrategy".to_string(),
            "DonchianChannels".to_string(),
            "HullMA".to_string(),
            "FibonacciReversion".to_string(),
            "IchimokuCloud".to_string(),
            "HeikinAshiTrend".to_string(),
            "BuyAndHold".to_string(),
        ]
    }
}
