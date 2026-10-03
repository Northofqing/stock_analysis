//! Additive CatalogV6 parent-order namespace. The V1–V5 catalogs and V2
//! genesis remain unchanged. Local DDL verification never grants catalog,
//! budget, decision, market-data, or execution authority.

use crate::trading::paper_ledger::LedgerError;
use diesel::prelude::*;
use diesel::sql_types::Text;

pub(crate) const CATALOG_GENERATION: i64 = 6;
pub(crate) const CATALOG_DOMAIN: &str = "stock_analysis.paper_book_v2.execution.catalog.v1";

pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table", "paper_book_v2_execution_manifest", "paper_book_v2_execution_manifest", "CREATE TABLE paper_book_v2_execution_manifest (
        account_id TEXT PRIMARY KEY NOT NULL REFERENCES paper_book_v2_account(account_id),
        epoch_id TEXT NOT NULL,
        cutover_id TEXT NOT NULL,
        genesis_event_hash TEXT NOT NULL CHECK(length(genesis_event_hash)=64),
        manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
        manifest_bytes BLOB NOT NULL CHECK(typeof(manifest_bytes)='blob' AND length(manifest_bytes)>0),
        fee_policy_instance_id TEXT NOT NULL,
        budget_policy_hash TEXT NOT NULL CHECK(length(budget_policy_hash)=64),
        fill_model_version TEXT NOT NULL,
        family_id TEXT NOT NULL,
        UNIQUE(account_id,epoch_id))"),
    ("table", "paper_book_v2_parent_order", "paper_book_v2_parent_order", "CREATE TABLE paper_book_v2_parent_order (
        account_id TEXT NOT NULL REFERENCES paper_book_v2_execution_manifest(account_id),
        parent_id TEXT NOT NULL,
        epoch_id TEXT NOT NULL,
        manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
        investment_decision_id TEXT NOT NULL,
        side TEXT NOT NULL CHECK(side IN ('Buy','Sell')),
        instrument_code TEXT NOT NULL,
        requested_quantity INTEGER NOT NULL CHECK(typeof(requested_quantity)='integer' AND requested_quantity>0 AND requested_quantity%100=0),
        max_price_micro_cny INTEGER NOT NULL CHECK(typeof(max_price_micro_cny)='integer' AND max_price_micro_cny>0),
        session_date TEXT NOT NULL,
        intent_hash TEXT NOT NULL CHECK(length(intent_hash)=64),
        intent_bytes BLOB NOT NULL CHECK(typeof(intent_bytes)='blob' AND length(intent_bytes)>0),
        PRIMARY KEY(account_id,parent_id), UNIQUE(account_id,investment_decision_id))"),
    ("table", "paper_book_v2_execution_event", "paper_book_v2_execution_event", "CREATE TABLE paper_book_v2_execution_event (
        account_id TEXT NOT NULL REFERENCES paper_book_v2_execution_manifest(account_id),
        seq INTEGER NOT NULL CHECK(typeof(seq)='integer' AND seq>0),
        command_id TEXT NOT NULL,
        command_hash TEXT NOT NULL CHECK(length(command_hash)=64),
        parent_id TEXT,
        previous_hash TEXT NOT NULL CHECK(length(previous_hash)=64),
        event_hash TEXT NOT NULL CHECK(length(event_hash)=64),
        kind TEXT NOT NULL CHECK(kind IN ('ExecutionOpenedV1','ParentSubmittedV1','ParentObservedNoFillV1','ParentFilledV1','ParentCancelledV1','ParentExpiredV1','QualifiedMarksV1')),
        payload BLOB NOT NULL CHECK(typeof(payload)='blob' AND length(payload)>0),
        PRIMARY KEY(account_id,seq), UNIQUE(account_id,command_id),
        FOREIGN KEY(account_id,parent_id) REFERENCES paper_book_v2_parent_order(account_id,parent_id))"),
    ("table", "paper_book_v2_execution_head", "paper_book_v2_execution_head", "CREATE TABLE paper_book_v2_execution_head (
        account_id TEXT PRIMARY KEY NOT NULL REFERENCES paper_book_v2_execution_manifest(account_id),
        version INTEGER NOT NULL CHECK(typeof(version)='integer' AND version>0),
        event_hash TEXT NOT NULL CHECK(length(event_hash)=64),
        projection_hash TEXT NOT NULL CHECK(length(projection_hash)=64),
        projection_bytes BLOB NOT NULL CHECK(typeof(projection_bytes)='blob' AND length(projection_bytes)>0))"),
    ("trigger", "paper_book_v2_execution_manifest_no_update", "paper_book_v2_execution_manifest", "CREATE TRIGGER paper_book_v2_execution_manifest_no_update BEFORE UPDATE ON paper_book_v2_execution_manifest
        BEGIN SELECT RAISE(ABORT,'immutable execution manifest'); END"),
    ("trigger", "paper_book_v2_execution_manifest_no_delete", "paper_book_v2_execution_manifest", "CREATE TRIGGER paper_book_v2_execution_manifest_no_delete BEFORE DELETE ON paper_book_v2_execution_manifest
        BEGIN SELECT RAISE(ABORT,'immutable execution manifest'); END"),
    ("trigger", "paper_book_v2_execution_manifest_no_reinsert", "paper_book_v2_execution_manifest", "CREATE TRIGGER paper_book_v2_execution_manifest_no_reinsert BEFORE INSERT ON paper_book_v2_execution_manifest
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_execution_manifest WHERE account_id=NEW.account_id)
        BEGIN SELECT RAISE(ABORT,'immutable execution manifest'); END"),
    ("trigger", "paper_book_v2_parent_order_no_update", "paper_book_v2_parent_order", "CREATE TRIGGER paper_book_v2_parent_order_no_update BEFORE UPDATE ON paper_book_v2_parent_order
        BEGIN SELECT RAISE(ABORT,'immutable parent intent'); END"),
    ("trigger", "paper_book_v2_parent_order_no_delete", "paper_book_v2_parent_order", "CREATE TRIGGER paper_book_v2_parent_order_no_delete BEFORE DELETE ON paper_book_v2_parent_order
        BEGIN SELECT RAISE(ABORT,'immutable parent intent'); END"),
    ("trigger", "paper_book_v2_parent_order_no_reinsert", "paper_book_v2_parent_order", "CREATE TRIGGER paper_book_v2_parent_order_no_reinsert BEFORE INSERT ON paper_book_v2_parent_order
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_parent_order WHERE account_id=NEW.account_id AND parent_id=NEW.parent_id)
        BEGIN SELECT RAISE(ABORT,'immutable parent intent'); END"),
    ("trigger", "paper_book_v2_execution_event_no_update", "paper_book_v2_execution_event", "CREATE TRIGGER paper_book_v2_execution_event_no_update BEFORE UPDATE ON paper_book_v2_execution_event
        BEGIN SELECT RAISE(ABORT,'append-only execution event'); END"),
    ("trigger", "paper_book_v2_execution_event_no_delete", "paper_book_v2_execution_event", "CREATE TRIGGER paper_book_v2_execution_event_no_delete BEFORE DELETE ON paper_book_v2_execution_event
        BEGIN SELECT RAISE(ABORT,'append-only execution event'); END"),
    ("trigger", "paper_book_v2_execution_event_no_reinsert", "paper_book_v2_execution_event", "CREATE TRIGGER paper_book_v2_execution_event_no_reinsert BEFORE INSERT ON paper_book_v2_execution_event
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_execution_event WHERE account_id=NEW.account_id AND (seq=NEW.seq OR command_id=NEW.command_id))
        BEGIN SELECT RAISE(ABORT,'append-only execution event'); END"),
    ("trigger", "paper_book_v2_execution_event_chain", "paper_book_v2_execution_event", "CREATE TRIGGER paper_book_v2_execution_event_chain BEFORE INSERT ON paper_book_v2_execution_event
        WHEN NOT ((NEW.seq=1 AND NEW.kind='ExecutionOpenedV1' AND NEW.parent_id IS NULL
            AND NOT EXISTS(SELECT 1 FROM paper_book_v2_execution_head WHERE account_id=NEW.account_id)
            AND EXISTS(SELECT 1 FROM paper_book_v2_execution_manifest WHERE account_id=NEW.account_id AND genesis_event_hash=NEW.previous_hash))
            OR (NEW.seq>1 AND NEW.kind!='ExecutionOpenedV1'
            AND EXISTS(SELECT 1 FROM paper_book_v2_execution_head WHERE account_id=NEW.account_id AND version=NEW.seq-1 AND event_hash=NEW.previous_hash)))
        BEGIN SELECT RAISE(ABORT,'execution chain gap'); END"),
    ("trigger", "paper_book_v2_execution_head_no_delete", "paper_book_v2_execution_head", "CREATE TRIGGER paper_book_v2_execution_head_no_delete BEFORE DELETE ON paper_book_v2_execution_head
        BEGIN SELECT RAISE(ABORT,'execution head deletion'); END"),
    ("trigger", "paper_book_v2_execution_head_insert", "paper_book_v2_execution_head", "CREATE TRIGGER paper_book_v2_execution_head_insert BEFORE INSERT ON paper_book_v2_execution_head
        WHEN NEW.version!=1 OR EXISTS(SELECT 1 FROM paper_book_v2_execution_head WHERE account_id=NEW.account_id)
            OR NOT EXISTS(SELECT 1 FROM paper_book_v2_execution_event WHERE account_id=NEW.account_id AND seq=1 AND event_hash=NEW.event_hash AND kind='ExecutionOpenedV1')
        BEGIN SELECT RAISE(ABORT,'execution head initial mismatch'); END"),
    ("trigger", "paper_book_v2_execution_head_cas", "paper_book_v2_execution_head", "CREATE TRIGGER paper_book_v2_execution_head_cas BEFORE UPDATE ON paper_book_v2_execution_head
        WHEN NEW.account_id!=OLD.account_id OR NEW.version!=OLD.version+1
            OR NOT EXISTS(SELECT 1 FROM paper_book_v2_execution_event WHERE account_id=NEW.account_id AND seq=NEW.version AND event_hash=NEW.event_hash AND previous_hash=OLD.event_hash)
        BEGIN SELECT RAISE(ABORT,'execution head CAS mismatch'); END"),
];

/// Used by same-runtime catalog reference construction. This function does
/// not install a policy, open an account, or upgrade application/user version.
pub(crate) fn create_schema(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    for (_, _, _, sql) in STATEMENTS {
        diesel::sql_query(*sql).execute(conn)?;
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, QueryableByName)]
struct Object {
    #[diesel(sql_type = Text)]
    namespace: String,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    owner: String,
    #[diesel(sql_type = Text)]
    sql: String,
}

/// Exact local namespace only. Whole CatalogV6 classification, original V1
/// history, sole owner2, fee descriptor, genesis and execution row replay are
/// separate mandatory validators in the same SQLite transaction.
pub(crate) fn verify_objects_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    let actual = diesel::sql_query("SELECT 'main' AS namespace,type AS kind,name,tbl_name AS owner,sql
        FROM main.sqlite_master WHERE sql IS NOT NULL AND
        (lower(name) GLOB 'paper_book_v2_execution_*' OR lower(tbl_name) GLOB 'paper_book_v2_execution_*'
            OR lower(name) GLOB 'paper_book_v2_parent_order*' OR lower(tbl_name) GLOB 'paper_book_v2_parent_order*')
        UNION ALL SELECT 'temp' AS namespace,type AS kind,name,tbl_name AS owner,sql
        FROM temp.sqlite_master WHERE sql IS NOT NULL AND
        (lower(name) GLOB 'paper_book_v2_execution_*' OR lower(tbl_name) GLOB 'paper_book_v2_execution_*'
            OR lower(name) GLOB 'paper_book_v2_parent_order*' OR lower(tbl_name) GLOB 'paper_book_v2_parent_order*')
        ORDER BY namespace,kind,name,owner,sql").load::<Object>(conn)?;
    let mut expected: Vec<Object> = STATEMENTS
        .iter()
        .map(|(kind, name, owner, sql)| Object {
            namespace: "main".into(),
            kind: (*kind).into(),
            name: (*name).into(),
            owner: (*owner).into(),
            sql: (*sql).into(),
        })
        .collect();
    expected.sort();
    if actual != expected {
        return Err(LedgerError::IntegrityFailure(
            "V6 execution namespace mismatch".into(),
        ));
    }
    Ok(())
}
