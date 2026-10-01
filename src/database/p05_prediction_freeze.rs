//! P-05 prediction-row freeze for a future `candidate-board-v2` counted producer.
//!
//! A frozen row records producer membership. It is not counted admission or a
//! physical delivery receipt; historical `candidate-board-v1` stays unlinked.

use super::DatabaseManager;
use crate::monitor::prediction::CandidateSampleSaveReport;
use chrono::{NaiveDate, NaiveTime};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Nullable, Text};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const TABLE: &str = "candidate_board_prediction_freeze_v2";
const MEMBER_TABLE: &str = "candidate_board_prediction_member_v2";

#[derive(Debug, thiserror::Error)]
pub enum CandidateBoardFreezeError {
    #[error("P05 freeze invalid: {0}")]
    Invalid(String),
    #[error("P05 freeze calendar unavailable: {0}")]
    Calendar(String),
    #[error("P05 freeze database: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("P05 freeze serialization: {0}")]
    Serialization(#[from] serde_json::Error),
}

type FreezeResult<T> = Result<T, CandidateBoardFreezeError>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenCandidateRow {
    prediction_row_id: i64,
    code: String,
}

impl FrozenCandidateRow {
    pub fn prediction_row_id(&self) -> i64 {
        self.prediction_row_id
    }

    pub fn code(&self) -> &str {
        &self.code
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateBoardSourceV2 {
    schema: String,
    business_date: String,
    occurrence_identity: String,
    target_date: String,
    calendar_authority_hash: String,
    /// Exact T0..T+5 vector, checked again against the current calendar.
    trading_dates: Vec<String>,
    rendered_sha256: String,
    ordered_rows: Vec<FrozenCandidateRow>,
}

/// Exact, committed producer bytes. This value alone does not authorize send.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenCandidateBoardV2 {
    business_date: String,
    occurrence_identity: String,
    target_date: String,
    calendar_authority_hash: String,
    trading_dates: Vec<String>,
    rendered_bytes: Vec<u8>,
    rendered_sha256: String,
    source_canonical: Vec<u8>,
    source_sha256: String,
    ordered_rows: Vec<FrozenCandidateRow>,
}

impl FrozenCandidateBoardV2 {
    pub fn business_date(&self) -> &str {
        &self.business_date
    }

    pub fn occurrence_identity(&self) -> &str {
        &self.occurrence_identity
    }

    pub fn target_date(&self) -> &str {
        &self.target_date
    }

    pub fn calendar_authority_hash(&self) -> &str {
        &self.calendar_authority_hash
    }

    pub fn trading_dates(&self) -> &[String] {
        &self.trading_dates
    }

    pub fn rendered_bytes(&self) -> &[u8] {
        &self.rendered_bytes
    }

    pub fn rendered_sha256(&self) -> &str {
        &self.rendered_sha256
    }

