//! Additive cohort evidence. Existing delivery tables remain the only sink owner.

use super::model::{DurableDeliveryError, Result};
use rusqlite::{Connection, Transaction};

pub(super) const TABLES: [&str; 6] = [
    "g5b_day_heads",
    "g5b_cohorts",
    "g5b_selected_occurrences",
    "g5b_occurrence_owners",
    "g5b_artifact_events",
    "g5b_day_seals",
];

const DDL: &str = r#"
CREATE TABLE g5b_cohorts(
 cohort_identity TEXT PRIMARY KEY NOT NULL CHECK(length(cohort_identity)=64 AND cohort_identity NOT GLOB '*[^0-9a-f]*'),
 business_date TEXT NOT NULL UNIQUE,
 selection_kind TEXT NOT NULL CHECK(selection_kind IN ('NonEmpty','Empty')),
 selection_canonical BLOB NOT NULL CHECK(typeof(selection_canonical)='blob'),
 selection_sha256 TEXT NOT NULL CHECK(sha256_hex(selection_canonical)=selection_sha256),
 cohort_preimage BLOB NOT NULL CHECK(typeof(cohort_preimage)='blob' AND sha256_hex(cohort_preimage)=cohort_identity),
 selected_count INTEGER NOT NULL CHECK(typeof(selected_count)='integer' AND selected_count BETWEEN 0 AND 3),
 cutoff_generation INTEGER NOT NULL CHECK(typeof(cutoff_generation)='integer' AND cutoff_generation>=0),
 cutoff_offset INTEGER NOT NULL CHECK(typeof(cutoff_offset)='integer' AND cutoff_offset>=0),
 prefix_sha256 TEXT NOT NULL CHECK(length(prefix_sha256)=64 AND prefix_sha256 NOT GLOB '*[^0-9a-f]*'),
 source_device TEXT, source_inode TEXT,
 admission_canonical BLOB NOT NULL CHECK(typeof(admission_canonical)='blob'),
 admission_sha256 TEXT NOT NULL CHECK(sha256_hex(admission_canonical)=admission_sha256),
 CHECK((selection_kind='NonEmpty' AND selected_count BETWEEN 1 AND 3 AND cutoff_generation>0
         AND cutoff_offset>0 AND source_device IS NOT NULL AND source_inode IS NOT NULL)
    OR (selection_kind='Empty' AND selected_count=0 AND cutoff_generation=0 AND cutoff_offset=0
         AND prefix_sha256=sha256_hex(X'') AND source_device IS NULL AND source_inode IS NULL)),
 UNIQUE(business_date,cohort_identity)
);
CREATE TABLE g5b_selected_occurrences(
 occurrence_identity TEXT PRIMARY KEY NOT NULL CHECK(length(occurrence_identity)=64 AND occurrence_identity NOT GLOB '*[^0-9a-f]*'),
 business_date TEXT NOT NULL, cohort_identity TEXT NOT NULL,
 selection_index INTEGER NOT NULL CHECK(typeof(selection_index)='integer' AND selection_index BETWEEN 0 AND 2),
 line_ordinal INTEGER NOT NULL CHECK(typeof(line_ordinal)='integer' AND line_ordinal>0),
 start_offset INTEGER NOT NULL CHECK(typeof(start_offset)='integer' AND start_offset>=0),
 end_offset INTEGER NOT NULL CHECK(typeof(end_offset)='integer' AND end_offset>start_offset),
 raw_line_bytes BLOB NOT NULL CHECK(typeof(raw_line_bytes)='blob' AND length(raw_line_bytes)=end_offset-start_offset AND substr(raw_line_bytes,-1)=X'0A'),
 raw_line_sha256 TEXT NOT NULL CHECK(sha256_hex(raw_line_bytes)=raw_line_sha256),
 record_canonical BLOB NOT NULL CHECK(typeof(record_canonical)='blob'),
 record_sha256 TEXT NOT NULL CHECK(sha256_hex(record_canonical)=record_sha256),
 occurrence_preimage BLOB NOT NULL CHECK(typeof(occurrence_preimage)='blob' AND sha256_hex(occurrence_preimage)=occurrence_identity),
 FOREIGN KEY(business_date,cohort_identity) REFERENCES g5b_cohorts(business_date,cohort_identity),
 UNIQUE(cohort_identity,selection_index), UNIQUE(cohort_identity,line_ordinal,start_offset,end_offset),
 UNIQUE(business_date,cohort_identity,occurrence_identity)
);
CREATE TABLE g5b_occurrence_owners(
 occurrence_identity TEXT PRIMARY KEY NOT NULL,
 business_date TEXT NOT NULL, cohort_identity TEXT NOT NULL,
 decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions(decision_identity),
 frozen_canonical BLOB NOT NULL CHECK(typeof(frozen_canonical)='blob'),
 frozen_sha256 TEXT NOT NULL CHECK(sha256_hex(frozen_canonical)=frozen_sha256),
 source_canonical BLOB NOT NULL CHECK(typeof(source_canonical)='blob'),
 source_sha256 TEXT NOT NULL CHECK(sha256_hex(source_canonical)=source_sha256),
 rendered_bytes BLOB NOT NULL CHECK(typeof(rendered_bytes)='blob'),
 rendered_sha256 TEXT NOT NULL CHECK(sha256_hex(rendered_bytes)=rendered_sha256),
 envelope_canonical BLOB NOT NULL CHECK(typeof(envelope_canonical)='blob'),
 envelope_sha256 TEXT NOT NULL CHECK(sha256_hex(envelope_canonical)=envelope_sha256),
 FOREIGN KEY(business_date,cohort_identity,occurrence_identity)
   REFERENCES g5b_selected_occurrences(business_date,cohort_identity,occurrence_identity)
);
CREATE TABLE g5b_artifact_events(
 event_identity TEXT PRIMARY KEY NOT NULL CHECK(length(event_identity)=64 AND event_identity NOT GLOB '*[^0-9a-f]*'),
 logical_intent TEXT NOT NULL CHECK(length(logical_intent)=64 AND logical_intent NOT GLOB '*[^0-9a-f]*'),
 phase TEXT NOT NULL CHECK(phase IN ('Prepared','Committed')),
 business_date TEXT NOT NULL, cohort_identity TEXT NOT NULL,
 artifact_role TEXT NOT NULL CHECK(artifact_role IN ('Selection','Attempt','Frozen','Archive')),
 occurrence_identity TEXT,
 prepared_revision INTEGER NOT NULL CHECK(typeof(prepared_revision)='integer' AND prepared_revision>0),
 commit_revision INTEGER,
 before_kind TEXT NOT NULL CHECK(before_kind IN ('Absent','ExistingExact')),
 before_sha256 TEXT,
 desired_bytes BLOB NOT NULL CHECK(typeof(desired_bytes)='blob' AND length(desired_bytes)>0),
 desired_sha256 TEXT NOT NULL CHECK(sha256_hex(desired_bytes)=desired_sha256),
 event_canonical BLOB NOT NULL CHECK(typeof(event_canonical)='blob'),
 event_sha256 TEXT NOT NULL CHECK(sha256_hex(event_canonical)=event_sha256),
 prepared_event_identity TEXT REFERENCES g5b_artifact_events(event_identity),
 file_witness_canonical BLOB, file_witness_sha256 TEXT,
 FOREIGN KEY(business_date,cohort_identity) REFERENCES g5b_cohorts(business_date,cohort_identity),
 FOREIGN KEY(business_date,cohort_identity,occurrence_identity)
   REFERENCES g5b_selected_occurrences(business_date,cohort_identity,occurrence_identity),
 CHECK((artifact_role IN ('Selection','Archive') AND occurrence_identity IS NULL)
    OR (artifact_role IN ('Attempt','Frozen') AND occurrence_identity IS NOT NULL)),
 CHECK((before_kind='Absent' AND before_sha256 IS NULL)
    OR (before_kind='ExistingExact' AND before_sha256 IS NOT NULL AND before_sha256=desired_sha256)),
 CHECK((phase='Prepared' AND commit_revision IS NULL AND prepared_event_identity IS NULL
          AND file_witness_canonical IS NULL AND file_witness_sha256 IS NULL)
    OR (phase='Committed' AND typeof(commit_revision)='integer' AND commit_revision>prepared_revision
          AND prepared_event_identity IS NOT NULL AND typeof(file_witness_canonical)='blob'
          AND file_witness_sha256 IS NOT NULL AND sha256_hex(file_witness_canonical)=file_witness_sha256)),
 UNIQUE(logical_intent,phase)
);
CREATE TABLE g5b_day_seals(
 seal_identity TEXT PRIMARY KEY NOT NULL CHECK(length(seal_identity)=64 AND seal_identity NOT GLOB '*[^0-9a-f]*'),
 business_date TEXT NOT NULL, cohort_identity TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>0),
 seal_canonical BLOB NOT NULL CHECK(typeof(seal_canonical)='blob'),
 seal_sha256 TEXT NOT NULL CHECK(sha256_hex(seal_canonical)=seal_sha256),
 seal_preimage BLOB NOT NULL CHECK(typeof(seal_preimage)='blob' AND sha256_hex(seal_preimage)=seal_identity),
 FOREIGN KEY(business_date,cohort_identity) REFERENCES g5b_cohorts(business_date,cohort_identity),
 UNIQUE(business_date,cohort_identity,revision),
 UNIQUE(business_date,cohort_identity,revision,seal_identity)
);
CREATE TABLE g5b_day_heads(
 business_date TEXT PRIMARY KEY NOT NULL,
 revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0),
 artifact_state TEXT NOT NULL CHECK(artifact_state IN ('Dirty','Clean')),
 cohort_identity TEXT, current_seal_identity TEXT,
 prospective_canonical BLOB, prospective_sha256 TEXT,
 CHECK((prospective_canonical IS NULL AND prospective_sha256 IS NULL)
    OR (typeof(prospective_canonical)='blob' AND prospective_sha256 IS NOT NULL
        AND sha256_hex(prospective_canonical)=prospective_sha256)),
 CHECK(current_seal_identity IS NULL OR (cohort_identity IS NOT NULL AND artifact_state='Clean')),
 FOREIGN KEY(business_date,cohort_identity) REFERENCES g5b_cohorts(business_date,cohort_identity),
 FOREIGN KEY(business_date,cohort_identity,revision,current_seal_identity)
   REFERENCES g5b_day_seals(business_date,cohort_identity,revision,seal_identity)
);
CREATE TRIGGER g5b_member_insert BEFORE INSERT ON g5b_selected_occurrences
BEGIN SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM g5b_cohorts c
 WHERE c.cohort_identity=NEW.cohort_identity AND c.business_date=NEW.business_date
 AND NEW.selection_index<c.selected_count AND NEW.line_ordinal<=c.cutoff_generation
 AND NEW.end_offset<=c.cutoff_offset) THEN RAISE(ABORT,'invalid cohort member') END; END;
