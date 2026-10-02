//! Schema 13 stores local P05 preparation evidence, never physical completion.

use super::model::{DurableDeliveryError, Result};
use rusqlite::{Connection, Transaction};

pub(super) const TABLES: [&str; 7] = [
    "p05_baseline_origins",
    "p05_baseline_heads",
    "p05_unit_drafts",
    "p05_unit_heads",
    "p05_prediction_prepare_events",
    "p05_unit_intents",
    "p05_unit_children",
];

const DDL: &str = r#"
CREATE TABLE p05_baseline_origins(
 origin_identity TEXT PRIMARY KEY NOT NULL CHECK(length(origin_identity)=64 AND origin_identity NOT GLOB '*[^0-9a-f]*'),
 family TEXT NOT NULL,
 business_date TEXT NOT NULL,
 origin_kind TEXT NOT NULL CHECK(origin_kind IN ('ProspectiveNoPreviousV2Baseline','CompletedUnitV2Baseline')),
 completed_unit_identity TEXT,
 completed_receipt_identity TEXT,
 accepted_physical_refs BLOB,
 origin_canonical BLOB NOT NULL CHECK(typeof(origin_canonical)='blob' AND length(origin_canonical) BETWEEN 1 AND 4194304),
 origin_sha256 TEXT NOT NULL CHECK(sha256_hex(origin_canonical)=origin_sha256),
 origin_preimage BLOB NOT NULL CHECK(typeof(origin_preimage)='blob' AND sha256_hex(origin_preimage)=origin_identity),
 CHECK((origin_kind='ProspectiveNoPreviousV2Baseline' AND completed_unit_identity IS NULL AND completed_receipt_identity IS NULL AND accepted_physical_refs IS NULL)
    OR (origin_kind='CompletedUnitV2Baseline' AND completed_unit_identity IS NOT NULL AND length(completed_unit_identity)=64 AND completed_unit_identity NOT GLOB '*[^0-9a-f]*'
        AND completed_receipt_identity IS NOT NULL AND length(completed_receipt_identity)=64 AND completed_receipt_identity NOT GLOB '*[^0-9a-f]*'
        AND typeof(accepted_physical_refs)='blob' AND length(accepted_physical_refs) BETWEEN 1 AND 4194304)),
 FOREIGN KEY(completed_unit_identity,business_date) REFERENCES p05_unit_drafts(draft_identity,business_date),
 UNIQUE(family,origin_identity)
);
CREATE TABLE p05_baseline_heads(
 family TEXT PRIMARY KEY NOT NULL,
 baseline_revision INTEGER NOT NULL CHECK(typeof(baseline_revision)='integer' AND baseline_revision>=0),
 origin_identity TEXT NOT NULL,
 FOREIGN KEY(family,origin_identity) REFERENCES p05_baseline_origins(family,origin_identity)
);
CREATE TABLE p05_unit_drafts(
 draft_identity TEXT PRIMARY KEY NOT NULL CHECK(length(draft_identity)=64 AND draft_identity NOT GLOB '*[^0-9a-f]*'),
 unit_occurrence TEXT NOT NULL UNIQUE,
 business_date TEXT NOT NULL UNIQUE,
 family TEXT NOT NULL,
 policy TEXT NOT NULL CHECK(policy='P05_AUCTION_UNIT_FIRST_OBSERVED_V1'),
 baseline_revision INTEGER NOT NULL CHECK(typeof(baseline_revision)='integer' AND baseline_revision>=0),
 baseline_origin_identity TEXT NOT NULL,
 draft_canonical BLOB NOT NULL CHECK(typeof(draft_canonical)='blob' AND length(draft_canonical) BETWEEN 1 AND 4194304),
 draft_sha256 TEXT NOT NULL CHECK(sha256_hex(draft_canonical)=draft_sha256),
 draft_preimage BLOB NOT NULL CHECK(typeof(draft_preimage)='blob' AND sha256_hex(draft_preimage)=draft_identity),
 FOREIGN KEY(family,baseline_origin_identity) REFERENCES p05_baseline_origins(family,origin_identity),
 UNIQUE(draft_identity,business_date)
);
CREATE TABLE p05_prediction_prepare_events(
 event_identity TEXT PRIMARY KEY NOT NULL CHECK(length(event_identity)=64 AND event_identity NOT GLOB '*[^0-9a-f]*'),
 draft_identity TEXT NOT NULL UNIQUE REFERENCES p05_unit_drafts(draft_identity),
 phase TEXT NOT NULL CHECK(phase='Started'),
 event_canonical BLOB NOT NULL CHECK(typeof(event_canonical)='blob' AND length(event_canonical) BETWEEN 1 AND 4194304),
 event_sha256 TEXT NOT NULL CHECK(sha256_hex(event_canonical)=event_sha256),
 event_preimage BLOB NOT NULL CHECK(typeof(event_preimage)='blob' AND sha256_hex(event_preimage)=event_identity),
 UNIQUE(draft_identity,event_identity)
);
CREATE TABLE p05_unit_intents(
 intent_identity TEXT PRIMARY KEY NOT NULL CHECK(length(intent_identity)=64 AND intent_identity NOT GLOB '*[^0-9a-f]*'),
 draft_identity TEXT NOT NULL UNIQUE REFERENCES p05_unit_drafts(draft_identity),
 started_event_identity TEXT NOT NULL,
 child_count INTEGER NOT NULL CHECK(typeof(child_count)='integer' AND child_count BETWEEN 2 AND 514),
 intent_canonical BLOB NOT NULL CHECK(typeof(intent_canonical)='blob' AND length(intent_canonical) BETWEEN 1 AND 4194304),
 intent_sha256 TEXT NOT NULL CHECK(sha256_hex(intent_canonical)=intent_sha256),
 intent_preimage BLOB NOT NULL CHECK(typeof(intent_preimage)='blob' AND sha256_hex(intent_preimage)=intent_identity),
 FOREIGN KEY(draft_identity,started_event_identity) REFERENCES p05_prediction_prepare_events(draft_identity,event_identity),
 UNIQUE(draft_identity,intent_identity)
);
CREATE TABLE p05_unit_children(
 child_identity TEXT PRIMARY KEY NOT NULL CHECK(length(child_identity)=64 AND child_identity NOT GLOB '*[^0-9a-f]*'),
 draft_identity TEXT NOT NULL,
 intent_identity TEXT NOT NULL,
 ordinal INTEGER NOT NULL CHECK(typeof(ordinal)='integer' AND ordinal BETWEEN 0 AND 513),
 child_kind TEXT NOT NULL CHECK(child_kind IN ('AuctionRepush','CandidateInvalidated','CandidateBoard')),
 decision_identity TEXT NOT NULL UNIQUE CHECK(length(decision_identity)=64 AND decision_identity NOT GLOB '*[^0-9a-f]*'),
 child_canonical BLOB NOT NULL CHECK(typeof(child_canonical)='blob' AND length(child_canonical) BETWEEN 1 AND 4194304),
 child_sha256 TEXT NOT NULL CHECK(sha256_hex(child_canonical)=child_sha256),
 child_preimage BLOB NOT NULL CHECK(typeof(child_preimage)='blob' AND sha256_hex(child_preimage)=child_identity),
 FOREIGN KEY(draft_identity,intent_identity) REFERENCES p05_unit_intents(draft_identity,intent_identity),
 UNIQUE(intent_identity,ordinal)
);
CREATE TABLE p05_unit_heads(
 draft_identity TEXT PRIMARY KEY NOT NULL REFERENCES p05_unit_drafts(draft_identity),
 mutation_revision INTEGER NOT NULL CHECK(typeof(mutation_revision)='integer' AND mutation_revision>=1),
 phase TEXT NOT NULL CHECK(phase IN ('Draft','Started','IntentComplete')),
 intent_identity TEXT,
 CHECK((phase='Draft' AND mutation_revision=1 AND intent_identity IS NULL)
    OR (phase='Started' AND mutation_revision=2 AND intent_identity IS NULL)
    OR (phase='IntentComplete' AND mutation_revision>=3 AND intent_identity IS NOT NULL)),
 FOREIGN KEY(draft_identity,intent_identity) REFERENCES p05_unit_intents(draft_identity,intent_identity)
);
CREATE TRIGGER p05_unit_heads_advance BEFORE UPDATE ON p05_unit_heads
BEGIN
 SELECT CASE WHEN NEW.draft_identity!=OLD.draft_identity OR NEW.mutation_revision!=OLD.mutation_revision+1
 OR NOT ((OLD.phase='Draft' AND NEW.phase='Started' AND EXISTS(SELECT 1 FROM p05_prediction_prepare_events e WHERE e.draft_identity=NEW.draft_identity))
 OR (OLD.phase='Started' AND NEW.phase='IntentComplete' AND EXISTS(SELECT 1 FROM p05_unit_intents i WHERE i.draft_identity=NEW.draft_identity AND i.intent_identity=NEW.intent_identity AND i.child_count=(SELECT COUNT(*) FROM p05_unit_children c WHERE c.intent_identity=i.intent_identity)))
 OR (OLD.phase='IntentComplete' AND NEW.phase='IntentComplete' AND NEW.intent_identity IS OLD.intent_identity))
 THEN RAISE(ABORT,'invalid P05 preparation advance') END;