    pub fn source_canonical(&self) -> &[u8] {
        &self.source_canonical
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn ordered_rows(&self) -> &[FrozenCandidateRow] {
        &self.ordered_rows
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateBoardFreezeOutcome {
    inserted: bool,
    record: FrozenCandidateBoardV2,
}

impl CandidateBoardFreezeOutcome {
    pub fn inserted(&self) -> bool {
        self.inserted
    }

    pub fn record(&self) -> &FrozenCandidateBoardV2 {
        &self.record
    }

    pub fn into_record(self) -> FrozenCandidateBoardV2 {
        self.record
    }
}

pub(super) fn create_schema(conn: &mut diesel::sqlite::SqliteConnection) -> Result<(), String> {
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS candidate_board_prediction_freeze_v2 (
            occurrence_identity TEXT PRIMARY KEY NOT NULL,
            business_date TEXT NOT NULL,
            target_date TEXT NOT NULL,
            calendar_authority_hash TEXT NOT NULL CHECK(length(calendar_authority_hash) = 64),
            rendered_bytes BLOB NOT NULL,
            rendered_sha256 TEXT NOT NULL CHECK(length(rendered_sha256) = 64),
            source_canonical BLOB NOT NULL,
            source_sha256 TEXT NOT NULL CHECK(length(source_sha256) = 64)
        );
        CREATE TRIGGER IF NOT EXISTS trg_candidate_board_prediction_freeze_v2_no_update
        BEFORE UPDATE ON candidate_board_prediction_freeze_v2 BEGIN
            SELECT RAISE(ABORT, 'P05 prediction freeze is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS trg_candidate_board_prediction_freeze_v2_no_delete
        BEFORE DELETE ON candidate_board_prediction_freeze_v2 BEGIN
            SELECT RAISE(ABORT, 'P05 prediction freeze is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS trg_candidate_board_prediction_freeze_v2_no_replace
        BEFORE INSERT ON candidate_board_prediction_freeze_v2
        WHEN EXISTS (
            SELECT 1 FROM candidate_board_prediction_freeze_v2
            WHERE occurrence_identity = NEW.occurrence_identity
        ) BEGIN
            SELECT RAISE(ABORT, 'P05 prediction freeze occurrence already exists');
        END;
        CREATE TABLE IF NOT EXISTS candidate_board_prediction_member_v2 (
            prediction_row_id INTEGER PRIMARY KEY NOT NULL,
            occurrence_identity TEXT NOT NULL,
            ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
            code TEXT NOT NULL,
            UNIQUE(occurrence_identity, ordinal),
            FOREIGN KEY(prediction_row_id) REFERENCES prediction_tracker(id) ON DELETE RESTRICT,
            FOREIGN KEY(occurrence_identity)
                REFERENCES candidate_board_prediction_freeze_v2(occurrence_identity)
                ON DELETE RESTRICT
        );
        CREATE TRIGGER IF NOT EXISTS trg_candidate_board_prediction_member_v2_no_update
        BEFORE UPDATE ON candidate_board_prediction_member_v2 BEGIN
            SELECT RAISE(ABORT, 'P05 prediction member is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS trg_candidate_board_prediction_member_v2_no_delete
        BEFORE DELETE ON candidate_board_prediction_member_v2 BEGIN
            SELECT RAISE(ABORT, 'P05 prediction member is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS trg_candidate_board_prediction_member_v2_no_replace
        BEFORE INSERT ON candidate_board_prediction_member_v2
        WHEN EXISTS (
            SELECT 1 FROM candidate_board_prediction_member_v2
            WHERE prediction_row_id = NEW.prediction_row_id
               OR (occurrence_identity = NEW.occurrence_identity AND ordinal = NEW.ordinal)
        ) BEGIN
            SELECT RAISE(ABORT, 'P05 prediction row or member slot already owned');
        END;",
    )
    .map_err(|error| error.to_string())
}

impl DatabaseManager {
    /// First committed occurrence owns its row IDs and exact card/source bytes.
    /// A retry must present a complete save report with DB-matching rows and the same Strong
    /// code sequence and card; its newly saved IDs remain ordinary samples.
    #[allow(clippy::too_many_arguments)]
    pub fn freeze_candidate_board_v2(
        &self,
        business_date: &str,
        occurrence_identity: &str,
        target_date: &str,
        rendered_bytes: &[u8],
        expected_strong_codes: &[String],
        report: &CandidateSampleSaveReport,
    ) -> FreezeResult<CandidateBoardFreezeOutcome> {
        let calendar = validate_request(
            business_date,
            occurrence_identity,
            target_date,
            rendered_bytes,
        )?;
        let rows = validate_report(expected_strong_codes, report)?;
        let mut conn = self.get_conn().map_err(|error| {
            CandidateBoardFreezeError::Invalid(format!("DB connection unavailable: {error}"))
        })?;
        conn.immediate_transaction::<_, CandidateBoardFreezeError, _>(|conn| {
            verify_saved_rows(conn, business_date, target_date, &rows)?;
            if let Some(existing) = load_verified(conn, occurrence_identity)? {
                let frozen_codes: Vec<&str> = existing
                    .ordered_rows
                    .iter()
                    .map(|row| row.code.as_str())
                    .collect();
                let expected_codes: Vec<&str> =
                    expected_strong_codes.iter().map(String::as_str).collect();
                if existing.business_date != business_date
                    || existing.target_date != target_date
                    || existing.rendered_bytes != rendered_bytes
                    || frozen_codes != expected_codes
                {
                    return Err(invalid(
                        "existing occurrence conflicts with current Strong card",
                    ));
                }
                return Ok(CandidateBoardFreezeOutcome {
                    inserted: false,
                    record: existing,
                });
            }

            let rendered_sha256 = sha256(rendered_bytes);
            let source = CandidateBoardSourceV2 {
                schema: "candidate-board-v2".to_owned(),
                business_date: business_date.to_owned(),
                occurrence_identity: occurrence_identity.to_owned(),
                target_date: target_date.to_owned(),
                calendar_authority_hash: calendar.authority_hash.clone(),
                trading_dates: calendar.trading_dates.clone(),
                rendered_sha256: rendered_sha256.clone(),
                ordered_rows: rows,
            };
            let source_canonical = serde_json::to_vec(&source)?;
            let record = FrozenCandidateBoardV2 {
                business_date: business_date.to_owned(),
                occurrence_identity: occurrence_identity.to_owned(),
                target_date: target_date.to_owned(),
                calendar_authority_hash: calendar.authority_hash,
                trading_dates: calendar.trading_dates,
                rendered_bytes: rendered_bytes.to_vec(),
                rendered_sha256,
                source_sha256: sha256(&source_canonical),
                source_canonical,
                ordered_rows: source.ordered_rows,
            };
            diesel::sql_query(format!(
                "INSERT INTO {TABLE} (occurrence_identity,business_date,target_date,
                    calendar_authority_hash,rendered_bytes,rendered_sha256,
                    source_canonical,source_sha256) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"
            ))
            .bind::<Text, _>(&record.occurrence_identity)
            .bind::<Text, _>(&record.business_date)
            .bind::<Text, _>(&record.target_date)
            .bind::<Text, _>(&record.calendar_authority_hash)
            .bind::<Binary, _>(&record.rendered_bytes)
            .bind::<Text, _>(&record.rendered_sha256)
            .bind::<Binary, _>(&record.source_canonical)
            .bind::<Text, _>(&record.source_sha256)
            .execute(conn)?;
            for (ordinal, member) in record.ordered_rows.iter().enumerate() {
                let ordinal = i64::try_from(ordinal)
                    .map_err(|_| invalid("Strong membership ordinal overflow"))?;
                diesel::sql_query(format!(
                    "INSERT INTO {MEMBER_TABLE}
                     (prediction_row_id,occurrence_identity,ordinal,code)
                     VALUES (?1,?2,?3,?4)"
                ))
                .bind::<BigInt, _>(member.prediction_row_id)
                .bind::<Text, _>(&record.occurrence_identity)
                .bind::<BigInt, _>(ordinal)
                .bind::<Text, _>(&member.code)
                .execute(conn)?;
            }
            // Read on the writer transaction: an inconsistent insertion rolls
            // back instead of returning an ambiguous committed identity.
            let committed = load_verified(conn, occurrence_identity)?
                .ok_or_else(|| invalid("inserted freeze is not readable"))?;
            if committed != record {
                return Err(invalid("inserted freeze differs from requested bytes"));
            }
            Ok(CandidateBoardFreezeOutcome {
                inserted: true,
                record: committed,
            })
        })
    }

    /// Revalidates the stored v2 source, T+5 authority, and member row facts
    /// inside one read snapshot. `Some` is producer evidence only.
    pub fn read_candidate_board_v2_freeze(
        &self,
        occurrence_identity: &str,
    ) -> FreezeResult<Option<FrozenCandidateBoardV2>> {
        validate_occurrence(occurrence_identity)?;
        let mut conn = self.get_conn().map_err(|error| {
            CandidateBoardFreezeError::Invalid(format!("DB connection unavailable: {error}"))
        })?;
        conn.transaction::<_, CandidateBoardFreezeError, _>(|conn| {
            load_verified(conn, occurrence_identity)
        })
    }
}

#[derive(QueryableByName)]
struct StoredFreeze {
    #[diesel(sql_type = Text)]
    occurrence_identity: String,
    #[diesel(sql_type = Text)]
    business_date: String,
    #[diesel(sql_type = Text)]
    target_date: String,
    #[diesel(sql_type = Text)]
    calendar_authority_hash: String,
    #[diesel(sql_type = Binary)]
    rendered_bytes: Vec<u8>,
    #[diesel(sql_type = Text)]
    rendered_sha256: String,
    #[diesel(sql_type = Binary)]
    source_canonical: Vec<u8>,
    #[diesel(sql_type = Text)]
    source_sha256: String,
}

fn load_verified(
    conn: &mut diesel::sqlite::SqliteConnection,
    occurrence_identity: &str,
) -> FreezeResult<Option<FrozenCandidateBoardV2>> {
    let stored = diesel::sql_query(format!(
        "SELECT occurrence_identity,business_date,target_date,calendar_authority_hash,
                rendered_bytes,rendered_sha256,source_canonical,source_sha256
         FROM {TABLE} WHERE occurrence_identity=?1"
    ))
    .bind::<Text, _>(occurrence_identity)
    .get_result::<StoredFreeze>(conn)
    .optional()?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let calendar = validate_request(
        &stored.business_date,
        &stored.occurrence_identity,
        &stored.target_date,
        &stored.rendered_bytes,
    )?;
    if stored.occurrence_identity != occurrence_identity
        || !valid_sha256_text(&stored.calendar_authority_hash)
        || stored.rendered_sha256 != sha256(&stored.rendered_bytes)
        || stored.source_sha256 != sha256(&stored.source_canonical)
    {
        return Err(invalid(
            "stored freeze identity, authority, or hash mismatch",
        ));
    }
    let source: CandidateBoardSourceV2 = serde_json::from_slice(&stored.source_canonical)?;
    if source.schema != "candidate-board-v2"
        || source.business_date != stored.business_date
        || source.occurrence_identity != stored.occurrence_identity
        || source.target_date != stored.target_date
        || source.calendar_authority_hash != stored.calendar_authority_hash
        || source.trading_dates != calendar.trading_dates
        || source.rendered_sha256 != stored.rendered_sha256
        || serde_json::to_vec(&source)? != stored.source_canonical
    {
        return Err(invalid(
            "stored source is not exact candidate-board-v2 canonical",
        ));
    }
    verify_stored_members(conn, &stored.occurrence_identity, &source.ordered_rows)?;
    verify_saved_rows(
        conn,
        &stored.business_date,
        &stored.target_date,
        &source.ordered_rows,
    )?;
    Ok(Some(FrozenCandidateBoardV2 {
        business_date: stored.business_date,
        occurrence_identity: stored.occurrence_identity,
        target_date: stored.target_date,
        calendar_authority_hash: stored.calendar_authority_hash,
        trading_dates: source.trading_dates,
        rendered_bytes: stored.rendered_bytes,
        rendered_sha256: stored.rendered_sha256,
        source_canonical: stored.source_canonical,
        source_sha256: stored.source_sha256,
        ordered_rows: source.ordered_rows,
    }))
}

#[derive(QueryableByName)]
struct StoredPredictionRow {
    #[diesel(sql_type = Text)]
    pred_date: String,
    #[diesel(sql_type = Text)]
    target_date: String,
    #[diesel(sql_type = Nullable<Text>)]
    stock_code: Option<String>,
    #[diesel(sql_type = Text)]
    pred_direction: String,
    #[diesel(sql_type = Nullable<Text>)]
    pred_detail: Option<String>,
}

#[derive(QueryableByName)]
struct StoredMember {
    #[diesel(sql_type = BigInt)]
    prediction_row_id: i64,
    #[diesel(sql_type = BigInt)]
    ordinal: i64,
    #[diesel(sql_type = Text)]
    code: String,
}

fn verify_stored_members(
    conn: &mut diesel::sqlite::SqliteConnection,
    occurrence_identity: &str,
    canonical_rows: &[FrozenCandidateRow],
) -> FreezeResult<()> {
    let members = diesel::sql_query(format!(
        "SELECT prediction_row_id,ordinal,code FROM {MEMBER_TABLE}
         WHERE occurrence_identity=?1 ORDER BY ordinal ASC"
    ))
    .bind::<Text, _>(occurrence_identity)
    .load::<StoredMember>(conn)?;
    if members.len() != canonical_rows.len()
        || members
            .iter()
            .zip(canonical_rows)
            .enumerate()
            .any(|(ordinal, (stored, canonical))| {
                i64::try_from(ordinal).ok() != Some(stored.ordinal)
                    || stored.prediction_row_id != canonical.prediction_row_id
                    || stored.code != canonical.code
            })
    {
        return Err(invalid("stored member rows differ from canonical source"));
    }
    Ok(())
}

fn verify_saved_rows(
    conn: &mut diesel::sqlite::SqliteConnection,
    business_date: &str,
    target_date: &str,
    rows: &[FrozenCandidateRow],
) -> FreezeResult<()> {
    if rows.is_empty() {
        return Err(invalid("Strong prediction membership is empty"));
    }
    let mut seen = BTreeSet::new();
    let mut previous_id = 0;
    for member in rows {
        if member.prediction_row_id <= previous_id
            || !seen.insert(member.code.as_str())
            || super::validate_evidence_code(&member.code).is_err()
        {
            return Err(invalid(
                "Strong prediction membership order/code is invalid",
            ));
        }
        previous_id = member.prediction_row_id;
        let row = diesel::sql_query(
            "SELECT pred_date,target_date,stock_code,pred_direction,pred_detail
             FROM prediction_tracker WHERE id=?1",
        )
        .bind::<BigInt, _>(member.prediction_row_id)
        .get_result::<StoredPredictionRow>(conn)
        .optional()?
        .ok_or_else(|| invalid("prediction member row is missing"))?;
        if row.pred_date != business_date
            || row.target_date != target_date
            || row.stock_code.as_deref() != Some(member.code.as_str())
            || row.pred_direction != "up"
            || row.pred_detail.as_deref() != Some("candidate-strong")
        {
            return Err(invalid("prediction member facts differ from frozen source"));
        }
    }
    Ok(())
}

fn validate_report(
    expected_strong_codes: &[String],
    report: &CandidateSampleSaveReport,
) -> FreezeResult<Vec<FrozenCandidateRow>> {
    if expected_strong_codes.is_empty()
        || report.attempted != expected_strong_codes.len()
        || report.saved != report.attempted
        || report.saved_rows.len() != report.saved
        || report.unknown != 0
        || !report.failures.is_empty()
        || report.worker_error.is_some()
    {
        return Err(invalid(
            "Strong sample save report is incomplete or unknown",
        ));
    }
    report
        .saved_rows
        .iter()
        .zip(expected_strong_codes)
        .map(|(saved, expected)| {
            if saved.code != *expected {
                return Err(invalid("Strong sample report code order mismatch"));
            }
            Ok(FrozenCandidateRow {
                prediction_row_id: saved.prediction_row_id,
                code: saved.code.clone(),
            })
        })
        .collect()
}

fn validate_request(
    business_date: &str,
    occurrence_identity: &str,
    target_date: &str,
    rendered_bytes: &[u8],
) -> FreezeResult<CalendarWitness> {
    let date = canonical_date(business_date)?;
    let target = canonical_date(target_date)?;
    if !crate::calendar::verified_a_share_trading_day(date)
        .map_err(CandidateBoardFreezeError::Calendar)?
    {
        return Err(invalid("business date is not a verified trading day"));
    }
    let mut expected = date;
    let mut trading_dates = vec![business_date.to_owned()];
    for _ in 0..5 {
        expected = crate::calendar::verified_next_a_share_trading_day(expected)
            .map_err(CandidateBoardFreezeError::Calendar)?;
        trading_dates.push(expected.format("%Y-%m-%d").to_string());
    }
    if target != expected {
        return Err(invalid("target is not the verified fifth trading day"));
    }
    if occurrence_identity
        != format!(
            "candidate-board:{business_date}:{}",
            validate_occurrence(occurrence_identity)?
        )
    {
        return Err(invalid("occurrence business date mismatch"));
    }
    if rendered_bytes.is_empty() || std::str::from_utf8(rendered_bytes).is_err() {
        return Err(invalid("rendered card is empty or not UTF-8"));
    }
    crate::calendar::verified_a_share_calendar_authority_hash(date)
        .map(|authority_hash| CalendarWitness {
            authority_hash: authority_hash.to_owned(),
            trading_dates,
        })
        .map_err(CandidateBoardFreezeError::Calendar)
}

struct CalendarWitness {
    /// Creation-time whole-file provenance. It may change when unrelated
    /// calendar years or comments change; the exact date vector is rechecked.
    authority_hash: String,
    trading_dates: Vec<String>,
}

fn valid_sha256_text(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_occurrence(occurrence_identity: &str) -> FreezeResult<String> {
    let suffix = occurrence_identity
        .strip_prefix("candidate-board:")
        .ok_or_else(|| invalid("occurrence family mismatch"))?;
    let (date, hhmm) = suffix
        .split_once(':')
        .ok_or_else(|| invalid("occurrence shape mismatch"))?;
    canonical_date(date)?;
    let parsed =
        NaiveTime::parse_from_str(hhmm, "%H:%M").map_err(|_| invalid("occurrence time invalid"))?;
    if hhmm.len() != 5 || parsed.format("%H:%M").to_string() != hhmm {
        return Err(invalid("occurrence time is not canonical"));
    }
    Ok(hhmm.to_owned())
}

fn canonical_date(value: &str) -> FreezeResult<NaiveDate> {
    let date =
        NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid("date is invalid"))?;
    if date.format("%Y-%m-%d").to_string() != value {
        return Err(invalid("date is not canonical"));
    }
    Ok(date)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn invalid(message: &'static str) -> CandidateBoardFreezeError {
    CandidateBoardFreezeError::Invalid(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::prediction::save_candidate_samples;
    use std::sync::{Arc, Barrier};

    const DATE: &str = "2026-09-23";
    const TARGET: &str = "2026-10-08";
    const OCCURRENCE: &str = "candidate-board:2026-09-23:10:30";

    fn private_db() -> (tempfile::TempDir, DatabaseManager) {
        let dir = tempfile::tempdir().unwrap();
        let db =
            DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_p05_freeze.db"))
                .unwrap();
        (dir, db)
    }

    fn report(db: &DatabaseManager, codes: &[String]) -> CandidateSampleSaveReport {
        let samples: Vec<(String, f64)> = codes.iter().map(|code| (code.clone(), 80.0)).collect();
        save_candidate_samples(db, DATE, TARGET, &samples)
    }

    fn codes() -> Vec<String> {
        vec!["TEST_CODE_p05_a".to_owned(), "TEST_CODE_p05_b".to_owned()]
    }

    #[test]
    fn p05_freeze_reopens_and_exact_retry_keeps_first_row_membership() {
        let (dir, db) = private_db();
        let codes = codes();
        let first_report = report(&db, &codes);
        let first = db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &first_report,
            )
            .unwrap();
        assert!(first.inserted);
        assert_eq!(first.record.ordered_rows.len(), 2);
        assert_eq!(
            first
                .record
                .trading_dates
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "2026-09-23",
                "2026-09-24",
                "2026-09-28",
                "2026-09-29",
                "2026-09-30",
                "2026-10-08",
            ]
        );
        assert_eq!(first.record.rendered_sha256, sha256(b"P05 exact card"));
        assert_eq!(
            first.record.source_sha256,
            sha256(&first.record.source_canonical)
        );
        assert!(first
            .record
            .source_canonical
            .starts_with(b"{\"schema\":\"candidate-board-v2\""));
        drop(db);

        let reopened =
            DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_p05_freeze.db"))
                .unwrap();
        assert_eq!(
            reopened.read_candidate_board_v2_freeze(OCCURRENCE).unwrap(),
            Some(first.record.clone())
        );
        let retry_report = report(&reopened, &codes);
        assert_ne!(first_report.saved_rows, retry_report.saved_rows);
        let retry = reopened
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &retry_report,
            )
            .unwrap();
        assert!(!retry.inserted);
        assert_eq!(retry.record, first.record);
        assert!(reopened
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 changed card",
                &codes,
                &retry_report,
            )
            .is_err());
    }

    #[test]
    fn p05_freeze_rejects_incomplete_report_even_when_first_writer_exists() {
        let (_dir, db) = private_db();
        let codes = codes();
        let complete = report(&db, &codes);
        for variant in 0..5 {
            let mut invalid_report = complete.clone();
            match variant {
                0 => invalid_report.unknown = 1,
                1 => {
                    let failed =
                        save_candidate_samples(&db, DATE, TARGET, &[("BAD CODE".to_owned(), 80.0)]);
                    assert_eq!(failed.failures.len(), 1);
                    invalid_report.failures = failed.failures;
                }
                2 => invalid_report.worker_error = Some("TEST_CODE worker failed".to_owned()),
                3 => invalid_report.saved -= 1,
                _ => invalid_report.saved_rows.swap(0, 1),
            }
            assert!(db
                .freeze_candidate_board_v2(
                    DATE,
                    OCCURRENCE,
                    TARGET,
                    b"P05 exact card",
                    &codes,
                    &invalid_report,
                )
                .is_err());
        }
        assert!(db
            .read_candidate_board_v2_freeze(OCCURRENCE)
            .unwrap()
            .is_none());
        db.freeze_candidate_board_v2(
            DATE,
            OCCURRENCE,
            TARGET,
            b"P05 exact card",
            &codes,
            &complete,
        )
        .unwrap();
        let mut failed_retry = report(&db, &codes);
        failed_retry.unknown = 1;
        assert!(db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &failed_retry,
            )
            .is_err());
    }

    #[test]
    fn p05_freeze_rejects_unverified_target_and_rolls_back_insert_failure() {
        let (_dir, db) = private_db();
        let codes = codes();
        let complete = report(&db, &codes);
        let wrong_target = db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                "2026-10-09",
                b"P05 exact card",
                &codes,
                &complete,
            )
            .unwrap_err()
            .to_string();
        assert!(wrong_target.contains("fifth trading day"), "{wrong_target}");

        let mut conn = db.get_conn().unwrap();
        conn.batch_execute(&format!(
            "CREATE TRIGGER TEST_CODE_p05_abort AFTER INSERT ON {TABLE} BEGIN
                 SELECT RAISE(ABORT, 'TEST_CODE abort freeze insert');
             END;"
        ))
        .unwrap();
        drop(conn);
        assert!(db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &complete,
            )
            .is_err());
        assert!(db
            .read_candidate_board_v2_freeze(OCCURRENCE)
            .unwrap()
            .is_none());
        let mut conn = db.get_conn().unwrap();
        conn.batch_execute("DROP TRIGGER TEST_CODE_p05_abort")
            .unwrap();
        drop(conn);
        assert!(
            db.freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &complete,
            )
            .unwrap()
            .inserted
        );
    }

