//! P-05 candidate cohort observation. `RealCandidateBatch` retains P5 JSONL
//! row witnesses and the batch retains chain_daily decoded-column witnesses.
//! V2 does not bind those witnesses. V3 binds the chain rows and survivor refs,
//! but producer and snapshot authority remain absent, so it is unqualified.
//! The production sender does not call it, and no occurrence or terminal
//! delivery result is bound here.

use super::{candidate_prediction_target_date, p05_chain_witness, RealCandidateBatch};
use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};
use stock_analysis::data_gateway::{parse_evidence_instant, BatchEvidence};
use stock_analysis::opportunity::candidate_panel::{
    classify_tier, format_candidate_board, CandidateEntry, CandidateSource, EvidenceTier,
};

const SCHEMA: &str = "P05_CANDIDATE_SOURCE_COHORT_V2";
const MISSING_ORIGIN: &str = "unqualified_missing_raw_candidate_source_identity";
const SCHEMA_V3: &str = "P05_CANDIDATE_SOURCE_COHORT_V3";
const MISSING_ORIGIN_V3: &str =
    "unqualified_missing_chain_snapshot_p5_producer_and_held_position_snapshot";

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
        Err("P-05 v2 origin qualification missing: chain_daily snapshot, P5 producer authority, and held-position snapshot are not retained")
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
/// Quote/statistics provenance and retained P5/chain rows do not qualify the
/// chain snapshot, producer, or held-position origins. Callers must reject
/// `require_qualified_origin()` until those witnesses are bound.
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

/// V3 binds the validated V2 observation to the same-query chain witness and
/// only the IndustryChain refs that survived candidate hard gates. It remains
/// an observation; it proves neither query snapshot authority nor delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P05CandidateCohortObservationV3 {
    canonical_bytes: Vec<u8>,
    observation_sha256: String,
    target_date: NaiveDate,
}

impl P05CandidateCohortObservationV3 {
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
        Err("P-05 v3 origin qualification missing: authoritative chain query snapshot, P5 producer authority, and held-position snapshot")
    }
}

#[derive(Serialize)]
struct CanonicalChainRow<'a> {
    ordinal: usize,
    date: &'a str,
    concept: &'a str,
    stocks: &'a str,
    continuation_count: i32,
    row_sha256: &'a str,
}

#[derive(Serialize)]
struct CanonicalChainRef<'a> {
    code: &'a str,
    source: &'static str,
    date: &'a str,
    concept: &'a str,
    ordinal: usize,
    row_sha256: &'a str,
    ordered_rows_sha256: &'a str,
}

#[derive(Serialize)]
struct CanonicalChainQuery<'a> {
    schema: &'a str,
    latest_date: Option<&'a str>,
    total_rows: usize,
    ordered_rows_sha256: &'a str,
    selected_rows: Vec<CanonicalChainRow<'a>>,
    survivor_refs: Vec<CanonicalChainRef<'a>>,
}

#[derive(Serialize)]
struct CanonicalCohortV3<'a> {
    schema: &'static str,
    origin_qualification: &'static str,
    business_date: String,
    target_date: String,
    /// Exact published V2 bytes, hex encoded so V3 is self-contained.
    v2_canonical_hex: String,
    v2_observation_sha256: &'a str,
    chain_query: CanonicalChainQuery<'a>,
}

