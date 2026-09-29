//! P-05 chain_daily witness from one ordered SQLite query result. These are
//! decoded column values, not raw SQLite bytes or producer-owned provenance.

use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};
use stock_analysis::database::concepts::ChainDailyRow;
use stock_analysis::opportunity::candidate_panel::{CandidateEntry, CandidateSource};

type SourceItem = (CandidateSource, String, String);

const QUERY_SCHEMA: &str = "P05_CHAIN_DAILY_LATEST_QUERY_V1";
const ROW_SCHEMA: &str = "P05_CHAIN_DAILY_ROW_V1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChainSourceDateRelation {
    Empty,
    Exact,
    Old,
    Future,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P05SelectedChainRow {
    /// One-based position in the complete, ordered latest-date query result.
    pub(super) ordinal: usize,
    /// Exact decoded values of the selected SQLite columns.
    pub(super) row: ChainDailyRow,
    pub(super) row_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P05ChainQueryWitness {
    pub(super) schema: &'static str,
    pub(super) latest_date: Option<String>,
    pub(super) total_rows: usize,
    /// Commits to all ordered rows, including those after the selected five.
    pub(super) ordered_rows_sha256: String,
    pub(super) selected_rows: Vec<P05SelectedChainRow>,
}

impl P05ChainQueryWitness {
    pub(super) fn source_date_relation(&self, business_date: NaiveDate) -> ChainSourceDateRelation {
        let Some(date) = self.latest_date.as_deref() else {
            return ChainSourceDateRelation::Empty;
        };
        let Ok(parsed) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            return ChainSourceDateRelation::Invalid;
        };
        if parsed.format("%Y-%m-%d").to_string() != date {
            return ChainSourceDateRelation::Invalid;
        }
        match parsed.cmp(&business_date) {
            std::cmp::Ordering::Less => ChainSourceDateRelation::Old,
            std::cmp::Ordering::Equal => ChainSourceDateRelation::Exact,
            std::cmp::Ordering::Greater => ChainSourceDateRelation::Future,
        }
    }

    pub(super) fn require_qualified_origin(&self) -> Result<(), &'static str> {
        Err("P-05 chain_daily query lacks an authoritative snapshot identity and producer generation version")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct P05ChainCandidateRef {
    pub(super) code: String,
    pub(super) source: CandidateSource,
    pub(super) date: String,
    pub(super) concept: String,
    pub(super) ordinal: usize,
    pub(super) row_sha256: String,
    pub(super) ordered_rows_sha256: String,
}

#[derive(Debug)]
pub(super) struct P05ChainProjection {
    pub(super) items: Vec<SourceItem>,
    pub(super) themes: std::collections::HashMap<String, String>,
    pub(super) witness: P05ChainQueryWitness,
    pub(super) candidate_refs: Vec<P05ChainCandidateRef>,
}

#[derive(Serialize)]
struct CanonicalRow<'a> {
    date: &'a str,
    concept: &'a str,
    stocks: &'a str,
    continuation_count: i32,
}

impl<'a> From<&'a ChainDailyRow> for CanonicalRow<'a> {
    fn from(row: &'a ChainDailyRow) -> Self {
        Self {
            date: &row.date,
            concept: &row.concept,
            stocks: &row.stocks,
            continuation_count: row.continuation_count,
        }
    }
}

fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    let canonical = serde_json::to_vec(value)
        .map_err(|error| format!("P-05 chain_daily witness encoding failed: {error}"))?;
    Ok(hex::encode(Sha256::digest(&canonical)))
}

