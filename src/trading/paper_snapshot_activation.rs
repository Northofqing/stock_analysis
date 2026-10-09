//! Explicit user-snapshot paper activation for the ordinary monitor database.
//! Only source facts and the new paper namespace are inspected; the singleton,
//! mutable actual-position projection, and GlobalSchema catalogs are untouched.

use super::paper_ledger::{
    self, AccountBinding, LedgerError, Mark, Money, PaperReceipt, Projection, SeedManifest,
    ValuationBatch,
};
use crate::database::paper_snapshot_activation_schema_v1 as schema;
use chrono::{DateTime, FixedOffset, NaiveDate, Timelike, Utc};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Text};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_MARK_SOURCE: &str =
    "user_reported_snapshot_observation_not_live_execution_quotes";
pub const OPENING_CLOSE_SOURCE: &str = "user_snapshot_opening_end_of_day_baseline";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSourceEvidence {
    pub image_path: PathBuf,
    pub image_sha256: String,
    pub snapshot_evidence_path: PathBuf,
    pub snapshot_evidence_sha256: String,
    pub import_receipt_path: PathBuf,
    pub import_receipt_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPaperActivationRequest {
    pub schema_version: u32,
    pub summary_row_id: i64,
    pub snapshot_id: String,
    pub source_batch_reference: String,
    pub source_evidence: SnapshotSourceEvidence,
    /// Explicit approval of a new paper closing baseline from this same snapshot.
    pub establish_after_hours_close: bool,
    /// Preview may prepare an empty source_hash; apply requires its exact hash.
    pub seed: SeedManifest,
}
#[derive(Clone, Debug, Serialize)]
pub struct SnapshotPaperActivationOutcome {
    pub database_identity: ActivationTargetDatabase,
    pub mode: &'static str,
    pub applied: bool,
    pub already_applied: bool,
    pub prepared_request: SnapshotPaperActivationRequest,
    pub binding: AccountBinding,
    pub source_bundle_hash: String,
    pub opening_close_date: Option<NaiveDate>,
    pub projection: Projection,
    pub seed_receipt: Option<PaperReceipt>,
    pub closing_receipt: Option<PaperReceipt>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ActivationTargetDatabase {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, QueryableByName)]
pub(crate) struct SummaryFact {
    #[diesel(sql_type=BigInt)]
    id: i64,
    #[diesel(sql_type=Text)]
    effective_at: String,
    #[diesel(sql_type=Double)]
    total_assets: f64,
    #[diesel(sql_type=Double)]
    securities_market_value: f64,
    #[diesel(sql_type=Double)]
    available_cash: f64,
    #[diesel(sql_type=Double)]
    position_ratio_pct: f64,
    #[diesel(sql_type=Double)]
    daily_pnl: f64,
    #[diesel(sql_type=Text)]
    source: String,
    #[diesel(sql_type=Text)]
    recorded_at: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, QueryableByName)]
pub(crate) struct SnapshotFact {
    #[diesel(sql_type=BigInt)]
    id: i64,
    #[diesel(sql_type=Text)]
    snapshot_id: String,
    #[diesel(sql_type=Text)]
    effective_at: String,
    #[diesel(sql_type=Text)]
    confirmed_at: String,
    #[diesel(sql_type=Text)]
    source: String,
    #[diesel(sql_type=BigInt)]
    confirm_empty: i64,
    #[diesel(sql_type=Text)]
    evidence_sha256: String,
    #[diesel(sql_type=BigInt)]
    item_count: i64,
    #[diesel(sql_type=Text)]
    recorded_at: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, QueryableByName)]
