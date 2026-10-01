#![cfg(unix)]

//! SQL retention/catalog defenses, not a producer or completion fixture.
//! Raw selection bytes come from a real isolated writer. The remaining opaque
//! fixture blobs exercise SQL shape/hash/FK guards without asserting a model
//! receipt, admitted owner, terminal outcome, or day seal.

use super::*;
use crate::monitor::alert_log::AlertLog;
use crate::monitor::g5b_selection_v2::{G5bSelectionEvidence, G5bSelectionV2Candidate};
use chrono::NaiveDate;
use rusqlite::{params, types::Value};
use sha2::{Digest, Sha256};

const RAW: &[u8] = b"{\"origin\":\"production\",\"triggered_at\":\"2026-10-02T15:00:00+08:00\",\"code\":\"600001\",\"name\":\"fixture\",\"level\":\"other\",\"category\":\"fixture\",\"message\":\"same\",\"t1_locked\":false}\n";

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    super::super::schema::register_sha256_function(&connection).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    // REPLACE does not execute DELETE triggers in this SQLite mode. Retention
    // must be enforced by the INSERT guard, independently of recursive triggers.
    connection
        .pragma_update(None, "recursive_triggers", "OFF")
        .unwrap();
    // Minimal actual referenced tables for this additive SQL/catalog contract.
    // No durable coordinator or production database is opened by this fixture.
    connection
        .execute_batch(
            "CREATE TABLE delivery_decisions(decision_identity TEXT PRIMARY KEY NOT NULL);
         CREATE TABLE immutable_audit_outbox(
           audit_identity TEXT PRIMARY KEY NOT NULL,
           decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity)
         );",
        )
        .unwrap();
    connection
}

fn installed() -> Connection {
    let mut connection = database();
    let transaction = connection.transaction().unwrap();
    initialize(&transaction).unwrap();
    transaction.commit().unwrap();
    connection
}

struct RawSelection {
    _root: tempfile::TempDir,
    evidence: G5bSelectionEvidence,
}

impl RawSelection {
    fn new(line_count: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let log = AlertLog::for_test(root.path()).unwrap();
        log.initialize_date_input_head(date()).unwrap();
        for _ in 0..line_count {
            log.append_test_date_raw_production_fixture(date(), RAW)
                .unwrap();
        }
        let fence = log.acquire_date_writer_fence(date()).unwrap();
        let prefix = log
            .inspect_date_input_prefix_locked(date(), &fence)
            .unwrap();
        let candidate = G5bSelectionV2Candidate::from_locked_prefix(&prefix).unwrap();
        let evidence = G5bSelectionEvidence::decode(candidate.canonical_bytes()).unwrap();
        evidence.verify_locked_prefix(&prefix).unwrap();
        Self {
            _root: root,
            evidence,
        }
    }
}

fn insert_cohort(
    connection: &Connection,
    selection: &RawSelection,
    replace: bool,
) -> rusqlite::Result<usize> {
    let evidence = &selection.evidence;
    let encoded = evidence.encoded();
    let source = encoded.cutoff.source_identity.as_ref().unwrap();
    let admission = b"{\"fixture\":\"SQL-shape-only-unadmitted\"}";
    connection.execute(
        &format!(
            "INSERT {} INTO g5b_cohorts(
               cohort_identity,business_date,selection_kind,selection_canonical,
               selection_sha256,cohort_preimage,selected_count,cutoff_generation,
               cutoff_offset,prefix_sha256,source_device,source_inode,
               admission_canonical,admission_sha256
             ) VALUES(?1,?2,'NonEmpty',?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            if replace { "OR REPLACE" } else { "" }
        ),
        params![
            evidence.cohort_identity(),
            encoded.business_date.to_string(),
            evidence.canonical(),
            hash(evidence.canonical()),
            evidence.cohort_preimage(),
            i64::try_from(encoded.selected.len()).unwrap(),
            i64::try_from(encoded.cutoff.generation).unwrap(),
            i64::try_from(encoded.cutoff.committed_offset).unwrap(),
            encoded.cutoff.prefix_sha256,
            source.device.to_string(),
            source.inode.to_string(),
            admission.as_slice(),
            hash(admission),
        ],
    )
}

