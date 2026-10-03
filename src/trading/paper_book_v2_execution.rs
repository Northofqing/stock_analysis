//! CatalogV6 modeled parent orders. Old V1/V5 financial facts are immutable.
//! No broker, production approval, startup DDL, or JSON capability factory.

use super::paper_book_v2_budget_v1::{
    self as budget, BudgetRecord, CashPartitions, LotDisposition, MarkedAllocation,
    WorkingReservation,
};
use super::paper_book_v2_fill_model::{
    self as fill_model, ModelOutcome, ModeledFill, Side, WindowRecord, MODEL_VERSION,
};
use super::paper_ledger::{LedgerError, Lot, Mark, Money, Projection};
use crate::database::global_schema_v1::paper_v6::{
    paper_catalog6_session, PaperCatalog6Error, PaperCatalog6ReadbackError,
    PaperCatalog6TransactionError,
};
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use crate::decision::approved_paper_intent_v1::{
    ApprovedPaperIntentV1, IntentRecord, QualifiedPaperExecutionWindowV1,
};
use crate::performance::fee_policy::{
    AShareFeePolicyV2, ExcludedFeeReason, FeeCoverage, FeeListingSegment, FeeMarket, FeeRate,
    FeeSecurityKind, QualifiedInstrument,
};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Nullable, Text};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MANIFEST_VERSION: &str = "paper-parent-execution-manifest/v1";
const PROJECTION_VERSION: &str = "paper-parent-projection/v1";
const MAX_RECORD_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, LedgerError> {
    let bytes =
        serde_json::to_vec(value).map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
    require(
        bytes.len() <= MAX_RECORD_BYTES,
        "execution record exceeds byte limit",
    )?;
    Ok(bytes)
}
pub(crate) fn decode<T: Serialize + DeserializeOwned>(bytes: &[u8]) -> Result<T, LedgerError> {
    require(
        bytes.len() <= MAX_RECORD_BYTES,
        "execution record exceeds byte limit",
    )?;
    let value: T =
        serde_json::from_slice(bytes).map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
    require(encode(&value)? == bytes, "noncanonical execution record")?;
    Ok(value)
}
fn require(condition: bool, reason: &str) -> Result<(), LedgerError> {
    if condition {
        Ok(())
    } else {
        Err(LedgerError::IntegrityFailure(reason.into()))
    }
}
pub(crate) fn hash<T: Serialize>(domain: &str, value: &T) -> Result<String, LedgerError> {
    let mut sha = Sha256::new();
    sha.update(domain.as_bytes());
    sha.update(b"\n");
    sha.update(encode(value)?);
    Ok(hex::encode(sha.finalize()))
}
fn raw_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn checked_shanghai_local(at: DateTime<Utc>) -> Result<DateTime<Utc>, LedgerError> {
    at.checked_add_signed(chrono::Duration::hours(8))
        .ok_or_else(|| {
            LedgerError::EvidenceUnavailable(
                "paper execution Shanghai clock exceeds supported range".into(),
            )
        })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionManifest {
    pub(crate) version: String,
    pub(crate) account_id: String,
    pub(crate) epoch_id: String,
    pub(crate) cutover_id: String,
    pub(crate) genesis_event_hash: String,
    pub(crate) genesis_projection_hash: String,
    pub(crate) fee_descriptor: Vec<u8>,
    pub(crate) fee_policy_instance_id: String,
    pub(crate) budget: BudgetRecord,
    pub(crate) fill_model_version: String,
    pub(crate) approved_reference: String,
}
impl ExecutionManifest {
    pub(crate) fn identity(&self) -> Result<String, LedgerError> {
        hash(MANIFEST_VERSION, self)
    }
    fn validate(&self) -> Result<AShareFeePolicyV2, LedgerError> {
        self.budget.validate_shape()?;
        let fee = fee_from_record(&self.fee_descriptor)?;
        require(
            self.version == MANIFEST_VERSION
                && self.fill_model_version == MODEL_VERSION
                && budget::token(&self.account_id)
                && budget::token(&self.epoch_id)
                && budget::token(&self.cutover_id)
                && budget::token(&self.approved_reference)
                && self.genesis_event_hash.len() == 64
                && self.genesis_projection_hash.len() == 64
                && self.fee_policy_instance_id == fee.instance_id(),
            "execution manifest invalid",
        )?;
        Ok(fee)
    }
}

/// A value-level fee descriptor reconstruction used only for exact replay.
/// The original immutable database descriptor must additionally match it.
fn fee_from_record(bytes: &[u8]) -> Result<AShareFeePolicyV2, LedgerError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| LedgerError::IntegrityFailure("fee descriptor UTF8".into()))?;
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| LedgerError::IntegrityFailure("fee descriptor field".into()))?;
        require(
            fields.insert(key, value).is_none(),
            "duplicate fee descriptor field",
        )?;
    }
    let get = |key: &str| {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| LedgerError::IntegrityFailure("fee descriptor missing field".into()))
    };
    let integer = |key: &str| {
        get(key)?
            .parse::<i64>()
            .map_err(|_| LedgerError::IntegrityFailure("fee descriptor integer".into()))
    };
    let segment = match get("segment")? {
        "ShanghaiMainA" => FeeListingSegment::ShanghaiMainA,
        "ShanghaiStarA" => FeeListingSegment::ShanghaiStarA,
        _ => {
            return Err(LedgerError::EvidenceUnavailable(
                "fee segment unavailable".into(),
            ))
        }
    };
    let reason = |key: &str| match get(key)? {
        "excluded_unmodeled" => Ok(ExcludedFeeReason::Unmodeled),
        "excluded_unverified" => Ok(ExcludedFeeReason::Unverified),
        _ => Err(LedgerError::IntegrityFailure("fee coverage field".into())),
    };
    let policy = AShareFeePolicyV2::new(
        QualifiedInstrument::new(FeeMarket::Shanghai, FeeSecurityKind::AShareStock, segment)
            .map_err(|e| LedgerError::EvidenceUnavailable(e.to_string()))?,
        FeeRate::new(
            integer("commission_rate_num")?,
            integer("commission_rate_den")?,
        )
        .map_err(|e| LedgerError::EvidenceUnavailable(e.to_string()))?,
        integer("commission_minimum_micro_cny")?,
        FeeCoverage::new(
            reason("coverage_transfer_fee")?,
            reason("coverage_other_charges")?,
        ),
        get("source_revision")?,
    )
    .map_err(|e| LedgerError::EvidenceUnavailable(e.to_string()))?;
    require(
        policy.canonical_bytes() == bytes,
        "fee descriptor is not canonical reviewed policy",
    )?;
    Ok(policy)
}

