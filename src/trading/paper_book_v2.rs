//! CatalogV5 zero-fill book cutover. The persistent reader validates both
//! generations; the only writer is compiled for isolated tests.

use crate::database::DatabaseManager;
#[cfg(test)]
use crate::trading::paper_ledger::verified_v1_snapshot_on;
use crate::trading::paper_ledger::{
    verified_v1_snapshot_with_audit_guard_on, AccountBinding, LedgerError, Money,
    V1AuditReplayGuard, VerifiedV1Snapshot,
};
#[cfg(test)]
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Nullable, Text};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MANIFEST_SCHEMA: &str = "paper-book-v2-cutover-manifest/v1";
const GENESIS_SCHEMA: &str = "paper-book-v2-genesis/v1";

#[derive(QueryableByName)]
struct V1AccountRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    epoch_id: String,
    #[diesel(sql_type = Text)]
    manifest_hash: String,
}

#[derive(QueryableByName)]
struct OwnerRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    active_generation: i64,
    #[diesel(sql_type = Text)]
    active_epoch_id: String,
    #[diesel(sql_type = Text)]
    active_manifest_hash: String,
    #[diesel(sql_type = BigInt)]
    owner_revision: i64,
    #[diesel(sql_type = Nullable<Text>)]
    cutover_id: Option<String>,
}

#[derive(QueryableByName)]
struct V2AccountRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    epoch_id: String,
    #[diesel(sql_type = Text)]
    manifest_hash: String,
    #[diesel(sql_type = Binary)]
    manifest_bytes: Vec<u8>,
    #[diesel(sql_type = Text)]
    fee_policy_instance_id: String,
    #[diesel(sql_type = Text)]
    v1_epoch_id: String,
    #[diesel(sql_type = Text)]
    v1_manifest_hash: String,
    #[diesel(sql_type = BigInt)]
    v1_head_version: i64,
    #[diesel(sql_type = Text)]
    v1_head_hash: String,
    #[diesel(sql_type = Text)]
    v1_projection_hash: String,
    #[diesel(sql_type = Text)]
    cutover_id: String,
}

#[derive(QueryableByName)]
struct V2EventRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    seq: i64,
    #[diesel(sql_type = Text)]
    command_id: String,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    event_hash: String,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Binary)]
    payload: Vec<u8>,
}

#[derive(QueryableByName)]
struct V2HeadRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    version: i64,
    #[diesel(sql_type = Text)]
    event_hash: String,
    #[diesel(sql_type = Binary)]
    projection_bytes: Vec<u8>,
    #[diesel(sql_type = Text)]
    projection_hash: String,
}

