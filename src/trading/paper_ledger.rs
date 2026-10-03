//! Independent, explicitly seeded paper account. No live-account balance refresh.

use crate::database::DatabaseManager;
use chrono::{DateTime, NaiveDate, Utc};
#[cfg(test)]
use diesel::connection::SimpleConnection;
use diesel::{
    prelude::*,
    sql_types::{BigInt, Text},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
#[path = "paper_ledger_execution.rs"]
mod execution;
use execution::{apply_fact, OrderFact};
pub use execution::{ExecuteIntent, PriceIntent, ValuationBatch};
#[path = "paper_ledger_adjudication.rs"]
mod adjudication;
pub use adjudication::{
    AccountProjectionImpact, Adjudication, AdjudicationAction, AdjudicationPreview,
    FillFingerprint, HistoricalProjectionImpact,
};
#[path = "paper_effective_fills.rs"]
mod effective;
pub use effective::{
    EffectiveFillRequest, EffectiveFillScope, EffectiveHistory, EffectiveProjectionReceipt,
    FillAuthority, FillLineage, VerifiedEffectiveFillSet,
};
pub(crate) use effective::{observe_actual_parent_fills, RecordedPaperV2EffectiveFillSet};
#[path = "paper_ledger_snapshot.rs"]
mod snapshot;
pub use snapshot::SnapshotRevision;
#[path = "paper_opening_inventory.rs"]
mod opening_inventory;
pub use opening_inventory::{
    EffectiveExitPnl, EffectiveSellablePosition, OpeningInventoryExit, OpeningInventorySample,
};

/// Reuses a caller-owned read transaction; never opens a second connection or
/// combines independently observed heads for attribution/report consumers.
pub(crate) fn verified_effective_fills_on(
    conn: &mut SqliteConnection,
    request: &EffectiveFillRequest,
) -> Result<VerifiedEffectiveFillSet, LedgerError> {
    effective::verified_on(conn, request)
}

const GENESIS: &str = "PAPER_LEDGER_GENESIS_V1";
pub const MONEY_MODEL: &str = "micro-cny-half-up-v1";
pub const FEE_MODEL: &str = "lot-rates-v1";

/// Signed micro-CNY; all persisted arithmetic is checked integer arithmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money(i64);
impl Money {
    pub const ZERO: Self = Self(0);
    pub(crate) const fn micros(self) -> i64 {
        self.0
    }
    pub(crate) const fn from_micros(value: i64) -> Self {
        Self(value)
    }
    pub fn from_cny(value: f64) -> Result<Self, LedgerError> {
        let scaled = value * 1_000_000.0;
        if !scaled.is_finite() || scaled.abs() >= i64::MAX as f64 {
            return Err(LedgerError::InvalidInput(
                "money overflow/non-finite".into(),
            ));
        }
        Ok(Self(scaled.round() as i64))
    }
    pub fn cny(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
    fn add(self, other: Self) -> Result<Self, LedgerError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(LedgerError::Overflow)
    }
    fn sub(self, other: Self) -> Result<Self, LedgerError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(LedgerError::Overflow)
    }
    fn mul(self, quantity: u32) -> Result<Self, LedgerError> {
        self.0
            .checked_mul(i64::from(quantity))
            .map(Self)
            .ok_or(LedgerError::Overflow)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("paper account not explicitly seeded")]
    NotSeeded,
    #[error("inactive paper account/epoch/manifest binding")]
    InactiveEpoch,
    #[error("paper command/business identity conflict")]
    IdentityConflict,
    #[error("paper head version changed; reread and explicitly retry")]
    VersionChanged,
    #[error("paper evidence unavailable: {0}")]
    EvidenceUnavailable(String),
    #[error("invalid paper input: {0}")]
    InvalidInput(String),
    #[error("paper ledger integrity failure: {0}")]
    IntegrityFailure(String),
    #[error("paper money/quantity overflow")]
    Overflow,
    #[error("paper database error: {0}")]
    Database(String),
    #[error("paper database busy; same command may be retried")]
    BusyRetryable,
    #[error("paper commit outcome unknown; recover with SAME command identity")]
    CommitOutcomeUnknown,
    #[error("paper execution cancelled")]
    Cancelled,
}
impl From<diesel::result::Error> for LedgerError {
    fn from(value: diesel::result::Error) -> Self {
        let message = value.to_string();
        if message.contains("locked") || message.contains("busy") {
            Self::BusyRetryable
        } else {
            Self::Database(message)
        }
    }
}

/// Reuse a full audit-chain proof only while replaying one database snapshot.
/// The V5 owner verifier holds a transaction around all uses of this guard.
#[derive(Default)]
pub(crate) struct V1AuditReplayGuard {
    validated: bool,
}

impl V1AuditReplayGuard {
    fn ensure_validated(&mut self, conn: &mut SqliteConnection) -> Result<(), LedgerError> {
        if !self.validated {
            crate::database::order_audit::validate_order_audit_chain(conn)?;
            self.validated = true;
        }
        Ok(())
    }
}

fn encode<T: Serialize>(value: &T) -> Result<String, LedgerError> {
    serde_json::to_string(value).map_err(|error| LedgerError::IntegrityFailure(error.to_string()))
}
fn decode<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, LedgerError> {
    serde_json::from_str(value).map_err(|error| LedgerError::IntegrityFailure(error.to_string()))
}
fn digest(bytes: &str) -> String {
    hex::encode(Sha256::digest(bytes.as_bytes()))
}
fn day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("Shanghai UTC+8"))
        .date_naive()
}

