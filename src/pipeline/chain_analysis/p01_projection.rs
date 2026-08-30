//! BR-241 exact-date P-01 chain projection from one admitted LimitPools batch.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::data_gateway::{GatewayBatch, ReviewDataGateway};
use crate::database::concepts::ChainDailyRow;
use crate::database::DatabaseManager;
use crate::market_domain::{LimitPoolEntry, LimitPoolKind, ProviderId};

const RECORD_HASH_DOMAIN: &[u8] = b"P01_LIMIT_POOL_RECORD_V1\0";
const CHAIN_ROW_HASH_DOMAIN: &[u8] = b"P01_CHAIN_ROW_V1\0";
const PERSISTENCE_RECEIPT_DOMAIN: &[u8] = b"P01_CHAIN_PERSISTENCE_RECEIPT_V1\0";
const P01_LIMIT_POOL_RECORD_LIMIT: usize = 200;

#[derive(Debug, Error)]
#[error("P-01 chain projection failed reason_code={reason_code} retryable={retryable}: {message}")]
pub struct P01ProjectionError {
    reason_code: &'static str,
    retryable: bool,
    message: String,
}

impl P01ProjectionError {
    fn terminal(reason_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            retryable: false,
            message: message.into(),
        }
    }

    fn transient(reason_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            retryable: true,
            message: message.into(),
        }
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P01ChainProjectionReceipt {
    pub evidence_date: NaiveDate,
    pub limit_pool_batch_id: String,
    pub ordered_limit_pool_record_hashes: Vec<String>,
    pub excluded_record_hashes: Vec<(String, &'static str)>,
    pub ordered_chain_row_hashes: Vec<String>,
    pub persistence_receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct P01CompletedDayEvidence {
    pub limit_pool: GatewayBatch<LimitPoolEntry>,
    /// Exact-date rows already verified against the projection before return.
    pub chain_rows: Vec<ChainDailyRow>,
    pub projection: P01ChainProjectionReceipt,
}

#[derive(Debug)]
struct ProjectedMember {
    code: String,
    streak: Option<u32>,
    change: f64,
    record_hash: String,
}

#[derive(Debug)]
struct ProjectedGroup {
    concept: String,
    members: Vec<ProjectedMember>,
    max_streak: Option<u32>,
    continuation_count: i32,
}

#[derive(Serialize)]
struct ChainRowCanonical<'a> {
    date: &'a str,
    concept: &'a str,
    stocks: &'a [String],
    continuation_count: i32,
}

#[derive(Serialize)]
struct PersistenceReceiptCanonical<'a> {
    evidence_date: &'a str,
    limit_pool_batch_id: &'a str,
    ordered_limit_pool_record_hashes: &'a [String],
    excluded_record_hashes: &'a [(String, &'static str)],
    ordered_chain_row_hashes: &'a [String],
}

/// Persist a deterministic exact-date chain projection from one already-admitted batch.
///
/// This interface has no notification dependency. It rejects any date/batch evidence
/// mismatch, retains every unclassified record as a hash+reason exclusion, atomically
/// replaces only `evidence_date`, then verifies the exact-date read-back.
pub fn persist_p01_chain_from_limit_pool(
    batch: &GatewayBatch<LimitPoolEntry>,
    evidence_date: NaiveDate,
) -> Result<P01ChainProjectionReceipt, P01ProjectionError> {
    persist_p01_chain_artifact(batch, evidence_date).map(|(receipt, _)| receipt)
}