fn insert_members(connection: &Connection, selection: &RawSelection) {
    for (index, line) in selection.evidence.encoded().selected.iter().enumerate() {
        let (identity, preimage) = &selection.evidence.occurrences()[index];
        connection
            .execute(
                "INSERT INTO g5b_selected_occurrences(
               occurrence_identity,business_date,cohort_identity,selection_index,
               line_ordinal,start_offset,end_offset,raw_line_bytes,raw_line_sha256,
               record_canonical,record_sha256,occurrence_preimage
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    identity,
                    date().to_string(),
                    selection.evidence.cohort_identity(),
                    i64::try_from(index).unwrap(),
                    i64::try_from(line.line_ordinal).unwrap(),
                    i64::try_from(line.start_offset).unwrap(),
                    i64::try_from(line.end_offset).unwrap(),
                    &line.raw_line_bytes,
                    &line.raw_line_sha256,
                    &line.record_canonical,
                    &line.record_sha256,
                    preimage,
                ],
            )
            .unwrap();
    }
}

fn insert_owner(
    connection: &Connection,
    selection: &RawSelection,
    index: usize,
    replace: bool,
) -> rusqlite::Result<usize> {
    let bytes = b"{\"fixture\":\"SQL-owner-shape-only\"}";
    connection.execute(
        &format!(
            "INSERT {} INTO g5b_occurrence_owners(
               occurrence_identity,business_date,cohort_identity,decision_identity,
               frozen_canonical,frozen_sha256,source_canonical,source_sha256,
               rendered_bytes,rendered_sha256,envelope_canonical,envelope_sha256
             ) VALUES(?1,?2,?3,'TEST_CODE_SQL_DECISION',?4,?5,?4,?5,?4,?5,?4,?5)",
            if replace { "OR REPLACE" } else { "" }
        ),
        params![
            &selection.evidence.occurrences()[index].0,
            date().to_string(),
            selection.evidence.cohort_identity(),
            bytes.as_slice(),
            hash(bytes),
        ],
    )
}

fn insert_prepared(
    connection: &Connection,
    selection: &RawSelection,
    label: &str,
    replace: bool,
) -> rusqlite::Result<usize> {
    let canonical = serde_json::to_vec(&serde_json::json!({"fixture": label})).unwrap();
    connection.execute(
        &format!(
            "INSERT {} INTO g5b_artifact_events(
               event_identity,logical_intent,phase,business_date,cohort_identity,
               artifact_role,occurrence_identity,prepared_revision,commit_revision,
               before_kind,before_sha256,desired_bytes,desired_sha256,event_canonical,
               event_sha256,prepared_event_identity,file_witness_canonical,file_witness_sha256
             ) VALUES(?1,?2,'Prepared',?3,?4,'Selection',NULL,1,NULL,
               'Absent',NULL,?5,?6,?7,?1,NULL,NULL,NULL)",
            if replace { "OR REPLACE" } else { "" }
        ),
        params![
            hash(&canonical),
            hash(b"TEST_CODE_SQL_LOGICAL_INTENT"),
            date().to_string(),
            selection.evidence.cohort_identity(),
            selection.evidence.canonical(),
            hash(selection.evidence.canonical()),
            canonical,
        ],
    )
}

