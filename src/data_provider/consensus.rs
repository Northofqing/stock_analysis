//! Registered business rules: BR-119, BR-164.
//! Domain model for seller-research consensus.
//!
//! Acquisition is owned by `crate::data_gateway::ConsensusDataGateway`.  This
//! module intentionally contains no provider URL, transport or wire parser.

use std::collections::HashMap;

/// Seller-consensus summary derived from a complete, admitted research batch.
#[derive(Debug, Clone, Default)]
pub struct ConsensusData {
    /// Number of reports in the admitted 180-day window.
    pub report_count: usize,
    /// Number of distinct broker organizations.
    pub broker_count: usize,
    /// Average current-fiscal-year EPS estimate.
    pub eps_this_year_avg: Option<f64>,
    /// Average next-fiscal-year EPS estimate.
    pub eps_next_year_avg: Option<f64>,
    /// Average second-next-fiscal-year EPS estimate.
    pub eps_next2_year_avg: Option<f64>,
    /// Rating distribution, retaining the provider labels.
    pub rating_distribution: HashMap<String, u32>,
    /// Optional provider target-price aggregates.
    pub target_price_high_avg: Option<f64>,
    pub target_price_low_avg: Option<f64>,
    /// Most recent admitted report date (`YYYY-MM-DD`).
    pub latest_report_date: Option<String>,
    /// Three most recent admitted reports.
    pub recent_reports: Vec<RecentReport>,
}

#[derive(Debug, Clone)]
pub struct RecentReport {
    pub title: String,
    pub org_name: String,
    pub publish_date: String,
    pub rating: String,
}

impl ConsensusData {
    /// Percentage of admitted ratings labelled buy/add/recommend.
    pub fn bullish_ratio(&self) -> Option<f64> {
        let total: u32 = self.rating_distribution.values().sum();
        if total == 0 {
            return None;
        }
        let bull: u32 = self
            .rating_distribution
            .iter()
            .filter(|(label, _)| {
                label.contains("买入") || label.contains("增持") || label.contains("推荐")
            })
            .map(|(_, count)| *count)
            .sum();
        Some(bull as f64 / total as f64 * 100.0)
    }

    /// Relative upside to the provider's high target-price aggregate.
    pub fn upside_pct(&self, current_price: f64) -> Option<f64> {
        if current_price <= 0.0 {
            return None;
        }
        let high = self.target_price_high_avg?;
        Some((high - current_price) / current_price * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bullish_ratio_and_optional_target_price_remain_explicit() {
        let consensus = ConsensusData {
            rating_distribution: HashMap::from([
                ("买入".to_string(), 2),
                ("增持".to_string(), 1),
                ("中性".to_string(), 1),
            ]),
            target_price_high_avg: Some(15.0),
            ..ConsensusData::default()
        };
        assert_eq!(consensus.bullish_ratio(), Some(75.0));
        assert_eq!(consensus.upside_pct(10.0), Some(50.0));
        assert_eq!(consensus.upside_pct(0.0), None);

        let unavailable = ConsensusData::default();
        assert_eq!(unavailable.bullish_ratio(), None);
        assert_eq!(unavailable.upside_pct(10.0), None);
    }
}