pub(super) fn observe_p05_candidate_cohort_v3(
    business_date: NaiveDate,
    batch: &RealCandidateBatch,
    rendered: &str,
) -> Result<P05CandidateCohortObservationV3, String> {
    let v2 = observe_p05_candidate_cohort_v2(business_date, batch, rendered)?;

    // The complete decoded rows came from the same query as the selected
    // rows. Reprojection checks the full digest, selected row columns/hash,
    // latest date, count, schema, and deterministic top-five order.
    let replay = p05_chain_witness::project_same_query(batch.chain_query.ordered_rows.clone())?;
    if replay.witness != batch.chain_query {
        return Err("P-05 v3 chain query witness differs from same-query rows".to_string());
    }
    let surviving_entries: std::collections::HashMap<_, _> = batch
        .entries
        .iter()
        .map(|entry| (entry.code.as_str(), entry))
        .collect();
    // Select by surviving identity before looking at its declared sources.
    // Otherwise removing IndustryChain from an entry and deleting its ref
    // would make both sides empty and hide a same-query source.
    let expected_refs: Vec<_> = replay
        .candidate_refs
        .into_iter()
        .filter(|reference| surviving_entries.contains_key(reference.code.as_str()))
        .collect();
    for reference in &expected_refs {
        if !surviving_entries[reference.code.as_str()]
            .sources
            .contains(&CandidateSource::IndustryChain)
        {
            return Err(format!(
                "P-05 v3 surviving candidate {} lost same-query IndustryChain source",
                reference.code
            ));
        }
    }
    if expected_refs != batch.chain_candidate_refs {
        return Err("P-05 v3 chain survivor refs differ from same-query rows".to_string());
    }
    for entry in &batch.entries {
        if entry.sources.contains(&CandidateSource::IndustryChain)
            && !expected_refs
                .iter()
                .any(|reference| reference.code == entry.code)
        {
            return Err(format!(
                "P-05 v3 surviving IndustryChain candidate {} has no same-query ref",
                entry.code
            ));
        }
    }

    let query = &batch.chain_query;
    let canonical = CanonicalCohortV3 {
        schema: SCHEMA_V3,
        origin_qualification: MISSING_ORIGIN_V3,
        business_date: business_date.format("%Y-%m-%d").to_string(),
        target_date: v2.target_date().format("%Y-%m-%d").to_string(),
        v2_canonical_hex: hex::encode(v2.canonical_bytes()),
        v2_observation_sha256: v2.observation_sha256(),
        chain_query: CanonicalChainQuery {
            schema: query.schema,
            latest_date: query.latest_date.as_deref(),
            total_rows: query.total_rows,
            ordered_rows_sha256: &query.ordered_rows_sha256,
            selected_rows: query
                .selected_rows
                .iter()
                .map(|selected| CanonicalChainRow {
                    ordinal: selected.ordinal,
                    date: &selected.row.date,
                    concept: &selected.row.concept,
                    stocks: &selected.row.stocks,
                    continuation_count: selected.row.continuation_count,
                    row_sha256: &selected.row_sha256,
                })
                .collect(),
            survivor_refs: batch
                .chain_candidate_refs
                .iter()
                .map(|reference| CanonicalChainRef {
                    code: &reference.code,
                    source: source_name(reference.source),
                    date: &reference.date,
                    concept: &reference.concept,
                    ordinal: reference.ordinal,
                    row_sha256: &reference.row_sha256,
                    ordered_rows_sha256: &reference.ordered_rows_sha256,
                })
                .collect(),
        },
    };
    let canonical_bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("P-05 v3 canonical source encoding failed: {error}"))?;
    let observation_sha256 = hex::encode(Sha256::digest(&canonical_bytes));
    Ok(P05CandidateCohortObservationV3 {
        canonical_bytes,
        observation_sha256,
        target_date: v2.target_date(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::database::concepts::ChainDailyRow;
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
                chain_query: super::super::p05_chain_witness::project_same_query(Vec::new())
                    .unwrap()
                    .witness,
                chain_candidate_refs: Vec::new(),
            },
        )
    }

    fn observe(date: NaiveDate, batch: &RealCandidateBatch) -> P05CandidateCohortObservationV2 {
        let rendered = format_candidate_board(&batch.entries);
        observe_p05_candidate_cohort_v2(date, batch, &rendered).unwrap()
    }

    fn chain_rows() -> Vec<ChainDailyRow> {
        [
            ("A", "TEST_CODE_600001", 6),
            ("B", "TEST_CODE_600004", 5),
            ("C", "TEST_CODE_600005", 4),
            ("D", "TEST_CODE_600006", 3),
            ("E", "TEST_CODE_600007", 2),
            ("F", "TEST_CODE_600008", 1),
        ]
        .into_iter()
        .map(|(concept, code, continuation_count)| ChainDailyRow {
            date: "2026-09-24".into(),
            concept: concept.into(),
            stocks: format!("[\"{code}\"]"),
            continuation_count,
        })
        .collect()
    }

    fn chain_fixture() -> (NaiveDate, RealCandidateBatch) {
        let (date, mut batch) = fixture();
        batch.entries[0]
            .sources
            .push(CandidateSource::IndustryChain);
        let projected = p05_chain_witness::project_same_query(chain_rows()).unwrap();
        batch.chain_candidate_refs =
            p05_chain_witness::selected_refs(&batch.entries, projected.candidate_refs);
        batch.chain_query = projected.witness;
        (date, batch)
    }

    fn observe_v3(date: NaiveDate, batch: &RealCandidateBatch) -> P05CandidateCohortObservationV3 {
        let rendered = format_candidate_board(&batch.entries);
        observe_p05_candidate_cohort_v3(date, batch, &rendered).unwrap()
    }

    #[test]
    fn v3_replay_binds_full_query_selected_rows_and_only_survivor_refs() {
        let (date, mut batch) = chain_fixture();
        let rendered = format_candidate_board(&batch.entries);
        let v2 = observe_p05_candidate_cohort_v2(date, &batch, &rendered).unwrap();
        let first = observe_v3(date, &batch);
        assert_eq!(first, observe_v3(date, &batch));
        assert_eq!(first.target_date(), v2.target_date());
        assert!(first.require_qualified_origin().is_err());
        let facts: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
        assert_eq!(facts["schema"], SCHEMA_V3);
        assert_eq!(facts["origin_qualification"], MISSING_ORIGIN_V3);
        assert_eq!(facts["v2_observation_sha256"], v2.observation_sha256());
        assert_eq!(
            hex::decode(facts["v2_canonical_hex"].as_str().unwrap()).unwrap(),
            v2.canonical_bytes()
        );
        assert_eq!(facts["chain_query"]["total_rows"], 6);
        assert_eq!(
            facts["chain_query"]["selected_rows"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            facts["chain_query"]["survivor_refs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            facts["chain_query"]["survivor_refs"][0]["code"],
            "TEST_CODE_600001"
        );
        assert_eq!(
            facts["chain_query"]["survivor_refs"][0]["ordered_rows_sha256"],
            batch.chain_query.ordered_rows_sha256
        );

        // Changing only the sixth row changes V3's full-query binding while
        // the published V2 bytes and the selected five rows stay identical.
        let selected_before = batch.chain_query.selected_rows.clone();
        let mut changed_rows = chain_rows();
        changed_rows[5].stocks = "[\"TEST_CODE_600009\"]".into();
        let projected = p05_chain_witness::project_same_query(changed_rows).unwrap();
        batch.chain_candidate_refs =
            p05_chain_witness::selected_refs(&batch.entries, projected.candidate_refs);
        batch.chain_query = projected.witness;
        assert_eq!(batch.chain_query.selected_rows, selected_before);
        assert_eq!(
            observe_p05_candidate_cohort_v2(date, &batch, &rendered)
                .unwrap()
                .canonical_bytes(),
            v2.canonical_bytes()
        );
        assert_ne!(
            first.observation_sha256(),
            observe_v3(date, &batch).observation_sha256()
        );
    }

    #[test]
    fn v3_rejects_tampered_or_misbound_chain_refs_without_changing_v2() {
        let (date, mut batch) = chain_fixture();
        let rendered = format_candidate_board(&batch.entries);
        let v2_bytes = observe_p05_candidate_cohort_v2(date, &batch, &rendered)
            .unwrap()
            .canonical_bytes()
            .to_vec();
        let original = batch.chain_candidate_refs[0].clone();
        let mut variants = Vec::new();
        let mut wrong = original.clone();
        wrong.ordinal += 1;
        variants.push(wrong);
        let mut wrong = original.clone();
        wrong.date = "2026-09-23".into();
        variants.push(wrong);
        let mut wrong = original.clone();
        wrong.concept = "B".into();
        variants.push(wrong);
        let mut wrong = original.clone();
        wrong.row_sha256 = "0".repeat(64);
        variants.push(wrong);
        let mut wrong = original.clone();
        wrong.ordered_rows_sha256 = "0".repeat(64);
        variants.push(wrong);
        let mut wrong = original.clone();
        wrong.code = "TEST_CODE_600004".into();
        variants.push(wrong);
        let mut wrong = original.clone();
        wrong.source = CandidateSource::StockPick;
        variants.push(wrong);
        for wrong in variants {
            batch.chain_candidate_refs = vec![wrong];
            assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_err());
            assert_eq!(
                observe_p05_candidate_cohort_v2(date, &batch, &rendered)
                    .unwrap()
                    .canonical_bytes(),
                v2_bytes
            );
        }
        batch.chain_candidate_refs.clear();
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_err());
    }

    #[test]
    fn v3_rejects_query_digest_selected_row_or_complete_row_tamper() {
        let (date, mut batch) = chain_fixture();
        let rendered = format_candidate_board(&batch.entries);
        let original = batch.chain_query.clone();
        batch.chain_query.ordered_rows_sha256 = "0".repeat(64);
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_err());
        batch.chain_query = original.clone();
        batch.chain_query.selected_rows[0].row.stocks = "[\"TEST_CODE_600009\"]".into();
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_err());
        batch.chain_query = original.clone();
        batch.chain_query.ordered_rows[5].stocks = "[\"TEST_CODE_600009\"]".into();
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_err());
        batch.chain_query = original;
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_ok());
    }

    #[test]
    fn v3_rejects_removing_chain_source_and_ref_from_surviving_candidate() {
        let (date, mut batch) = chain_fixture();
        assert_eq!(batch.chain_candidate_refs.len(), 1);
        batch.entries[0]
            .sources
            .retain(|source| *source != CandidateSource::IndustryChain);
        batch.chain_candidate_refs.clear();
        let rendered = format_candidate_board(&batch.entries);
        assert!(observe_p05_candidate_cohort_v2(date, &batch, &rendered).is_ok());
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered)
            .unwrap_err()
            .contains("lost same-query IndustryChain source"));

        // A candidate actually removed by the existing hard gates has no
        // surviving identity and therefore needs no final chain ref.
        batch.entries.remove(0);
        let rendered = format_candidate_board(&batch.entries);
        assert!(observe_p05_candidate_cohort_v3(date, &batch, &rendered).is_ok());
    }

    #[test]
    fn v3_keeps_selected_empty_stocks_row_without_inventing_survivor_ref() {
        let (date, mut batch) = fixture();
        let projected = p05_chain_witness::project_same_query(vec![ChainDailyRow {
            date: "2026-09-24".into(),
            concept: "A".into(),
            stocks: "[]".into(),
            continuation_count: 1,
        }])
        .unwrap();
        assert_eq!(projected.witness.selected_rows.len(), 1);
        assert_eq!(projected.witness.selected_rows[0].row.stocks, "[]");
        assert!(projected.candidate_refs.is_empty());
        batch.chain_query = projected.witness;
        let observed = observe_v3(date, &batch);
        let facts: serde_json::Value = serde_json::from_slice(observed.canonical_bytes()).unwrap();
        assert_eq!(
            facts["chain_query"]["selected_rows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(facts["chain_query"]["survivor_refs"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(observed.require_qualified_origin().is_err());
    }

    #[test]
    fn v3_empty_chain_stays_observed_but_unqualified() {
        let (date, batch) = fixture();
        let observed = observe_v3(date, &batch);
        let facts: serde_json::Value = serde_json::from_slice(observed.canonical_bytes()).unwrap();
        assert_eq!(facts["chain_query"]["total_rows"], 0);
        assert!(facts["chain_query"]["selected_rows"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(facts["chain_query"]["survivor_refs"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(observed.require_qualified_origin().is_err());
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