END;
CREATE TRIGGER p05_unit_heads_retained BEFORE DELETE ON p05_unit_heads
BEGIN SELECT RAISE(ABORT,'P05 head is retained'); END;
CREATE TRIGGER p05_baseline_heads_advance BEFORE UPDATE ON p05_baseline_heads
BEGIN
 SELECT CASE WHEN NEW.family!=OLD.family OR NEW.baseline_revision!=OLD.baseline_revision+1
 OR NEW.origin_identity IS OLD.origin_identity OR NOT EXISTS(SELECT 1 FROM p05_baseline_origins n JOIN p05_baseline_origins o ON o.origin_identity=OLD.origin_identity
 WHERE n.origin_identity=NEW.origin_identity AND n.family=OLD.family AND n.origin_kind='CompletedUnitV2Baseline'
 AND ((o.origin_kind='ProspectiveNoPreviousV2Baseline' AND n.business_date>=o.business_date) OR (o.origin_kind='CompletedUnitV2Baseline' AND n.business_date>o.business_date)))
 THEN RAISE(ABORT,'invalid P05 baseline advance') END;
END;
CREATE TRIGGER p05_baseline_heads_retained BEFORE DELETE ON p05_baseline_heads
BEGIN SELECT RAISE(ABORT,'P05 baseline head is retained'); END;
"#;