fn persist_p01_chain_artifact(
    batch: &GatewayBatch<LimitPoolEntry>,
    evidence_date: NaiveDate,
) -> Result<(P01ChainProjectionReceipt, Vec<ChainDailyRow>), P01ProjectionError> {
    let evidence_date_text = evidence_date.format("%Y-%m-%d").to_string();
    validate_batch_evidence(batch, evidence_date, &evidence_date_text)?;

    let mut seen_codes = HashSet::with_capacity(batch.records().len());
    let mut grouped: BTreeMap<String, Vec<ProjectedMember>> = BTreeMap::new();
    let mut ordered_limit_pool_record_hashes = Vec::with_capacity(batch.records().len());
    let mut excluded_record_hashes = Vec::new();

    for record in batch.records() {
        validate_record_evidence(record, batch, evidence_date, &evidence_date_text)?;
        let code = record.instrument.code().to_owned();
        if !seen_codes.insert(code.clone()) {
            return Err(P01ProjectionError::terminal(
                "p01_limit_pool_duplicate_code",
                format!("LimitPools contains duplicate code {code}"),
            ));
        }
        let record_hash = hash_serializable(RECORD_HASH_DOMAIN, record).map_err(|error| {
            P01ProjectionError::terminal(
                "p01_limit_pool_record_hash_failed",
                format!("LimitPools record {code} canonical serialization failed: {error}"),
            )
        })?;
        ordered_limit_pool_record_hashes.push((code.clone(), record_hash.clone()));

        let classification = record
            .industry
            .as_ref()
            .or(record.board_name.as_ref())
            .or(record.reason.as_ref())
            .map(|value| value.as_str().to_owned());
        match classification {
            Some(concept) => grouped.entry(concept).or_default().push(ProjectedMember {
                code,
                streak: record.streak.map(|value| value.get()),
                change: record.change.get(),
                record_hash,
            }),
            None => excluded_record_hashes.push((record_hash, "p01_chain_classification_missing")),
        }
    }

    if grouped.is_empty() {
        return Err(P01ProjectionError::terminal(
            "p01_limit_pool_has_no_classified_chain_members",
            format!(
                "LimitPools batch {} has no provider-classified records for {evidence_date}",
                batch.evidence().batch_id
            ),
        ));
    }

    ordered_limit_pool_record_hashes.sort_by(|left, right| left.0.cmp(&right.0));
    let ordered_limit_pool_record_hashes = ordered_limit_pool_record_hashes
        .into_iter()
        .map(|(_, hash)| hash)
        .collect::<Vec<_>>();
    excluded_record_hashes.sort();

    let mut groups = grouped
        .into_iter()
        .map(|(concept, mut members)| {
            members.sort_by(compare_members);
            let max_streak = members.iter().filter_map(|member| member.streak).max();
            let continuation_count = i32::try_from(
                members
                    .iter()
                    .filter(|member| member.streak.is_some_and(|value| value >= 2))
                    .count(),
            )
            .map_err(|_| {
                P01ProjectionError::terminal(
                    "p01_chain_continuation_count_overflow",
                    format!("chain {concept} continuation count exceeds SQLite INTEGER"),
                )
            })?;
            Ok(ProjectedGroup {
                concept,
                members,
                max_streak,
                continuation_count,
            })
        })
        .collect::<Result<Vec<_>, P01ProjectionError>>()?;
    groups.sort_by(|left, right| {
        right
            .members
            .len()
            .cmp(&left.members.len())
            .then_with(|| right.max_streak.cmp(&left.max_streak))
            .then_with(|| left.concept.cmp(&right.concept))
    });

    let rows = groups
        .iter()
        .map(|group| {
            (
                group.concept.clone(),
                group
                    .members
                    .iter()
                    .map(|member| member.code.clone())
                    .collect::<Vec<_>>(),
                group.continuation_count,
            )
        })
        .collect::<Vec<_>>();
    let ordered_chain_row_hashes = rows
        .iter()
        .map(|(concept, stocks, continuation_count)| {
            hash_serializable(
                CHAIN_ROW_HASH_DOMAIN,
                &ChainRowCanonical {
                    date: &evidence_date_text,
                    concept,
                    stocks,
                    continuation_count: *continuation_count,
                },
            )
            .map_err(|error| {
                P01ProjectionError::terminal(
                    "p01_chain_row_hash_failed",
                    format!("chain row {concept} canonical serialization failed: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let database = DatabaseManager::get();
    database
        .replace_chain_clusters_for_date_strict(evidence_date, &rows)
        .map_err(|error| P01ProjectionError::transient("p01_chain_persistence_failed", error))?;
    let read_back = database
        .get_chain_clusters_for_date_strict(evidence_date)
        .map_err(|error| P01ProjectionError::transient("p01_chain_readback_failed", error))?;
    verify_read_back(&rows, &read_back, &evidence_date_text)?;
    let ordered_chain_rows = rows
        .iter()
        .map(|(concept, stocks, continuation_count)| {
            let stocks = serde_json::to_string(stocks).map_err(|error| {
                P01ProjectionError::terminal(
                    "p01_chain_projection_json_failed",
                    format!("chain row {concept} stock serialization failed: {error}"),
                )
            })?;
            Ok(ChainDailyRow {
                date: evidence_date_text.clone(),
                concept: concept.clone(),
                stocks,
                continuation_count: *continuation_count,
            })
        })
        .collect::<Result<Vec<_>, P01ProjectionError>>()?;

    let persistence_receipt_sha256 = hash_serializable(
        PERSISTENCE_RECEIPT_DOMAIN,
        &PersistenceReceiptCanonical {
            evidence_date: &evidence_date_text,
            limit_pool_batch_id: &batch.evidence().batch_id,
            ordered_limit_pool_record_hashes: &ordered_limit_pool_record_hashes,
            excluded_record_hashes: &excluded_record_hashes,
            ordered_chain_row_hashes: &ordered_chain_row_hashes,
        },
    )
    .map_err(|error| {
        P01ProjectionError::terminal(
            "p01_chain_persistence_receipt_failed",
            format!("canonical persistence receipt serialization failed: {error}"),
        )
    })?;

    Ok((
        P01ChainProjectionReceipt {
            evidence_date,
            limit_pool_batch_id: batch.evidence().batch_id.clone(),
            ordered_limit_pool_record_hashes,
            excluded_record_hashes,
            ordered_chain_row_hashes,
            persistence_receipt_sha256,
        },
        ordered_chain_rows,
    ))
}

/// Acquire one exact completed-day LimitPools batch through the registered gateway seam,
/// then persist its deterministic chain projection without exposing a provider bypass.
pub fn acquire_and_persist_p01_chain(
    evidence_date: NaiveDate,
) -> Result<P01CompletedDayEvidence, P01ProjectionError> {
    let limit_pool = ReviewDataGateway::new()
        .current_upper_limit_pool(evidence_date)
        .map_err(|error| P01ProjectionError {
            reason_code: "p01_limit_pool_acquisition_failed",
            retryable: error.retryable(),
            message: format!(
                "LimitPools gateway failed reason_code={}: {}",
                error.reason_code(),
                error.message()
            ),
        })?;
    let (projection, chain_rows) = persist_p01_chain_artifact(&limit_pool, evidence_date)?;
    Ok(P01CompletedDayEvidence {
        limit_pool,
        chain_rows,
        projection,
    })
}

fn validate_batch_evidence(
    batch: &GatewayBatch<LimitPoolEntry>,
    evidence_date: NaiveDate,
    evidence_date_text: &str,
) -> Result<(), P01ProjectionError> {
    if batch.records().len() > P01_LIMIT_POOL_RECORD_LIMIT {
        return Err(P01ProjectionError::terminal(
            "p01_limit_pool_over_limit",
            format!(
                "LimitPools batch contains {} records, exceeding exact request limit {}",
                batch.records().len(),
                P01_LIMIT_POOL_RECORD_LIMIT
            ),
        ));
    }
    let evidence = batch.evidence();
    if !matches!(
        evidence.provider,
        ProviderId::Eastmoney | ProviderId::Tonghuashun
    ) || evidence.source.trim().is_empty()
        || evidence.observed_at.trim().is_empty()
        || evidence.batch_id.trim().is_empty()
        || evidence.source_at.as_deref() != Some(evidence_date_text)
    {
        return Err(P01ProjectionError::terminal(
            "p01_limit_pool_batch_evidence_mismatch",
            format!("LimitPools batch evidence does not bind exact date {evidence_date}"),
        ));
    }
    Ok(())
}

fn validate_record_evidence(
    record: &LimitPoolEntry,
    batch: &GatewayBatch<LimitPoolEntry>,
    evidence_date: NaiveDate,
    evidence_date_text: &str,
) -> Result<(), P01ProjectionError> {
    let record_date =
        NaiveDate::parse_from_str(record.trading_date.as_str(), "%Y-%m-%d").map_err(|error| {
            P01ProjectionError::terminal(
                "p01_limit_pool_record_date_invalid",
                format!("invalid provider trading date: {error}"),
            )
        })?;
    let evidence = batch.evidence();
    if record.kind != LimitPoolKind::Upper
        || record_date != evidence_date
        || record.evidence.provider() != evidence.provider
        || record.evidence.batch_id() != evidence.batch_id
        || record.evidence.source_at() != Some(evidence_date_text)
        || record.evidence.observed_at() != evidence.observed_at
    {
        return Err(P01ProjectionError::terminal(
            "p01_limit_pool_record_evidence_mismatch",
            format!(
                "LimitPools record {} does not bind exact batch/date evidence",
                record.instrument.code()
            ),
        ));
    }
    Ok(())
}

fn compare_members(left: &ProjectedMember, right: &ProjectedMember) -> Ordering {
    right
        .streak
        .cmp(&left.streak)
        .then_with(|| right.change.total_cmp(&left.change))
        .then_with(|| left.code.cmp(&right.code))
        .then_with(|| left.record_hash.cmp(&right.record_hash))
}

fn verify_read_back(
    expected: &[(String, Vec<String>, i32)],
    actual: &[ChainDailyRow],
    evidence_date: &str,
) -> Result<(), P01ProjectionError> {
    if actual.len() != expected.len() || actual.iter().any(|row| row.date != evidence_date) {
        return Err(P01ProjectionError::terminal(
            "p01_chain_readback_mismatch",
            format!(
                "exact-date chain read-back cardinality/date mismatch expected={} actual={}",
                expected.len(),
                actual.len()
            ),
        ));
    }
    let expected = expected
        .iter()
        .map(|(concept, stocks, continuation_count)| {
            (concept.as_str(), (stocks.as_slice(), *continuation_count))
        })
        .collect::<HashMap<_, _>>();
    for row in actual {
        let stocks = serde_json::from_str::<Vec<String>>(&row.stocks).map_err(|error| {
            P01ProjectionError::terminal(
                "p01_chain_readback_invalid_json",
                format!("chain row {} stocks JSON invalid: {error}", row.concept),
            )
        })?;
        match expected.get(row.concept.as_str()) {
            Some((expected_stocks, expected_count))
                if *expected_stocks == stocks.as_slice()
                    && *expected_count == row.continuation_count => {}
            _ => {
                return Err(P01ProjectionError::terminal(
                    "p01_chain_readback_mismatch",
                    format!("chain row {} differs from projection", row.concept),
                ));
            }
        }
    }
    Ok(())
}

fn hash_serializable(domain: &[u8], value: &impl Serialize) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(value)?;
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(bytes);
    Ok(hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use diesel::prelude::*;
    use diesel::sql_types::Text;

    use crate::data_gateway::{BatchEvidence, GatewayBatch};
    use crate::database::DatabaseManager;
    use crate::market_domain::{
        AssetClass, Exchange, InstrumentId, IsoDate, LimitPoolEntry, LimitPoolKind, NonEmptyText,
        PositiveU32, Price, ProviderId, Ratio, RatioUnit, SourceEvidence,
    };

    struct ChainDateGuard(NaiveDate);

    impl Drop for ChainDateGuard {
        fn drop(&mut self) {
            if let Ok(mut conn) = DatabaseManager::get().get_conn() {
                let date = self.0.format("%Y-%m-%d").to_string();
                let _ = diesel::sql_query("DELETE FROM chain_daily WHERE date = ?")
                    .bind::<Text, _>(&date)
                    .execute(&mut conn);
            }
        }
    }

    fn entry(code: &str, industry: Option<&str>, streak: Option<u32>) -> LimitPoolEntry {
        let date = "2198-11-17";
        let batch_id = "TEST_CODE_P01_LIMIT_POOL_BATCH";
        LimitPoolEntry {
            kind: LimitPoolKind::Upper,
            instrument: InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity)
                .expect("valid TEST_CODE instrument"),
            trading_date: IsoDate::new(date).expect("valid evidence date"),
            price: Price::new(10.0).expect("positive provider price"),
            change: Ratio::new(10.0, RatioUnit::Percent).expect("finite provider change"),
            volume: None,
            turnover: None,
            sealed_amount: None,
            first_seal_at: None,
            last_seal_at: None,
            break_count: None,
            streak: streak.map(|value| PositiveU32::new(value).expect("positive streak")),
            industry: industry.map(|value| NonEmptyText::new(value).expect("classification")),
            board_name: None,
            seal_state: None,
            reseal_count: None,
            reason: None,
            evidence: SourceEvidence::new(
                ProviderId::Eastmoney,
                "2198-11-17T15:00:00+08:00",
                batch_id,
            )
            .expect("record evidence")
            .with_source_at(date)
            .expect("record source date"),
        }
    }

    fn complete_limit_pool() -> GatewayBatch<LimitPoolEntry> {
        GatewayBatch::Available {
            records: vec![
                entry("TEST_CODE_600002", Some("TEST_CODE_AI_CHAIN"), Some(1)),
                entry("TEST_CODE_600001", Some("TEST_CODE_AI_CHAIN"), Some(3)),
            ],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_EASTMONEY_LIMIT_POOLS".to_string(),
                source_at: Some("2198-11-17".to_string()),
                observed_at: "2198-11-17T15:00:00+08:00".to_string(),
                batch_id: "TEST_CODE_P01_LIMIT_POOL_BATCH".to_string(),
            },
        }
    }

    fn over_limit_pool() -> GatewayBatch<LimitPoolEntry> {
        GatewayBatch::Available {
            records: (0..=200)
                .map(|index| {
                    entry(
                        &format!("TEST_CODE_{index:06}"),
                        Some("TEST_CODE_AI_CHAIN"),
                        Some(1),
                    )
                })
                .collect(),
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_EASTMONEY_LIMIT_POOLS".to_string(),
                source_at: Some("2198-11-17".to_string()),
                observed_at: "2198-11-17T15:00:00+08:00".to_string(),
                batch_id: "TEST_CODE_P01_LIMIT_POOL_BATCH".to_string(),
            },
        }
    }

    #[test]
    #[serial_test::serial]
    fn p01_projection_uses_only_supplied_limit_pool() {
        DatabaseManager::init(None).expect("test database init");
        let evidence_date = NaiveDate::from_ymd_opt(2198, 11, 17).expect("valid date");
        let _guard = ChainDateGuard(evidence_date);
        DatabaseManager::get()
            .save_chain_clusters(
                "2198-11-17",
                &[(
                    "TEST_CODE_STALE_CHAIN".to_string(),
                    vec!["TEST_CODE_600099".to_string()],
                    9,
                )],
            )
            .expect("seed stale same-date projection");
        let batch = complete_limit_pool();

        let receipt =
            super::persist_p01_chain_from_limit_pool(&batch, evidence_date).expect("projection");

        assert_eq!(receipt.evidence_date, evidence_date);
        assert_eq!(receipt.limit_pool_batch_id, batch.evidence().batch_id);
        let rows = DatabaseManager::get()
            .get_chain_clusters_for_date_strict(evidence_date)
            .expect("read exact persisted projection");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].concept, "TEST_CODE_AI_CHAIN");
        assert_eq!(rows[0].continuation_count, 1);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&rows[0].stocks).expect("stock codes JSON"),
            vec!["TEST_CODE_600001", "TEST_CODE_600002"]
        );
    }

    #[test]
    fn p01_projection_rejects_more_than_the_requested_limit() {
        let error = super::persist_p01_chain_from_limit_pool(
            &over_limit_pool(),
            NaiveDate::from_ymd_opt(2198, 11, 17).unwrap(),
        )
        .expect_err("201 LimitPools records must not pass the exact 200-record request bound");

        assert_eq!(error.reason_code(), "p01_limit_pool_over_limit");
        assert!(!error.retryable());
    }

    #[test]
    #[serial_test::serial]
    fn p01_projection_uses_provider_classification_precedence_and_retains_exclusions() {
        DatabaseManager::init(None).expect("test database init");
        let evidence_date = NaiveDate::from_ymd_opt(2198, 11, 17).expect("valid date");
        let _guard = ChainDateGuard(evidence_date);
        let mut industry = entry("TEST_CODE_600011", Some("TEST_CODE_INDUSTRY"), Some(2));
        industry.board_name = Some(NonEmptyText::new("TEST_CODE_IGNORED_BOARD").unwrap());
        industry.reason = Some(NonEmptyText::new("TEST_CODE_IGNORED_REASON").unwrap());
        let mut board = entry("TEST_CODE_600012", None, Some(1));
        board.board_name = Some(NonEmptyText::new("TEST_CODE_BOARD").unwrap());
        board.reason = Some(NonEmptyText::new("TEST_CODE_IGNORED_REASON").unwrap());
        let mut reason = entry("TEST_CODE_600013", None, None);
        reason.reason = Some(NonEmptyText::new("TEST_CODE_REASON").unwrap());
        let unclassified = entry("TEST_CODE_600014", None, None);
        let batch = GatewayBatch::Available {
            records: vec![unclassified, reason, board, industry],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_EASTMONEY_LIMIT_POOLS".to_string(),
                source_at: Some("2198-11-17".to_string()),
                observed_at: "2198-11-17T15:00:00+08:00".to_string(),
                batch_id: "TEST_CODE_P01_LIMIT_POOL_BATCH".to_string(),
            },
        };

        let receipt =
            super::persist_p01_chain_from_limit_pool(&batch, evidence_date).expect("projection");

        assert_eq!(receipt.excluded_record_hashes.len(), 1);
        assert_eq!(
            receipt.excluded_record_hashes[0].1,
            "p01_chain_classification_missing"
        );
        let rows = DatabaseManager::get()
            .get_chain_clusters_for_date_strict(evidence_date)
            .expect("read exact persisted projection");
        let concepts = rows
            .iter()
            .map(|row| row.concept.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            concepts,
            std::collections::HashSet::from([
                "TEST_CODE_INDUSTRY",
                "TEST_CODE_BOARD",
                "TEST_CODE_REASON",
            ])
        );
        assert!(!concepts.contains("TEST_CODE_IGNORED_BOARD"));
        assert!(!concepts.contains("TEST_CODE_IGNORED_REASON"));
    }

    #[test]
    #[serial_test::serial]
    fn p01_projection_fails_when_no_record_has_provider_classification() {
        DatabaseManager::init(None).expect("test database init");
        let evidence_date = NaiveDate::from_ymd_opt(2198, 11, 17).expect("valid date");
        let _guard = ChainDateGuard(evidence_date);
        let batch = GatewayBatch::Available {
            records: vec![entry("TEST_CODE_600021", None, Some(1))],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_EASTMONEY_LIMIT_POOLS".to_string(),
                source_at: Some("2198-11-17".to_string()),
                observed_at: "2198-11-17T15:00:00+08:00".to_string(),
                batch_id: "TEST_CODE_P01_LIMIT_POOL_BATCH".to_string(),
            },
        };

        let error = super::persist_p01_chain_from_limit_pool(&batch, evidence_date)
            .expect_err("unclassified batch must fail");

        assert_eq!(
            error.reason_code(),
            "p01_limit_pool_has_no_classified_chain_members"
        );
        assert!(DatabaseManager::get()
            .get_chain_clusters_for_date_strict(evidence_date)
            .expect("read exact date after rejection")
            .is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn p01_projection_rejects_record_outside_exact_evidence_date() {
        DatabaseManager::init(None).expect("test database init");
        let evidence_date = NaiveDate::from_ymd_opt(2198, 11, 17).expect("valid date");
        let _guard = ChainDateGuard(evidence_date);
        let mut wrong_date = entry("TEST_CODE_600022", Some("TEST_CODE_DATE_MISMATCH"), Some(1));
        wrong_date.trading_date = IsoDate::new("2198-11-16").expect("valid other date");
        wrong_date.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2198-11-17T15:00:00+08:00",
            "TEST_CODE_P01_LIMIT_POOL_BATCH",
        )
        .expect("record evidence")
        .with_source_at("2198-11-16")
        .expect("record source date");
        let batch = GatewayBatch::Available {
            records: vec![wrong_date],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_EASTMONEY_LIMIT_POOLS".to_string(),
                source_at: Some("2198-11-17".to_string()),
                observed_at: "2198-11-17T15:00:00+08:00".to_string(),
                batch_id: "TEST_CODE_P01_LIMIT_POOL_BATCH".to_string(),
            },
        };

        let error = super::persist_p01_chain_from_limit_pool(&batch, evidence_date)
            .expect_err("record outside exact evidence date must fail");

        assert_eq!(
            error.reason_code(),
            "p01_limit_pool_record_evidence_mismatch"
        );
        assert!(DatabaseManager::get()
            .get_chain_clusters_for_date_strict(evidence_date)
            .expect("read exact date after rejection")
            .is_empty());
    }
}
