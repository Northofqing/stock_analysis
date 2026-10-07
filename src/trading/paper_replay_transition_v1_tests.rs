//! Lower fixtures use the actual persistent borrower, never an accepted pin.
use super::paper_book_v2_budget_v1::{
    self as budget, BudgetRecord, InitialLotAllocation, LotDisposition, ProfitPolicy,
};
use super::paper_book_v2_execution::{
    self as execution, CommandRecord, Effect, ExecutionManifest, ExecutionProjection, HeadIdentity,
};
use super::paper_book_v2_fill_model::{Side, WindowRecord, MODEL_VERSION};
use super::paper_ledger::{
    self as ledger, Lot, Mark, Money, Projection, RiskPolicyV1, SeedLot, SeedManifest,
};
use super::paper_replay_financial_work_v1::{
    self as fw, ClosedFinancialText as Txt, FinancialWork,
};
use crate::database::global_schema_v1::replay_work::{self as work, ReplayTerminalFailure};
use crate::decision::approved_paper_intent_v1::{IntentRecord, TimeInForce, INTENT_VERSION};
use crate::performance::fee_policy::*;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
const MANIFEST_VERSION: &str = "paper-parent-execution-manifest/v1";
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 24).unwrap()
}
fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 2, 0, second).unwrap()
}
fn genesis(cash: i64, lots: Vec<Lot>) -> Projection {
    let marks: BTreeMap<_, _> = lots
        .iter()
        .map(|lot| {
            (
                lot.code.clone(),
                Mark {
                    code: lot.code.clone(),
                    price: Money::from_micros(10_000_000),
                    observed_at: at(0),
                    source: "TEST_CODE_ORIGINAL_GENESIS_MARK".into(),
                },
            )
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "cash":cash,
        "lots":lots,
        "marks":marks,
        "fees":0,
        "realized_pnl":0,
        "seed_equity":cash,
        "as_of":at(0),
        "closes":{
        }
    }))
    .unwrap()
}
fn policy(genesis: &Projection, a: i64, b: i64) -> BudgetRecord {
    let mut initial_lots: Vec<_> = genesis
        .lots
        .iter()
        .map(|l| InitialLotAllocation {
            lot_id: l.lot_id.clone(),
            original_quantity: l.quantity,
            disposition: LotDisposition::AllocatedToStrategy,
            chain_id: Some("TEST_CODE_CHAIN".into()),
        })
        .collect();
    initial_lots.sort_by(|a, b| a.lot_id.cmp(&b.lot_id));
    BudgetRecord {
        version: budget::POLICY_VERSION.into(),
        family_id: "TEST_CODE_FAMILY".into(),
        effective_from: date(),
        effective_through: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
        authorized_budget_micro_cny: b,
        initial_strategy_cash_micro_cny: a,
        concentration_bps: 10_000,
        chain_exposure_bps: 10_000,
        cash_floor_bps: 0,
        max_order_exposure_micro_cny: b,
        original_seed_reference: "TEST_CODE_FULL_GENESIS".into(),
        review_reference: "TEST_CODE_EXPLICIT_ASSUMPTION".into(),
        profit_policy: ProfitPolicy::ReinvestWithinFixedAuthorizedBudget,
        initial_lots,
    }
}
fn fee() -> AShareFeePolicyV2 {
    AShareFeePolicyV2::new(
        QualifiedInstrument::new(
            FeeMarket::Shanghai,
            FeeSecurityKind::AShareStock,
            FeeListingSegment::ShanghaiMainA,
        )
        .unwrap(),
        FeeRate::new(3, 10_000).unwrap(),
        5_000_000,
        FeeCoverage::initial_model(),
        "TEST_CODE_EXPLICIT_REVIEWED_DESCRIPTOR",
    )
    .unwrap()
}
fn manifest(g: &Projection, a: i64, b: i64) -> ExecutionManifest {
    let fee = fee();
    ExecutionManifest {
        version: MANIFEST_VERSION.into(),
        account_id: "TEST_CODE_ACCOUNT".into(),
        epoch_id: "TEST_CODE_EPOCH_V2".into(),
        cutover_id: "TEST_CODE_CUTOVER".into(),
        genesis_event_hash: "a".repeat(64),
        genesis_projection_hash: hex::encode(Sha256::digest(serde_json::to_vec(g).unwrap())),
        fee_descriptor: fee.canonical_bytes(),
        fee_policy_instance_id: fee.instance_id(),
        budget: policy(g, a, b),
        fill_model_version: MODEL_VERSION.into(),
        approved_reference: "TEST_CODE_NOT_PRODUCTION".into(),
    }
}
fn window(id: &str, second: u32, price: i64, volume: u32) -> WindowRecord {
    WindowRecord {
        version: MODEL_VERSION.into(),
        observation_id: id.into(),
        instrument_code: "600001".into(),
        session_date: date(),
        source_at: at(second),
        observed_at: at(second),
        fresh_through: at(second) + chrono::Duration::minutes(1),
        source_reference: "TEST_CODE_MODELED_QUOTE".into(),
        facts_contract: "TEST_CODE_RECORDED_FACTS".into(),
        facts_batch_id: "TEST_CODE_BATCH".into(),
        facts_source: "TEST_CODE_SOURCE".into(),
        facts_source_at: at(0).to_rfc3339(),
        facts_observed_at: at(0).to_rfc3339(),
        fee_segment: "ShanghaiMainA".into(),
        listed: true,
        suspended: false,
        tick_micro_cny: 10_000,
        lower_micro_cny: 7_000_000,
        upper_micro_cny: 30_000_000,
        regime_version: "TEST_CODE_EXPLICIT_BAND".into(),
        price_micro_cny: price,
        modeled_available_quantity: volume,
    }
}
fn intent(m: &ExecutionManifest, id: &str, side: Side, quantity: u32, second: u32) -> IntentRecord {
    IntentRecord {
        version: INTENT_VERSION.into(),
        account_id: m.account_id.clone(),
        epoch_id: m.epoch_id.clone(),
        execution_manifest_hash: m.identity().unwrap(),
        parent_id: id.into(),
        investment_decision_id: format!("TEST_CODE_DECISION_{id}"),
        family_id: m.budget.family_id.clone(),
        chain_id: "TEST_CODE_CHAIN".into(),
        instrument_code: "600001".into(),
        instrument_name: "synthetic".into(),
        side,
        quantity,
        limit_micro_cny: 10_000_000,
        fee_price_cap_micro_cny: 10_000_000,
        session_date: date(),
        time_in_force: TimeInForce::DaySession,
        approved_at: at(second),
        approval_reference: "TEST_CODE_RECORD_ONLY".into(),
        source_window: window(&format!("TEST_CODE_APPROVAL_{id}"), second, 10_000_000, 100),
    }
}
fn head() -> HeadIdentity {
    HeadIdentity {
        version: 1,
        event_hash: "b".repeat(64),
    }
}
fn lot(id: &str, quantity: u32, fee: i64) -> Lot {
    Lot {
        lot_id: id.into(),
        code: "600001".into(),
        name: "synthetic old lot".into(),
        quantity,
        basis_price: Money::from_micros(10_000_000),
        buy_fee_remaining: Money::from_micros(fee),
        acquired_on: NaiveDate::from_ymd_opt(2026, 9, 23).unwrap(),
        sellable_from: date(),
        reported_cost: Some(Money::from_micros(6_000_000)),
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Case {
    Qualification,
    Texts,
    FeeDescriptor,
    SourceRevision,
    FeeOverflowOrder,
    Hashes,
    HistoricalExtent,
    OversizedResource,
    Collections,
    Growth,
    Sort,
    NodeShort,
    GrowShort,
    SortShort,
    TextShort,
    Cumulative,
    Grown,
    SellFifo,
    Expire,
    StagedInvalid,
    StagedResource,
    V1,
    Book,
    IntentOrder,
    FeeRecord,
    CalendarReuse,
    WriterMismatch,
    LotMapShort,
    DescriptorMapShort,
    ClaimShort,
    ExposureShort,
    SetExact,
    LotMapExact,
    DescriptorMapExact,
    ClaimExact,
    ExposureExact,
    GrowExact,
    SortExact,
    TextExact,
}
#[test]
fn transition_qualification() {
    work::financial_fixture(Case::Qualification);
}
#[test]
fn transition_texts() {
    work::financial_fixture(Case::Texts);
}
#[test]
fn transition_fee_descriptor() {
    work::financial_fixture(Case::FeeDescriptor);
}
#[test]
fn transition_source_revision() {
    work::financial_fixture(Case::SourceRevision);
}
#[test]
fn transition_fee_overflow_order() {
    work::financial_fixture(Case::FeeOverflowOrder);
}
#[test]
fn transition_hashes() {
    work::financial_fixture(Case::Hashes);
}
#[test]
fn transition_historical_extent() {
    work::financial_fixture(Case::HistoricalExtent);
}
#[test]
fn transition_oversized_resource() {
    work::financial_fixture(Case::OversizedResource);
}
#[test]
fn transition_collections() {
    work::financial_fixture(Case::Collections);
}
#[test]
fn transition_growth() {
    work::financial_fixture(Case::Growth);
}
#[test]
fn transition_sort() {
    work::financial_fixture(Case::Sort);
}
#[test]
fn transition_node_short() {
    work::financial_fixture(Case::NodeShort);
}
#[test]
fn transition_grow_short() {
    work::financial_fixture(Case::GrowShort);
}
#[test]
fn transition_sort_short() {
    work::financial_fixture(Case::SortShort);
}
#[test]
fn transition_text_short() {
    work::financial_fixture(Case::TextShort);
}
#[test]
fn transition_cumulative() {
    work::financial_fixture(Case::Cumulative);
}
#[test]
fn transition_grown() {
    work::financial_fixture(Case::Grown);
}
#[test]
fn transition_sell_fifo() {
    work::financial_fixture(Case::SellFifo);
}
#[test]
fn transition_expire() {
    work::financial_fixture(Case::Expire);
}
#[test]
fn transition_staged_invalid() {
    work::financial_fixture(Case::StagedInvalid);
}
#[test]
fn transition_staged_resource() {
    work::financial_fixture(Case::StagedResource);
}
#[test]
fn transition_v1() {
    work::financial_fixture(Case::V1);
}
#[test]
fn transition_book() {
    work::financial_fixture(Case::Book);
}
#[test]
fn transition_intent_order() {
    work::financial_fixture(Case::IntentOrder);
}
fn step(
    state: &mut ExecutionProjection,
    original: &mut ExecutionProjection,
    m: &ExecutionManifest,
    request: CommandRecord,
    w: &mut FinancialWork<'_, '_>,
) -> Effect {
    let before = w.used();
    let effect = execution::apply_request_with_work(state, m, &request, w).unwrap();
    let historical =
        execution::apply_request_with_work(original, m, &request, &mut FinancialWork::Historical)
            .unwrap();
    assert_eq!(
        serde_json::to_vec(&effect).unwrap(),
        serde_json::to_vec(&historical).unwrap()
    );
    assert_eq!(state, original);
    assert!(w.used() > before);
    effect
}
fn state_pair(
    lots: Vec<Lot>,
    w: &mut FinancialWork<'_, '_>,
) -> (ExecutionProjection, ExecutionProjection, ExecutionManifest) {
    let g = genesis(100_000_000_000, lots);
    let m = manifest(&g, 100_000_000_000, 200_000_000_000);
    let paid = ExecutionProjection::initial_with_work(&g, &m.budget, w).unwrap();
    let plain =
        ExecutionProjection::initial_with_work(&g, &m.budget, &mut FinancialWork::Historical)
            .unwrap();
    assert_eq!(paid, plain);
    (paid, plain, m)
}
fn terminal_retry(w: &mut FinancialWork<'_, '_>) {
    let e = w.finish().unwrap_err();
    assert!(matches!(
        e,
        fw::FinancialFailure::Terminal(ReplayTerminalFailure::Resource(_))
    ));
    let used = w.used();
    assert!(w.text(Txt::ProjectionVersion).is_err());
    assert_eq!(w.used(), used);
    assert_eq!(format!("{:?}", w.finish().unwrap_err()), format!("{e:?}"));
}
#[test]
fn transition_writer_bound_mismatch_is_sticky() {
    work::financial_fixture(Case::WriterMismatch);
}
#[test]
fn transition_fee_record_branches() {
    work::financial_fixture(Case::FeeRecord);
}
#[test]
fn transition_calendar_payment_survives_memory_move() {
    work::financial_fixture(Case::CalendarReuse);
}
pub(crate) fn run(case: Case, w: &mut FinancialWork<'_, '_>) {
    match case {
        Case::Qualification => unreachable!(),
        Case::LotMapShort
        | Case::DescriptorMapShort
        | Case::ClaimShort
        | Case::ExposureShort
        | Case::SetExact
        | Case::LotMapExact
        | Case::DescriptorMapExact
        | Case::ClaimExact
        | Case::ExposureExact
        | Case::GrowExact
        | Case::SortExact
        | Case::TextExact => boundary_case(case, w),
        Case::WriterMismatch => {
            let error = w.text(Txt::ProjectionVersion).unwrap_err();
            assert!(matches!(
                error,
                fw::FinancialFailure::Terminal(ReplayTerminalFailure::TransitionQualification(
                    work::ReplayTransitionQualificationFailure::WriterMismatch
                ))
            ));
            let used = w.used();
            assert!(used > 0);
            assert!(w.text(Txt::ProjectionVersion).is_err());
            assert_eq!(w.used(), used);
        }
        Case::CalendarReuse => {
            let mut moved = std::mem::replace(w, FinancialWork::Historical);
            let before = moved.used();
            assert!(moved.calendar_day(date()).unwrap());
            assert!(moved.used() > before + 122);
            let before = moved.used();
            assert!(moved.calendar_day(date()).unwrap());
            assert_eq!(moved.used() - before, 122);
            *w = moved;
            let before = w.used();
            assert!(w.calendar_day(date()).unwrap());
            assert_eq!(w.used() - before, 122);
        }
        Case::FeeRecord => {
            let (mut state, _, m) = state_pair(vec![], w);
            let original = state.clone();
            for (suffix, expected) in [
                ("segment=ShanghaiMainA\n", "duplicate fee descriptor field"),
                (
                    "unrecognized=field\n",
                    "fee descriptor is not canonical reviewed policy",
                ),
            ] {
                let mut changed = m.clone();
                changed.fee_descriptor.extend_from_slice(suffix.as_bytes());
                let err = execution::apply_request_with_work(
                    &mut state,
                    &changed,
                    &CommandRecord::Open {
                        manifest: changed.clone(),
                    },
                    w,
                )
                .unwrap_err();
                match err {
                    fw::FinancialFailure::Financial(ledger::LedgerError::IntegrityFailure(
                        text,
                    )) => assert_eq!(text, expected),
                    other => panic!("wrong descriptor branch: {other:?}"),
                };
                assert_eq!(state, original);
            }
            let mut changed = m.clone();
            let text = String::from_utf8(changed.fee_descriptor)
                .unwrap()
                .replace("segment=ShanghaiMainA", "segment=Unsupported");
            changed.fee_descriptor = text.into_bytes();
            let err = execution::apply_request_with_work(
                &mut state,
                &changed,
                &CommandRecord::Open {
                    manifest: changed.clone(),
                },
                w,
            )
            .unwrap_err();
            match err {
                fw::FinancialFailure::Financial(ledger::LedgerError::EvidenceUnavailable(text)) => {
                    assert_eq!(text, "fee segment unavailable")
                }
                other => panic!("wrong segment branch: {other:?}"),
            };
            assert_eq!(state, original);
        }
        Case::Texts => {
            for error in [
                AShareFeeV2Error::UnsupportedInstrument,
                AShareFeeV2Error::ScopeMismatch,
                AShareFeeV2Error::UnsupportedCoverage,
                AShareFeeV2Error::InvalidNotional,
                AShareFeeV2Error::UnsupportedTradeDate,
                AShareFeeV2Error::InvalidCommissionRate,
                AShareFeeV2Error::InvalidCommissionMinimum,
                AShareFeeV2Error::InvalidSourceRevision,
                AShareFeeV2Error::Overflow,
            ] {
                let expected = error.to_string();
                let before = w.used();
                assert_eq!(w.text(Txt::FeeError(&error)).unwrap(), expected);
                assert_eq!(w.used() - before, expected.len() as u64);
            }
            assert_eq!(
                match w
                    .error(Txt::Seed(
                        fw::SeedText::InvalidSeedIdentityEffectiveTimeOrPolicy
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: invalid seed identity, effective time or policy"
            );
            assert_eq!(
                match w.error(Txt::Seed(fw::SeedText::InvalidSeedMark)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: invalid seed mark"
            );
            assert_eq!(
                match w.error(Txt::Seed(fw::SeedText::InvalidSeedLot)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: invalid seed lot"
            );
            assert_eq!(
                match w
                    .error(Txt::Seed(fw::SeedText::SeedLotMarkMissing))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: seed lot mark missing"
            );
            assert_eq!(
                match w
                    .error(Txt::Seed(fw::SeedText::ExplicitSellabilityLacksEvidence))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: explicit sellability lacks evidence"
            );
            assert_eq!(
                match w
                    .error(Txt::Seed(fw::SeedText::SeedMarkCoverageMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: seed mark coverage mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Seed(fw::SeedText::UnapprovedSeedResidualEmptyEquity))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: unapproved seed residual/empty equity"
            );
            assert_eq!(
                match w
                    .error(Txt::V1(fw::V1Text::InvalidDuplicateExtraneousValuationMark))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: invalid/duplicate/extraneous valuation mark"
            );
            assert_eq!(
                match w
                    .error(Txt::V1(fw::V1Text::IncompleteWholeAccountValuation))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: incomplete whole-account valuation"
            );
            assert_eq!(
                match w.error(Txt::V1(fw::V1Text::SecondGenesis)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: second genesis"
            );
            assert_eq!(
                match w.error(Txt::V1(fw::V1Text::MarkInventoryMismatch)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: mark inventory mismatch"
            );
            assert_eq!(
                match w.error(Txt::V1(fw::V1Text::FIFOBeforeLotMismatch)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: FIFO before-lot mismatch"
            );
            assert_eq!(
                match w.error(Txt::V1(fw::V1Text::DuplicateLotIdentity)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: duplicate lot identity"
            );
            assert_eq!(
                match w.error(Txt::V1(fw::V1Text::NegativePaperCash)).unwrap() {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: negative paper cash"
            );
            assert_eq!(
                match w
                    .error(Txt::V1(fw::V1Text::NonfillHasFinancialEffects))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: nonfill has financial effects"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::GenesisAccountMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book genesis account mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::V1SourceAnchorMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book V1 source anchor mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::FeeInstanceMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book fee instance mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::ManifestFieldsMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book manifest fields mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::ManifestHashMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book manifest hash mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::GenesisEventMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book genesis event mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::GenesisHashMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book genesis hash mismatch"
            );
            assert_eq!(
                match w
                    .error(Txt::Book(fw::BookText::V2ProjectionMismatch))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: V2 book V2 projection mismatch"
            );
            assert_eq!(match w.error(Txt::Execution(fw::ExecutionText::PaperExecutionShanghaiClockExceedsSupportedRange)).unwrap(){
                fw::FinancialFailure::Financial(e)=>e.to_string(),
                _=>panic!("wrong category")
            }, "paper evidence unavailable: paper execution Shanghai clock exceeds supported range");
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::ExecutionManifestInvalid))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: execution manifest invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FeeDescriptorUTF8))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fee descriptor UTF8"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FeeDescriptorField))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fee descriptor field"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::DuplicateFeeDescriptorField
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: duplicate fee descriptor field"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FeeDescriptorMissingField))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fee descriptor missing field"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FeeDescriptorInteger))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fee descriptor integer"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FeeSegmentUnavailable))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: fee segment unavailable"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FeeCoverageField))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fee coverage field"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::FeeDescriptorIsNotCanonicalReviewedPolicy
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fee descriptor is not canonical reviewed policy"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::AccountCashDiffersFromExecutionPartition
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: account cash differs from execution partition"
            );
            assert_eq!(match w.error(Txt::Execution(fw::ExecutionText::RecordedValuationWindowDiffersFromOriginalMark)).unwrap(){
                fw::FinancialFailure::Financial(e)=>e.to_string(),
                _=>panic!("wrong category")
            }, "paper ledger integrity failure: recorded valuation window differs from original mark");
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::FullLotDispositionsDiffer))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: full lot dispositions differ"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::ParentQuantityDiffers))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: parent quantity differs"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::WorkingParentRemainderInvalid
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: working parent remainder invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::ReservationOwnerDiffers))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: reservation owner differs"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::ReservationComponentsDiffer
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: reservation components differ"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::TerminalParentRetainsReservation
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: terminal parent retains reservation"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::WorkingReservationExceedsStrategyCash
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: working reservation exceeds strategy cash"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::ClaimReferencesAbsentLot))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: claim references absent lot"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::SellClaimsOverbookLot))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: sell claims overbook lot"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::AllocatedHoldingMarkAbsent
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: allocated holding mark absent"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::AllocatedHoldingMarkIsNotCurrentSession
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: allocated holding mark is not current session"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::AllocatedHoldingQualifiedValuationWindowAbsent
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: allocated holding qualified valuation window absent"
            );
            assert_eq!(match w.error(Txt::Execution(fw::ExecutionText::AllocatedHoldingQualifiedValuationWindowExpiredOrDiffers)).unwrap(){
                fw::FinancialFailure::Financial(e)=>e.to_string(),
                _=>panic!("wrong category")
            }, "paper ledger integrity failure: allocated holding qualified valuation window expired or differs");
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::RecordedAdmittedBoardDiffersFromFeeScope
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: recorded admitted board differs from fee scope"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::SubmitManifestOwnerDiffers
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: submit manifest owner differs"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::BudgetPolicySessionNotEffective
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: budget policy session not effective"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::ParentObservationPrecedesPriorFinancialFact
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: parent observation precedes prior financial fact"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::StrategyCashCannotReserveSellFees
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: strategy cash cannot reserve sell fees"
            );
            assert_eq!(match w.error(Txt::Execution(fw::ExecutionText::AllocatedFIFOSellableSharesUnavailableOrAlreadyReserved)).unwrap(){
                fw::FinancialFailure::Financial(e)=>e.to_string(),
                _=>panic!("wrong category")
            }, "paper ledger integrity failure: allocated FIFO sellable shares unavailable or already reserved");
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::FillObservationPrecedesPriorFinancialFact
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fill observation precedes prior financial fact"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::WindowDoesNotMatchWorkingDayParent
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: window does not match working day parent"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::DuplicateFillLot))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: duplicate fill lot"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::ReservedSellLotDisappeared
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: reserved sell lot disappeared"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::SellLotIsNotAssignedSellable
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: sell lot is not assigned/sellable"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::FillExceedsReservedFIFOShares
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: fill exceeds reserved FIFO shares"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::CancelExpireNotCurrentWorkingOrder
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: cancel/expire not current working order"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::DayOrderNotYetExpiredOnVerifiedSession
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: day order not yet expired on verified session"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::QualifiedMarkSetEmpty))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: qualified mark set empty"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::MarkPrecedesPriorFinancialFact
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: mark precedes prior financial fact"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(fw::ExecutionText::MarkSetDateCodeDuplicate))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: mark set date/code duplicate"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::MarksOmitFullAccountHolding
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: marks omit full account holding"
            );
            assert_eq!(
                match w
                    .error(Txt::Execution(
                        fw::ExecutionText::ExecutionRecordExceedsByteLimit
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: execution record exceeds byte limit"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::NonpositivePriceOrQuantity))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget nonpositive price or quantity"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::ExecutionCashPartitionsDiffer))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper ledger integrity failure: execution cash partitions differ"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::DescriptorIsInvalid))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget descriptor is invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(
                        fw::BudgetText::CompleteOrderedLotAllocationIsInvalid
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget complete ordered lot allocation is invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::AllocationDoesNotMatchGenesis))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget allocation does not match genesis"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::DuplicateGenesisLot))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget duplicate genesis lot"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::UnknownGenesisLot))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget unknown genesis lot"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::GenesisQuantityChanged))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget genesis quantity changed"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::GenesisMarkAbsent))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget genesis mark absent"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(
                        fw::BudgetText::InitialAllocatedCapitalExceedsFixedBudget
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget initial allocated capital exceeds fixed budget"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::MarkedAllocationIsInvalid))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget marked allocation is invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(fw::BudgetText::WorkingReservationIsInvalid))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget working reservation is invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Budget(
                        fw::BudgetText::NewBuyExceedsFixedAllInOrCashLimits
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent budget new buy exceeds fixed all-in or cash limits"
            );
            assert_eq!(match w.error(Txt::Fill(fw::FillText::ExecutionWindowShanghaiClockExceedsSupportedRange)).unwrap(){
                fw::FinancialFailure::Financial(e)=>e.to_string(),
                _=>panic!("wrong category")
            }, "paper evidence unavailable: execution window Shanghai clock exceeds supported range");
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::ExecutionWindowIsNotAdmissible))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: execution window is not admissible"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::ParentWholeLotBoundsInvalid))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: parent whole-lot bounds invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::ExecutionPriceExceedsFrozenFeeCap))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: execution price exceeds frozen fee cap"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::OddLotModelUnavailable))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: odd lot model unavailable"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::FeeDescriptorUTF8Unavailable))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: fee descriptor UTF8 unavailable"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::FeeDescriptorRateUnavailable))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: fee descriptor rate unavailable"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::FeeDescriptorRateDuplicated))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: fee descriptor rate duplicated"
            );
            assert_eq!(
                match w
                    .error(Txt::Fill(fw::FillText::FeeDescriptorRateInvalid))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "paper evidence unavailable: fee descriptor rate invalid"
            );
            assert_eq!(
                match w
                    .error(Txt::Intent(
                        fw::IntentText::ClosedParentIntentBindingInvalid
                    ))
                    .unwrap()
                {
                    fw::FinancialFailure::Financial(e) => e.to_string(),
                    _ => panic!("wrong category"),
                },
                "invalid paper input: closed parent intent binding invalid"
            );
            let l = lot("lot", 100, 0);
            assert_eq!(w.text(Txt::MissingMark(&l)).unwrap(), "missing mark 600001");
            assert_eq!(
                w.text(Txt::SeedLotOrdinal(usize::MAX)).unwrap(),
                format!("seed:{}", usize::MAX)
            );
        }
        Case::FeeDescriptor => {
            let p = fee();
            assert_eq!(w.fee_descriptor(&p).unwrap(), p.canonical_bytes());
            let before = w.used();
            assert_eq!(w.fee_instance(&p).unwrap(), p.instance_id());
            let once = w.used() - before;
            assert_eq!(w.fee_instance(&p).unwrap(), p.instance_id());
            assert_eq!(w.used() - before, 2 * once);
        }
        Case::SourceRevision => {
            let p = fee();
            let before = w.used();
            let bad = "x".repeat(129);
            let e = AShareFeePolicyV2::new_with_work(
                p.scope(),
                FeeRate::new(3, 10000).unwrap(),
                5_000_000,
                FeeCoverage::initial_model(),
                &bad,
                w,
            )
            .unwrap_err();
            assert!(matches!(
                e,
                fw::FinancialFailure::Fee(AShareFeeV2Error::InvalidSourceRevision)
            ));
            assert_eq!(w.used() - before, 129);
            let before = w.used();
            assert!(AShareFeePolicyV2::new_with_work(
                p.scope(),
                FeeRate::new(3, 10000).unwrap(),
                -1,
                FeeCoverage::initial_model(),
                &bad,
                w
            )
            .is_err());
            assert_eq!(w.used(), before);
        }
        Case::FeeOverflowOrder => {
            let p = AShareFeePolicyV2::new(
                fee().scope(),
                FeeRate::new(1, 1).unwrap(),
                0,
                FeeCoverage::initial_model(),
                "x",
            )
            .unwrap();
            let before = w.used();
            let e = fill_fee_with_work(
                &p,
                p.scope(),
                crate::performance::fee_evidence::FillSide::Sell,
                i64::MAX,
                date(),
                FeeCoverageRequirement::ModeledComponentsOnly,
                w,
            )
            .unwrap_err();
            assert!(matches!(
                e,
                fw::FinancialFailure::Fee(AShareFeeV2Error::Overflow)
            ));
            assert!(w.used() > before);
        }
        Case::Hashes => {
            let g = genesis(100_000_000_000, vec![lot("a", 100, 17)]);
            let m = manifest(&g, 100_000_000_000, 200_000_000_000);
            let win = window("w", 0, 10_000_000, 100);
            for input in [
                fw::ClosedFinancialHash::Inventory(&g.lots),
                fw::ClosedFinancialHash::ExecutionManifest(&m),
                fw::ClosedFinancialHash::ExecutionWindow(&win),
                fw::ClosedFinancialHash::FillIdentity {
                    account: "a",
                    parent: "p",
                    observation: "o",
                },
            ] {
                let wanted = match &input {
                    fw::ClosedFinancialHash::Inventory(v) => {
                        reference_hash(None, &serde_json::to_vec(v).unwrap())
                    }
                    fw::ClosedFinancialHash::ExecutionManifest(v) => reference_hash(
                        Some(b"paper-parent-execution-manifest/v1"),
                        &serde_json::to_vec(v).unwrap(),
                    ),
                    fw::ClosedFinancialHash::ExecutionWindow(v) => reference_hash(
                        Some(b"paper-execution-window/v1"),
                        &serde_json::to_vec(v).unwrap(),
                    ),
                    _ => reference_hash(
                        Some(b"paper-parent-fill-id/v1"),
                        &serde_json::to_vec(&("a", "p", "o")).unwrap(),
                    ),
                };
                assert_eq!(w.fixed_hash(input).unwrap(), wanted);
            }
            assert_eq!(w.hash_hits(), [4, 3, 4, 4]);
            let mut destination_entries = [0; 11];
            destination_entries[10] = 4;
            assert_eq!(w.operation_entries(), destination_entries);
        }
        Case::HistoricalExtent => {
            let g = genesis(1, vec![]);
            let mut m = manifest(&g, 1, 2);
            m.approved_reference.clear();
            let overhead = serde_json::to_vec(&m).unwrap().len();
            m.approved_reference = "x".repeat(32 * 1024 * 1024 - overhead);
            let bytes = serde_json::to_vec(&m).unwrap();
            assert_eq!(bytes.len(), 32 * 1024 * 1024);
            assert_eq!(
                m.identity().unwrap(),
                reference_hash(Some(b"paper-parent-execution-manifest/v1"), &bytes)
            );
            m.approved_reference.push('x');
            assert_eq!(
                m.identity().unwrap_err().to_string(),
                "paper ledger integrity failure: execution record exceeds byte limit"
            );
            assert_eq!(w.used(), 0);
        }
        Case::OversizedResource => {
            let g = genesis(1, vec![]);
            let mut m = manifest(&g, 1, 2);
            m.approved_reference = "x".repeat(32 * 1024 * 1024);
            let json_len = serde_json::to_vec(&m).unwrap().len() as u64;
            let attempted = w.used() + work::financial_fixture_serializer_escrow_bytes() + json_len;
            let mut expected = None;
            for _ in 0..2 {
                match w
                    .fixed_hash(fw::ClosedFinancialHash::ExecutionManifest(&m))
                    .unwrap_err()
                {
                    fw::FinancialFailure::Terminal(actual) => {
                        if let Some(first) = expected {
                            assert_eq!(actual, first);
                        } else {
                            expected = Some(actual);
                        }
                        w.assert_fixture_resource(attempted);
                    }
                    other => panic!("unexpected oversized failure: {other:?}"),
                }
                assert_eq!(w.used(), attempted);
                assert_eq!(w.operation_entries(), [0; 11]);
                assert_eq!(w.hash_hits(), [0; 4]);
                match w.finish().unwrap_err() {
                    fw::FinancialFailure::Terminal(actual) => assert_eq!(Some(actual), expected),
                    other => panic!("first terminal changed: {other:?}"),
                }
            }
        }
        Case::Collections => {
            let mut set = BTreeSet::new();
            assert!(w.set(&mut set, "a").unwrap());
            let before = w.used();
            assert!(!w.set(&mut set, "a").unwrap());
            assert!(w.used() > before);
            let l = lot("a", 100, 0);
            let mut lots = BTreeMap::new();
            w.lot_ref(&mut lots, "a", &l).unwrap();
            assert_eq!(lots["a"].quantity, 100);
            let mut fields = BTreeMap::new();
            assert_eq!(w.descriptor(&mut fields, "a", "b").unwrap(), None);
            assert_eq!(w.descriptor(&mut fields, "a", "c").unwrap(), Some("b"));
            let mut claims = BTreeMap::new();
            *w.claim(&mut claims, "a").unwrap() += 3;
            *w.claim(&mut claims, "a").unwrap() += 4;
            assert_eq!(claims["a"], 7);
            let mut exposure = BTreeMap::new();
            *w.exposure(&mut exposure, "a").unwrap() += i128::MAX;
            assert_eq!(exposure["a"], i128::MAX);
        }
        Case::Growth => {
            let mut values = Vec::new();
            let first = w.copy(&"a".to_string()).unwrap();
            let before = w.used();
            w.push(&mut values, first).unwrap();
            assert!(w.used() > before);
            let cap = values.capacity();
            for _ in 1..cap {
                let v = w.copy(&"b".to_string()).unwrap();
                let before = w.used();
                w.push(&mut values, v).unwrap();
                assert_eq!(w.used(), before);
            }
            let v = w.copy(&"c".to_string()).unwrap();
            let before = w.used();
            w.push(&mut values, v).unwrap();
            assert!(w.used() > before);
            assert_eq!(values.len(), cap + 1);
        }
        Case::Sort => {
            let a = lot("a", 100, 0);
            let b = lot("b", 100, 0);
            let mut lots = Vec::new();
            w.push(&mut lots, &b).unwrap();
            w.push(&mut lots, &a).unwrap();
            let before = w.used();
            w.sort_fifo(&mut lots).unwrap();
            let charge = w.used() - before;
            assert!(charge > 0);
            assert_eq!(lots[0].lot_id, "a");
            w.sort_fifo(&mut lots).unwrap();
            assert_eq!(w.used() - before, 2 * charge);
        }
        Case::NodeShort => boundary_case(case, w),
        Case::GrowShort => boundary_case(case, w),
        Case::SortShort => boundary_case(case, w),
        Case::TextShort => boundary_case(case, w),
        Case::Cumulative => {
            let value = "x".repeat(1_000_000);
            let mut successes = 0;
            loop {
                match w.copy(&value) {
                    Ok(v) => {
                        assert_eq!(v, value);
                        successes += 1;
                    }
                    Err(_) => break,
                }
            }
            assert!(successes > 1);
            terminal_retry(w);
        }
        Case::Grown => {
            let (mut state, mut plain, m) = state_pair(vec![lot("old", 100, 17)], w);
            let commands = vec![
                CommandRecord::QualifiedMarks {
                    expected: head(),
                    windows: vec![window("m0", 0, 10_000_000, 100)],
                },
                CommandRecord::Submit {
                    expected: head(),
                    intent: intent(&m, "parent", Side::Buy, 300, 1),
                },
                CommandRecord::Evaluate {
                    expected: head(),
                    parent_id: "parent".into(),
                    window: window("nf", 2, 10_000_000, 0),
                },
                CommandRecord::Evaluate {
                    expected: head(),
                    parent_id: "parent".into(),
                    window: window("f1", 3, 10_000_000, 100),
                },
                CommandRecord::Evaluate {
                    expected: head(),
                    parent_id: "parent".into(),
                    window: window("f2", 4, 10_000_000, 100),
                },
                CommandRecord::Cancel {
                    expected: head(),
                    parent_id: "parent".into(),
                    at: at(5),
                },
                CommandRecord::QualifiedMarks {
                    expected: head(),
                    windows: vec![window("m1", 6, 10_000_000, 100)],
                },
            ];
            for (i, c) in commands.into_iter().enumerate() {
                let e = step(&mut state, &mut plain, &m, c, w);
                if i == 2 {
                    assert!(matches!(e, Effect::ObservedNoFill(_)));
                    assert_eq!(state.fills.len(), 0);
                }
                if i == 4 {
                    assert_eq!(state.fills.len(), 2);
                }
            }
            assert_eq!(state.parents["parent"].filled, 200);
            assert_eq!(state.parents["parent"].cancelled, 100);
            assert_eq!(state.account.lots.len(), 3);
        }
        Case::SellFifo => {
            let (mut state, mut plain, m) =
                state_pair(vec![lot("b", 200, 19), lot("a", 200, 17)], w);
            step(
                &mut state,
                &mut plain,
                &m,
                CommandRecord::Submit {
                    expected: head(),
                    intent: intent(&m, "sell", Side::Sell, 300, 1),
                },
                w,
            );
            assert_eq!(state.parents["sell"].sell_claims[0].lot_id, "a");
            step(
                &mut state,
                &mut plain,
                &m,
                CommandRecord::Evaluate {
                    expected: head(),
                    parent_id: "sell".into(),
                    window: window("sf1", 2, 10_000_000, 100),
                },
                w,
            );
            assert_eq!(state.fills[0].inherited_buy_fee_micro_cny, 8);
            step(
                &mut state,
                &mut plain,
                &m,
                CommandRecord::Evaluate {
                    expected: head(),
                    parent_id: "sell".into(),
                    window: window("sf2", 3, 10_000_000, 100),
                },
                w,
            );
            assert_eq!(state.fills[1].inherited_buy_fee_micro_cny, 9);
            assert!(state.account.lots.iter().all(|l| l.lot_id != "a"));
        }
        Case::Expire => {
            let (mut state, mut plain, m) = state_pair(vec![], w);
            step(
                &mut state,
                &mut plain,
                &m,
                CommandRecord::Submit {
                    expected: head(),
                    intent: intent(&m, "p", Side::Buy, 100, 1),
                },
                w,
            );
            let e = step(
                &mut state,
                &mut plain,
                &m,
                CommandRecord::Expire {
                    expected: head(),
                    parent_id: "p".into(),
                    at: at(0) + chrono::Duration::hours(6),
                },
                w,
            );
            assert!(matches!(e, Effect::Expired));
            assert_eq!(state.parents["p"].reservation.cash_reserve, 0);
        }
        Case::StagedInvalid => {
            let (mut state, _, mut m) = state_pair(vec![lot("a", 100, 0)], w);
            let original = state.clone();
            m.version = "bad".into();
            let before = w.used();
            let e = execution::apply_request_with_work(
                &mut state,
                &m,
                &CommandRecord::Open {
                    manifest: m.clone(),
                },
                w,
            )
            .unwrap_err();
            assert!(matches!(
                e,
                fw::FinancialFailure::Financial(ledger::LedgerError::IntegrityFailure(_))
            ));
            assert_eq!(state, original);
            assert!(w.used() > before);
            assert!(w.finish().is_ok());
        }
        Case::StagedResource => {
            let (mut state, _, m) = state_pair(vec![], w);
            state.account.marks.insert(
                "large".into(),
                Mark {
                    code: "large".into(),
                    price: Money::from_micros(1),
                    observed_at: at(0),
                    source: "x".repeat(17 * 1024 * 1024),
                },
            );
            let original = state.clone();
            assert!(execution::apply_request_with_work(
                &mut state,
                &m,
                &CommandRecord::Open {
                    manifest: m.clone()
                },
                w
            )
            .is_err());
            assert_eq!(state, original);
            terminal_retry(w);
        }
        Case::V1 => {
            let seed = SeedManifest {
                account_id: "a".into(),
                epoch_id: "e".into(),
                command_id: "c".into(),
                cutover_at: at(0),
                account_effective_at: at(0),
                positions_effective_at: at(0),
                source_reference: "fixture".into(),
                source_hash: "a".repeat(64),
                approved_by: "test".into(),
                cash: Money::from_micros(100_000_000_000),
                original_total: Money::from_micros(101_000_000_000),
                excluded_residual: None,
                lots: vec![SeedLot {
                    code: "600001".into(),
                    name: "fixture".into(),
                    quantity: 100,
                    reported_cost: None,
                    sellable_from: None,
                    sellability_evidence: None,
                }],
                marks: vec![Mark {
                    code: "600001".into(),
                    price: Money::from_micros(10_000_000),
                    observed_at: at(0),
                    source: "fixture".into(),
                }],
                policy: RiskPolicyV1::default(),
            };
            let paid = ledger::seed_projection_with_work(&seed, w).unwrap();
            let plain =
                ledger::seed_projection_with_work(&seed, &mut FinancialWork::Historical).unwrap();
            assert_eq!(paid, plain);
            assert_eq!(paid.lots[0].lot_id, "seed:0");
            ledger::transition_v1_fixture(paid, w);
        }
        Case::Book => super::paper_book_v2::transition_genesis_fixture(w),
        Case::IntentOrder => {
            let g = genesis(1, vec![]);
            let m = manifest(&g, 1, 2);
            let mut i = intent(&m, "p", Side::Buy, 100, 0);
            i.version = "bad".into();
            i.source_window.version = "bad-window".into();
            let e = i.validate_with_work(w).unwrap_err();
            assert!(matches!(
                e,
                fw::FinancialFailure::Financial(ledger::LedgerError::EvidenceUnavailable(_))
            ));
            i.source_window.version = MODEL_VERSION.into();
            let e = i.validate_with_work(w).unwrap_err();
            assert!(matches!(
                e,
                fw::FinancialFailure::Financial(ledger::LedgerError::InvalidInput(_))
            ));
        }
    }
}
fn reference_hash(domain: Option<&[u8]>, bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    if let Some(d) = domain {
        h.update(d);
        h.update(b"\n");
    }
    h.update(bytes);
    hex::encode(h.finalize())
}
#[test]
fn transition_lot_map_short() {
    work::financial_fixture(Case::LotMapShort);
}
#[test]
fn transition_descriptor_map_short() {
    work::financial_fixture(Case::DescriptorMapShort);
}
#[test]
fn transition_claim_short() {
    work::financial_fixture(Case::ClaimShort);
}
#[test]
fn transition_exposure_short() {
    work::financial_fixture(Case::ExposureShort);
}
#[test]
fn transition_set_exact() {
    work::financial_fixture(Case::SetExact);
}
#[test]
fn transition_lot_map_exact() {
    work::financial_fixture(Case::LotMapExact);
}
#[test]
fn transition_descriptor_map_exact() {
    work::financial_fixture(Case::DescriptorMapExact);
}
#[test]
fn transition_claim_exact() {
    work::financial_fixture(Case::ClaimExact);
}
#[test]
fn transition_exposure_exact() {
    work::financial_fixture(Case::ExposureExact);
}
#[test]
fn transition_grow_exact() {
    work::financial_fixture(Case::GrowExact);
}
#[test]
fn transition_sort_exact() {
    work::financial_fixture(Case::SortExact);
}
#[test]
fn transition_text_exact() {
    work::financial_fixture(Case::TextExact);
}
fn boundary_case(case: Case, w: &mut FinancialWork<'_, '_>) {
    let exact = matches!(
        case,
        Case::SetExact
            | Case::LotMapExact
            | Case::DescriptorMapExact
            | Case::ClaimExact
            | Case::ExposureExact
            | Case::GrowExact
            | Case::SortExact
            | Case::TextExact
    );
    let entry = match case {
        Case::NodeShort | Case::SetExact => 0,
        Case::LotMapShort | Case::LotMapExact => 1,
        Case::DescriptorMapShort | Case::DescriptorMapExact => 2,
        Case::ClaimShort | Case::ClaimExact => 3,
        Case::ExposureShort | Case::ExposureExact => 4,
        Case::GrowShort | Case::GrowExact => 5,
        Case::SortShort | Case::SortExact => 7,
        Case::TextShort | Case::TextExact => 8,
        _ => unreachable!(),
    };
    let a = lot("a", 100, 0);
    let b = lot("b", 100, 0);
    let mut first_failure = None;
    let mut first_usage = None;
    let mut after_success = [0; 11];
    let mut set = BTreeSet::new();
    let mut refs = BTreeMap::new();
    let mut fields = BTreeMap::new();
    let mut claims = BTreeMap::new();
    let mut exposures = BTreeMap::new();
    let mut values: Vec<String> = Vec::new();
    let mut fifo = vec![&b, &a];
    for attempt in 0..3 {
        if exact && attempt == 1 {
            // Drop the successful destination without refund; a fresh destination
            // makes the next same named request require real insertion/backing.
            set = BTreeSet::new();
            refs = BTreeMap::new();
            fields = BTreeMap::new();
            claims = BTreeMap::new();
            exposures = BTreeMap::new();
            values = Vec::new();
            fifo = vec![&b, &a];
        }
        // After a refusal, retry the same operation against the same destination.
        let result = match entry {
            0 => w.set(&mut set, "a").map(|inserted| assert!(inserted)),
            1 => w.lot_ref(&mut refs, "a", &a),
            2 => w
                .descriptor(&mut fields, "a", "value")
                .map(|previous| assert!(previous.is_none())),
            3 => w.claim(&mut claims, "a").map(|slot| {
                assert_eq!(*slot, 0);
                *slot = 7;
            }),
            4 => w.exposure(&mut exposures, "a").map(|slot| {
                assert_eq!(*slot, 0);
                *slot = 9;
            }),
            5 => w.push(&mut values, String::new()),
            7 => w.sort_fifo(&mut fifo),
            8 => w
                .text(Txt::ProjectionVersion)
                .map(|text| assert_eq!(text, "paper-parent-projection/v1")),
            _ => unreachable!(),
        };
        if exact && attempt == 0 {
            result.unwrap();
            assert_eq!(w.used(), 16 * 1024 * 1024);
            assert!(w.finish().is_ok());
            after_success[entry] = 1;
            if entry == 5 {
                after_success[6] = 1;
            }
            if entry == 8 {
                after_success[9] = 1;
            }
            assert_eq!(w.operation_entries(), after_success);
            match entry {
                0 => assert!(set.contains("a")),
                1 => assert_eq!(refs["a"].lot_id, "a"),
                2 => assert_eq!(fields["a"], "value"),
                3 => assert_eq!(claims["a"], 7),
                4 => assert_eq!(exposures["a"], 9),
                5 => {
                    assert_eq!(values.len(), 1);
                    assert!(values.capacity() > 0);
                }
                7 => assert_eq!(fifo[0].lot_id, "a"),
                _ => {}
            }
        } else {
            let failure = match result.unwrap_err() {
                fw::FinancialFailure::Terminal(e) => e,
                other => panic!("wrong boundary failure {other:?}"),
            };
            let expected_usage = if exact {
                16 * 1024 * 1024 + w.expected_boundary_cost()
            } else {
                16 * 1024 * 1024 + 1
            };
            w.assert_fixture_resource(expected_usage);
            assert!(w.used() > 16 * 1024 * 1024);
            if !exact {
                assert_eq!(w.used(), 16 * 1024 * 1024 + 1);
            }
            if let Some(first) = first_failure {
                assert_eq!(failure, first);
                assert_eq!(Some(w.used()), first_usage);
            } else {
                first_failure = Some(failure);
                first_usage = Some(w.used());
            }
            assert_eq!(w.operation_entries(), after_success);
            assert!(
                set.is_empty()
                    && refs.is_empty()
                    && fields.is_empty()
                    && claims.is_empty()
                    && exposures.is_empty()
            );
            assert!(values.is_empty());
            assert_eq!(values.capacity(), 0);
            assert_eq!(fifo[0].lot_id, "b");
            match w.finish().unwrap_err() {
                fw::FinancialFailure::Terminal(e) => assert_eq!(e, failure),
                other => panic!("terminal changed {other:?}"),
            }
        }
    }
}