/// Created only by the actual checked namespace reader, not recorded views.
#[derive(Debug)]
pub(crate) struct ActualExecutionBinding {
    account_id: String,
    epoch_id: String,
    manifest_hash: String,
    family_id: String,
    database_authority: DatabaseConnectionAuthority,
    fee_segment: FeeListingSegment,
    #[cfg(test)]
    isolated_test: bool,
}
impl ActualExecutionBinding {
    pub(crate) fn account_id(&self) -> &str {
        &self.account_id
    }
    pub(crate) fn epoch_id(&self) -> &str {
        &self.epoch_id
    }
    pub(crate) fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }
    pub(crate) fn family_id(&self) -> &str {
        &self.family_id
    }
    pub(crate) fn database_authority(&self) -> &DatabaseConnectionAuthority {
        &self.database_authority
    }
    pub(crate) fn require_record_binding(
        &self,
        r: &IntentRecord,
        authority: &DatabaseConnectionAuthority,
    ) -> Result<(), LedgerError> {
        require(
            authority == &self.database_authority
                && r.account_id == self.account_id
                && r.epoch_id == self.epoch_id
                && r.execution_manifest_hash == self.manifest_hash
                && r.family_id == self.family_id,
            "actual intent owner/namespace differs",
        )
    }
    pub(crate) fn require_window_namespace(
        &self,
        authority: &DatabaseConnectionAuthority,
    ) -> Result<(), LedgerError> {
        require(
            authority == &self.database_authority,
            "actual window namespace differs",
        )
    }
    pub(crate) fn require_fee_segment(&self, segment: &str) -> Result<(), LedgerError> {
        require(
            matches!(
                (self.fee_segment, segment),
                (FeeListingSegment::ShanghaiMainA, "ShanghaiMainA")
                    | (FeeListingSegment::ShanghaiStarA, "ShanghaiStarA")
            ),
            "actual fee scope differs from admitted board",
        )
    }
    #[cfg(test)]
    pub(crate) fn require_test_issuer(&self) -> Result<(), LedgerError> {
        require(
            self.isolated_test,
            "Test issuer requires constructor-issued isolated namespace",
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ParentStatus {
    Working,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
}
impl ParentStatus {
    fn working(self) -> bool {
        matches!(self, Self::Working | Self::PartiallyFilled)
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LotClaim {
    pub(crate) lot_id: String,
    pub(crate) quantity: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentState {
    pub(crate) intent: IntentRecord,
    pub(crate) status: ParentStatus,
    pub(crate) filled: u32,
    pub(crate) remaining: u32,
    pub(crate) cancelled: u32,
    pub(crate) reservation: WorkingReservation,
    pub(crate) sell_claims: Vec<LotClaim>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FillRecord {
    pub(crate) fill_id: String,
    pub(crate) parent_id: String,
    pub(crate) observation_id: String,
    pub(crate) executed_at: DateTime<Utc>,
    pub(crate) side: Side,
    pub(crate) model: ModeledFill,
    pub(crate) inherited_buy_fee_micro_cny: i64,
    pub(crate) realized_pnl_micro_cny: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionProjection {
    pub(crate) version: String,
    pub(crate) account: Projection,
    pub(crate) cash: CashPartitions,
    pub(crate) lot_assignments: BTreeMap<String, Option<String>>,
    pub(crate) parents: BTreeMap<String, ParentState>,
    pub(crate) used_windows: BTreeMap<String, String>,
    pub(crate) fills: Vec<FillRecord>,
    /// Recorded windows explain each new qualified mark; they are never
    /// deserialized into a live source capability. Original genesis marks do
    /// not grant a fresh valuation window for a later risk-increasing buy.
    valuation_windows: BTreeMap<String, WindowRecord>,
}
impl ExecutionProjection {
    fn initial(genesis: &Projection, budget: &BudgetRecord) -> Result<Self, LedgerError> {
        let cash = budget.initial_cash(genesis)?;
        let lot_assignments = budget
            .initial_lots
            .iter()
            .map(|r| {
                (
                    r.lot_id.clone(),
                    if r.disposition == LotDisposition::AllocatedToStrategy {
                        r.chain_id.clone()
                    } else {
                        None
                    },
                )
            })
            .collect();
        let value = Self {
            version: PROJECTION_VERSION.into(),
            account: genesis.clone(),
            cash,
            lot_assignments,
            parents: BTreeMap::new(),
            used_windows: BTreeMap::new(),
            fills: Vec::new(),
            valuation_windows: BTreeMap::new(),
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), LedgerError> {
        self.cash.validate()?;
        require(
            self.version == PROJECTION_VERSION
                && self.account.cash.micros() == self.cash.account_cash,
            "account cash differs from execution partition",
        )?;
        for (code, window) in &self.valuation_windows {
            window.validate()?;
            require(
                code == &window.instrument_code
                    && self.account.marks.get(code) == Some(&mark_from_window(window)),
                "recorded valuation window differs from original mark",
            )?;
        }
        let lot_ids: BTreeSet<_> = self
            .account
            .lots
            .iter()
            .map(|l| l.lot_id.as_str())
            .collect();
        require(
            lot_ids.len() == self.account.lots.len()
                && lot_ids == self.lot_assignments.keys().map(String::as_str).collect(),
            "full lot dispositions differ",
        )?;
        let mut total_reserve = 0_i128;
        let mut claims: BTreeMap<&str, u32> = BTreeMap::new();
        for (id, p) in &self.parents {
            p.intent.validate()?;
            require(
                id == &p.intent.parent_id
                    && p.filled
                        .checked_add(p.remaining)
                        .and_then(|n| n.checked_add(p.cancelled))
                        == Some(p.intent.quantity),
                "parent quantity differs",
            )?;
            if p.status.working() {
                require(
                    p.remaining > 0 && p.remaining % 100 == 0,
                    "working parent remainder invalid",
                )?;
                total_reserve = total_reserve
                    .checked_add(i128::from(p.reservation.cash_reserve))
                    .ok_or(LedgerError::Overflow)?;
                require(
                    p.reservation.parent_id == *id
                        && p.reservation.code == p.intent.instrument_code
                        && p.reservation.chain_id == p.intent.chain_id,
                    "reservation owner differs",
                )?;
                require(
                    budget::checked(
                        i128::from(p.reservation.buy_max_notional)
                            + i128::from(p.reservation.fee_reserve),
                    )? == p.reservation.cash_reserve,
                    "reservation components differ",
                )?;
                for c in &p.sell_claims {
                    let sum = claims.entry(&c.lot_id).or_default();
                    *sum = sum.checked_add(c.quantity).ok_or(LedgerError::Overflow)?;
                }
            } else {
                require(
                    p.reservation.cash_reserve == 0
                        && p.reservation.buy_max_notional == 0
                        && p.reservation.fee_reserve == 0
                        && p.sell_claims.is_empty(),
                    "terminal parent retains reservation",
                )?;
            }
        }
        require(
            budget::checked(total_reserve)? <= self.cash.strategy_cash,
            "working reservation exceeds strategy cash",
        )?;
        for (id, claimed) in claims {
            let lot = self
                .account
                .lots
                .iter()
                .find(|l| l.lot_id == id)
                .ok_or_else(|| {
                    LedgerError::IntegrityFailure("claim references absent lot".into())
                })?;
            require(claimed <= lot.quantity, "sell claims overbook lot")?;
        }
        Ok(())
    }
    fn reservations(&self) -> Vec<WorkingReservation> {
        self.parents
            .values()
            .filter(|p| p.status.working())
            .map(|p| p.reservation.clone())
            .collect()
    }
    fn marked_at(&self, at: DateTime<Utc>) -> Result<Vec<MarkedAllocation>, LedgerError> {
        let day = checked_shanghai_local(at)?.date_naive();
        self.account
            .lots
            .iter()
            .filter_map(|lot| {
                self.lot_assignments
                    .get(&lot.lot_id)
                    .and_then(|c| c.as_ref())
                    .map(|c| (lot, c))
            })
            .map(|(lot, chain)| {
                let mark = self.account.marks.get(&lot.code).ok_or_else(|| {
                    LedgerError::EvidenceUnavailable("allocated holding mark absent".into())
                })?;
                require(
                    checked_shanghai_local(mark.observed_at)?.date_naive() == day,
                    "allocated holding mark is not current session",
                )?;
                let window = self.valuation_windows.get(&lot.code).ok_or_else(|| {
                    LedgerError::EvidenceUnavailable(
                        "allocated holding qualified valuation window absent".into(),
                    )
                })?;
                window.validate()?;
                require(
                    mark == &mark_from_window(window)
                        && window.session_date == day
                        && window.observed_at <= at
                        && at <= window.fresh_through,
                    "allocated holding qualified valuation window expired or differs",
                )?;
                Ok(MarkedAllocation {
                    code: lot.code.clone(),
                    chain_id: chain.clone(),
                    marked_value: budget::notional(mark.price.micros(), lot.quantity)?,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadIdentity {
    pub(crate) version: i64,
    pub(crate) event_hash: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "operation", content = "request")]
pub(crate) enum CommandRecord {
    Open {
        manifest: ExecutionManifest,
    },
    Submit {
        expected: HeadIdentity,
        intent: IntentRecord,
    },
    Evaluate {
        expected: HeadIdentity,
        parent_id: String,
        window: WindowRecord,
    },
    Cancel {
        expected: HeadIdentity,
        parent_id: String,
        at: DateTime<Utc>,
    },
    Expire {
        expected: HeadIdentity,
        parent_id: String,
        at: DateTime<Utc>,
    },
    QualifiedMarks {
        expected: HeadIdentity,
        windows: Vec<WindowRecord>,
    },
}
impl CommandRecord {
    fn expected(&self) -> Option<&HeadIdentity> {
        match self {
            Self::Open { .. } => None,
            Self::Submit { expected, .. }
            | Self::Evaluate { expected, .. }
            | Self::Cancel { expected, .. }
            | Self::Expire { expected, .. }
            | Self::QualifiedMarks { expected, .. } => Some(expected),
        }
    }
    fn parent_id(&self) -> Option<&str> {
        match self {
            Self::Submit { intent, .. } => Some(&intent.parent_id),
            Self::Evaluate { parent_id, .. }
            | Self::Cancel { parent_id, .. }
            | Self::Expire { parent_id, .. } => Some(parent_id),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "effect", content = "record")]
enum Effect {
    Opened,
    Submitted(ParentState),
    ObservedNoFill(fill_model::NoFillReason),
    Filled(FillRecord),
    Cancelled,
    Expired,
    Marks,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fact {
    request: CommandRecord,
    effect: Effect,
}
impl Fact {
    fn kind(&self) -> &'static str {
        match &self.effect {
            Effect::Opened => "ExecutionOpenedV1",
            Effect::Submitted(_) => "ParentSubmittedV1",
            Effect::ObservedNoFill(_) => "ParentObservedNoFillV1",
            Effect::Filled(_) => "ParentFilledV1",
            Effect::Cancelled => "ParentCancelledV1",
            Effect::Expired => "ParentExpiredV1",
            Effect::Marks => "QualifiedMarksV1",
        }
    }
}

fn require_fee_scope(policy: &AShareFeePolicyV2, window: &WindowRecord) -> Result<(), LedgerError> {
    require(
        matches!(
            (policy.scope().segment(), window.fee_segment.as_str()),
            (FeeListingSegment::ShanghaiMainA, "ShanghaiMainA")
                | (FeeListingSegment::ShanghaiStarA, "ShanghaiStarA")
        ),
        "recorded admitted board differs from fee scope",
    )
}
fn command_hash(request: &CommandRecord) -> Result<String, LedgerError> {
    // The real capture time stays in the event. Cancel/Expire retry identity
    // binds its original head and parent, not a newly sampled wall clock.
    match request {
        CommandRecord::Cancel {
            expected,
            parent_id,
            ..
        } => hash("paper-parent-command/v1", &("Cancel", expected, parent_id)),
        CommandRecord::Expire {
            expected,
            parent_id,
            ..
        } => hash("paper-parent-command/v1", &("Expire", expected, parent_id)),
        _ => hash("paper-parent-command/v1", request),
    }
}
fn require_live_request(request: &CommandRecord, now: DateTime<Utc>) -> Result<(), LedgerError> {
    let check = |window: &WindowRecord| {
        require(
            window.observed_at <= now && now <= window.fresh_through,
            "live execution window expired",
        )
    };
    match request {
        CommandRecord::Submit { intent, .. } => check(&intent.source_window),
        CommandRecord::Evaluate { window, .. } => check(window),
        CommandRecord::QualifiedMarks { windows, .. } => {
            for w in windows {
                check(w)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn reservation(
    intent: &IntentRecord,
    remaining: u32,
    fee: &AShareFeePolicyV2,
) -> Result<WorkingReservation, LedgerError> {
    let fees = fill_model::worst_case_fee(
        intent.side,
        remaining,
        intent.fee_price_cap_micro_cny,
        intent.session_date,
        fee,
    )?;
    let value = if intent.side == Side::Buy && remaining > 0 {
        budget::notional(intent.fee_price_cap_micro_cny, remaining)?
    } else {
        0
    };
    Ok(WorkingReservation {
        parent_id: intent.parent_id.clone(),
        code: intent.instrument_code.clone(),
        chain_id: intent.chain_id.clone(),
        buy_max_notional: value,
        fee_reserve: fees,
        cash_reserve: budget::checked(i128::from(value) + i128::from(fees))?,
    })
}
fn mark_from_window(window: &WindowRecord) -> Mark {
    Mark {
        code: window.instrument_code.clone(),
        price: Money::from_micros(window.price_micro_cny),
        observed_at: window.observed_at,
        source: window.source_reference.clone(),
    }
}
fn apply_request(
    state: &mut ExecutionProjection,
    manifest: &ExecutionManifest,
    request: &CommandRecord,
) -> Result<Effect, LedgerError> {
    let mut staged = state.clone();
    let effect = apply_request_body(&mut staged, manifest, request)?;
    *state = staged;
    Ok(effect)
}

fn apply_request_body(
    state: &mut ExecutionProjection,
    manifest: &ExecutionManifest,
    request: &CommandRecord,
) -> Result<Effect, LedgerError> {
    let fee = manifest.validate()?;
    match request {
        CommandRecord::Open { .. } => Err(LedgerError::IdentityConflict),
        CommandRecord::Submit { intent, .. } => {
            intent.validate()?;
            require(
                intent.account_id == manifest.account_id
                    && intent.epoch_id == manifest.epoch_id
                    && intent.execution_manifest_hash == manifest.identity()?
                    && intent.family_id == manifest.budget.family_id,
                "submit manifest owner differs",
            )?;
            if state.parents.contains_key(&intent.parent_id)
                || state
                    .parents
                    .values()
                    .any(|p| p.intent.investment_decision_id == intent.investment_decision_id)
            {
                return Err(LedgerError::IdentityConflict);
            }
            require(
                intent.session_date >= manifest.budget.effective_from
                    && intent.session_date <= manifest.budget.effective_through,
                "budget policy session not effective",
            )?;
            require_fee_scope(&fee, &intent.source_window)?;
            require(
                intent.approved_at >= state.account.as_of,
                "parent observation precedes prior financial fact",
            )?;
            state.account.marks.insert(
                intent.instrument_code.clone(),
                mark_from_window(&intent.source_window),
            );
            state
                .valuation_windows
                .insert(intent.instrument_code.clone(), intent.source_window.clone());
            let reserve = reservation(intent, intent.quantity, &fee)?;
            let mut claims = Vec::new();
            match intent.side {
                Side::Buy => budget::require_new_buy(
                    &manifest.budget,
                    &state.cash,
                    &state.marked_at(intent.approved_at)?,
                    &state.reservations(),
                    &reserve,
                )?,
                Side::Sell => {
                    let existing = state.reservations().iter().try_fold(0_i128, |sum, r| {
                        sum.checked_add(i128::from(r.cash_reserve))
                            .ok_or(LedgerError::Overflow)
                    })?;
                    require(
                        budget::checked(existing + i128::from(reserve.cash_reserve))?
                            <= state.cash.strategy_cash,
                        "strategy cash cannot reserve sell fees",
                    )?;
                    let mut left = intent.quantity;
                    let mut lots: Vec<_> = state
                        .account
                        .lots
                        .iter()
                        .filter(|lot| {
                            lot.code == intent.instrument_code
                                && lot.sellable_from <= intent.session_date
                                && state.lot_assignments.get(&lot.lot_id)
                                    == Some(&Some(intent.chain_id.clone()))
                        })
                        .collect();
                    lots.sort_by(|a, b| {
                        (a.acquired_on, &a.lot_id).cmp(&(b.acquired_on, &b.lot_id))
                    });
                    for lot in lots {
                        let reserved = state
                            .parents
                            .values()
                            .filter(|p| p.status.working())
                            .flat_map(|p| p.sell_claims.iter())
                            .filter(|c| c.lot_id == lot.lot_id)
                            .try_fold(0_u32, |sum, c| {
                                sum.checked_add(c.quantity).ok_or(LedgerError::Overflow)
                            })?;
                        let take = left.min(
                            lot.quantity
                                .checked_sub(reserved)
                                .ok_or(LedgerError::Overflow)?,
                        );
                        if take > 0 {
                            claims.push(LotClaim {
                                lot_id: lot.lot_id.clone(),
                                quantity: take,
                            });
                            left -= take;
                        }
                        if left == 0 {
                            break;
                        }
                    }
                    require(
                        left == 0,
                        "allocated FIFO sellable shares unavailable or already reserved",
                    )?;
                }
            }
            let parent = ParentState {
                intent: intent.clone(),
                status: ParentStatus::Working,
                filled: 0,
                remaining: intent.quantity,
                cancelled: 0,
                reservation: reserve,
                sell_claims: claims,
            };
            state
                .parents
                .insert(intent.parent_id.clone(), parent.clone());
            state.account.as_of = intent.approved_at;
            state.validate()?;
            Ok(Effect::Submitted(parent))
        }
        CommandRecord::Evaluate {
            parent_id, window, ..
        } => {
            window.validate()?;
            require_fee_scope(&fee, window)?;
            require(
                window.observed_at >= state.account.as_of,
                "fill observation precedes prior financial fact",
            )?;
            let original = state
                .parents
                .get(parent_id)
                .cloned()
                .ok_or(LedgerError::IdentityConflict)?;
            require(
                original.status.working()
                    && window.instrument_code == original.intent.instrument_code
                    && window.session_date == original.intent.session_date,
                "window does not match working day parent",
            )?;
            if state.used_windows.contains_key(&window.observation_id) {
                return Err(LedgerError::IdentityConflict);
            }
            state.used_windows.insert(
                window.observation_id.clone(),
                hash("paper-execution-window/v1", window)?,
            );
            let result = fill_model::model(
                original.intent.side,
                original.remaining,
                original.intent.limit_micro_cny,
                original.intent.fee_price_cap_micro_cny,
                window,
                &fee,
            )?;
            state
                .account
                .marks
                .insert(window.instrument_code.clone(), mark_from_window(window));
            state
                .valuation_windows
                .insert(window.instrument_code.clone(), window.clone());
            state.account.as_of = window.observed_at;
            let effect = match result {
                ModelOutcome::NoFill(reason) => Effect::ObservedNoFill(reason),
                ModelOutcome::Fill(modeled) => {
                    let mut parent = original;
                    parent.filled = parent
                        .filled
                        .checked_add(modeled.quantity)
                        .ok_or(LedgerError::Overflow)?;
                    parent.remaining = parent
                        .remaining
                        .checked_sub(modeled.quantity)
                        .ok_or(LedgerError::Overflow)?;
                    let fill_id = hash(
                        "paper-parent-fill-id/v1",
                        &(
                            manifest.account_id.as_str(),
                            parent_id.as_str(),
                            window.observation_id.as_str(),
                        ),
                    )?;
                    let mut inherited_fee = 0_i128;
                    let mut basis = 0_i128;
                    match parent.intent.side {
                        Side::Buy => {
                            let debit = budget::checked(
                                i128::from(modeled.notional_micro_cny)
                                    + i128::from(modeled.total_fee_micro_cny),
                            )?;
                            state.cash.apply_strategy_delta(
                                debit.checked_neg().ok_or(LedgerError::Overflow)?,
                            )?;
                            require(
                                !state.lot_assignments.contains_key(&fill_id),
                                "duplicate fill lot",
                            )?;
                            state.account.lots.push(Lot {
                                lot_id: fill_id.clone(),
                                code: parent.intent.instrument_code.clone(),
                                name: parent.intent.instrument_name.clone(),
                                quantity: modeled.quantity,
                                basis_price: Money::from_micros(modeled.price_micro_cny),
                                buy_fee_remaining: Money::from_micros(modeled.total_fee_micro_cny),
                                acquired_on: window.session_date,
                                sellable_from: modeled.sellable_from,
                                reported_cost: None,
                            });
                            state
                                .lot_assignments
                                .insert(fill_id.clone(), Some(parent.intent.chain_id.clone()));
                        }
                        Side::Sell => {
                            let mut left = modeled.quantity;
                            for claim in &mut parent.sell_claims {
                                if left == 0 {
                                    break;
                                }
                                let take = left.min(claim.quantity);
                                if take == 0 {
                                    continue;
                                }
                                let lot = state
                                    .account
                                    .lots
                                    .iter_mut()
                                    .find(|l| l.lot_id == claim.lot_id)
                                    .ok_or_else(|| {
                                        LedgerError::IntegrityFailure(
                                            "reserved sell lot disappeared".into(),
                                        )
                                    })?;
                                require(
                                    lot.sellable_from <= window.session_date
                                        && state.lot_assignments.get(&lot.lot_id)
                                            == Some(&Some(parent.intent.chain_id.clone())),
                                    "sell lot is not assigned/sellable",
                                )?;
                                let allocated = if take == lot.quantity {
                                    lot.buy_fee_remaining.micros()
                                } else {
                                    budget::checked(
                                        i128::from(lot.buy_fee_remaining.micros())
                                            * i128::from(take)
                                            / i128::from(lot.quantity),
                                    )?
                                };
                                inherited_fee = inherited_fee
                                    .checked_add(i128::from(allocated))
                                    .ok_or(LedgerError::Overflow)?;
                                basis = basis
                                    .checked_add(i128::from(budget::notional(
                                        lot.basis_price.micros(),
                                        take,
                                    )?))
                                    .ok_or(LedgerError::Overflow)?;
                                lot.quantity -= take;
                                lot.buy_fee_remaining = Money::from_micros(
                                    lot.buy_fee_remaining
                                        .micros()
                                        .checked_sub(allocated)
                                        .ok_or(LedgerError::Overflow)?,
                                );
                                claim.quantity -= take;
                                left -= take;
                            }
                            require(left == 0, "fill exceeds reserved FIFO shares")?;
                            parent.sell_claims.retain(|c| c.quantity > 0);
                            let empty: Vec<_> = state
                                .account
                                .lots
                                .iter()
                                .filter(|l| l.quantity == 0)
                                .map(|l| l.lot_id.clone())
                                .collect();
                            state.account.lots.retain(|l| l.quantity > 0);
                            for id in empty {
                                state.lot_assignments.remove(&id);
                            }
                            state.cash.apply_strategy_delta(budget::checked(
                                i128::from(modeled.notional_micro_cny)
                                    - i128::from(modeled.total_fee_micro_cny),
                            )?)?;
                        }
                    }
                    let realized = if parent.intent.side == Side::Sell {
                        budget::checked(
                            i128::from(modeled.notional_micro_cny)
                                - i128::from(modeled.total_fee_micro_cny)
                                - basis
                                - inherited_fee,
                        )?
                    } else {
                        0
                    };
                    state.account.realized_pnl = Money::from_micros(budget::checked(
                        i128::from(state.account.realized_pnl.micros()) + i128::from(realized),
                    )?);
                    state.account.fees = Money::from_micros(budget::checked(
                        i128::from(state.account.fees.micros())
                            + i128::from(modeled.total_fee_micro_cny),
                    )?);
                    state.account.cash = Money::from_micros(state.cash.account_cash);
                    parent.reservation = reservation(&parent.intent, parent.remaining, &fee)?;
                    parent.status = if parent.remaining == 0 {
                        ParentStatus::Filled
                    } else {
                        ParentStatus::PartiallyFilled
                    };
                    let record = FillRecord {
                        fill_id,
                        parent_id: parent_id.clone(),
                        observation_id: window.observation_id.clone(),
                        executed_at: window.observed_at,
                        side: parent.intent.side,
                        model: modeled,
                        inherited_buy_fee_micro_cny: budget::checked(inherited_fee)?,
                        realized_pnl_micro_cny: realized,
                    };
                    state.parents.insert(parent_id.clone(), parent);
                    state.fills.push(record.clone());
                    Effect::Filled(record)
                }
            };
            state.validate()?;
            Ok(effect)
        }
        CommandRecord::Cancel { parent_id, at, .. }
        | CommandRecord::Expire { parent_id, at, .. } => {
            let parent = state
                .parents
                .get_mut(parent_id)
                .ok_or(LedgerError::IdentityConflict)?;
            require(
                parent.status.working() && *at >= state.account.as_of,
                "cancel/expire not current working order",
            )?;
            let expired = matches!(request, CommandRecord::Expire { .. });
            if expired {
                let local = checked_shanghai_local(*at)?;
                let day = local.date_naive();
                require(
                    (day > parent.intent.session_date
                        || (day == parent.intent.session_date
                            && local.time().num_seconds_from_midnight() >= 15 * 3600))
                        && crate::calendar::verified_a_share_trading_day(day)
                            .map_err(LedgerError::EvidenceUnavailable)?,
                    "day order not yet expired on verified session",
                )?;
            }
            parent.status = if expired {
                ParentStatus::Expired
            } else {
                ParentStatus::Cancelled
            };
            parent.cancelled = parent.remaining;
            parent.remaining = 0;
            parent.reservation = reservation(&parent.intent, 0, &fee)?;
            parent.sell_claims.clear();
            state.account.as_of = *at;
            state.validate()?;
            Ok(if expired {
                Effect::Expired
            } else {
                Effect::Cancelled
            })
        }
        CommandRecord::QualifiedMarks { windows, .. } => {
            require(!windows.is_empty(), "qualified mark set empty")?;
            let at = windows[0].observed_at;
            let mut seen = BTreeSet::new();
            require(
                at >= state.account.as_of,
                "mark precedes prior financial fact",
            )?;
            for window in windows {
                window.validate()?;
                require(
                    window.observed_at == at && seen.insert(window.instrument_code.as_str()),
                    "mark set date/code duplicate",
                )?;
                state
                    .account
                    .marks
                    .insert(window.instrument_code.clone(), mark_from_window(window));
                state
                    .valuation_windows
                    .insert(window.instrument_code.clone(), window.clone());
            }
            let holdings: BTreeSet<_> =
                state.account.lots.iter().map(|l| l.code.as_str()).collect();
            require(
                holdings.iter().all(|c| seen.contains(c)),
                "marks omit full account holding",
            )?;
            state.account.as_of = at;
            state.validate()?;
            Ok(Effect::Marks)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, QueryableByName)]
struct ManifestRow {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    epoch_id: String,
    #[diesel(sql_type=Text)]
    cutover_id: String,
    #[diesel(sql_type=Text)]
    genesis_event_hash: String,
    #[diesel(sql_type=Text)]
    manifest_hash: String,
    #[diesel(sql_type=Binary)]
    manifest_bytes: Vec<u8>,
    #[diesel(sql_type=Text)]
    fee_policy_instance_id: String,
    #[diesel(sql_type=Text)]
    budget_policy_hash: String,
    #[diesel(sql_type=Text)]
    fill_model_version: String,
    #[diesel(sql_type=Text)]
    family_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, QueryableByName)]
struct ParentRow {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    parent_id: String,
    #[diesel(sql_type=Text)]
    epoch_id: String,
    #[diesel(sql_type=Text)]
    manifest_hash: String,
    #[diesel(sql_type=Text)]
    investment_decision_id: String,
    #[diesel(sql_type=Text)]
    side: String,
    #[diesel(sql_type=Text)]
    instrument_code: String,
    #[diesel(sql_type=BigInt)]
    requested_quantity: i64,
    #[diesel(sql_type=BigInt)]
    max_price_micro_cny: i64,
    #[diesel(sql_type=Text)]
    session_date: String,
    #[diesel(sql_type=Text)]
    intent_hash: String,
    #[diesel(sql_type=Binary)]
    intent_bytes: Vec<u8>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, QueryableByName)]
struct EventRow {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=BigInt)]
    seq: i64,
    #[diesel(sql_type=Text)]
    command_id: String,
    #[diesel(sql_type=Text)]
    command_hash: String,
    #[diesel(sql_type=Nullable<Text>)]
    parent_id: Option<String>,
    #[diesel(sql_type=Text)]
    previous_hash: String,
    #[diesel(sql_type=Text)]
    event_hash: String,
    #[diesel(sql_type=Text)]
    kind: String,
    #[diesel(sql_type=Binary)]
    payload: Vec<u8>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, QueryableByName)]
struct HeadRow {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=BigInt)]
    version: i64,
    #[diesel(sql_type=Text)]
    event_hash: String,
    #[diesel(sql_type=Text)]
    projection_hash: String,
    #[diesel(sql_type=Binary)]
    projection_bytes: Vec<u8>,
}
#[derive(QueryableByName)]
struct Scalar {
    #[diesel(sql_type=BigInt)]
    value: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SqlRows {
    manifests: Vec<ManifestRow>,
    parents: Vec<ParentRow>,
    events: Vec<EventRow>,
    heads: Vec<HeadRow>,
}
fn sql_rows(conn: &mut SqliteConnection) -> Result<SqlRows, LedgerError> {
    // SQLite affinity must not coerce arbitrary stored types into our DTOs.
    for (table, texts, integers, blobs, nullable) in [
        (
            "paper_book_v2_execution_manifest",
            &[
                "account_id",
                "epoch_id",
                "cutover_id",
                "genesis_event_hash",
                "manifest_hash",
                "fee_policy_instance_id",
                "budget_policy_hash",
                "fill_model_version",
                "family_id",
            ][..],
            &[][..],
            &["manifest_bytes"][..],
            &[][..],
        ),
        (
            "paper_book_v2_parent_order",
            &[
                "account_id",
                "parent_id",
                "epoch_id",
                "manifest_hash",
                "investment_decision_id",
                "side",
                "instrument_code",
                "session_date",
                "intent_hash",
            ][..],
            &["requested_quantity", "max_price_micro_cny"][..],
            &["intent_bytes"][..],
            &[][..],
        ),
        (
            "paper_book_v2_execution_event",
            &[
                "account_id",
                "command_id",
                "command_hash",
                "previous_hash",
                "event_hash",
                "kind",
            ][..],
            &["seq"][..],
            &["payload"][..],
            &["parent_id"][..],
        ),
        (
            "paper_book_v2_execution_head",
            &["account_id", "event_hash", "projection_hash"][..],
            &["version"][..],
            &["projection_bytes"][..],
            &[][..],
        ),
    ] {
        let mut predicates = Vec::new();
        for name in texts {
            predicates.push(format!("typeof({name})!='text'"));
        }
        for name in integers {
            predicates.push(format!("typeof({name})!='integer'"));
        }
        for name in blobs {
            predicates.push(format!("typeof({name})!='blob'"));
        }
        for name in nullable {
            predicates.push(format!("typeof({name}) NOT IN ('null','text')"));
        }
        let invalid = diesel::sql_query(format!(
            "SELECT COUNT(*) AS value FROM {table} WHERE {}",
            predicates.join(" OR ")
        ))
        .get_result::<Scalar>(conn)?
        .value;
        require(invalid == 0, "execution SQL cell type differs")?;
    }
    Ok(SqlRows{
        manifests:diesel::sql_query("SELECT account_id,epoch_id,cutover_id,genesis_event_hash,manifest_hash,manifest_bytes,fee_policy_instance_id,budget_policy_hash,fill_model_version,family_id FROM paper_book_v2_execution_manifest ORDER BY account_id").load(conn)?,
        parents:diesel::sql_query("SELECT account_id,parent_id,epoch_id,manifest_hash,investment_decision_id,side,instrument_code,requested_quantity,max_price_micro_cny,session_date,intent_hash,intent_bytes FROM paper_book_v2_parent_order ORDER BY account_id,parent_id").load(conn)?,
        events:diesel::sql_query("SELECT account_id,seq,command_id,command_hash,parent_id,previous_hash,event_hash,kind,payload FROM paper_book_v2_execution_event ORDER BY account_id,seq").load(conn)?,
        heads:diesel::sql_query("SELECT account_id,version,event_hash,projection_hash,projection_bytes FROM paper_book_v2_execution_head ORDER BY account_id").load(conn)?,
    })
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedExecutionView {
    pub(crate) manifest: ExecutionManifest,
    pub(crate) manifest_hash: String,
    pub(crate) head: HeadIdentity,
    pub(crate) projection: ExecutionProjection,
}
fn event_hash(
    account: &str,
    seq: i64,
    command: &str,
    previous: &str,
    payload: &[u8],
) -> Result<String, LedgerError> {
    hash(
        "paper-parent-event/v1",
        &(account, seq, command, previous, payload),
    )
}
fn parent_matches(row: &ParentRow, intent: &IntentRecord) -> Result<(), LedgerError> {
    require(
        row.account_id == intent.account_id
            && row.parent_id == intent.parent_id
            && row.epoch_id == intent.epoch_id
            && row.manifest_hash == intent.execution_manifest_hash
            && row.investment_decision_id == intent.investment_decision_id
            && row.side == intent.side.text()
            && row.instrument_code == intent.instrument_code
            && row.requested_quantity == i64::from(intent.quantity)
            && row.max_price_micro_cny == intent.fee_price_cap_micro_cny
            && row.session_date == intent.session_date.to_string()
            && row.intent_hash == hash("paper-parent-intent/v1", intent)?
            && row.intent_bytes == encode(intent)?,
        "immutable parent SQL/context differs",
    )
}

/// Closed pure row validator after the sole Global owner has validated the
/// complete CatalogV6 on this same connection. No catalog capture authority,
/// namespace capability or approval is issued here.
pub(crate) fn verify_rows_on(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    read_views_body_on(conn).map(|_| ())
}
/// Fixed7 pure historical replay, called after the sole Global catalog gate.
pub(crate) fn verify_rows_on_catalog7(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    crate::database::paper_book_v2_execution_schema_v1::verify_objects_on(conn)?;
    crate::database::paper_book_v2_schema::verify_v7_manifest_on(conn)
        .map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
    read_views_rows_on(conn).map(|_| ())
}
/// Fixed8 complete historical replay after the unique Global gate.
pub(crate) fn verify_rows_on_catalog8(conn: &mut SqliteConnection) -> Result<(), LedgerError> {
    crate::database::paper_book_v2_execution_schema_v1::verify_objects_on(conn)?;
    crate::database::paper_book_v2_schema::verify_v8_manifest_on(conn)
        .map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
    read_views_rows_on(conn).map(|_| ())
}
fn read_views_body_on(
    conn: &mut SqliteConnection,
) -> Result<BTreeMap<String, RecordedExecutionView>, LedgerError> {
    crate::database::paper_book_v2_execution_schema_v1::verify_objects_on(conn)?;
    crate::database::paper_book_v2_schema::verify_v6_manifest_on(conn)
        .map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
    read_views_rows_on(conn)
}
fn read_views_rows_on(
    conn: &mut SqliteConnection,
) -> Result<BTreeMap<String, RecordedExecutionView>, LedgerError> {
    super::paper_book_v2::verify_owner_rows_on(conn)?;
    let rows = sql_rows(conn)?;
    let mut views = BTreeMap::new();
    let accounts: BTreeSet<_> = rows
        .manifests
        .iter()
        .map(|r| r.account_id.as_str())
        .collect();
    require(
        accounts.len() == rows.manifests.len()
            && rows
                .parents
                .iter()
                .all(|r| accounts.contains(r.account_id.as_str()))
            && rows
                .events
                .iter()
                .all(|r| accounts.contains(r.account_id.as_str()))
            && rows
                .heads
                .iter()
                .all(|r| accounts.contains(r.account_id.as_str()))
            && rows.heads.len() == rows.manifests.len(),
        "orphan/duplicate execution account or head",
    )?;
    for row in &rows.manifests {
        let manifest: ExecutionManifest = decode(&row.manifest_bytes)?;
        let fee = manifest.validate()?;
        let genesis = super::paper_book_v2::read_verified_genesis_body_on(conn, &row.account_id)?;
        #[derive(QueryableByName)]
        struct FeeRow {
            #[diesel(sql_type=Binary)]
            descriptor_bytes: Vec<u8>,
            #[diesel(sql_type=Text)]
            policy_instance_id: String,
        }
        let original_fee=diesel::sql_query("SELECT descriptor_bytes,policy_instance_id FROM paper_book_v2_fee_manifest WHERE singleton=1").get_result::<FeeRow>(conn)?;
        require(
            row.account_id == manifest.account_id
                && row.epoch_id == manifest.epoch_id
                && row.cutover_id == manifest.cutover_id
                && row.genesis_event_hash == manifest.genesis_event_hash
                && row.manifest_hash == manifest.identity()?
                && row.fee_policy_instance_id == manifest.fee_policy_instance_id
                && row.budget_policy_hash == hash(budget::POLICY_VERSION, &manifest.budget)?
                && row.fill_model_version == manifest.fill_model_version
                && row.family_id == manifest.budget.family_id
                && genesis.epoch_id == manifest.epoch_id
                && genesis.cutover_id == manifest.cutover_id
                && genesis.event_hash == manifest.genesis_event_hash
                && genesis.projection_hash == manifest.genesis_projection_hash
                && genesis.fee_policy_instance_id == fee.instance_id()
                && original_fee.policy_instance_id == fee.instance_id()
                && original_fee.descriptor_bytes == manifest.fee_descriptor,
            "manifest/genesis/fee original bindings differ",
        )?;
        let initial: Projection = decode(&genesis.projection_bytes)?;
        let mut state = ExecutionProjection::initial(&initial, &manifest.budget)?;
        let events: Vec<_> = rows
            .events
            .iter()
            .filter(|e| e.account_id == row.account_id)
            .collect();
        require(!events.is_empty(), "execution opening missing")?;
        let parents: BTreeMap<_, _> = rows
            .parents
            .iter()
            .filter(|p| p.account_id == row.account_id)
            .map(|p| (p.parent_id.as_str(), p))
            .collect();
        let mut seen_parents = BTreeSet::new();
        let mut seen_decisions = BTreeSet::new();
        let mut seen_commands = BTreeSet::new();
        let mut previous = manifest.genesis_event_hash.clone();
        for (index, event) in events.iter().enumerate() {
            require(
                event.seq == i64::try_from(index).map_err(|_| LedgerError::Overflow)? + 1
                    && budget::token(&event.command_id)
                    && seen_commands.insert(event.command_id.as_str())
                    && event.previous_hash == previous,
                "event sequence/command/previous mismatch",
            )?;
            let fact: Fact = decode(&event.payload)?;
            require(
                event.command_hash == command_hash(&fact.request)?
                    && event.parent_id.as_deref() == fact.request.parent_id()
                    && event.kind == fact.kind()
                    && event.event_hash
                        == event_hash(
                            &row.account_id,
                            event.seq,
                            &event.command_id,
                            &previous,
                            &event.payload,
                        )?,
                "event canonical hash/context differs",
            )?;
            let expected = if index == 0 {
                require(
                    fact.request
                        == CommandRecord::Open {
                            manifest: manifest.clone(),
                        },
                    "opening original manifest differs",
                )?;
                Effect::Opened
            } else {
                require(
                    fact.request.expected()
                        == Some(&HeadIdentity {
                            version: event.seq - 1,
                            event_hash: previous.clone(),
                        }),
                    "command expected head differs",
                )?;
                if let CommandRecord::Submit { intent, .. } = &fact.request {
                    let parent = parents.get(intent.parent_id.as_str()).ok_or_else(|| {
                        LedgerError::IntegrityFailure("submit immutable parent missing".into())
                    })?;
                    parent_matches(parent, intent)?;
                    // Borrow the verified immutable row for the whole replay,
                    // rather than the event-local decoded Fact.
                    require(
                        seen_decisions.insert(parent.investment_decision_id.as_str()),
                        "duplicate investment decision parent",
                    )?;
                    require(
                        seen_parents.insert(parent.parent_id.as_str()),
                        "duplicate parent submission",
                    )?;
                }
                apply_request(&mut state, &manifest, &fact.request)?
            };
            require(
                expected == fact.effect,
                "stored effect differs from full deterministic replay",
            )?;
            previous = event.event_hash.clone();
        }
        require(
            seen_parents.len() == parents.len(),
            "orphan immutable parent",
        )?;
        let head = rows
            .heads
            .iter()
            .find(|h| h.account_id == row.account_id)
            .ok_or_else(|| LedgerError::IntegrityFailure("execution head missing".into()))?;
        let projection = encode(&state)?;
        require(
            head.version == events.last().unwrap().seq
                && head.event_hash == previous
                && head.projection_bytes == projection
                && head.projection_hash == raw_hash(&projection),
            "full execution projection/head differs",
        )?;
        views.insert(
            row.account_id.clone(),
            RecordedExecutionView {
                manifest,
                manifest_hash: row.manifest_hash.clone(),
                head: HeadIdentity {
                    version: head.version,
                    event_hash: head.event_hash.clone(),
                },
                projection: state,
            },
        );
    }
    Ok(views)
}

pub(crate) enum PaperV2Command {
    Submit {
        command_id: String,
        expected: HeadIdentity,
        approved: ApprovedPaperIntentV1,
    },
    Evaluate {
        command_id: String,
        expected: HeadIdentity,
        parent_id: String,
        window: QualifiedPaperExecutionWindowV1,
    },
    Cancel {
        command_id: String,
        expected: HeadIdentity,
        parent_id: String,
    },
    Expire {
        command_id: String,
        expected: HeadIdentity,
        parent_id: String,
    },
    QualifiedMarks {
        command_id: String,
        expected: HeadIdentity,
        windows: Vec<QualifiedPaperExecutionWindowV1>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PaperV2Receipt {
    pub(crate) account_id: String,
    pub(crate) command_id: String,
    pub(crate) head: HeadIdentity,
    pub(crate) replayed: bool,
}

/// Original committed command facts with a current observation witness.
/// No Deserialize or conversion to an approval/window/write capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedCommandReceipt {
    receipt: PaperV2Receipt,
    request: CommandRecord,
    observed_head: HeadIdentity,
}
impl RecordedCommandReceipt {
    pub(crate) fn receipt(&self) -> &PaperV2Receipt {
        &self.receipt
    }
    pub(crate) fn request(&self) -> &CommandRecord {
        &self.request
    }
    pub(crate) fn observed_head(&self) -> &HeadIdentity {
        &self.observed_head
    }
}

fn recover_command_body_on(
    conn: &mut SqliteConnection,
    account: &str,
    command: &str,
) -> Result<Option<RecordedCommandReceipt>, LedgerError> {
    require(budget::token(command), "command id invalid")?;
    let views = read_views_body_on(conn)?;
    let view = views.get(account).ok_or(LedgerError::NotSeeded)?;
    let rows = sql_rows(conn)?;
    let Some(event) = rows
        .events
        .iter()
        .find(|event| event.account_id == account && event.command_id == command)
    else {
        return Ok(None);
    };
    let original: Fact = decode(&event.payload)?;
    Ok(Some(RecordedCommandReceipt {
        receipt: PaperV2Receipt {
            account_id: account.into(),
            command_id: command.into(),
            head: HeadIdentity {
                version: event.seq,
                event_hash: event.event_hash.clone(),
            },
            replayed: true,
        },
        request: original.request,
        observed_head: view.head.clone(),
    }))
}
fn command_record(
    command: PaperV2Command,
    actual: &ActualExecutionBinding,
    now: DateTime<Utc>,
) -> Result<(String, CommandRecord), LedgerError> {
    let value = match command {
        PaperV2Command::Submit {
            command_id,
            expected,
            approved,
        } => {
            approved.require_binding(actual)?;
            (
                command_id,
                CommandRecord::Submit {
                    expected,
                    intent: approved.record().clone(),
                },
            )
        }
        PaperV2Command::Evaluate {
            command_id,
            expected,
            parent_id,
            window,
        } => {
            window.require_binding(actual)?;
            (
                command_id,
                CommandRecord::Evaluate {
                    expected,
                    parent_id,
                    window: window.record().clone(),
                },
            )
        }
        PaperV2Command::Cancel {
            command_id,
            expected,
            parent_id,
        } => (
            command_id,
            CommandRecord::Cancel {
                expected,
                parent_id,
                at: now,
            },
        ),
        PaperV2Command::Expire {
            command_id,
            expected,
            parent_id,
        } => (
            command_id,
            CommandRecord::Expire {
                expected,
                parent_id,
                at: now,
            },
        ),
        PaperV2Command::QualifiedMarks {
            command_id,
            expected,
            windows,
        } => {
            let mut recorded = Vec::new();
            for window in windows {
                window.require_binding(actual)?;
                recorded.push(window.record().clone());
            }
            (
                command_id,
                CommandRecord::QualifiedMarks {
                    expected,
                    windows: recorded,
                },
            )
        }
    };
    require(budget::token(&value.0), "command id invalid")?;
    Ok(value)
}
fn append_on(
    conn: &mut SqliteConnection,
    account: &str,
    command: &str,
    request: CommandRecord,
    now: DateTime<Utc>,
) -> Result<PaperV2Receipt, LedgerError> {
    let views = read_views_body_on(conn)?;
    let view = views.get(account).ok_or(LedgerError::NotSeeded)?;
    let rows = sql_rows(conn)?;
    let requested_hash = command_hash(&request)?;
    if let Some(existing) = rows
        .events
        .iter()
        .find(|e| e.account_id == account && e.command_id == command)
    {
        if existing.command_hash != requested_hash {
            return Err(LedgerError::IdentityConflict);
        }
        return Ok(PaperV2Receipt {
            account_id: account.into(),
            command_id: command.into(),
            head: HeadIdentity {
                version: existing.seq,
                event_hash: existing.event_hash.clone(),
            },
            replayed: true,
        });
    }
    if request.expected() != Some(&view.head) {
        return Err(LedgerError::VersionChanged);
    }
    require_live_request(&request, now)?;
    let mut projection = view.projection.clone();
    let effect = apply_request(&mut projection, &view.manifest, &request)?;
    if matches!(&request, CommandRecord::Submit { intent, .. } if intent.side == Side::Buy) {
        // The request's fixed observation time explains replay. New writes
        // additionally consume the original windows at the real TX time.
        projection.marked_at(now)?;
    }
    if let CommandRecord::Submit { intent, .. } = &request {
        diesel::sql_query("INSERT INTO paper_book_v2_parent_order(account_id,parent_id,epoch_id,manifest_hash,investment_decision_id,side,instrument_code,requested_quantity,max_price_micro_cny,session_date,intent_hash,intent_bytes) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind::<Text,_>(&intent.account_id).bind::<Text,_>(&intent.parent_id).bind::<Text,_>(&intent.epoch_id).bind::<Text,_>(&intent.execution_manifest_hash).bind::<Text,_>(&intent.investment_decision_id).bind::<Text,_>(intent.side.text()).bind::<Text,_>(&intent.instrument_code).bind::<BigInt,_>(i64::from(intent.quantity)).bind::<BigInt,_>(intent.fee_price_cap_micro_cny).bind::<Text,_>(intent.session_date.to_string()).bind::<Text,_>(hash("paper-parent-intent/v1",intent)?).bind::<Binary,_>(encode(intent)?).execute(conn)?;
    }
    let seq = view
        .head
        .version
        .checked_add(1)
        .ok_or(LedgerError::Overflow)?;
    let fact = Fact { request, effect };
    let payload = encode(&fact)?;
    let event = event_hash(account, seq, command, &view.head.event_hash, &payload)?;
    insert_event_on(
        conn,
        account,
        seq,
        command,
        &requested_hash,
        fact.request.parent_id(),
        &view.head.event_hash,
        &event,
        fact.kind(),
        &payload,
    )?;
    let raw = encode(&projection)?;
    let updated=diesel::sql_query("UPDATE paper_book_v2_execution_head SET version=?,event_hash=?,projection_hash=?,projection_bytes=? WHERE account_id=? AND version=? AND event_hash=?")
        .bind::<BigInt,_>(seq).bind::<Text,_>(&event).bind::<Text,_>(raw_hash(&raw)).bind::<Binary,_>(raw).bind::<Text,_>(account).bind::<BigInt,_>(view.head.version).bind::<Text,_>(&view.head.event_hash).execute(conn)?;
    require(updated == 1, "head compare-and-swap lost")?;
    verify_rows_on(conn)?;
    Ok(PaperV2Receipt {
        account_id: account.into(),
        command_id: command.into(),
        head: HeadIdentity {
            version: seq,
            event_hash: event,
        },
        replayed: false,
    })
}
#[allow(clippy::too_many_arguments)]
fn insert_event_on(
    conn: &mut SqliteConnection,
    account: &str,
    seq: i64,
    command: &str,
    command_hash: &str,
    parent: Option<&str>,
    previous: &str,
    event: &str,
    kind: &str,
    payload: &[u8],
) -> Result<(), LedgerError> {
    diesel::sql_query("INSERT INTO paper_book_v2_execution_event(account_id,seq,command_id,command_hash,parent_id,previous_hash,event_hash,kind,payload) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind::<Text,_>(account).bind::<BigInt,_>(seq).bind::<Text,_>(command).bind::<Text,_>(command_hash).bind::<Nullable<Text>,_>(parent).bind::<Text,_>(previous).bind::<Text,_>(event).bind::<Text,_>(kind).bind::<Binary,_>(payload).execute(conn)?;
    Ok(())
}

/// Global owns descriptor/catalog/loan checks and the true COMMIT boundary.
/// Structural qualification never supplies investment or source approval.
fn map_catalog(error: PaperCatalog6Error) -> LedgerError {
    LedgerError::EvidenceUnavailable(format!("paper Global Catalog6 unavailable: {error}"))
}
fn map_transaction(error: PaperCatalog6TransactionError<LedgerError>) -> LedgerError {
    match error {
        PaperCatalog6TransactionError::BeforeCommit(error) => map_catalog(error),
        PaperCatalog6TransactionError::Consumer(error) => error,
        PaperCatalog6TransactionError::CommitOutcomeUnknown(_)
        | PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(_) => {
            LedgerError::CommitOutcomeUnknown
        }
    }
}
fn map_readback(error: PaperCatalog6ReadbackError<LedgerError>) -> LedgerError {
    match error {
        PaperCatalog6ReadbackError::ObservationUnavailable(error) => map_catalog(error),
        PaperCatalog6ReadbackError::Consumer(error) => error,
    }
}

fn actual_binding(
    view: &RecordedExecutionView,
    authority: &DatabaseConnectionAuthority,
    db: &DatabaseManager,
) -> Result<ActualExecutionBinding, LedgerError> {
    let fee = view.manifest.validate()?;
    Ok(ActualExecutionBinding {
        account_id: view.manifest.account_id.clone(),
        epoch_id: view.manifest.epoch_id.clone(),
        manifest_hash: view.manifest_hash.clone(),
        family_id: view.manifest.budget.family_id.clone(),
        database_authority: authority.clone(),
        fee_segment: fee.scope().segment(),
        #[cfg(test)]
        isolated_test: db.has_isolated_p05_consumer_origin(),
    })
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TestPhase {
    AfterSql,
    LastSqlBeforeCommit,
    AfterRead,
}
#[cfg(test)]
thread_local! {static TEST_HOOK:std::cell::RefCell<Option<(TestPhase,Box<dyn FnOnce(&mut SqliteConnection)>)>>=std::cell::RefCell::new(None);}
#[cfg(test)]
pub(crate) struct TestHookGuard;
#[cfg(test)]
impl Drop for TestHookGuard {
    fn drop(&mut self) {
        TEST_HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}
#[cfg(test)]
pub(crate) fn install_test_hook(
    phase: TestPhase,
    hook: impl FnOnce(&mut SqliteConnection) + 'static,
) -> TestHookGuard {
    TEST_HOOK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some((phase, Box::new(hook)));
    });
    TestHookGuard
}
#[cfg(test)]
fn run_test_hook(phase: TestPhase, conn: &mut SqliteConnection) {
    let hook = TEST_HOOK.with(|slot| {
        let mut value = slot.borrow_mut();
        if value.as_ref().is_some_and(|(p, _)| *p == phase) {
            value.take().map(|(_, f)| f)
        } else {
            None
        }
    });
    if let Some(hook) = hook {
        hook(conn);
    }
}

/// Binding includes full canonical financial history, not just the current
/// projection. Original owner/genesis validation also runs at each boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SqlBinding {
    rows: SqlRows,
}
impl SqlBinding {
    fn capture(conn: &mut SqliteConnection) -> Result<Self, LedgerError> {
        verify_rows_on(conn)?;
        Ok(Self {
            rows: sql_rows(conn)?,
        })
    }
    fn validate(&self, conn: &mut SqliteConnection) -> Result<(), LedgerError> {
        verify_rows_on(conn)?;
        require(
            self.rows == sql_rows(conn)?,
            "execution SQL binding changed after operation",
        )
    }

    fn require_fresh_request(
        &self,
        account: &str,
        request: Option<&CommandRecord>,
        now: DateTime<Utc>,
    ) -> Result<(), LedgerError> {
        let Some(request) = request else {
            // Exact existing commands preserve the original receipt. They do
            // not resample, acquire a new source window or authorize a fill.
            return Ok(());
        };
        require_live_request(request, now)?;
        if matches!(request, CommandRecord::Submit { intent, .. } if intent.side == Side::Buy) {
            let head = self
                .rows
                .heads
                .iter()
                .find(|row| row.account_id == account)
                .ok_or(LedgerError::NotSeeded)?;
            let projection: ExecutionProjection = decode(&head.projection_bytes)?;
            projection.marked_at(now)?;
        }
        Ok(())
    }
}

/// The production clock is sampled only inside the actual transaction/tail.
/// A fixed clock can be consumed only by the constructor-issued Test manager.
fn operation_now(
    db: &DatabaseManager,
    test_now: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, LedgerError> {
    if let Some(now) = test_now {
        #[cfg(test)]
        {
            require(
                db.has_isolated_p05_consumer_origin(),
                "test clock requires actual isolated manager",
            )?;
            return Ok(now);
        }
        #[cfg(not(test))]
        {
            let _ = (db, now);
            return Err(LedgerError::EvidenceUnavailable(
                "caller supplied production clock unavailable".into(),
            ));
        }
    }
    Ok(Utc::now())
}

fn apply_on_actual_manager(
    db: &DatabaseManager,
    account: &str,
    command: PaperV2Command,
    test_now: Option<DateTime<Utc>>,
) -> Result<PaperV2Receipt, LedgerError> {
    let mut session = paper_catalog6_session(db).map_err(map_catalog)?;
    let (receipt, _, _) = session
        .with_immediate_catalog6(
            |conn, authority, _proof| {
                let views = read_views_body_on(conn)?;
                let view = views.get(account).ok_or(LedgerError::NotSeeded)?;
                let actual = actual_binding(view, authority, db)?;
                let now = operation_now(db, test_now)?;
                let (id, record) = command_record(command, &actual, now)?;
                let receipt = append_on(conn, account, &id, record.clone(), now)?;
                let fresh_request = (!receipt.replayed).then_some(record);
                let binding = SqlBinding::capture(conn)?;
                #[cfg(test)]
                if db.has_isolated_p05_consumer_origin() {
                    run_test_hook(TestPhase::AfterSql, conn);
                    run_test_hook(TestPhase::LastSqlBeforeCommit, conn);
                }
                Ok((receipt, binding, fresh_request))
            },
            |conn, _authority, _proof, (_, binding, fresh_request)| {
                // Global runs ALL hooks and the complete catalog/history gate
                // before this mandatory tail, on both writer and fresh reader.
                binding.validate(conn)?;
                binding.require_fresh_request(
                    account,
                    fresh_request.as_ref(),
                    operation_now(db, test_now)?,
                )
            },
        )
        .map_err(map_transaction)?;
    Ok(receipt)
}

/// The production manager is the original singleton only. Production approval
/// remains unavailable. Global qualification uses no fallback DB.
pub(crate) fn apply_actual(
    account: &str,
    command: PaperV2Command,
) -> Result<PaperV2Receipt, LedgerError> {
    let db = DatabaseManager::try_get().ok_or_else(|| {
        LedgerError::EvidenceUnavailable("production database singleton unavailable".into())
    })?;
    apply_on_actual_manager(db, account, command, None)
}
#[cfg(test)]
pub(crate) fn apply_for_isolated_test(
    db: &DatabaseManager,
    account: &str,
    command: PaperV2Command,
    now: DateTime<Utc>,
) -> Result<PaperV2Receipt, LedgerError> {
    require(
        db.has_isolated_p05_consumer_origin(),
        "test clock requires actual isolated manager",
    )?;
    apply_on_actual_manager(db, account, command, Some(now))
}
fn read_checked_on_actual_manager<T>(
    db: &DatabaseManager,
    operation: impl FnOnce(
        &mut SqliteConnection,
        &DatabaseConnectionAuthority,
    ) -> Result<T, LedgerError>,
    additional_tail: impl Fn(&mut SqliteConnection, &T) -> Result<(), LedgerError>,
) -> Result<T, LedgerError> {
    let mut session = paper_catalog6_session(db).map_err(map_catalog)?;
    let (view, _) = session
        .with_readonly_catalog6(
            |conn, proof| {
                let view = operation(conn, proof.connection_authority())?;
                let binding = SqlBinding::capture(conn)?;
                #[cfg(test)]
                if db.has_isolated_p05_consumer_origin() {
                    run_test_hook(TestPhase::AfterRead, conn);
                }
                Ok((view, binding))
            },
            |conn, _proof, (view, binding)| {
                binding.validate(conn)?;
                additional_tail(conn, view)
            },
        )
        .map_err(map_readback)?;
    Ok(view)
}
fn read_on_actual_manager(
    db: &DatabaseManager,
    account: &str,
) -> Result<RecordedExecutionView, LedgerError> {
    read_checked_on_actual_manager(
        db,
        |conn, _| {
            read_views_body_on(conn)?
                .remove(account)
                .ok_or(LedgerError::NotSeeded)
        },
        |_, _| Ok(()),
    )
}
pub(crate) fn read_actual(account: &str) -> Result<RecordedExecutionView, LedgerError> {
    let db = DatabaseManager::try_get().ok_or_else(|| {
        LedgerError::EvidenceUnavailable("production database singleton unavailable".into())
    })?;
    read_on_actual_manager(db, account)
}

pub(crate) fn recover_actual_command(
    account: &str,
    command: &str,
) -> Result<Option<RecordedCommandReceipt>, LedgerError> {
    let db = DatabaseManager::try_get().ok_or_else(|| {
        LedgerError::EvidenceUnavailable("production database singleton unavailable".into())
    })?;
    read_checked_on_actual_manager(
        db,
        |conn, _| recover_command_body_on(conn, account, command),
        |_, _| Ok(()),
    )
}

/// Explicit CatalogV6 historical observation through the same Global reader.
/// The old public V1–V5 readers/writers keep their existing catalog gates.
pub(crate) fn read_original_v1_actual(
    original: &super::paper_ledger::AccountBinding,
) -> Result<super::paper_ledger::PaperView, LedgerError> {
    let db = DatabaseManager::try_get().ok_or_else(|| {
        LedgerError::EvidenceUnavailable("production database singleton unavailable".into())
    })?;
    read_checked_on_actual_manager(
        db,
        |conn, _| super::paper_ledger::read_verified_original_v1_body_on(conn, original),
        |conn, expected| {
            require(
                super::paper_ledger::read_verified_original_v1_body_on(conn, original)?
                    == *expected,
                "original V1 observation changed after read",
            )
        },
    )
}
#[cfg(test)]
pub(crate) fn read_for_isolated_test(
    db: &DatabaseManager,
    account: &str,
) -> Result<RecordedExecutionView, LedgerError> {
    require(
        db.has_isolated_p05_consumer_origin(),
        "test reader requires actual isolated manager",
    )?;
    read_on_actual_manager(db, account)
}
#[cfg(test)]
pub(crate) fn binding_for_isolated_test(
    db: &DatabaseManager,
    account: &str,
) -> Result<ActualExecutionBinding, LedgerError> {
    require(
        db.has_isolated_p05_consumer_origin(),
        "test issuer requires actual isolated manager",
    )?;
    read_checked_on_actual_manager(
        db,
        |conn, authority| {
            let views = read_views_body_on(conn)?;
            actual_binding(
                views.get(account).ok_or(LedgerError::NotSeeded)?,
                authority,
                db,
            )
        },
        |_, _| Ok(()),
    )
}

#[cfg(test)]
pub(crate) fn recover_command_for_isolated_test(
    db: &DatabaseManager,
    account: &str,
    command: &str,
) -> Result<Option<RecordedCommandReceipt>, LedgerError> {
    require(
        db.has_isolated_p05_consumer_origin(),
        "test command reader requires actual isolated manager",
    )?;
    read_checked_on_actual_manager(
        db,
        |conn, _| recover_command_body_on(conn, account, command),
        |_, _| Ok(()),
    )
}

#[cfg(test)]
fn open_body_on(
    conn: &mut SqliteConnection,
    account: &str,
    command: &str,
    manifest: ExecutionManifest,
) -> Result<PaperV2Receipt, LedgerError> {
    let before = read_views_body_on(conn)?;
    let command_record = CommandRecord::Open {
        manifest: manifest.clone(),
    };
    let identity = command_hash(&command_record)?;
    if let Some(view) = before.get(account) {
        let rows = sql_rows(conn)?;
        let original = rows
            .events
            .iter()
            .find(|e| e.account_id == account && e.seq == 1)
            .ok_or(LedgerError::NotSeeded)?;
        if original.command_id != command
            || original.command_hash != identity
            || view.manifest != manifest
        {
            return Err(LedgerError::IdentityConflict);
        }
        return Ok(PaperV2Receipt {
            account_id: account.into(),
            command_id: command.into(),
            head: HeadIdentity {
                version: 1,
                event_hash: original.event_hash.clone(),
            },
            replayed: true,
        });
    }
    manifest.validate()?;
    let genesis = super::paper_book_v2::read_verified_genesis_body_on(conn, account)?;
    require(
        manifest.account_id == account
            && manifest.epoch_id == genesis.epoch_id
            && manifest.cutover_id == genesis.cutover_id
            && manifest.genesis_event_hash == genesis.event_hash
            && manifest.genesis_projection_hash == genesis.projection_hash
            && budget::token(command),
        "execution opening differs from full original genesis",
    )?;
    let raw_genesis: Projection = decode(&genesis.projection_bytes)?;
    let projection = ExecutionProjection::initial(&raw_genesis, &manifest.budget)?;
    let manifest_hash = manifest.identity()?;
    diesel::sql_query("INSERT INTO paper_book_v2_execution_manifest(account_id,epoch_id,cutover_id,genesis_event_hash,manifest_hash,manifest_bytes,fee_policy_instance_id,budget_policy_hash,fill_model_version,family_id) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind::<Text,_>(account).bind::<Text,_>(&manifest.epoch_id).bind::<Text,_>(&manifest.cutover_id).bind::<Text,_>(&manifest.genesis_event_hash).bind::<Text,_>(&manifest_hash).bind::<Binary,_>(encode(&manifest)?).bind::<Text,_>(&manifest.fee_policy_instance_id).bind::<Text,_>(hash(budget::POLICY_VERSION,&manifest.budget)?).bind::<Text,_>(&manifest.fill_model_version).bind::<Text,_>(&manifest.budget.family_id).execute(conn)?;
    let payload = encode(&Fact {
        request: command_record,
        effect: Effect::Opened,
    })?;
    let event = event_hash(account, 1, command, &manifest.genesis_event_hash, &payload)?;
    insert_event_on(
        conn,
        account,
        1,
        command,
        &identity,
        None,
        &manifest.genesis_event_hash,
        &event,
        "ExecutionOpenedV1",
        &payload,
    )?;
    let raw = encode(&projection)?;
    diesel::sql_query("INSERT INTO paper_book_v2_execution_head(account_id,version,event_hash,projection_hash,projection_bytes) VALUES(?,1,?,?,?)").bind::<Text,_>(account).bind::<Text,_>(&event).bind::<Text,_>(raw_hash(&raw)).bind::<Binary,_>(raw).execute(conn)?;
    verify_rows_on(conn)?;
    Ok(PaperV2Receipt {
        account_id: account.into(),
        command_id: command.into(),
        head: HeadIdentity {
            version: 1,
            event_hash: event,
        },
        replayed: false,
    })
}
#[cfg(test)]
pub(crate) fn open_for_isolated_test(
    db: &DatabaseManager,
    account: &str,
    command: &str,
    manifest: ExecutionManifest,
) -> Result<PaperV2Receipt, LedgerError> {
    require(
        db.has_isolated_p05_consumer_origin(),
        "execution opening requires actual isolated manager",
    )?;
    let mut session = paper_catalog6_session(db).map_err(map_catalog)?;
    let (receipt, _) = session
        .with_immediate_catalog6(
            |conn, _authority, _proof| {
                let receipt = open_body_on(conn, account, command, manifest)?;
                let binding = SqlBinding::capture(conn)?;
                run_test_hook(TestPhase::AfterSql, conn);
                run_test_hook(TestPhase::LastSqlBeforeCommit, conn);
                Ok((receipt, binding))
            },
            |conn, _authority, _proof, (_, binding)| binding.validate(conn),
        )
        .map_err(map_transaction)?;
    Ok(receipt)
}

#[cfg(test)]
#[path = "paper_book_v2_execution_tests.rs"]
mod tests;
