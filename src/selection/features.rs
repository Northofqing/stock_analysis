use crate::selection::quality::{validate_daily, QualityError, SelectionBar};
use serde::{Deserialize, Serialize};

pub const FEATURE_VERSION: &str = "raw-selection-v1";
const MIN_DAILY_BARS: usize = 21;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSelectionFeatures {
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub five_day_return: Option<f64>,
    pub volume_vs_5d: Option<f64>,
    pub volume_vs_20d: Option<f64>,
    pub intraday_volume_pace: Option<f64>,
    pub price_vs_ma5: Option<f64>,
    pub price_vs_ma10: Option<f64>,
    pub price_vs_ma20: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntradayVolumeEvidence {
    pub cumulative_volume: f64,
    /// Real cumulative volumes observed at the same completed five-minute slot.
    pub historical_same_slot_volumes: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct T0MarketEvidence {
    pub evaluation_price: f64,
    pub observed_volume: f64,
    pub latest_settled_market_date: chrono::NaiveDate,
    pub latest_settled_close: f64,
    pub latest_settled_volume: f64,
    pub prior_5d_average_volume: f64,
    pub prior_20d_average_volume: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureError {
    code: &'static str,
    message: String,
}

impl FeatureError {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FeatureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FeatureError {}

impl From<QualityError> for FeatureError {
    fn from(value: QualityError) -> Self {
        Self {
            code: value.code(),
            message: value.to_string(),
        }
    }
}

impl RawSelectionFeatures {
    pub fn with_intraday_volume_pace(
        mut self,
        evidence: &IntradayVolumeEvidence,
    ) -> Result<Self, FeatureError> {
        self.intraday_volume_pace = Some(compute_intraday_volume_pace(evidence)?);
        Ok(self)
    }
}

pub fn compute_daily_features(bars: &[SelectionBar]) -> Result<RawSelectionFeatures, FeatureError> {
    if bars.len() < MIN_DAILY_BARS {
        return Err(feature_error(
            "daily_feature_history_insufficient",
            format!(
                "daily feature history has {} bars; {} required",
                bars.len(),
                MIN_DAILY_BARS
            ),
        ));
    }
    let validated = validate_daily(bars)?;
    let bars = validated.bars();
    let count = bars.len();
    let latest = &bars[count - 1];

    let ma5 = close_mean(&bars[count - 5..]);
    let ma10 = close_mean(&bars[count - 10..]);
    let ma20 = close_mean(&bars[count - 20..]);
    let five_day_base = bars[count - 6].close;

    let prior_5d_volume = positive_mean(
        bars[count - 6..count - 1].iter().map(|bar| bar.volume),
        "volume_baseline_missing",
        "prior five-day volume baseline is missing",
    )?;
    let prior_20d_volume = positive_mean(
        bars[count - 21..count - 1].iter().map(|bar| bar.volume),
        "volume_baseline_missing",
        "prior twenty-day volume baseline is missing",
    )?;

    Ok(RawSelectionFeatures {
        ma5: Some(ma5),
        ma10: Some(ma10),
        ma20: Some(ma20),
        five_day_return: Some(latest.close / five_day_base - 1.0),
        volume_vs_5d: Some(latest.volume / prior_5d_volume),
        volume_vs_20d: Some(latest.volume / prior_20d_volume),
        intraday_volume_pace: None,
        price_vs_ma5: Some(latest.close / ma5 - 1.0),
        price_vs_ma10: Some(latest.close / ma10 - 1.0),
        price_vs_ma20: Some(latest.close / ma20 - 1.0),
    })
}

pub fn compute_intraday_volume_pace(
    evidence: &IntradayVolumeEvidence,
) -> Result<f64, FeatureError> {
    if !evidence.cumulative_volume.is_finite() || evidence.cumulative_volume < 0.0 {
        return Err(feature_error(
            "intraday_volume_invalid",
            format!(
                "intraday cumulative volume must be finite and nonnegative, got {}",
                evidence.cumulative_volume
            ),
        ));
    }
    let baseline = positive_mean(
        evidence.historical_same_slot_volumes.iter().copied(),
        "intraday_volume_baseline_missing",
        "historical same-slot volume baseline is missing",
    )?;
    Ok(evidence.cumulative_volume / baseline)
}

fn close_mean(bars: &[SelectionBar]) -> f64 {
    bars.iter().map(|bar| bar.close).sum::<f64>() / bars.len() as f64
}

fn positive_mean(
    values: impl Iterator<Item = f64>,
    code: &'static str,
    missing_message: &'static str,
) -> Result<f64, FeatureError> {
    let values = values.collect::<Vec<_>>();
    if values.is_empty()
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(feature_error(code, missing_message));
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return Err(feature_error(code, missing_message));
    }
    Ok(mean)
}

fn feature_error(code: &'static str, message: impl Into<String>) -> FeatureError {
    FeatureError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::quality::{PriceAdjustment, SelectionBar};
    use chrono::NaiveDate;

    fn linear_bars(count: usize) -> Vec<SelectionBar> {
        let mut date = NaiveDate::from_ymd_opt(2026, 6, 22).expect("valid test date");
        let mut bars = Vec::with_capacity(count);
        for index in 0..count {
            let close = index as f64 + 10.0;
            bars.push(SelectionBar {
                code: "TEST_CODE_000001".to_string(),
                market_date: date,
                open: close,
                high: close,
                low: close,
                close,
                volume: (index as f64 + 1.0) * 100.0,
                amount: close * (index as f64 + 1.0) * 100.0,
                settled: true,
                adjustment: PriceAdjustment::Unadjusted,
                reference_previous_close: None,
            });
            date = crate::calendar::next_trading_day(date);
        }
        bars
    }

    #[test]
    fn computes_raw_daily_features_from_twenty_one_settled_bars() {
        let features = compute_daily_features(&linear_bars(21)).expect("valid features");

        assert_eq!(features.ma5, Some(28.0));
        assert_eq!(features.ma10, Some(25.5));
        assert_eq!(features.ma20, Some(20.5));
        assert!(
            (features.five_day_return.expect("five-day return") - 5.0 / 25.0).abs() < f64::EPSILON
        );
        assert_eq!(features.price_vs_ma5, Some(30.0 / 28.0 - 1.0));
        assert!(features.intraday_volume_pace.is_none());
    }

    #[test]
    fn missing_volume_denominator_stays_missing_and_blocks_formal_feature_set() {
        let mut bars = linear_bars(21);
        for bar in &mut bars[..20] {
            bar.volume = 0.0;
        }

        let error = compute_daily_features(&bars).unwrap_err();
        assert_eq!(error.code(), "volume_baseline_missing");
    }

    #[test]
    fn requires_twenty_one_quality_valid_bars_and_never_emits_a_score() {
        let error = compute_daily_features(&linear_bars(20)).unwrap_err();
        assert_eq!(error.code(), "daily_feature_history_insufficient");

        let mut invalid = linear_bars(21);
        invalid[20].amount = f64::NAN;
        let error = compute_daily_features(&invalid).unwrap_err();
        assert_eq!(error.code(), "amount_nonfinite");
    }

    #[test]
    fn intraday_volume_pace_requires_a_real_positive_same_slot_baseline() {
        let features = compute_daily_features(&linear_bars(21)).expect("valid daily features");
        let completed = features
            .clone()
            .with_intraday_volume_pace(&IntradayVolumeEvidence {
                cumulative_volume: 1_200.0,
                historical_same_slot_volumes: vec![800.0, 1_000.0, 1_200.0],
            })
            .expect("valid intraday evidence");
        assert_eq!(completed.intraday_volume_pace, Some(1.2));

        let error = features
            .with_intraday_volume_pace(&IntradayVolumeEvidence {
                cumulative_volume: 1_200.0,
                historical_same_slot_volumes: Vec::new(),
            })
            .unwrap_err();
        assert_eq!(error.code(), "intraday_volume_baseline_missing");
    }
}
