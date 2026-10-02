//! Schema 14 adds P05 counted ownership and immutable physical completion evidence.
//! Schema 12 and 13 tables are retained byte for byte.
use super::model::{DurableDeliveryError, Result};
use rusqlite::{Connection, Transaction};

pub(super) const TABLES: [&str; 4] = [
    "p05_s2_child_owners",
    "p05_s2_mutation_events",
    "p05_s2_completion_receipts",
    "p05_s2_completion_heads",
];
const DDL: &str = r#"
CREATE TABLE p05_s2_child_owners(
 owner_identity TEXT PRIMARY KEY NOT NULL,
 child_identity TEXT NOT NULL UNIQUE REFERENCES p05_unit_children(child_identity),
 draft_identity TEXT NOT NULL REFERENCES p05_unit_drafts(draft_identity),
 intent_identity TEXT NOT NULL REFERENCES p05_unit_intents(intent_identity),
 decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions(decision_identity) DEFERRABLE INITIALLY DEFERRED,
 envelope_canonical BLOB NOT NULL CHECK(typeof(envelope_canonical)='blob'),
 envelope_sha256 TEXT NOT NULL CHECK(sha256_hex(envelope_canonical)=envelope_sha256),
 owner_canonical BLOB NOT NULL CHECK(typeof(owner_canonical)='blob' AND length(owner_canonical) BETWEEN 1 AND 4194304),
 owner_sha256 TEXT NOT NULL CHECK(sha256_hex(owner_canonical)=owner_sha256),
 owner_preimage BLOB NOT NULL CHECK(typeof(owner_preimage)='blob' AND sha256_hex(owner_preimage)=owner_identity)
);
CREATE TABLE p05_s2_mutation_events(
 event_identity TEXT PRIMARY KEY NOT NULL,
 draft_identity TEXT NOT NULL REFERENCES p05_unit_drafts(draft_identity),
 mutation_revision INTEGER NOT NULL CHECK(typeof(mutation_revision)='integer' AND mutation_revision>=4),
 decision_identity TEXT NOT NULL REFERENCES delivery_decisions(decision_identity),
 before_sha256 TEXT NOT NULL,
 after_sha256 TEXT NOT NULL,
 event_canonical BLOB NOT NULL CHECK(typeof(event_canonical)='blob' AND length(event_canonical) BETWEEN 1 AND 4194304),
 event_sha256 TEXT NOT NULL CHECK(sha256_hex(event_canonical)=event_sha256),
 event_preimage BLOB NOT NULL CHECK(typeof(event_preimage)='blob' AND sha256_hex(event_preimage)=event_identity),
 UNIQUE(draft_identity,mutation_revision)
);
CREATE TABLE p05_s2_completion_receipts(
 completion_identity TEXT PRIMARY KEY NOT NULL,
 draft_identity TEXT NOT NULL REFERENCES p05_unit_drafts(draft_identity),
 mutation_revision INTEGER NOT NULL CHECK(typeof(mutation_revision)='integer' AND mutation_revision>=4),
 completion_canonical BLOB NOT NULL CHECK(typeof(completion_canonical)='blob' AND length(completion_canonical) BETWEEN 1 AND 4194304),
 completion_sha256 TEXT NOT NULL CHECK(sha256_hex(completion_canonical)=completion_sha256),
 completion_preimage BLOB NOT NULL CHECK(typeof(completion_preimage)='blob' AND sha256_hex(completion_preimage)=completion_identity),
 UNIQUE(draft_identity,mutation_revision), UNIQUE(draft_identity,completion_identity)
);
CREATE TABLE p05_s2_completion_heads(
 draft_identity TEXT PRIMARY KEY NOT NULL REFERENCES p05_unit_drafts(draft_identity),
 first_completion_identity TEXT NOT NULL,
 current_completion_identity TEXT,
 FOREIGN KEY(draft_identity,first_completion_identity) REFERENCES p05_s2_completion_receipts(draft_identity,completion_identity),
 FOREIGN KEY(draft_identity,current_completion_identity) REFERENCES p05_s2_completion_receipts(draft_identity,completion_identity)
);
CREATE TRIGGER p05_s2_completion_head_update BEFORE UPDATE ON p05_s2_completion_heads
BEGIN SELECT CASE WHEN NEW.draft_identity!=OLD.draft_identity OR NEW.first_completion_identity!=OLD.first_completion_identity
 THEN RAISE(ABORT,'P05 first completion is immutable') END; END;
CREATE TRIGGER p05_s2_completion_head_retained BEFORE DELETE ON p05_s2_completion_heads
BEGIN SELECT RAISE(ABORT,'P05 completion head is retained'); END;
CREATE TRIGGER p05_s2_new_family_decision BEFORE INSERT ON delivery_decisions
WHEN NEW.push_kind IN ('AuctionRepush','CandidateBoard','CandidateInvalidated')
 AND EXISTS(SELECT 1 FROM p05_unit_drafts d WHERE d.business_date=NEW.business_date)
