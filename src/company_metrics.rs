//! BR-119/BR-120 company-comparison domain values consumed by scoring.
//!
//! These structures contain only already-verified downstream values. Data
//! acquisition belongs to `crate::data_gateway`; unsupported fields stay
//! absent instead of being synthesized by a provider-shaped fallback.

#[derive(Debug, Clone, Default)]
pub struct IndustryBenchmark {
    pub industry_name: String,
    pub board_code: String,
    pub peer_count: usize,
    pub stock_pe: Option<f64>,
    pub stock_pb: Option<f64>,
    pub stock_roe: Option<f64>,
    pub stock_growth: Option<f64>,
    pub median_pe: Option<f64>,
    pub median_pb: Option<f64>,
    pub median_roe: Option<f64>,
    pub median_growth: Option<f64>,
    pub pe_percentile: Option<f64>,
    pub pb_percentile: Option<f64>,
    pub roe_percentile: Option<f64>,
    pub growth_percentile: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ValuationHistory {
    pub current_pe: Option<f64>,
    pub current_pb: Option<f64>,
    pub pe_percentile: Option<f64>,
    pub pb_percentile: Option<f64>,
    pub pe_min: Option<f64>,
    pub pe_max: Option<f64>,
    pub pe_median: Option<f64>,
    pub pb_min: Option<f64>,
    pub pb_max: Option<f64>,
    pub pb_median: Option<f64>,
    pub sample_days: usize,
    pub oldest_date: Option<String>,
    pub newest_date: Option<String>,
}
