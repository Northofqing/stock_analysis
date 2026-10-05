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
use crate::trading::paper_replay_financial_work_v1::{self as fw, FinancialWork, FinancialFailure, ClosedFinancialText as Txt};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
#[path = "paper_ledger_execution.rs"]
mod execution;
use execution::apply_fact;
pub(crate) use execution::OrderFact;
pub use execution::{ExecuteIntent, PriceIntent, ValuationBatch};
#[path = "paper_ledger_adjudication.rs"]
mod adjudication;
pub(crate) use adjudication::{
    parse_raw_timestamp_with_work,
    raw_decision_prefix_with_work,
    raw_contradiction_with_work,
    AdjudText,
    RecomputeFill,
    RecomputeMarket,
    sort_recompute_fills_owner,
    sort_recompute_markets_owner
};
pub use adjudication::{
    AccountProjectionImpact, Adjudication, AdjudicationAction, AdjudicationPreview,
    FillFingerprint, HistoricalProjectionImpact,
};
#[path = "paper_effective_fills.rs"]
mod effective;
pub(crate) use effective::{
    copy_economic_fill,
    OrderedEconomic,
    sort_economic_owner
};
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
        fw::historical(self.binding_with_work(&mut FinancialWork::Historical))
    }
    pub(crate) fn binding_with_work(&self, work: &mut FinancialWork<'_, '_>) -> fw::Result<AccountBinding> {
        Ok(AccountBinding {
            account_id: work.copy(&self.account_id)?,
            epoch_id: work.copy(&self.epoch_id)?,
            manifest_hash: work.seed_binding_hash(self)?,
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
    pub(crate) fn replay_unavailable_reason(&self)->Option<&str>{
        self.economic_unavailable.as_deref()
    }

    fn require_available(&self) -> Result<(), LedgerError> {
        fw::historical(self.require_available_with_work(&mut FinancialWork::Historical))
    }
    fn require_available_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
        w.finish()?;
        if self.economic_unavailable.is_some(){
            return Err(LedgerError::EvidenceUnavailable(w.text(Txt::EconomicUnavailable(self))?).into());
        }
        Ok(())
    }
    pub fn equity(&self) -> Result<Money, LedgerError> {
        fw::historical(self.equity_with_work(&mut FinancialWork::Historical))
    }
    pub(crate) fn equity_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<Money> {
        w.finish()?;
        self.lots.iter().try_fold(self.cash, |equity, lot| {
            let mark=w.option(self.marks.get(&lot.code), Txt::MissingMark(lot))?;
            Ok(equity.add(mark.price.mul(lot.quantity)?)?)
        })
    }
    pub fn daily_pnl(&self) -> Option<Money> {
        let previous = crate::calendar::verified_prev_a_share_trading_day(day(self.as_of)).ok()?;
        self.equity().ok()?.sub(*self.closes.get(&previous)?).ok()
    }
    pub fn inventory_fingerprint(&self) -> Result<String, LedgerError> {
        fw::historical(self.inventory_fingerprint_with_work(&mut FinancialWork::Historical))
    }
    pub(crate) fn inventory_fingerprint_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<String> {
        w.finish()?;
        w.fixed_hash(fw::ClosedFinancialHash::Inventory(&self.lots))
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
    fw::historical(seed_projection_with_work(seed, &mut FinancialWork::Historical))
}
pub(crate) fn seed_projection_with_work(seed: &SeedManifest, w: &mut FinancialWork<'_, '_>) -> fw::Result<Projection> {
    w.finish()?;
    if [ &seed.account_id, &seed.epoch_id, &seed.command_id, &seed.source_reference, &seed.approved_by, ] .iter() .any(|v| v.trim().is_empty()) || seed.source_hash.len() != 64 || !seed.source_hash.bytes().all(|c| c.is_ascii_hexdigit()) || seed.account_effective_at != seed.positions_effective_at || seed.account_effective_at != seed.cutover_at || seed.cash < Money::ZERO || seed .excluded_residual .is_some_and(|residual| residual < Money::ZERO) || seed.policy.max_position_bps > 10000 || seed.policy.cash_floor_bps > 10000 {
        return Err(w.error(Txt::Seed(fw::SeedText::InvalidSeedIdentityEffectiveTimeOrPolicy))?);
    }
    let next = w.calendar_next(day(seed.cutover_at))?;
    let mut marks = BTreeMap::new();
    for mark in &seed.marks {
        if mark.price <= Money::ZERO || mark.observed_at != seed.cutover_at || mark.source.trim().is_empty() || {
            let key=w.copy(&mark.code)?;
            let value=w.copy(mark)?;
            let duplicate=marks.contains_key(&key);
            w.insert(&mut marks, key, value)?;
            duplicate
        }
        {
            return Err(w.error(Txt::Seed(fw::SeedText::InvalidSeedMark))?);
        }
    }
    let mut lots = Vec::new();
    for (i, lot) in seed.lots.iter().enumerate() {
        if lot.quantity == 0 || !lot.quantity.is_multiple_of(100) || lot.code.trim().is_empty() {
            return Err(w.error(Txt::Seed(fw::SeedText::InvalidSeedLot))?);
        }
        let mark = w.option(marks.get(&lot.code), Txt::Seed(fw::SeedText::SeedLotMarkMissing))?;
        let sellable_from = match lot.sellable_from {
            Some(date) if lot .sellability_evidence .as_ref() .is_some_and(|e| !e.trim().is_empty()) => {
                date
            }
            None => next,
            _ => {
                return Err(w.error(Txt::Seed(fw::SeedText::ExplicitSellabilityLacksEvidence))?)
            }
        };
        let incoming=Lot {
            lot_id: w.text(Txt::SeedLotOrdinal(i))?,
            code: w.copy(&lot.code)?,
            name: w.copy(&lot.name)?,
            quantity: lot.quantity,
            basis_price: mark.price,
            buy_fee_remaining: Money::ZERO,
            acquired_on: day(seed.cutover_at),
            sellable_from,
            reported_cost: lot.reported_cost,
        };
        w.push(&mut lots, incoming)?;
    }
    let mut codes=std::collections::BTreeSet::new();
    for lot in &lots {
        w.set(&mut codes, lot.code.as_str())?;
    }
    if marks.len()!=codes.len() {
        return Err(w.error(Txt::Seed(fw::SeedText::SeedMarkCoverageMismatch))?);
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
    projection.seed_equity = projection.equity_with_work(w)?;
    if projection.seed_equity <= Money::ZERO || seed.original_total.sub(projection.seed_equity)? != seed.excluded_residual.unwrap_or(Money::ZERO) {
        return Err(w.error(Txt::Seed(fw::SeedText::UnapprovedSeedResidualEmptyEquity))?);
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
    let mut work = FinancialWork::Historical;
    let mut cursor = fw::historical(start_original_replay(binding, &account, &mut work))?;
    for row in events(conn, &binding.account_id)? {
        if until.is_some_and(|version| row.seq > version) {
            break;
        }
        let pending = fw::historical(validate_next_original_event(binding, &mut cursor, row, &mut work))?;
        if let Fact::AdjudicatedV1(ruling) = &pending.fact {
            adjudication::verify_ruling(
                conn, binding, pending.version, &cursor.previous, ruling,
                catalog_already_verified, audit_guard.as_deref_mut(),
                cursor.state.as_ref().ok_or_else(|| LedgerError::IntegrityFailure("ruling before genesis".into()))?,
            )?;
        }
        fw::historical(apply_original_pending(&mut cursor, pending, &mut work))?;
    }
    fw::historical(finish_original_events(cursor, &mut work))
}

struct OriginalReplayCursor {
    manifest: SeedManifest,
    previous: String,
    state: Option<Projection>,
    version: i64,
}
struct OriginalPending {
    fact: Fact,
    row: EventRow,
    version: i64,
}
fn start_original_replay(binding: &AccountBinding, account: &AccountRow, work: &mut FinancialWork<'_, '_>) -> fw::Result<OriginalReplayCursor> {
    let manifest: SeedManifest = work.decode(account.manifest_bytes.as_bytes())?;
    if account.epoch_id != binding.epoch_id || account.manifest_hash != binding.manifest_hash || manifest.binding_with_work(work)? != *binding {
        return Err(LedgerError::InactiveEpoch.into());
    }
    Ok(OriginalReplayCursor {
        manifest, previous: work.history_text(fw::HistoryText::Ledger(LedgerHistoryText::Genesis))?, state: None, version: 0
    })
}
fn validate_next_original_event(binding: &AccountBinding, cursor: &mut OriginalReplayCursor, mut row: EventRow, work: &mut FinancialWork<'_, '_>) -> fw::Result<OriginalPending> {
    cursor.version += 1;
    if !raw_v1_event_link_matches(row.seq, cursor.version, &row.previous_hash, Some(&cursor.previous)) || row.event_hash != work.history_hash(crate::trading::paper_replay_codec_v1::HistoryOutput::Event {
        account: &binding.account_id, seq: row.seq, command: &row.command_id, previous: &cursor.previous, payload: &row.payload,
    })? {
        return Err(ledger_history_error(work, LedgerHistoryText::EventChain)?);
    }
    let fact: Fact = work.decode(row.payload.as_bytes())?;
    let indexed = match &fact {
        Fact::Order(order) => (Some(work.copy(&order.plan_id)?), Some(work.copy(&order.intent_hash)?), i64::from(order.status != LedgerStatus::Rejected), order.paper_trade_id, Some(order.audit.id)),
        _ => (None, None, 0, None, None),
    };
    if indexed != (row.business_plan_id.take(), row.intent_hash.take(), row.is_terminal, row.paper_trade_id, row.order_audit_id) {
        return Err(ledger_history_error(work, LedgerHistoryText::Metadata)?);
    }
    Ok(OriginalPending {
        fact, row, version: cursor.version
    })
}
// Private to this owner: its SQL orchestrator verifies a ruling between the
// pending step and this application. No Target/capability entry is introduced.
fn apply_original_pending(cursor: &mut OriginalReplayCursor, pending: OriginalPending, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    match pending.fact {
        Fact::Seeded {
            manifest: seed,
            ..
        }
        if pending.version == 1 && seed == cursor.manifest => {
            cursor.state = Some(seed_projection_with_work(&seed, work)?);
        }
        fact if pending.version > 1 => {
            let state = match cursor.state.as_mut() {
                Some(state) => state,
                None => return Err(ledger_history_error(work, LedgerHistoryText::MissingGenesis)?),
            };
            execution::apply_fact_with_work(state, &fact, work)?;
        }
        _ => return Err(ledger_history_error(work, LedgerHistoryText::GenesisOrder)?),
    }
    cursor.previous = pending.row.event_hash;
    Ok(())
}
fn finish_original_events(cursor: OriginalReplayCursor, work: &mut FinancialWork<'_, '_>) -> fw::Result<PaperView> {
    let projection = match cursor.state {
        Some(state) => state,
        None => return Err(ledger_history_error(work, LedgerHistoryText::MissingGenesis)?),
    };
    Ok(PaperView {
        version: cursor.version, event_hash: cursor.previous, projection
    })
}
#[derive(Clone, Copy)]
pub(crate) enum LedgerHistoryText {
    Genesis, EventChain, Metadata, MissingGenesis, GenesisOrder, Snapshot, HeadMismatch, LegacySeed,
}
impl LedgerHistoryText {
    pub(crate) fn write(self, out: &mut fw::FinancialSink<'_>) -> Result<(), ()> {
        out.bytes(match self {
            Self::Genesis => GENESIS.as_bytes(),
            Self::EventChain => b"event chain mismatch",
            Self::Metadata => b"event index/receipt metadata mismatch",
            Self::MissingGenesis => b"missing genesis",
            Self::GenesisOrder => b"invalid genesis/event order",
            Self::Snapshot => b"unknown/invalid derived snapshot payload",
            Self::HeadMismatch => b"projection/head mismatch",
            Self::LegacySeed => b"legacy prefix lacks seed",
        })
    }
}
fn ledger_history_error(work: &mut FinancialWork<'_, '_>, text: LedgerHistoryText) -> fw::Result<FinancialFailure> {
    Ok(LedgerError::IntegrityFailure(work.history_text(fw::HistoryText::Ledger(text))?).into())
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
    fw::historical(compare_original_head(&view, &head, &mut FinancialWork::Historical))?;
    Ok(view)
}

fn compare_original_head(view: &PaperView, head: &HeadRow, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    if !raw_v1_head_link_matches(head.version, &head.event_hash, view.version, &view.event_hash) || head.projection_hash != work.raw_hash(head.projection_bytes.as_bytes())? || !work.canonical_equal(&view.projection, head.projection_bytes.as_bytes())? {
        return Err(ledger_history_error(work, LedgerHistoryText::HeadMismatch)?);
    }
    Ok(())
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

// Finite replay DTO seeds stay with the owners of private fields.
#[allow(dead_code, non_camel_case_types)]
mod replay_codec_owner {
    use super::adjudication::AdjudicatedFact;
    use super::execution::LotChange;
    use super::*;
    use crate::performance::snapshot::PerformanceSnapshot;
    use crate::trading::paper_replay_codec_v1 as c;
    use crate::trading::paper_replay_shapes_v1 as s;
    use serde::de::{EnumAccess as _, VariantAccess as _};
    impl c::sealed::Value for RiskPolicyV1 {}
    impl c::Value for RiskPolicyV1 {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "max_position_bps",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cash_floor_bps",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "max_slippage_bps",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,RiskPolicyV1,true,{max_position_bps:u32=>false,cash_floor_bps:u32=>false,max_slippage_bps:u32=>false},RiskPolicyV1{max_position_bps,cash_floor_bps,max_slippage_bps})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(RiskPolicyV1 {
                max_position_bps: c::Value::paid_copy(&self.max_position_bps, w)?,
                cash_floor_bps: c::Value::paid_copy(&self.cash_floor_bps, w)?,
                max_slippage_bps: c::Value::paid_copy(&self.max_slippage_bps, w)?,
            })
        }
    }
    impl c::sealed::Value for Mark {}
    impl c::Value for Mark {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "code",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "price",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "observed_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source",
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
            c::record_read!(de,input,Mark,true,{code:String=>false,price:Money=>false,observed_at:DateTime<Utc> =>false,source:String=>false},Mark{code,price,observed_at,source})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(Mark {
                code: c::Value::paid_copy(&self.code, w)?,
                price: c::Value::paid_copy(&self.price, w)?,
                observed_at: c::Value::paid_copy(&self.observed_at, w)?,
                source: c::Value::paid_copy(&self.source, w)?,
            })
        }
    }
    impl c::sealed::Value for SeedLot {}
    impl c::Value for SeedLot {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "code",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "name",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "reported_cost",
                    shape: &<Option<Money> as c::Value>::SHAPE,
                    optional: <Option<Money> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sellable_from",
                    shape: &<Option<NaiveDate> as c::Value>::SHAPE,
                    optional: <Option<NaiveDate> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sellability_evidence",
                    shape: &<Option<String> as c::Value>::SHAPE,
                    optional: <Option<String> as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,SeedLot,true,{code:String=>false,name:String=>false,quantity:u32=>false,reported_cost:Option<Money> =>false,sellable_from:Option<NaiveDate> =>false,sellability_evidence:Option<String> =>false},SeedLot{code,name,quantity,reported_cost,sellable_from,sellability_evidence})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(SeedLot {
                code: c::Value::paid_copy(&self.code, w)?,
                name: c::Value::paid_copy(&self.name, w)?,
                quantity: c::Value::paid_copy(&self.quantity, w)?,
                reported_cost: c::Value::paid_copy(&self.reported_cost, w)?,
                sellable_from: c::Value::paid_copy(&self.sellable_from, w)?,
                sellability_evidence: c::Value::paid_copy(&self.sellability_evidence, w)?,
            })
        }
    }
    impl c::sealed::Value for SeedManifest {}
    impl c::Value for SeedManifest {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
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
                    name: "command_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cutover_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "account_effective_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "positions_effective_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source_reference",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "approved_by",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cash",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "original_total",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "excluded_residual",
                    shape: &<Option<Money> as c::Value>::SHAPE,
                    optional: <Option<Money> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "lots",
                    shape: &<Vec<SeedLot> as c::Value>::SHAPE,
                    optional: <Vec<SeedLot> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "marks",
                    shape: &<Vec<Mark> as c::Value>::SHAPE,
                    optional: <Vec<Mark> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "policy",
                    shape: &<RiskPolicyV1 as c::Value>::SHAPE,
                    optional: <RiskPolicyV1 as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,SeedManifest,true,{account_id:String=>false,epoch_id:String=>false,command_id:String=>false,cutover_at:DateTime<Utc> =>false,account_effective_at:DateTime<Utc> =>false,positions_effective_at:DateTime<Utc> =>false,source_reference:String=>false,source_hash:String=>false,approved_by:String=>false,cash:Money=>false,original_total:Money=>false,excluded_residual:Option<Money> =>false,lots:Vec<SeedLot> =>false,marks:Vec<Mark> =>false,policy:RiskPolicyV1=>false},SeedManifest{account_id,epoch_id,command_id,cutover_at,account_effective_at,positions_effective_at,source_reference,source_hash,approved_by,cash,original_total,excluded_residual,lots,marks,policy})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(SeedManifest {
                account_id: c::Value::paid_copy(&self.account_id, w)?,
                epoch_id: c::Value::paid_copy(&self.epoch_id, w)?,
                command_id: c::Value::paid_copy(&self.command_id, w)?,
                cutover_at: c::Value::paid_copy(&self.cutover_at, w)?,
                account_effective_at: c::Value::paid_copy(&self.account_effective_at, w)?,
                positions_effective_at: c::Value::paid_copy(&self.positions_effective_at, w)?,
                source_reference: c::Value::paid_copy(&self.source_reference, w)?,
                source_hash: c::Value::paid_copy(&self.source_hash, w)?,
                approved_by: c::Value::paid_copy(&self.approved_by, w)?,
                cash: c::Value::paid_copy(&self.cash, w)?,
                original_total: c::Value::paid_copy(&self.original_total, w)?,
                excluded_residual: c::Value::paid_copy(&self.excluded_residual, w)?,
                lots: c::Value::paid_copy(&self.lots, w)?,
                marks: c::Value::paid_copy(&self.marks, w)?,
                policy: c::Value::paid_copy(&self.policy, w)?,
            })
        }
    }
    impl c::sealed::Value for AccountBinding {}
    impl c::Value for AccountBinding {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
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
                    name: "manifest_hash",
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
            c::record_read!(de,input,AccountBinding,true,{account_id:String=>false,epoch_id:String=>false,manifest_hash:String=>false},AccountBinding{account_id,epoch_id,manifest_hash})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(AccountBinding {
                account_id: c::Value::paid_copy(&self.account_id, w)?,
                epoch_id: c::Value::paid_copy(&self.epoch_id, w)?,
                manifest_hash: c::Value::paid_copy(&self.manifest_hash, w)?,
            })
        }
    }
    impl c::sealed::Value for Lot {}
    impl c::Value for Lot {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "lot_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "code",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "name",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "basis_price",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "buy_fee_remaining",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "acquired_on",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sellable_from",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "reported_cost",
                    shape: &<Option<Money> as c::Value>::SHAPE,
                    optional: <Option<Money> as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,Lot,true,{lot_id:String=>false,code:String=>false,name:String=>false,quantity:u32=>false,basis_price:Money=>false,buy_fee_remaining:Money=>false,acquired_on:NaiveDate=>false,sellable_from:NaiveDate=>false,reported_cost:Option<Money> =>false},Lot{lot_id,code,name,quantity,basis_price,buy_fee_remaining,acquired_on,sellable_from,reported_cost})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(Lot {
                lot_id: c::Value::paid_copy(&self.lot_id, w)?,
                code: c::Value::paid_copy(&self.code, w)?,
                name: c::Value::paid_copy(&self.name, w)?,
                quantity: c::Value::paid_copy(&self.quantity, w)?,
                basis_price: c::Value::paid_copy(&self.basis_price, w)?,
                buy_fee_remaining: c::Value::paid_copy(&self.buy_fee_remaining, w)?,
                acquired_on: c::Value::paid_copy(&self.acquired_on, w)?,
                sellable_from: c::Value::paid_copy(&self.sellable_from, w)?,
                reported_cost: c::Value::paid_copy(&self.reported_cost, w)?,
            })
        }
    }
    impl c::sealed::Value for Projection {}
    impl c::Value for Projection {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "cash",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "lots",
                    shape: &<Vec<Lot> as c::Value>::SHAPE,
                    optional: <Vec<Lot> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "marks",
                    shape: &<BTreeMap<String, Mark> as c::Value>::SHAPE,
                    optional: <BTreeMap<String, Mark> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fees",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "realized_pnl",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "seed_equity",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "as_of",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "closes",
                    shape: &<BTreeMap<NaiveDate, Money> as c::Value>::SHAPE,
                    optional: <BTreeMap<NaiveDate, Money> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "economic_unavailable",
                    shape: &<Option<String> as c::Value>::SHAPE,
                    optional: <Option<String> as c::Value>::OPTIONAL,
                    positional_default: true,
                },
            ],
            false,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,Projection,true,{cash:Money=>false,lots:Vec<Lot> =>false,marks:BTreeMap<String, Mark> =>false,fees:Money=>false,realized_pnl:Money=>false,seed_equity:Money=>false,as_of:DateTime<Utc> =>false,closes:BTreeMap<NaiveDate, Money> =>false,economic_unavailable:Option<String> =>true},Projection{cash,lots,marks,fees,realized_pnl,seed_equity,as_of,closes,economic_unavailable})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(Projection {
                cash: c::Value::paid_copy(&self.cash, w)?,
                lots: c::Value::paid_copy(&self.lots, w)?,
                marks: c::Value::paid_copy(&self.marks, w)?,
                fees: c::Value::paid_copy(&self.fees, w)?,
                realized_pnl: c::Value::paid_copy(&self.realized_pnl, w)?,
                seed_equity: c::Value::paid_copy(&self.seed_equity, w)?,
                as_of: c::Value::paid_copy(&self.as_of, w)?,
                closes: c::Value::paid_copy(&self.closes, w)?,
                economic_unavailable: c::Value::paid_copy(&self.economic_unavailable, w)?,
            })
        }
    }
    impl c::sealed::Value for LedgerStatus {}
    impl c::Value for LedgerStatus {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "Seeded",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Marked",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Filled",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "NotFilled",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Invalidated",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Rejected",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Adjudicated",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "SnapshotRecorded",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Seeded<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Seeded<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::Seeded)
                }
            }
            struct Seed_Marked<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Marked<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::Marked)
                }
            }
            struct Seed_Filled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Filled<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::Filled)
                }
            }
            struct Seed_NotFilled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_NotFilled<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::NotFilled)
                }
            }
            struct Seed_Invalidated<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Invalidated<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::Invalidated)
                }
            }
            struct Seed_Rejected<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Rejected<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::Rejected)
                }
            }
            struct Seed_Adjudicated<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Adjudicated<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::Adjudicated)
                }
            }
            struct Seed_SnapshotRecorded<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_SnapshotRecorded<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LedgerStatus::SnapshotRecorded)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = LedgerStatus;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &[
                            "Seeded",
                            "Marked",
                            "Filled",
                            "NotFilled",
                            "Invalidated",
                            "Rejected",
                            "Adjudicated",
                            "SnapshotRecorded",
                        ],
                    })?;
                    let object = self.0.bytes[self.0.span.start] == b'{';
                    let span = if object {
                        self.0
                            .span
                            .children(self.0.bytes)
                            .next()
                            .ok_or_else(c::span_error)?
                            .1
                    } else {
                        self.0.span
                    };
                    let origin = self.0.origin;
                    match tag {
                        "Seeded" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::Seeded)
                        }
                        "Marked" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::Marked)
                        }
                        "Filled" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::Filled)
                        }
                        "NotFilled" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::NotFilled)
                        }
                        "Invalidated" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::Invalidated)
                        }
                        "Rejected" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::Rejected)
                        }
                        "Adjudicated" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::Adjudicated)
                        }
                        "SnapshotRecorded" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LedgerStatus::SnapshotRecorded)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &[
                    "Seeded",
                    "Marked",
                    "Filled",
                    "NotFilled",
                    "Invalidated",
                    "Rejected",
                    "Adjudicated",
                    "SnapshotRecorded",
                ],
                EV(input),
            )
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                LedgerStatus::Seeded => LedgerStatus::Seeded,
                LedgerStatus::Marked => LedgerStatus::Marked,
                LedgerStatus::Filled => LedgerStatus::Filled,
                LedgerStatus::NotFilled => LedgerStatus::NotFilled,
                LedgerStatus::Invalidated => LedgerStatus::Invalidated,
                LedgerStatus::Rejected => LedgerStatus::Rejected,
                LedgerStatus::Adjudicated => LedgerStatus::Adjudicated,
                LedgerStatus::SnapshotRecorded => LedgerStatus::SnapshotRecorded,
            })
        }
    }
    impl c::sealed::Value for AuditLink {}
    impl c::Value for AuditLink {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "id",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "previous_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "record_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "created_at",
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
            c::record_read!(de,input,AuditLink,true,{id:i64=>false,previous_hash:String=>false,record_hash:String=>false,created_at:String=>false},AuditLink{id,previous_hash,record_hash,created_at})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(AuditLink {
                id: c::Value::paid_copy(&self.id, w)?,
                previous_hash: c::Value::paid_copy(&self.previous_hash, w)?,
                record_hash: c::Value::paid_copy(&self.record_hash, w)?,
                created_at: c::Value::paid_copy(&self.created_at, w)?,
            })
        }
    }
    impl c::sealed::Value for Fact {}
    impl c::Value for Fact {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "Seeded",
                body: s::Body::Record(
                    &[
                        s::Field {
                            name: "manifest",
                            shape: &<SeedManifest as c::Value>::SHAPE,
                            optional: <SeedManifest as c::Value>::OPTIONAL,
                            positional_default: false,
                        },
                        s::Field {
                            name: "legacy_high_water_id",
                            shape: &<i64 as c::Value>::SHAPE,
                            optional: <i64 as c::Value>::OPTIONAL,
                            positional_default: false,
                        },
                        s::Field {
                            name: "legacy_audit_high_water",
                            shape: &<String as c::Value>::SHAPE,
                            optional: <String as c::Value>::OPTIONAL,
                            positional_default: false,
                        },
                    ],
                    false,
                ),
            },
            s::Variant {
                name: "Order",
                body: s::Body::Value(&<OrderFact as c::Value>::SHAPE),
            },
            s::Variant {
                name: "Marked",
                body: s::Body::Value(&<ValuationBatch as c::Value>::SHAPE),
            },
            s::Variant {
                name: "AdjudicatedV1",
                body: s::Body::Value(&<adjudication::AdjudicatedFact as c::Value>::SHAPE),
            },
            s::Variant {
                name: "DerivedSnapshotV1",
                body: s::Body::Value(&<SnapshotRevision as c::Value>::SHAPE),
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Seeded<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Seeded<'de, '_, '_, '_> {
                type Value = Fact;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,Fact,true,{manifest:SeedManifest=>false,legacy_high_water_id:i64=>false,legacy_audit_high_water:String=>false},Fact::Seeded{manifest,legacy_high_water_id,legacy_audit_high_water})
                }
            }
            struct Seed_Order<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Order<'de, '_, '_, '_> {
                type Value = Fact;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <OrderFact as c::Value>::read(de, self.0).map(Fact::Order)
                }
            }
            struct Seed_Marked<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Marked<'de, '_, '_, '_> {
                type Value = Fact;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <ValuationBatch as c::Value>::read(de, self.0).map(Fact::Marked)
                }
            }
            struct Seed_AdjudicatedV1<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_AdjudicatedV1<'de, '_, '_, '_> {
                type Value = Fact;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <adjudication::AdjudicatedFact as c::Value>::read(de, self.0)
                        .map(Fact::AdjudicatedV1)
                }
            }
            struct Seed_DerivedSnapshotV1<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_DerivedSnapshotV1<'de, '_, '_, '_> {
                type Value = Fact;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <SnapshotRevision as c::Value>::read(de, self.0).map(Fact::DerivedSnapshotV1)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = Fact;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &[
                            "Seeded",
                            "Order",
                            "Marked",
                            "AdjudicatedV1",
                            "DerivedSnapshotV1",
                        ],
                    })?;
                    let object = self.0.bytes[self.0.span.start] == b'{';
                    let span = if object {
                        self.0
                            .span
                            .children(self.0.bytes)
                            .next()
                            .ok_or_else(c::span_error)?
                            .1
                    } else {
                        self.0.span
                    };
                    let origin = self.0.origin;
                    match tag {
                        "Seeded" => {
                            value.newtype_variant_seed(Seed_Seeded(self.0.child(span, origin)))
                        }
                        "Order" => {
                            value.newtype_variant_seed(Seed_Order(self.0.child(span, origin)))
                        }
                        "Marked" => {
                            value.newtype_variant_seed(Seed_Marked(self.0.child(span, origin)))
                        }
                        "AdjudicatedV1" => value
                            .newtype_variant_seed(Seed_AdjudicatedV1(self.0.child(span, origin))),
                        "DerivedSnapshotV1" => value.newtype_variant_seed(Seed_DerivedSnapshotV1(
                            self.0.child(span, origin),
                        )),
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &[
                    "Seeded",
                    "Order",
                    "Marked",
                    "AdjudicatedV1",
                    "DerivedSnapshotV1",
                ],
                EV(input),
            )
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                Fact::Seeded {
                    manifest,
                    legacy_high_water_id,
                    legacy_audit_high_water,
                } => Fact::Seeded {
                    manifest: c::Value::paid_copy(manifest, w)?,
                    legacy_high_water_id: c::Value::paid_copy(legacy_high_water_id, w)?,
                    legacy_audit_high_water: c::Value::paid_copy(legacy_audit_high_water, w)?,
                },
                Fact::Order(value) => Fact::Order(c::Value::paid_copy(value, w)?),
                Fact::Marked(value) => Fact::Marked(c::Value::paid_copy(value, w)?),
                Fact::AdjudicatedV1(value) => Fact::AdjudicatedV1(c::Value::paid_copy(value, w)?),
                Fact::DerivedSnapshotV1(value) => {
                    Fact::DerivedSnapshotV1(c::Value::paid_copy(value, w)?)
                }
            })
        }
    }
    impl c::sealed::Value for FillFingerprint {}
    impl c::Value for FillFingerprint {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "paper_trade_id",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "plan_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "event_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "raw_trade_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "audit_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fact_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "legacy_before_cutover",
                    shape: &<bool as c::Value>::SHAPE,
                    optional: <bool as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "legacy_no_terminal",
                    shape: &<bool as c::Value>::SHAPE,
                    optional: <bool as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            true,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,FillFingerprint,true,{paper_trade_id:i64=>false,plan_id:String=>false,event_hash:String=>false,raw_trade_hash:String=>false,audit_hash:String=>false,fact_at:DateTime<Utc> =>false,legacy_before_cutover:bool=>false,legacy_no_terminal:bool=>false},FillFingerprint{paper_trade_id,plan_id,event_hash,raw_trade_hash,audit_hash,fact_at,legacy_before_cutover,legacy_no_terminal})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(FillFingerprint {
                paper_trade_id: c::Value::paid_copy(&self.paper_trade_id, w)?,
                plan_id: c::Value::paid_copy(&self.plan_id, w)?,
                event_hash: c::Value::paid_copy(&self.event_hash, w)?,
                raw_trade_hash: c::Value::paid_copy(&self.raw_trade_hash, w)?,
                audit_hash: c::Value::paid_copy(&self.audit_hash, w)?,
                fact_at: c::Value::paid_copy(&self.fact_at, w)?,
                legacy_before_cutover: c::Value::paid_copy(&self.legacy_before_cutover, w)?,
                legacy_no_terminal: c::Value::paid_copy(&self.legacy_no_terminal, w)?,
            })
        }
    }
    impl c::sealed::Value for AdjudicationAction {}
    impl c::Value for AdjudicationAction {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "Quarantine",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "CorrectionDeclared",
                body: s::Body::Record(
                    &[
                        s::Field {
                            name: "price",
                            shape: &<Money as c::Value>::SHAPE,
                            optional: <Money as c::Value>::OPTIONAL,
                            positional_default: false,
                        },
                        s::Field {
                            name: "quantity",
                            shape: &<u32 as c::Value>::SHAPE,
                            optional: <u32 as c::Value>::OPTIONAL,
                            positional_default: false,
                        },
                        s::Field {
                            name: "fact_at",
                            shape: &<DateTime<Utc> as c::Value>::SHAPE,
                            optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                            positional_default: false,
                        },
                    ],
                    false,
                ),
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Quarantine<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Quarantine<'de, '_, '_, '_> {
                type Value = AdjudicationAction;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(AdjudicationAction::Quarantine)
                }
            }
            struct Seed_CorrectionDeclared<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_CorrectionDeclared<'de, '_, '_, '_> {
                type Value = AdjudicationAction;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,AdjudicationAction,true,{price:Money=>false,quantity:u32=>false,fact_at:DateTime<Utc> =>false},AdjudicationAction::CorrectionDeclared{price,quantity,fact_at})
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = AdjudicationAction;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["Quarantine", "CorrectionDeclared"],
                    })?;
                    let object = self.0.bytes[self.0.span.start] == b'{';
                    let span = if object {
                        self.0
                            .span
                            .children(self.0.bytes)
                            .next()
                            .ok_or_else(c::span_error)?
                            .1
                    } else {
                        self.0.span
                    };
                    let origin = self.0.origin;
                    match tag {
                        "Quarantine" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(AdjudicationAction::Quarantine)
                        }
                        "CorrectionDeclared" => value.newtype_variant_seed(
                            Seed_CorrectionDeclared(self.0.child(span, origin)),
                        ),
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum("codec", &["Quarantine", "CorrectionDeclared"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                AdjudicationAction::Quarantine => AdjudicationAction::Quarantine,
                AdjudicationAction::CorrectionDeclared {
                    price,
                    quantity,
                    fact_at,
                } => AdjudicationAction::CorrectionDeclared {
                    price: c::Value::paid_copy(price, w)?,
                    quantity: c::Value::paid_copy(quantity, w)?,
                    fact_at: c::Value::paid_copy(fact_at, w)?,
                },
            })
        }
    }
    impl c::sealed::Value for Adjudication {}
    impl c::Value for Adjudication {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "binding",
                    shape: &<AccountBinding as c::Value>::SHAPE,
                    optional: <AccountBinding as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "request_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "expected_version",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "expected_head",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "expected_predecessor",
                    shape: &<Option<String> as c::Value>::SHAPE,
                    optional: <Option<String> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "original",
                    shape: &<FillFingerprint as c::Value>::SHAPE,
                    optional: <FillFingerprint as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "action",
                    shape: &<AdjudicationAction as c::Value>::SHAPE,
                    optional: <AdjudicationAction as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "reason",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "evidence",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "operator",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "decision_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            true,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,Adjudication,true,{binding:AccountBinding=>false,request_id:String=>false,expected_version:i64=>false,expected_head:String=>false,expected_predecessor:Option<String> =>false,original:FillFingerprint=>false,action:AdjudicationAction=>false,reason:String=>false,evidence:String=>false,operator:String=>false,source:String=>false,decision_at:DateTime<Utc> =>false},Adjudication{binding,request_id,expected_version,expected_head,expected_predecessor,original,action,reason,evidence,operator,source,decision_at})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(Adjudication {
                binding: c::Value::paid_copy(&self.binding, w)?,
                request_id: c::Value::paid_copy(&self.request_id, w)?,
                expected_version: c::Value::paid_copy(&self.expected_version, w)?,
                expected_head: c::Value::paid_copy(&self.expected_head, w)?,
                expected_predecessor: c::Value::paid_copy(&self.expected_predecessor, w)?,
                original: c::Value::paid_copy(&self.original, w)?,
                action: c::Value::paid_copy(&self.action, w)?,
                reason: c::Value::paid_copy(&self.reason, w)?,
                evidence: c::Value::paid_copy(&self.evidence, w)?,
                operator: c::Value::paid_copy(&self.operator, w)?,
                source: c::Value::paid_copy(&self.source, w)?,
                decision_at: c::Value::paid_copy(&self.decision_at, w)?,
            })
        }
    }
    impl c::sealed::Value for AdjudicatedFact {}
    impl c::Value for AdjudicatedFact {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "request",
                    shape: &<Adjudication as c::Value>::SHAPE,
                    optional: <Adjudication as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "projection",
                    shape: &<Projection as c::Value>::SHAPE,
                    optional: <Projection as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "historical_projection",
                    shape: &<Option<(String, Option<String>)> as c::Value>::SHAPE,
                    optional: <Option<(String, Option<String>)> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            true,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,AdjudicatedFact,true,{request:Adjudication=>false,projection:Projection=>false,historical_projection:Option<(String, Option<String>)> =>false},AdjudicatedFact{request,projection,historical_projection})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(AdjudicatedFact {
                request: c::Value::paid_copy(&self.request, w)?,
                projection: c::Value::paid_copy(&self.projection, w)?,
                historical_projection: c::Value::paid_copy(&self.historical_projection, w)?,
            })
        }
    }
    impl c::sealed::Value for SnapshotRevision {}
    impl c::Value for SnapshotRevision {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "target_date",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "algorithm",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "projection",
                    shape: &<EffectiveProjectionReceipt as c::Value>::SHAPE,
                    optional: <EffectiveProjectionReceipt as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "metrics",
                    shape: &<crate::performance::snapshot::PerformanceSnapshot as c::Value>::SHAPE,
                    optional:
                        <crate::performance::snapshot::PerformanceSnapshot as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "opening_exclusions",
                    shape: &<Vec<OpeningInventoryExit> as c::Value>::SHAPE,
                    optional: <Vec<OpeningInventoryExit> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "account_realized_pnl",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "result_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            true,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,SnapshotRevision,true,{target_date:NaiveDate=>false,algorithm:String=>false,projection:EffectiveProjectionReceipt=>false,metrics:crate::performance::snapshot::PerformanceSnapshot=>false,opening_exclusions:Vec<OpeningInventoryExit> =>false,account_realized_pnl:Money=>false,result_hash:String=>false},SnapshotRevision{target_date,algorithm,projection,metrics,opening_exclusions,account_realized_pnl,result_hash})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(SnapshotRevision {
                target_date: c::Value::paid_copy(&self.target_date, w)?,
                algorithm: c::Value::paid_copy(&self.algorithm, w)?,
                projection: c::Value::paid_copy(&self.projection, w)?,
                metrics: c::Value::paid_copy(&self.metrics, w)?,
                opening_exclusions: c::Value::paid_copy(&self.opening_exclusions, w)?,
                account_realized_pnl: c::Value::paid_copy(&self.account_realized_pnl, w)?,
                result_hash: c::Value::paid_copy(&self.result_hash, w)?,
            })
        }
    }
    impl c::sealed::Value for EffectiveFillScope {}
    impl c::Value for EffectiveFillScope {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "LegacyRaw",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Epoch",
                body: s::Body::Value(&<AccountBinding as c::Value>::SHAPE),
            },
            s::Variant {
                name: "LegacyBeforeCutover",
                body: s::Body::Value(&<AccountBinding as c::Value>::SHAPE),
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_LegacyRaw<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_LegacyRaw<'de, '_, '_, '_> {
                type Value = EffectiveFillScope;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(EffectiveFillScope::LegacyRaw)
                }
            }
            struct Seed_Epoch<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Epoch<'de, '_, '_, '_> {
                type Value = EffectiveFillScope;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <AccountBinding as c::Value>::read(de, self.0).map(EffectiveFillScope::Epoch)
                }
            }
            struct Seed_LegacyBeforeCutover<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_LegacyBeforeCutover<'de, '_, '_, '_> {
                type Value = EffectiveFillScope;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <AccountBinding as c::Value>::read(de, self.0)
                        .map(EffectiveFillScope::LegacyBeforeCutover)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = EffectiveFillScope;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["LegacyRaw", "Epoch", "LegacyBeforeCutover"],
                    })?;
                    let object = self.0.bytes[self.0.span.start] == b'{';
                    let span = if object {
                        self.0
                            .span
                            .children(self.0.bytes)
                            .next()
                            .ok_or_else(c::span_error)?
                            .1
                    } else {
                        self.0.span
                    };
                    let origin = self.0.origin;
                    match tag {
                        "LegacyRaw" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(EffectiveFillScope::LegacyRaw)
                        }
                        "Epoch" => {
                            value.newtype_variant_seed(Seed_Epoch(self.0.child(span, origin)))
                        }
                        "LegacyBeforeCutover" => value.newtype_variant_seed(
                            Seed_LegacyBeforeCutover(self.0.child(span, origin)),
                        ),
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &["LegacyRaw", "Epoch", "LegacyBeforeCutover"],
                EV(input),
            )
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                EffectiveFillScope::LegacyRaw => EffectiveFillScope::LegacyRaw,
                EffectiveFillScope::Epoch(value) => {
                    EffectiveFillScope::Epoch(c::Value::paid_copy(value, w)?)
                }
                EffectiveFillScope::LegacyBeforeCutover(value) => {
                    EffectiveFillScope::LegacyBeforeCutover(c::Value::paid_copy(value, w)?)
                }
            })
        }
    }
    impl c::sealed::Value for EffectiveHistory {}
    impl c::Value for EffectiveHistory {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "AsKnown",
                body: s::Body::Record(
                    &[s::Field {
                        name: "ledger_version",
                        shape: &<Option<i64> as c::Value>::SHAPE,
                        optional: <Option<i64> as c::Value>::OPTIONAL,
                        positional_default: false,
                    }],
                    false,
                ),
            },
            s::Variant {
                name: "RestatedLatest",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_AsKnown<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_AsKnown<'de, '_, '_, '_> {
                type Value = EffectiveHistory;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,EffectiveHistory,true,{ledger_version:Option<i64> =>false},EffectiveHistory::AsKnown{ledger_version})
                }
            }
            struct Seed_RestatedLatest<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_RestatedLatest<'de, '_, '_, '_> {
                type Value = EffectiveHistory;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(EffectiveHistory::RestatedLatest)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = EffectiveHistory;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["AsKnown", "RestatedLatest"],
                    })?;
                    let object = self.0.bytes[self.0.span.start] == b'{';
                    let span = if object {
                        self.0
                            .span
                            .children(self.0.bytes)
                            .next()
                            .ok_or_else(c::span_error)?
                            .1
                    } else {
                        self.0.span
                    };
                    let origin = self.0.origin;
                    match tag {
                        "AsKnown" => {
                            value.newtype_variant_seed(Seed_AsKnown(self.0.child(span, origin)))
                        }
                        "RestatedLatest" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(EffectiveHistory::RestatedLatest)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum("codec", &["AsKnown", "RestatedLatest"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                EffectiveHistory::AsKnown { ledger_version } => EffectiveHistory::AsKnown {
                    ledger_version: c::Value::paid_copy(ledger_version, w)?,
                },
                EffectiveHistory::RestatedLatest => EffectiveHistory::RestatedLatest,
            })
        }
    }
    impl c::sealed::Value for EffectiveFillRequest {}
    impl c::Value for EffectiveFillRequest {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "scope",
                    shape: &<EffectiveFillScope as c::Value>::SHAPE,
                    optional: <EffectiveFillScope as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "history",
                    shape: &<EffectiveHistory as c::Value>::SHAPE,
                    optional: <EffectiveHistory as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "as_of",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,EffectiveFillRequest,true,{scope:EffectiveFillScope=>false,history:EffectiveHistory=>false,as_of:NaiveDate=>false},EffectiveFillRequest{scope,history,as_of})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(EffectiveFillRequest {
                scope: c::Value::paid_copy(&self.scope, w)?,
                history: c::Value::paid_copy(&self.history, w)?,
                as_of: c::Value::paid_copy(&self.as_of, w)?,
            })
        }
    }
    impl c::sealed::Value for EffectiveProjectionReceipt {}
    impl c::Value for EffectiveProjectionReceipt {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "request",
                    shape: &<EffectiveFillRequest as c::Value>::SHAPE,
                    optional: <EffectiveFillRequest as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "raw_high_water",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "raw_source_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "ledger_head",
                    shape: &<Option<(i64, String)> as c::Value>::SHAPE,
                    optional: <Option<(i64, String)> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "inventory_fingerprint",
                    shape: &<Option<String> as c::Value>::SHAPE,
                    optional: <Option<String> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "economic_head",
                    shape: &<Option<(i64, String)> as c::Value>::SHAPE,
                    optional: <Option<(i64, String)> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "adjudication_head",
                    shape: &<Option<(i64, String)> as c::Value>::SHAPE,
                    optional: <Option<(i64, String)> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cutover_at",
                    shape: &<Option<DateTime<Utc>> as c::Value>::SHAPE,
                    optional: <Option<DateTime<Utc>> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cutover_raw_high_water",
                    shape: &<Option<i64> as c::Value>::SHAPE,
                    optional: <Option<i64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "rule_version",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "projection_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "catalog_generation",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "catalog_objects_hash",
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
            c::record_read!(de,input,EffectiveProjectionReceipt,true,{request:EffectiveFillRequest=>false,raw_high_water:i64=>false,raw_source_hash:String=>false,ledger_head:Option<(i64, String)> =>false,inventory_fingerprint:Option<String> =>false,economic_head:Option<(i64, String)> =>false,adjudication_head:Option<(i64, String)> =>false,cutover_at:Option<DateTime<Utc>> =>false,cutover_raw_high_water:Option<i64> =>false,rule_version:String=>false,projection_hash:String=>false,catalog_generation:i64=>false,catalog_objects_hash:String=>false},EffectiveProjectionReceipt{request,raw_high_water,raw_source_hash,ledger_head,inventory_fingerprint,economic_head,adjudication_head,cutover_at,cutover_raw_high_water,rule_version,projection_hash,catalog_generation,catalog_objects_hash})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(EffectiveProjectionReceipt {
                request: c::Value::paid_copy(&self.request, w)?,
                raw_high_water: c::Value::paid_copy(&self.raw_high_water, w)?,
                raw_source_hash: c::Value::paid_copy(&self.raw_source_hash, w)?,
                ledger_head: c::Value::paid_copy(&self.ledger_head, w)?,
                inventory_fingerprint: c::Value::paid_copy(&self.inventory_fingerprint, w)?,
                economic_head: c::Value::paid_copy(&self.economic_head, w)?,
                adjudication_head: c::Value::paid_copy(&self.adjudication_head, w)?,
                cutover_at: c::Value::paid_copy(&self.cutover_at, w)?,
                cutover_raw_high_water: c::Value::paid_copy(&self.cutover_raw_high_water, w)?,
                rule_version: c::Value::paid_copy(&self.rule_version, w)?,
                projection_hash: c::Value::paid_copy(&self.projection_hash, w)?,
                catalog_generation: c::Value::paid_copy(&self.catalog_generation, w)?,
                catalog_objects_hash: c::Value::paid_copy(&self.catalog_objects_hash, w)?,
            })
        }
    }
    impl c::sealed::Value for OpeningInventoryExit {}
    impl c::Value for OpeningInventoryExit {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "fill_id",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "original_quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "opening_quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "strategy_quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "seed_lot_ids",
                    shape: &<Vec<String> as c::Value>::SHAPE,
                    optional: <Vec<String> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "opening_basis",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "opening_buy_fee",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "opening_sell_fee",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "opening_net_pnl",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,OpeningInventoryExit,true,{fill_id:i64=>false,original_quantity:u32=>false,opening_quantity:u32=>false,strategy_quantity:u32=>false,seed_lot_ids:Vec<String> =>false,opening_basis:Money=>false,opening_buy_fee:Money=>false,opening_sell_fee:Money=>false,opening_net_pnl:Money=>false},OpeningInventoryExit{fill_id,original_quantity,opening_quantity,strategy_quantity,seed_lot_ids,opening_basis,opening_buy_fee,opening_sell_fee,opening_net_pnl})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(OpeningInventoryExit {
                fill_id: c::Value::paid_copy(&self.fill_id, w)?,
                original_quantity: c::Value::paid_copy(&self.original_quantity, w)?,
                opening_quantity: c::Value::paid_copy(&self.opening_quantity, w)?,
                strategy_quantity: c::Value::paid_copy(&self.strategy_quantity, w)?,
                seed_lot_ids: c::Value::paid_copy(&self.seed_lot_ids, w)?,
                opening_basis: c::Value::paid_copy(&self.opening_basis, w)?,
                opening_buy_fee: c::Value::paid_copy(&self.opening_buy_fee, w)?,
                opening_sell_fee: c::Value::paid_copy(&self.opening_sell_fee, w)?,
                opening_net_pnl: c::Value::paid_copy(&self.opening_net_pnl, w)?,
            })
        }
    }
    impl c::sealed::Value for PerformanceSnapshot {}
    impl c::Value for PerformanceSnapshot {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "id",
                    shape: &<i32 as c::Value>::SHAPE,
                    optional: <i32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "date",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "total_trades",
                    shape: &<i32 as c::Value>::SHAPE,
                    optional: <i32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "winning_trades",
                    shape: &<i32 as c::Value>::SHAPE,
                    optional: <i32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "losing_trades",
                    shape: &<i32 as c::Value>::SHAPE,
                    optional: <i32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "total_pnl",
                    shape: &<f64 as c::Value>::SHAPE,
                    optional: <f64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sharpe_ratio",
                    shape: &<Option<f64> as c::Value>::SHAPE,
                    optional: <Option<f64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sortino_ratio",
                    shape: &<Option<f64> as c::Value>::SHAPE,
                    optional: <Option<f64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "win_rate",
                    shape: &<Option<f64> as c::Value>::SHAPE,
                    optional: <Option<f64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "max_drawdown",
                    shape: &<Option<f64> as c::Value>::SHAPE,
                    optional: <Option<f64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "info_ratio",
                    shape: &<Option<f64> as c::Value>::SHAPE,
                    optional: <Option<f64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "created_at",
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
            c::record_read!(de,input,PerformanceSnapshot,true,{id:i32=>false,date:String=>false,total_trades:i32=>false,winning_trades:i32=>false,losing_trades:i32=>false,total_pnl:f64=>false,sharpe_ratio:Option<f64> =>false,sortino_ratio:Option<f64> =>false,win_rate:Option<f64> =>false,max_drawdown:Option<f64> =>false,info_ratio:Option<f64> =>false,created_at:String=>false},PerformanceSnapshot{id,date,total_trades,winning_trades,losing_trades,total_pnl,sharpe_ratio,sortino_ratio,win_rate,max_drawdown,info_ratio,created_at})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(PerformanceSnapshot {
                id: c::Value::paid_copy(&self.id, w)?,
                date: c::Value::paid_copy(&self.date, w)?,
                total_trades: c::Value::paid_copy(&self.total_trades, w)?,
                winning_trades: c::Value::paid_copy(&self.winning_trades, w)?,
                losing_trades: c::Value::paid_copy(&self.losing_trades, w)?,
                total_pnl: c::Value::paid_copy(&self.total_pnl, w)?,
                sharpe_ratio: c::Value::paid_copy(&self.sharpe_ratio, w)?,
                sortino_ratio: c::Value::paid_copy(&self.sortino_ratio, w)?,
                win_rate: c::Value::paid_copy(&self.win_rate, w)?,
                max_drawdown: c::Value::paid_copy(&self.max_drawdown, w)?,
                info_ratio: c::Value::paid_copy(&self.info_ratio, w)?,
                created_at: c::Value::paid_copy(&self.created_at, w)?,
            })
        }
    }
    impl c::sealed::Element for SeedLot {}
    impl c::ArrayElement for SeedLot {}
    impl c::sealed::Element for Mark {}
    impl c::ArrayElement for Mark {}
    impl c::sealed::Element for Lot {}
    impl c::ArrayElement for Lot {}
    impl c::sealed::Element for OpeningInventoryExit {}
    impl c::ArrayElement for OpeningInventoryExit {}
    impl c::sealed::Entry for (String, Mark) {}
    impl c::MapEntry for (String, Mark) {}
    impl c::sealed::Entry for (NaiveDate, Money) {}
    impl c::MapEntry for (NaiveDate, Money) {}
    impl c::sealed::Value for Money {}
    impl c::Value for Money {
        const SHAPE: s::Shape = <i64 as c::Value>::SHAPE;
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            <i64 as c::Value>::read(de, input).map(Money::from_micros)
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(*self)
        }
    }
    impl c::sealed::Root for SeedManifest {}
    impl c::Root for SeedManifest {
        const ROOT: s::RootKind = s::RootKind::Seed;
        const CANONICAL: bool = false;
    }
    impl c::sealed::Root for AccountBinding {}
    impl c::Root for AccountBinding {
        const ROOT: s::RootKind = s::RootKind::Binding;
        const CANONICAL: bool = false;
    }
    impl c::sealed::Root for Projection {}
    impl c::Root for Projection {
        const ROOT: s::RootKind = s::RootKind::Projection;
        const CANONICAL: bool = false;
    }
    impl c::sealed::Root for Fact {}
    impl c::Root for Fact {
        const ROOT: s::RootKind = s::RootKind::V1Fact;
        const CANONICAL: bool = false;
    }
}
#[cfg(test)]
pub(crate) fn replay_codec_fixtures(
    case: crate::trading::paper_replay_codec_v1::CodecFixtureCase,
    work: &mut crate::database::global_schema_v1::replay_work::CodecMechanics<'_, '_>,
) {
    crate::trading::paper_replay_codec_v1::exercise_root::<SeedManifest>(case, work);
    crate::trading::paper_replay_codec_v1::exercise_root::<AccountBinding>(case, work);
    crate::trading::paper_replay_codec_v1::exercise_root::<Projection>(case, work);
    crate::trading::paper_replay_codec_v1::exercise_root::<Fact>(case, work);
}

