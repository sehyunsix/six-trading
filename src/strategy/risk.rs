use super::{Opportunity, RiskReport};
use crate::web::AppState;

pub struct RiskManager;

impl RiskManager {
    pub fn analyze_opportunities(
        opportunities: &[Opportunity],
        _state: &AppState
    ) -> (Vec<Opportunity>, RiskReport) {
        // 1. Calculate general portfolio risk
        let total_risk = if opportunities.len() > 5 { 0.8 } else { 0.3 };
        let leverage_risk = 0.1; // Static for now
        let drawdown_warning = total_risk > 0.7;

        let mut processed_opps = opportunities.to_vec();
        
        // 2. Adjust individual risk scores based on state (mock logic)
        for opp in processed_opps.iter_mut() {
            if opp.score > 0.8 {
                opp.risk_score *= 0.8; // Lower perceived risk for high confidence
            }
            if drawdown_warning {
                opp.risk_score *= 1.5; // Increase risk if portfolio is stressed
            }
        }

        let report = RiskReport {
            total_risk,
            leverage_risk,
            drawdown_warning,
            recommended_max_size: 0.005,
        };

        (processed_opps, report)
    }

    pub fn select_best_trade(opportunities: &[Opportunity]) -> Option<super::Signal> {
        // Simple selection: highest score with risk_score < 0.5
        opportunities.iter()
            .filter(|o| o.risk_score < 0.5)
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap())
            .map(|o| o.signal.clone())
    }
}