/// Project the exact rows returned by the P-05 ordered latest-date query.
/// No second database read or inferred source clock is involved.
pub(super) fn project_same_query(rows: Vec<ChainDailyRow>) -> Result<P05ChainProjection, String> {
    let latest_date = rows.first().map(|row| row.date.clone());
    // SQLite's default BINARY text order compares UTF-8 bytes, matching Rust's
    // lexicographic String order for this uncollated concept column.
    for pair in rows.windows(2) {
        let previous = &pair[0];
        let next = &pair[1];
        if next.date != previous.date
            || previous.continuation_count < next.continuation_count
            || (previous.continuation_count == next.continuation_count
                && previous.concept >= next.concept)
        {
            return Err("P-05 chain_daily latest-date query order/identity invalid".to_string());
        }
    }
    let canonical_rows: Vec<_> = rows.iter().map(CanonicalRow::from).collect();
    let ordered_rows_sha256 = digest(&(QUERY_SCHEMA, &latest_date, &canonical_rows))?;

    let mut items = Vec::new();
    let mut themes = std::collections::HashMap::new();
    let mut selected_rows = Vec::new();
    let mut candidate_refs = Vec::new();
    for (index, row) in rows.iter().take(5).enumerate() {
        let ordinal = index + 1;
        let row_sha256 = digest(&(ROW_SCHEMA, CanonicalRow::from(row)))?;
        selected_rows.push(P05SelectedChainRow {
            ordinal,
            row: row.clone(),
            row_sha256: row_sha256.clone(),
        });
        let codes = serde_json::from_str::<Vec<String>>(&row.stocks).map_err(|error| {
            format!(
                "chain_daily 第 {ordinal} 个主线 {} stocks JSON 非法: {error}",
                row.concept
            )
        })?;
        let Some(code) = codes.first().map(|value| value.trim()) else {
            continue;
        };
        if !super::valid_source_stock_code(code) {
            return Err(format!(
                "chain_daily 主线 {} 头部 code 非法: {code}",
                row.concept
            ));
        }
        if row.concept.trim().is_empty() {
            return Err(format!("chain_daily 主线 {code} concept 为空"));
        }
        items.push((
            CandidateSource::IndustryChain,
            code.to_string(),
            row.concept.clone(),
        ));
        themes.insert(code.to_string(), row.concept.clone());
        candidate_refs.push(P05ChainCandidateRef {
            code: code.to_string(),
            source: CandidateSource::IndustryChain,
            date: row.date.clone(),
            concept: row.concept.clone(),
            ordinal,
            row_sha256,
            ordered_rows_sha256: ordered_rows_sha256.clone(),
        });
    }
    Ok(P05ChainProjection {
        items,
        themes,
        witness: P05ChainQueryWitness {
            schema: QUERY_SCHEMA,
            latest_date,
            total_rows: rows.len(),
            ordered_rows_sha256,
            selected_rows,
        },
        candidate_refs,
    })
}

pub(super) fn link_candidates(
    entries: &[CandidateEntry],
    refs: Vec<P05ChainCandidateRef>,
) -> Result<Vec<P05ChainCandidateRef>, String> {
    for reference in &refs {
        if !entries
            .iter()
            .any(|entry| entry.code == reference.code && entry.sources.contains(&reference.source))
        {
            return Err(format!(
                "P-05 chain_daily {}:{} has no same-query merged candidate {}",
                reference.date, reference.concept, reference.code
            ));
        }
    }
    Ok(refs)
}

