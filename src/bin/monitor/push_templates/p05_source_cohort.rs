//! P-05 candidate cohort observation. Original candidate-source identity is not
//! retained by `RealCandidateBatch`, so this contract is always unqualified.
//! The production sender does not call it, and no occurrence or terminal
//! delivery result is bound here.

use super::{candidate_prediction_target_date, RealCandidateBatch};
use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};
use stock_analysis::data_gateway::{parse_evidence_instant, BatchEvidence};
use stock_analysis::opportunity::candidate_panel::{
    classify_tier, format_candidate_board, CandidateEntry, CandidateSource, EvidenceTier,
};

const SCHEMA: &str = "P05_CANDIDATE_SOURCE_COHORT_V2";
const MISSING_ORIGIN: &str = "unqualified_missing_raw_candidate_source_identity";

/// Deterministic observed facts, never a counted delivery binding or a
/// successful-push denominator. Production source qualification fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P05CandidateCohortObservationV2 {
    canonical_bytes: Vec<u8>,
    observation_sha256: String,
    target_date: NaiveDate,
}

impl P05CandidateCohortObservationV2 {
    pub(super) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub(super) fn observation_sha256(&self) -> &str {
        &self.observation_sha256
    }

    pub(super) fn target_date(&self) -> NaiveDate {
        self.target_date
    }

    pub(super) fn require_qualified_origin(&self) -> Result<(), &'static str> {
        Err("P-05 v2 origin source identity missing: raw P5 file rows and chain_daily snapshot are not retained")
    }
}

#[derive(Serialize)]
struct CanonicalSource<'a> {
    provider: stock_analysis::market_domain::ProviderId,
    source: &'a str,
    source_at: &'a str,
    observed_at: &'a str,
    batch_id: &'a str,
}

impl<'a> CanonicalSource<'a> {
    fn required(role: &'static str, evidence: &'a BatchEvidence) -> Result<Self, String> {
        let required = |field: &'static str, value: &'a str| {
            if value.is_empty() || value.trim() != value {
                Err(format!("P-05 v2 {role} {field} missing or noncanonical"))
            } else {
                Ok(value)
            }
        };
        let source = required("source", &evidence.source)?;
        let source_at = required(
            "source_at",
            evidence.source_at.as_deref().unwrap_or_default(),
        )?;
        let observed_at = required("observed_at", &evidence.observed_at)?;
        let batch_id = required("batch_id", &evidence.batch_id)?;
        parse_evidence_instant(
            "P05CandidateSourceCohortV2",
            evidence.provider,
            "observed_at",
            observed_at,
        )
        .map_err(|error| format!("P-05 v2 {role} observed_at invalid: {error}"))?;
        Ok(Self {
            provider: evidence.provider,
            source,
            source_at,
            observed_at,
            batch_id,
        })
    }
}

#[derive(Serialize)]
struct CanonicalStrong<'a> {
    board_ordinal: usize,
    code: &'a str,
    name: &'a str,
    sources: Vec<&'static str>,
    evidence: &'a [String],
    current_price: f64,
    change_pct: f64,
    raw_heat_score: Option<f64>,
    sample_score: f64,
}

#[derive(Serialize)]
struct CanonicalCohort<'a> {
    schema: &'static str,
    origin_qualification: &'static str,
    business_date: String,
    target_date: String,
    quote_source: CanonicalSource<'a>,
    statistics_source: CanonicalSource<'a>,
    rendered_sha256: String,
    ordered_strong: Vec<CanonicalStrong<'a>>,
}

fn source_name(source: CandidateSource) -> &'static str {
    match source {
        CandidateSource::StockPick => "StockPick",
        CandidateSource::OptimalClose => "OptimalClose",
        CandidateSource::VolumeWatchlist => "VolumeWatchlist",
        CandidateSource::VolumeRealTrade => "VolumeRealTrade",
        CandidateSource::IndustryChain => "IndustryChain",
        CandidateSource::NewsCatalyst => "NewsCatalyst",
    }
}