pub(crate) struct PositionFact {
    #[diesel(sql_type=Text)]
    code: String,
    #[diesel(sql_type=Text)]
    name: String,
    #[diesel(sql_type=BigInt)]
    quantity: i64,
    #[diesel(sql_type=Double)]
    cost_price: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceBundle {
    schema_version: u32,
    source_batch_reference: String,
    source_evidence: SnapshotSourceEvidence,
    summary: SummaryFact,
    snapshot: SnapshotFact,
    items: Vec<PositionFact>,
    snapshot_evidence_json: String,
    import_receipt_json: String,
    opening_marks: Vec<Mark>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivationProof {
    schema_version: u32,
    pub(crate) request: SnapshotPaperActivationRequest,
    source_bundle: SourceBundle,
    activated_at: DateTime<Utc>,
}
#[derive(Deserialize)]
struct ImagePosition {
    code: String,
    name: String,
    quantity: i64,
    cost_price: String,
    current_price: String,
    market_value: String,
}
#[derive(Deserialize)]
struct ImageEvidence {
    original_image_sha256: String,
    effective_at: DateTime<FixedOffset>,
    confirmed_at: DateTime<FixedOffset>,
    market_prices_role: String,
    snapshot_id: String,
    position_evidence_sha256: String,
    positions: Vec<ImagePosition>,
}
#[derive(Deserialize)]
struct ImportReadback {
    summary: SummaryFact,
    positions: SnapshotFact,
    items: Vec<PositionFact>,
}
#[derive(Deserialize)]
struct FileIdentity {
    device: u64,
    inode: u64,
}
#[derive(Deserialize)]
struct ImportReceipt {
    status: String,
    database: PathBuf,
    database_identity: FileIdentity,
    effective_at: DateTime<FixedOffset>,
    confirmed_at: DateTime<FixedOffset>,
    position_row_id: i64,
    account_summary_row_id: i64,
    snapshot_id: String,
    image_sha256: String,
    readback: ImportReadback,
}
fn invalid(message: &str) -> LedgerError {
    LedgerError::InvalidInput(message.into())
}
fn unavailable(message: &str) -> LedgerError {
    LedgerError::EvidenceUnavailable(message.into())
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, LedgerError> {
    serde_json::to_vec(value).map_err(|e| invalid(&e.to_string()))
}
fn parse<T: for<'a> Deserialize<'a>>(bytes: &str) -> Result<T, LedgerError> {
    serde_json::from_str(bytes)
        .map_err(|e| invalid(&format!("snapshot source evidence contract: {e}")))
}
fn time(value: &str) -> Result<DateTime<Utc>, LedgerError> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| invalid("invalid snapshot source timestamp"))
}
fn sha_valid(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn text_money(value: &str) -> Result<Money, LedgerError> {
    Money::from_cny(
        value
            .parse::<f64>()
            .map_err(|_| invalid("invalid snapshot amount"))?,
    )
}
fn shanghai_day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
        .date_naive()
}
fn opening_close_date(
    request: &SnapshotPaperActivationRequest,
) -> Result<Option<NaiveDate>, LedgerError> {
    if !request.establish_after_hours_close {
        return Ok(None);
    }
    let at = request
        .seed
        .cutover_at
        .with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap());
    if at.hour() < 15
        || !crate::calendar::verified_a_share_trading_day(at.date_naive())
            .map_err(unavailable_string)?
    {
        return Err(invalid("opening paper closing baseline requires this snapshot after a completed verified trading day"));
    }
    Ok(Some(at.date_naive()))
}
fn unavailable_string(message: String) -> LedgerError {
    LedgerError::EvidenceUnavailable(message)
}
fn bundle_hash(bundle: &SourceBundle) -> Result<String, LedgerError> {
    let mut bytes = b"stock_analysis.snapshot_paper_activation.source.v1\0".to_vec();
    bytes.extend(encode(bundle)?);
    Ok(schema::hash_bytes(&bytes))
}

