//! BR-156: deterministic hard admission gate for formal shadow candidates.

use crate::selection::features::RawSelectionFeatures;
use serde::{Deserialize, Serialize};

pub const ADMISSION_VERSION: &str = "admission-v1";
pub const MAX_PRICE_ABOVE_MA20: f64 = 0.15;
pub const MAX_FIVE_DAY_RETURN: f64 = 0.20;
pub const MIN_SETTLED_VOLUME_RATIO: f64 = 1.0;
pub const MIN_INTRADAY_VOLUME_PACE: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionEvaluationWindow {
    Intraday,
    PostClose,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdmissionFailure {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdmissionRejection {
    pub admission_version: String,
    pub failures: Vec<AdmissionFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AdmissionDecision {
    Admitted { admission_version: String },
    Rejected(AdmissionRejection),
}

impl AdmissionDecision {
    pub fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted { .. })
    }

    pub fn reason_codes(&self) -> Vec<&str> {
        match self {
            Self::Admitted { .. } => Vec::new(),
            Self::Rejected(rejection) => rejection
                .failures
                .iter()
                .map(|failure| failure.code.as_str())
                .collect(),
        }
    }
}

pub fn evaluate_admission(
    window: SelectionEvaluationWindow,
    features: &RawSelectionFeatures,
) -> AdmissionDecision {
    let mut failures = Vec::new();

    let ma5 = required_feature("ma5", features.ma5, &mut failures);
    let ma10 = required_feature("ma10", features.ma10, &mut failures);
    let ma20 = required_feature("ma20", features.ma20, &mut failures);
    let five_day_return =
        required_feature("five_day_return", features.five_day_return, &mut failures);
    let volume_vs_5d = required_feature("volume_vs_5d", features.volume_vs_5d, &mut failures);
    let volume_vs_20d = required_feature("volume_vs_20d", features.volume_vs_20d, &mut failures);
    let price_vs_ma5 = required_feature("price_vs_ma5", features.price_vs_ma5, &mut failures);
    let _price_vs_ma10 = required_feature("price_vs_ma10", features.price_vs_ma10, &mut failures);
    let price_vs_ma20 = required_feature("price_vs_ma20", features.price_vs_ma20, &mut failures);

    let intraday_volume_pace = match window {
        SelectionEvaluationWindow::Intraday => required_feature(
            "intraday_volume_pace",
            features.intraday_volume_pace,
            &mut failures,
        ),
        SelectionEvaluationWindow::PostClose => features.intraday_volume_pace.and_then(|value| {
            optional_finite_feature("intraday_volume_pace", value, &mut failures)
        }),
    };

    if let (Some(ma5), Some(ma10), Some(ma20)) = (ma5, ma10, ma20) {
        if ma5 <= 0.0 || ma10 <= 0.0 || ma20 <= 0.0 {
            failures.push(failure(
                "moving_average_nonpositive",
                format!("moving averages must be positive: ma5={ma5}, ma10={ma10}, ma20={ma20}"),
            ));
        } else if ma5 < ma10 || ma10 < ma20 {
            failures.push(failure(
                "trend_alignment_failed",
                format!(
                    "admission-v1 requires ma5>=ma10>=ma20: ma5={ma5}, ma10={ma10}, ma20={ma20}"
                ),
            ));
        }
    }

    if let Some(value) = price_vs_ma5 {
        if value < 0.0 {
            failures.push(failure(
                "price_below_ma5",
                format!("price_vs_ma5={value} is below the inclusive lower bound 0"),
            ));
        }
    }

    if let Some(value) = price_vs_ma20 {
        if !(0.0..=MAX_PRICE_ABOVE_MA20).contains(&value) {
            failures.push(failure(
                "price_ma20_distance_out_of_range",
                format!("price_vs_ma20={value} is outside [0,{MAX_PRICE_ABOVE_MA20}]"),
            ));
        }
    }

    if let Some(value) = five_day_return {
        if !(0.0..=MAX_FIVE_DAY_RETURN).contains(&value) {
            failures.push(failure(
                "five_day_return_out_of_range",
                format!("five_day_return={value} is outside [0,{MAX_FIVE_DAY_RETURN}]"),
            ));
        }
    }

    if let (Some(volume_vs_5d), Some(volume_vs_20d)) = (volume_vs_5d, volume_vs_20d) {
        if volume_vs_5d < MIN_SETTLED_VOLUME_RATIO || volume_vs_20d < MIN_SETTLED_VOLUME_RATIO {
            failures.push(failure(
                "settled_volume_confirmation_failed",
                format!(
                    "admission-v1 requires both settled volume ratios >= {MIN_SETTLED_VOLUME_RATIO}: volume_vs_5d={volume_vs_5d}, volume_vs_20d={volume_vs_20d}"
                ),
            ));
        }
    }

    if window == SelectionEvaluationWindow::Intraday {
        if let Some(value) = intraday_volume_pace {
            if value < MIN_INTRADAY_VOLUME_PACE {
                failures.push(failure(
                    "intraday_volume_confirmation_failed",
                    format!("intraday_volume_pace={value} is below {MIN_INTRADAY_VOLUME_PACE}"),
                ));
            }
        }
    }

    if failures.is_empty() {
        AdmissionDecision::Admitted {
            admission_version: ADMISSION_VERSION.to_owned(),
        }
    } else {
        AdmissionDecision::Rejected(AdmissionRejection {
            admission_version: ADMISSION_VERSION.to_owned(),
            failures,
        })
    }
}

fn required_feature(
    name: &'static str,
    value: Option<f64>,
    failures: &mut Vec<AdmissionFailure>,
) -> Option<f64> {
    match value {
        None => {
            failures.push(failure(
                format!("{name}_missing"),
                format!("admission-v1 requires feature {name}"),
            ));
            None
        }
        Some(value) => optional_finite_feature(name, value, failures),
    }
}

fn optional_finite_feature(
    name: &'static str,
    value: f64,
    failures: &mut Vec<AdmissionFailure>,
) -> Option<f64> {
    if value.is_finite() {
        Some(value)
    } else {
        failures.push(failure(
            format!("{name}_nonfinite"),
            format!("feature {name} must be finite, got {value}"),
        ));
        None
    }
}

fn failure(code: impl Into<String>, detail: impl Into<String>) -> AdmissionFailure {
    AdmissionFailure {
        code: code.into(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strong_features() -> RawSelectionFeatures {
        RawSelectionFeatures {
            ma5: Some(10.8),
            ma10: Some(10.5),
            ma20: Some(10.0),
            five_day_return: Some(0.08),
            volume_vs_5d: Some(1.3),
            volume_vs_20d: Some(1.2),
            intraday_volume_pace: Some(1.1),
            price_vs_ma5: Some(0.02),
            price_vs_ma10: Some(0.05),
            price_vs_ma20: Some(0.10),
        }
    }

    #[test]
    fn admits_only_complete_trend_and_volume_confirmed_security() {
        assert_eq!(
            evaluate_admission(SelectionEvaluationWindow::Intraday, &strong_features()),
            AdmissionDecision::Admitted {
                admission_version: ADMISSION_VERSION.to_owned()
            }
        );
    }

    #[test]
    fn weak_security_returns_all_applicable_failures_in_stable_order() {
        let mut features = strong_features();
        features.ma5 = Some(9.8);
        features.ma10 = Some(10.0);
        features.ma20 = Some(10.2);
        features.five_day_return = Some(-0.03);
        features.volume_vs_5d = Some(0.8);
        features.volume_vs_20d = Some(0.9);
        features.price_vs_ma5 = Some(-0.02);
        features.price_vs_ma20 = Some(-0.05);

        let decision = evaluate_admission(SelectionEvaluationWindow::PostClose, &features);
        assert_eq!(
            decision.reason_codes(),
            [
                "trend_alignment_failed",
                "price_below_ma5",
                "price_ma20_distance_out_of_range",
                "five_day_return_out_of_range",
                "settled_volume_confirmation_failed",
            ]
        );
    }

    #[test]
    fn overextended_security_is_rejected_instead_of_entering_formal_batch() {
        let mut features = strong_features();
        features.five_day_return = Some(0.200_001);
        features.price_vs_ma20 = Some(0.150_001);

        let decision = evaluate_admission(SelectionEvaluationWindow::PostClose, &features);
        assert_eq!(
            decision.reason_codes(),
            [
                "price_ma20_distance_out_of_range",
                "five_day_return_out_of_range",
            ]
        );
    }

    #[test]
    fn intraday_requires_real_same_slot_volume_confirmation() {
        let mut features = strong_features();
        features.intraday_volume_pace = None;
        assert_eq!(
            evaluate_admission(SelectionEvaluationWindow::Intraday, &features).reason_codes(),
            ["intraday_volume_pace_missing"]
        );

        features.intraday_volume_pace = Some(0.99);
        assert_eq!(
            evaluate_admission(SelectionEvaluationWindow::Intraday, &features).reason_codes(),
            ["intraday_volume_confirmation_failed"]
        );
    }

    #[test]
    fn post_close_does_not_invent_or_require_intraday_pace() {
        let mut features = strong_features();
        features.intraday_volume_pace = None;
        assert!(matches!(
            evaluate_admission(SelectionEvaluationWindow::PostClose, &features),
            AdmissionDecision::Admitted { .. }
        ));
    }

    #[test]
    fn missing_and_nonfinite_features_are_explicit_rejections() {
        let mut features = strong_features();
        features.ma10 = None;
        features.volume_vs_20d = Some(f64::NAN);
        features.price_vs_ma5 = Some(f64::INFINITY);

        assert_eq!(
            evaluate_admission(SelectionEvaluationWindow::PostClose, &features).reason_codes(),
            [
                "ma10_missing",
                "volume_vs_20d_nonfinite",
                "price_vs_ma5_nonfinite",
            ]
        );
    }

    #[test]
    fn documented_boundaries_are_inclusive() {
        let features = RawSelectionFeatures {
            ma5: Some(10.0),
            ma10: Some(10.0),
            ma20: Some(10.0),
            five_day_return: Some(0.20),
            volume_vs_5d: Some(1.0),
            volume_vs_20d: Some(1.0),
            intraday_volume_pace: Some(1.0),
            price_vs_ma5: Some(0.0),
            price_vs_ma10: Some(0.0),
            price_vs_ma20: Some(0.15),
        };
        assert!(matches!(
            evaluate_admission(SelectionEvaluationWindow::Intraday, &features),
            AdmissionDecision::Admitted { .. }
        ));
    }
}