fn insert_unpublished_seal(
    connection: &Connection,
    selection: &RawSelection,
    label: &str,
    replace: bool,
) -> rusqlite::Result<usize> {
    let canonical = serde_json::to_vec(&serde_json::json!({"fixture": label})).unwrap();
    let mut preimage = b"TEST_CODE_UNPUBLISHED_SEAL\0".to_vec();
    preimage.extend_from_slice(&canonical);
    connection.execute(
        &format!(
            "INSERT {} INTO g5b_day_seals(
               seal_identity,business_date,cohort_identity,revision,seal_canonical,
               seal_sha256,seal_preimage
             ) VALUES(?1,?2,?3,2,?4,?5,?6)",
            if replace { "OR REPLACE" } else { "" }
        ),
        params![
            hash(&preimage),
            date().to_string(),
            selection.evidence.cohort_identity(),
            &canonical,
            hash(&canonical),
            preimage,
        ],
    )
}

fn snapshot(connection: &Connection, table: &str) -> Vec<Vec<Value>> {
    assert!(TABLES.contains(&table));
    let mut statement = connection
        .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = statement.column_count();
    let rows = statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| row.get(index))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

#[test]
fn schema12_retention_exact_replace_cannot_delete_and_reinsert_any_evidence_row() {
    let connection = installed();
    let selection = RawSelection::new(1);
    insert_cohort(&connection, &selection, false).unwrap();
    insert_members(&connection, &selection);
    connection
        .execute(
            "INSERT INTO delivery_decisions VALUES('TEST_CODE_SQL_DECISION')",
            [],
        )
        .unwrap();
    insert_owner(&connection, &selection, 0, false).unwrap();
    insert_prepared(&connection, &selection, "original prepared", false).unwrap();
    insert_unpublished_seal(&connection, &selection, "unpublished SQL fixture", false).unwrap();
    verify_foreign_keys(&connection).unwrap();
    for table in &TABLES[1..] {
        let before = snapshot(&connection, table);
        assert_eq!(before.len(), 1);
        // Every byte, hash, key and FK is already correct for this stored row.
        // Without INSERT retention the REPLACE silently changes its rowid.
        let result = connection.execute(
            &format!("INSERT OR REPLACE INTO {table} SELECT * FROM {table}"),
            [],
        );
        assert!(result.is_err(), "exact REPLACE must reject for {table}");
        assert_eq!(
            snapshot(&connection, table),
            before,
            "retained rowid/bytes {table}"
        );
    }
}

#[test]
fn schema12_retention_unique_conflicts_cannot_replace_other_primary_identities() {
    // Date uniqueness: both candidates really came from separate source inodes,
    // with correct canonical/domain/hash fields for the same business date.
    let connection = installed();
    let original = RawSelection::new(1);
    let competing = RawSelection::new(1);
    assert_ne!(
        original.evidence.cohort_identity(),
        competing.evidence.cohort_identity()
    );
    insert_cohort(&connection, &original, false).unwrap();
    let before = snapshot(&connection, "g5b_cohorts");
    assert!(insert_cohort(&connection, &competing, true).is_err());
    assert_eq!(snapshot(&connection, "g5b_cohorts"), before);

    // Decision-owner uniqueness: a different actual selected occurrence cannot
    // steal one existing decision, even though all SQL shape/hash/FKs match.
    let connection = installed();
    let selection = RawSelection::new(2);
    insert_cohort(&connection, &selection, false).unwrap();
    insert_members(&connection, &selection);
    connection
        .execute(
            "INSERT INTO delivery_decisions VALUES('TEST_CODE_SQL_DECISION')",
            [],
        )
        .unwrap();
    insert_owner(&connection, &selection, 0, false).unwrap();
    let before = snapshot(&connection, "g5b_occurrence_owners");
    assert!(insert_owner(&connection, &selection, 1, true).is_err());
    assert_eq!(snapshot(&connection, "g5b_occurrence_owners"), before);

    // Logical-intent and seal-revision conflicts use fresh, internally correct
    // bytes/hashes/identities. A bad input hash is not the rejection mechanism.
    insert_prepared(&connection, &selection, "original prepared", false).unwrap();
    let before = snapshot(&connection, "g5b_artifact_events");
    assert!(insert_prepared(&connection, &selection, "changed prepared", true).is_err());
    assert_eq!(snapshot(&connection, "g5b_artifact_events"), before);
    insert_unpublished_seal(
        &connection,
        &selection,
        "first unpublished SQL fixture",
        false,
    )
    .unwrap();
    let before = snapshot(&connection, "g5b_day_seals");
    assert!(insert_unpublished_seal(
        &connection,
        &selection,
        "second unpublished SQL fixture",
        true
    )
    .is_err());
    assert_eq!(snapshot(&connection, "g5b_day_seals"), before);
}