/// v1 freezes existing policy denominators: concentration/pretrade equity,
/// cash floor/(pretrade equity - commission). No runtime env changes on replay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskPolicyV1 {
    pub max_position_bps: u32,
    pub cash_floor_bps: u32,
    pub max_slippage_bps: u32,
}
impl Default for RiskPolicyV1 {
    fn default() -> Self {
        Self {
            max_position_bps: 1000,
            cash_floor_bps: 1500,
            max_slippage_bps: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub code: String,
    pub price: Money,
    pub observed_at: DateTime<Utc>,
    pub source: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedLot {
    pub code: String,
    pub name: String,
    pub quantity: u32,
    pub reported_cost: Option<Money>,
    /// None means unknown acquisition: conservatively next verified trading day.
    pub sellable_from: Option<NaiveDate>,
    pub sellability_evidence: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedManifest {
    pub account_id: String,
    pub epoch_id: String,
    pub command_id: String,
    pub cutover_at: DateTime<Utc>,
    pub account_effective_at: DateTime<Utc>,
    pub positions_effective_at: DateTime<Utc>,
    pub source_reference: String,
    pub source_hash: String,
    pub approved_by: String,
    pub cash: Money,
    pub original_total: Money,
    /// Explicit approval to exclude this residual; never becomes tradeable cash.
    pub excluded_residual: Option<Money>,
    pub lots: Vec<SeedLot>,
    pub marks: Vec<Mark>,
    pub policy: RiskPolicyV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBinding {
    pub account_id: String,
    pub epoch_id: String,
    pub manifest_hash: String,
}
impl SeedManifest {
    pub fn binding(&self) -> Result<AccountBinding, LedgerError> {
        Ok(AccountBinding {
            account_id: self.account_id.clone(),
            epoch_id: self.epoch_id.clone(),
            manifest_hash: digest(&encode(&(1, MONEY_MODEL, FEE_MODEL, self))?),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lot {
    pub lot_id: String,
    pub code: String,
    pub name: String,
    pub quantity: u32,
    pub basis_price: Money,
    pub buy_fee_remaining: Money,
    pub acquired_on: NaiveDate,
    pub sellable_from: NaiveDate,
    pub reported_cost: Option<Money>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    pub cash: Money,
    pub lots: Vec<Lot>,
    pub marks: BTreeMap<String, Mark>,
    pub fees: Money,
    pub realized_pnl: Money,
    pub seed_equity: Money,
    pub as_of: DateTime<Utc>,
    pub closes: BTreeMap<NaiveDate, Money>,
    /// Absent on frozen V1 bytes. A ruling may expose an unresolved dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    economic_unavailable: Option<String>,
}
impl Projection {
    fn require_available(&self) -> Result<(), LedgerError> {
        if let Some(reason) = &self.economic_unavailable {
            return Err(LedgerError::EvidenceUnavailable(reason.clone()));
        }
        Ok(())
    }
    pub fn equity(&self) -> Result<Money, LedgerError> {
        self.lots.iter().try_fold(self.cash, |equity, lot| {
            let mark = self.marks.get(&lot.code).ok_or_else(|| {
                LedgerError::EvidenceUnavailable(format!("missing mark {}", lot.code))
            })?;
            equity.add(mark.price.mul(lot.quantity)?)
        })
    }
    pub fn daily_pnl(&self) -> Option<Money> {
        let previous = crate::calendar::verified_prev_a_share_trading_day(day(self.as_of)).ok()?;
        self.equity().ok()?.sub(*self.closes.get(&previous)?).ok()
    }
    pub fn inventory_fingerprint(&self) -> Result<String, LedgerError> {
        Ok(digest(&encode(&self.lots)?))
    }
    /// Net unrealized result since cutover; seed reported cost is reference only.
    pub fn unrealized_pnl(&self) -> Result<Money, LedgerError> {
        self.lots.iter().try_fold(Money::ZERO, |total, lot| {
            let mark = self.marks.get(&lot.code).ok_or_else(|| {
                LedgerError::EvidenceUnavailable(format!("missing mark {}", lot.code))
            })?;
            total.add(
                mark.price
                    .sub(lot.basis_price)?
                    .mul(lot.quantity)?
                    .sub(lot.buy_fee_remaining)?,
            )
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperView {
    pub version: i64,
    pub event_hash: String,
    projection: Projection,
}
impl std::ops::Deref for PaperView {
    type Target = Projection;
    fn deref(&self) -> &Projection {
        &self.projection
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperReceipt {
    pub version: i64,
    pub event_hash: String,
    pub already_applied: bool,
    pub status: LedgerStatus,
    pub reason: Option<String>,
    pub paper_trade_id: Option<i64>,
    pub audit: Option<AuditLink>,
    pub cash_delta: Money,
    pub fee: Money,
    pub fill_price: Option<Money>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerStatus {
    Seeded,
    Marked,
    Filled,
    NotFilled,
    Invalidated,
    Rejected,
    Adjudicated,
    SnapshotRecorded,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLink {
    pub id: i64,
    pub previous_hash: String,
    pub record_hash: String,
    pub created_at: String,
}
/// Canonical account-scoped fill view. Task5 corrections must extend this same
/// fact-chain projection rather than rereading all legacy compatible rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveFill {
    pub version: i64,
    pub event_hash: String,
    pub paper_trade_id: i64,
    pub plan_id: String,
    pub code: String,
    pub direction: String,
    pub quantity: u32,
    pub fill_price: Money,
    pub fee: Money,
    pub occurred_at: DateTime<Utc>,
    pub audit: AuditLink,
}
pub enum PaperCommand {
    Seed(SeedManifest),
    Execute(ExecuteIntent),
    Mark(ValuationBatch),
    Adjudicate(Adjudication),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Fact {
    Seeded {
        manifest: SeedManifest,
        legacy_high_water_id: i64,
        legacy_audit_high_water: String,
    },
    Order(OrderFact),
    Marked(ValuationBatch),
    AdjudicatedV1(adjudication::AdjudicatedFact),
    DerivedSnapshotV1(SnapshotRevision),
}
fn receipt(version: i64, hash: String, fact: &Fact, repeated: bool) -> PaperReceipt {
    let mut result = PaperReceipt {
        version,
        event_hash: hash,
        already_applied: repeated,
        status: LedgerStatus::Seeded,
        reason: None,
        paper_trade_id: None,
        audit: None,
        cash_delta: Money::ZERO,
        fee: Money::ZERO,
        fill_price: None,
    };
    match fact {
        Fact::Seeded { .. } => {}
        Fact::Marked(_) => result.status = LedgerStatus::Marked,
        Fact::DerivedSnapshotV1(_) => result.status = LedgerStatus::SnapshotRecorded,
        Fact::AdjudicatedV1(ruling) => {
            result.status = LedgerStatus::Adjudicated;
            result.reason = ruling.projection.economic_unavailable.clone();
            if let Some((_, reason)) = &ruling.historical_projection {
                result.reason = reason.clone();
            }
        }
        Fact::Order(order) => {
            result.status = order.status;
            result.reason = order.reason.clone();
            result.paper_trade_id = order.paper_trade_id;
            result.audit = Some(order.audit.clone());
            result.cash_delta = order.cash_delta;
            result.fee = order
                .commission
                .add(order.stamp)
                .expect("checked before persistence");
            result.fill_price =
                (order.status == LedgerStatus::Filled).then_some(order.requested_price);
        }
    }
    result
}

pub struct PaperLedger<'a> {
    db: &'a DatabaseManager,
    clock: &'a (dyn Fn() -> DateTime<Utc> + Sync),
    #[cfg(test)]
    after_commit_fault: Option<&'a (dyn Fn() -> bool + Sync)>,
    #[cfg(test)]
    before_commit_fault: Option<&'a (dyn Fn() -> bool + Sync)>,
}
impl<'a> PaperLedger<'a> {
    pub fn open(db: &'a DatabaseManager, clock: &'a (dyn Fn() -> DateTime<Utc> + Sync)) -> Self {
        Self {
            db,
            clock,
            #[cfg(test)]
            after_commit_fault: None,
            #[cfg(test)]
            before_commit_fault: None,
        }
    }
    pub fn apply(&self, command: PaperCommand) -> Result<PaperReceipt, LedgerError> {
        self.apply_controlled(command, &AtomicBool::new(false))
    }
    pub fn apply_controlled(
        &self,
        command: PaperCommand,
        cancelled: &AtomicBool,
    ) -> Result<PaperReceipt, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        let mut ready_to_commit = false;
        let result = conn.immediate_transaction(|conn| {
            if cancelled.load(Ordering::SeqCst) {
                return Err(LedgerError::Cancelled);
            }
            let binding = match &command {
                PaperCommand::Seed(seed) => seed.binding()?,
                PaperCommand::Execute(intent) => intent.binding.clone(),
                PaperCommand::Mark(batch) => batch.binding.clone(),
                PaperCommand::Adjudicate(request) => request.binding.clone(),
            };
            require_v1_owner_on(conn, &binding)?;
            let at = (self.clock)();
            if cancelled.load(Ordering::SeqCst) {
                return Err(LedgerError::Cancelled);
            }
            let result = match command {
                PaperCommand::Seed(seed) => self.seed(conn, seed),
                PaperCommand::Execute(intent) => self.execute(conn, intent, at),
                PaperCommand::Mark(batch) => self.mark(conn, batch, at),
                PaperCommand::Adjudicate(request) => self.adjudicate_on(conn, request, at),
            }?;
            #[cfg(test)]
            if self.before_commit_fault.is_some_and(|fault| fault()) {
                return Err(LedgerError::Database(
                    "TEST_CODE injected post-write transaction failure".into(),
                ));
            }
            if cancelled.load(Ordering::SeqCst) {
                return Err(LedgerError::Cancelled);
            }
            ready_to_commit = true;
            Ok(result)
        });
        match result {
            Err(_) if ready_to_commit => Err(LedgerError::CommitOutcomeUnknown),
            #[cfg(test)]
            Ok(_) if self.after_commit_fault.is_some_and(|fault| fault()) => {
                Err(LedgerError::CommitOutcomeUnknown)
            }
            result => result,
        }
    }
    pub fn read(&self, binding: &AccountBinding) -> Result<PaperView, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        conn.transaction(|conn| {
            verify_v4_read_catalog_on(conn)?;
            let view = load(conn, binding)?;
            view.require_available()?;
            Ok(view)
        })
    }
    /// Runtime admission check before fetching quotes or recovering a terminal.
    /// The locked writer checks the same owner again before any mutation.
    pub(crate) fn require_active_v1_owner(
        &self,
        binding: &AccountBinding,
    ) -> Result<(), LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        conn.transaction(|conn| require_v1_owner_on(conn, binding))
    }
    /// Recover a committed business terminal without obtaining new market
    /// evidence. This is read-only, validates the same chain and intent as
    /// Execute, and never authorizes a new attempt or revives a rejection.
    pub fn recover_terminal(
        &self,
        binding: &AccountBinding,
        signal: &super::paper_trade::PaperSignal,
        price_intent: PriceIntent,
    ) -> Result<Option<PaperReceipt>, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        conn.transaction(|conn| {
            verify_v4_read_catalog_on(conn)?;
            load(conn, binding)?;
            let hash = execution::order_intent_hash(binding, signal, price_intent)?;
            replay_command(conn, binding, None, Some((&signal.plan_id, &hash)), None)
        })
    }
    pub fn read_at_version(
        &self,
        binding: &AccountBinding,
        version: i64,
    ) -> Result<PaperView, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        conn.transaction(|conn| {
            verify_v4_read_catalog_on(conn)?;
            let head = load(conn, binding)?;
            if version < 1 || version > head.version {
                return Err(LedgerError::InvalidInput(
                    "version outside committed history".into(),
                ));
            }
            if version == head.version {
                head.require_available()?;
                return Ok(head);
            }
            let view = replay_through(conn, binding, Some(version))?;
            view.require_available()?;
            Ok(view)
        })
    }
    pub fn effective_fills(
        &self,
        binding: &AccountBinding,
    ) -> Result<Vec<EffectiveFill>, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        conn.transaction(|conn| {
            verify_v4_read_catalog_on(conn)?;
            load(conn, binding)?.require_available()?;
            let rulings = adjudication::latest_rulings(&events(conn, &binding.account_id)?)?;
            let mut fills = Vec::new();
            for event in events(conn, &binding.account_id)? {
                if let Fact::Order(order) = decode(&event.payload)? {
                    if order.status == LedgerStatus::Filled {
                        let mut fill = EffectiveFill {
                            version: event.seq,
                            event_hash: event.event_hash,
                            paper_trade_id: order.paper_trade_id.ok_or_else(|| {
                                LedgerError::IntegrityFailure(
                                    "filled event lacks compatible identity".into(),
                                )
                            })?,
                            plan_id: order.plan_id,
                            code: order.code,
                            direction: order.direction,
                            quantity: order.quantity,
                            fill_price: order.requested_price,
                            fee: order.commission.add(order.stamp)?,
                            occurred_at: order.occurred_at,
                            audit: order.audit,
                        };
                        if let Some(action) = rulings.get(&fill.paper_trade_id) {
                            match action {
                                AdjudicationAction::Quarantine => continue,
                                AdjudicationAction::CorrectionDeclared {
                                    price,
                                    quantity,
                                    fact_at,
                                } => {
                                    fill.fill_price = *price;
                                    fill.quantity = *quantity;
                                    fill.occurred_at = *fact_at;
                                    fill.fee =
                                        adjudication::fee(*price, *quantity, &fill.direction)?;
                                }
                            }
                        }
                        fills.push(fill);
                    }
                }
            }
            Ok(fills)
        })
    }
    /// Explicit maintenance only: rebuild an absent cache from verified facts.
    /// Existing contradictory state is never overwritten or silently repaired.
    pub fn repair_missing_projection(
        &self,
        binding: &AccountBinding,
    ) -> Result<PaperView, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|error| LedgerError::Database(error.to_string()))?;
        conn.immediate_transaction(|conn| {
            require_v1_owner_on(conn, binding)?;
            if head(conn, binding)?.is_some() { return load(conn,binding); }
            let view = replay(conn,binding)?;
            let bytes = encode(&view.projection)?;
            diesel::sql_query("INSERT INTO paper_ledger_head(account_id,version,event_hash,projection_bytes,projection_hash) VALUES (?,?,?,?,?)")
                .bind::<Text,_>(&binding.account_id).bind::<BigInt,_>(view.version).bind::<Text,_>(&view.event_hash)
                .bind::<Text,_>(&bytes).bind::<Text,_>(digest(&bytes)).execute(conn)?;
            Ok(view)
        })
    }
    fn seed(
        &self,
        conn: &mut SqliteConnection,
        seed: SeedManifest,
    ) -> Result<PaperReceipt, LedgerError> {
        let binding = seed.binding()?;
        let existing = account(conn, &seed.account_id)?;
        if let Some(existing) = existing {
            if existing.manifest_hash != binding.manifest_hash {
                return Err(LedgerError::IdentityConflict);
            }
            load(conn, &binding)?;
            let event = events(conn, &binding.account_id)?.remove(0);
            return Ok(receipt(1, event.event_hash, &decode(&event.payload)?, true));
        }
        let projection = seed_projection(&seed)?;
        if seed.cutover_at > (self.clock)() {
            return Err(LedgerError::InvalidInput("future cutover".into()));
        }
        let legacy = diesel::sql_query("SELECT COALESCE(MAX(id),0) AS value FROM paper_trades")
            .get_result::<IntegerRow>(conn)?
            .value;
        let legacy_audit_high_water =
            crate::database::order_audit::validate_order_audit_chain(conn)?;
        let payload = encode(&Fact::Seeded {
            manifest: seed.clone(),
            legacy_high_water_id: legacy,
            legacy_audit_high_water,
        })?;
        let hash = event_hash(&binding.account_id, 1, &seed.command_id, GENESIS, &payload)?;
        diesel::sql_query("INSERT INTO paper_ledger_account(account_id,epoch_id,manifest_hash,manifest_bytes,money_model,fee_model) VALUES (?,?,?,?,?,?)")
            .bind::<Text,_>(&binding.account_id).bind::<Text,_>(&binding.epoch_id).bind::<Text,_>(&binding.manifest_hash)
            .bind::<Text,_>(encode(&seed)?).bind::<Text,_>(MONEY_MODEL).bind::<Text,_>(FEE_MODEL).execute(conn)?;
        diesel::sql_query("INSERT INTO paper_ledger_event(account_id,seq,command_id,previous_hash,event_hash,payload) VALUES (?,1,?,?,?,?)")
            .bind::<Text,_>(&binding.account_id).bind::<Text,_>(&seed.command_id).bind::<Text,_>(GENESIS).bind::<Text,_>(&hash).bind::<Text,_>(&payload).execute(conn)?;
        let bytes = encode(&projection)?;
        diesel::sql_query("INSERT INTO paper_ledger_head(account_id,version,event_hash,projection_bytes,projection_hash) VALUES (?,1,?,?,?)")
            .bind::<Text,_>(&binding.account_id).bind::<Text,_>(&hash).bind::<Text,_>(&bytes).bind::<Text,_>(digest(&bytes)).execute(conn)?;
        Ok(receipt(1, hash, &decode(&payload)?, false))
    }
}
pub(super) fn require_v1_owner_on(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
) -> Result<(), LedgerError> {
    crate::database::paper_book_owner_schema_v1::require_v1_owner_on(
        conn,
        &binding.account_id,
        &binding.epoch_id,
        &binding.manifest_hash,
    )
    .map_err(|error| match error {
        crate::database::paper_book_owner_schema_v1::PaperBookOwnerError::InactiveOwner => {
            LedgerError::InactiveEpoch
        }
        error => LedgerError::IntegrityFailure(error.to_string()),
    })
}
/// Historical reads retain V1-V3 behavior; V4/V5 views require the complete
/// owner/catalog namespace in the same read transaction.
fn verify_v4_read_catalog_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    let generation = diesel::sql_query("SELECT user_version AS value FROM pragma_user_version()")
        .get_result::<IntegerRow>(conn)?
        .value;
    let owner_objects = diesel::sql_query(
        "SELECT ((SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'paper_book_owner_*' OR lower(tbl_name) GLOB 'paper_book_owner_*')
              + (SELECT COUNT(*) FROM temp.sqlite_master WHERE lower(name) GLOB 'paper_book_owner_*' OR lower(tbl_name) GLOB 'paper_book_owner_*')) AS value",
    )
    .get_result::<IntegerRow>(conn)?
    .value;
    let fee_objects = diesel::sql_query(
        "SELECT ((SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'paper_book_v2_*' OR lower(tbl_name) GLOB 'paper_book_v2_*')
              + (SELECT COUNT(*) FROM temp.sqlite_master WHERE lower(name) GLOB 'paper_book_v2_*' OR lower(tbl_name) GLOB 'paper_book_v2_*')) AS value",
    )
    .get_result::<IntegerRow>(conn)?
    .value;
    if generation == 5 {
        crate::database::paper_book_owner_schema_v2::verify_catalog_v5_on(conn)
            .map_err(|error| LedgerError::IntegrityFailure(error.to_string()))?;
    } else if generation >= 4 || owner_objects != 0 || (generation != 2 && fee_objects != 0) {
        crate::database::paper_book_owner_schema_v1::verify_catalog_v4_on(conn)
            .map_err(|error| LedgerError::IntegrityFailure(error.to_string()))?;
    }
    Ok(())
}
fn seed_projection(seed: &SeedManifest) -> Result<Projection, LedgerError> {
    if [
        &seed.account_id,
        &seed.epoch_id,
        &seed.command_id,
        &seed.source_reference,
        &seed.approved_by,
    ]
    .iter()
    .any(|v| v.trim().is_empty())
        || seed.source_hash.len() != 64
        || !seed.source_hash.bytes().all(|c| c.is_ascii_hexdigit())
        || seed.account_effective_at != seed.positions_effective_at
        || seed.account_effective_at != seed.cutover_at
        || seed.cash < Money::ZERO
        || seed
            .excluded_residual
            .is_some_and(|residual| residual < Money::ZERO)
        || seed.policy.max_position_bps > 10000
        || seed.policy.cash_floor_bps > 10000
    {
        return Err(LedgerError::InvalidInput(
            "invalid seed identity, effective time or policy".into(),
        ));
    }
    let next = crate::calendar::verified_next_a_share_trading_day(day(seed.cutover_at))
        .map_err(LedgerError::EvidenceUnavailable)?;
    let mut marks = BTreeMap::new();
    for mark in &seed.marks {
        if mark.price <= Money::ZERO
            || mark.observed_at != seed.cutover_at
            || mark.source.trim().is_empty()
            || marks.insert(mark.code.clone(), mark.clone()).is_some()
        {
            return Err(LedgerError::InvalidInput("invalid seed mark".into()));
        }
    }
    let mut lots = Vec::new();
    for (i, lot) in seed.lots.iter().enumerate() {
        if lot.quantity == 0 || !lot.quantity.is_multiple_of(100) || lot.code.trim().is_empty() {
            return Err(LedgerError::InvalidInput("invalid seed lot".into()));
        }
        let mark = marks
            .get(&lot.code)
            .ok_or_else(|| LedgerError::EvidenceUnavailable("seed lot mark missing".into()))?;
        let sellable_from = match lot.sellable_from {
            Some(date)
                if lot
                    .sellability_evidence
                    .as_ref()
                    .is_some_and(|e| !e.trim().is_empty()) =>
            {
                date
            }
            None => next,
            _ => {
                return Err(LedgerError::InvalidInput(
                    "explicit sellability lacks evidence".into(),
                ))
            }
        };
        lots.push(Lot {
            lot_id: format!("seed:{i}"),
            code: lot.code.clone(),
            name: lot.name.clone(),
            quantity: lot.quantity,
            basis_price: mark.price,
            buy_fee_remaining: Money::ZERO,
            acquired_on: day(seed.cutover_at),
            sellable_from,
            reported_cost: lot.reported_cost,
        });
    }
    if marks.len()
        != lots
            .iter()
            .map(|lot| &lot.code)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    {
        return Err(LedgerError::InvalidInput(
            "seed mark coverage mismatch".into(),
        ));
    }
    let mut projection = Projection {
        cash: seed.cash,
        lots,
        marks,
        fees: Money::ZERO,
        realized_pnl: Money::ZERO,
        seed_equity: Money::ZERO,
        as_of: seed.cutover_at,
        closes: BTreeMap::new(),
        economic_unavailable: None,
    };
    projection.seed_equity = projection.equity()?;
    if projection.seed_equity <= Money::ZERO
        || seed.original_total.sub(projection.seed_equity)?
            != seed.excluded_residual.unwrap_or(Money::ZERO)
    {
        return Err(LedgerError::InvalidInput(
            "unapproved seed residual/empty equity".into(),
        ));
    }
    Ok(projection)
}
#[derive(QueryableByName)]
struct IntegerRow {
    #[diesel(sql_type = BigInt)]
    value: i64,
}
#[derive(QueryableByName)]
struct AccountRow {
    #[diesel(sql_type = Text)]
    epoch_id: String,
    #[diesel(sql_type = Text)]
    manifest_hash: String,
    #[diesel(sql_type = Text)]
    manifest_bytes: String,
}
#[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(QueryableByName)]
pub(crate) struct EventRow {
    #[diesel(sql_type = BigInt)]
    seq: i64,
    #[diesel(sql_type = Text)]
    command_id: String,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    event_hash: String,
    #[diesel(sql_type = Text)]
    payload: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
    business_plan_id: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
    intent_hash: Option<String>,
    #[diesel(sql_type = BigInt)]
    is_terminal: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable<BigInt>)]
    paper_trade_id: Option<i64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<BigInt>)]
    order_audit_id: Option<i64>,
}
#[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(QueryableByName)]
pub(crate) struct HeadRow {
    #[diesel(sql_type = BigInt)]
    version: i64,
    #[diesel(sql_type = Text)]
    event_hash: String,
    #[diesel(sql_type = Text)]
    projection_bytes: String,
    #[diesel(sql_type = Text)]
    projection_hash: String,
}
impl EventRow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_bounded_sql_parts(
        seq: i64,
        command_id: String,
        previous_hash: String,
        event_hash: String,
        payload: String,
        business_plan_id: Option<String>,
        intent_hash: Option<String>,
        is_terminal: i64,
        paper_trade_id: Option<i64>,
        order_audit_id: Option<i64>,
    ) -> Self {
        Self {
            seq,
            command_id,
            previous_hash,
            event_hash,
            payload,
            business_plan_id,
            intent_hash,
            is_terminal,
            paper_trade_id,
            order_audit_id,
        }
    }
}
impl HeadRow {
    pub(crate) fn from_bounded_sql_parts(
        version: i64,
        event_hash: String,
        projection_bytes: String,
        projection_hash: String,
    ) -> Self {
        Self {
            version,
            event_hash,
            projection_bytes,
            projection_hash,
        }
    }
}

