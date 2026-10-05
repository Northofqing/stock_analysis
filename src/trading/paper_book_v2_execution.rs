//! CatalogV6 modeled parent orders. Old V1/V5 financial facts are immutable.
//! No broker, production approval, startup DDL, or JSON capability factory.

use crate::trading::paper_replay_financial_work_v1::{self as fw, FinancialWork, FinancialFailure, ClosedFinancialText as Txt};
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
    let bytes = serde_json::to_vec(value).map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
    fw::historical(require_execution_record_extent_with_work(&bytes, &mut FinancialWork::Historical))?;
    Ok(bytes)
}
pub(crate) fn decode<T: Serialize + DeserializeOwned>(bytes: &[u8]) -> Result<T, LedgerError> {
    fw::historical(require_execution_record_extent_with_work(&bytes, &mut FinancialWork::Historical))?;
    let value: T = serde_json::from_slice(bytes).map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?;
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
    fw::historical(checked_shanghai_local_with_work(at, &mut FinancialWork::Historical))
}
fn checked_shanghai_local_with_work(at: DateTime<Utc>, w: &mut FinancialWork<'_, '_>) -> fw::Result<DateTime<Utc>> {
    w.finish()?;
    w.option(at.checked_add_signed(chrono::Duration::hours(8)), Txt::Execution(fw::ExecutionText::PaperExecutionShanghaiClockExceedsSupportedRange))
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
        fw::historical(self.identity_with_work(&mut FinancialWork::Historical))
    }
    pub(crate) fn identity_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<String> {
        w.finish()?;
        w.fixed_hash(fw::ClosedFinancialHash::ExecutionManifest(self))
    }
    fn validate(&self) -> Result<AShareFeePolicyV2, LedgerError> {
        fw::historical(self.validate_with_work(&mut FinancialWork::Historical))
    }
    fn validate_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<AShareFeePolicyV2> {
        w.finish()?;
        self.budget.validate_shape_with_work(w)?;
        let fee = fee_from_record_with_work(&self.fee_descriptor, w)?;
        {
            let condition = self.version == MANIFEST_VERSION && self.fill_model_version == MODEL_VERSION && budget::token(&self.account_id) && budget::token(&self.epoch_id) && budget::token(&self.cutover_id) && budget::token(&self.approved_reference) && self.genesis_event_hash.len() == 64 && self.genesis_projection_hash.len() == 64 && self.fee_policy_instance_id == w.fee_instance(&fee)?;
            w.require(condition, Txt::Execution(fw::ExecutionText::ExecutionManifestInvalid))
        } ?;
        Ok(fee)
    }
}

