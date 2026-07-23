//! BR-157 read-only raw outcome reporting for visible shadow samples.

use crate::database::selection::{ReportFilter, VisibleSample};
use chrono::NaiveDate;
use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SelectionReportError {
    #[error("invalid selection report filter: {0}")]
    InvalidFilter(String),
    #[error("invalid visible selection sample {candidate_id}: {reason}")]
    InvalidSample {
        candidate_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawOutcomeReport {
    pub from_market_date: Option<NaiveDate>,
    pub to_market_date: Option<NaiveDate>,
    pub visible_sample_count: usize,
    pub groups: Vec<RawOutcomeGroup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawOutcomeGroup {
    pub provider: String,
    pub chain_id: String,
    pub relation_kind: String,
    pub feature_bucket: String,
    pub sample_count: usize,
    pub t0_close_count: usize,
    pub d1_settled_count: usize,
    pub missing_t0_count: usize,
    pub missing_d1_count: usize,
    pub close_return_median: Option<f64>,
    pub close_return_q25: Option<f64>,
    pub close_return_q75: Option<f64>,
    pub mfe_median: Option<f64>,
    pub mae_median: Option<f64>,
    pub volume_vs_t0_median: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    provider: String,
    chain_id: String,
    relation_kind: String,
    feature_bucket: String,
}

#[derive(Debug, Default)]
struct GroupAccumulator {
    sample_count: usize,
    t0_close_count: usize,
    d1_settled_count: usize,
    missing_t0_count: usize,
    missing_d1_count: usize,
    close_returns: Vec<f64>,
    mfes: Vec<f64>,
    maes: Vec<f64>,
    volume_vs_t0: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct FeatureProjection {
    relation: RelationProjection,
    features: FeatureValueProjection,
}

#[derive(Debug, Deserialize)]
struct RelationProjection {
    matched_by: String,
}

#[derive(Debug, Deserialize)]
struct FeatureValueProjection {
    price_vs_ma20: Option<f64>,
    volume_vs_5d: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct T0Projection {
    close_return: f64,
}

#[derive(Debug, Deserialize)]
struct D1Projection {
    close_return: f64,
    mfe: f64,
    mae: f64,
    volume_vs_t0: f64,
}

pub fn build_report(
    samples: &[VisibleSample],
    filter: &ReportFilter,
) -> Result<RawOutcomeReport, SelectionReportError> {
    validate_filter(filter)?;
    let mut groups = BTreeMap::<GroupKey, GroupAccumulator>::new();
    let mut visible_sample_count = 0usize;

    for sample in samples
        .iter()
        .filter(|sample| matches_filter(sample, filter))
    {
        if visible_sample_count == filter.limit {
            break;
        }
        validate_identity(sample)?;
        let feature = parse_feature(sample)?;
        let key = GroupKey {
            provider: sample.provider.clone(),
            chain_id: sample.chain_id.clone(),
            relation_kind: feature.relation.matched_by,
            feature_bucket: feature_bucket(
                sample,
                feature.features.price_vs_ma20,
                feature.features.volume_vs_5d,
            )?,
        };
        let group = groups.entry(key).or_default();
        group.sample_count += 1;
        visible_sample_count += 1;

        match sample.t0_outcome_payload_json.as_deref() {
            Some(payload) => {
                let t0 = parse_t0(sample, payload)?;
                require_finite(sample, "T0 close_return", t0.close_return)?;
                group.t0_close_count += 1;
            }
            None => group.missing_t0_count += 1,
        }

        match sample.d1_outcome_payload_json.as_deref() {
            Some(payload) => {
                if sample.t0_outcome_payload_json.is_none() {
                    return Err(invalid_sample(
                        sample,
                        "D+1 outcome exists without immutable T0 close outcome",
                    ));
                }
                let d1 = parse_d1(sample, payload)?;
                for (field, value) in [
                    ("D+1 close_return", d1.close_return),
                    ("D+1 MFE", d1.mfe),
                    ("D+1 MAE", d1.mae),
                    ("D+1 volume_vs_t0", d1.volume_vs_t0),
                ] {
                    require_finite(sample, field, value)?;
                }
                if d1.volume_vs_t0 <= 0.0 {
                    return Err(invalid_sample(
                        sample,
                        format!("D+1 volume_vs_t0 must be positive, got {}", d1.volume_vs_t0),
                    ));
                }
                group.d1_settled_count += 1;
                group.close_returns.push(d1.close_return);
                group.mfes.push(d1.mfe);
                group.maes.push(d1.mae);
                group.volume_vs_t0.push(d1.volume_vs_t0);
            }
            None => group.missing_d1_count += 1,
        }
    }

    let groups = groups
        .into_iter()
        .map(|(key, values)| RawOutcomeGroup {
            provider: key.provider,
            chain_id: key.chain_id,
            relation_kind: key.relation_kind,
            feature_bucket: key.feature_bucket,
            sample_count: values.sample_count,
            t0_close_count: values.t0_close_count,
            d1_settled_count: values.d1_settled_count,
            missing_t0_count: values.missing_t0_count,
            missing_d1_count: values.missing_d1_count,
            close_return_median: quantile(&values.close_returns, 0.5),
            close_return_q25: quantile(&values.close_returns, 0.25),
            close_return_q75: quantile(&values.close_returns, 0.75),
            mfe_median: quantile(&values.mfes, 0.5),
            mae_median: quantile(&values.maes, 0.5),
            volume_vs_t0_median: quantile(&values.volume_vs_t0, 0.5),
        })
        .collect();

    Ok(RawOutcomeReport {
        from_market_date: filter.from_market_date,
        to_market_date: filter.to_market_date,
        visible_sample_count,
        groups,
    })
}

pub fn render_text(report: &RawOutcomeReport) -> String {
    let mut rendered = format!(
        "事件选股原始影子结果\n可见样本数: {}\n区间: {} 至 {}\n",
        report.visible_sample_count,
        optional_date(report.from_market_date),
        optional_date(report.to_market_date)
    );
    if report.groups.is_empty() {
        rendered.push_str("无符合条件的可见样本\n");
        return rendered;
    }
    for (index, group) in report.groups.iter().enumerate() {
        rendered.push_str(&format!(
            "\n{}. provider={} | chain={} | relation={} | bucket={}\n",
            index + 1,
            group.provider,
            group.chain_id,
            group.relation_kind,
            group.feature_bucket
        ));
        rendered.push_str(&format!(
            "   样本数={} | T0={} | D1={} | T0缺失={} | D1缺失={}\n",
            group.sample_count,
            group.t0_close_count,
            group.d1_settled_count,
            group.missing_t0_count,
            group.missing_d1_count
        ));
        rendered.push_str(&format!(
            "   D1收益中位数={} | Q25={} | Q75={} | MFE中位数={} | MAE中位数={} | D1/T0量能中位数={}\n",
            optional_percent(group.close_return_median),
            optional_percent(group.close_return_q25),
            optional_percent(group.close_return_q75),
            optional_percent(group.mfe_median),
            optional_percent(group.mae_median),
            optional_ratio(group.volume_vs_t0_median),
        ));
    }
    rendered
}

fn validate_filter(filter: &ReportFilter) -> Result<(), SelectionReportError> {
    if filter.limit == 0 {
        return Err(SelectionReportError::InvalidFilter(
            "limit must be greater than zero".to_owned(),
        ));
    }
    if filter
        .from_market_date
        .zip(filter.to_market_date)
        .is_some_and(|(from, to)| from > to)
    {
        return Err(SelectionReportError::InvalidFilter(
            "from_market_date must not be after to_market_date".to_owned(),
        ));
    }
    for (field, value) in [
        ("provider", filter.provider.as_deref()),
        ("chain_id", filter.chain_id.as_deref()),
        ("stock_code", filter.stock_code.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(SelectionReportError::InvalidFilter(format!(
                "{field} must not be blank"
            )));
        }
    }
    Ok(())
}

fn matches_filter(sample: &VisibleSample, filter: &ReportFilter) -> bool {
    filter
        .from_market_date
        .is_none_or(|from| sample.evaluation_market_date >= from)
        && filter
            .to_market_date
            .is_none_or(|to| sample.evaluation_market_date <= to)
        && filter
            .provider
            .as_deref()
            .is_none_or(|provider| sample.provider == provider)
        && filter
            .chain_id
            .as_deref()
            .is_none_or(|chain_id| sample.chain_id == chain_id)
        && filter
            .stock_code
            .as_deref()
            .is_none_or(|stock_code| sample.stock_code == stock_code)
}

fn validate_identity(sample: &VisibleSample) -> Result<(), SelectionReportError> {
    for (field, value) in [
        ("candidate_id", sample.candidate_id.as_str()),
        ("provider", sample.provider.as_str()),
        ("chain_id", sample.chain_id.as_str()),
        ("stock_code", sample.stock_code.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(invalid_sample(sample, format!("{field} must not be blank")));
        }
    }
    Ok(())
}

fn parse_feature(sample: &VisibleSample) -> Result<FeatureProjection, SelectionReportError> {
    serde_json::from_str(&sample.feature_payload_json).map_err(|error| {
        invalid_sample(
            sample,
            format!("feature snapshot cannot be parsed: {error}"),
        )
    })
}

fn parse_t0(sample: &VisibleSample, payload: &str) -> Result<T0Projection, SelectionReportError> {
    serde_json::from_str(payload)
        .map_err(|error| invalid_sample(sample, format!("T0 outcome cannot be parsed: {error}")))
}

fn parse_d1(sample: &VisibleSample, payload: &str) -> Result<D1Projection, SelectionReportError> {
    serde_json::from_str(payload)
        .map_err(|error| invalid_sample(sample, format!("D+1 outcome cannot be parsed: {error}")))
}

fn feature_bucket(
    sample: &VisibleSample,
    price_vs_ma20: Option<f64>,
    volume_vs_5d: Option<f64>,
) -> Result<String, SelectionReportError> {
    for (field, value) in [
        ("price_vs_ma20", price_vs_ma20),
        ("volume_vs_5d", volume_vs_5d),
    ] {
        if value.is_some_and(|value| !value.is_finite()) {
            return Err(invalid_sample(
                sample,
                format!("feature bucket {field} must be finite"),
            ));
        }
    }
    let price = match price_vs_ma20 {
        None => "price:missing",
        Some(value) if value < 0.0 => "price:below_ma20",
        Some(value) if value < 0.05 => "price:0_5pct_above_ma20",
        Some(value) if value < 0.10 => "price:5_10pct_above_ma20",
        Some(value) if value < 0.15 => "price:10_15pct_above_ma20",
        Some(_) => "price:15pct_plus_above_ma20",
    };
    let volume = match volume_vs_5d {
        None => "volume:missing",
        Some(value) if value < 1.0 => "volume:below_1x",
        Some(value) if value < 1.5 => "volume:1_1.5x",
        Some(value) if value < 2.0 => "volume:1.5_2x",
        Some(_) => "volume:2x_plus",
    };
    Ok(format!("{price}|{volume}"))
}

fn require_finite(
    sample: &VisibleSample,
    field: &str,
    value: f64,
) -> Result<(), SelectionReportError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(invalid_sample(
            sample,
            format!("{field} must be finite, got {value}"),
        ))
    }
}

fn invalid_sample(sample: &VisibleSample, reason: impl Into<String>) -> SelectionReportError {
    SelectionReportError::InvalidSample {
        candidate_id: sample.candidate_id.clone(),
        reason: reason.into(),
    }
}

fn quantile(values: &[f64], probability: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = (sorted.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        Some(sorted[lower])
    } else {
        let weight = position - lower as f64;
        Some(sorted[lower] * (1.0 - weight) + sorted[upper] * weight)
    }
}

fn optional_date(value: Option<NaiveDate>) -> String {
    value.map_or_else(|| "不限".to_owned(), |date| date.to_string())
}

fn optional_percent(value: Option<f64>) -> String {
    value.map_or_else(
        || "缺失".to_owned(),
        |value| format!("{:.2}%", value * 100.0),
    )
}

fn optional_ratio(value: Option<f64>) -> String {
    value.map_or_else(|| "缺失".to_owned(), |value| format!("{value:.2}x"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_sample(candidate_id: &str, d1_close_return: f64) -> VisibleSample {
        VisibleSample {
            candidate_id: candidate_id.to_owned(),
            run_id: "TEST_CODE_run".to_owned(),
            event_id: "TEST_CODE_event".to_owned(),
            provider: "TEST_CODE_provider".to_owned(),
            chain_id: "power-grid".to_owned(),
            stock_code: "TEST_CODE_600396".to_owned(),
            stock_name: "华电辽能".to_owned(),
            evaluation_market_date: NaiveDate::from_ymd_opt(2026, 7, 23).expect("date"),
            feature_payload_json: r#"{
                "relation":{"matched_by":"ExactSecurityCode"},
                "features":{"price_vs_ma20":0.04,"volume_vs_5d":1.6}
            }"#
            .to_owned(),
            t0_outcome_payload_json: Some(r#"{"close_return":0.03}"#.to_owned()),
            d1_outcome_payload_json: Some(format!(
                r#"{{
                    "close_return":{d1_close_return},
                    "mfe":0.06,
                    "mae":-0.02,
                    "volume_vs_t0":1.2
                }}"#
            )),
        }
    }

    #[test]
    fn report_groups_only_visible_samples_and_never_claims_success_rate() {
        let samples = vec![
            visible_sample("TEST_CODE_candidate_1", 0.02),
            visible_sample("TEST_CODE_candidate_2", 0.04),
        ];
        let report = build_report(&samples, &ReportFilter::default()).expect("raw report");

        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].sample_count, 2);
        assert_eq!(report.groups[0].close_return_median, Some(0.03));
        let rendered = render_text(&report);
        assert!(rendered.contains("样本数"));
        assert!(rendered.contains("收益中位数"));
        assert!(rendered.contains("MFE"));
        assert!(rendered.contains("MAE"));
        assert!(!rendered.contains("成功率"));
        assert!(!rendered.contains("胜率"));
        assert!(!rendered.contains("推荐"));
    }

    #[test]
    fn report_keeps_missing_outcomes_separate() {
        let mut sample = visible_sample("TEST_CODE_candidate_missing", 0.02);
        sample.t0_outcome_payload_json = None;
        sample.d1_outcome_payload_json = None;

        let report = build_report(&[sample], &ReportFilter::default()).expect("raw report");
        let group = &report.groups[0];
        assert_eq!(group.sample_count, 1);
        assert_eq!(group.missing_t0_count, 1);
        assert_eq!(group.missing_d1_count, 1);
        assert_eq!(group.close_return_median, None);
    }

    #[test]
    fn report_applies_exact_provider_and_chain_filters() {
        let included = visible_sample("TEST_CODE_candidate_included", 0.02);
        let mut excluded = visible_sample("TEST_CODE_candidate_excluded", 0.04);
        excluded.provider = "TEST_CODE_other_provider".to_owned();
        let filter = ReportFilter {
            provider: Some("TEST_CODE_provider".to_owned()),
            chain_id: Some("power-grid".to_owned()),
            ..ReportFilter::default()
        };

        let report = build_report(&[included, excluded], &filter).expect("filtered report");

        assert_eq!(report.visible_sample_count, 1);
    }

    #[test]
    fn report_rejects_d1_without_immutable_t0() {
        let mut sample = visible_sample("TEST_CODE_candidate_invalid", 0.02);
        sample.t0_outcome_payload_json = None;

        let error =
            build_report(&[sample], &ReportFilter::default()).expect_err("invalid outcome lineage");

        assert!(error.to_string().contains("without immutable T0"));
    }
}