pub(super) fn selected_refs(
    entries: &[CandidateEntry],
    refs: Vec<P05ChainCandidateRef>,
) -> Vec<P05ChainCandidateRef> {
    let selected_codes: std::collections::HashSet<&str> = entries
        .iter()
        .filter(|entry| entry.sources.contains(&CandidateSource::IndustryChain))
        .map(|entry| entry.code.as_str())
        .collect();
    refs.into_iter()
        .filter(|reference| {
            reference.source == CandidateSource::IndustryChain
                && selected_codes.contains(reference.code.as_str())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::opportunity::candidate_panel::merge_candidates;

    fn row(date: &str, concept: &str, code: &str, count: i32) -> ChainDailyRow {
        ChainDailyRow {
            date: date.to_string(),
            concept: concept.to_string(),
            stocks: format!("[\"{code}\"]"),
            continuation_count: count,
        }
    }

    fn latest_rows() -> Vec<ChainDailyRow> {
        vec![
            row("2026-09-24", "乙", "TEST_CODE_000002", 3),
            row("2026-09-24", "甲", "TEST_CODE_000001", 3),
            row("2026-09-24", "丁", "TEST_CODE_000004", 2),
            row("2026-09-24", "丙", "TEST_CODE_000003", 2),
            row("2026-09-24", "己", "TEST_CODE_000006", 1),
            row("2026-09-24", "戊", "TEST_CODE_000005", 1),
        ]
    }

    #[test]
    fn first_five_selected_decoded_rows_link_to_same_query_candidates() {
        let projected = project_same_query(latest_rows()).unwrap();
        assert_eq!(projected.witness.schema, QUERY_SCHEMA);
        assert_eq!(projected.witness.latest_date.as_deref(), Some("2026-09-24"));
        assert_eq!(projected.witness.total_rows, 6);
        assert_eq!(projected.witness.selected_rows.len(), 5);
        assert_eq!(projected.candidate_refs.len(), 5);
        assert_eq!(projected.witness.selected_rows[0].row.concept, "乙");
        assert_eq!(projected.witness.selected_rows[4].row.concept, "己");
        assert_eq!(
            projected.witness.selected_rows[4].row.stocks,
            "[\"TEST_CODE_000006\"]"
        );
        assert_eq!(projected.candidate_refs[4].ordinal, 5);
        assert_eq!(
            projected.candidate_refs[4].source,
            CandidateSource::IndustryChain
        );
        assert_eq!(
            projected.candidate_refs[4].row_sha256,
            projected.witness.selected_rows[4].row_sha256
        );
        let merged = merge_candidates(projected.items.clone());
        let refs = link_candidates(&merged, projected.candidate_refs.clone()).unwrap();
        assert_eq!(refs.len(), 5);
        assert!(refs.iter().all(|reference| {
            reference.ordered_rows_sha256 == projected.witness.ordered_rows_sha256
        }));
        assert!(projected.witness.require_qualified_origin().is_err());

        // The caller applies the existing hard gates before this projection.
        // Only refs for candidates still present in that result survive.
        let surviving = merged
            .into_iter()
            .filter(|entry| entry.code == "TEST_CODE_000002")
            .collect::<Vec<_>>();
        let selected = selected_refs(&surviving, refs);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].concept, "乙");
        assert!(selected_refs(&[], projected.candidate_refs).is_empty());
    }

    #[test]
    fn replay_and_column_mutation_change_versioned_query_and_row_digests() {
        let rows = latest_rows();
        let first = project_same_query(rows.clone()).unwrap();
        let replay = project_same_query(rows.clone()).unwrap();
        assert_eq!(first.witness, replay.witness);
        let mut changed = rows.clone();
        changed[0].stocks = "[\"TEST_CODE_999999\"]".into();
        let changed = project_same_query(changed).unwrap();
        assert_ne!(
            first.witness.ordered_rows_sha256,
            changed.witness.ordered_rows_sha256
        );
        assert_ne!(
            first.witness.selected_rows[0].row_sha256,
            changed.witness.selected_rows[0].row_sha256
        );
        let mut beyond_five = rows;
        beyond_five[5].stocks = "[\"TEST_CODE_999999\"]".into();
        let beyond_five = project_same_query(beyond_five).unwrap();
        assert_ne!(
            first.witness.ordered_rows_sha256,
            beyond_five.witness.ordered_rows_sha256
        );
        assert_eq!(
            first.witness.selected_rows,
            beyond_five.witness.selected_rows
        );
    }

    #[test]
    fn empty_old_and_invalid_dates_never_become_current_source_authority() {
        let business_date = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let empty = project_same_query(Vec::new()).unwrap();
        assert_eq!(
            empty.witness.source_date_relation(business_date),
            ChainSourceDateRelation::Empty
        );
        assert!(empty.witness.require_qualified_origin().is_err());
        let old = project_same_query(latest_rows()).unwrap();
        assert_eq!(
            old.witness.source_date_relation(business_date),
            ChainSourceDateRelation::Old
        );
        assert_eq!(
            old.witness
                .source_date_relation(NaiveDate::from_ymd_opt(2026, 9, 24).unwrap()),
            ChainSourceDateRelation::Exact
        );
        let invalid =
            project_same_query(vec![row("bad-date", "甲", "TEST_CODE_000001", 1)]).unwrap();
        assert_eq!(
            invalid.witness.source_date_relation(business_date),
            ChainSourceDateRelation::Invalid
        );
        assert!(old.witness.require_qualified_origin().is_err());
    }

    #[test]
    fn mismatched_date_or_tie_order_is_rejected_before_candidate_projection() {
        let mut rows = latest_rows();
        rows.swap(0, 1);
        assert!(project_same_query(rows)
            .unwrap_err()
            .contains("order/identity"));
        let mut rows = latest_rows();
        rows[1].date = "2026-09-23".into();
        assert!(project_same_query(rows)
            .unwrap_err()
            .contains("order/identity"));
    }
}