CREATE TRIGGER g5b_committed_insert BEFORE INSERT ON g5b_artifact_events WHEN NEW.phase='Committed'
BEGIN SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM g5b_artifact_events p
 WHERE p.event_identity=NEW.prepared_event_identity AND p.phase='Prepared'
 AND p.logical_intent=NEW.logical_intent AND p.business_date=NEW.business_date
 AND p.cohort_identity=NEW.cohort_identity AND p.artifact_role=NEW.artifact_role
 AND p.occurrence_identity IS NEW.occurrence_identity AND p.prepared_revision=NEW.prepared_revision
 AND p.before_kind=NEW.before_kind AND p.before_sha256 IS NEW.before_sha256
 AND p.desired_bytes=NEW.desired_bytes AND p.desired_sha256=NEW.desired_sha256)
 THEN RAISE(ABORT,'committed event does not match prepared') END; END;
CREATE TRIGGER g5b_head_update BEFORE UPDATE ON g5b_day_heads
BEGIN
 SELECT CASE WHEN NEW.business_date!=OLD.business_date OR NEW.revision<OLD.revision
 OR NEW.revision>OLD.revision+1 OR (OLD.cohort_identity IS NOT NULL AND NEW.cohort_identity IS NOT OLD.cohort_identity)
 OR (NEW.revision!=OLD.revision AND NEW.current_seal_identity IS NOT NULL)
 THEN RAISE(ABORT,'invalid cohort head advance') END;
 SELECT CASE WHEN OLD.prospective_canonical IS NOT NULL
 AND (NEW.prospective_canonical IS NOT OLD.prospective_canonical OR NEW.prospective_sha256 IS NOT OLD.prospective_sha256)
 THEN RAISE(ABORT,'prospective observation is immutable') END;
 SELECT CASE WHEN OLD.prospective_canonical IS NULL AND NEW.prospective_canonical IS NOT NULL
 AND (OLD.revision!=0 OR OLD.cohort_identity IS NOT NULL OR NEW.revision!=0 OR NEW.cohort_identity IS NOT NULL)
 THEN RAISE(ABORT,'prospective observation cannot be retrofitted to a cohort') END;
 SELECT CASE WHEN NEW.artifact_state='Clean' AND EXISTS(SELECT 1 FROM g5b_artifact_events p
 WHERE p.business_date=NEW.business_date AND p.phase='Prepared' AND NOT EXISTS
 (SELECT 1 FROM g5b_artifact_events c WHERE c.prepared_event_identity=p.event_identity AND c.phase='Committed'))
 THEN RAISE(ABORT,'unmatched artifact prevents clean') END;