BEGIN SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM p05_s2_child_owners o JOIN p05_unit_children c ON c.child_identity=o.child_identity
 WHERE o.decision_identity=NEW.decision_identity AND c.decision_identity=NEW.decision_identity
 AND o.draft_identity=c.draft_identity AND o.intent_identity=c.intent_identity
 AND o.envelope_canonical=NEW.envelope_canonical AND o.envelope_sha256=NEW.envelope_sha256)
 THEN RAISE(ABORT,'P05 Unit requires actual contextual child owner') END; END;
CREATE TRIGGER p05_s2_completed_origin BEFORE INSERT ON p05_baseline_origins
WHEN NEW.origin_kind='CompletedUnitV2Baseline'
BEGIN SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM p05_s2_completion_receipts r
 WHERE r.completion_identity=NEW.completed_receipt_identity AND r.draft_identity=NEW.completed_unit_identity)
 THEN RAISE(ABORT,'P05 Completed baseline requires actual immutable completion') END; END;
"#;

fn compiled_ddl() -> String {
    let mut sql = DDL.to_owned();
    for (table,conflicts) in [
        (TABLES[0],"owner_identity=NEW.owner_identity OR child_identity=NEW.child_identity OR decision_identity=NEW.decision_identity"),
        (TABLES[1],"event_identity=NEW.event_identity OR (draft_identity=NEW.draft_identity AND mutation_revision=NEW.mutation_revision)"),
        (TABLES[2],"completion_identity=NEW.completion_identity OR (draft_identity=NEW.draft_identity AND mutation_revision=NEW.mutation_revision)"),
        (TABLES[3],"draft_identity=NEW.draft_identity"),
    ] {
        sql.push_str(&format!("CREATE TRIGGER {table}_no_replace BEFORE INSERT ON {table} WHEN EXISTS(SELECT 1 FROM {table} WHERE {conflicts}) BEGIN SELECT RAISE(ABORT,'P05 S2 evidence cannot be replaced'); END;\n"));
        if table!=TABLES[3] {
            sql.push_str(&format!("CREATE TRIGGER {table}_no_update BEFORE UPDATE ON {table} BEGIN SELECT RAISE(ABORT,'P05 S2 evidence is immutable'); END;\nCREATE TRIGGER {table}_no_delete BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT,'P05 S2 evidence is retained'); END;\n"));
        }
    }
    sql
}
fn invalid(reason: &str) -> DurableDeliveryError {
    DurableDeliveryError::InvalidConfiguration(reason.into())
}
fn reject_temp(c: &Connection) -> Result<()> {
    let n:i64=c.query_row("SELECT COUNT(*) FROM temp.sqlite_master WHERE lower(name) GLOB 'p05_s2_*' OR lower(tbl_name) GLOB 'p05_s2_*'",[],|r|r.get(0))?;
    if n != 0 {
        return Err(invalid("temporary P05 S2 shadow"));
    }
    Ok(())
}
pub(super) fn reject_preexisting_extension(c: &Connection) -> Result<()> {
    reject_temp(c)?;
    let n:i64=c.query_row("SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'p05_s2_*' OR lower(tbl_name) GLOB 'p05_s2_*'",[],|r|r.get(0))?;
    if n != 0 {
        return Err(invalid("reserved P05 S2 objects predate schema14"));
    }
    Ok(())
}
pub(super) fn initialize(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(&compiled_ddl())?;
    verify_catalog(tx)
}
pub(super) fn verify_catalog(c: &Connection) -> Result<()> {
    reject_temp(c)?;
    static EXPECTED: std::sync::OnceLock<Vec<(String, String, String)>> =
        std::sync::OnceLock::new();
    let expected = EXPECTED.get_or_init(|| {
        let sql = compiled_ddl();
        let starts = sql
            .match_indices("CREATE ")
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        for (i, start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(sql.len());
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
    let rows=c.prepare("SELECT type,name,COALESCE(sql,'') FROM main.sqlite_master WHERE sql IS NOT NULL AND (lower(name) GLOB 'p05_s2_*' OR lower(tbl_name) GLOB 'p05_s2_*') ORDER BY type,name")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<Vec<(String,String,String)>>>()?;
    if rows != *expected {
        return Err(invalid("P05 S2 catalog differs from schema14"));
    }
    Ok(())
}
#[cfg(test)]
pub(super) fn remove_empty_extension_for_legacy_test(c: &Connection) {
    let present: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name=?1",
            [TABLES[0]],
            |r| r.get(0),
        )
        .unwrap();
    if present == 0 {
        return;
    }
    verify_catalog(c).unwrap();
    for table in TABLES {
        assert_eq!(
            c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "legacy shaping must not discard S2 evidence"
        );
    }
    c.execute_batch(
        "DROP TRIGGER p05_s2_new_family_decision; DROP TRIGGER p05_s2_completed_origin;",
    )
    .unwrap();
    for table in [TABLES[3], TABLES[2], TABLES[1], TABLES[0]] {
        c.execute_batch(&format!("DROP TABLE {table};")).unwrap();
    }
}
