//! Narrow native-main snapshot activation. It does not issue GlobalSchema
//! authority or change the ordinary monitor database identity.

use crate::trading::paper_ledger::{AccountBinding, LedgerError};
use crate::trading::paper_snapshot_activation::{validate_stored_proof, ActivationProof};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use once_cell::sync::Lazy;
use serde::Serialize;
use sha2::{Digest, Sha256};

const SOURCE_TABLES: &[&str] = &[
    "paper_trades",
    "order_audit",
    "order_audit_chain",
    "user_account_summary",
    "user_position_snapshot",
    "user_position_snapshot_item",
];

pub(crate) const STATEMENTS: &[&str] = &[
    "CREATE TABLE paper_snapshot_activation_v1 (
       singleton INTEGER PRIMARY KEY CHECK(singleton=1),
       account_id TEXT NOT NULL UNIQUE,epoch_id TEXT NOT NULL UNIQUE,
       manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
       proof_hash TEXT NOT NULL CHECK(length(proof_hash)=64),proof_bytes TEXT NOT NULL)",
    "CREATE TRIGGER paper_snapshot_activation_no_update BEFORE UPDATE ON paper_snapshot_activation_v1
       BEGIN SELECT RAISE(ABORT,'immutable snapshot paper activation'); END",
    "CREATE TRIGGER paper_snapshot_activation_no_delete BEFORE DELETE ON paper_snapshot_activation_v1
       BEGIN SELECT RAISE(ABORT,'immutable snapshot paper activation'); END",
    "CREATE TRIGGER paper_snapshot_activation_no_reinsert BEFORE INSERT ON paper_snapshot_activation_v1
       WHEN EXISTS(SELECT 1 FROM paper_snapshot_activation_v1)
         OR EXISTS(SELECT 1 FROM paper_ledger_account)
       BEGIN SELECT RAISE(ABORT,'single immutable snapshot paper activation'); END",
    "CREATE TRIGGER paper_snapshot_activation_account_insert BEFORE INSERT ON paper_ledger_account
       WHEN EXISTS(SELECT 1 FROM paper_ledger_account)
         OR NOT EXISTS(SELECT 1 FROM paper_snapshot_activation_v1
             WHERE account_id=NEW.account_id AND epoch_id=NEW.epoch_id AND manifest_hash=NEW.manifest_hash)
       BEGIN SELECT RAISE(ABORT,'snapshot paper account binding mismatch'); END",
    "CREATE TRIGGER paper_snapshot_activation_event_insert BEFORE INSERT ON paper_ledger_event
       WHEN NOT EXISTS(SELECT 1 FROM paper_snapshot_activation_v1 p JOIN paper_ledger_account a
             ON a.account_id=p.account_id AND a.epoch_id=p.epoch_id AND a.manifest_hash=p.manifest_hash
             WHERE p.account_id=NEW.account_id)
         OR EXISTS(SELECT 1 FROM paper_ledger_event
             WHERE (account_id=NEW.account_id AND (seq=NEW.seq OR command_id=NEW.command_id
               OR (NEW.is_terminal=1 AND NEW.business_plan_id IS NOT NULL AND is_terminal=1
                   AND business_plan_id=NEW.business_plan_id)))
               OR (NEW.paper_trade_id IS NOT NULL AND paper_trade_id=NEW.paper_trade_id)
               OR (NEW.order_audit_id IS NOT NULL AND order_audit_id=NEW.order_audit_id))
       BEGIN SELECT RAISE(ABORT,'snapshot paper event binding mismatch or replacement'); END",
    "CREATE TRIGGER paper_snapshot_activation_head_insert BEFORE INSERT ON paper_ledger_head
       WHEN NOT EXISTS(SELECT 1 FROM paper_snapshot_activation_v1 p JOIN paper_ledger_account a
             ON a.account_id=p.account_id AND a.epoch_id=p.epoch_id AND a.manifest_hash=p.manifest_hash
             WHERE p.account_id=NEW.account_id)
         OR EXISTS(SELECT 1 FROM paper_ledger_head WHERE account_id=NEW.account_id)
       BEGIN SELECT RAISE(ABORT,'snapshot paper head binding mismatch or replacement'); END",
    "CREATE TRIGGER paper_snapshot_activation_head_update BEFORE UPDATE ON paper_ledger_head
       WHEN NEW.account_id!=OLD.account_id OR NOT EXISTS(
           SELECT 1 FROM paper_snapshot_activation_v1 p JOIN paper_ledger_account a
             ON a.account_id=p.account_id AND a.epoch_id=p.epoch_id AND a.manifest_hash=p.manifest_hash
             WHERE p.account_id=NEW.account_id)
       BEGIN SELECT RAISE(ABORT,'snapshot paper head binding mismatch'); END",
];