fn validate_bundle(
    request: &SnapshotPaperActivationRequest,
    bundle: &SourceBundle,
) -> Result<(), LedgerError> {
    let seed = &request.seed;
    if request.schema_version != 1
        || bundle.schema_version != 1
        || request.summary_row_id <= 0
        || request.summary_row_id != bundle.summary.id
        || request.snapshot_id != bundle.snapshot.snapshot_id
        || request.source_batch_reference.trim().is_empty()
        || request.source_batch_reference != bundle.source_batch_reference
        || seed.source_reference != request.source_batch_reference
        || request.source_evidence != bundle.source_evidence
        || seed.marks != bundle.opening_marks
        || seed.excluded_residual.is_some_and(|v| v != Money::ZERO)
        || bundle.summary.source.trim().is_empty()
        || bundle.snapshot.source != "user_confirmed_full_snapshot"
        || time(&bundle.summary.effective_at)? != seed.cutover_at
        || time(&bundle.snapshot.effective_at)? != seed.cutover_at
        || seed.account_effective_at != seed.cutover_at
        || seed.positions_effective_at != seed.cutover_at
        || seed.cash != Money::from_cny(bundle.summary.available_cash)?
        || seed.original_total != Money::from_cny(bundle.summary.total_assets)?
        || !bundle.summary.daily_pnl.is_finite()
        || !bundle.summary.position_ratio_pct.is_finite()
        || !(0.0..=100.0).contains(&bundle.summary.position_ratio_pct)
        || bundle.snapshot.item_count != bundle.items.len() as i64
        || (bundle.snapshot.confirm_empty == 1) != bundle.items.is_empty()
        || !matches!(bundle.snapshot.confirm_empty, 0 | 1)
        || time(&bundle.snapshot.confirmed_at)? < seed.cutover_at
    {
        return Err(invalid(
            "paper seed does not match the exact imported same-batch account/position facts",
        ));
    }
    let source = &request.source_evidence;
    if ![
        &source.image_sha256,
        &source.snapshot_evidence_sha256,
        &source.import_receipt_sha256,
    ]
    .iter()
    .all(|v| sha_valid(v))
        || schema::hash_bytes(bundle.snapshot_evidence_json.as_bytes())
            != source.snapshot_evidence_sha256
        || schema::hash_bytes(bundle.import_receipt_json.as_bytes()) != source.import_receipt_sha256
    {
        return Err(invalid("snapshot artifact hash mismatch"));
    }
    let mut evidence: ImageEvidence = parse(&bundle.snapshot_evidence_json)?;
    let mut receipt: ImportReceipt = parse(&bundle.import_receipt_json)?;
    receipt.readback.items.sort_by(|a, b| a.code.cmp(&b.code));
    evidence.positions.sort_by(|a, b| a.code.cmp(&b.code));
    if evidence.original_image_sha256 != source.image_sha256
        || evidence.snapshot_id != request.snapshot_id
        || evidence.position_evidence_sha256 != bundle.snapshot.evidence_sha256
        || evidence.effective_at.with_timezone(&Utc) != seed.cutover_at
        || evidence.confirmed_at.with_timezone(&Utc) != time(&bundle.snapshot.confirmed_at)?
        || evidence.market_prices_role != SNAPSHOT_MARK_SOURCE
        || receipt.status != "committed_and_verified"
        || receipt.image_sha256 != source.image_sha256
        || !receipt.database.is_absolute()
        || receipt.database_identity.device == 0
        || receipt.database_identity.inode == 0
        || receipt.account_summary_row_id != request.summary_row_id
        || receipt.position_row_id != bundle.snapshot.id
        || receipt.snapshot_id != request.snapshot_id
        || receipt.effective_at.with_timezone(&Utc) != seed.cutover_at
        || receipt.confirmed_at.with_timezone(&Utc) != time(&bundle.snapshot.confirmed_at)?
        || receipt.readback.summary != bundle.summary
        || receipt.readback.positions != bundle.snapshot
        || receipt.readback.items != bundle.items
        || evidence.positions.len() != bundle.items.len()
        || seed.lots.len() != bundle.items.len()
        || seed.marks.len() != bundle.items.len()
        || bundle.items.windows(2).any(|w| w[0].code >= w[1].code)
    {
        return Err(invalid(
            "snapshot image/import receipt does not prove this exact source batch",
        ));
    }
    let mut lots = seed.lots.iter().collect::<Vec<_>>();
    lots.sort_by(|a, b| a.code.cmp(&b.code));
    let mut marks = seed.marks.iter().collect::<Vec<_>>();
    marks.sort_by(|a, b| a.code.cmp(&b.code));
    let mut market_micros = 0_i64;
    for (((item, image), lot), mark) in bundle
        .items
        .iter()
        .zip(&evidence.positions)
        .zip(lots)
        .zip(marks)
    {
        let quantity = u32::try_from(item.quantity)
            .map_err(|_| invalid("snapshot quantity outside paper range"))?;
        if item.code != image.code
            || item.name != image.name
            || item.quantity != image.quantity
            || item.code != lot.code
            || item.name != lot.name
            || lot.quantity != quantity
            || item.code != mark.code
            || mark.observed_at != seed.cutover_at
            || mark.source != SNAPSHOT_MARK_SOURCE
            || mark.price != text_money(&image.current_price)?
            || mark.price <= Money::ZERO
            || lot.reported_cost != Some(Money::from_cny(item.cost_price)?)
            || item.cost_price <= 0.0
            || Money::from_cny(item.cost_price)? != text_money(&image.cost_price)?
            || lot.sellable_from.is_some()
            || lot.sellability_evidence.is_some()
        {
            return Err(invalid("snapshot opening lots/marks differ from same-batch evidence or imply unsupported sellability"));
        }
        let value = mark
            .price
            .micros()
            .checked_mul(i64::from(quantity))
            .ok_or(LedgerError::Overflow)?;
        if value != text_money(&image.market_value)?.micros() {
            return Err(invalid(
                "snapshot per-security market value does not balance",
            ));
        }
        market_micros = market_micros
            .checked_add(value)
            .ok_or(LedgerError::Overflow)?;
    }
    if Money::from_micros(market_micros) != Money::from_cny(bundle.summary.securities_market_value)?
        || seed
            .cash
            .micros()
            .checked_add(market_micros)
            .ok_or(LedgerError::Overflow)?
            != seed.original_total.micros()
    {
        return Err(invalid(
            "snapshot cash plus security marks does not balance to imported account total",
        ));
    }
    // Recompute the existing complete-snapshot canonical evidence independently.
    let input_json = serde_json::json!({"schema_version":1,"effective_at":bundle.snapshot.effective_at,
        "confirm_empty":bundle.items.is_empty(),"items":bundle.items.iter().map(|item| serde_json::json!({
            "code":item.code,"name":item.name,"quantity":item.quantity,"cost_price":item.cost_price})).collect::<Vec<_>>()});
    let canonical =
        crate::portfolio::user_position_snapshot::user_position_snapshot_input_from_json(
            &input_json.to_string(),
            DateTime::parse_from_rfc3339(&bundle.snapshot.confirmed_at)
                .map_err(|_| invalid("snapshot confirmation timestamp"))?,
        )
        .map_err(unavailable_string)?;
    if canonical.snapshot_id != bundle.snapshot.snapshot_id
        || canonical.evidence_sha256 != bundle.snapshot.evidence_sha256
    {
        return Err(invalid("imported complete-snapshot evidence hash mismatch"));
    }
    opening_close_date(request)?;
    paper_ledger::seed_projection(seed)?;
    Ok(())
}
pub(crate) fn validate_stored_proof(
    proof: &ActivationProof,
) -> Result<AccountBinding, LedgerError> {
    if proof.schema_version != 1
        || proof.activated_at < proof.request.seed.cutover_at
        || proof.activated_at < time(&proof.source_bundle.snapshot.confirmed_at)?
        || shanghai_day(proof.activated_at) != shanghai_day(proof.request.seed.cutover_at)
        || proof.request.seed.source_hash != bundle_hash(&proof.source_bundle)?
    {
        return Err(invalid("snapshot activation stored source proof mismatch"));
    }
    validate_bundle(&proof.request, &proof.source_bundle)?;
    proof.request.seed.binding()
}

