//! Explicit native paper epoch review from one immutable detached snapshot.
//! The wrapper's original target identity is checked against the sealed import
//! receipt; copying a seeded database does not make its new inode the target.
use super::report::{Period, ReadState};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use stock_analysis::database::DatabaseManager;
use stock_analysis::trading::paper_ledger::{
    AccountBinding, EffectiveFillRequest, EffectiveFillScope, EffectiveHistory, PaperLedger,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseIdentity {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSource {
    pub schema_version: u32,
    pub original_database: DatabaseIdentity,
    pub snapshot_database: DatabaseIdentity,
    pub snapshot_sha256: String,
    pub paper_binding_sha256: Option<String>,
}
impl DatabaseIdentity {
    fn verify(&self) -> anyhow::Result<()> {
        let metadata = std::fs::symlink_metadata(&self.path)?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "paper source identity must name a regular file"
        );
        anyhow::ensure!(
            self.path.is_absolute() && self.path.canonicalize()? == self.path,
            "paper source identity path must be canonical"
        );
        anyhow::ensure!(
            metadata.dev() == self.device && metadata.ino() == self.inode,
            "paper source physical identity changed"
        );
        Ok(())
    }
}
impl SnapshotSource {
    pub fn load(path: &Path, snapshot: &Path, sha: &str) -> anyhow::Result<Self> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let before = file.metadata()?;
        anyhow::ensure!(
            before.is_file()
                && before.permissions().mode() & 0o077 == 0
                && before.len() <= 128 * 1024,
            "snapshot-source manifest must be a private bounded regular file"
        );
        let mut bytes = Vec::new();
        file.by_ref().take(128 * 1024 + 1).read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        let named = std::fs::symlink_metadata(path)?;
        anyhow::ensure!(
            before.dev() == after.dev()
                && before.ino() == after.ino()
                && before.len() == after.len()
                && before.mtime() == after.mtime()
                && before.mtime_nsec() == after.mtime_nsec()
                && before.ctime() == after.ctime()
                && before.ctime_nsec() == after.ctime_nsec()
                && before.dev() == named.dev()
                && before.ino() == named.ino()
                && before.len() == bytes.len() as u64,
            "snapshot-source manifest changed during read"
        );
        let source: Self = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            source.schema_version == 1
                && source.snapshot_sha256 == sha
                && source.snapshot_database.path == snapshot.canonicalize()?,
            "snapshot-source manifest does not bind this snapshot"
        );
        source.snapshot_database.verify()?;
        source.original_database.verify()?;
        anyhow::ensure!(
            (
                source.original_database.device,
                source.original_database.inode
            ) != (
                source.snapshot_database.device,
                source.snapshot_database.inode
            ),
            "paper source and detached snapshot must be distinct physical files"
        );
        Ok(source)
    }
    pub fn verify_original(&self) -> anyhow::Result<()> {
        self.original_database.verify()
    }
}

#[derive(Debug, Serialize)]
pub struct Holding {
    pub code: String,
    pub name: String,
    pub quantity: u32,
    pub sellable_from: NaiveDate,
    pub basis_price_cny: f64,
    pub reported_actual_cost_cny: Option<f64>,
    pub mark_price_cny: f64,
    pub mark_observed_at: DateTime<Utc>,
    pub mark_source: String,
}
#[derive(Debug, Serialize)]
pub struct PaperAccount {
    pub schema_version: &'static str,
    pub ledger_kind: &'static str,
    pub account_scope: &'static str,
    pub original_actual_account_as_of: DateTime<Utc>,
    pub original_actual_positions_as_of: DateTime<Utc>,
    pub binding: AccountBinding,
    pub original_database: DatabaseIdentity,
    pub snapshot_sha256: String,
    pub version: i64,
    pub event_hash: String,
    pub projection_sha256: String,
    pub cutover_at: DateTime<Utc>,
    pub ledger_as_of: DateTime<Utc>,
    pub valuation_effective_at: DateTime<Utc>,
    pub valuation_is_current_observation_day: bool,
    pub seed_equity_cny: f64,
    pub cash_cny: f64,
    pub market_value_cny: f64,
    pub total_equity_cny: f64,
    pub since_cutover_net_pnl_cny: f64,
    pub since_cutover_net_pnl_pct: f64,
    pub realized_net_pnl_cny: f64,
    pub unrealized_net_pnl_cny: f64,
    pub modeled_fees_cny: f64,
    pub fee_model: &'static str,
    pub daily_pnl_cny: ReadState<f64>,
    pub daily_baseline_date: Option<NaiveDate>,
    pub holdings: Vec<Holding>,
    pub boundary: &'static str,
}
#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    value: i64,
}
#[derive(QueryableByName)]
struct EventPayload {
    #[diesel(sql_type = Text)]
    payload: String,
}
#[derive(QueryableByName)]
struct Proof {
    #[diesel(sql_type=Text)]
    proof_bytes: String,
}