END;
CREATE TRIGGER g5b_head_delete BEFORE DELETE ON g5b_day_heads
BEGIN SELECT RAISE(ABORT,'cohort heads are retained'); END;
"#;

fn immutable_ddl() -> String {
    let mut sql = String::from(DDL);
    for (table, conflicts) in [
        ("g5b_cohorts", "cohort_identity=NEW.cohort_identity OR business_date=NEW.business_date"),
        ("g5b_selected_occurrences", "occurrence_identity=NEW.occurrence_identity OR (cohort_identity=NEW.cohort_identity AND selection_index=NEW.selection_index) OR (cohort_identity=NEW.cohort_identity AND line_ordinal=NEW.line_ordinal AND start_offset=NEW.start_offset AND end_offset=NEW.end_offset)"),
        ("g5b_occurrence_owners", "occurrence_identity=NEW.occurrence_identity OR decision_identity=NEW.decision_identity"),
        ("g5b_artifact_events", "event_identity=NEW.event_identity OR (logical_intent=NEW.logical_intent AND phase=NEW.phase)"),
        ("g5b_day_seals", "seal_identity=NEW.seal_identity OR (business_date=NEW.business_date AND cohort_identity=NEW.cohort_identity AND revision=NEW.revision)"),
        ("g5b_day_heads", "business_date=NEW.business_date"),
    ] {
        sql.push_str(&format!("CREATE TRIGGER {table}_replacement_insert BEFORE INSERT ON {table} WHEN EXISTS(SELECT 1 FROM {table} WHERE {conflicts}) BEGIN SELECT RAISE(ABORT,'g5b evidence cannot be replaced'); END;\n"));
    }
    for table in &TABLES[1..] {
        // TABLES[1..] includes all five immutable tables, not the mutable head.
        sql.push_str(&format!("CREATE TRIGGER {table}_immutable_update BEFORE UPDATE ON {table} BEGIN SELECT RAISE(ABORT,'g5b evidence is immutable'); END;\nCREATE TRIGGER {table}_immutable_delete BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT,'g5b evidence is retained'); END;\n"));
    }
    sql
}