fn compiled_ddl() -> String {
    let mut sql = DDL.to_owned();
    for (table, conflicts) in [
        (TABLES[0], "origin_identity=NEW.origin_identity OR (family=NEW.family AND origin_kind='ProspectiveNoPreviousV2Baseline' AND NEW.origin_kind='ProspectiveNoPreviousV2Baseline') OR (completed_receipt_identity IS NOT NULL AND completed_receipt_identity=NEW.completed_receipt_identity)"),
        (TABLES[1], "family=NEW.family"),
        (TABLES[2], "draft_identity=NEW.draft_identity OR business_date=NEW.business_date OR unit_occurrence=NEW.unit_occurrence"),
        (TABLES[3], "draft_identity=NEW.draft_identity"),
        (TABLES[4], "event_identity=NEW.event_identity OR draft_identity=NEW.draft_identity"),
        (TABLES[5], "intent_identity=NEW.intent_identity OR draft_identity=NEW.draft_identity"),
        (TABLES[6], "child_identity=NEW.child_identity OR decision_identity=NEW.decision_identity OR (intent_identity=NEW.intent_identity AND ordinal=NEW.ordinal)"),
    ] {
        sql.push_str(&format!("CREATE TRIGGER {table}_no_replace BEFORE INSERT ON {table} WHEN EXISTS(SELECT 1 FROM {table} WHERE {conflicts}) BEGIN SELECT RAISE(ABORT,'P05 evidence cannot be replaced'); END;\n"));
        if !matches!(table,"p05_unit_heads"|"p05_baseline_heads") {
            sql.push_str(&format!("CREATE TRIGGER {table}_no_update BEFORE UPDATE ON {table} BEGIN SELECT RAISE(ABORT,'P05 evidence is immutable'); END;\nCREATE TRIGGER {table}_no_delete BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT,'P05 evidence is retained'); END;\n"));
        }
    }
    sql
}