/// A value-level fee descriptor reconstruction used only for exact replay.
/// The original immutable database descriptor must additionally match it.
fn fee_from_record(bytes: &[u8]) -> Result<AShareFeePolicyV2, LedgerError> {
    fw::historical(fee_from_record_with_work(bytes, &mut FinancialWork::Historical))
}
fn fee_from_record_with_work(bytes: &[u8], w: &mut FinancialWork<'_, '_>) -> fw::Result<AShareFeePolicyV2> {
    w.finish()?;
    let text=match std::str::from_utf8(bytes){
        Ok(t)=>t,
        Err(_)=>return Err(w.error(Txt::Execution(fw::ExecutionText::FeeDescriptorUTF8))?)
    };
    let mut fields=BTreeMap::new();
    for line in text.lines(){
        let (key, value)=w.option(line.split_once('='), Txt::Execution(fw::ExecutionText::FeeDescriptorField))?;
        let absent=w.descriptor(&mut fields, key, value)?.is_none();
        w.require(absent, Txt::Execution(fw::ExecutionText::DuplicateFeeDescriptorField))?;
    }
    let segment=match descriptor_get(&fields, DescriptorField::Segment, w)?{
        "ShanghaiMainA"=>FeeListingSegment::ShanghaiMainA,
        "ShanghaiStarA"=>FeeListingSegment::ShanghaiStarA,
        _=>return Err(w.error(Txt::Execution(fw::ExecutionText::FeeSegmentUnavailable))?)
    };
    let scope=fw::fee_evidence(QualifiedInstrument::new(FeeMarket::Shanghai, FeeSecurityKind::AShareStock, segment).map_err(Into::into), w)?;
    let num=descriptor_integer(&fields, DescriptorField::RateNum, w)?;
    let den=descriptor_integer(&fields, DescriptorField::RateDen, w)?;
    let rate=fw::fee_evidence(FeeRate::new(num, den).map_err(Into::into), w)?;
    let minimum=descriptor_integer(&fields, DescriptorField::Minimum, w)?;
    let transfer=descriptor_reason(&fields, DescriptorField::Transfer, w)?;
    let other=descriptor_reason(&fields, DescriptorField::Other, w)?;
    let revision=descriptor_get(&fields, DescriptorField::Revision, w)?;
    let result=AShareFeePolicyV2::new_with_work(scope, rate, minimum, FeeCoverage::new(transfer, other), revision, w);
    let policy=fw::fee_evidence(result, w)?;
    let canonical=w.fee_descriptor(&policy)?;
    w.require(canonical==bytes, Txt::Execution(fw::ExecutionText::FeeDescriptorIsNotCanonicalReviewedPolicy))?;
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
        fw::historical(Self::initial_with_work(genesis, budget, &mut FinancialWork::Historical))
    }
    pub(crate) fn initial_with_work(genesis: &Projection, budget: &BudgetRecord, w: &mut FinancialWork<'_, '_>) -> fw::Result<Self> {
        w.finish()?;
        let cash = budget.initial_cash_with_work(genesis, w)?;
        let mut lot_assignments=BTreeMap::new();
        for r in &budget.initial_lots {
            let key=w.copy(&r.lot_id)?;
            let value=if r.disposition==LotDisposition::AllocatedToStrategy{
                w.copy(&r.chain_id)?
            } else{
                None
            };
            w.insert(&mut lot_assignments, key, value)?;
        }
        let value = Self {
            version: w.text(Txt::ProjectionVersion)?,
            account: w.copy(genesis)?,
            cash,
            lot_assignments,
            parents: BTreeMap::new(),
            used_windows: BTreeMap::new(),
            fills: Vec::new(),
            valuation_windows: BTreeMap::new(),
        };
        value.validate_with_work(w)?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), LedgerError> {
        fw::historical(self.validate_with_work(&mut FinancialWork::Historical))
    }
    fn validate_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
        w.finish()?;
        self.cash.validate_with_work(w)?;
        {
            let condition = self.version == PROJECTION_VERSION && self.account.cash.micros() == self.cash.account_cash;
            w.require(condition, Txt::Execution(fw::ExecutionText::AccountCashDiffersFromExecutionPartition))
        } ?;
        for (code, window) in &self.valuation_windows {
            window.validate_with_work(w)?;
            {
                let condition = code == &window.instrument_code && self.account.marks.get(code) == Some(&mark_from_window_with_work(window, w)?);
                w.require(condition, Txt::Execution(fw::ExecutionText::RecordedValuationWindowDiffersFromOriginalMark))
            } ?;
        }
        let mut lot_ids=BTreeSet::new();
        for lot in &self.account.lots{
            w.set(&mut lot_ids, lot.lot_id.as_str())?;
        }
        {
            let condition = lot_ids.len() == self.account.lots.len() && {
                let mut assignments=BTreeSet::new();
                for key in self.lot_assignments.keys(){
                    w.set(&mut assignments, key.as_str())?;
                }
                lot_ids==assignments
            };
            w.require(condition, Txt::Execution(fw::ExecutionText::FullLotDispositionsDiffer))
        } ?;
        let mut total_reserve = 0_i128;
        let mut claims: BTreeMap<&str,
        u32> = BTreeMap::new();
        for (id, p) in &self.parents {
            p.intent.validate_with_work(w)?;
            {
                let condition = id == &p.intent.parent_id && p.filled .checked_add(p.remaining) .and_then(|n| n.checked_add(p.cancelled)) == Some(p.intent.quantity);
                w.require(condition, Txt::Execution(fw::ExecutionText::ParentQuantityDiffers))
            } ?;
            if p.status.working() {
                {
                    let condition = p.remaining > 0 && p.remaining % 100 == 0;
                    w.require(condition, Txt::Execution(fw::ExecutionText::WorkingParentRemainderInvalid))
                } ?;
                total_reserve = total_reserve .checked_add(i128::from(p.reservation.cash_reserve)) .ok_or(LedgerError::Overflow)?;
                {
                    let condition = p.reservation.parent_id == *id && p.reservation.code == p.intent.instrument_code && p.reservation.chain_id == p.intent.chain_id;
                    w.require(condition, Txt::Execution(fw::ExecutionText::ReservationOwnerDiffers))
                } ?;
                {
                    let condition = budget::checked( i128::from(p.reservation.buy_max_notional) + i128::from(p.reservation.fee_reserve), )? == p.reservation.cash_reserve;
                    w.require(condition, Txt::Execution(fw::ExecutionText::ReservationComponentsDiffer))
                } ?;
                for c in &p.sell_claims {
                    let sum = w.claim(&mut claims, &c.lot_id)?;
                    *sum = sum.checked_add(c.quantity).ok_or(LedgerError::Overflow)?;
                }
            } else {
                {
                    let condition = p.reservation.cash_reserve == 0 && p.reservation.buy_max_notional == 0 && p.reservation.fee_reserve == 0 && p.sell_claims.is_empty();
                    w.require(condition, Txt::Execution(fw::ExecutionText::TerminalParentRetainsReservation))
                } ?;
            }
        }
        {
            let condition = budget::checked(total_reserve)? <= self.cash.strategy_cash;
            w.require(condition, Txt::Execution(fw::ExecutionText::WorkingReservationExceedsStrategyCash))
        } ?;
        for (id, claimed) in claims {
            let lot=w.option(self.account.lots.iter().find(|l|l.lot_id==id), Txt::Execution(fw::ExecutionText::ClaimReferencesAbsentLot))?;
            {
                let condition = claimed <= lot.quantity;
                w.require(condition, Txt::Execution(fw::ExecutionText::SellClaimsOverbookLot))
            } ?;
        }
        Ok(())
    }
    fn reservations(&self)->Vec<WorkingReservation>{
        fw::historical(self.reservations_with_work(&mut FinancialWork::Historical)).expect("Historical reservation copies")
    }
    fn reservations_with_work(&self, w:&mut FinancialWork<'_, '_>)->fw::Result<Vec<WorkingReservation>>{
        w.finish()?;
        let mut out=Vec::new();
        for p in self.parents.values().filter(|p|p.status.working()){
            let v=w.copy(&p.reservation)?;
            w.push(&mut out, v)?;
        }
        Ok(out)
    }

    fn marked_at(&self, at: DateTime<Utc>) -> Result<Vec<MarkedAllocation>, LedgerError> {
        fw::historical(self.marked_at_with_work(at, &mut FinancialWork::Historical))
    }
    fn marked_at_with_work(&self, at: DateTime<Utc>, w: &mut FinancialWork<'_, '_>) -> fw::Result<Vec<MarkedAllocation>> {
        w.finish()?;
        let day=checked_shanghai_local_with_work(at, w)?.date_naive();
        let mut out=Vec::new();
        for (lot, chain) in self.account.lots.iter().filter_map(|lot|self.lot_assignments.get(&lot.lot_id).and_then(|c|c.as_ref()).map(|c|(lot, c))){
            let mark=w.option(self.account.marks.get(&lot.code), Txt::Execution(fw::ExecutionText::AllocatedHoldingMarkAbsent))?;
            {
                let condition = checked_shanghai_local_with_work(mark.observed_at, w)?.date_naive() == day;
                w.require(condition, Txt::Execution(fw::ExecutionText::AllocatedHoldingMarkIsNotCurrentSession))
            } ?;
            let window=w.option(self.valuation_windows.get(&lot.code), Txt::Execution(fw::ExecutionText::AllocatedHoldingQualifiedValuationWindowAbsent))?;
            window.validate_with_work(w)?;
            {
                let condition = mark == &mark_from_window_with_work(window, w)? && window.session_date == day && window.observed_at <= at && at <= window.fresh_through;
                w.require(condition, Txt::Execution(fw::ExecutionText::AllocatedHoldingQualifiedValuationWindowExpiredOrDiffers))
            } ?;
            let incoming=MarkedAllocation {
                code: w.copy(&lot.code)?,
                chain_id: w.copy(chain)?,
                marked_value: budget::notional_with_work(mark.price.micros(), lot.quantity, w)?,
            };
            w.push(&mut out, incoming)?;
        }
        Ok(out)
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
pub(crate) enum Effect {
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
    fw::historical(require_fee_scope_with_work(policy, window, &mut FinancialWork::Historical))
}
fn require_fee_scope_with_work(policy: &AShareFeePolicyV2, window: &WindowRecord, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    w.finish()?;
    {
        let condition = matches!( (policy.scope().segment(), window.fee_segment.as_str()), (FeeListingSegment::ShanghaiMainA, "ShanghaiMainA") | (FeeListingSegment::ShanghaiStarA, "ShanghaiStarA") );
        w.require(condition, Txt::Execution(fw::ExecutionText::RecordedAdmittedBoardDiffersFromFeeScope))
    }
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
fn reservation( intent: &IntentRecord, remaining: u32, fee: &AShareFeePolicyV2, ) -> Result<WorkingReservation, LedgerError> {
    fw::historical(reservation_with_work(intent, remaining, fee, &mut FinancialWork::Historical))
}
fn reservation_with_work( intent: &IntentRecord, remaining: u32, fee: &AShareFeePolicyV2, w: &mut FinancialWork<'_, '_>) -> fw::Result<WorkingReservation> {
    w.finish()?;
    let fees = fill_model::worst_case_fee_with_work( intent.side, remaining, intent.fee_price_cap_micro_cny, intent.session_date, fee, w, )?;
    let value = if intent.side == Side::Buy && remaining > 0 {
        budget::notional_with_work(intent.fee_price_cap_micro_cny, remaining, w)?
    } else {
        0
    };
    Ok(WorkingReservation {
        parent_id: w.copy(&intent.parent_id)?,
        code: w.copy(&intent.instrument_code)?,
        chain_id: w.copy(&intent.chain_id)?,
        buy_max_notional: value,
        fee_reserve: fees,
        cash_reserve: budget::checked(i128::from(value) + i128::from(fees))?,
    })
}
fn mark_from_window(window: &WindowRecord) -> Mark {
    fw::historical(mark_from_window_with_work(window, &mut FinancialWork::Historical)).expect("Historical Mark copy")
}
fn mark_from_window_with_work(window:&WindowRecord, w:&mut FinancialWork<'_, '_>)->fw::Result<Mark>{
    w.finish()?;
    Ok(Mark {
        code: w.copy(&window.instrument_code)?,
        price: Money::from_micros(window.price_micro_cny),
        observed_at: window.observed_at,
        source: w.copy(&window.source_reference)?,
    })
}
fn apply_request( state: &mut ExecutionProjection, manifest: &ExecutionManifest, request: &CommandRecord, ) -> Result<Effect, LedgerError> {
    fw::historical(apply_request_with_work(state, manifest, request, &mut FinancialWork::Historical))
}
pub(crate) fn apply_request_with_work( state: &mut ExecutionProjection, manifest: &ExecutionManifest, request: &CommandRecord, w: &mut FinancialWork<'_, '_>) -> fw::Result<Effect> {
    w.finish()?;
    let mut staged=w.copy(state)?;
    let effect=apply_request_body_with_work(&mut staged, manifest, request, w)?;
    w.finish()?;
    *state=staged;
    Ok(effect)
}

fn apply_request_body( state: &mut ExecutionProjection, manifest: &ExecutionManifest, request: &CommandRecord, ) -> Result<Effect, LedgerError> {
    fw::historical(apply_request_body_with_work(state, manifest, request, &mut FinancialWork::Historical))
}
fn apply_request_body_with_work( state: &mut ExecutionProjection, manifest: &ExecutionManifest, request: &CommandRecord, w: &mut FinancialWork<'_, '_>) -> fw::Result<Effect> {
    w.finish()?;
    let fee = manifest.validate_with_work(w)?;
    match request {
        CommandRecord::Open {
            ..
        }
        => Err(LedgerError::IdentityConflict.into()),
        CommandRecord::Submit {
            intent,
            ..
        }
        => {
            intent.validate_with_work(w)?;
            {
                let condition = intent.account_id == manifest.account_id && intent.epoch_id == manifest.epoch_id && intent.execution_manifest_hash == manifest.identity_with_work(w)? && intent.family_id == manifest.budget.family_id;
                w.require(condition, Txt::Execution(fw::ExecutionText::SubmitManifestOwnerDiffers))
            } ?;
            if state.parents.contains_key(&intent.parent_id) || state .parents .values() .any(|p| p.intent.investment_decision_id == intent.investment_decision_id) {
                return Err(LedgerError::IdentityConflict.into());
            }
            {
                let condition = intent.session_date >= manifest.budget.effective_from && intent.session_date <= manifest.budget.effective_through;
                w.require(condition, Txt::Execution(fw::ExecutionText::BudgetPolicySessionNotEffective))
            } ?;
            require_fee_scope_with_work(&fee, &intent.source_window, w)?;
            {
                let condition = intent.approved_at >= state.account.as_of;
                w.require(condition, Txt::Execution(fw::ExecutionText::ParentObservationPrecedesPriorFinancialFact))
            } ?;
            {
                let key=w.copy(&intent.instrument_code)?;
                let incoming=mark_from_window_with_work(&intent.source_window, w)?;
                w.insert(&mut state.account.marks, key, incoming)?;
            };
            {
                let key=w.copy(&intent.instrument_code)?;
                let incoming=w.copy(&intent.source_window)?;
                w.insert(&mut state.valuation_windows, key, incoming)?;
            };
            let reserve = reservation_with_work(intent, intent.quantity, &fee, w)?;
            let mut claims = Vec::new();
            match intent.side {
                Side::Buy => {
                    let marked=state.marked_at_with_work(intent.approved_at, w)?;
                    let reservations=state.reservations_with_work(w)?;
                    budget::require_new_buy_with_work(&manifest.budget, &state.cash, &marked, &reservations, &reserve, w)?;
                },
                Side::Sell => {
                    let existing = state.reservations_with_work(w)?.iter().try_fold(0_i128, |sum, r| {
                        sum.checked_add(i128::from(r.cash_reserve)) .ok_or(LedgerError::Overflow)
                    })?;
                    {
                        let condition = budget::checked(existing + i128::from(reserve.cash_reserve))? <= state.cash.strategy_cash;
                        w.require(condition, Txt::Execution(fw::ExecutionText::StrategyCashCannotReserveSellFees))
                    } ?;
                    let mut left = intent.quantity;
                    let mut lots=Vec::new();
                    for lot in state.account.lots.iter().filter(|lot| {
                        lot.code == intent.instrument_code && lot.sellable_from <= intent.session_date && state.lot_assignments.get(&lot.lot_id).and_then(Option::as_deref) == Some(intent.chain_id.as_str())
                    }) {
                        w.push(&mut lots, lot)?;
                    }
                    w.sort_fifo(&mut lots)?;
                    for lot in lots {
                        let reserved = state .parents .values() .filter(|p| p.status.working()) .flat_map(|p| p.sell_claims.iter()) .filter(|c| c.lot_id == lot.lot_id) .try_fold(0_u32, |sum, c| {
                            sum.checked_add(c.quantity).ok_or(LedgerError::Overflow)
                        })?;
                        let take = left.min( lot.quantity .checked_sub(reserved) .ok_or(LedgerError::Overflow)?, );
                        if take > 0 {
                            {
                                let incoming=LotClaim {
                                    lot_id: w.copy(&lot.lot_id)?,
                                    quantity: take,
                                };
                                w.push(&mut claims, incoming)?;
                            };
                            left -= take;
                        }
                        if left == 0 {
                            break;
                        }
                    }
                    {
                        let condition = left == 0;
                        w.require(condition, Txt::Execution(fw::ExecutionText::AllocatedFIFOSellableSharesUnavailableOrAlreadyReserved))
                    } ?;
                }
            }
            let parent = ParentState {
                intent: w.copy(intent)?,
                status: ParentStatus::Working,
                filled: 0,
                remaining: intent.quantity,
                cancelled: 0,
                reservation: reserve,
                sell_claims: claims,
            };
            {
                let key=w.copy(&intent.parent_id)?;
                let incoming=w.copy(&parent)?;
                w.insert(&mut state.parents, key, incoming)?;
            };
            state.account.as_of = intent.approved_at;
            state.validate_with_work(w)?;
            Ok(Effect::Submitted(parent))
        }
        CommandRecord::Evaluate {
            parent_id,
            window,
            ..
        }
        => {
            window.validate_with_work(w)?;
            require_fee_scope_with_work(&fee, window, w)?;
            {
                let condition = window.observed_at >= state.account.as_of;
                w.require(condition, Txt::Execution(fw::ExecutionText::FillObservationPrecedesPriorFinancialFact))
            } ?;
            let original = w.copy(state.parents.get(parent_id).ok_or(LedgerError::IdentityConflict)?)?;
            {
                let condition = original.status.working() && window.instrument_code == original.intent.instrument_code && window.session_date == original.intent.session_date;
                w.require(condition, Txt::Execution(fw::ExecutionText::WindowDoesNotMatchWorkingDayParent))
            } ?;
            if state.used_windows.contains_key(&window.observation_id) {
                return Err(LedgerError::IdentityConflict.into());
            }
            {
                let key=w.copy(&window.observation_id)?;
                let incoming=w.fixed_hash(fw::ClosedFinancialHash::ExecutionWindow(window))?;
                w.insert(&mut state.used_windows, key, incoming)?;
            };
            let result = fill_model::model_with_work( original.intent.side, original.remaining, original.intent.limit_micro_cny, original.intent.fee_price_cap_micro_cny, window, &fee, w, )?;
            {
                let key=w.copy(&window.instrument_code)?;
                let incoming=mark_from_window_with_work(window, w)?;
                w.insert(&mut state.account.marks, key, incoming)?;
            };
            {
                let key=w.copy(&window.instrument_code)?;
                let incoming=w.copy(window)?;
                w.insert(&mut state.valuation_windows, key, incoming)?;
            };
            state.account.as_of = window.observed_at;
            let effect = match result {
                ModelOutcome::NoFill(reason) => Effect::ObservedNoFill(reason),
                ModelOutcome::Fill(modeled) => {
                    let mut parent = original;
                    parent.filled = parent .filled .checked_add(modeled.quantity) .ok_or(LedgerError::Overflow)?;
                    parent.remaining = parent .remaining .checked_sub(modeled.quantity) .ok_or(LedgerError::Overflow)?;
                    let fill_id=w.fixed_hash(fw::ClosedFinancialHash::FillIdentity{
                        account:&manifest.account_id,
                        parent:parent_id,
                        observation:&window.observation_id
                    })?;
                    let mut inherited_fee = 0_i128;
                    let mut basis = 0_i128;
                    match parent.intent.side {
                        Side::Buy => {
                            let debit = budget::checked( i128::from(modeled.notional_micro_cny) + i128::from(modeled.total_fee_micro_cny), )?;
                            state.cash.apply_strategy_delta_with_work( debit.checked_neg().ok_or(LedgerError::Overflow)?, w, )?;
                            {
                                let condition = !state.lot_assignments.contains_key(&fill_id);
                                w.require(condition, Txt::Execution(fw::ExecutionText::DuplicateFillLot))
                            } ?;
                            {
                                let incoming=Lot {
                                    lot_id: w.copy(&fill_id)?,
                                    code: w.copy(&parent.intent.instrument_code)?,
                                    name: w.copy(&parent.intent.instrument_name)?,
                                    quantity: modeled.quantity,
                                    basis_price: Money::from_micros(modeled.price_micro_cny),
                                    buy_fee_remaining: Money::from_micros(modeled.total_fee_micro_cny),
                                    acquired_on: window.session_date,
                                    sellable_from: modeled.sellable_from,
                                    reported_cost: None,
                                };
                                w.push(&mut state.account.lots, incoming)?;
                            };
                            {
                                let key=w.copy(&fill_id)?;
                                let incoming=Some(w.copy(&parent.intent.chain_id)?);
                                w.insert(&mut state.lot_assignments, key, incoming)?;
                            };
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
                                let lot=w.option(state.account.lots.iter_mut().find(|l|l.lot_id==claim.lot_id), Txt::Execution(fw::ExecutionText::ReservedSellLotDisappeared))?;
                                {
                                    let condition = lot.sellable_from <= window.session_date && state.lot_assignments.get(&lot.lot_id).and_then(Option::as_deref) == Some(parent.intent.chain_id.as_str());
                                    w.require(condition, Txt::Execution(fw::ExecutionText::SellLotIsNotAssignedSellable))
                                } ?;
                                let allocated = if take == lot.quantity {
                                    lot.buy_fee_remaining.micros()
                                } else {
                                    budget::checked( i128::from(lot.buy_fee_remaining.micros()) * i128::from(take) / i128::from(lot.quantity), )?
                                };
                                inherited_fee = inherited_fee .checked_add(i128::from(allocated)) .ok_or(LedgerError::Overflow)?;
                                basis = basis .checked_add(i128::from(budget::notional_with_work( lot.basis_price.micros(), take, w, )?)) .ok_or(LedgerError::Overflow)?;
                                lot.quantity -= take;
                                lot.buy_fee_remaining = Money::from_micros( lot.buy_fee_remaining .micros() .checked_sub(allocated) .ok_or(LedgerError::Overflow)?, );
                                claim.quantity -= take;
                                left -= take;
                            }
                            {
                                let condition = left == 0;
                                w.require(condition, Txt::Execution(fw::ExecutionText::FillExceedsReservedFIFOShares))
                            } ?;
                            parent.sell_claims.retain(|c| c.quantity > 0);
                            let mut empty=Vec::new();
                            for lot in state.account.lots.iter().filter(|l|l.quantity==0){
                                let id=w.copy(&lot.lot_id)?;
                                w.push(&mut empty, id)?;
                            }
                            state.account.lots.retain(|l| l.quantity > 0);
                            for id in empty {
                                state.lot_assignments.remove(&id);
                            }
                            state.cash.apply_strategy_delta_with_work(budget::checked( i128::from(modeled.notional_micro_cny) - i128::from(modeled.total_fee_micro_cny), )?, w)?;
                        }
                    }
                    let realized = if parent.intent.side == Side::Sell {
                        budget::checked( i128::from(modeled.notional_micro_cny) - i128::from(modeled.total_fee_micro_cny) - basis - inherited_fee, )?
                    } else {
                        0
                    };
                    state.account.realized_pnl = Money::from_micros(budget::checked( i128::from(state.account.realized_pnl.micros()) + i128::from(realized), )?);
                    state.account.fees = Money::from_micros(budget::checked( i128::from(state.account.fees.micros()) + i128::from(modeled.total_fee_micro_cny), )?);
                    state.account.cash = Money::from_micros(state.cash.account_cash);
                    parent.reservation = reservation_with_work(&parent.intent, parent.remaining, &fee, w)?;
                    parent.status = if parent.remaining == 0 {
                        ParentStatus::Filled
                    } else {
                        ParentStatus::PartiallyFilled
                    };
                    let record = FillRecord {
                        fill_id,
                        parent_id: w.copy(parent_id)?,
                        observation_id: w.copy(&window.observation_id)?,
                        executed_at: window.observed_at,
                        side: parent.intent.side,
                        model: modeled,
                        inherited_buy_fee_micro_cny: budget::checked(inherited_fee)?,
                        realized_pnl_micro_cny: realized,
                    };
                    {
                        let key=w.copy(parent_id)?;
                        let incoming=parent;
                        w.insert(&mut state.parents, key, incoming)?;
                    };
                    {
                        let incoming=w.copy(&record)?;
                        w.push(&mut state.fills, incoming)?;
                    };
                    Effect::Filled(record)
                }
            };
            state.validate_with_work(w)?;
            Ok(effect)
        }
        CommandRecord::Cancel {
            parent_id,
            at,
            ..
        }
        | CommandRecord::Expire {
            parent_id,
            at,
            ..
        }
        => {
            let parent = state .parents .get_mut(parent_id) .ok_or(LedgerError::IdentityConflict)?;
            {
                let condition = parent.status.working() && *at >= state.account.as_of;
                w.require(condition, Txt::Execution(fw::ExecutionText::CancelExpireNotCurrentWorkingOrder))
            } ?;
            let expired = matches!(request, CommandRecord::Expire {
                ..
            });
            if expired {
                let local = checked_shanghai_local_with_work(*at, w)?;
                let day = local.date_naive();
                {
                    let condition = (day > parent.intent.session_date || (day == parent.intent.session_date && local.time().num_seconds_from_midnight() >= 15 * 3600)) && w.calendar_day(day)?;
                    w.require(condition, Txt::Execution(fw::ExecutionText::DayOrderNotYetExpiredOnVerifiedSession))
                } ?;
            }
            parent.status = if expired {
                ParentStatus::Expired
            } else {
                ParentStatus::Cancelled
            };
            parent.cancelled = parent.remaining;
            parent.remaining = 0;
            parent.reservation = reservation_with_work(&parent.intent, 0, &fee, w)?;
            parent.sell_claims.clear();
            state.account.as_of = *at;
            state.validate_with_work(w)?;
            Ok(if expired {
                Effect::Expired
            } else {
                Effect::Cancelled
            })
        }
        CommandRecord::QualifiedMarks {
            windows,
            ..
        }
        => {
            {
                let condition = !windows.is_empty();
                w.require(condition, Txt::Execution(fw::ExecutionText::QualifiedMarkSetEmpty))
            } ?;
            let at = windows[0].observed_at;
            let mut seen = BTreeSet::new();
            {
                let condition = at >= state.account.as_of;
                w.require(condition, Txt::Execution(fw::ExecutionText::MarkPrecedesPriorFinancialFact))
            } ?;
            for window in windows {
                window.validate_with_work(w)?;
                {
                    let condition = window.observed_at == at && w.set(&mut seen, window.instrument_code.as_str())?;
                    w.require(condition, Txt::Execution(fw::ExecutionText::MarkSetDateCodeDuplicate))
                } ?;
                {
                    let key=w.copy(&window.instrument_code)?;
                    let incoming=mark_from_window_with_work(window, w)?;
                    w.insert(&mut state.account.marks, key, incoming)?;
                };
                {
                    let key=w.copy(&window.instrument_code)?;
                    let incoming=w.copy(window)?;
                    w.insert(&mut state.valuation_windows, key, incoming)?;
                };
            }
            let mut holdings=BTreeSet::new();
            for lot in &state.account.lots{
                w.set(&mut holdings, lot.code.as_str())?;
            }
            {
                let condition = holdings.iter().all(|c| seen.contains(c));
                w.require(condition, Txt::Execution(fw::ExecutionText::MarksOmitFullAccountHolding))
            } ?;
            state.account.as_of = at;
            state.validate_with_work(w)?;
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

// Finite replay DTO seeds stay with the owners of private fields.
#[allow(dead_code, non_camel_case_types)]
mod replay_codec_owner {
    use super::budget::{InitialLotAllocation, ProfitPolicy};
    use super::fill_model::NoFillReason;
    use super::*;
    use crate::decision::approved_paper_intent_v1::TimeInForce;
    use crate::trading::paper_replay_codec_v1 as c;
    use crate::trading::paper_replay_shapes_v1 as s;
    use serde::de::{EnumAccess as _, VariantAccess as _};
    impl c::sealed::Value for ProfitPolicy {}
    impl c::Value for ProfitPolicy {
        const SHAPE: s::Shape = s::Shape::External(&[s::Variant {
            name: "ReinvestWithinFixedAuthorizedBudget",
            body: s::Body::Unit,
        }]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_ReinvestWithinFixedAuthorizedBudget<'de, 'w, 'loan, 'pool>(
                c::Input<'de, 'w, 'loan, 'pool>,
            );
            impl<'de> serde::de::DeserializeSeed<'de>
                for Seed_ReinvestWithinFixedAuthorizedBudget<'de, '_, '_, '_>
            {
                type Value = ProfitPolicy;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(ProfitPolicy::ReinvestWithinFixedAuthorizedBudget)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = ProfitPolicy;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["ReinvestWithinFixedAuthorizedBudget"],
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
                        "ReinvestWithinFixedAuthorizedBudget" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(ProfitPolicy::ReinvestWithinFixedAuthorizedBudget)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum("codec", &["ReinvestWithinFixedAuthorizedBudget"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                ProfitPolicy::ReinvestWithinFixedAuthorizedBudget => {
                    ProfitPolicy::ReinvestWithinFixedAuthorizedBudget
                }
            })
        }
    }
    impl c::sealed::Value for LotDisposition {}
    impl c::Value for LotDisposition {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "AllocatedToStrategy",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "UnassignedReadOnly",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_AllocatedToStrategy<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_AllocatedToStrategy<'de, '_, '_, '_> {
                type Value = LotDisposition;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LotDisposition::AllocatedToStrategy)
                }
            }
            struct Seed_UnassignedReadOnly<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_UnassignedReadOnly<'de, '_, '_, '_> {
                type Value = LotDisposition;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(LotDisposition::UnassignedReadOnly)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = LotDisposition;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["AllocatedToStrategy", "UnassignedReadOnly"],
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
                        "AllocatedToStrategy" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LotDisposition::AllocatedToStrategy)
                        }
                        "UnassignedReadOnly" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(LotDisposition::UnassignedReadOnly)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &["AllocatedToStrategy", "UnassignedReadOnly"],
                EV(input),
            )
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                LotDisposition::AllocatedToStrategy => LotDisposition::AllocatedToStrategy,
                LotDisposition::UnassignedReadOnly => LotDisposition::UnassignedReadOnly,
            })
        }
    }
    impl c::sealed::Value for InitialLotAllocation {}
    impl c::Value for InitialLotAllocation {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "lot_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "original_quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "disposition",
                    shape: &<LotDisposition as c::Value>::SHAPE,
                    optional: <LotDisposition as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "chain_id",
                    shape: &<Option<String> as c::Value>::SHAPE,
                    optional: <Option<String> as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,InitialLotAllocation,true,{lot_id:String=>false,original_quantity:u32=>false,disposition:LotDisposition=>false,chain_id:Option<String> =>false},InitialLotAllocation{lot_id,original_quantity,disposition,chain_id})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(InitialLotAllocation {
                lot_id: c::Value::paid_copy(&self.lot_id, w)?,
                original_quantity: c::Value::paid_copy(&self.original_quantity, w)?,
                disposition: c::Value::paid_copy(&self.disposition, w)?,
                chain_id: c::Value::paid_copy(&self.chain_id, w)?,
            })
        }
    }
    impl c::sealed::Value for BudgetRecord {}
    impl c::Value for BudgetRecord {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "version",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "family_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "effective_from",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "effective_through",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "authorized_budget_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "initial_strategy_cash_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "concentration_bps",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "chain_exposure_bps",
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
                    name: "max_order_exposure_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "original_seed_reference",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "review_reference",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "profit_policy",
                    shape: &<ProfitPolicy as c::Value>::SHAPE,
                    optional: <ProfitPolicy as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "initial_lots",
                    shape: &<Vec<InitialLotAllocation> as c::Value>::SHAPE,
                    optional: <Vec<InitialLotAllocation> as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,BudgetRecord,true,{version:String=>false,family_id:String=>false,effective_from:NaiveDate=>false,effective_through:NaiveDate=>false,authorized_budget_micro_cny:i64=>false,initial_strategy_cash_micro_cny:i64=>false,concentration_bps:u32=>false,chain_exposure_bps:u32=>false,cash_floor_bps:u32=>false,max_order_exposure_micro_cny:i64=>false,original_seed_reference:String=>false,review_reference:String=>false,profit_policy:ProfitPolicy=>false,initial_lots:Vec<InitialLotAllocation> =>false},BudgetRecord{version,family_id,effective_from,effective_through,authorized_budget_micro_cny,initial_strategy_cash_micro_cny,concentration_bps,chain_exposure_bps,cash_floor_bps,max_order_exposure_micro_cny,original_seed_reference,review_reference,profit_policy,initial_lots})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(BudgetRecord {
                version: c::Value::paid_copy(&self.version, w)?,
                family_id: c::Value::paid_copy(&self.family_id, w)?,
                effective_from: c::Value::paid_copy(&self.effective_from, w)?,
                effective_through: c::Value::paid_copy(&self.effective_through, w)?,
                authorized_budget_micro_cny: c::Value::paid_copy(
                    &self.authorized_budget_micro_cny,
                    w,
                )?,
                initial_strategy_cash_micro_cny: c::Value::paid_copy(
                    &self.initial_strategy_cash_micro_cny,
                    w,
                )?,
                concentration_bps: c::Value::paid_copy(&self.concentration_bps, w)?,
                chain_exposure_bps: c::Value::paid_copy(&self.chain_exposure_bps, w)?,
                cash_floor_bps: c::Value::paid_copy(&self.cash_floor_bps, w)?,
                max_order_exposure_micro_cny: c::Value::paid_copy(
                    &self.max_order_exposure_micro_cny,
                    w,
                )?,
                original_seed_reference: c::Value::paid_copy(&self.original_seed_reference, w)?,
                review_reference: c::Value::paid_copy(&self.review_reference, w)?,
                profit_policy: c::Value::paid_copy(&self.profit_policy, w)?,
                initial_lots: c::Value::paid_copy(&self.initial_lots, w)?,
            })
        }
    }
    impl c::sealed::Value for CashPartitions {}
    impl c::Value for CashPartitions {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "account_cash",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "strategy_cash",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "unassigned_cash",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,CashPartitions,true,{account_cash:i64=>false,strategy_cash:i64=>false,unassigned_cash:i64=>false},CashPartitions{account_cash,strategy_cash,unassigned_cash})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(CashPartitions {
                account_cash: c::Value::paid_copy(&self.account_cash, w)?,
                strategy_cash: c::Value::paid_copy(&self.strategy_cash, w)?,
                unassigned_cash: c::Value::paid_copy(&self.unassigned_cash, w)?,
            })
        }
    }
    impl c::sealed::Value for WorkingReservation {}
    impl c::Value for WorkingReservation {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "parent_id",
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
                    name: "chain_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "buy_max_notional",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_reserve",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cash_reserve",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,WorkingReservation,true,{parent_id:String=>false,code:String=>false,chain_id:String=>false,buy_max_notional:i64=>false,fee_reserve:i64=>false,cash_reserve:i64=>false},WorkingReservation{parent_id,code,chain_id,buy_max_notional,fee_reserve,cash_reserve})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(WorkingReservation {
                parent_id: c::Value::paid_copy(&self.parent_id, w)?,
                code: c::Value::paid_copy(&self.code, w)?,
                chain_id: c::Value::paid_copy(&self.chain_id, w)?,
                buy_max_notional: c::Value::paid_copy(&self.buy_max_notional, w)?,
                fee_reserve: c::Value::paid_copy(&self.fee_reserve, w)?,
                cash_reserve: c::Value::paid_copy(&self.cash_reserve, w)?,
            })
        }
    }
    impl c::sealed::Value for TimeInForce {}
    impl c::Value for TimeInForce {
        const SHAPE: s::Shape = s::Shape::External(&[s::Variant {
            name: "DaySession",
            body: s::Body::Unit,
        }]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_DaySession<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_DaySession<'de, '_, '_, '_> {
                type Value = TimeInForce;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(TimeInForce::DaySession)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = TimeInForce;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["DaySession"],
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
                        "DaySession" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(TimeInForce::DaySession)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum("codec", &["DaySession"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                TimeInForce::DaySession => TimeInForce::DaySession,
            })
        }
    }
    impl c::sealed::Value for IntentRecord {}
    impl c::Value for IntentRecord {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "version",
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
                    name: "execution_manifest_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "parent_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "investment_decision_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "family_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "chain_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "instrument_code",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "instrument_name",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "side",
                    shape: &<Side as c::Value>::SHAPE,
                    optional: <Side as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "limit_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_price_cap_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "session_date",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "time_in_force",
                    shape: &<TimeInForce as c::Value>::SHAPE,
                    optional: <TimeInForce as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "approved_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "approval_reference",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source_window",
                    shape: &<WindowRecord as c::Value>::SHAPE,
                    optional: <WindowRecord as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,IntentRecord,true,{version:String=>false,account_id:String=>false,epoch_id:String=>false,execution_manifest_hash:String=>false,parent_id:String=>false,investment_decision_id:String=>false,family_id:String=>false,chain_id:String=>false,instrument_code:String=>false,instrument_name:String=>false,side:Side=>false,quantity:u32=>false,limit_micro_cny:i64=>false,fee_price_cap_micro_cny:i64=>false,session_date:NaiveDate=>false,time_in_force:TimeInForce=>false,approved_at:DateTime<Utc> =>false,approval_reference:String=>false,source_window:WindowRecord=>false},IntentRecord{version,account_id,epoch_id,execution_manifest_hash,parent_id,investment_decision_id,family_id,chain_id,instrument_code,instrument_name,side,quantity,limit_micro_cny,fee_price_cap_micro_cny,session_date,time_in_force,approved_at,approval_reference,source_window})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(IntentRecord {
                version: c::Value::paid_copy(&self.version, w)?,
                account_id: c::Value::paid_copy(&self.account_id, w)?,
                epoch_id: c::Value::paid_copy(&self.epoch_id, w)?,
                execution_manifest_hash: c::Value::paid_copy(&self.execution_manifest_hash, w)?,
                parent_id: c::Value::paid_copy(&self.parent_id, w)?,
                investment_decision_id: c::Value::paid_copy(&self.investment_decision_id, w)?,
                family_id: c::Value::paid_copy(&self.family_id, w)?,
                chain_id: c::Value::paid_copy(&self.chain_id, w)?,
                instrument_code: c::Value::paid_copy(&self.instrument_code, w)?,
                instrument_name: c::Value::paid_copy(&self.instrument_name, w)?,
                side: c::Value::paid_copy(&self.side, w)?,
                quantity: c::Value::paid_copy(&self.quantity, w)?,
                limit_micro_cny: c::Value::paid_copy(&self.limit_micro_cny, w)?,
                fee_price_cap_micro_cny: c::Value::paid_copy(&self.fee_price_cap_micro_cny, w)?,
                session_date: c::Value::paid_copy(&self.session_date, w)?,
                time_in_force: c::Value::paid_copy(&self.time_in_force, w)?,
                approved_at: c::Value::paid_copy(&self.approved_at, w)?,
                approval_reference: c::Value::paid_copy(&self.approval_reference, w)?,
                source_window: c::Value::paid_copy(&self.source_window, w)?,
            })
        }
    }
    impl c::sealed::Value for Side {}
    impl c::Value for Side {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "Buy",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Sell",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Buy<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Buy<'de, '_, '_, '_> {
                type Value = Side;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(Side::Buy)
                }
            }
            struct Seed_Sell<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Sell<'de, '_, '_, '_> {
                type Value = Side;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(Side::Sell)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = Side;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["Buy", "Sell"],
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
                        "Buy" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(Side::Buy)
                        }
                        "Sell" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(Side::Sell)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum("codec", &["Buy", "Sell"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                Side::Buy => Side::Buy,
                Side::Sell => Side::Sell,
            })
        }
    }
    impl c::sealed::Value for WindowRecord {}
    impl c::Value for WindowRecord {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "version",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "observation_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "instrument_code",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "session_date",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "observed_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fresh_through",
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
                    name: "facts_contract",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "facts_batch_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "facts_source",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "facts_source_at",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "facts_observed_at",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_segment",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "listed",
                    shape: &<bool as c::Value>::SHAPE,
                    optional: <bool as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "suspended",
                    shape: &<bool as c::Value>::SHAPE,
                    optional: <bool as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "tick_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "lower_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "upper_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "regime_version",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "price_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "modeled_available_quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,WindowRecord,true,{version:String=>false,observation_id:String=>false,instrument_code:String=>false,session_date:NaiveDate=>false,source_at:DateTime<Utc> =>false,observed_at:DateTime<Utc> =>false,fresh_through:DateTime<Utc> =>false,source_reference:String=>false,facts_contract:String=>false,facts_batch_id:String=>false,facts_source:String=>false,facts_source_at:String=>false,facts_observed_at:String=>false,fee_segment:String=>false,listed:bool=>false,suspended:bool=>false,tick_micro_cny:i64=>false,lower_micro_cny:i64=>false,upper_micro_cny:i64=>false,regime_version:String=>false,price_micro_cny:i64=>false,modeled_available_quantity:u32=>false},WindowRecord{version,observation_id,instrument_code,session_date,source_at,observed_at,fresh_through,source_reference,facts_contract,facts_batch_id,facts_source,facts_source_at,facts_observed_at,fee_segment,listed,suspended,tick_micro_cny,lower_micro_cny,upper_micro_cny,regime_version,price_micro_cny,modeled_available_quantity})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(WindowRecord {
                version: c::Value::paid_copy(&self.version, w)?,
                observation_id: c::Value::paid_copy(&self.observation_id, w)?,
                instrument_code: c::Value::paid_copy(&self.instrument_code, w)?,
                session_date: c::Value::paid_copy(&self.session_date, w)?,
                source_at: c::Value::paid_copy(&self.source_at, w)?,
                observed_at: c::Value::paid_copy(&self.observed_at, w)?,
                fresh_through: c::Value::paid_copy(&self.fresh_through, w)?,
                source_reference: c::Value::paid_copy(&self.source_reference, w)?,
                facts_contract: c::Value::paid_copy(&self.facts_contract, w)?,
                facts_batch_id: c::Value::paid_copy(&self.facts_batch_id, w)?,
                facts_source: c::Value::paid_copy(&self.facts_source, w)?,
                facts_source_at: c::Value::paid_copy(&self.facts_source_at, w)?,
                facts_observed_at: c::Value::paid_copy(&self.facts_observed_at, w)?,
                fee_segment: c::Value::paid_copy(&self.fee_segment, w)?,
                listed: c::Value::paid_copy(&self.listed, w)?,
                suspended: c::Value::paid_copy(&self.suspended, w)?,
                tick_micro_cny: c::Value::paid_copy(&self.tick_micro_cny, w)?,
                lower_micro_cny: c::Value::paid_copy(&self.lower_micro_cny, w)?,
                upper_micro_cny: c::Value::paid_copy(&self.upper_micro_cny, w)?,
                regime_version: c::Value::paid_copy(&self.regime_version, w)?,
                price_micro_cny: c::Value::paid_copy(&self.price_micro_cny, w)?,
                modeled_available_quantity: c::Value::paid_copy(
                    &self.modeled_available_quantity,
                    w,
                )?,
            })
        }
    }
    impl c::sealed::Value for NoFillReason {}
    impl c::Value for NoFillReason {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "Suspended",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "OutsideLimit",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "LessThanWholeLot",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Suspended<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Suspended<'de, '_, '_, '_> {
                type Value = NoFillReason;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(NoFillReason::Suspended)
                }
            }
            struct Seed_OutsideLimit<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_OutsideLimit<'de, '_, '_, '_> {
                type Value = NoFillReason;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(NoFillReason::OutsideLimit)
                }
            }
            struct Seed_LessThanWholeLot<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_LessThanWholeLot<'de, '_, '_, '_> {
                type Value = NoFillReason;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(NoFillReason::LessThanWholeLot)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = NoFillReason;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["Suspended", "OutsideLimit", "LessThanWholeLot"],
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
                        "Suspended" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(NoFillReason::Suspended)
                        }
                        "OutsideLimit" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(NoFillReason::OutsideLimit)
                        }
                        "LessThanWholeLot" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(NoFillReason::LessThanWholeLot)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &["Suspended", "OutsideLimit", "LessThanWholeLot"],
                EV(input),
            )
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                NoFillReason::Suspended => NoFillReason::Suspended,
                NoFillReason::OutsideLimit => NoFillReason::OutsideLimit,
                NoFillReason::LessThanWholeLot => NoFillReason::LessThanWholeLot,
            })
        }
    }
    impl c::sealed::Value for ModeledFill {}
    impl c::Value for ModeledFill {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "price_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "notional_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "commission_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "stamp_tax_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "total_fee_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_policy_instance_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "stamp_tax_bracket",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sellable_from",
                    shape: &<NaiveDate as c::Value>::SHAPE,
                    optional: <NaiveDate as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,ModeledFill,true,{quantity:u32=>false,price_micro_cny:i64=>false,notional_micro_cny:i64=>false,commission_micro_cny:i64=>false,stamp_tax_micro_cny:i64=>false,total_fee_micro_cny:i64=>false,fee_policy_instance_id:String=>false,stamp_tax_bracket:String=>false,sellable_from:NaiveDate=>false},ModeledFill{quantity,price_micro_cny,notional_micro_cny,commission_micro_cny,stamp_tax_micro_cny,total_fee_micro_cny,fee_policy_instance_id,stamp_tax_bracket,sellable_from})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(ModeledFill {
                quantity: c::Value::paid_copy(&self.quantity, w)?,
                price_micro_cny: c::Value::paid_copy(&self.price_micro_cny, w)?,
                notional_micro_cny: c::Value::paid_copy(&self.notional_micro_cny, w)?,
                commission_micro_cny: c::Value::paid_copy(&self.commission_micro_cny, w)?,
                stamp_tax_micro_cny: c::Value::paid_copy(&self.stamp_tax_micro_cny, w)?,
                total_fee_micro_cny: c::Value::paid_copy(&self.total_fee_micro_cny, w)?,
                fee_policy_instance_id: c::Value::paid_copy(&self.fee_policy_instance_id, w)?,
                stamp_tax_bracket: c::Value::paid_copy(&self.stamp_tax_bracket, w)?,
                sellable_from: c::Value::paid_copy(&self.sellable_from, w)?,
            })
        }
    }
    impl c::sealed::Value for ExecutionManifest {}
    impl c::Value for ExecutionManifest {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "version",
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
                    name: "genesis_event_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "genesis_projection_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_descriptor",
                    shape: &<Vec<u8> as c::Value>::SHAPE,
                    optional: <Vec<u8> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fee_policy_instance_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "budget",
                    shape: &<BudgetRecord as c::Value>::SHAPE,
                    optional: <BudgetRecord as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fill_model_version",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "approved_reference",
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
            c::record_read!(de,input,ExecutionManifest,true,{version:String=>false,account_id:String=>false,epoch_id:String=>false,cutover_id:String=>false,genesis_event_hash:String=>false,genesis_projection_hash:String=>false,fee_descriptor:Vec<u8> =>false,fee_policy_instance_id:String=>false,budget:BudgetRecord=>false,fill_model_version:String=>false,approved_reference:String=>false},ExecutionManifest{version,account_id,epoch_id,cutover_id,genesis_event_hash,genesis_projection_hash,fee_descriptor,fee_policy_instance_id,budget,fill_model_version,approved_reference})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(ExecutionManifest {
                version: c::Value::paid_copy(&self.version, w)?,
                account_id: c::Value::paid_copy(&self.account_id, w)?,
                epoch_id: c::Value::paid_copy(&self.epoch_id, w)?,
                cutover_id: c::Value::paid_copy(&self.cutover_id, w)?,
                genesis_event_hash: c::Value::paid_copy(&self.genesis_event_hash, w)?,
                genesis_projection_hash: c::Value::paid_copy(&self.genesis_projection_hash, w)?,
                fee_descriptor: c::Value::paid_copy(&self.fee_descriptor, w)?,
                fee_policy_instance_id: c::Value::paid_copy(&self.fee_policy_instance_id, w)?,
                budget: c::Value::paid_copy(&self.budget, w)?,
                fill_model_version: c::Value::paid_copy(&self.fill_model_version, w)?,
                approved_reference: c::Value::paid_copy(&self.approved_reference, w)?,
            })
        }
    }
    impl c::sealed::Value for ParentStatus {}
    impl c::Value for ParentStatus {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "Working",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "PartiallyFilled",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Filled",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Cancelled",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "Expired",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Working<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Working<'de, '_, '_, '_> {
                type Value = ParentStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(ParentStatus::Working)
                }
            }
            struct Seed_PartiallyFilled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_PartiallyFilled<'de, '_, '_, '_> {
                type Value = ParentStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(ParentStatus::PartiallyFilled)
                }
            }
            struct Seed_Filled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Filled<'de, '_, '_, '_> {
                type Value = ParentStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(ParentStatus::Filled)
                }
            }
            struct Seed_Cancelled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Cancelled<'de, '_, '_, '_> {
                type Value = ParentStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(ParentStatus::Cancelled)
                }
            }
            struct Seed_Expired<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Expired<'de, '_, '_, '_> {
                type Value = ParentStatus;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(ParentStatus::Expired)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = ParentStatus;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &[
                            "Working",
                            "PartiallyFilled",
                            "Filled",
                            "Cancelled",
                            "Expired",
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
                        "Working" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(ParentStatus::Working)
                        }
                        "PartiallyFilled" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(ParentStatus::PartiallyFilled)
                        }
                        "Filled" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(ParentStatus::Filled)
                        }
                        "Cancelled" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(ParentStatus::Cancelled)
                        }
                        "Expired" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(ParentStatus::Expired)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &[
                    "Working",
                    "PartiallyFilled",
                    "Filled",
                    "Cancelled",
                    "Expired",
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
                ParentStatus::Working => ParentStatus::Working,
                ParentStatus::PartiallyFilled => ParentStatus::PartiallyFilled,
                ParentStatus::Filled => ParentStatus::Filled,
                ParentStatus::Cancelled => ParentStatus::Cancelled,
                ParentStatus::Expired => ParentStatus::Expired,
            })
        }
    }
    impl c::sealed::Value for LotClaim {}
    impl c::Value for LotClaim {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "lot_id",
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
            ],
            true,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,LotClaim,true,{lot_id:String=>false,quantity:u32=>false},LotClaim{lot_id,quantity})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(LotClaim {
                lot_id: c::Value::paid_copy(&self.lot_id, w)?,
                quantity: c::Value::paid_copy(&self.quantity, w)?,
            })
        }
    }
    impl c::sealed::Value for ParentState {}
    impl c::Value for ParentState {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "intent",
                    shape: &<IntentRecord as c::Value>::SHAPE,
                    optional: <IntentRecord as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "status",
                    shape: &<ParentStatus as c::Value>::SHAPE,
                    optional: <ParentStatus as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "filled",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "remaining",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cancelled",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "reservation",
                    shape: &<WorkingReservation as c::Value>::SHAPE,
                    optional: <WorkingReservation as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "sell_claims",
                    shape: &<Vec<LotClaim> as c::Value>::SHAPE,
                    optional: <Vec<LotClaim> as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,ParentState,true,{intent:IntentRecord=>false,status:ParentStatus=>false,filled:u32=>false,remaining:u32=>false,cancelled:u32=>false,reservation:WorkingReservation=>false,sell_claims:Vec<LotClaim> =>false},ParentState{intent,status,filled,remaining,cancelled,reservation,sell_claims})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(ParentState {
                intent: c::Value::paid_copy(&self.intent, w)?,
                status: c::Value::paid_copy(&self.status, w)?,
                filled: c::Value::paid_copy(&self.filled, w)?,
                remaining: c::Value::paid_copy(&self.remaining, w)?,
                cancelled: c::Value::paid_copy(&self.cancelled, w)?,
                reservation: c::Value::paid_copy(&self.reservation, w)?,
                sell_claims: c::Value::paid_copy(&self.sell_claims, w)?,
            })
        }
    }
    impl c::sealed::Value for FillRecord {}
    impl c::Value for FillRecord {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "fill_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "parent_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "observation_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "executed_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "side",
                    shape: &<Side as c::Value>::SHAPE,
                    optional: <Side as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "model",
                    shape: &<ModeledFill as c::Value>::SHAPE,
                    optional: <ModeledFill as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "inherited_buy_fee_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "realized_pnl_micro_cny",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,FillRecord,true,{fill_id:String=>false,parent_id:String=>false,observation_id:String=>false,executed_at:DateTime<Utc> =>false,side:Side=>false,model:ModeledFill=>false,inherited_buy_fee_micro_cny:i64=>false,realized_pnl_micro_cny:i64=>false},FillRecord{fill_id,parent_id,observation_id,executed_at,side,model,inherited_buy_fee_micro_cny,realized_pnl_micro_cny})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(FillRecord {
                fill_id: c::Value::paid_copy(&self.fill_id, w)?,
                parent_id: c::Value::paid_copy(&self.parent_id, w)?,
                observation_id: c::Value::paid_copy(&self.observation_id, w)?,
                executed_at: c::Value::paid_copy(&self.executed_at, w)?,
                side: c::Value::paid_copy(&self.side, w)?,
                model: c::Value::paid_copy(&self.model, w)?,
                inherited_buy_fee_micro_cny: c::Value::paid_copy(
                    &self.inherited_buy_fee_micro_cny,
                    w,
                )?,
                realized_pnl_micro_cny: c::Value::paid_copy(&self.realized_pnl_micro_cny, w)?,
            })
        }
    }
    impl c::sealed::Value for ExecutionProjection {}
    impl c::Value for ExecutionProjection {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "version",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "account",
                    shape: &<Projection as c::Value>::SHAPE,
                    optional: <Projection as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cash",
                    shape: &<CashPartitions as c::Value>::SHAPE,
                    optional: <CashPartitions as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "lot_assignments",
                    shape: &<BTreeMap<String, Option<String>> as c::Value>::SHAPE,
                    optional: <BTreeMap<String, Option<String>> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "parents",
                    shape: &<BTreeMap<String, ParentState> as c::Value>::SHAPE,
                    optional: <BTreeMap<String, ParentState> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "used_windows",
                    shape: &<BTreeMap<String, String> as c::Value>::SHAPE,
                    optional: <BTreeMap<String, String> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "fills",
                    shape: &<Vec<FillRecord> as c::Value>::SHAPE,
                    optional: <Vec<FillRecord> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "valuation_windows",
                    shape: &<BTreeMap<String, WindowRecord> as c::Value>::SHAPE,
                    optional: <BTreeMap<String, WindowRecord> as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,ExecutionProjection,true,{version:String=>false,account:Projection=>false,cash:CashPartitions=>false,lot_assignments:BTreeMap<String, Option<String>> =>false,parents:BTreeMap<String, ParentState> =>false,used_windows:BTreeMap<String, String> =>false,fills:Vec<FillRecord> =>false,valuation_windows:BTreeMap<String, WindowRecord> =>false},ExecutionProjection{version,account,cash,lot_assignments,parents,used_windows,fills,valuation_windows})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(ExecutionProjection {
                version: c::Value::paid_copy(&self.version, w)?,
                account: c::Value::paid_copy(&self.account, w)?,
                cash: c::Value::paid_copy(&self.cash, w)?,
                lot_assignments: c::Value::paid_copy(&self.lot_assignments, w)?,
                parents: c::Value::paid_copy(&self.parents, w)?,
                used_windows: c::Value::paid_copy(&self.used_windows, w)?,
                fills: c::Value::paid_copy(&self.fills, w)?,
                valuation_windows: c::Value::paid_copy(&self.valuation_windows, w)?,
            })
        }
    }
    impl c::sealed::Value for HeadIdentity {}
    impl c::Value for HeadIdentity {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "version",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "event_hash",
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
            c::record_read!(de,input,HeadIdentity,true,{version:i64=>false,event_hash:String=>false},HeadIdentity{version,event_hash})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(HeadIdentity {
                version: c::Value::paid_copy(&self.version, w)?,
                event_hash: c::Value::paid_copy(&self.event_hash, w)?,
            })
        }
    }
    impl c::sealed::Value for CommandRecord {}
    impl c::Value for CommandRecord {
        const SHAPE: s::Shape = s::Shape::Adjacent(
            "operation",
            "request",
            &[
                s::Variant {
                    name: "Open",
                    body: s::Body::Record(
                        &[s::Field {
                            name: "manifest",
                            shape: &<ExecutionManifest as c::Value>::SHAPE,
                            optional: <ExecutionManifest as c::Value>::OPTIONAL,
                            positional_default: false,
                        }],
                        true,
                    ),
                },
                s::Variant {
                    name: "Submit",
                    body: s::Body::Record(
                        &[
                            s::Field {
                                name: "expected",
                                shape: &<HeadIdentity as c::Value>::SHAPE,
                                optional: <HeadIdentity as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "intent",
                                shape: &<IntentRecord as c::Value>::SHAPE,
                                optional: <IntentRecord as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                        ],
                        true,
                    ),
                },
                s::Variant {
                    name: "Evaluate",
                    body: s::Body::Record(
                        &[
                            s::Field {
                                name: "expected",
                                shape: &<HeadIdentity as c::Value>::SHAPE,
                                optional: <HeadIdentity as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "parent_id",
                                shape: &<String as c::Value>::SHAPE,
                                optional: <String as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "window",
                                shape: &<WindowRecord as c::Value>::SHAPE,
                                optional: <WindowRecord as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                        ],
                        true,
                    ),
                },
                s::Variant {
                    name: "Cancel",
                    body: s::Body::Record(
                        &[
                            s::Field {
                                name: "expected",
                                shape: &<HeadIdentity as c::Value>::SHAPE,
                                optional: <HeadIdentity as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "parent_id",
                                shape: &<String as c::Value>::SHAPE,
                                optional: <String as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "at",
                                shape: &<DateTime<Utc> as c::Value>::SHAPE,
                                optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                        ],
                        true,
                    ),
                },
                s::Variant {
                    name: "Expire",
                    body: s::Body::Record(
                        &[
                            s::Field {
                                name: "expected",
                                shape: &<HeadIdentity as c::Value>::SHAPE,
                                optional: <HeadIdentity as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "parent_id",
                                shape: &<String as c::Value>::SHAPE,
                                optional: <String as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "at",
                                shape: &<DateTime<Utc> as c::Value>::SHAPE,
                                optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                        ],
                        true,
                    ),
                },
                s::Variant {
                    name: "QualifiedMarks",
                    body: s::Body::Record(
                        &[
                            s::Field {
                                name: "expected",
                                shape: &<HeadIdentity as c::Value>::SHAPE,
                                optional: <HeadIdentity as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                            s::Field {
                                name: "windows",
                                shape: &<Vec<WindowRecord> as c::Value>::SHAPE,
                                optional: <Vec<WindowRecord> as c::Value>::OPTIONAL,
                                positional_default: false,
                            },
                        ],
                        true,
                    ),
                },
            ],
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Open<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Open<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,CommandRecord,false,{manifest:ExecutionManifest=>false},CommandRecord::Open{manifest})
                }
            }
            struct Seed_Submit<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Submit<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,CommandRecord,false,{expected:HeadIdentity=>false,intent:IntentRecord=>false},CommandRecord::Submit{expected,intent})
                }
            }
            struct Seed_Evaluate<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Evaluate<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,CommandRecord,false,{expected:HeadIdentity=>false,parent_id:String=>false,window:WindowRecord=>false},CommandRecord::Evaluate{expected,parent_id,window})
                }
            }
            struct Seed_Cancel<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Cancel<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,CommandRecord,false,{expected:HeadIdentity=>false,parent_id:String=>false,at:DateTime<Utc> =>false},CommandRecord::Cancel{expected,parent_id,at})
                }
            }
            struct Seed_Expire<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Expire<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,CommandRecord,false,{expected:HeadIdentity=>false,parent_id:String=>false,at:DateTime<Utc> =>false},CommandRecord::Expire{expected,parent_id,at})
                }
            }
            struct Seed_QualifiedMarks<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_QualifiedMarks<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    let input = self.0;
                    c::record_read!(de,input,CommandRecord,false,{expected:HeadIdentity=>false,windows:Vec<WindowRecord> =>false},CommandRecord::QualifiedMarks{expected,windows})
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = CommandRecord;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    mut self,
                    mut map: A,
                ) -> Result<Self::Value, A::Error> {
                    let s::Shape::Adjacent(tagname, contentname, variants) =
                        <CommandRecord as c::Value>::SHAPE
                    else {
                        unreachable!()
                    };
                    let (selected, _, origin) = s::adjacent(
                        self.0.bytes,
                        self.0.span,
                        tagname,
                        contentname,
                        variants,
                        self.0.origin,
                    )
                    .ok_or_else(c::span_error)?;
                    let mut out = None;
                    let mut spans = self.0.span.children(self.0.bytes);
                    while let Some(key) = map.next_key_seed(c::KeySeed {
                        names: &["operation", "request"],
                    })? {
                        let span = spans.next().ok_or_else(c::span_error)?.1;
                        match key {
                            "operation" => {
                                let tag_origin = self.0.origin;
                                let found = map.next_value_seed(c::TagSeed {
                                    names: &[
                                        "Open",
                                        "Submit",
                                        "Evaluate",
                                        "Cancel",
                                        "Expire",
                                        "QualifiedMarks",
                                    ],
                                    input: self.0.child(span, tag_origin),
                                })?;
                                if found != selected.name {
                                    return Err(c::span_error());
                                }
                            }
                            "request" => {
                                out = Some(match selected.name {
                                    "Open" => {
                                        map.next_value_seed(Seed_Open(self.0.child(span, origin)))?
                                    }
                                    "Submit" => map
                                        .next_value_seed(Seed_Submit(self.0.child(span, origin)))?,
                                    "Evaluate" => map.next_value_seed(Seed_Evaluate(
                                        self.0.child(span, origin),
                                    ))?,
                                    "Cancel" => map
                                        .next_value_seed(Seed_Cancel(self.0.child(span, origin)))?,
                                    "Expire" => map
                                        .next_value_seed(Seed_Expire(self.0.child(span, origin)))?,
                                    "QualifiedMarks" => map.next_value_seed(
                                        Seed_QualifiedMarks(self.0.child(span, origin)),
                                    )?,
                                    _ => return Err(c::span_error()),
                                });
                            }
                            _ => return Err(c::span_error()),
                        }
                    }
                    if let Some(v) = out {
                        return Ok(v);
                    }
                    match selected.name {
                        _ => Err(self.0.error(c::K::MissingField, "codec missing")),
                    }
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    mut self,
                    mut seq: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut spans = self.0.span.children(self.0.bytes);
                    let _ = spans.next().ok_or_else(c::span_error)?;
                    let tag = seq
                        .next_element_seed(c::KeySeed {
                            names: &[
                                "Open",
                                "Submit",
                                "Evaluate",
                                "Cancel",
                                "Expire",
                                "QualifiedMarks",
                            ],
                        })?
                        .ok_or_else(c::span_error)?;
                    let span = spans.next().ok_or_else(c::span_error)?.1;
                    let origin = self.0.origin;
                    match tag {
                        "Open" => seq
                            .next_element_seed(Seed_Open(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Submit" => seq
                            .next_element_seed(Seed_Submit(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Evaluate" => seq
                            .next_element_seed(Seed_Evaluate(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Cancel" => seq
                            .next_element_seed(Seed_Cancel(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Expire" => seq
                            .next_element_seed(Seed_Expire(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "QualifiedMarks" => seq
                            .next_element_seed(Seed_QualifiedMarks(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_struct("codec", &["operation", "request"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                CommandRecord::Open { manifest } => CommandRecord::Open {
                    manifest: c::Value::paid_copy(manifest, w)?,
                },
                CommandRecord::Submit { expected, intent } => CommandRecord::Submit {
                    expected: c::Value::paid_copy(expected, w)?,
                    intent: c::Value::paid_copy(intent, w)?,
                },
                CommandRecord::Evaluate {
                    expected,
                    parent_id,
                    window,
                } => CommandRecord::Evaluate {
                    expected: c::Value::paid_copy(expected, w)?,
                    parent_id: c::Value::paid_copy(parent_id, w)?,
                    window: c::Value::paid_copy(window, w)?,
                },
                CommandRecord::Cancel {
                    expected,
                    parent_id,
                    at,
                } => CommandRecord::Cancel {
                    expected: c::Value::paid_copy(expected, w)?,
                    parent_id: c::Value::paid_copy(parent_id, w)?,
                    at: c::Value::paid_copy(at, w)?,
                },
                CommandRecord::Expire {
                    expected,
                    parent_id,
                    at,
                } => CommandRecord::Expire {
                    expected: c::Value::paid_copy(expected, w)?,
                    parent_id: c::Value::paid_copy(parent_id, w)?,
                    at: c::Value::paid_copy(at, w)?,
                },
                CommandRecord::QualifiedMarks { expected, windows } => {
                    CommandRecord::QualifiedMarks {
                        expected: c::Value::paid_copy(expected, w)?,
                        windows: c::Value::paid_copy(windows, w)?,
                    }
                }
            })
        }
    }
    impl c::sealed::Value for Effect {}
    impl c::Value for Effect {
        const SHAPE: s::Shape = s::Shape::Adjacent(
            "effect",
            "record",
            &[
                s::Variant {
                    name: "Opened",
                    body: s::Body::Unit,
                },
                s::Variant {
                    name: "Submitted",
                    body: s::Body::Value(&<ParentState as c::Value>::SHAPE),
                },
                s::Variant {
                    name: "ObservedNoFill",
                    body: s::Body::Value(&<fill_model::NoFillReason as c::Value>::SHAPE),
                },
                s::Variant {
                    name: "Filled",
                    body: s::Body::Value(&<FillRecord as c::Value>::SHAPE),
                },
                s::Variant {
                    name: "Cancelled",
                    body: s::Body::Unit,
                },
                s::Variant {
                    name: "Expired",
                    body: s::Body::Unit,
                },
                s::Variant {
                    name: "Marks",
                    body: s::Body::Unit,
                },
            ],
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_Opened<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Opened<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(Effect::Opened)
                }
            }
            struct Seed_Submitted<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Submitted<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <ParentState as c::Value>::read(de, self.0).map(Effect::Submitted)
                }
            }
            struct Seed_ObservedNoFill<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_ObservedNoFill<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <fill_model::NoFillReason as c::Value>::read(de, self.0)
                        .map(Effect::ObservedNoFill)
                }
            }
            struct Seed_Filled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Filled<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    <FillRecord as c::Value>::read(de, self.0).map(Effect::Filled)
                }
            }
            struct Seed_Cancelled<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Cancelled<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(Effect::Cancelled)
                }
            }
            struct Seed_Expired<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Expired<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(Effect::Expired)
                }
            }
            struct Seed_Marks<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_Marks<'de, '_, '_, '_> {
                type Value = Effect;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(Effect::Marks)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = Effect;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    mut self,
                    mut map: A,
                ) -> Result<Self::Value, A::Error> {
                    let s::Shape::Adjacent(tagname, contentname, variants) =
                        <Effect as c::Value>::SHAPE
                    else {
                        unreachable!()
                    };
                    let (selected, _, origin) = s::adjacent(
                        self.0.bytes,
                        self.0.span,
                        tagname,
                        contentname,
                        variants,
                        self.0.origin,
                    )
                    .ok_or_else(c::span_error)?;
                    let mut out = None;
                    let mut spans = self.0.span.children(self.0.bytes);
                    while let Some(key) = map.next_key_seed(c::KeySeed {
                        names: &["effect", "record"],
                    })? {
                        let span = spans.next().ok_or_else(c::span_error)?.1;
                        match key {
                            "effect" => {
                                let tag_origin = self.0.origin;
                                let found = map.next_value_seed(c::TagSeed {
                                    names: &[
                                        "Opened",
                                        "Submitted",
                                        "ObservedNoFill",
                                        "Filled",
                                        "Cancelled",
                                        "Expired",
                                        "Marks",
                                    ],
                                    input: self.0.child(span, tag_origin),
                                })?;
                                if found != selected.name {
                                    return Err(c::span_error());
                                }
                            }
                            "record" => {
                                out = Some(match selected.name {
                                    "Opened" => map
                                        .next_value_seed(Seed_Opened(self.0.child(span, origin)))?,
                                    "Submitted" => map.next_value_seed(Seed_Submitted(
                                        self.0.child(span, origin),
                                    ))?,
                                    "ObservedNoFill" => map.next_value_seed(
                                        Seed_ObservedNoFill(self.0.child(span, origin)),
                                    )?,
                                    "Filled" => map
                                        .next_value_seed(Seed_Filled(self.0.child(span, origin)))?,
                                    "Cancelled" => map.next_value_seed(Seed_Cancelled(
                                        self.0.child(span, origin),
                                    ))?,
                                    "Expired" => map.next_value_seed(Seed_Expired(
                                        self.0.child(span, origin),
                                    ))?,
                                    "Marks" => {
                                        map.next_value_seed(Seed_Marks(self.0.child(span, origin)))?
                                    }
                                    _ => return Err(c::span_error()),
                                });
                            }
                            _ => return Err(c::span_error()),
                        }
                    }
                    if let Some(v) = out {
                        return Ok(v);
                    }
                    match selected.name {
                        "Opened" => Ok(Effect::Opened),
                        "Cancelled" => Ok(Effect::Cancelled),
                        "Expired" => Ok(Effect::Expired),
                        "Marks" => Ok(Effect::Marks),
                        _ => Err(self.0.error(c::K::MissingField, "codec missing")),
                    }
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    mut self,
                    mut seq: A,
                ) -> Result<Self::Value, A::Error> {
                    let mut spans = self.0.span.children(self.0.bytes);
                    let _ = spans.next().ok_or_else(c::span_error)?;
                    let tag = seq
                        .next_element_seed(c::KeySeed {
                            names: &[
                                "Opened",
                                "Submitted",
                                "ObservedNoFill",
                                "Filled",
                                "Cancelled",
                                "Expired",
                                "Marks",
                            ],
                        })?
                        .ok_or_else(c::span_error)?;
                    let span = spans.next().ok_or_else(c::span_error)?.1;
                    let origin = self.0.origin;
                    match tag {
                        "Opened" => seq
                            .next_element_seed(Seed_Opened(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Submitted" => seq
                            .next_element_seed(Seed_Submitted(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "ObservedNoFill" => seq
                            .next_element_seed(Seed_ObservedNoFill(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Filled" => seq
                            .next_element_seed(Seed_Filled(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Cancelled" => seq
                            .next_element_seed(Seed_Cancelled(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Expired" => seq
                            .next_element_seed(Seed_Expired(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        "Marks" => seq
                            .next_element_seed(Seed_Marks(self.0.child(span, origin)))?
                            .ok_or_else(c::span_error),
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_struct("codec", &["effect", "record"], EV(input))
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                Effect::Opened => Effect::Opened,
                Effect::Submitted(value) => Effect::Submitted(c::Value::paid_copy(value, w)?),
                Effect::ObservedNoFill(value) => {
                    Effect::ObservedNoFill(c::Value::paid_copy(value, w)?)
                }
                Effect::Filled(value) => Effect::Filled(c::Value::paid_copy(value, w)?),
                Effect::Cancelled => Effect::Cancelled,
                Effect::Expired => Effect::Expired,
                Effect::Marks => Effect::Marks,
            })
        }
    }
    impl c::sealed::Value for Fact {}
    impl c::Value for Fact {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "request",
                    shape: &<CommandRecord as c::Value>::SHAPE,
                    optional: <CommandRecord as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "effect",
                    shape: &<Effect as c::Value>::SHAPE,
                    optional: <Effect as c::Value>::OPTIONAL,
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
            c::record_read!(de,input,Fact,true,{request:CommandRecord=>false,effect:Effect=>false},Fact{request,effect})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(Fact {
                request: c::Value::paid_copy(&self.request, w)?,
                effect: c::Value::paid_copy(&self.effect, w)?,
            })
        }
    }
    impl c::sealed::Element for InitialLotAllocation {}
    impl c::ArrayElement for InitialLotAllocation {}
    impl c::sealed::Element for LotClaim {}
    impl c::ArrayElement for LotClaim {}
    impl c::sealed::Element for FillRecord {}
    impl c::ArrayElement for FillRecord {}
    impl c::sealed::Element for WindowRecord {}
    impl c::ArrayElement for WindowRecord {}
    impl c::sealed::Entry for (String, Option<String>) {}
    impl c::MapEntry for (String, Option<String>) {}
    impl c::sealed::Entry for (String, ParentState) {}
    impl c::MapEntry for (String, ParentState) {}
    impl c::sealed::Entry for (String, String) {}
    impl c::MapEntry for (String, String) {}
    impl c::sealed::Entry for (String, WindowRecord) {}
    impl c::MapEntry for (String, WindowRecord) {}
    impl c::sealed::Root for ExecutionManifest {}
    impl c::Root for ExecutionManifest {
        const ROOT: s::RootKind = s::RootKind::ExecutionManifest;
        const CANONICAL: bool = true;
    }
    impl c::sealed::Root for Fact {}
    impl c::Root for Fact {
        const ROOT: s::RootKind = s::RootKind::ExecutionFact;
        const CANONICAL: bool = true;
    }
    impl c::sealed::Root for ExecutionProjection {}
    impl c::Root for ExecutionProjection {
        const ROOT: s::RootKind = s::RootKind::ExecutionProjection;
        const CANONICAL: bool = true;
    }
}
#[cfg(test)]
pub(crate) fn replay_codec_fixtures(
    case: crate::trading::paper_replay_codec_v1::CodecFixtureCase,
    work: &mut crate::database::global_schema_v1::replay_work::CodecMechanics<'_, '_>,
) {
    crate::trading::paper_replay_codec_v1::exercise_root::<ExecutionManifest>(case, work);
    crate::trading::paper_replay_codec_v1::exercise_root::<Fact>(case, work);
    crate::trading::paper_replay_codec_v1::exercise_root::<ExecutionProjection>(case, work);
}

struct ExecutionRecordExtentExceeded;
fn execution_record_extent(bytes:&[u8])->std::result::Result<(), ExecutionRecordExtentExceeded>{
    if bytes.len()<=MAX_RECORD_BYTES{
        Ok(())
    } else{
        Err(ExecutionRecordExtentExceeded)
    }
}
pub(crate) fn require_execution_record_extent_with_work(bytes:&[u8], w:&mut FinancialWork<'_, '_>)->fw::Result<()>{
    w.finish()?;
    match execution_record_extent(bytes){
        Ok(())=>Ok(()),
        Err(_)=>Err(w.error(Txt::Execution(fw::ExecutionText::ExecutionRecordExceedsByteLimit))?)
    }
}
pub(crate) fn sort_fifo_lots_owner(lots:&mut Vec<&Lot>){
    lots.sort_by(|a, b|(a.acquired_on, &a.lot_id).cmp(&(b.acquired_on, &b.lot_id)));
}

#[derive(Clone,Copy)]
enum DescriptorField{Segment,RateNum,RateDen,Minimum,Transfer,Other,Revision}
impl DescriptorField{fn key(self)->&'static str{match self{Self::Segment=>"segment",Self::RateNum=>"commission_rate_num",Self::RateDen=>"commission_rate_den",Self::Minimum=>"commission_minimum_micro_cny",Self::Transfer=>"coverage_transfer_fee",Self::Other=>"coverage_other_charges",Self::Revision=>"source_revision"}}}
fn descriptor_get<'a>(fields:&BTreeMap<&'a str, &'a str>, key:DescriptorField, w:&mut FinancialWork<'_, '_>)->fw::Result<&'a str>{
    w.option(fields.get(key.key()).copied(), Txt::Execution(fw::ExecutionText::FeeDescriptorMissingField))
}
fn descriptor_integer(fields:&BTreeMap<&str, &str>, key:DescriptorField, w:&mut FinancialWork<'_, '_>)->fw::Result<i64>{
    let value=descriptor_get(fields, key, w)?;
    match value.parse(){
        Ok(n)=>Ok(n),
        Err(_)=>Err(w.error(Txt::Execution(fw::ExecutionText::FeeDescriptorInteger))?)
    }
}
fn descriptor_reason(fields:&BTreeMap<&str, &str>, key:DescriptorField, w:&mut FinancialWork<'_, '_>)->fw::Result<ExcludedFeeReason>{
    match descriptor_get(fields, key, w)?{
        "excluded_unmodeled"=>Ok(ExcludedFeeReason::Unmodeled),
        "excluded_unverified"=>Ok(ExcludedFeeReason::Unverified),
        _=>Err(w.error(Txt::Execution(fw::ExecutionText::FeeCoverageField))?)
    }
}

/// Fixed execution input owner. Original authorization/window objects are
/// retained, not reconstructed from the ordinary cloned command record.
pub(crate) struct RetainedExecutionInput<'a> {
    account: &'a str,
    command: PaperV2Command,
    actual: Option<ActualExecutionBinding>,
    sampled_at: Option<DateTime<Utc>>,
    record: Option<CommandRecord>,
    receipt: Option<PaperV2Receipt>,
    binding: Option<SqlBinding>,
    recorded_windows: Vec<WindowRecord>,
}
pub(crate) struct RetainedExecutionAcquired {
    receipt: PaperV2Receipt,
    binding: SqlBinding,
    fresh_request: Option<CommandRecord>,
}
type RetainedExecutionOutcome<'db, 'a> =
    crate::database::global_schema_v1::paper_v6::RetainedPaperWriteOutcome<
        'db, RetainedExecutionInput<'a>, RetainedExecutionAcquired, LedgerError>;

fn retained_execution_input(account: &str, command: PaperV2Command) -> RetainedExecutionInput<'_> {
    RetainedExecutionInput { account, command, actual: None, sampled_at: None,
        record: None, receipt: None, binding: None, recorded_windows: Vec::new() }
}
fn retained_execution_command_id(command: &PaperV2Command) -> &str {
    match command {
        PaperV2Command::Submit { command_id, .. }
        | PaperV2Command::Evaluate { command_id, .. }
        | PaperV2Command::Cancel { command_id, .. }
        | PaperV2Command::Expire { command_id, .. }
        | PaperV2Command::QualifiedMarks { command_id, .. } => command_id,
    }
}
fn retained_execution_record(
    input: &mut RetainedExecutionInput<'_>,
    actual: &ActualExecutionBinding,
    now: DateTime<Utc>,
) -> Result<(), LedgerError> {
    let record = match &input.command {
        PaperV2Command::Submit { expected, approved, .. } => {
            approved.require_binding(actual)?;
            CommandRecord::Submit { expected: expected.clone(), intent: approved.record().clone() }
        }
        PaperV2Command::Evaluate { expected, parent_id, window, .. } => {
            window.require_binding(actual)?;
            CommandRecord::Evaluate { expected: expected.clone(), parent_id: parent_id.clone(), window: window.record().clone() }
        }
        PaperV2Command::Cancel { expected, parent_id, .. } =>
            CommandRecord::Cancel { expected: expected.clone(), parent_id: parent_id.clone(), at: now },
        PaperV2Command::Expire { expected, parent_id, .. } =>
            CommandRecord::Expire { expected: expected.clone(), parent_id: parent_id.clone(), at: now },
        PaperV2Command::QualifiedMarks { expected, windows, .. } => {
            for window in windows {
                window.require_binding(actual)?;
                input.recorded_windows.push(window.record().clone());
            }
            CommandRecord::QualifiedMarks { expected: expected.clone(), windows: std::mem::take(&mut input.recorded_windows) }
        }
    };
    input.record = Some(record);
    require(budget::token(retained_execution_command_id(&input.command)), "command id invalid")?;
    Ok(())
}
fn retained_execution_run<'db, 'a>(
    db: &'db DatabaseManager,
    input: RetainedExecutionInput<'a>,
    test_now: Option<DateTime<Utc>>,
) -> RetainedExecutionOutcome<'db, 'a> {
    use crate::database::global_schema_v1::paper_v6::RetainedPaperWrite;
    RetainedPaperWrite::open(db, input).run_once(
        |conn, authority, _proof, input| {
            let views = read_views_body_on(conn)?;
            let view = views.get(input.account).ok_or(LedgerError::NotSeeded)?;
            input.actual = Some(actual_binding(view, authority, db)?);
            input.sampled_at = Some(operation_now(db, test_now)?);
            let actual = input.actual.take().expect("actual binding retained");
            let sampled_at = input.sampled_at.expect("actual time retained");
            let record_result = retained_execution_record(input, &actual, sampled_at);
            input.actual = Some(actual);
            record_result?;
            input.receipt = Some(append_on(conn, input.account,
                retained_execution_command_id(&input.command),
                input.record.as_ref().expect("actual record retained").clone(),
                input.sampled_at.expect("actual time retained"))?);
            input.binding = Some(SqlBinding::capture(conn)?);
            #[cfg(test)]
            if db.has_isolated_p05_consumer_origin() {
                run_test_hook(TestPhase::AfterSql, conn);
                run_test_hook(TestPhase::LastSqlBeforeCommit, conn);
            }
            let receipt = input.receipt.take().expect("actual receipt retained");
            let fresh_request = if receipt.replayed { None } else { input.record.take() };
            Ok(RetainedExecutionAcquired { receipt,
                binding: input.binding.take().expect("actual SQL binding retained"), fresh_request })
        },
        |conn, _authority, _proof, input, acquired| {
            acquired.binding.validate(conn)?;
            acquired.binding.require_fresh_request(input.account,
                acquired.fresh_request.as_ref(), operation_now(db, test_now)?)
        },
    )
}
/// Additive ownership route only. The original production refusal and approval
/// rules remain in paper_catalog6_session; legacy borrowed routes are unchanged.
pub(crate) fn apply_retained_actual<'a>(
    account: &'a str,
    command: PaperV2Command,
) -> RetainedExecutionOutcome<'static, 'a> {
    use crate::database::global_schema_v1::paper_v6::{RetainedPaperWrite, RetainedPaperWriteOutcome};
    let input = retained_execution_input(account, command);
    match DatabaseManager::try_get() {
        Some(db) => retained_execution_run(db, input, None),
        None => RetainedPaperWriteOutcome::Held(RetainedPaperWrite::unopened(input, PaperCatalog6Error::Authority)),
    }
}

/// Test observation borrows the actual nonClone command; it cannot mint one.
#[cfg(test)]
pub(crate) fn retained_execution_command_for_test<'i, 'a>(
    input: &'i RetainedExecutionInput<'a>,
) -> &'i PaperV2Command {
    &input.command
}