pub fn native_present(db: &DatabaseManager) -> Result<bool, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query("SELECT COUNT(*) AS value FROM sqlite_master WHERE lower(name) GLOB 'paper_snapshot_activation_*' OR lower(tbl_name) GLOB 'paper_snapshot_activation_*'")
        .get_result::<Count>(&mut conn).map(|row| row.value > 0).map_err(|e| e.to_string())
}

pub fn read(
    db: &DatabaseManager,
    period: &Period,
    source: Option<&SnapshotSource>,
) -> Result<PaperAccount, String> {
    let source = source.ok_or("paper account requires wrapper snapshot-source manifest; source_label is not target identity")?;
    source.verify_original().map_err(|e| e.to_string())?;
    let raw = std::env::var(stock_analysis::trading::paper_ledger_runtime::BINDING_ENV)
        .map_err(|_| "paper account requires explicit binding; no LegacyRaw fallback")?;
    if source.paper_binding_sha256.as_deref() != Some(super::bytes_sha256(raw.as_bytes()).as_str())
    {
        return Err("paper binding differs from wrapper-frozen source manifest".into());
    }
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if value.as_object().is_none_or(|v| {
        v.len() != 3
            || !["account_id", "epoch_id", "manifest_hash"]
                .iter()
                .all(|k| v.contains_key(*k))
    }) {
        return Err("paper binding must contain exactly account_id/epoch_id/manifest_hash".into());
    }
    let binding: AccountBinding = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if !native_present(db)? {
        return Err("independent snapshot account is not activated in this database".into());
    }
    let now = period.observed_at.with_timezone(&Utc);
    let clock = move || now;
    let ledger = PaperLedger::open(db, &clock);
    // DatabaseManager owns immutable query-only detached bytes. Both calls use
    // that exact snapshot; the view and opaque economic receipt must agree.
    let view = ledger.read(&binding).map_err(|e| e.to_string())?;
    let effective = ledger
        .verified_effective_fills(&EffectiveFillRequest {
            scope: EffectiveFillScope::Epoch(binding.clone()),
            history: EffectiveHistory::RestatedLatest,
            as_of: period.observed_at.date_naive(),
        })
        .map_err(|e| e.to_string())?;
    let receipt = effective.receipt();
    if receipt.ledger_head.as_ref() != Some(&(view.version, view.event_hash.clone()))
        || receipt.inventory_fingerprint.as_deref()
            != Some(
                view.inventory_fingerprint()
                    .map_err(|e| e.to_string())?
                    .as_str(),
            )
    {
        return Err(
            "paper account and effective receipt do not share the same snapshot head".into(),
        );
    }
    // Ledger replay above validates immutable proof bytes, canonical manifest,
    // namespace and event hashes. This additional read grants only provenance.
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let proof =
        diesel::sql_query("SELECT proof_bytes FROM paper_snapshot_activation_v1 WHERE singleton=1")
            .get_result::<Proof>(&mut conn)
            .map_err(|e| e.to_string())?;
    let proof: serde_json::Value =
        serde_json::from_str(&proof.proof_bytes).map_err(|e| e.to_string())?;
    let original: serde_json::Value = serde_json::from_str(
        proof["source_bundle"]["import_receipt_json"]
            .as_str()
            .ok_or("paper proof has no original import receipt")?,
    )
    .map_err(|e| e.to_string())?;
    if original["database_identity"]["device"].as_u64() != Some(source.original_database.device)
        || original["database_identity"]["inode"].as_u64() != Some(source.original_database.inode)
        || original["database"].as_str() != source.original_database.path.to_str()
    {
        return Err("paper original target does not match immutable import receipt; copied fixture source rejected".into());
    }
    let activated_at: DateTime<Utc> =
        serde_json::from_value(proof["activated_at"].clone()).map_err(|e| e.to_string())?;
    if activated_at > now {
        return Err(
            "paper activation was committed after observed_at; historical head unavailable".into(),
        );
    }
    for event in
        diesel::sql_query("SELECT payload FROM paper_ledger_event WHERE account_id=? ORDER BY seq")
            .bind::<Text, _>(&binding.account_id)
            .load::<EventPayload>(&mut conn)
            .map_err(|e| e.to_string())?
    {
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload).map_err(|e| e.to_string())?;
        let timestamp = if let Some(fact) = payload.get("Seeded") {
            fact["manifest"]["cutover_at"].as_str()
        } else if let Some(fact) = payload.get("Order") {
            fact["occurred_at"].as_str()
        } else if let Some(fact) = payload.get("Marked") {
            fact["as_of"].as_str()
        } else if let Some(fact) = payload.get("AdjudicatedV1") {
            fact["request"]["decision_at"].as_str()
        } else if let Some(fact) = payload.get("DerivedSnapshotV1") {
            let at = chrono::NaiveDateTime::parse_from_str(
                fact["metrics"]["created_at"]
                    .as_str()
                    .ok_or("derived paper event lacks original creation time")?,
                "%Y-%m-%d %H:%M:%S",
            )
            .map_err(|e| e.to_string())?
            .and_utc();
            if at > now {
                return Err(
                    "paper derived head event is after observed_at; historical head unavailable"
                        .into(),
                );
            }
            continue;
        } else {
            return Err(
                "unsupported paper event temporal shape; historical observation unavailable".into(),
            );
        };
        let at =
            DateTime::parse_from_rfc3339(timestamp.ok_or("paper event lacks original timestamp")?)
                .map_err(|e| e.to_string())?
                .with_timezone(&Utc);
        if at > now {
            return Err(
                "paper head event is after observed_at; historical head unavailable".into(),
            );
        }
    }
    let seed: stock_analysis::trading::paper_ledger::SeedManifest =
        serde_json::from_value(proof["request"]["seed"].clone()).map_err(|e| e.to_string())?;
    let cutover = receipt.cutover_at.ok_or("paper receipt missing cutover")?;
    if view.as_of > now || cutover > now {
        return Err("paper account contains future financial facts".into());
    }
    let mut holdings = Vec::new();
    for lot in &view.lots {
        let mark = view
            .marks
            .get(&lot.code)
            .ok_or("paper account has incomplete marks")?;
        if mark.observed_at > now {
            return Err("paper account contains future valuation marks".into());
        }
        holdings.push(Holding {
            code: lot.code.clone(),
            name: lot.name.clone(),
            quantity: lot.quantity,
            sellable_from: lot.sellable_from,
            basis_price_cny: lot.basis_price.cny(),
            reported_actual_cost_cny: lot.reported_cost.map(|m| m.cny()),
            mark_price_cny: mark.price.cny(),
            mark_observed_at: mark.observed_at,
            mark_source: mark.source.clone(),
        });
    }
    let valuation_at = holdings
        .iter()
        .map(|h| h.mark_observed_at)
        .min()
        .unwrap_or(view.as_of);
    let day = |at: DateTime<Utc>| {
        at.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
            .date_naive()
    };
    let today = period.observed_at.date_naive();
    let valuation_current = holdings.is_empty() || day(valuation_at) == today;
    let previous = stock_analysis::calendar::verified_prev_a_share_trading_day(today)?;
    let (baseline_date, baseline) = if valuation_current && day(cutover) != today {
        (Some(previous), view.closes.get(&previous).copied())
    } else {
        (None, None)
    };
    let equity = view.equity().map_err(|e| e.to_string())?.cny();
    let seed_equity = view.seed_equity.cny();
    if seed_equity <= 0.0 {
        return Err("paper seed equity must be positive".into());
    }
    let daily = if !valuation_current {
        ReadState::unavailable("held prices are from an earlier observation day; no daily PnL")
    } else if day(cutover) == today {
        ReadState::unavailable(
            "seed-day PnL is since-cutover only; no prior paper close daily denominator",
        )
    } else if let Some(baseline) = baseline {
        ReadState::available(equity - baseline.cny())
    } else {
        ReadState::unavailable("missing exact previous verified trading-day paper close")
    };
    source.verify_original().map_err(|e| e.to_string())?;
    Ok(PaperAccount {schema_version:"weekly-native-paper-account-v1",ledger_kind:"NativeSnapshotPaperV1",account_scope:"current_account_as_of_observed_at_not_historical_week_close",original_actual_account_as_of:seed.account_effective_at,original_actual_positions_as_of:seed.positions_effective_at,binding,original_database:source.original_database.clone(),snapshot_sha256:source.snapshot_sha256.clone(),
        version:view.version,event_hash:view.event_hash.clone(),projection_sha256:receipt.projection_hash.clone(),cutover_at:cutover,
        ledger_as_of:view.as_of,valuation_effective_at:valuation_at,valuation_is_current_observation_day:valuation_current,
        seed_equity_cny:seed_equity,cash_cny:view.cash.cny(),market_value_cny:equity-view.cash.cny(),total_equity_cny:equity,
        since_cutover_net_pnl_cny:equity-seed_equity,since_cutover_net_pnl_pct:(equity/seed_equity-1.0)*100.0,
        realized_net_pnl_cny:view.realized_pnl.cny(),unrealized_net_pnl_cny:view.unrealized_pnl().map_err(|e|e.to_string())?.cny(),
        modeled_fees_cny:view.fees.cny(),fee_model:stock_analysis::trading::paper_ledger::FEE_MODEL,
        daily_pnl_cny:daily,daily_baseline_date:baseline_date.filter(|_|baseline.is_some()),holdings,
        boundary:"Independent seeded paper account since cutover, valued from persisted marks at their original observation times; source snapshot is immutable, original target is physically pinned. Modeled paper fees only, no observed brokerage settlement or family attribution; original real-account losses are not this epoch's PnL."})
}