#[derive(Debug, Clone, PartialEq, Eq, QueryableByName, Serialize)]
struct CatalogObject {
    #[diesel(sql_type=Text)]
    namespace: String,
    #[diesel(sql_type=Text)]
    kind: String,
    #[diesel(sql_type=Text)]
    name: String,
    #[diesel(sql_type=Text)]
    table_name: String,
    #[diesel(sql_type=Nullable<Text>)]
    sql: Option<String>,
}
#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    value: i64,
}
#[derive(QueryableByName)]
struct Identity {
    #[diesel(sql_type=BigInt)]
    application_id: i64,
    #[diesel(sql_type=BigInt)]
    user_version: i64,
}
#[derive(QueryableByName)]
struct ProofRow {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    epoch_id: String,
    #[diesel(sql_type=Text)]
    manifest_hash: String,
    #[diesel(sql_type=Text)]
    proof_hash: String,
    #[diesel(sql_type=Text)]
    proof_bytes: String,
}

fn failure(message: &str) -> LedgerError {
    LedgerError::IntegrityFailure(message.into())
}
pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(crate) fn require_native_identity_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    let identity = diesel::sql_query(
        "SELECT application_id,user_version FROM pragma_application_id(),pragma_user_version()",
    )
    .get_result::<Identity>(conn)?;
    if identity.application_id != 0 || identity.user_version != 0 {
        return Err(failure(
            "snapshot activation requires the unchanged native-main database identity",
        ));
    }
    Ok(())
}
pub(crate) fn is_present_on(conn: &mut SqliteConnection) -> Result<bool, LedgerError> {
    let count = diesel::sql_query("SELECT ((SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'paper_snapshot_activation_*' OR lower(tbl_name) GLOB 'paper_snapshot_activation_*')
      + (SELECT COUNT(*) FROM temp.sqlite_master WHERE lower(name) GLOB 'paper_snapshot_activation_*' OR lower(tbl_name) GLOB 'paper_snapshot_activation_*')) AS value")
        .get_result::<Count>(conn)?.value;
    Ok(count != 0)
}
fn namespace_objects_on(conn: &mut SqliteConnection) -> Result<Vec<CatalogObject>, LedgerError> {
    Ok(diesel::sql_query("SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql FROM main.sqlite_master
      WHERE lower(name) GLOB 'paper_ledger_*' OR lower(tbl_name) GLOB 'paper_ledger_*'
        OR lower(name) GLOB 'paper_snapshot_activation_*' OR lower(tbl_name) GLOB 'paper_snapshot_activation_*'
        OR lower(name) GLOB 'paper_book_owner_*' OR lower(tbl_name) GLOB 'paper_book_owner_*'
        OR lower(name) GLOB 'paper_book_v2_*' OR lower(tbl_name) GLOB 'paper_book_v2_*'
      UNION ALL SELECT 'temp',type,name,tbl_name,sql FROM temp.sqlite_master
      WHERE lower(name) GLOB 'paper_ledger_*' OR lower(tbl_name) GLOB 'paper_ledger_*'
        OR lower(name) GLOB 'paper_snapshot_activation_*' OR lower(tbl_name) GLOB 'paper_snapshot_activation_*'
        OR lower(name) GLOB 'paper_book_owner_*' OR lower(tbl_name) GLOB 'paper_book_owner_*'
        OR lower(name) GLOB 'paper_book_v2_*' OR lower(tbl_name) GLOB 'paper_book_v2_*'
      ORDER BY namespace,kind,name,table_name,sql").load(conn)?)
}
fn source_objects_on(conn: &mut SqliteConnection) -> Result<Vec<CatalogObject>, LedgerError> {
    Ok(diesel::sql_query("SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql FROM main.sqlite_master
       WHERE lower(name) IN ('paper_trades','order_audit','order_audit_chain','user_account_summary','user_position_snapshot','user_position_snapshot_item')
          OR lower(tbl_name) IN ('paper_trades','order_audit','order_audit_chain','user_account_summary','user_position_snapshot','user_position_snapshot_item')
       UNION ALL SELECT 'temp',type,name,tbl_name,sql FROM temp.sqlite_master
       WHERE lower(name) IN ('paper_trades','order_audit','order_audit_chain','user_account_summary','user_position_snapshot','user_position_snapshot_item')
          OR lower(tbl_name) IN ('paper_trades','order_audit','order_audit_chain','user_account_summary','user_position_snapshot','user_position_snapshot_item')
       ORDER BY namespace,kind,name,table_name,sql").load(conn)?)
}

/// Replay only the frozen fact-table subset into an in-memory reference. No
/// caller catalog is normalized, rewritten, or granted whole-catalog authority.
pub(crate) fn create_source_reference_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    let fixture = include_str!("fixtures/global_schema_legacy_ddl_v1.tsv");
    if hash_bytes(fixture.as_bytes())
        != "938c1bc039e0b2443ab0f2e3a8c5bebe218b2c6fe928f6ad2c15d3019fde9922"
    {
        return Err(failure("frozen source DDL fixture digest mismatch"));
    }
    let entries = super::global_schema_catalog_v1::legacy_catalog_registry_entries_v1()
        .map_err(|e| failure(&e.to_string()))?;
    for line in fixture.lines().filter(|line| !line.starts_with('#')) {
        let (id, sql_hex) = line
            .split_once('|')
            .ok_or_else(|| failure("invalid frozen source DDL"))?;
        if entries.iter().any(|entry| {
            entry.ddl_id == id && SOURCE_TABLES.contains(&entry.identity.table_name.as_str())
        }) {
            let sql = String::from_utf8(
                hex::decode(sql_hex).map_err(|_| failure("invalid frozen source SQL encoding"))?,
            )
            .map_err(|_| failure("invalid frozen source SQL text"))?;
            conn.batch_execute(&sql)?;
        }
    }
    Ok(())
}
static EXPECTED_SOURCE: Lazy<Result<Vec<CatalogObject>, String>> = Lazy::new(|| {
    let mut conn = SqliteConnection::establish(":memory:").map_err(|e| e.to_string())?;
    create_source_reference_on(&mut conn).map_err(|e| e.to_string())?;
    source_objects_on(&mut conn).map_err(|e| e.to_string())
});
static EXPECTED_NAMESPACE: Lazy<Result<Vec<CatalogObject>, String>> = Lazy::new(|| {
    let mut conn = SqliteConnection::establish(":memory:").map_err(|e| e.to_string())?;
    super::paper_ledger_schema_v1::create_schema(&mut conn).map_err(|e| e.to_string())?;
    for sql in STATEMENTS {
        conn.batch_execute(sql).map_err(|e| e.to_string())?;
    }
    namespace_objects_on(&mut conn).map_err(|e| e.to_string())
});
pub(crate) fn verify_source_catalog_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    if source_objects_on(conn)? != *EXPECTED_SOURCE.as_ref().map_err(|e| failure(e))? {
        return Err(failure("snapshot activation source fact namespace is missing, altered, shadowed, or has extra objects"));
    }
    Ok(())
}
pub(crate) fn require_uninitialized_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    require_native_identity_on(conn)?;
    verify_source_catalog_on(conn)?;
    if !namespace_objects_on(conn)?.is_empty() {
        return Err(failure(
            "snapshot activation refuses an existing or partial paper namespace",
        ));
    }
    Ok(())
}
pub(crate) fn verify_namespace_on(conn: &mut SqliteConnection) -> Result<String, LedgerError> {
    require_native_identity_on(conn)?;
    verify_source_catalog_on(conn)?;
    let objects = namespace_objects_on(conn)?;
    if objects != *EXPECTED_NAMESPACE.as_ref().map_err(|e| failure(e))? {
        return Err(failure(
            "snapshot paper namespace is incomplete, altered, shadowed, or has extra objects",
        ));
    }
    Ok(hash_bytes(
        &serde_json::to_vec(&objects).map_err(|e| failure(&e.to_string()))?,
    ))
}
pub(crate) fn install_on(
    conn: &mut SqliteConnection,
    proof: &ActivationProof,
) -> Result<(), LedgerError> {
    require_uninitialized_on(conn)?;
    let binding = validate_stored_proof(proof)?;
    super::paper_ledger_schema_v1::create_schema(conn)?;
    for sql in STATEMENTS {
        conn.batch_execute(sql)?;
    }
    let bytes = serde_json::to_string(proof).map_err(|e| failure(&e.to_string()))?;
    diesel::sql_query("INSERT INTO main.paper_snapshot_activation_v1(singleton,account_id,epoch_id,manifest_hash,proof_hash,proof_bytes) VALUES(1,?,?,?,?,?)")
        .bind::<Text,_>(&binding.account_id).bind::<Text,_>(&binding.epoch_id).bind::<Text,_>(&binding.manifest_hash)
        .bind::<Text,_>(hash_bytes(bytes.as_bytes())).bind::<Text,_>(bytes).execute(conn)?;
    verify_namespace_on(conn)?;
    Ok(())
}
pub(crate) fn proof_on(conn: &mut SqliteConnection) -> Result<ActivationProof, LedgerError> {
    verify_namespace_on(conn)?;
    let rows = diesel::sql_query("SELECT account_id,epoch_id,manifest_hash,proof_hash,proof_bytes FROM main.paper_snapshot_activation_v1")
        .load::<ProofRow>(conn)?;
    let [row] = rows.as_slice() else {
        return Err(failure(
            "snapshot paper activation requires exactly one immutable proof",
        ));
    };
    if hash_bytes(row.proof_bytes.as_bytes()) != row.proof_hash {
        return Err(failure("snapshot paper activation proof hash mismatch"));
    }
    let proof: ActivationProof =
        serde_json::from_str(&row.proof_bytes).map_err(|e| failure(&e.to_string()))?;
    let binding = validate_stored_proof(&proof)?;
    if binding.account_id != row.account_id
        || binding.epoch_id != row.epoch_id
        || binding.manifest_hash != row.manifest_hash
        || serde_json::to_string(&proof).map_err(|e| failure(&e.to_string()))? != row.proof_bytes
    {
        return Err(failure(
            "snapshot paper activation proof binding or canonical bytes mismatch",
        ));
    }
    Ok(proof)
}
pub(crate) fn require_owner_on(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
) -> Result<(), LedgerError> {
    if verify_active_on(conn)? != *binding {
        return Err(LedgerError::InactiveEpoch);
    }
    Ok(())
}
pub(crate) fn verify_active_on(conn: &mut SqliteConnection) -> Result<AccountBinding, LedgerError> {
    let proof = proof_on(conn)?;
    let binding = proof.request.seed.binding()?;
    let bytes = serde_json::to_string(&proof.request.seed).map_err(|e| failure(&e.to_string()))?;
    let accounts = diesel::sql_query("SELECT COUNT(*) AS value FROM main.paper_ledger_account")
        .get_result::<Count>(conn)?
        .value;
    let matched = diesel::sql_query("SELECT COUNT(*) AS value FROM main.paper_ledger_account a JOIN main.paper_ledger_head h ON h.account_id=a.account_id
        WHERE a.account_id=? AND a.epoch_id=? AND a.manifest_hash=? AND a.manifest_bytes=?
          AND a.money_model='micro-cny-half-up-v1' AND a.fee_model='lot-rates-v1' AND h.version>=1
          AND EXISTS(SELECT 1 FROM main.paper_ledger_event e WHERE e.account_id=a.account_id AND e.seq=1)")
        .bind::<Text,_>(&binding.account_id).bind::<Text,_>(&binding.epoch_id).bind::<Text,_>(&binding.manifest_hash)
        .bind::<Text,_>(bytes).get_result::<Count>(conn)?.value;
    if accounts != 1 || matched != 1 {
        return Err(failure(
            "snapshot paper activation has missing or conflicting seeded account/head",
        ));
    }
    Ok(binding)
}