#[derive(QueryableByName)]
struct FeeRow {
    #[diesel(sql_type = Text)]
    policy_instance_id: String,
    #[diesel(sql_type = Binary)]
    descriptor_bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CutoverManifestV1 {
    schema: String,
    account_id: String,
    epoch_id: String,
    cutover_id: String,
    command_id: String,
    v1_epoch_id: String,
    v1_manifest_hash: String,
    v1_head_version: i64,
    v1_head_hash: String,
    v1_projection_hash: String,
    v1_equity: Money,
    fee_policy_instance_id: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct GenesisPayloadV1 {
    schema: String,
    manifest_hash: String,
    v1_head_hash: String,
    v1_projection_hash: String,
    fee_policy_instance_id: String,
    cutover_id: String,
}

fn invalid(detail: &str) -> LedgerError {
    LedgerError::IntegrityFailure(format!("V2 book {detail}"))
}

fn require(condition: bool, detail: &str) -> Result<(), LedgerError> {
    if condition {
        Ok(())
    } else {
        Err(invalid(detail))
    }
}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, LedgerError> {
    serde_json::to_vec(value).map_err(|error| invalid(&error.to_string()))
}

fn decode_canonical<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, LedgerError> {
    let decoded: T = serde_json::from_slice(bytes).map_err(|error| invalid(&error.to_string()))?;
    require(canonical(&decoded)? == bytes, "noncanonical stored bytes")?;
    Ok(decoded)
}

fn sha256_domain(domain: &[u8], bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn manifest_hash(bytes: &[u8]) -> String {
    sha256_domain(b"paper-book-v2-cutover-manifest/v1\n", bytes)
}

fn genesis_hash(
    account_id: &str,
    command_id: &str,
    previous_hash: &str,
    payload: &[u8],
) -> Result<String, LedgerError> {
    Ok(sha256_domain(
        b"paper-book-v2-genesis-event/v1\n",
        &canonical(&(account_id, 1_i64, command_id, previous_hash, payload))?,
    ))
}

fn projection_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn fee_row(conn: &mut SqliteConnection) -> Result<FeeRow, LedgerError> {
    Ok(diesel::sql_query("SELECT policy_instance_id,descriptor_bytes FROM paper_book_v2_fee_manifest WHERE singleton=1")
        .get_result(conn)?)
}

fn v2_account(conn: &mut SqliteConnection, account_id: &str) -> Result<V2AccountRow, LedgerError> {
    Ok(diesel::sql_query(
        "SELECT account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,
        v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id
        FROM paper_book_v2_account WHERE account_id=?",
    )
    .bind::<Text, _>(account_id)
    .get_result(conn)?)
}

fn v2_event(conn: &mut SqliteConnection, account_id: &str) -> Result<V2EventRow, LedgerError> {
    Ok(diesel::sql_query(
        "SELECT account_id,seq,command_id,previous_hash,event_hash,kind,payload
        FROM paper_book_v2_event WHERE account_id=?",
    )
    .bind::<Text, _>(account_id)
    .get_result(conn)?)
}

fn v2_head(conn: &mut SqliteConnection, account_id: &str) -> Result<V2HeadRow, LedgerError> {
    Ok(diesel::sql_query(
        "SELECT account_id,version,event_hash,projection_bytes,projection_hash
        FROM paper_book_v2_head WHERE account_id=?",
    )
    .bind::<Text, _>(account_id)
    .get_result(conn)?)
}

fn expected_manifest(
    old: &V1AccountRow,
    account: &V2AccountRow,
    event: &V2EventRow,
    snapshot: &VerifiedV1Snapshot,
    fee: &FeeRow,
) -> CutoverManifestV1 {
    CutoverManifestV1 {
        schema: MANIFEST_SCHEMA.into(),
        account_id: old.account_id.clone(),
        epoch_id: account.epoch_id.clone(),
        cutover_id: account.cutover_id.clone(),
        command_id: event.command_id.clone(),
        v1_epoch_id: old.epoch_id.clone(),
        v1_manifest_hash: old.manifest_hash.clone(),
        v1_head_version: snapshot.version,
        v1_head_hash: snapshot.event_hash.clone(),
        v1_projection_hash: snapshot.projection_hash.clone(),
        v1_equity: snapshot.equity,
        fee_policy_instance_id: fee.policy_instance_id.clone(),
    }
}

fn verify_genesis_rows(
    old: &V1AccountRow,
    account: &V2AccountRow,
    event: &V2EventRow,
    head: &V2HeadRow,
    snapshot: &VerifiedV1Snapshot,
    fee: &FeeRow,
) -> Result<(), LedgerError> {
    require(
        account.account_id == old.account_id
            && event.account_id == old.account_id
            && head.account_id == old.account_id,
        "genesis account mismatch",
    )?;
    require(
        account.v1_epoch_id == old.epoch_id
            && account.v1_manifest_hash == old.manifest_hash
            && account.v1_head_version == snapshot.version
            && account.v1_head_hash == snapshot.event_hash
            && account.v1_projection_hash == snapshot.projection_hash,
        "V1 source anchor mismatch",
    )?;
    require(
        account.fee_policy_instance_id == fee.policy_instance_id,
        "fee instance mismatch",
    )?;
    let manifest: CutoverManifestV1 = decode_canonical(&account.manifest_bytes)?;
    require(
        manifest == expected_manifest(old, account, event, snapshot, fee),
        "manifest fields mismatch",
    )?;
    require(
        account.manifest_hash == manifest_hash(&account.manifest_bytes),
        "manifest hash mismatch",
    )?;
    let expected_payload = GenesisPayloadV1 {
        schema: GENESIS_SCHEMA.into(),
        manifest_hash: account.manifest_hash.clone(),
        v1_head_hash: snapshot.event_hash.clone(),
        v1_projection_hash: snapshot.projection_hash.clone(),
        fee_policy_instance_id: fee.policy_instance_id.clone(),
        cutover_id: account.cutover_id.clone(),
    };
    let payload: GenesisPayloadV1 = decode_canonical(&event.payload)?;
    require(
        payload == expected_payload
            && event.kind == "Genesis"
            && event.seq == 1
            && event.previous_hash == snapshot.event_hash,
        "genesis event mismatch",
    )?;
    require(
        event.event_hash
            == genesis_hash(
                &old.account_id,
                &event.command_id,
                &event.previous_hash,
                &event.payload,
            )?,
        "genesis hash mismatch",
    )?;
    require(
        head.version == 1
            && head.event_hash == event.event_hash
            && head.projection_bytes.as_slice() == snapshot.projection_bytes.as_bytes()
            && head.projection_hash == snapshot.projection_hash
            && head.projection_hash == projection_hash(&head.projection_bytes),
        "V2 projection mismatch",
    )?;
    Ok(())
}

/// Row-level half of the CatalogV5 verifier. Caller first checks the exact
/// schema/fee/review namespace; this function cannot issue cutover authority.
pub(crate) fn verify_owner_rows_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    conn.transaction(verify_owner_rows_in_transaction_on)
}

fn verify_owner_rows_in_transaction_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    let old_accounts: Vec<V1AccountRow> = diesel::sql_query(
        "SELECT account_id,epoch_id,manifest_hash FROM paper_ledger_account ORDER BY account_id",
    )
    .load(conn)?;
    let owners: Vec<OwnerRow> = diesel::sql_query(
        "SELECT account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id
         FROM paper_book_owner_v2 ORDER BY account_id",
    ).load(conn)?;
    let accounts: Vec<V2AccountRow> = diesel::sql_query(
        "SELECT account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,
         v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id
         FROM paper_book_v2_account ORDER BY account_id",
    )
    .load(conn)?;
    let events: Vec<V2EventRow> = diesel::sql_query(
        "SELECT account_id,seq,command_id,previous_hash,event_hash,kind,payload
         FROM paper_book_v2_event ORDER BY account_id,seq",
    )
    .load(conn)?;
    let heads: Vec<V2HeadRow> = diesel::sql_query(
        "SELECT account_id,version,event_hash,projection_bytes,projection_hash
         FROM paper_book_v2_head ORDER BY account_id",
    )
    .load(conn)?;
    let fee = fee_row(conn)?;
    require(!fee.descriptor_bytes.is_empty(), "missing fee descriptor")?;
    let old_epochs: BTreeSet<String> = old_accounts
        .iter()
        .map(|row| row.epoch_id.clone())
        .collect();
    require(old_epochs.len() == old_accounts.len(), "duplicate V1 epoch")?;
    require(owners.len() == old_accounts.len(), "owner backfill gap")?;
    let owner_count = owners.len();
    let account_count = accounts.len();
    let event_count = events.len();
    let head_count = heads.len();
    let mut owners: BTreeMap<String, OwnerRow> = owners
        .into_iter()
        .map(|row| (row.account_id.clone(), row))
        .collect();
    let mut accounts: BTreeMap<String, V2AccountRow> = accounts
        .into_iter()
        .map(|row| (row.account_id.clone(), row))
        .collect();
    let mut events: BTreeMap<String, V2EventRow> = events
        .into_iter()
        .map(|row| (row.account_id.clone(), row))
        .collect();
    let mut heads: BTreeMap<String, V2HeadRow> = heads
        .into_iter()
        .map(|row| (row.account_id.clone(), row))
        .collect();
    require(
        owners.len() == owner_count
            && accounts.len() == account_count
            && events.len() == event_count
            && heads.len() == head_count,
        "duplicate owner or V2 account/event/head row",
    )?;
    let mut audit_guard = V1AuditReplayGuard::default();
    for old in old_accounts {
        let owner = owners
            .remove(&old.account_id)
            .ok_or_else(|| invalid("missing owner"))?;
        match owner.active_generation {
            1 => {
                require(
                    owner.owner_revision == 1
                        && owner.cutover_id.is_none()
                        && owner.active_epoch_id == old.epoch_id
                        && owner.active_manifest_hash == old.manifest_hash
                        && !accounts.contains_key(&old.account_id)
                        && !events.contains_key(&old.account_id)
                        && !heads.contains_key(&old.account_id),
                    "V1Active owner mismatch",
                )?;
            }
            2 => {
                let account = accounts
                    .remove(&old.account_id)
                    .ok_or_else(|| invalid("missing V2 account"))?;
                let event = events
                    .remove(&old.account_id)
                    .ok_or_else(|| invalid("missing V2 genesis"))?;
                let head = heads
                    .remove(&old.account_id)
                    .ok_or_else(|| invalid("missing V2 head"))?;
                require(
                    owner.owner_revision == 2
                        && owner.cutover_id.as_deref() == Some(account.cutover_id.as_str())
                        && owner.active_epoch_id == account.epoch_id
                        && owner.active_manifest_hash == account.manifest_hash
                        && !old_epochs.contains(&account.epoch_id),
                    "V2Active owner or epoch mismatch",
                )?;
                let binding = AccountBinding {
                    account_id: old.account_id.clone(),
                    epoch_id: old.epoch_id.clone(),
                    manifest_hash: old.manifest_hash.clone(),
                };
                let snapshot =
                    verified_v1_snapshot_with_audit_guard_on(conn, &binding, &mut audit_guard)?;
                verify_genesis_rows(&old, &account, &event, &head, &snapshot, &fee)?;
            }
            _ => return Err(invalid("unknown active generation")),
        }
    }
    require(
        owners.is_empty() && accounts.is_empty() && events.is_empty() && heads.is_empty(),
        "orphan owner or V2 rows",
    )
}

/// Read-only V2 genesis view. It carries no fill, order, or write authority.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct VerifiedV2GenesisView {
    pub(crate) account_id: String,
    pub(crate) epoch_id: String,
    pub(crate) manifest_hash: String,
    pub(crate) v1_epoch_id: String,
    pub(crate) v1_head_version: i64,
    pub(crate) v1_head_hash: String,
    pub(crate) version: i64,
    pub(crate) event_hash: String,
    pub(crate) projection_bytes: Vec<u8>,
    pub(crate) projection_hash: String,
    pub(crate) cutover_id: String,
    pub(crate) fee_policy_instance_id: String,
}