// Historical entrypoints always choose Historical. No complete Target replay
// entry is exposed before account/codec/adjudication and all other reads close.
#[allow(dead_code)]
enum V1ReadSource<'a, 'loan> {
    Historical(&'a mut SqliteConnection),
    Bounded(&'a mut crate::database::global_schema_v1::replay_work::V1RowsLoan<'loan>),
}
#[allow(dead_code)]
enum V1RowsError {
    Historical(LedgerError),
    Terminal(crate::database::global_schema_v1::replay_work::ReplayTerminalFailure),
}
impl V1ReadSource<'_, '_> {
    fn events(&mut self, id: &str) -> Result<Vec<EventRow>, V1RowsError> {
        match self {
            Self::Historical(conn) => events_historical(conn, id).map_err(V1RowsError::Historical),
            Self::Bounded(loan) => loan.events(id).map_err(V1RowsError::Terminal),
        }
    }
    fn head(&mut self, binding: &AccountBinding) -> Result<Option<HeadRow>, V1RowsError> {
        match self {
            Self::Historical(conn) => {
                head_historical(conn, binding).map_err(V1RowsError::Historical)
            }
            Self::Bounded(loan) => loan
                .head(&binding.account_id)
                .map_err(V1RowsError::Terminal),
        }
    }
}

fn account(conn: &mut SqliteConnection, id: &str) -> Result<Option<AccountRow>, LedgerError> {
    Ok(diesel::sql_query(
        "SELECT epoch_id,manifest_hash,manifest_bytes FROM paper_ledger_account WHERE account_id=?",
    )
    .bind::<Text, _>(id)
    .get_result(conn)
    .optional()?)
}
fn events(conn: &mut SqliteConnection, id: &str) -> Result<Vec<EventRow>, LedgerError> {
    match V1ReadSource::Historical(conn).events(id) {
        Ok(rows) => Ok(rows),
        Err(V1RowsError::Historical(error)) => Err(error),
        Err(V1RowsError::Terminal(_)) => unreachable!("Historical never borrows Target work"),
    }
}
fn events_historical(conn: &mut SqliteConnection, id: &str) -> Result<Vec<EventRow>, LedgerError> {
    Ok(diesel::sql_query("SELECT seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id FROM paper_ledger_event WHERE account_id=? ORDER BY seq").bind::<Text,_>(id).load(conn)?)
}
fn event_hash(
    account_id: &str,
    seq: i64,
    command: &str,
    previous: &str,
    payload: &str,
) -> Result<String, LedgerError> {
    Ok(digest(&encode(&(
        "PAPER_EVENT_V1",
        account_id,
        seq,
        command,
        previous,
        payload,
    ))?))
}
fn replay(conn: &mut SqliteConnection, binding: &AccountBinding) -> Result<PaperView, LedgerError> {
    replay_through(conn, binding, None)
}
fn replay_through(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    until: Option<i64>,
) -> Result<PaperView, LedgerError> {
    replay_through_inner(conn, binding, until, false, None)
}

fn replay_through_inner(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    until: Option<i64>,
    catalog_already_verified: bool,
    mut audit_guard: Option<&mut V1AuditReplayGuard>,
) -> Result<PaperView, LedgerError> {
    let account = account(conn, &binding.account_id)?.ok_or(LedgerError::NotSeeded)?;
    let manifest: SeedManifest = decode(&account.manifest_bytes)?;
    if account.epoch_id != binding.epoch_id
        || account.manifest_hash != binding.manifest_hash
        || manifest.binding()? != *binding
    {
        return Err(LedgerError::InactiveEpoch);
    }
    let mut previous = GENESIS.to_string();
    let mut state = None;
    let mut version = 0;
    for row in events(conn, &binding.account_id)? {
        if until.is_some_and(|version| row.seq > version) {
            break;
        }
        version += 1;
        if row.seq != version
            || row.previous_hash != previous
            || row.event_hash
                != event_hash(
                    &binding.account_id,
                    row.seq,
                    &row.command_id,
                    &previous,
                    &row.payload,
                )?
        {
            return Err(LedgerError::IntegrityFailure("event chain mismatch".into()));
        }
        let fact: Fact = decode(&row.payload)?;
        if metadata(&fact)
            != (
                row.business_plan_id,
                row.intent_hash,
                row.is_terminal,
                row.paper_trade_id,
                row.order_audit_id,
            )
        {
            return Err(LedgerError::IntegrityFailure(
                "event index/receipt metadata mismatch".into(),
            ));
        }
        if let Fact::AdjudicatedV1(ruling) = &fact {
            adjudication::verify_ruling(
                conn,
                binding,
                row.seq,
                &row.previous_hash,
                ruling,
                catalog_already_verified,
                audit_guard.as_deref_mut(),
                state
                    .as_ref()
                    .ok_or_else(|| LedgerError::IntegrityFailure("ruling before genesis".into()))?,
            )?;
        }
        match fact {
            Fact::Seeded { manifest: seed, .. } if version == 1 && seed == manifest => {
                state = Some(seed_projection(&seed)?)
            }
            fact if version > 1 => apply_fact(
                state
                    .as_mut()
                    .ok_or_else(|| LedgerError::IntegrityFailure("missing genesis".into()))?,
                &fact,
            )?,
            _ => {
                return Err(LedgerError::IntegrityFailure(
                    "invalid genesis/event order".into(),
                ))
            }
        }
        previous = row.event_hash;
    }
    let projection =
        state.ok_or_else(|| LedgerError::IntegrityFailure("missing genesis".into()))?;
    Ok(PaperView {
        version,
        event_hash: previous,
        projection,
    })
}
fn head(conn: &mut SqliteConnection, binding: &AccountBinding) -> Result<Option<HeadRow>, LedgerError> {
    match V1ReadSource::Historical(conn).head(binding) {
        Ok(row) => Ok(row),
        Err(V1RowsError::Historical(error)) => Err(error),
        Err(V1RowsError::Terminal(_)) => unreachable!("Historical never borrows Target work"),
    }
}
fn head_historical(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
) -> Result<Option<HeadRow>, LedgerError> {
    Ok(diesel::sql_query("SELECT version,event_hash,projection_bytes,projection_hash FROM paper_ledger_head WHERE account_id=?").bind::<Text,_>(&binding.account_id).get_result::<HeadRow>(conn).optional()?)
}
fn load(conn: &mut SqliteConnection, binding: &AccountBinding) -> Result<PaperView, LedgerError> {
    load_inner(conn, binding, false)
}

fn load_inner(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    catalog_already_verified: bool,
) -> Result<PaperView, LedgerError> {
    load_inner_with_audit_guard(conn, binding, catalog_already_verified, None)
}

fn load_inner_with_audit_guard(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    catalog_already_verified: bool,
    audit_guard: Option<&mut V1AuditReplayGuard>,
) -> Result<PaperView, LedgerError> {
    let view = replay_through_inner(conn, binding, None, catalog_already_verified, audit_guard)?;
    let head = head(conn, binding)?.ok_or_else(|| {
        LedgerError::IntegrityFailure("missing projection; explicit repair required".into())
    })?;
    if head.version != view.version
        || head.event_hash != view.event_hash
        || head.projection_hash != digest(&head.projection_bytes)
        || head.projection_bytes != encode(&view.projection)?
    {
        return Err(LedgerError::IntegrityFailure(
            "projection/head mismatch".into(),
        ));
    }
    Ok(view)
}

/// Replays V1 and compares the exact persisted head and economic projection.
/// V2 cutover and V2Active read verification use this without reopening a
/// connection or granting V1 write authority. Its caller must first verify
/// the catalog on this same connection; that check is omitted from nested
/// adjudication replay to avoid recursively entering the V2 owner verifier.
pub(crate) fn verified_v1_snapshot_on(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
) -> Result<VerifiedV1Snapshot, LedgerError> {
    conn.transaction(|conn| {
        verified_v1_snapshot_with_audit_guard_on(
            conn,
            binding,
            &mut V1AuditReplayGuard::default(),
        )
    })
}

/// Caller owns one transaction covering every reuse of `audit_guard`.
pub(crate) fn verified_v1_snapshot_with_audit_guard_on(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    audit_guard: &mut V1AuditReplayGuard,
) -> Result<VerifiedV1Snapshot, LedgerError> {
    let view = load_inner_with_audit_guard(conn, binding, true, Some(audit_guard))?;
    view.require_available()?;
    let equity = view.equity()?;
    let stored = head(conn, binding)?
        .ok_or_else(|| LedgerError::IntegrityFailure("missing V1 projection head".into()))?;
    Ok(VerifiedV1Snapshot {
        version: view.version,
        event_hash: view.event_hash,
        projection_bytes: stored.projection_bytes,
        projection_hash: stored.projection_hash,
        equity,
    })
}

/// Full original V1 observation after the caller has verified the complete
/// CatalogV6 and every original owner/genesis row on this same snapshot.
/// This body never grants V1 write or approval authority.
pub(crate) fn read_verified_original_v1_body_on(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
) -> Result<PaperView, LedgerError> {
    let view = load_inner(conn, binding, true)?;
    view.require_available()?;
    Ok(view)
}

pub(crate) struct VerifiedV1Snapshot {
    pub(crate) version: i64,
    pub(crate) event_hash: String,
    pub(crate) projection_bytes: String,
    pub(crate) projection_hash: String,
    pub(crate) equity: Money,
}

fn append(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    command_id: &str,
    before: &PaperView,
    fact: Fact,
) -> Result<PaperReceipt, LedgerError> {
    if command_id.trim().is_empty() {
        return Err(LedgerError::InvalidInput("empty command identity".into()));
    }
    let mut next = before.projection.clone();
    apply_fact(&mut next, &fact)?;
    let version = before.version.checked_add(1).ok_or(LedgerError::Overflow)?;
    let payload = encode(&fact)?;
    let hash = event_hash(
        &binding.account_id,
        version,
        command_id,
        &before.event_hash,
        &payload,
    )?;
    let (plan, intent_hash, terminal, trade_id, audit_id) = metadata(&fact);
    diesel::sql_query("INSERT INTO paper_ledger_event(account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id) VALUES (?,?,?,?,?,?,?,?,?,?,?)")
        .bind::<Text,_>(&binding.account_id).bind::<BigInt,_>(version).bind::<Text,_>(command_id).bind::<Text,_>(&before.event_hash).bind::<Text,_>(&hash).bind::<Text,_>(payload)
        .bind::<diesel::sql_types::Nullable<Text>,_>(plan).bind::<diesel::sql_types::Nullable<Text>,_>(intent_hash).bind::<BigInt,_>(terminal)
        .bind::<diesel::sql_types::Nullable<BigInt>,_>(trade_id).bind::<diesel::sql_types::Nullable<BigInt>,_>(audit_id).execute(conn)?;
    let bytes = encode(&next)?;
    let rows = diesel::sql_query("UPDATE paper_ledger_head SET version=?,event_hash=?,projection_bytes=?,projection_hash=? WHERE account_id=? AND version=? AND event_hash=?")
        .bind::<BigInt,_>(version).bind::<Text,_>(&hash).bind::<Text,_>(&bytes).bind::<Text,_>(digest(&bytes))
        .bind::<Text,_>(&binding.account_id).bind::<BigInt,_>(before.version).bind::<Text,_>(&before.event_hash).execute(conn)?;
    if rows != 1 {
        return Err(LedgerError::VersionChanged);
    }
    Ok(receipt(version, hash, &fact, false))
}

fn metadata(
    fact: &Fact,
) -> (
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
    Option<i64>,
) {
    match fact {
        Fact::Order(order) => (
            Some(order.plan_id.clone()),
            Some(order.intent_hash.clone()),
            i64::from(order.status != LedgerStatus::Rejected),
            order.paper_trade_id,
            Some(order.audit.id),
        ),
        _ => (None, None, 0, None, None),
    }
}

/// Called only after load has verified the entire bound chain. Evidence such as
/// new quote timestamps/version never turns an existing business fill into a retry.
fn replay_command(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    command: Option<&str>,
    plan: Option<(&str, &str)>,
    mark: Option<&ValuationBatch>,
) -> Result<Option<PaperReceipt>, LedgerError> {
    for row in events(conn, &binding.account_id)? {
        let fact: Fact = decode(&row.payload)?;
        let same_command = command.is_some_and(|command| row.command_id == command);
        match &fact {
            Fact::Order(order)
                if same_command || plan.is_some_and(|(id, _)| order.plan_id == id) =>
            {
                let Some((id, hash)) = plan else {
                    return Err(LedgerError::IdentityConflict);
                };
                if order.plan_id != id || order.intent_hash != hash {
                    return Err(LedgerError::IdentityConflict);
                }
                if same_command || order.status != LedgerStatus::Rejected {
                    return Ok(Some(receipt(row.seq, row.event_hash, &fact, true)));
                }
            }
            Fact::Marked(original) if same_command => {
                let Some(mark) = mark else {
                    return Err(LedgerError::IdentityConflict);
                };
                if encode(&(original.as_of, original.closing, &original.marks))?
                    != encode(&(mark.as_of, mark.closing, &mark.marks))?
                {
                    return Err(LedgerError::IdentityConflict);
                }
                return Ok(Some(receipt(row.seq, row.event_hash, &fact, true)));
            }
            _ if same_command => return Err(LedgerError::IdentityConflict),
            _ => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "paper_ledger_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn test_historical_v1_sql_rows(
    conn: &mut SqliteConnection,
    id: &str,
) -> (Vec<EventRow>, Option<HeadRow>) {
    let binding = AccountBinding {
        account_id: id.to_owned(),
        epoch_id: String::new(),
        manifest_hash: String::new(),
    };
    (events(conn, id).unwrap(), head(conn, &binding).unwrap())
}