pub(super) fn reject_preexisting_extension(connection: &Connection) -> Result<()> {
    reject_temporary_extension(connection)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'g5b_*' OR lower(tbl_name) GLOB 'g5b_*'",
        [],
        |row| row.get(0),
    )?;
    if count != 0 {
        return Err(invalid("reserved g5b objects predate schema 12"));
    }
    Ok(())
}

pub(super) fn initialize(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(&immutable_ddl())?;
    verify_catalog(transaction)?;
    verify_foreign_keys(transaction)
}

pub(super) fn verify_catalog(connection: &Connection) -> Result<()> {
    reject_temporary_extension(connection)?;
    // SQLite retains our CREATE text without the final separator. Compare the
    // exact compiled declarations, including every CHECK/FK/trigger. This is
    // pure string material: no reference SQLite connection is opened at runtime.
    static MANIFEST: std::sync::OnceLock<Vec<(String, String, String)>> =
        std::sync::OnceLock::new();
    let expected = MANIFEST.get_or_init(|| {
        let sql = immutable_ddl();
        let starts = sql
            .match_indices("CREATE ")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        for (index, start) in starts.iter().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(sql.len());
            let declaration = sql[*start..end].trim().trim_end_matches(';').trim_end();
            let mut words = declaration.split_whitespace();
            words.next();
            let kind = words.next().unwrap_or("").to_ascii_lowercase();
            let name = words.next().unwrap_or("").split('(').next().unwrap_or("");
            rows.push((kind, name.to_owned(), declaration.to_owned()));
        }
        rows.sort();
        rows
    });
    if manifest(connection)? != *expected {
        return Err(invalid(
            "g5b schema objects or constraints differ from schema 12",
        ));
    }
    // The existing audit outbox must retain its actual non-null decision FK.
    let real_fk: i64 = connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_list('immutable_audit_outbox') WHERE \"table\"='delivery_decisions' AND \"from\"='decision_identity' AND \"to\"='decision_identity'", [], |r|r.get(0))?;
    let nonnull: i64 = connection.query_row("SELECT \"notnull\" FROM pragma_table_info('immutable_audit_outbox') WHERE name='decision_identity'", [], |r|r.get(0))?;
    if real_fk != 1 || nonnull != 1 {
        return Err(invalid("immutable outbox lost real decision authority"));
    }
    Ok(())
}