fn strong_row<'a>(
    ordinal: usize,
    entry: &'a CandidateEntry,
    batch: &RealCandidateBatch,
) -> Result<CanonicalStrong<'a>, String> {
    if classify_tier(&entry.evidence) != EvidenceTier::Strong {
        return Err(format!(
            "P-05 v2 {} Strong tier has no Strong evidence",
            entry.code
        ));
    }
    if entry.sources.is_empty() || entry.evidence.is_empty() {
        return Err(format!(
            "P-05 v2 {} Strong source/evidence missing",
            entry.code
        ));
    }
    let price = entry
        .current_price
        .ok_or_else(|| format!("P-05 v2 {} Strong price missing", entry.code))?;
    let change_pct = entry
        .change_pct
        .ok_or_else(|| format!("P-05 v2 {} Strong change missing", entry.code))?;
    let score = entry.heat_score.unwrap_or(50.0);
    if !price.is_finite() || price <= 0.0 || !change_pct.is_finite() || !score.is_finite() {
        return Err(format!(
            "P-05 v2 {} Strong numeric fact invalid",
            entry.code
        ));
    }
    let quote = batch
        .quotes
        .get(&entry.code)
        .ok_or_else(|| format!("P-05 v2 {} Strong quote missing", entry.code))?;
    if quote.code != entry.code
        || quote.name != entry.name
        || quote.price != price
        || quote.change_pct != change_pct
    {
        return Err(format!(
            "P-05 v2 {} Strong quote/entry mismatch",
            entry.code
        ));
    }
    Ok(CanonicalStrong {
        board_ordinal: ordinal,
        code: &entry.code,
        name: &entry.name,
        sources: entry.sources.iter().copied().map(source_name).collect(),
        evidence: &entry.evidence,
        current_price: price,
        change_pct,
        raw_heat_score: entry.heat_score,
        sample_score: score,
    })
}