#[cfg(test)]
pub(crate) fn replay_codec_nonfinite(
    work: &mut crate::database::global_schema_v1::replay_work::CodecMechanics<'_, '_>,
) {
    use crate::trading::paper_replay_codec_v1 as c;
    let bytes = c::fixed_snapshot_fixture();
    let mut fact: Fact = serde_json::from_slice(&bytes).unwrap();
    let Fact::DerivedSnapshotV1(ref mut snapshot) = fact else {
        unreachable!()
    };
    snapshot.metrics.total_pnl = f64::NAN;
    snapshot.metrics.sharpe_ratio = Some(f64::INFINITY);
    snapshot.metrics.sortino_ratio = Some(f64::NEG_INFINITY);
    let expected = serde_json::to_vec(&fact).unwrap();
    assert!(std::str::from_utf8(&expected)
        .unwrap()
        .contains("\"total_pnl\":null"));
    assert_eq!(c::encode_core(&fact, work).unwrap(), expected);
    let copied = c::Value::paid_copy(&fact, work).unwrap();
    assert_eq!(c::encode_core(&copied, work).unwrap(), expected);
}

#[cfg(test)]
pub(crate) fn transition_v1_fixture(state:Projection, w:&mut FinancialWork<'_, '_>){
    execution::transition_fixture(state, w)
}