fn manifest(connection: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement=connection.prepare("SELECT type,name,COALESCE(sql,'') FROM main.sqlite_master WHERE sql IS NOT NULL AND (lower(name) GLOB 'g5b_*' OR lower(tbl_name) GLOB 'g5b_*') ORDER BY type,name")?;
    let rows = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(super) fn verify_foreign_keys(connection: &Connection) -> Result<()> {
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    if count != 0 {
        return Err(invalid(
            "foreign key violations prevent schema 12 operation",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> DurableDeliveryError {
    DurableDeliveryError::InvalidConfiguration(message.to_owned())
}

fn reject_temporary_extension(connection: &Connection) -> Result<()> {
    let count: i64=connection.query_row("SELECT COUNT(*) FROM temp.sqlite_master WHERE lower(name) GLOB 'g5b_*' OR lower(tbl_name) GLOB 'g5b_*'", [], |r|r.get(0))?;
    if count != 0 {
        return Err(invalid("temporary schema shadows reserved g5b objects"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "schema_g5b_cohort_replacement_tests.rs"]
mod replacement_tests;

#[cfg(test)]
pub(super) fn remove_empty_extension_for_legacy_test(connection: &Connection) {
    let rows: i64 = TABLES
        .iter()
        .map(|table| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        })
        .sum();
    assert_eq!(
        rows, 0,
        "TEST_CODE legacy fixture cannot discard actual cohort evidence"
    );
    connection.execute_batch("DROP TABLE g5b_day_heads; DROP TABLE g5b_day_seals; DROP TABLE g5b_occurrence_owners; DROP TABLE g5b_artifact_events; DROP TABLE g5b_selected_occurrences; DROP TABLE g5b_cohorts;").unwrap();
}