/// Verify the entire CatalogV5 and every owner inside the same read transaction
/// before exposing one V2Active genesis. Prepared V1Active accounts are rejected.
pub(crate) fn read_v2_on(
    db: &DatabaseManager,
    account_id: &str,
) -> Result<VerifiedV2GenesisView, LedgerError> {
    let mut conn = db
        .get_conn()
        .map_err(|error| LedgerError::Database(error.to_string()))?;
    conn.transaction(|conn| {
        crate::database::paper_book_owner_schema_v2::verify_catalog_v5_on(conn)
            .map_err(|error| invalid(&error.to_string()))?;
        read_verified_genesis_body_on(conn, account_id)
    })
}

#[cfg(test)]
use crate::performance::fee_policy::AShareFeePolicyV2;

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct TestCutoverRequest {
    pub(crate) old_binding: AccountBinding,
    pub(crate) new_epoch_id: String,
    pub(crate) cutover_id: String,
    pub(crate) command_id: String,
    pub(crate) expected_v1_version: i64,
    pub(crate) expected_v1_head_hash: String,
    pub(crate) expected_v1_projection_hash: String,
    pub(crate) reviewed_fee_policy: AShareFeePolicyV2,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TestCutoverFault {
    None,
    AfterGenesisWrites,
    AfterOwnerCas,
    DeferredForeignKeyOnCommit,
    AfterCommitOutcomeUnknown,
}

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TestCutoverReceipt {
    pub(crate) account_id: String,
    pub(crate) epoch_id: String,
    pub(crate) cutover_id: String,
    pub(crate) manifest_hash: String,
    pub(crate) event_hash: String,
    pub(crate) already_applied: bool,
}

#[cfg(test)]
fn validate_request(request: &TestCutoverRequest) -> Result<(), LedgerError> {
    for value in [
        &request.new_epoch_id,
        &request.cutover_id,
        &request.command_id,
    ] {
        if value.trim().is_empty() || value.len() > 128 {
            return Err(LedgerError::InvalidInput(
                "invalid V2 cutover identity".into(),
            ));
        }
    }
    if request.new_epoch_id == request.old_binding.epoch_id
        || request.expected_v1_version < 1
        || request.expected_v1_head_hash.len() != 64
        || request.expected_v1_projection_hash.len() != 64
    {
        return Err(LedgerError::InvalidInput(
            "invalid V2 cutover anchor".into(),
        ));
    }
    Ok(())
}

/// The only V2 owner writer. It is unavailable to production builds and
/// requires an isolated TEST_CODE connection. It never creates an order/fill.
/// SQLite DDL privilege is not an authorization boundary: a DDL-capable actor
/// can remove triggers, so production cutover still needs an external owner.
#[cfg(test)]
pub(crate) fn cutover_for_isolated_test(
    db: &DatabaseManager,
    request: &TestCutoverRequest,
    fault: TestCutoverFault,
) -> Result<TestCutoverReceipt, LedgerError> {
    let mut conn = db
        .get_conn()
        .map_err(|error| LedgerError::Database(error.to_string()))?;
    let mut ready_to_commit = false;
    let result = conn.immediate_transaction(|conn| {
        crate::database::paper_book_owner_schema_v2::require_isolated_for_test(conn)
            .map_err(|error| invalid(&error.to_string()))?;
        crate::database::paper_book_owner_schema_v2::verify_catalog_v5_on(conn)
            .map_err(|error| invalid(&error.to_string()))?;
        validate_request(request)?;
        let snapshot = verified_v1_snapshot_on(conn, &request.old_binding)?;
        if snapshot.version != request.expected_v1_version
            || snapshot.event_hash != request.expected_v1_head_hash
            || snapshot.projection_hash != request.expected_v1_projection_hash
        {
            return Err(LedgerError::VersionChanged);
        }
        let fee = fee_row(conn)?;
        if fee.policy_instance_id != request.reviewed_fee_policy.instance_id()
            || fee.descriptor_bytes != request.reviewed_fee_policy.canonical_bytes()
        {
            return Err(invalid("reviewed fee descriptor bytes mismatch"));
        }
        let owner: OwnerRow = diesel::sql_query(
            "SELECT account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id
             FROM paper_book_owner_v2 WHERE account_id=?",
        ).bind::<Text,_>(&request.old_binding.account_id).get_result(conn)?;
        let old = V1AccountRow {
            account_id: request.old_binding.account_id.clone(),
            epoch_id: request.old_binding.epoch_id.clone(),
            manifest_hash: request.old_binding.manifest_hash.clone(),
        };
        let manifest = CutoverManifestV1 {
            schema: MANIFEST_SCHEMA.into(),
            account_id: old.account_id.clone(),
            epoch_id: request.new_epoch_id.clone(),
            cutover_id: request.cutover_id.clone(),
            command_id: request.command_id.clone(),
            v1_epoch_id: old.epoch_id.clone(),
            v1_manifest_hash: old.manifest_hash.clone(),
            v1_head_version: snapshot.version,
            v1_head_hash: snapshot.event_hash.clone(),
            v1_projection_hash: snapshot.projection_hash.clone(),
            v1_equity: snapshot.equity,
            fee_policy_instance_id: fee.policy_instance_id.clone(),
        };
        let manifest_bytes = canonical(&manifest)?;
        let manifest_hash = manifest_hash(&manifest_bytes);
        if owner.active_generation == 2 {
            let account = v2_account(conn, &old.account_id)?;
            let event = v2_event(conn, &old.account_id)?;
            if account.manifest_bytes != manifest_bytes
                || account.manifest_hash != manifest_hash
                || event.command_id != request.command_id
                || owner.cutover_id.as_deref() != Some(request.cutover_id.as_str())
            {
                return Err(LedgerError::IdentityConflict);
            }
            return Ok(TestCutoverReceipt {
                account_id: old.account_id,
                epoch_id: account.epoch_id,
                cutover_id: account.cutover_id,
                manifest_hash,
                event_hash: event.event_hash,
                already_applied: true,
            });
        }
        if owner.active_generation != 1 || owner.owner_revision != 1
            || owner.active_epoch_id != old.epoch_id
            || owner.active_manifest_hash != old.manifest_hash
            || owner.cutover_id.is_some()
        {
            return Err(LedgerError::InactiveEpoch);
        }
        #[derive(QueryableByName)]
        struct Count { #[diesel(sql_type = BigInt)] value: i64 }
        let old_epoch_count = diesel::sql_query("SELECT COUNT(*) AS value FROM paper_ledger_account WHERE epoch_id=?")
            .bind::<Text,_>(&request.new_epoch_id).get_result::<Count>(conn)?.value;
        if old_epoch_count != 0 {
            return Err(LedgerError::IdentityConflict);
        }
        diesel::sql_query("INSERT INTO paper_book_v2_account
            (account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,
             v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id)
             VALUES (?,?,?,?,?,?,?,?,?,?,?)")
            .bind::<Text,_>(&old.account_id)
            .bind::<Text,_>(&request.new_epoch_id)
            .bind::<Text,_>(&manifest_hash)
            .bind::<Binary,_>(&manifest_bytes)
            .bind::<Text,_>(&fee.policy_instance_id)
            .bind::<Text,_>(&old.epoch_id)
            .bind::<Text,_>(&old.manifest_hash)
            .bind::<BigInt,_>(snapshot.version)
            .bind::<Text,_>(&snapshot.event_hash)
            .bind::<Text,_>(&snapshot.projection_hash)
            .bind::<Text,_>(&request.cutover_id)
            .execute(conn)?;
        let payload = canonical(&GenesisPayloadV1 {
            schema: GENESIS_SCHEMA.into(),
            manifest_hash: manifest_hash.clone(),
            v1_head_hash: snapshot.event_hash.clone(),
            v1_projection_hash: snapshot.projection_hash.clone(),
            fee_policy_instance_id: fee.policy_instance_id.clone(),
            cutover_id: request.cutover_id.clone(),
        })?;
        let event_hash = genesis_hash(&old.account_id, &request.command_id,
            &snapshot.event_hash, &payload)?;
        diesel::sql_query("INSERT INTO paper_book_v2_event
            (account_id,seq,command_id,previous_hash,event_hash,kind,payload)
            VALUES (?,1,?,?,?,'Genesis',?)")
            .bind::<Text,_>(&old.account_id)
            .bind::<Text,_>(&request.command_id)
            .bind::<Text,_>(&snapshot.event_hash)
            .bind::<Text,_>(&event_hash)
            .bind::<Binary,_>(&payload)
            .execute(conn)?;
        diesel::sql_query("INSERT INTO paper_book_v2_head
            (account_id,version,event_hash,projection_bytes,projection_hash) VALUES (?,1,?,?,?)")
            .bind::<Text,_>(&old.account_id)
            .bind::<Text,_>(&event_hash)
            .bind::<Binary,_>(snapshot.projection_bytes.as_bytes())
            .bind::<Text,_>(&snapshot.projection_hash)
            .execute(conn)?;
        let account = v2_account(conn, &old.account_id)?;
        let event = v2_event(conn, &old.account_id)?;
        let head = v2_head(conn, &old.account_id)?;
        verify_genesis_rows(&old, &account, &event, &head, &snapshot, &fee)?;
        if fault == TestCutoverFault::AfterGenesisWrites {
            return Err(LedgerError::Database("TEST_CODE fault after V2 genesis writes".into()));
        }
        diesel::sql_query("DROP TRIGGER paper_book_owner_v2_transition").execute(conn)?;
        let changed = diesel::sql_query("UPDATE paper_book_owner_v2 SET
            active_generation=2,active_epoch_id=?,active_manifest_hash=?,owner_revision=2,cutover_id=?
            WHERE account_id=? AND active_generation=1 AND active_epoch_id=?
              AND active_manifest_hash=? AND owner_revision=1 AND cutover_id IS NULL")
            .bind::<Text,_>(&request.new_epoch_id)
            .bind::<Text,_>(&manifest_hash)
            .bind::<Text,_>(&request.cutover_id)
            .bind::<Text,_>(&old.account_id)
            .bind::<Text,_>(&old.epoch_id)
            .bind::<Text,_>(&old.manifest_hash)
            .execute(conn)?;
        if changed != 1 { return Err(LedgerError::VersionChanged); }
        if fault == TestCutoverFault::AfterOwnerCas {
            return Err(LedgerError::Database("TEST_CODE fault after owner CAS".into()));
        }
        diesel::sql_query(crate::database::paper_book_owner_schema_v2::OWNER_TRANSITION_GUARD_DDL)
            .execute(conn)?;
        crate::database::paper_book_owner_schema_v2::verify_catalog_v5_on(conn)
            .map_err(|error| invalid(&error.to_string()))?;
        if fault == TestCutoverFault::DeferredForeignKeyOnCommit {
            conn.batch_execute(
                "CREATE TEMP TABLE TEST_CODE_commit_parent (id INTEGER PRIMARY KEY);
                 CREATE TEMP TABLE TEST_CODE_commit_child (
                     parent_id INTEGER REFERENCES TEST_CODE_commit_parent(id)
                     DEFERRABLE INITIALLY DEFERRED
                 );
                 INSERT INTO TEST_CODE_commit_child (parent_id) VALUES (1);",
            )?;
        }
        ready_to_commit = true;
        Ok(TestCutoverReceipt {
            account_id: old.account_id,
            epoch_id: request.new_epoch_id.clone(),
            cutover_id: request.cutover_id.clone(),
            manifest_hash,
            event_hash,
            already_applied: false,
        })
    });
    match result {
        Err(_) if ready_to_commit => Err(LedgerError::CommitOutcomeUnknown),
        Ok(_) if fault == TestCutoverFault::AfterCommitOutcomeUnknown => {
            Err(LedgerError::CommitOutcomeUnknown)
        }
        result => result,
    }
}

/// Read-only body after full catalog and all original owner/genesis rows have
/// been verified on this same transaction. It grants no execution authority.
pub(crate) fn read_verified_genesis_body_on(
    conn: &mut SqliteConnection,
    account_id: &str,
) -> Result<VerifiedV2GenesisView, LedgerError> {
    let owner = diesel::sql_query(
        "SELECT account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id
         FROM paper_book_owner_v2 WHERE account_id=?",
    )
    .bind::<Text, _>(account_id)
    .get_result::<OwnerRow>(conn)
    .optional()?
    .ok_or(LedgerError::InactiveEpoch)?;
    if owner.active_generation != 2 {
        return Err(LedgerError::InactiveEpoch);
    }
    let account = v2_account(conn, account_id)?;
    let head = v2_head(conn, account_id)?;
    Ok(VerifiedV2GenesisView {
        account_id: account.account_id,
        epoch_id: account.epoch_id,
        manifest_hash: account.manifest_hash,
        v1_epoch_id: account.v1_epoch_id,
        v1_head_version: account.v1_head_version,
        v1_head_hash: account.v1_head_hash,
        version: head.version,
        event_hash: head.event_hash,
        projection_bytes: head.projection_bytes,
        projection_hash: head.projection_hash,
        cutover_id: account.cutover_id,
        fee_policy_instance_id: account.fee_policy_instance_id,
    })
}

// Finite replay DTO seeds stay with the owners of private fields.
#[allow(dead_code, non_camel_case_types)]
mod replay_codec_owner {
    use super::*;
    use crate::trading::paper_replay_codec_v1 as c;
    use crate::trading::paper_replay_shapes_v1 as s;
    use serde::de::{EnumAccess as _, VariantAccess as _};
    impl c::sealed::Value for CutoverManifestV1 {}
    impl c::Value for CutoverManifestV1 {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "schema",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "account_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "epoch_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cutover_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "command_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_epoch_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_manifest_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_head_version",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_head_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_projection_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_equity",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_policy_instance_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            false,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,CutoverManifestV1,true,{schema:String=>false,account_id:String=>false,epoch_id:String=>false,cutover_id:String=>false,command_id:String=>false,v1_epoch_id:String=>false,v1_manifest_hash:String=>false,v1_head_version:i64=>false,v1_head_hash:String=>false,v1_projection_hash:String=>false,v1_equity:Money=>false,fee_policy_instance_id:String=>false},CutoverManifestV1{schema,account_id,epoch_id,cutover_id,command_id,v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,v1_equity,fee_policy_instance_id})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(CutoverManifestV1 {
                schema: c::Value::paid_copy(&self.schema, w)?,
                account_id: c::Value::paid_copy(&self.account_id, w)?,
                epoch_id: c::Value::paid_copy(&self.epoch_id, w)?,
                cutover_id: c::Value::paid_copy(&self.cutover_id, w)?,
                command_id: c::Value::paid_copy(&self.command_id, w)?,
                v1_epoch_id: c::Value::paid_copy(&self.v1_epoch_id, w)?,
                v1_manifest_hash: c::Value::paid_copy(&self.v1_manifest_hash, w)?,
                v1_head_version: c::Value::paid_copy(&self.v1_head_version, w)?,
                v1_head_hash: c::Value::paid_copy(&self.v1_head_hash, w)?,
                v1_projection_hash: c::Value::paid_copy(&self.v1_projection_hash, w)?,
                v1_equity: c::Value::paid_copy(&self.v1_equity, w)?,
                fee_policy_instance_id: c::Value::paid_copy(&self.fee_policy_instance_id, w)?,
            })
        }
    }
    impl c::sealed::Value for GenesisPayloadV1 {}
    impl c::Value for GenesisPayloadV1 {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "schema",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "manifest_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_head_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "v1_projection_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_policy_instance_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cutover_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            false,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,GenesisPayloadV1,true,{schema:String=>false,manifest_hash:String=>false,v1_head_hash:String=>false,v1_projection_hash:String=>false,fee_policy_instance_id:String=>false,cutover_id:String=>false},GenesisPayloadV1{schema,manifest_hash,v1_head_hash,v1_projection_hash,fee_policy_instance_id,cutover_id})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(GenesisPayloadV1 {
                schema: c::Value::paid_copy(&self.schema, w)?,
                manifest_hash: c::Value::paid_copy(&self.manifest_hash, w)?,
                v1_head_hash: c::Value::paid_copy(&self.v1_head_hash, w)?,
                v1_projection_hash: c::Value::paid_copy(&self.v1_projection_hash, w)?,
                fee_policy_instance_id: c::Value::paid_copy(&self.fee_policy_instance_id, w)?,
                cutover_id: c::Value::paid_copy(&self.cutover_id, w)?,
            })
        }
    }
    impl c::sealed::Root for CutoverManifestV1 {}
    impl c::Root for CutoverManifestV1 {
        const ROOT: s::RootKind = s::RootKind::Cutover;
        const CANONICAL: bool = true;
    }
    impl c::sealed::Root for GenesisPayloadV1 {}
    impl c::Root for GenesisPayloadV1 {
        const ROOT: s::RootKind = s::RootKind::Genesis;
        const CANONICAL: bool = true;
    }
}
#[cfg(test)]
pub(crate) fn replay_codec_fixtures(
    case: crate::trading::paper_replay_codec_v1::CodecFixtureCase,
    work: &mut crate::database::global_schema_v1::replay_work::CodecMechanics<'_, '_>,
) {
    crate::trading::paper_replay_codec_v1::exercise_root::<CutoverManifestV1>(case, work);
    crate::trading::paper_replay_codec_v1::exercise_root::<GenesisPayloadV1>(case, work);
}