#[test]
fn schema12_retention_head_replace_cannot_reset_revision_or_reopen_identity() {
    let connection = installed();
    connection
        .execute(
            "INSERT INTO g5b_day_heads(business_date,revision,artifact_state) VALUES(?1,7,'Dirty')",
            [date().to_string()],
        )
        .unwrap();
    let before = snapshot(&connection, "g5b_day_heads");
    let result = connection.execute("INSERT OR REPLACE INTO g5b_day_heads(business_date,revision,artifact_state) VALUES(?1,0,'Dirty')", [date().to_string()]);
    assert!(result.is_err());
    assert_eq!(snapshot(&connection, "g5b_day_heads"), before);
    assert!(connection
        .execute(
            "DELETE FROM g5b_day_heads WHERE business_date=?1",
            [date().to_string()]
        )
        .is_err());
    assert_eq!(snapshot(&connection, "g5b_day_heads"), before);
}

#[test]
fn schema12_catalog_rejects_temporary_shadows_with_case_insensitive_names() {
    for table in TABLES {
        for name in [table.to_owned(), table.to_ascii_uppercase()] {
            let connection = installed();
            let before = manifest(&connection).unwrap();
            connection
                .execute_batch(&format!("CREATE TEMP TABLE {name}(fixture TEXT)"))
                .unwrap();
            assert!(verify_catalog(&connection).is_err(), "temp shadow {name}");
            assert_eq!(
                manifest(&connection).unwrap(),
                before,
                "main schema unchanged"
            );
            connection
                .execute_batch(&format!("DROP TABLE temp.{name}"))
                .unwrap();
            verify_catalog(&connection).unwrap();
        }
    }
}

#[test]
fn schema12_catalog_rejects_ordinary_named_attached_indexes_and_temp_triggers() {
    let connection = installed();
    let before = manifest(&connection).unwrap();
    connection
        .execute_batch("CREATE INDEX TEST_CODE_HIDDEN_INDEX ON g5b_cohorts(selection_kind)")
        .unwrap();
    assert!(verify_catalog(&connection).is_err());
    connection
        .execute_batch("DROP INDEX TEST_CODE_HIDDEN_INDEX")
        .unwrap();
    assert_eq!(manifest(&connection).unwrap(), before);
    verify_catalog(&connection).unwrap();

    connection.execute_batch("CREATE TEMP TRIGGER TEST_CODE_HIDDEN_TEMP_TRIGGER BEFORE UPDATE ON main.g5b_cohorts BEGIN SELECT 1; END;").unwrap();
    assert!(verify_catalog(&connection).is_err());
    assert_eq!(
        manifest(&connection).unwrap(),
        before,
        "temp trigger leaves main manifest unchanged"
    );
    connection
        .execute_batch("DROP TRIGGER temp.TEST_CODE_HIDDEN_TEMP_TRIGGER")
        .unwrap();
    verify_catalog(&connection).unwrap();
}

#[test]
fn schema12_catalog_rejects_reserved_temporary_objects_before_legacy_extension() {
    let connection = database();
    reject_preexisting_extension(&connection).unwrap();
    connection
        .execute_batch("CREATE TEMP TABLE G5B_COHORTS(fixture TEXT)")
        .unwrap();
    assert!(reject_preexisting_extension(&connection).is_err());
    assert!(manifest(&connection).unwrap().is_empty());
    connection
        .execute_batch("DROP TABLE temp.G5B_COHORTS")
        .unwrap();
    reject_preexisting_extension(&connection).unwrap();
}