/// Observe ordered Strong rows from the supplied batch and supplied render.
/// The render is checked against the formatter for those supplied entries;
/// this function is not wired to the production sender or its occurrence.
/// Quote/statistics provenance alone cannot qualify the original P5 file and
/// chain records; callers must reject `require_qualified_origin()` until an
/// independently validated origin witness is retained and bound.
pub(super) fn observe_p05_candidate_cohort_v2(
    business_date: NaiveDate,
    batch: &RealCandidateBatch,
    rendered: &str,
) -> Result<P05CandidateCohortObservationV2, String> {
    if !stock_analysis::calendar::verified_a_share_trading_day(business_date)? {
        return Err(format!(
            "P-05 v2 {business_date} is not an A-share trading day"
        ));
    }
    if batch.entries.is_empty() {
        return Err("P-05 v2 candidate board is empty".to_string());
    }
    let quote_source = CanonicalSource::required(
        "quote",
        batch
            .quote_evidence
            .as_ref()
            .ok_or("P-05 v2 quote source missing")?,
    )?;
    let statistics_source = CanonicalSource::required(
        "statistics",
        batch
            .statistics_evidence
            .as_ref()
            .ok_or("P-05 v2 statistics source missing")?,
    )?;
    if rendered != format_candidate_board(&batch.entries) {
        return Err("P-05 v2 rendered board differs from RealCandidateBatch".to_string());
    }

    let mut seen = std::collections::HashSet::new();
    let mut ordered_strong = Vec::new();
    for (index, entry) in batch.entries.iter().enumerate() {
        if !seen.insert(entry.code.as_str()) {
            return Err(format!("P-05 v2 duplicate candidate {}", entry.code));
        }
        if classify_tier(&entry.evidence) != entry.tier {
            return Err(format!(
                "P-05 v2 {} tier differs from source evidence",
                entry.code
            ));
        }
        if entry.tier == EvidenceTier::Strong {
            ordered_strong.push(strong_row(index + 1, entry, batch)?);
        }
    }
    let target_date = candidate_prediction_target_date(business_date)?;
    let canonical = CanonicalCohort {
        schema: SCHEMA,
        origin_qualification: MISSING_ORIGIN,
        business_date: business_date.format("%Y-%m-%d").to_string(),
        target_date: target_date.format("%Y-%m-%d").to_string(),
        quote_source,
        statistics_source,
        rendered_sha256: hex::encode(Sha256::digest(rendered.as_bytes())),
        ordered_strong,
    };
    let canonical_bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("P-05 v2 canonical source encoding failed: {error}"))?;
    let observation_sha256 = hex::encode(Sha256::digest(&canonical_bytes));
    Ok(P05CandidateCohortObservationV2 {
        canonical_bytes,
        observation_sha256,
        target_date,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::market_data::TopStock;
    use stock_analysis::market_domain::ProviderId;
    use stock_analysis::opportunity::candidate_panel::CandidateSource;

    fn evidence(role: &str) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tencent,
            source: format!("TEST_CODE_{role}"),
            source_at: Some("2026-09-24T09:30:00+08:00".into()),
            observed_at: "2026-09-24T09:30:01+08:00".into(),
            batch_id: format!("TEST_CODE_{role}_batch"),
        }
    }

    fn entry(code: &str, name: &str, tier: EvidenceTier) -> CandidateEntry {
        CandidateEntry {
            code: code.into(),
            name: name.into(),
            sources: vec![CandidateSource::StockPick],
            tier,
            evidence: if tier == EvidenceTier::Strong {
                vec!["布林+MACD主升浪".into()]
            } else {
                vec!["放量参考".into()]
            },
            current_price: Some(10.0),
            change_pct: Some(1.0),
            heat_score: None,
        }
    }

    fn fixture() -> (NaiveDate, RealCandidateBatch) {
        let entries = vec![
            entry("TEST_CODE_600001", "强甲", EvidenceTier::Strong),
            entry("TEST_CODE_600002", "参考", EvidenceTier::Reference),
            entry("TEST_CODE_600003", "强乙", EvidenceTier::Strong),
        ];
        let quotes = entries
            .iter()
            .map(|entry| {
                (
                    entry.code.clone(),
                    TopStock {
                        code: entry.code.clone(),
                        name: entry.name.clone(),
                        price: entry.current_price.unwrap(),
                        change_pct: entry.change_pct.unwrap(),
                        volume_ratio: None,
                        main_net_yi: None,
                    },
                )
            })
            .collect();
        (
            NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
            RealCandidateBatch {
                entries,
                quotes,
                themes: Default::default(),
                quote_evidence: Some(evidence("quote")),
                statistics_evidence: Some(evidence("statistics")),
                p5_files: Vec::new(),
                p5_candidate_refs: Vec::new(),
            },
        )
    }

    fn observe(date: NaiveDate, batch: &RealCandidateBatch) -> P05CandidateCohortObservationV2 {
        let rendered = format_candidate_board(&batch.entries);
        observe_p05_candidate_cohort_v2(date, batch, &rendered).unwrap()
    }

    #[test]
    fn replay_preserves_complete_ordered_strong_cohort_and_verified_target_day() {
        let (date, batch) = fixture();
        let first = observe(date, &batch);
        assert_eq!(first, observe(date, &batch));
        assert_eq!(first.target_date().to_string(), "2026-10-09");
        let facts: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
        assert_eq!(facts["schema"], SCHEMA);
        assert_eq!(facts["origin_qualification"], MISSING_ORIGIN);
        assert_eq!(facts["business_date"], "2026-09-24");
        assert_eq!(facts["target_date"], "2026-10-09");
        assert_eq!(facts["ordered_strong"].as_array().unwrap().len(), 2);
        assert_eq!(facts["ordered_strong"][0]["board_ordinal"], 1);
        assert_eq!(facts["ordered_strong"][0]["code"], "TEST_CODE_600001");
        assert_eq!(facts["ordered_strong"][1]["board_ordinal"], 3);
        assert_eq!(facts["ordered_strong"][1]["code"], "TEST_CODE_600003");
        assert!(facts["ordered_strong"][0]["raw_heat_score"].is_null());
        assert_eq!(facts["ordered_strong"][0]["sample_score"], 50.0);
        assert_eq!(first.observation_sha256().len(), 64);
        assert!(first.require_qualified_origin().is_err());
    }

    #[test]
    fn missing_and_explicit_default_heat_score_have_distinct_observations() {
        let (date, mut batch) = fixture();
        let rendered = format_candidate_board(&batch.entries);
        let missing = observe_p05_candidate_cohort_v2(date, &batch, &rendered).unwrap();
        batch.entries[0].heat_score = Some(50.0);
        assert_eq!(rendered, format_candidate_board(&batch.entries));
        let explicit = observe_p05_candidate_cohort_v2(date, &batch, &rendered).unwrap();
        assert_ne!(missing.observation_sha256(), explicit.observation_sha256());
        let facts: serde_json::Value = serde_json::from_slice(explicit.canonical_bytes()).unwrap();
        assert_eq!(facts["ordered_strong"][0]["raw_heat_score"], 50.0);
        assert_eq!(facts["ordered_strong"][0]["sample_score"], 50.0);
    }

    #[test]
    fn cohort_fingerprint_changes_with_content_order_or_source() {
        let (date, mut batch) = fixture();
        let baseline = observe(date, &batch);
        batch.entries[0].evidence.push("新的强证据".into());
        assert_ne!(
            baseline.observation_sha256(),
            observe(date, &batch).observation_sha256()
        );
        batch.entries[0].evidence.pop();
        batch.entries.swap(0, 2);
        assert_ne!(
            baseline.observation_sha256(),
            observe(date, &batch).observation_sha256()
        );
        batch.entries.swap(0, 2);
        batch
            .quote_evidence
            .as_mut()
            .unwrap()
            .batch_id
            .push_str("_changed");
        assert_ne!(
            baseline.observation_sha256(),
            observe(date, &batch).observation_sha256()
        );
    }

    #[test]
    fn missing_source_or_mismatched_render_and_quote_fail_closed() {
        let (date, mut batch) = fixture();
        let rendered = format_candidate_board(&batch.entries);
        assert!(
            observe_p05_candidate_cohort_v2(date, &batch, &(rendered.clone() + "tampered"))
                .is_err()
        );
        batch.quote_evidence = None;
        assert!(observe_p05_candidate_cohort_v2(date, &batch, &rendered).is_err());
        batch.quote_evidence = Some(evidence("quote"));
        batch.statistics_evidence.as_mut().unwrap().source_at = None;
        assert!(observe_p05_candidate_cohort_v2(date, &batch, &rendered).is_err());
        batch.statistics_evidence = Some(evidence("statistics"));
        batch.quotes.get_mut("TEST_CODE_600001").unwrap().price = 9.0;
        assert!(observe_p05_candidate_cohort_v2(date, &batch, &rendered).is_err());
        batch.quotes.get_mut("TEST_CODE_600001").unwrap().price = 10.0;
        batch.entries[0].evidence = vec!["放量参考".into()];
        let rendered = format_candidate_board(&batch.entries);
        assert!(observe_p05_candidate_cohort_v2(date, &batch, &rendered).is_err());
        batch.entries[0].evidence = vec!["布林+MACD主升浪".into()];
        batch.entries[0].tier = EvidenceTier::Reference;
        let rendered = format_candidate_board(&batch.entries);
        assert!(observe_p05_candidate_cohort_v2(date, &batch, &rendered).is_err());
    }
}