fn read_hashed_file(path: &Path, expected_hash: &str, limit: u64) -> Result<Vec<u8>, LedgerError> {
    if !sha_valid(expected_hash) {
        return Err(invalid("explicit lowercase source SHA-256 required"));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|_| unavailable("explicit snapshot source file unavailable"))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid(
            "snapshot source file is not regular or exceeds its size limit",
        ));
    }
    let bytes = std::fs::read(path).map_err(|_| unavailable("snapshot source file read failed"))?;
    if bytes.len() as u64 > limit || schema::hash_bytes(&bytes) != expected_hash {
        return Err(invalid("snapshot source file hash mismatch"));
    }
    Ok(bytes)
}
fn read_sources_on(
    conn: &mut SqliteConnection,
    request: &SnapshotPaperActivationRequest,
    now: DateTime<Utc>,
) -> Result<SourceBundle, LedgerError> {
    let summary = diesel::sql_query("SELECT id,effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source,recorded_at
        FROM main.user_account_summary ORDER BY effective_at DESC,id DESC LIMIT 1").get_result::<SummaryFact>(conn).optional()?.ok_or_else(|| unavailable("imported account summary missing"))?;
    let snapshot = diesel::sql_query("SELECT id,snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count,recorded_at
        FROM main.user_position_snapshot ORDER BY effective_at DESC,confirmed_at DESC,snapshot_id DESC LIMIT 1").get_result::<SnapshotFact>(conn).optional()?.ok_or_else(|| unavailable("imported complete position snapshot missing"))?;
    if summary.id != request.summary_row_id || snapshot.snapshot_id != request.snapshot_id {
        return Err(unavailable(
            "activation source rows are no longer the latest approved account/positions",
        ));
    }
    let items = diesel::sql_query("SELECT code,name,quantity,cost_price FROM main.user_position_snapshot_item WHERE snapshot_id=? ORDER BY code")
        .bind::<Text,_>(&request.snapshot_id).load::<PositionFact>(conn)?;
    if request.seed.cutover_at > now
        || time(&snapshot.confirmed_at)? > now
        || shanghai_day(request.seed.cutover_at) != shanghai_day(now)
        || !crate::calendar::verified_a_share_trading_day(shanghai_day(request.seed.cutover_at))
            .map_err(unavailable_string)?
    {
        return Err(unavailable("snapshot activation requires the latest confirmed snapshot from today's verified Shanghai trading day"));
    }
    let source = &request.source_evidence;
    read_hashed_file(&source.image_path, &source.image_sha256, 40 * 1024 * 1024)?;
    let snapshot_evidence_json = String::from_utf8(read_hashed_file(
        &source.snapshot_evidence_path,
        &source.snapshot_evidence_sha256,
        2 * 1024 * 1024,
    )?)
    .map_err(|_| invalid("snapshot evidence is not UTF-8"))?;
    let import_receipt_json = String::from_utf8(read_hashed_file(
        &source.import_receipt_path,
        &source.import_receipt_sha256,
        2 * 1024 * 1024,
    )?)
    .map_err(|_| invalid("snapshot import receipt is not UTF-8"))?;
    Ok(SourceBundle {
        schema_version: 1,
        source_batch_reference: request.source_batch_reference.clone(),
        source_evidence: source.clone(),
        summary,
        snapshot,
        items,
        snapshot_evidence_json,
        import_receipt_json,
        opening_marks: request.seed.marks.clone(),
    })
}

fn prepared_projection(
    request: &SnapshotPaperActivationRequest,
) -> Result<Projection, LedgerError> {
    let mut projection = paper_ledger::seed_projection(&request.seed)?;
    if let Some(date) = opening_close_date(request)? {
        projection.closes.insert(date, projection.seed_equity);
        for mark in projection.marks.values_mut() {
            mark.source = OPENING_CLOSE_SOURCE.into();
        }
    }
    Ok(projection)
}
fn prepare_on(
    conn: &mut SqliteConnection,
    request: &SnapshotPaperActivationRequest,
    now: DateTime<Utc>,
    apply: bool,
) -> Result<(ActivationProof, bool), LedgerError> {
    if apply && !sha_valid(&request.seed.source_hash) {
        return Err(invalid(
            "apply requires the exact prepared source_hash from preview",
        ));
    }
    if schema::is_present_on(conn)? {
        schema::verify_active_on(conn)?;
        let proof = schema::proof_on(conn)?;
        let mut candidate = request.clone();
        if !apply && candidate.seed.source_hash.is_empty() {
            candidate.seed.source_hash = proof.request.seed.source_hash.clone();
        }
        if candidate != proof.request {
            return Err(LedgerError::IdentityConflict);
        }
        paper_ledger::read_on(conn, &proof.request.seed.binding()?)?;
        return Ok((proof, true));
    }
    schema::require_uninitialized_on(conn)?;
    let bundle = read_sources_on(conn, request, now)?;
    let hash = bundle_hash(&bundle)?;
    if !request.seed.source_hash.is_empty() && request.seed.source_hash != hash {
        return Err(invalid("activation source proof changed since preview"));
    }
    let mut prepared = request.clone();
    prepared.seed.source_hash = hash;
    validate_bundle(&prepared, &bundle)?;
    let proof = ActivationProof {
        schema_version: 1,
        request: prepared,
        source_bundle: bundle,
        activated_at: now,
    };
    validate_stored_proof(&proof)?;
    Ok((proof, false))
}
pub fn preview_snapshot_paper_activation(
    conn: &mut SqliteConnection,
    request: &SnapshotPaperActivationRequest,
    now: DateTime<Utc>,
) -> Result<SnapshotPaperActivationOutcome, LedgerError> {
    conn.transaction(|conn| {
        let (proof, already_applied) = prepare_on(conn, request, now, false)?;
        let projection = if already_applied {
            (*paper_ledger::read_on(conn, &proof.request.seed.binding()?)?).clone()
        } else {
            prepared_projection(&proof.request)?
        };
        outcome(
            target_identity_on(conn)?,
            &proof,
            false,
            already_applied,
            projection,
            None,
            None,
        )
    })
}
pub fn apply_snapshot_paper_activation(
    conn: &mut SqliteConnection,
    request: &SnapshotPaperActivationRequest,
    now: DateTime<Utc>,
) -> Result<SnapshotPaperActivationOutcome, LedgerError> {
    conn.immediate_transaction(|conn| {
        let (proof, already_applied) = prepare_on(conn, request, now, true)?;
        let binding = proof.request.seed.binding()?;
        if already_applied {
            let projection = (*paper_ledger::read_on(conn, &binding)?).clone();
            return outcome(
                target_identity_on(conn)?,
                &proof,
                false,
                true,
                projection,
                None,
                None,
            );
        }
        schema::install_on(conn, &proof)?;
        // Installation proved this exact binding before creating its account.
        // Runtime owner admission requires the complete seeded account/head.
        if schema::proof_on(conn)?.request.seed.binding()? != binding {
            return Err(LedgerError::InactiveEpoch);
        }
        let seed_receipt = paper_ledger::seed_on(conn, proof.request.seed.clone(), now)?;
        let closing_receipt = if opening_close_date(&proof.request)?.is_some() {
            let view = paper_ledger::read_on(conn, &binding)?;
            let mut marks = proof.request.seed.marks.clone();
            for mark in &mut marks {
                mark.source = OPENING_CLOSE_SOURCE.into();
            }
            Some(paper_ledger::mark_on(
                conn,
                ValuationBatch {
                    binding: binding.clone(),
                    command_id: format!("{}:opening-close", proof.request.seed.command_id),
                    expected_version: view.version,
                    inventory_fingerprint: view.inventory_fingerprint()?,
                    as_of: proof.request.seed.cutover_at,
                    closing: true,
                    marks,
                },
                now,
            )?)
        } else {
            None
        };
        schema::verify_active_on(conn)?;
        let projection = (*paper_ledger::read_on(conn, &binding)?).clone();
        outcome(
            target_identity_on(conn)?,
            &proof,
            true,
            false,
            projection,
            Some(seed_receipt),
            closing_receipt,
        )
    })
}
fn outcome(
    database_identity: ActivationTargetDatabase,
    proof: &ActivationProof,
    applied: bool,
    already_applied: bool,
    projection: Projection,
    seed_receipt: Option<PaperReceipt>,
    closing_receipt: Option<PaperReceipt>,
) -> Result<SnapshotPaperActivationOutcome, LedgerError> {
    Ok(SnapshotPaperActivationOutcome {
        database_identity,
        mode: if applied {
            "apply"
        } else if already_applied {
            "already_applied"
        } else {
            "preview"
        },
        applied,
        already_applied,
        prepared_request: proof.request.clone(),
        binding: proof.request.seed.binding()?,
        source_bundle_hash: bundle_hash(&proof.source_bundle)?,
        opening_close_date: opening_close_date(&proof.request)?,
        projection,
        seed_receipt,
        closing_receipt,
    })
}
fn target_identity_on(
    conn: &mut SqliteConnection,
) -> Result<ActivationTargetDatabase, LedgerError> {
    #[derive(QueryableByName)]
    struct MainFile {
        #[diesel(sql_type=Text)]
        file: String,
    }
    let file = diesel::sql_query("SELECT file FROM pragma_database_list() WHERE name='main'")
        .get_result::<MainFile>(conn)?
        .file;
    let path = std::fs::canonicalize(file)
        .map_err(|_| unavailable("activation target must be an existing database file"))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|_| unavailable("activation target database identity unavailable"))?;
    Ok(ActivationTargetDatabase {
        path,
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

/// Open only the explicitly selected existing file. No singleton, migrations,
/// journal-mode writes, or monitor initialization occur in this operator path.
pub fn open_snapshot_activation_database(
    path: &Path,
    writable: bool,
) -> Result<SqliteConnection, LedgerError> {
    let path = path
        .canonicalize()
        .map_err(|_| unavailable("explicit activation database path does not exist"))?;
    if !path.is_file() {
        return Err(invalid(
            "activation database must be an existing regular file",
        ));
    }
    let mut url = url::Url::from_file_path(&path)
        .map_err(|_| invalid("activation database path cannot form a SQLite file URI"))?;
    url.query_pairs_mut()
        .append_pair("mode", if writable { "rw" } else { "ro" });
    let mut conn = SqliteConnection::establish(url.as_str())
        .map_err(|e| LedgerError::Database(e.to_string()))?;
    conn.batch_execute("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    if !writable {
        conn.batch_execute("PRAGMA query_only=ON;")?;
    }
    Ok(conn)
}

#[cfg(test)]
#[path = "paper_snapshot_activation_tests.rs"]
mod tests;