    #[test]
    fn p05_freeze_two_connections_allow_one_conflicting_first_writer() {
        let (dir, db1) = private_db();
        let db2 =
            DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_p05_freeze.db"))
                .unwrap();
        let codes = codes();
        let report1 = report(&db1, &codes);
        let report2 = report(&db2, &codes);
        let retry_report1 = report1.clone();
        let retry_report2 = report2.clone();
        let barrier = Arc::new(Barrier::new(2));
        let codes2 = codes.clone();
        let retry_codes = codes.clone();
        let barrier2 = Arc::clone(&barrier);
        let first = std::thread::spawn(move || {
            barrier.wait();
            db1.freeze_candidate_board_v2(DATE, OCCURRENCE, TARGET, b"first card", &codes, &report1)
        });
        let second = std::thread::spawn(move || {
            barrier2.wait();
            db2.freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"second card",
                &codes2,
                &report2,
            )
        });
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let first_won = first.is_ok();
        let winning = first.or(second).unwrap().record;
        let reader =
            DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_p05_freeze.db"))
                .unwrap();
        assert_eq!(
            reader.read_candidate_board_v2_freeze(OCCURRENCE).unwrap(),
            Some(winning)
        );
        let (losing_card, losing_report) = if first_won {
            (b"second card".as_slice(), &retry_report2)
        } else {
            (b"first card".as_slice(), &retry_report1)
        };
        let retry_error = reader
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                losing_card,
                &retry_codes,
                losing_report,
            )
            .unwrap_err()
            .to_string();
        assert!(
            retry_error.contains("existing occurrence conflicts"),
            "{retry_error}"
        );
    }

    #[test]
    fn p05_freeze_one_prediction_row_cannot_belong_to_another_occurrence() {
        let (_dir, db) = private_db();
        let codes = codes();
        let complete = report(&db, &codes);
        let original = db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &complete,
            )
            .unwrap()
            .into_record();
        let other_occurrence = "candidate-board:2026-09-23:10:31";
        let error = db
            .freeze_candidate_board_v2(
                DATE,
                other_occurrence,
                TARGET,
                b"another occurrence card",
                &codes,
                &complete,
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("already owned") || error.contains("UNIQUE"),
            "{error}"
        );
        assert!(db
            .read_candidate_board_v2_freeze(other_occurrence)
            .unwrap()
            .is_none());
        assert_eq!(
            db.read_candidate_board_v2_freeze(OCCURRENCE).unwrap(),
            Some(original)
        );
    }

    #[test]
    fn p05_freeze_second_member_failure_rolls_back_header_and_first_member() {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }

        let (_dir, db) = private_db();
        let codes = codes();
        let complete = report(&db, &codes);
        let mut conn = db.get_conn().unwrap();
        conn.batch_execute(&format!(
            "CREATE TRIGGER TEST_CODE_p05_abort_second_member
             BEFORE INSERT ON {MEMBER_TABLE} WHEN NEW.ordinal = 1 BEGIN
                 SELECT RAISE(ABORT, 'TEST_CODE second member failure');
             END;"
        ))
        .unwrap();
        drop(conn);

        let error = db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &complete,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("second member failure"), "{error}");
        let mut conn = db.get_conn().unwrap();
        let header_count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {TABLE}"))
            .get_result::<Count>(&mut *conn)
            .unwrap()
            .count;
        let member_count =
            diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {MEMBER_TABLE}"))
                .get_result::<Count>(&mut *conn)
                .unwrap()
                .count;
        assert_eq!((header_count, member_count), (0, 0));
        conn.batch_execute("DROP TRIGGER TEST_CODE_p05_abort_second_member")
            .unwrap();
        drop(conn);
        assert!(db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &complete,
            )
            .unwrap()
            .inserted());
    }

    #[test]
    fn p05_freeze_rejects_stored_canonical_and_member_tamper() {
        let (_dir, db) = private_db();
        let codes = codes();
        let complete = report(&db, &codes);
        let record = db
            .freeze_candidate_board_v2(
                DATE,
                OCCURRENCE,
                TARGET,
                b"P05 exact card",
                &codes,
                &complete,
            )
            .unwrap()
            .record;
        let mut conn = db.get_conn().unwrap();
        assert!(diesel::sql_query(format!(
            "UPDATE {TABLE} SET rendered_bytes=X'00' WHERE occurrence_identity=?1"
        ))
        .bind::<Text, _>(OCCURRENCE)
        .execute(&mut *conn)
        .is_err());
        assert!(diesel::sql_query(format!(
            "INSERT OR REPLACE INTO {MEMBER_TABLE}
             (prediction_row_id,occurrence_identity,ordinal,code) VALUES (?1,?2,?3,?4)"
        ))
        .bind::<BigInt, _>(record.ordered_rows[0].prediction_row_id)
        .bind::<Text, _>(OCCURRENCE)
        .bind::<BigInt, _>(0)
        .bind::<Text, _>("TEST_CODE_replaced")
        .execute(&mut *conn)
        .is_err());
        let mut alternate_source: CandidateBoardSourceV2 =
            serde_json::from_slice(&record.source_canonical).unwrap();
        alternate_source.rendered_sha256 = sha256(b"replacement card");
        let alternate_canonical = serde_json::to_vec(&alternate_source).unwrap();
        assert!(diesel::sql_query(format!(
            "INSERT OR REPLACE INTO {TABLE} (
                occurrence_identity,business_date,target_date,calendar_authority_hash,
                rendered_bytes,rendered_sha256,source_canonical,source_sha256
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"
        ))
        .bind::<Text, _>(OCCURRENCE)
        .bind::<Text, _>(DATE)
        .bind::<Text, _>(TARGET)
        .bind::<Text, _>(&record.calendar_authority_hash)
        .bind::<Binary, _>(b"replacement card".as_slice())
        .bind::<Text, _>(&alternate_source.rendered_sha256)
        .bind::<Binary, _>(&alternate_canonical)
        .bind::<Text, _>(sha256(&alternate_canonical))
        .execute(&mut *conn)
        .is_err());
        drop(conn);
        assert_eq!(
            db.read_candidate_board_v2_freeze(OCCURRENCE).unwrap(),
            Some(record.clone())
        );
        let mut conn = db.get_conn().unwrap();
        conn.batch_execute("DROP TRIGGER trg_candidate_board_prediction_member_v2_no_update")
            .unwrap();
        diesel::sql_query(format!(
            "UPDATE {MEMBER_TABLE} SET code='TEST_CODE_tampered' WHERE prediction_row_id=?1"
        ))
        .bind::<BigInt, _>(record.ordered_rows[0].prediction_row_id)
        .execute(&mut *conn)
        .unwrap();
        drop(conn);
        assert!(db.read_candidate_board_v2_freeze(OCCURRENCE).is_err());
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(format!(
            "UPDATE {MEMBER_TABLE} SET code=?1 WHERE prediction_row_id=?2"
        ))
        .bind::<Text, _>(&record.ordered_rows[0].code)
        .bind::<BigInt, _>(record.ordered_rows[0].prediction_row_id)
        .execute(&mut *conn)
        .unwrap();
        // Simulate damaged or replaced storage after dropping its protection.
        conn.batch_execute("DROP TRIGGER trg_candidate_board_prediction_freeze_v2_no_update")
            .unwrap();
        let noncanonical = serde_json::to_string_pretty(
            &serde_json::from_slice::<serde_json::Value>(&record.source_canonical).unwrap(),
        )
        .unwrap()
        .into_bytes();
        diesel::sql_query(format!(
            "UPDATE {TABLE} SET source_canonical=?1,source_sha256=?2 WHERE occurrence_identity=?3"
        ))
        .bind::<Binary, _>(&noncanonical)
        .bind::<Text, _>(sha256(&noncanonical))
        .bind::<Text, _>(OCCURRENCE)
        .execute(&mut *conn)
        .unwrap();
        drop(conn);
        assert!(db.read_candidate_board_v2_freeze(OCCURRENCE).is_err());

        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(format!(
            "UPDATE {TABLE} SET source_canonical=?1,source_sha256=?2 WHERE occurrence_identity=?3"
        ))
        .bind::<Binary, _>(&record.source_canonical)
        .bind::<Text, _>(&record.source_sha256)
        .bind::<Text, _>(OCCURRENCE)
        .execute(&mut *conn)
        .unwrap();
        diesel::sql_query(
            "UPDATE prediction_tracker SET stock_code='TEST_CODE_tampered' WHERE id=?1",
        )
        .bind::<BigInt, _>(record.ordered_rows[0].prediction_row_id)
        .execute(&mut *conn)
        .unwrap();
        drop(conn);
        assert!(db.read_candidate_board_v2_freeze(OCCURRENCE).is_err());
    }
}