#[cfg(test)]
pub(crate) fn history_owner_fixture(
    case: crate::trading::paper_replay_history_v1_tests::Case,
    work: &mut FinancialWork<'_, '_>,
) {
    use crate::trading::paper_replay_history_v1_tests::{
        self as test,
        Case
    };
    let seed = test::seed();
    let binding = seed.binding().unwrap();
    let account = AccountRow {
        epoch_id: seed.epoch_id.clone(), manifest_hash: binding.manifest_hash.clone(),
        manifest_bytes: encode(&seed).unwrap(),
    };
    let marked = ValuationBatch {
        binding: binding.clone(), command_id: "history-mark".into(), expected_version: 1,
        inventory_fingerprint: seed_projection(&seed).unwrap().inventory_fingerprint().unwrap(),
        as_of: test::at(), closing: true,
        marks: vec![Mark {
            price: Money::from_micros(11_000_000), ..seed.marks[0].clone()
        }],
    };
    let facts = [
        Fact::Seeded {
            manifest: seed.clone(),
            legacy_high_water_id: 0,
            legacy_audit_high_water: "0".into()
        },
        Fact::Marked(marked),
    ];
    let mut previous = GENESIS.to_owned();
    let mut events = Vec::new();
    for (index, fact) in facts.iter().enumerate() {
        let command = format!("history-command-{index}");
        let payload = encode(fact).unwrap();
        let hash = event_hash(&binding.account_id, index as i64 + 1, &command, &previous, &payload).unwrap();
        events.push(EventRow {
            seq: index as i64 + 1, command_id: command, previous_hash: previous,
            event_hash: hash.clone(), payload, business_plan_id: None, intent_hash: None,
            is_terminal: 0, paper_trade_id: None, order_audit_id: None,
        });
        previous = hash;
    }
    if matches!(case, Case::Adjudication) {
        adjudication::history_adjudication_fixture(&binding, &events, work);
        return;
    }
    if matches!(case, Case::Serialization) {
        let snapshot = crate::trading::paper_replay_codec_v1::fixed_snapshot_fixture();
        let Fact::DerivedSnapshotV1(mut revision) = decode(&String::from_utf8(snapshot).unwrap()).unwrap() else {
            panic!("fixed snapshot");
        };
        revision.algorithm = "PaperSnapshotNetFifoV1".into();
        revision.target_date = test::date();
        revision.projection.request.as_of = test::date();
        revision.metrics.date = test::date().to_string();
        revision.metrics.total_pnl = -0.0;
        revision.account_realized_pnl = Money::from_micros(987_654_321);
        revision.result_hash = snapshot::result_hash_fixture(&revision);
        snapshot::validate_with_work(&revision, work).unwrap();
        let original_state = seed_projection(&seed).unwrap();
        let mut state = work.copy(&original_state).unwrap();
        let fact = Fact::DerivedSnapshotV1(revision.clone());
        execution::apply_fact_with_work(&mut state, &fact, work).unwrap();
        assert_eq!(state, original_state, "a valid different receipt is validated, never applied as Projection");
        for broken_algorithm in [true, false] {
            let mut invalid = revision.clone();
            if broken_algorithm {
                invalid.algorithm = "wrong".into();
            }
            else {
                invalid.result_hash = "different".into();
            }
            let expected = snapshot::validate(&invalid).unwrap_err();
            let actual = snapshot::validate_with_work(&invalid, work).unwrap_err();
            assert!(matches!(actual, FinancialFailure::Financial(LedgerError::IntegrityFailure(ref text))
                if expected.to_string() == LedgerError::IntegrityFailure(text.clone()).to_string()));
        }
        let expected = encode(&("PaperSnapshotNetFifoV1", revision.target_date, &revision.projection,
            &revision.metrics, &revision.opening_exclusions, revision.account_realized_pnl)).unwrap();
        let actual = work.history_hash(crate::trading::paper_replay_codec_v1::HistoryOutput::Snapshot(&revision)).unwrap();
        assert_eq!(actual, digest(&expected));
        for row in &events {
            assert_eq!(work.history_hash(crate::trading::paper_replay_codec_v1::HistoryOutput::Event {
                account: &binding.account_id, seq: row.seq, command: &row.command_id,
                previous: &row.previous_hash, payload: &row.payload,
            }).unwrap(), row.event_hash);
        }
        return;
    }
    let mut original = start_original_replay(&binding, &account, &mut FinancialWork::Historical).unwrap();
    for row in &events {
        let copied = EventRow::from_bounded_sql_parts(row.seq, row.command_id.clone(), row.previous_hash.clone(),
            row.event_hash.clone(), row.payload.clone(), None, None, 0, None, None);
        let pending = validate_next_original_event(&binding, &mut original, copied, &mut FinancialWork::Historical).unwrap();
        apply_original_pending(&mut original, pending, &mut FinancialWork::Historical).unwrap();
    }
    let original = finish_original_events(original, &mut FinancialWork::Historical).unwrap();
    let head = HeadRow {
        version: original.version, event_hash: original.event_hash.clone(),
        projection_bytes: encode(&original.projection).unwrap(), projection_hash: digest(&encode(&original.projection).unwrap()),
    };
    let passes = if matches!(case, Case::Gen1Double) {
        2
    }
    else {
        1
    };
    for _ in 0..passes {
        let used = work.history_used();
        let mut paid = start_original_replay(&binding, &account, work).unwrap();
        for row in &events {
            // Fixtures supply retained rows as data. This constructor does not
            // establish SQL source payment or a provider/Target capability.
            let copied = EventRow::from_bounded_sql_parts(row.seq, row.command_id.clone(), row.previous_hash.clone(),
                row.event_hash.clone(), row.payload.clone(), None, None, 0, None, None);
            let pending = validate_next_original_event(&binding, &mut paid, copied, work).unwrap();
            apply_original_pending(&mut paid, pending, work).unwrap();
        }
        let paid = finish_original_events(paid, work).unwrap();
        assert_eq!(paid.version, 2);
        assert_eq!(paid.projection, original.projection);
        assert_eq!(paid.event_hash, original.event_hash);
        assert_eq!(paid.projection.marks["600001"].price, Money::from_micros(11_000_000));
        assert_eq!(paid.projection.closes.len(), 1);
        compare_original_head(&paid, &head, work).unwrap();
        assert!(work.history_used() > used);
        if matches!(case, Case::OriginalReplay) {
            execution::transition_fixture(seed_projection_with_work(&seed, work).unwrap(), work);
        }
    }
}

#[cfg(test)]
pub(crate) fn history_raw_fixture<'loan, 'pool>(work: FinancialWork<'loan, 'pool>) -> FinancialWork<'loan, 'pool> {
    adjudication::history_raw_fixture(work)
}

#[cfg(test)]
pub(crate) fn history_legacy_fixture<'loan, 'pool>(
    work: FinancialWork<'loan, 'pool>,
) -> FinancialWork<'loan, 'pool> {
    effective::history_legacy_fixture(work)
}

// No owned fields, replay, hashes or qualification are constructed here.
// None is the actual first-event domain constant; old replay supplies its
// existing paid cursor.previous, preserving its exact short-circuit position.
pub(crate) fn raw_v1_event_link_matches(
    sequence: i64, expected_sequence: i64, previous: &str, previous_event_hash: Option<&str>,
) -> bool {
    sequence == expected_sequence && previous == previous_event_hash.unwrap_or(GENESIS)
}
pub(crate) fn raw_v1_head_link_matches(
    head_version: i64, head_hash: &str, last_version: i64, last_event_hash: &str,
) -> bool {
    head_version == last_version && head_hash == last_event_hash
}