pub(super) fn reject_preexisting_extension(connection: &Connection) -> Result<()> {
    reject_temp(connection)?;
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'p05_*' OR lower(tbl_name) GLOB 'p05_*'", [], |r| r.get(0))?;
    if count != 0 {
        return Err(invalid("reserved P05 objects predate schema13"));
    }
    Ok(())
}

pub(super) fn initialize(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch(&compiled_ddl())?;
    verify_catalog(transaction)
}

pub(super) fn verify_catalog(connection: &Connection) -> Result<()> {
    reject_temp(connection)?;
    static EXPECTED: std::sync::OnceLock<Vec<(String, String, String)>> =
        std::sync::OnceLock::new();
    let expected = EXPECTED.get_or_init(|| {
        let sql = compiled_ddl();
        let starts = sql
            .match_indices("CREATE ")
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        for (index, start) in starts.iter().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(sql.len());
            let declaration = sql[*start..end].trim().trim_end_matches(';').trim_end();
            let mut words = declaration.split_whitespace();
            words.next();
            let kind = words.next().unwrap().to_ascii_lowercase();
            let name = words.next().unwrap().split('(').next().unwrap().to_owned();
            rows.push((kind, name, declaration.to_owned()));
        }
        rows.sort();
        rows
    });
    // Schema14 attests its separate p05_s2 catalog. The original seven table
    // declarations and their original triggers retain this exact manifest.
    let mut statement = connection.prepare("SELECT type,name,COALESCE(sql,'') FROM main.sqlite_master WHERE sql IS NOT NULL AND (lower(name) GLOB 'p05_*' OR lower(tbl_name) GLOB 'p05_*') AND lower(name) NOT GLOB 'p05_s2_*' AND lower(tbl_name) NOT GLOB 'p05_s2_*' ORDER BY type,name")?;
    let actual = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<(String, String, String)>>>()?;
    if actual != *expected {
        return Err(invalid("P05 schema objects differ from schema13"));
    }
    Ok(())
}

fn reject_temp(connection: &Connection) -> Result<()> {
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM temp.sqlite_master WHERE lower(name) GLOB 'p05_*' OR lower(tbl_name) GLOB 'p05_*'", [], |r| r.get(0))?;
    if count != 0 {
        return Err(invalid("temporary schema shadows reserved P05 objects"));
    }
    Ok(())
}

fn invalid(reason: &str) -> DurableDeliveryError {
    DurableDeliveryError::InvalidConfiguration(reason.to_owned())
}

#[cfg(test)]
pub(super) fn remove_empty_extension_for_legacy_test(connection: &Connection) {
    super::schema_p05_unit_runtime::remove_empty_extension_for_legacy_test(connection);
    for table in TABLES {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "legacy shaping must never discard P05 evidence");
    }
    for table in [
        TABLES[3], TABLES[6], TABLES[5], TABLES[4], TABLES[2], TABLES[1], TABLES[0],
    ] {
        connection
            .execute_batch(&format!("DROP TABLE {table};"))
            .unwrap();
    }
}
