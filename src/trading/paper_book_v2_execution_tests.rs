//! Financial/replay predicates plus genuine isolated GlobalV6 lifecycle.
//! Explicit synthetic Test source/approval facts are never production authority.
use super::budget::{InitialLotAllocation, ProfitPolicy};
use super::*;
use crate::decision::approved_paper_intent_v1::{TimeInForce, INTENT_VERSION};
use chrono::TimeZone;

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
    serde_json::from_value(serde_json::json!({"cash":cash,"lots":lots,"marks":marks,"fees":0,"realized_pnl":0,"seed_equity":cash,"as_of":at(0),"closes":{}})).unwrap()
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
        genesis_projection_hash: raw_hash(&encode(g).unwrap()),
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
fn submit(
    state: &mut ExecutionProjection,
    m: &ExecutionManifest,
    id: &str,
    side: Side,
    q: u32,
    second: u32,
) -> Result<Effect, LedgerError> {
    apply_request(
        state,
        m,
        &CommandRecord::Submit {
            expected: head(),
            intent: intent(m, id, side, q, second),
        },
    )
}
fn evaluate(
    state: &mut ExecutionProjection,
    m: &ExecutionManifest,
    parent: &str,
    id: &str,
    second: u32,
    volume: u32,
) -> Result<Effect, LedgerError> {
    apply_request(
        state,
        m,
        &CommandRecord::Evaluate {
            expected: head(),
            parent_id: parent.into(),
            window: window(id, second, 10_000_000, volume),
        },
    )
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

#[test]
fn paper_v2_execution_budget_initial_cap_includes_full_genesis_allocated_marks() {
    let g = genesis(200_000_000_000, vec![]);
    assert!(policy(&g, 200_000_000_000, 100_000_000_000)
        .initial_cash(&g)
        .is_err());
    let g = genesis(100_000_000_000, vec![lot("old", 100, 17)]);
    let p = policy(&g, 100_000_000_000, 100_000_000_000);
    assert!(p.initial_cash(&g).is_err());
    let p = policy(&g, 99_000_000_000, 100_000_000_000);
    assert_eq!(p.initial_cash(&g).unwrap().unassigned_cash, 1_000_000_000);
    let mut bad = p.clone();
    bad.initial_lots[0].original_quantity = 99;
    assert!(bad.initial_cash(&g).is_err());
    bad = p;
    bad.initial_lots.clear();
    assert!(bad.initial_cash(&g).is_err());
}
#[test]
fn paper_v2_execution_budget_large_account_cash_cannot_pay_all_in_b_fees() {
    let g = genesis(200_000_000_000, vec![]);
    let p = policy(&g, 100_000_000_000, 100_000_000_000);
    let cash = p.initial_cash(&g).unwrap();
    let r = WorkingReservation {
        parent_id: "parent".into(),
        code: "600001".into(),
        chain_id: "chain".into(),
        buy_max_notional: 100_000_000_000,
        fee_reserve: 5_000_000,
        cash_reserve: 100_005_000_000,
    };
    assert!(budget::require_new_buy(&p, &cash, &[], &[], &r).is_err());
    let mut under = r.clone();
    under.cash_reserve = 0;
    assert!(budget::require_new_buy(&p, &cash, &[], &[], &under).is_err());
}
#[test]
fn paper_v2_execution_recorded_parent_partial_no_fill_cancel_preserves_cash_partitions() {
    let g = genesis(20_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "buy", Side::Buy, 300, 1).unwrap();
    assert_eq!(state.parents["buy"].reservation.cash_reserve, 3_015_000_000);
    evaluate(&mut state, &m, "buy", "window1", 2, 100).unwrap();
    let after_first = state.clone();
    assert!(matches!(
        evaluate(&mut state, &m, "buy", "noFill", 3, 0).unwrap(),
        Effect::ObservedNoFill(_)
    ));
    assert_eq!(state.cash, after_first.cash);
    assert_eq!(
        state.parents["buy"].reservation,
        after_first.parents["buy"].reservation
    );
    evaluate(&mut state, &m, "buy", "window2", 4, 100).unwrap();
    assert_eq!(state.parents["buy"].remaining, 100);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::Cancel {
            expected: head(),
            parent_id: "buy".into(),
            at: at(5),
        },
    )
    .unwrap();
    assert_eq!(state.parents["buy"].status, ParentStatus::Cancelled);
    assert_eq!(state.parents["buy"].remaining, 0);
    assert_eq!(state.parents["buy"].cancelled, 100);
    assert_eq!(state.parents["buy"].reservation.cash_reserve, 0);
    assert_eq!(state.cash.strategy_cash, 7_990_000_000);
    assert_eq!(state.cash.unassigned_cash, 10_000_000_000);
    assert_eq!(state.cash.account_cash, 17_990_000_000);
    assert_eq!(state.account.fees.micros(), 10_000_000);
    assert!(apply_request(
        &mut state,
        &m,
        &CommandRecord::Cancel {
            expected: head(),
            parent_id: "buy".into(),
            at: at(6)
        }
    )
    .is_err());
    state.validate().unwrap();
}
#[test]
fn paper_v2_execution_recorded_three_partial_fills_keep_each_minimum_and_t1() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "buy", Side::Buy, 300, 1).unwrap();
    for (id, second) in [("one", 2), ("two", 3), ("three", 4)] {
        evaluate(&mut state, &m, "buy", id, second, 100).unwrap();
    }
    assert_eq!(state.parents["buy"].status, ParentStatus::Filled);
    assert_eq!(state.account.fees.micros(), 15_000_000);
    assert_eq!(state.fills.len(), 3);
    assert!(state
        .account
        .lots
        .iter()
        .all(|l| l.quantity == 100
            && l.sellable_from == NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()));
    assert!(submit(&mut state, &m, "same-day-sell", Side::Sell, 100, 5).is_err());
}
#[test]
fn paper_v2_execution_recorded_orders_cannot_double_claim_cash_or_fifo_inventory() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "one", Side::Buy, 600, 1).unwrap();
    assert!(submit(&mut state, &m, "two", Side::Buy, 600, 2).is_err());
    let g = genesis(10_000_000_000, vec![lot("old", 300, 9_000_000)]);
    let m = manifest(&g, 6_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "sell1", Side::Sell, 200, 1).unwrap();
    assert!(submit(&mut state, &m, "sell2", Side::Sell, 200, 2).is_err());
    evaluate(&mut state, &m, "sell1", "sell-window", 3, 100).unwrap();
    assert_eq!(state.account.lots[0].quantity, 200);
    assert_eq!(state.account.lots[0].buy_fee_remaining.micros(), 6_000_000);
    assert_eq!(state.fills[0].inherited_buy_fee_micro_cny, 3_000_000);
    submit(&mut state, &m, "sell2", Side::Sell, 100, 4).unwrap();
    state.validate().unwrap();
}
#[test]
fn paper_v2_execution_recorded_mark_appreciation_retains_old_reserve_and_denies_new_buy() {
    let g = genesis(6_000_000_000, vec![lot("old", 300, 0)]);
    let m = manifest(&g, 4_000_000_000, 7_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "one", Side::Buy, 100, 1).unwrap();
    let reserve = state.parents["one"].reservation.clone();
    let mark = window("high", 2, 30_000_000, 0);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::QualifiedMarks {
            expected: head(),
            windows: vec![mark],
        },
    )
    .unwrap();
    assert_eq!(state.parents["one"].reservation, reserve);
    assert_eq!(state.account.marks["600001"].price.micros(), 30_000_000);
    // A fresh approval at the observed higher mark cannot silently replace the
    // over-budget valuation with a lower price just to obtain admission.
    let mut new = intent(&m, "two", Side::Buy, 200, 3);
    new.source_window.price_micro_cny = 30_000_000;
    new.limit_micro_cny = 30_000_000;
    new.fee_price_cap_micro_cny = 30_000_000;
    assert!(apply_request(
        &mut state,
        &m,
        &CommandRecord::Submit {
            expected: head(),
            intent: new
        }
    )
    .is_err());
    apply_request(
        &mut state,
        &m,
        &CommandRecord::Cancel {
            expected: head(),
            parent_id: "one".into(),
            at: at(4),
        },
    )
    .unwrap();
}
#[test]
fn paper_v2_execution_cash_partition_loss_never_borrows_unassigned() {
    let mut cash = CashPartitions {
        account_cash: 200,
        strategy_cash: 100,
        unassigned_cash: 100,
    };
    cash.apply_strategy_delta(-100).unwrap();
    assert_eq!(cash.strategy_cash, 0);
    assert!(cash.apply_strategy_delta(-1).is_err());
    assert_eq!(cash.account_cash, 100);
    assert_eq!(cash.unassigned_cash, 100);
    cash.apply_strategy_delta(20).unwrap();
    assert_eq!(cash.strategy_cash, 20);
    assert_eq!(cash.account_cash, 120);
    assert_eq!(cash.unassigned_cash, 100);
}
#[test]
fn paper_v2_execution_integer_money_above_f64_precision_remains_exact() {
    let value = 9_007_199_254_740_993_i64;
    let money = Money::from_micros(value);
    assert_eq!(money.micros(), value);
    let g = genesis(value, vec![]);
    let p = policy(&g, value, value);
    let cash = p.initial_cash(&g).unwrap();
    assert_eq!(cash.strategy_cash, value);
    assert_eq!(cash.unassigned_cash, 0);
    assert_eq!(
        decode::<Projection>(&encode(&g).unwrap())
            .unwrap()
            .cash
            .micros(),
        value
    );
}
#[test]
fn paper_v2_execution_same_window_cannot_fill_two_parents_or_reuse_no_fill() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "one", Side::Buy, 100, 1).unwrap();
    submit(&mut state, &m, "two", Side::Buy, 100, 2).unwrap();
    evaluate(&mut state, &m, "one", "shared", 3, 0).unwrap();
    assert!(evaluate(&mut state, &m, "two", "shared", 4, 100).is_err());
    assert_eq!(state.fills.len(), 0);
}
#[test]
fn paper_v2_execution_missing_or_invalid_window_is_not_observed_no_fill() {
    let mut w = window("invalid", 1, 10_000_000, 100);
    w.listed = false;
    assert!(fill_model::model(Side::Buy, 100, 10_000_000, 10_000_000, &w, &fee()).is_err());
    w.listed = true;
    w.price_micro_cny += 1;
    assert!(fill_model::model(Side::Buy, 100, 10_000_000, 10_000_000, &w, &fee()).is_err());
    w.price_micro_cny = 10_000_000;
    w.suspended = true;
    assert!(matches!(
        fill_model::model(Side::Buy, 100, 10_000_000, 10_000_000, &w, &fee()).unwrap(),
        ModelOutcome::NoFill(fill_model::NoFillReason::Suspended)
    ));
    w.suspended = false;
    w.session_date = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
    assert!(w.validate().is_err());
}
#[test]
fn paper_v2_execution_closed_records_reject_unknown_noncanonical_and_altered_fees() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let raw = encode(&m).unwrap();
    assert_eq!(decode::<ExecutionManifest>(&raw).unwrap(), m);
    let mut value = serde_json::to_value(&m).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("approved".into(), serde_json::json!(true));
    assert!(decode::<ExecutionManifest>(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut whitespace = vec![b' '];
    whitespace.extend(&raw);
    assert!(decode::<ExecutionManifest>(&whitespace).is_err());
    let mut bad = m.fee_descriptor.clone();
    bad.push(b'\n');
    assert!(fee_from_record(&bad).is_err());
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
}
#[test]
fn paper_v2_execution_cancel_command_identity_preserves_retry_without_new_clock() {
    let original = CommandRecord::Cancel {
        expected: head(),
        parent_id: "one".into(),
        at: at(1),
    };
    let later = CommandRecord::Cancel {
        expected: head(),
        parent_id: "one".into(),
        at: at(2),
    };
    assert_ne!(encode(&original).unwrap(), encode(&later).unwrap());
    assert_eq!(
        command_hash(&original).unwrap(),
        command_hash(&later).unwrap()
    );
    let changed = CommandRecord::Cancel {
        expected: head(),
        parent_id: "two".into(),
        at: at(2),
    };
    assert_ne!(
        command_hash(&original).unwrap(),
        command_hash(&changed).unwrap()
    );
}
#[test]
fn paper_v2_execution_local_schema_has_no_header_or_qualification_effect() {
    let mut conn = SqliteConnection::establish(":memory:").unwrap();
    crate::database::paper_book_v2_execution_schema_v1::create_schema(&mut conn).unwrap();
    crate::database::paper_book_v2_execution_schema_v1::verify_objects_on(&mut conn).unwrap();
    assert_eq!(
        diesel::sql_query("SELECT user_version AS value FROM pragma_user_version()")
            .get_result::<Scalar>(&mut conn)
            .unwrap()
            .value,
        0
    );
    assert!(sql_rows(&mut conn).unwrap().events.is_empty());
    diesel::sql_query("DROP TRIGGER paper_book_v2_execution_head_cas")
        .execute(&mut conn)
        .unwrap();
    assert!(
        crate::database::paper_book_v2_execution_schema_v1::verify_objects_on(&mut conn).is_err()
    );
    assert_eq!(
        diesel::sql_query("SELECT COUNT(*) AS value FROM paper_book_v2_execution_head")
            .get_result::<Scalar>(&mut conn)
            .unwrap()
            .value,
        0
    );
}
#[test]
fn paper_v2_execution_local_schema_rejects_temp_shadow_without_healing() {
    let mut conn = SqliteConnection::establish(":memory:").unwrap();
    crate::database::paper_book_v2_execution_schema_v1::create_schema(&mut conn).unwrap();
    diesel::sql_query("CREATE TEMP TABLE paper_book_v2_parent_order(unknown TEXT)")
        .execute(&mut conn)
        .unwrap();
    assert!(
        crate::database::paper_book_v2_execution_schema_v1::verify_objects_on(&mut conn).is_err()
    );
    assert_eq!(diesel::sql_query("SELECT COUNT(*) AS value FROM temp.sqlite_master WHERE name='paper_book_v2_parent_order'").get_result::<Scalar>(&mut conn).unwrap().value,1);
}

#[test]
fn paper_v2_execution_recorded_assigned_profit_can_be_reused_without_expanding_fixed_b() {
    let g = genesis(20_000_000_000, vec![lot("old", 100, 0)]);
    let m = manifest(&g, 2_000_000_000, 3_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    let mut sell = intent(&m, "sell", Side::Sell, 100, 1);
    sell.fee_price_cap_micro_cny = 30_000_000;
    apply_request(
        &mut state,
        &m,
        &CommandRecord::Submit {
            expected: head(),
            intent: sell,
        },
    )
    .unwrap();
    let sold = window("profit", 2, 20_000_000, 100);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::Evaluate {
            expected: head(),
            parent_id: "sell".into(),
            window: sold,
        },
    )
    .unwrap();
    assert_eq!(state.cash.strategy_cash, 3_994_000_000);
    assert_eq!(state.cash.unassigned_cash, 18_000_000_000);
    assert_eq!(state.account.realized_pnl.micros(), 994_000_000);
    assert_eq!(m.budget.authorized_budget_micro_cny, 3_000_000_000);
    submit(&mut state, &m, "reuse", Side::Buy, 100, 3).unwrap();
    let before = state.clone();
    assert!(submit(&mut state, &m, "expand", Side::Buy, 200, 4).is_err());
    assert_eq!(state, before);
}

#[test]
fn paper_v2_execution_recorded_failed_order_leaves_original_reservations_exact() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "one", Side::Buy, 100, 1).unwrap();
    let original = state.clone();
    let mut other = intent(&m, "other", Side::Buy, 100, 2);
    other.investment_decision_id = state.parents["one"].intent.investment_decision_id.clone();
    assert!(apply_request(
        &mut state,
        &m,
        &CommandRecord::Submit {
            expected: head(),
            intent: other
        }
    )
    .is_err());
    assert_eq!(state, original);
    let mut bad = window("bad", 2, 10_000_000, 100);
    bad.fee_segment = "ShanghaiStarA".into();
    assert!(apply_request(
        &mut state,
        &m,
        &CommandRecord::Evaluate {
            expected: head(),
            parent_id: "one".into(),
            window: bad
        }
    )
    .is_err());
    assert_eq!(state, original);
}

/// Produces only a recorded fixture after the original real admission API.
/// It is not a namespace-issued live window or an approved trading capability.
fn admitted_recorded_star_window(id: &str, second: u32) -> WindowRecord {
    use crate::data_gateway::qualified_trading_facts::AuthorityTradingFactsRecord;
    use crate::data_gateway::{
        AuthorityLifecycle, AuthoritySuspensionCoverage, BatchEvidence, QualifiedListingStatus,
        QualifiedPriceBand, QualifiedSuspensionStatus, QualifiedTradingFacts,
        QualifiedTradingFactsRequest, SecurityBoard,
    };
    use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};
    let instrument = InstrumentId::new(Exchange::Shanghai, "688001", AssetClass::Equity).unwrap();
    let facts = QualifiedTradingFacts::admit(
        QualifiedTradingFactsRequest::new(instrument.clone(), date()),
        AuthorityTradingFactsRecord {
            instrument,
            lifecycle: Some(AuthorityLifecycle {
                listed_on: NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                delisted_on: None,
                covered_through: date(),
            }),
            price_regime: Some(
                QualifiedPriceBand::new(
                    SecurityBoard::Star,
                    false,
                    10_000,
                    7_000_000,
                    30_000_000,
                    date(),
                    date(),
                    "TEST_CODE_STAR_REGIME",
                )
                .unwrap(),
            ),
            suspension: Some(AuthoritySuspensionCoverage::trading(date(), date())),
            evidence: BatchEvidence {
                provider: ProviderId::Custom,
                source: "TEST_CODE_FACTS_SOURCE".into(),
                source_at: Some(at(0).to_rfc3339()),
                observed_at: at(0).to_rfc3339(),
                batch_id: "TEST_CODE_STAR_FACTS_BATCH".into(),
            },
            contract_version: "TEST_CODE_REAL_ADMIT_SYNTHETIC_SOURCE".into(),
            fresh_through: date(),
        },
    )
    .unwrap();
    assert_eq!(
        facts.lifecycle().require().unwrap(),
        &QualifiedListingStatus::Listed
    );
    assert_eq!(
        facts.suspension().require().unwrap(),
        &QualifiedSuspensionStatus::Trading
    );
    let band = facts.price_regime().require().unwrap();
    let mut w = window(id, second, 20_000_000, 100);
    w.instrument_code = facts.request().instrument().code().into();
    w.fee_segment = "ShanghaiStarA".into();
    w.facts_contract = facts.contract_version().into();
    w.facts_batch_id = facts.evidence().unwrap().batch_id.clone();
    w.facts_source = facts.evidence().unwrap().source.clone();
    w.facts_source_at = facts.evidence().unwrap().source_at.clone().unwrap();
    w.facts_observed_at = facts.evidence().unwrap().observed_at.clone();
    w.regime_version = band.version().into();
    w.tick_micro_cny = band.tick_micros();
    w.lower_micro_cny = band.lower_price_micros();
    w.upper_micro_cny = band.upper_price_micros();
    w.validate().unwrap();
    w
}
#[test]
fn paper_v2_execution_recorded_cross_segment_unassigned_valuation_preserves_allocation_and_cash() {
    let mut star = lot("unassigned", 100, 0);
    star.code = "688001".into();
    let g = genesis(10_000_000_000, vec![star]);
    let mut m = manifest(&g, 6_000_000_000, 6_000_000_000);
    m.budget.initial_lots[0].disposition = LotDisposition::UnassignedReadOnly;
    m.budget.initial_lots[0].chain_id = None;
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    let cash = state.cash.clone();
    let allocation = state.lot_assignments.clone();
    let mark = admitted_recorded_star_window("star-mark", 1);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::QualifiedMarks {
            expected: head(),
            windows: vec![mark],
        },
    )
    .unwrap();
    assert_eq!(state.account.marks["688001"].price.micros(), 20_000_000);
    assert_eq!(state.cash, cash);
    assert_eq!(state.lot_assignments, allocation);
    assert_eq!(m.budget.authorized_budget_micro_cny, 6_000_000_000);
    assert_eq!(state.account.fees.micros(), 0);
}
#[test]
fn paper_v2_execution_recorded_cross_segment_trade_denied_after_real_fact_admission() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 6_000_000_000, 6_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    let original = state.clone();
    let mut trade = intent(&m, "star-trade", Side::Buy, 100, 1);
    trade.instrument_code = "688001".into();
    trade.source_window = admitted_recorded_star_window("star-trade-facts", 1);
    trade.limit_micro_cny = 20_000_000;
    trade.fee_price_cap_micro_cny = 20_000_000;
    let error = apply_request(
        &mut state,
        &m,
        &CommandRecord::Submit {
            expected: head(),
            intent: trade,
        },
    )
    .unwrap_err();
    assert!(
        matches!(error,LedgerError::IntegrityFailure(ref why) if why=="recorded admitted board differs from fee scope")
    );
    assert_eq!(state, original);
}

#[test]
fn paper_v2_execution_recorded_day_expiry_releases_only_remaining_after_verified_close() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "expiring", Side::Buy, 300, 1).unwrap();
    evaluate(&mut state, &m, "expiring", "one-fill", 2, 100).unwrap();
    let original = state.clone();
    for when in [
        Utc.with_ymd_and_hms(2026, 9, 24, 6, 59, 59).unwrap(),
        Utc.with_ymd_and_hms(2026, 9, 25, 7, 0, 0).unwrap(),
    ] {
        assert!(apply_request(
            &mut state,
            &m,
            &CommandRecord::Expire {
                expected: head(),
                parent_id: "expiring".into(),
                at: when,
            },
        )
        .is_err());
        assert_eq!(
            state, original,
            "failed expiry cannot release a reservation"
        );
    }
    assert!(matches!(
        apply_request(
            &mut state,
            &m,
            &CommandRecord::Expire {
                expected: head(),
                parent_id: "expiring".into(),
                at: Utc.with_ymd_and_hms(2026, 9, 24, 7, 0, 0).unwrap(),
            },
        )
        .unwrap(),
        Effect::Expired
    ));
    let parent = &state.parents["expiring"];
    assert_eq!(parent.status, ParentStatus::Expired);
    assert_eq!(
        (parent.filled, parent.remaining, parent.cancelled),
        (100, 0, 200)
    );
    assert_eq!(parent.reservation.cash_reserve, 0);
    assert_eq!(
        state.cash, original.cash,
        "expiry does not refund a paid fill or fee"
    );
    assert_eq!(state.fills, original.fills);
    assert_eq!(state.account.lots, original.account.lots);
    let expired = state.clone();
    assert!(apply_request(
        &mut state,
        &m,
        &CommandRecord::Expire {
            expected: head(),
            parent_id: "expiring".into(),
            at: Utc.with_ymd_and_hms(2026, 9, 28, 7, 0, 0).unwrap(),
        },
    )
    .is_err());
    assert_eq!(
        state, expired,
        "new command cannot release an expired parent twice"
    );
}

#[test]
fn paper_v2_execution_recorded_outside_limit_is_no_fill_with_full_reservation_kept() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "waiting", Side::Buy, 300, 1).unwrap();
    let original = state.clone();
    let result = apply_request(
        &mut state,
        &m,
        &CommandRecord::Evaluate {
            expected: head(),
            parent_id: "waiting".into(),
            window: window("above-limit", 2, 11_000_000, 300),
        },
    )
    .unwrap();
    assert_eq!(
        result,
        Effect::ObservedNoFill(fill_model::NoFillReason::OutsideLimit)
    );
    assert_eq!(state.parents, original.parents);
    assert_eq!(state.cash, original.cash);
    assert_eq!(state.account.lots, original.account.lots);
    assert_eq!(state.fills, original.fills);
    assert_eq!(state.account.marks["600001"].price.micros(), 11_000_000);
    assert!(state.used_windows.contains_key("above-limit"));
}

#[test]
fn paper_v2_execution_live_expired_source_is_unavailable_not_a_recorded_no_fill() {
    let w = window("expired", 1, 10_000_000, 0);
    let request = CommandRecord::Evaluate {
        expected: head(),
        parent_id: "waiting".into(),
        window: w.clone(),
    };
    // The recorded observation is valid for replay, yet it cannot be a new
    // live write after its originally supplied source lifetime.
    assert!(w.validate().is_ok());
    assert!(require_live_request(&request, w.fresh_through).is_ok());
    let error =
        require_live_request(&request, w.fresh_through + chrono::Duration::seconds(1)).unwrap_err();
    assert!(
        matches!(error, LedgerError::IntegrityFailure(ref why) if why == "live execution window expired")
    );
}

fn other_code_lot(id: &str, code: &str) -> Lot {
    let mut value = lot(id, 100, 17);
    value.code = code.into();
    value
}
fn window_on_at(id: &str, code: &str, when: DateTime<Utc>) -> WindowRecord {
    let mut value = window(id, 0, 10_000_000, 100);
    value.instrument_code = code.into();
    value.source_at = when;
    value.observed_at = when;
    value.fresh_through = when + chrono::Duration::minutes(1);
    value
}
fn intent_on_at(
    m: &ExecutionManifest,
    id: &str,
    code: &str,
    side: Side,
    when: DateTime<Utc>,
) -> IntentRecord {
    let mut value = intent(m, id, side, 100, 0);
    value.instrument_code = code.into();
    value.approved_at = when;
    value.source_window = window_on_at(&format!("approval-{id}"), code, when);
    value
}

#[test]
fn paper_v2_execution_recorded_genesis_mark_does_not_mint_a_live_allocated_valuation() {
    let g = genesis(20_000_000_000, vec![other_code_lot("old", "600002")]);
    let m = manifest(&g, 10_000_000_000, 11_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    assert!(state.valuation_windows.is_empty());
    let original = state.clone();
    let error = submit(&mut state, &m, "new-buy", Side::Buy, 100, 1).unwrap_err();
    assert!(
        matches!(error, LedgerError::EvidenceUnavailable(ref why) if why == "allocated holding qualified valuation window absent")
    );
    assert_eq!(state, original);
    assert_eq!(state.account.marks, g.marks);
    assert_eq!(encode(&state.account).unwrap(), encode(&g).unwrap());
    assert_eq!(
        m.budget.initial_cash(&g).unwrap().strategy_cash,
        10_000_000_000
    );
}

#[test]
fn paper_v2_execution_recorded_stale_other_valuation_blocks_buy_but_not_cancel_sell_or_marks() {
    let g = genesis(
        20_000_000_000,
        vec![
            other_code_lot("old-a", "600002"),
            other_code_lot("old-b", "600003"),
        ],
    );
    let m = manifest(&g, 10_000_000_000, 12_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    apply_request(
        &mut state,
        &m,
        &CommandRecord::QualifiedMarks {
            expected: head(),
            windows: vec![
                window_on_at("mark-a", "600002", at(1)),
                window_on_at("mark-b", "600003", at(1)),
            ],
        },
    )
    .unwrap();
    submit(&mut state, &m, "waiting", Side::Buy, 100, 10).unwrap();
    let original = state.clone();
    let after_expiry = at(0) + chrono::Duration::seconds(70);
    let error = apply_request(
        &mut state,
        &m,
        &CommandRecord::Submit {
            expected: head(),
            intent: intent_on_at(&m, "stale-buy", "600001", Side::Buy, after_expiry),
        },
    )
    .unwrap_err();
    assert!(
        matches!(error, LedgerError::IntegrityFailure(ref why) if why == "allocated holding qualified valuation window expired or differs")
    );
    assert_eq!(state, original);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::Cancel {
            expected: head(),
            parent_id: "waiting".into(),
            at: after_expiry,
        },
    )
    .unwrap();
    assert_eq!(state.parents["waiting"].reservation.cash_reserve, 0);
    let sell_at = after_expiry + chrono::Duration::seconds(1);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::Submit {
            expected: head(),
            intent: intent_on_at(&m, "reduce-risk", "600002", Side::Sell, sell_at),
        },
    )
    .unwrap();
    assert!(state.valuation_windows["600003"].fresh_through < sell_at);
    assert_eq!(state.parents["reduce-risk"].sell_claims[0].lot_id, "old-a");
    let cash = state.cash.clone();
    let reserve = state.parents["reduce-risk"].reservation.clone();
    let mark_at = sell_at + chrono::Duration::seconds(1);
    apply_request(
        &mut state,
        &m,
        &CommandRecord::QualifiedMarks {
            expected: head(),
            windows: vec![
                window_on_at("fresh-a", "600002", mark_at),
                window_on_at("fresh-b", "600003", mark_at),
            ],
        },
    )
    .unwrap();
    assert_eq!(state.cash, cash);
    assert_eq!(state.parents["reduce-risk"].reservation, reserve);
    assert_eq!(state.valuation_windows["600003"].observed_at, mark_at);
    let mut tampered = state.clone();
    tampered.account.marks.get_mut("600003").unwrap().source = "foreign mark".into();
    assert!(tampered.validate().is_err());
}

#[test]
fn paper_v2_execution_recorded_tail_rechecks_actual_time_and_preserves_exact_retry_boundary() {
    let g = genesis(20_000_000_000, vec![other_code_lot("old", "600002")]);
    let m = manifest(&g, 10_000_000_000, 11_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    apply_request(
        &mut state,
        &m,
        &CommandRecord::QualifiedMarks {
            expected: head(),
            windows: vec![window_on_at("old-mark", "600002", at(1))],
        },
    )
    .unwrap();
    let request = CommandRecord::Submit {
        expected: head(),
        intent: intent(&m, "new-buy", Side::Buy, 100, 10),
    };
    apply_request(&mut state, &m, &request).unwrap();
    // Low-authority binding data exercises the time predicate only. Actual
    // SQL capture/namespace/Global6 lifecycle coverage remains separate.
    let binding = SqlBinding {
        rows: SqlRows {
            manifests: Vec::new(),
            parents: Vec::new(),
            events: Vec::new(),
            heads: vec![HeadRow {
                account_id: m.account_id.clone(),
                version: 3,
                event_hash: "c".repeat(64),
                projection_hash: raw_hash(&encode(&state).unwrap()),
                projection_bytes: encode(&state).unwrap(),
            }],
        },
    };
    assert!(binding
        .require_fresh_request(
            &m.account_id,
            Some(&request),
            at(0) + chrono::Duration::seconds(60)
        )
        .is_ok());
    let later = at(0) + chrono::Duration::seconds(62);
    // The submitted source lasts through second70. The other original
    // holding's last genuine window expired at second61, so it blocks here.
    assert!(require_live_request(&request, later).is_ok());
    assert!(binding
        .require_fresh_request(&m.account_id, Some(&request), later)
        .is_err());
    assert!(binding
        .require_fresh_request(&m.account_id, None, later)
        .is_ok());
}

/// Actual old SQLite seed/mark/cutover fixture. It does not create or qualify
/// CatalogV6. The later Global factory must perform that separate extension.
struct OriginalV5Fixture {
    directory: tempfile::TempDir,
    db: DatabaseManager,
    old_binding: crate::trading::paper_ledger::AccountBinding,
    cutover: crate::trading::paper_book_v2::TestCutoverRequest,
    old_view: crate::trading::paper_ledger::PaperView,
    old_payloads: Vec<String>,
    old_projection_bytes: Vec<u8>,
}
#[derive(QueryableByName)]
struct OriginalText {
    #[diesel(sql_type = Text)]
    value: String,
}
fn original_payloads(db: &DatabaseManager, account: &str) -> Vec<String> {
    diesel::sql_query(
        "SELECT payload AS value FROM paper_ledger_event WHERE account_id=? ORDER BY seq",
    )
    .bind::<Text, _>(account)
    .load::<OriginalText>(&mut db.get_conn().unwrap())
    .unwrap()
    .into_iter()
    .map(|row| row.value)
    .collect()
}
fn original_v5_fixture() -> OriginalV5Fixture {
    original_v5_fixture_with_fee(fee())
}
fn original_v5_fixture_with_fee(policy: AShareFeePolicyV2) -> OriginalV5Fixture {
    use crate::trading::paper_book_v2::{
        cutover_for_isolated_test, TestCutoverFault, TestCutoverRequest,
    };
    use crate::trading::paper_ledger::{
        PaperCommand, PaperLedger, RiskPolicyV1, SeedLot, SeedManifest, ValuationBatch,
    };
    use diesel::connection::SimpleConnection;
    let directory = tempfile::Builder::new()
        .prefix("TEST_CODE_PAPER_EXECUTION_")
        .tempdir()
        .unwrap();
    let db = DatabaseManager::open_frozen_catalog_for_isolated_test(
        directory.path().join("TEST_CODE_paper_v5.db"),
    )
    .unwrap();
    {
        let mut conn = db.get_conn().unwrap();
        crate::database::paper_ledger_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
    }
    let seed = SeedManifest {
        account_id: "TEST_CODE_ORIGINAL_ACCOUNT".into(),
        epoch_id: "TEST_CODE_ORIGINAL_EPOCH".into(),
        command_id: "TEST_CODE_ORIGINAL_SEED".into(),
        cutover_at: at(0),
        account_effective_at: at(0),
        positions_effective_at: at(0),
        source_reference: "TEST_CODE_EXPLICIT_SYNTHETIC_GENESIS_SOURCE".into(),
        source_hash: "a".repeat(64),
        approved_by: "TEST_CODE_EXPLICIT_SYNTHETIC_NOT_USER_APPROVAL".into(),
        cash: Money::from_micros(20_000_000_000),
        original_total: Money::from_micros(24_000_000_000),
        excluded_residual: Some(Money::from_micros(1_000_000_000)),
        lots: vec![SeedLot {
            code: "600001".into(),
            name: "synthetic inherited holding".into(),
            quantity: 300,
            reported_cost: Some(Money::from_micros(8_000_000)),
            sellable_from: Some(date()),
            sellability_evidence: Some("TEST_CODE_EXPLICIT_EXISTING_SELLABILITY".into()),
        }],
        marks: vec![Mark {
            code: "600001".into(),
            price: Money::from_micros(10_000_000),
            observed_at: at(0),
            source: "TEST_CODE_ORIGINAL_MARK".into(),
        }],
        policy: RiskPolicyV1 {
            max_position_bps: 10_000,
            cash_floor_bps: 0,
            max_slippage_bps: 200,
        },
    };
    let old_binding = seed.binding().unwrap();
    let clock = || at(1);
    let ledger = PaperLedger::open(&db, &clock);
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let seeded = ledger.read(&old_binding).unwrap();
    ledger
        .apply(PaperCommand::Mark(ValuationBatch {
            binding: old_binding.clone(),
            command_id: "TEST_CODE_ORIGINAL_SECOND_FACT".into(),
            expected_version: seeded.version,
            inventory_fingerprint: seeded.inventory_fingerprint().unwrap(),
            as_of: at(1),
            closing: false,
            marks: vec![Mark {
                code: "600001".into(),
                price: Money::from_micros(11_000_000),
                observed_at: at(1),
                source: "TEST_CODE_SECOND_ORIGINAL_MARK".into(),
            }],
        }))
        .unwrap();
    let old_view = ledger.read(&old_binding).unwrap();
    let old_payloads = original_payloads(&db, &old_binding.account_id);
    {
        let mut conn = db.get_conn().unwrap();
        crate::database::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute("PRAGMA user_version=3").unwrap();
        crate::database::paper_book_owner_schema_v1::install_catalog_v4_for_isolated_test(
            &mut conn, &policy,
        )
        .unwrap();
        crate::database::paper_book_owner_schema_v2::install_catalog_v5_for_isolated_test(
            &mut conn,
        )
        .unwrap();
    }
    let snapshot = crate::trading::paper_ledger::verified_v1_snapshot_on(
        &mut db.get_conn().unwrap(),
        &old_binding,
    )
    .unwrap();
    let old_projection_bytes = snapshot.projection_bytes.into_bytes();
    let cutover = TestCutoverRequest {
        old_binding: old_binding.clone(),
        new_epoch_id: "TEST_CODE_ACTUAL_V2_EPOCH".into(),
        cutover_id: "TEST_CODE_ACTUAL_V2_CUTOVER".into(),
        command_id: "TEST_CODE_ACTUAL_V2_GENESIS".into(),
        expected_v1_version: snapshot.version,
        expected_v1_head_hash: snapshot.event_hash,
        expected_v1_projection_hash: snapshot.projection_hash,
        reviewed_fee_policy: policy,
    };
    cutover_for_isolated_test(&db, &cutover, TestCutoverFault::None).unwrap();
    OriginalV5Fixture {
        directory,
        db,
        old_binding,
        cutover,
        old_view,
        old_payloads,
        old_projection_bytes,
    }
}

#[test]
fn paper_v2_execution_actual_original_v5_fixture_preserves_full_genesis_and_closes_old_writer() {
    use crate::trading::paper_book_v2::{cutover_for_isolated_test, read_v2_on, TestCutoverFault};
    use crate::trading::paper_ledger::{PaperCommand, PaperLedger, ValuationBatch};
    let f = original_v5_fixture();
    assert!(f.db.has_isolated_p05_consumer_origin());
    assert!(f
        .directory
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("TEST_CODE_"));
    let actual = read_v2_on(&f.db, &f.old_binding.account_id).unwrap();
    assert_eq!(actual.projection_bytes, f.old_projection_bytes);
    assert_eq!(actual.v1_head_version, 2);
    assert_eq!(actual.v1_head_hash, f.old_view.event_hash);
    assert_eq!(actual.cutover_id, f.cutover.cutover_id);
    assert_eq!(actual.fee_policy_instance_id, fee().instance_id());
    let original: Projection = decode(&actual.projection_bytes).unwrap();
    assert_eq!(original.cash.micros(), 20_000_000_000);
    assert_eq!(original.lots[0].quantity, 300);
    assert_eq!(original.lots[0].basis_price.micros(), 10_000_000);
    assert_eq!(original.lots[0].reported_cost.unwrap().micros(), 8_000_000);
    assert_eq!(original.marks["600001"].price.micros(), 11_000_000);
    assert_eq!(original.seed_equity.micros(), 23_000_000_000);
    // The verified original financial snapshot's mark is11, not the
    // inherited basis10/reported-cost8. C0 must use this exact snapshot.
    assert!(policy(&original, 10_000_000_000, 13_000_000_000)
        .initial_cash(&original)
        .is_err());
    let capital = policy(&original, 9_700_000_000, 13_000_000_000)
        .initial_cash(&original)
        .unwrap();
    assert_eq!(capital.account_cash, 20_000_000_000);
    assert_eq!(capital.strategy_cash, 9_700_000_000);
    assert_eq!(capital.unassigned_cash, 10_300_000_000);
    assert_eq!(encode(&original).unwrap(), actual.projection_bytes);
    assert_eq!(
        original_payloads(&f.db, &f.old_binding.account_id),
        f.old_payloads
    );
    let clock = || at(2);
    let ledger = PaperLedger::open(&f.db, &clock);
    assert_eq!(ledger.read(&f.old_binding).unwrap(), f.old_view);
    let attempt = PaperCommand::Mark(ValuationBatch {
        binding: f.old_binding.clone(),
        command_id: "TEST_CODE_INACTIVE_V1_MARK".into(),
        expected_version: f.old_view.version,
        inventory_fingerprint: f.old_view.inventory_fingerprint().unwrap(),
        as_of: at(2),
        closing: false,
        marks: vec![Mark {
            code: "600001".into(),
            price: Money::from_micros(12_000_000),
            observed_at: at(2),
            source: "TEST_CODE_NEVER_WRITTEN".into(),
        }],
    });
    assert!(matches!(
        ledger.apply(attempt),
        Err(LedgerError::InactiveEpoch)
    ));
    assert_eq!(
        original_payloads(&f.db, &f.old_binding.account_id),
        f.old_payloads
    );
    assert!(
        cutover_for_isolated_test(&f.db, &f.cutover, TestCutoverFault::None)
            .unwrap()
            .already_applied
    );
    let database_path = f.directory.path().join("TEST_CODE_paper_v5.db");
    // A cold reopen must release every old SQLite connection and its
    // source-local SHM proof before a new descriptor source is established.
    // All expected original financial data stays owned by the other fields.
    drop(ledger);
    drop(f.db);
    assert!(database_path.metadata().unwrap().len() > 0);
    let other = DatabaseManager::open_frozen_catalog_for_isolated_test(database_path).unwrap();
    assert_eq!(
        read_v2_on(&other, &f.old_binding.account_id).unwrap(),
        actual
    );
    assert_eq!(
        PaperLedger::open(&other, &clock)
            .read(&f.old_binding)
            .unwrap(),
        f.old_view
    );
    assert_eq!(
        original_payloads(&other, &f.old_binding.account_id),
        f.old_payloads
    );
    // Real Test origin and the genuine old owner do not by themselves issue
    // a V6 writer/catalog/window. The current closed boundary remains closed.
    let error = binding_for_isolated_test(&other, &f.old_binding.account_id).unwrap_err();
    assert!(
        matches!(error, LedgerError::EvidenceUnavailable(ref why) if why.contains("Catalog6RequalificationRequired"))
    );
}

/// Genuine old replay/sole-owner cutover followed by the Global-owned V6
/// migration. Source/approval amounts remain explicit synthetic Test facts.
struct ActualV6Fixture {
    original: OriginalV5Fixture,
    manifest: ExecutionManifest,
}
fn actual_v6_fixture() -> ActualV6Fixture {
    actual_v6_fixture_with_fee(fee())
}
fn actual_v6_fixture_with_fee(fee_policy: AShareFeePolicyV2) -> ActualV6Fixture {
    use crate::database::global_schema_v1::paper_v6::{
        migrate_catalog6_for_isolated_test, prepare_final_selection_for_isolated_v5_test,
    };
    let original = original_v5_fixture_with_fee(fee_policy.clone());
    prepare_final_selection_for_isolated_v5_test(&original.db).unwrap();
    migrate_catalog6_for_isolated_test(&original.db).unwrap();
    let genesis = {
        let mut session = paper_catalog6_session(&original.db).unwrap();
        session
            .with_readonly_catalog6(
                |conn, _proof| {
                    crate::trading::paper_book_v2::read_verified_genesis_body_on(
                        conn,
                        &original.old_binding.account_id,
                    )
                },
                |_, _, _| Ok(()),
            )
            .unwrap()
    };
    let projection: Projection = decode(&genesis.projection_bytes).unwrap();
    let mut manifest = manifest(&projection, 9_700_000_000, 13_000_000_000);
    manifest.account_id = genesis.account_id;
    manifest.epoch_id = genesis.epoch_id;
    manifest.cutover_id = genesis.cutover_id;
    manifest.genesis_event_hash = genesis.event_hash;
    manifest.genesis_projection_hash = genesis.projection_hash;
    manifest.fee_descriptor = fee_policy.canonical_bytes();
    manifest.fee_policy_instance_id = fee_policy.instance_id();
    open_for_isolated_test(
        &original.db,
        &manifest.account_id,
        "TEST_CODE_OPEN_EXECUTION",
        manifest.clone(),
    )
    .unwrap();
    ActualV6Fixture { original, manifest }
}
fn admitted_main_facts_for_code(code: &str) -> crate::data_gateway::QualifiedTradingFacts {
    use crate::data_gateway::qualified_trading_facts::AuthorityTradingFactsRecord;
    use crate::data_gateway::{
        AuthorityLifecycle, AuthoritySuspensionCoverage, BatchEvidence, QualifiedPriceBand,
        QualifiedTradingFacts, QualifiedTradingFactsRequest, SecurityBoard,
    };
    use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};
    let instrument = InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap();
    QualifiedTradingFacts::admit(
        QualifiedTradingFactsRequest::new(instrument.clone(), date()),
        AuthorityTradingFactsRecord {
            instrument,
            lifecycle: Some(AuthorityLifecycle {
                listed_on: NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                delisted_on: None,
                covered_through: date(),
            }),
            price_regime: Some(
                QualifiedPriceBand::new(
                    SecurityBoard::Main,
                    false,
                    10_000,
                    7_000_000,
                    30_000_000,
                    date(),
                    date(),
                    "TEST_CODE_MAIN_REGIME",
                )
                .unwrap(),
            ),
            suspension: Some(AuthoritySuspensionCoverage::trading(date(), date())),
            evidence: BatchEvidence {
                provider: ProviderId::Custom,
                source: "TEST_CODE_SYNTHETIC_SOURCE_REAL_ADMISSION".into(),
                source_at: Some(at(0).to_rfc3339()),
                observed_at: at(0).to_rfc3339(),
                batch_id: "TEST_CODE_ACTUAL_MAIN_BATCH".into(),
            },
            contract_version: "TEST_CODE_REAL_ADMIT_NOT_PRODUCTION".into(),
            fresh_through: date(),
        },
    )
    .unwrap()
}
fn actual_window(
    db: &DatabaseManager,
    account: &str,
    id: &str,
    second: u32,
    price: i64,
    quantity: u32,
) -> QualifiedPaperExecutionWindowV1 {
    actual_window_for_code(db, account, "600001", id, second, price, quantity)
}
fn actual_window_for_code(
    db: &DatabaseManager,
    account: &str,
    code: &str,
    id: &str,
    second: u32,
    price: i64,
    quantity: u32,
) -> QualifiedPaperExecutionWindowV1 {
    use crate::decision::approved_paper_intent_v1::{
        issue_window_for_isolated_test, TestWindowObservation,
    };
    let actual = binding_for_isolated_test(db, account).unwrap();
    issue_window_for_isolated_test(
        &actual,
        &admitted_main_facts_for_code(code),
        TestWindowObservation {
            observation_id: id.into(),
            source_reference: "TEST_CODE_EXPLICIT_MODELED_WINDOW".into(),
            source_at: at(second),
            observed_at: at(second),
            fresh_through: at(second) + chrono::Duration::minutes(1),
            price_micro_cny: price,
            modeled_available_quantity: quantity,
        },
    )
    .unwrap()
}
fn actual_approval(
    db: &DatabaseManager,
    account: &str,
    parent: &str,
    side: Side,
    quantity: u32,
    second: u32,
) -> ApprovedPaperIntentV1 {
    actual_approval_for_code(db, account, "600001", parent, side, quantity, second)
}
fn actual_approval_for_code(
    db: &DatabaseManager,
    account: &str,
    code: &str,
    parent: &str,
    side: Side,
    quantity: u32,
    second: u32,
) -> ApprovedPaperIntentV1 {
    use crate::decision::approved_paper_intent_v1::{
        issue_intent_for_isolated_test, TestIntentRequest,
    };
    let actual = binding_for_isolated_test(db, account).unwrap();
    let window = actual_window_for_code(
        db,
        account,
        code,
        &format!("TEST_CODE_APPROVAL_{parent}"),
        second,
        10_000_000,
        100,
    );
    issue_intent_for_isolated_test(
        &actual,
        &admitted_main_facts_for_code(code),
        window,
        TestIntentRequest {
            parent_id: parent.into(),
            investment_decision_id: format!("TEST_CODE_DECISION_{parent}"),
            chain_id: "TEST_CODE_CHAIN".into(),
            name: "TEST_CODE_SYNTHETIC_NAME".into(),
            side,
            quantity,
            limit_micro_cny: 10_000_000,
            fee_price_cap_micro_cny: 10_000_000,
        },
    )
    .unwrap()
}
fn actual_view(f: &ActualV6Fixture) -> RecordedExecutionView {
    read_for_isolated_test(&f.original.db, &f.manifest.account_id).unwrap()
}
fn actual_rows(db: &DatabaseManager) -> SqlRows {
    read_checked_on_actual_manager(db, |conn, _| sql_rows(conn), |_, _| Ok(())).unwrap()
}
fn actual_marks(f: &ActualV6Fixture, second: u32) -> PaperV2Receipt {
    let window = actual_window(
        &f.original.db,
        &f.manifest.account_id,
        "TEST_CODE_INITIAL_FRESH_MARK",
        second,
        10_000_000,
        100,
    );
    apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::QualifiedMarks {
            command_id: "TEST_CODE_INITIAL_MARK_COMMAND".into(),
            expected: actual_view(f).head,
            windows: vec![window],
        },
        at(second),
    )
    .unwrap()
}
fn actual_submit(
    f: &ActualV6Fixture,
    parent: &str,
    side: Side,
    quantity: u32,
    second: u32,
) -> PaperV2Receipt {
    let approved = actual_approval(
        &f.original.db,
        &f.manifest.account_id,
        parent,
        side,
        quantity,
        second,
    );
    apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Submit {
            command_id: format!("TEST_CODE_SUBMIT_{parent}"),
            expected: actual_view(f).head,
            approved,
        },
        at(second),
    )
    .unwrap()
}
fn actual_evaluate(
    f: &ActualV6Fixture,
    parent: &str,
    id: &str,
    second: u32,
    quantity: u32,
) -> PaperV2Receipt {
    let window = actual_window(
        &f.original.db,
        &f.manifest.account_id,
        id,
        second,
        10_000_000,
        quantity,
    );
    apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Evaluate {
            command_id: format!("TEST_CODE_EVALUATE_{id}"),
            expected: actual_view(f).head,
            parent_id: parent.into(),
            window,
        },
        at(second),
    )
    .unwrap()
}

#[test]
fn paper_v2_execution_actual_global_open_reopen_readonly_original_and_exact_retry() {
    let f = actual_v6_fixture();
    let first = actual_view(&f);
    assert_eq!(first.head.version, 1);
    assert_eq!(
        first.projection.account,
        decode::<Projection>(&f.original.old_projection_bytes).unwrap()
    );
    assert_eq!(first.projection.cash.strategy_cash, 9_700_000_000);
    assert_eq!(first.projection.cash.unassigned_cash, 10_300_000_000);
    let before = actual_rows(&f.original.db);
    assert!(
        open_for_isolated_test(
            &f.original.db,
            &f.manifest.account_id,
            "TEST_CODE_OPEN_EXECUTION",
            f.manifest.clone()
        )
        .unwrap()
        .replayed
    );
    assert_eq!(actual_rows(&f.original.db), before);
    let database_path = f.original.directory.path().join("TEST_CODE_paper_v5.db");
    // Keep the real immutable financial snapshots, manifest and directory,
    // while closing the old manager before issuing a new source-local proof.
    drop(f.original.db);
    assert!(database_path.metadata().unwrap().len() > 0);
    let reopened = DatabaseManager::open_frozen_catalog_for_isolated_test(database_path).unwrap();
    assert_eq!(
        read_for_isolated_test(&reopened, &f.manifest.account_id).unwrap(),
        first
    );
    let original = read_checked_on_actual_manager(
        &reopened,
        |conn, _| {
            crate::trading::paper_ledger::read_verified_original_v1_body_on(
                conn,
                &f.original.old_binding,
            )
        },
        |conn, expected| {
            require(
                crate::trading::paper_ledger::read_verified_original_v1_body_on(
                    conn,
                    &f.original.old_binding,
                )? == *expected,
                "original changed",
            )
        },
    )
    .unwrap();
    assert_eq!(original, f.original.old_view);
    assert_eq!(
        original_payloads(&reopened, &f.manifest.account_id),
        f.original.old_payloads
    );
    let restored = recover_command_for_isolated_test(
        &reopened,
        &f.manifest.account_id,
        "TEST_CODE_OPEN_EXECUTION",
    )
    .unwrap()
    .unwrap();
    assert_eq!(restored.receipt().head, first.head);
    assert_eq!(restored.observed_head(), &first.head);
    assert!(restored.receipt().replayed);
    assert!(
        matches!(restored.request(), CommandRecord::Open { manifest } if manifest == &f.manifest)
    );
    assert_eq!(actual_rows(&reopened), before);
}

#[test]
fn paper_v2_execution_actual_global_partial_no_fill_cancel_and_original_receipt() {
    let f = actual_v6_fixture();
    actual_marks(&f, 2);
    let expected = actual_view(&f).head;
    let submitted = actual_submit(&f, "TEST_CODE_BUY", Side::Buy, 300, 3);
    let working = actual_view(&f);
    assert_eq!(
        working.projection.parents["TEST_CODE_BUY"]
            .reservation
            .cash_reserve,
        3_015_000_000
    );
    let empty = actual_evaluate(&f, "TEST_CODE_BUY", "TEST_CODE_NO_FILL", 4, 99);
    let no_fill = actual_view(&f);
    assert_eq!(
        no_fill.projection.parents["TEST_CODE_BUY"].status,
        ParentStatus::Working
    );
    assert_eq!(no_fill.projection.cash, working.projection.cash);
    assert_eq!(
        no_fill.projection.parents["TEST_CODE_BUY"].reservation,
        working.projection.parents["TEST_CODE_BUY"].reservation
    );
    assert!(no_fill.projection.fills.is_empty());
    assert_eq!(empty.head.version, submitted.head.version + 1);
    actual_evaluate(&f, "TEST_CODE_BUY", "TEST_CODE_PARTIAL", 5, 100);
    let partial = actual_view(&f);
    assert_eq!(
        partial.projection.parents["TEST_CODE_BUY"].status,
        ParentStatus::PartiallyFilled
    );
    assert_eq!(partial.projection.parents["TEST_CODE_BUY"].filled, 100);
    assert_eq!(partial.projection.parents["TEST_CODE_BUY"].remaining, 200);
    assert_eq!(
        partial.projection.parents["TEST_CODE_BUY"]
            .reservation
            .cash_reserve,
        2_010_000_000
    );
    assert_eq!(partial.projection.cash.strategy_cash, 8_695_000_000);
    assert_eq!(partial.projection.cash.account_cash, 18_995_000_000);
    assert_eq!(partial.projection.cash.unassigned_cash, 10_300_000_000);
    assert_eq!(
        partial.projection.fills[0].model.total_fee_micro_cny,
        5_000_000
    );
    assert_eq!(
        partial.projection.fills[0].model.sellable_from,
        NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
    );
    let original_submit = recover_command_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        "TEST_CODE_SUBMIT_TEST_CODE_BUY",
    )
    .unwrap()
    .unwrap();
    assert_eq!(original_submit.receipt().head, submitted.head);
    assert!(
        matches!(original_submit.request(), CommandRecord::Submit { expected: original, .. } if original == &expected)
    );
    let cancelled = apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Cancel {
            command_id: "TEST_CODE_CANCEL".into(),
            expected: partial.head.clone(),
            parent_id: "TEST_CODE_BUY".into(),
        },
        at(6),
    )
    .unwrap();
    let after = actual_view(&f);
    assert_eq!(
        after.projection.parents["TEST_CODE_BUY"].status,
        ParentStatus::Cancelled
    );
    assert_eq!(after.projection.parents["TEST_CODE_BUY"].cancelled, 200);
    assert_eq!(
        after.projection.parents["TEST_CODE_BUY"]
            .reservation
            .cash_reserve,
        0
    );
    assert_eq!(after.projection.cash, partial.projection.cash);
    let before = actual_rows(&f.original.db);
    let repeated = apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Cancel {
            command_id: "TEST_CODE_CANCEL".into(),
            expected: partial.head,
            parent_id: "TEST_CODE_BUY".into(),
        },
        at(0) + chrono::Duration::minutes(10),
    )
    .unwrap();
    assert!(repeated.replayed);
    assert_eq!(repeated.head, cancelled.head);
    assert_eq!(actual_rows(&f.original.db), before);
}

#[test]
fn paper_v2_execution_actual_global_three_minimum_fees_fifo_and_t1_claims() {
    let f = actual_v6_fixture();
    actual_marks(&f, 2);
    actual_submit(&f, "TEST_CODE_BUY_THREE", Side::Buy, 300, 3);
    for (second, id) in [
        (4, "TEST_CODE_PART1"),
        (5, "TEST_CODE_PART2"),
        (6, "TEST_CODE_PART3"),
    ] {
        actual_evaluate(&f, "TEST_CODE_BUY_THREE", id, second, 100);
    }
    let bought = actual_view(&f);
    assert_eq!(
        bought.projection.parents["TEST_CODE_BUY_THREE"].status,
        ParentStatus::Filled
    );
    assert_eq!(bought.projection.cash.strategy_cash, 6_685_000_000);
    assert_eq!(bought.projection.account.fees.micros(), 15_000_000);
    assert_eq!(bought.projection.fills.len(), 3);
    assert!(bought
        .projection
        .fills
        .iter()
        .all(|fill| fill.model.total_fee_micro_cny == 5_000_000));
    let before = actual_rows(&f.original.db);
    let approved = actual_approval(
        &f.original.db,
        &f.manifest.account_id,
        "TEST_CODE_SELL_TOO_MUCH",
        Side::Sell,
        400,
        7,
    );
    assert!(apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Submit {
            command_id: "TEST_CODE_T1_REJECT".into(),
            expected: bought.head,
            approved,
        },
        at(7)
    )
    .is_err());
    assert_eq!(actual_rows(&f.original.db), before);
    actual_submit(&f, "TEST_CODE_SELL_OLD", Side::Sell, 200, 7);
    let claims = actual_view(&f).projection.parents["TEST_CODE_SELL_OLD"]
        .sell_claims
        .clone();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].lot_id, f.manifest.budget.initial_lots[0].lot_id);
    actual_evaluate(&f, "TEST_CODE_SELL_OLD", "TEST_CODE_OLD_FILL", 8, 100);
    let sold = actual_view(&f);
    assert_eq!(
        sold.projection.parents["TEST_CODE_SELL_OLD"].sell_claims[0].quantity,
        100
    );
    assert_eq!(
        sold.projection
            .fills
            .last()
            .unwrap()
            .model
            .stamp_tax_micro_cny,
        500_000
    );
    assert_eq!(sold.projection.cash.unassigned_cash, 10_300_000_000);
    assert_eq!(
        original_payloads(&f.original.db, &f.manifest.account_id),
        f.original.old_payloads
    );
}

#[test]
fn paper_v2_execution_actual_global_foreign_namespace_capability_rejects_before_append() {
    let f = actual_v6_fixture();
    let other = actual_v6_fixture();
    actual_marks(&f, 2);
    let approved = actual_approval(
        &other.original.db,
        &other.manifest.account_id,
        "TEST_CODE_FOREIGN",
        Side::Buy,
        100,
        3,
    );
    let before = actual_rows(&f.original.db);
    assert!(
        matches!(apply_for_isolated_test(&f.original.db, &f.manifest.account_id, PaperV2Command::Submit {
        command_id: "TEST_CODE_FOREIGN_SUBMIT".into(), expected: actual_view(&f).head, approved,
    }, at(3)), Err(LedgerError::IntegrityFailure(ref why)) if why == "actual intent owner/namespace differs")
    );
    assert_eq!(actual_rows(&f.original.db), before);
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
}

#[test]
fn paper_v2_execution_actual_global_new_buy_requires_fresh_holdings_but_exact_retry_does_not() {
    let f = actual_v6_fixture();
    // The original V1 allocated holding is600001. A genuine approved600002
    // source window may refresh600002, but cannot manufacture a valuation
    // window for600001. Its original genesis mark remains a financial fact.
    assert!(actual_view(&f)
        .projection
        .account
        .lots
        .iter()
        .all(|lot| lot.code == "600001"));
    assert!(actual_view(&f)
        .projection
        .lot_assignments
        .values()
        .all(|chain| chain.as_deref() == Some("TEST_CODE_CHAIN")));
    let before = actual_rows(&f.original.db);
    let approved = actual_approval_for_code(
        &f.original.db,
        &f.manifest.account_id,
        "600002",
        "TEST_CODE_NO_FRESH",
        Side::Buy,
        100,
        2,
    );
    assert!(matches!(apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Submit {
            command_id: "TEST_CODE_NO_FRESH_SUBMIT".into(),
            expected: actual_view(&f).head,
            approved,
        },
        at(2)
    ), Err(LedgerError::EvidenceUnavailable(ref why)) if why == "allocated holding qualified valuation window absent"));
    assert_eq!(actual_rows(&f.original.db), before);
    actual_marks(&f, 2);
    let expected = actual_view(&f).head;
    let approved = actual_approval_for_code(
        &f.original.db,
        &f.manifest.account_id,
        "600002",
        "TEST_CODE_EXACT",
        Side::Buy,
        100,
        3,
    );
    let original = apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Submit {
            command_id: "TEST_CODE_SUBMIT_TEST_CODE_EXACT".into(),
            expected: expected.clone(),
            approved,
        },
        at(3),
    )
    .unwrap();
    assert_eq!(
        actual_view(&f).projection.parents["TEST_CODE_EXACT"]
            .intent
            .instrument_code,
        "600002"
    );
    assert_eq!(
        actual_view(&f).projection.valuation_windows["600001"].observed_at,
        at(2)
    );
    let before = actual_rows(&f.original.db);
    let approved = actual_approval_for_code(
        &f.original.db,
        &f.manifest.account_id,
        "600002",
        "TEST_CODE_EXACT",
        Side::Buy,
        100,
        3,
    );
    let repeated = apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Submit {
            command_id: "TEST_CODE_SUBMIT_TEST_CODE_EXACT".into(),
            expected,
            approved,
        },
        at(0) + chrono::Duration::minutes(10),
    )
    .unwrap();
    assert!(repeated.replayed);
    assert_eq!(repeated.head, original.head);
    assert_eq!(actual_rows(&f.original.db), before);
    let approved = actual_approval_for_code(
        &f.original.db,
        &f.manifest.account_id,
        "600002",
        "TEST_CODE_TOO_LATE",
        Side::Buy,
        100,
        4,
    );
    assert!(matches!(apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Submit {
            command_id: "TEST_CODE_STALE_HOLDING_SUBMIT".into(),
            expected: actual_view(&f).head,
            approved,
        },
        at(0) + chrono::Duration::seconds(63)
    ), Err(LedgerError::IntegrityFailure(ref why)) if why == "allocated holding qualified valuation window expired or differs"));
    assert_eq!(actual_rows(&f.original.db), before);

    // The same-instrument source window legitimately refreshes the original
    // holding in Submit before new-buy admission. No separate marks required.
    let same = actual_v6_fixture();
    let before = actual_view(&same);
    assert!(before.projection.valuation_windows.is_empty());
    let receipt = actual_submit(&same, "TEST_CODE_SAME_INSTRUMENT", Side::Buy, 100, 2);
    let after = actual_view(&same);
    assert_eq!(receipt.head.version, before.head.version + 1);
    assert_eq!(
        after.projection.valuation_windows["600001"].observed_at,
        at(2)
    );
    assert_eq!(
        after.projection.account.marks["600001"],
        mark_from_window(&after.projection.valuation_windows["600001"])
    );
    assert_eq!(
        after.projection.parents["TEST_CODE_SAME_INSTRUMENT"]
            .intent
            .instrument_code,
        "600001"
    );
    assert_eq!(
        after.projection.parents["TEST_CODE_SAME_INSTRUMENT"].status,
        ParentStatus::Working
    );
    assert_eq!(
        after.projection.parents["TEST_CODE_SAME_INSTRUMENT"]
            .reservation
            .cash_reserve,
        1_005_000_000
    );
    assert_eq!(
        after.projection.account.lots,
        before.projection.account.lots
    );
    assert_eq!(
        after.projection.lot_assignments,
        before.projection.lot_assignments
    );
    assert_eq!(after.projection.cash, before.projection.cash);
    assert_eq!(
        after.projection.account.fees,
        before.projection.account.fees
    );
    assert_eq!(
        original_payloads(&same.original.db, &same.manifest.account_id),
        same.original.old_payloads
    );
}

/// Legal additional event in the very same writer TX, after the candidate
/// binding was captured. This keeps whole-history replay valid and specifically
/// tests the mandatory exact binding, rather than corrupting a DDL/hash cell.
fn append_hook_cancel(conn: &mut SqliteConnection, account: &str, parent: &str, id: &str) {
    let view = read_views_body_on(conn).unwrap().remove(account).unwrap();
    append_on(
        conn,
        account,
        id,
        CommandRecord::Cancel {
            expected: view.head,
            parent_id: parent.into(),
            at: at(7),
        },
        at(7),
    )
    .unwrap();
    verify_rows_on(conn).unwrap();
}
#[test]
fn paper_v2_execution_actual_global_all_paper_sql_hooks_rollback_extra_legal_event() {
    for phase in [TestPhase::AfterSql, TestPhase::LastSqlBeforeCommit] {
        for exact_retry in [false, true] {
            let f = actual_v6_fixture();
            actual_marks(&f, 2);
            let expected = actual_view(&f).head;
            let submitted = actual_submit(&f, "TEST_CODE_HOOK_BUY", Side::Buy, 100, 3);
            let approved = actual_approval(
                &f.original.db,
                &f.manifest.account_id,
                "TEST_CODE_HOOK_BUY",
                Side::Buy,
                100,
                3,
            );
            let before = actual_rows(&f.original.db);
            let account = f.manifest.account_id.clone();
            let hits = std::rc::Rc::new(std::cell::Cell::new(0));
            let hit = hits.clone();
            let guard = install_test_hook(phase, move |conn| {
                hit.set(hit.get() + 1);
                append_hook_cancel(
                    conn,
                    &account,
                    "TEST_CODE_HOOK_BUY",
                    "TEST_CODE_EXTRA_AFTER_SQL",
                );
            });
            let command = if exact_retry {
                PaperV2Command::Submit {
                    command_id: "TEST_CODE_SUBMIT_TEST_CODE_HOOK_BUY".into(),
                    expected,
                    approved,
                }
            } else {
                let window = actual_window(
                    &f.original.db,
                    &f.manifest.account_id,
                    "TEST_CODE_CANDIDATE_NO_FILL",
                    4,
                    10_000_000,
                    0,
                );
                PaperV2Command::Evaluate {
                    command_id: "TEST_CODE_CANDIDATE_NO_FILL_COMMAND".into(),
                    expected: submitted.head,
                    parent_id: "TEST_CODE_HOOK_BUY".into(),
                    window,
                }
            };
            assert!(
                matches!(apply_for_isolated_test(&f.original.db, &f.manifest.account_id, command, at(4)), Err(LedgerError::IntegrityFailure(ref why)) if why == "execution SQL binding changed after operation")
            );
            drop(guard);
            assert_eq!(hits.get(), 1);
            assert_eq!(actual_rows(&f.original.db), before);
            assert_eq!(
                actual_view(&f).projection.parents["TEST_CODE_HOOK_BUY"].status,
                ParentStatus::Working
            );
            assert!(recover_command_for_isolated_test(
                &f.original.db,
                &f.manifest.account_id,
                "TEST_CODE_EXTRA_AFTER_SQL"
            )
            .unwrap()
            .is_none());
        }
    }
}

const PAPER_CHILD_PATH: &str = "TEST_CODE_PAPER_GLOBAL_CHILD_PATH";
const PAPER_CHILD_ACTION: &str = "TEST_CODE_PAPER_GLOBAL_CHILD_ACTION";
const PAPER_CHILD_EXPECTED: &str = "TEST_CODE_PAPER_GLOBAL_CHILD_EXPECTED";
const PAPER_CHILD_HEAD_HASH: &str = "TEST_CODE_PAPER_GLOBAL_CHILD_HEAD_HASH";
const PAPER_CHILD_COMMAND: &str = "TEST_CODE_PAPER_GLOBAL_CHILD_COMMAND";
fn run_actual_child(
    f: &ActualV6Fixture,
    action: &str,
    command: &str,
    expected: Option<&HeadIdentity>,
) {
    let name = "trading::paper_book_v2_execution::tests::TEST_CODE_paper_v2_execution_global_child";
    let mut process = std::process::Command::new(std::env::current_exe().unwrap());
    process
        .args(["--ignored", "--exact", &name, "--nocapture"])
        .env(
            PAPER_CHILD_PATH,
            f.original.directory.path().join("TEST_CODE_paper_v5.db"),
        )
        .env(PAPER_CHILD_ACTION, action)
        .env(PAPER_CHILD_COMMAND, command);
    if let Some(expected) = expected {
        process
            .env(PAPER_CHILD_EXPECTED, expected.version.to_string())
            .env(PAPER_CHILD_HEAD_HASH, &expected.event_hash);
    }
    let output = process.output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        output.status.success(),
        "actual child failed: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("running 1 test"),
        "child filter did not run exactly one case: {stdout}"
    );
    assert!(
        stdout.contains("TEST_CODE_PAPER_CHILD_COMMITTED"),
        "child did not commit original financial event: {stdout}"
    );
}
#[test]
#[ignore = "only invoked by exact parent reexec with an actual isolated Test namespace"]
fn TEST_CODE_paper_v2_execution_global_child() {
    let path = std::path::PathBuf::from(std::env::var_os(PAPER_CHILD_PATH).unwrap());
    let db = DatabaseManager::open_frozen_catalog_for_isolated_test(path).unwrap();
    let action = std::env::var(PAPER_CHILD_ACTION).unwrap();
    let command = std::env::var(PAPER_CHILD_COMMAND).unwrap();
    let account = "TEST_CODE_ORIGINAL_ACCOUNT";
    let view = read_for_isolated_test(&db, account).unwrap();
    let expected = if let Ok(version) = std::env::var(PAPER_CHILD_EXPECTED) {
        HeadIdentity {
            version: version.parse().unwrap(),
            event_hash: std::env::var(PAPER_CHILD_HEAD_HASH).unwrap(),
        }
    } else {
        view.head
    };
    let receipt = apply_for_isolated_test(
        &db,
        account,
        PaperV2Command::Cancel {
            command_id: command.clone(),
            expected,
            parent_id: action,
        },
        at(8),
    )
    .unwrap();
    assert!(!receipt.replayed);
    let recovered = recover_command_for_isolated_test(&db, account, &command)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.receipt().head, receipt.head);
    println!(
        "TEST_CODE_PAPER_CHILD_COMMITTED {} {}",
        command, receipt.head.event_hash
    );
}

#[test]
fn paper_v2_execution_actual_global_postcommit_fresh_reader_unknown_preserves_both_commands() {
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_PARENT_CANCEL", Side::Sell, 100, 2);
    actual_submit(&f, "TEST_CODE_CHILD_CANCEL", Side::Sell, 100, 3);
    let expected = actual_view(&f).head;
    let directory = f.original.directory.path().to_path_buf();
    let account = f.manifest.account_id.clone();
    let manifest = f.manifest.clone();
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let hit = hits.clone();
    let guard = crate::database::install_retained_readback_after_checkpoint_hook(move || {
        hit.set(hit.get() + 1);
        let name =
            "trading::paper_book_v2_execution::tests::TEST_CODE_paper_v2_execution_global_child";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", &name, "--nocapture"])
            .env(PAPER_CHILD_PATH, directory.join("TEST_CODE_paper_v5.db"))
            .env(PAPER_CHILD_ACTION, "TEST_CODE_CHILD_CANCEL")
            .env(PAPER_CHILD_COMMAND, "TEST_CODE_COMMITTED_CHILD")
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "actual postcommit child failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("running 1 test"));
        assert!(stdout.contains("TEST_CODE_PAPER_CHILD_COMMITTED TEST_CODE_COMMITTED_CHILD"));
    });
    let outcome = apply_for_isolated_test(
        &f.original.db,
        &account,
        PaperV2Command::Cancel {
            command_id: "TEST_CODE_COMMITTED_PARENT".into(),
            expected: expected.clone(),
            parent_id: "TEST_CODE_PARENT_CANCEL".into(),
        },
        at(7),
    );
    drop(guard);
    assert!(matches!(outcome, Err(LedgerError::CommitOutcomeUnknown)));
    assert_eq!(hits.get(), 1);
    let current = actual_view(&f);
    assert_eq!(current.head.version, expected.version + 2);
    assert_eq!(
        current.projection.parents["TEST_CODE_PARENT_CANCEL"].status,
        ParentStatus::Cancelled
    );
    assert_eq!(
        current.projection.parents["TEST_CODE_CHILD_CANCEL"].status,
        ParentStatus::Cancelled
    );
    assert_eq!(current.manifest, manifest);
    let before = actual_rows(&f.original.db);
    for (command, parent, version) in [
        (
            "TEST_CODE_COMMITTED_PARENT",
            "TEST_CODE_PARENT_CANCEL",
            expected.version + 1,
        ),
        (
            "TEST_CODE_COMMITTED_CHILD",
            "TEST_CODE_CHILD_CANCEL",
            expected.version + 2,
        ),
    ] {
        let original = recover_command_for_isolated_test(&f.original.db, &account, command)
            .unwrap()
            .unwrap();
        assert_eq!(original.receipt().head.version, version);
        assert_eq!(original.observed_head(), &current.head);
        assert!(
            matches!(original.request(), CommandRecord::Cancel { parent_id, .. } if parent_id == parent)
        );
    }
    let retry = apply_for_isolated_test(
        &f.original.db,
        &account,
        PaperV2Command::Cancel {
            command_id: "TEST_CODE_COMMITTED_PARENT".into(),
            expected,
            parent_id: "TEST_CODE_PARENT_CANCEL".into(),
        },
        at(0) + chrono::Duration::minutes(10),
    )
    .unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.head.version, current.head.version - 1);
    assert_eq!(actual_rows(&f.original.db), before);
    assert_eq!(
        original_payloads(&f.original.db, &account),
        f.original.old_payloads
    );
}

#[test]
fn paper_v2_execution_actual_global_second_process_stale_head_cannot_double_release_cash() {
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_RACE_ONE", Side::Sell, 100, 2);
    actual_submit(&f, "TEST_CODE_RACE_TWO", Side::Sell, 100, 3);
    let expected = actual_view(&f).head;
    run_actual_child(
        &f,
        "TEST_CODE_RACE_ONE",
        "TEST_CODE_RACE_WINNER",
        Some(&expected),
    );
    let before = actual_rows(&f.original.db);
    assert!(matches!(
        apply_for_isolated_test(
            &f.original.db,
            &f.manifest.account_id,
            PaperV2Command::Cancel {
                command_id: "TEST_CODE_RACE_LOSER".into(),
                expected,
                parent_id: "TEST_CODE_RACE_TWO".into(),
            },
            at(8)
        ),
        Err(LedgerError::VersionChanged)
    ));
    assert_eq!(actual_rows(&f.original.db), before);
    assert_eq!(
        actual_view(&f).projection.parents["TEST_CODE_RACE_TWO"].status,
        ParentStatus::Working
    );
    assert!(recover_command_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        "TEST_CODE_RACE_LOSER"
    )
    .unwrap()
    .is_none());
}

#[test]
fn paper_v2_execution_closed_window_extreme_clock_is_typed_rejection_not_panic() {
    for observed in [
        DateTime::<Utc>::MAX_UTC,
        DateTime::<Utc>::MAX_UTC - chrono::Duration::hours(7),
    ] {
        let mut record = window("TEST_CODE_EXTREME_CLOSED_RECORD", 0, 10_000_000, 100);
        record.source_at = observed;
        record.observed_at = observed;
        record.fresh_through = observed;
        record.facts_source_at = observed.to_rfc3339();
        record.facts_observed_at = observed.to_rfc3339();
        let raw = encode(&record).unwrap();
        let parsed: WindowRecord = decode(&raw).unwrap();
        let result = std::panic::catch_unwind(|| parsed.validate())
            .expect("closed record validation must not panic");
        assert!(
            matches!(result, Err(LedgerError::EvidenceUnavailable(ref why)) if why == "execution window Shanghai clock exceeds supported range")
        );
    }
}

#[test]
fn paper_v2_execution_actual_global_expire_releases_once_without_new_source() {
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_EXPIRING", Side::Sell, 100, 2);
    let view = actual_view(&f);
    let before = actual_rows(&f.original.db);
    assert!(apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Expire {
            command_id: "TEST_CODE_EXPIRE_EARLY".into(),
            expected: view.head.clone(),
            parent_id: "TEST_CODE_EXPIRING".into(),
        },
        at(3)
    )
    .is_err());
    assert_eq!(actual_rows(&f.original.db), before);
    let close = Utc.with_ymd_and_hms(2026, 9, 24, 7, 0, 0).unwrap();
    let receipt = apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Expire {
            command_id: "TEST_CODE_EXPIRE".into(),
            expected: view.head.clone(),
            parent_id: "TEST_CODE_EXPIRING".into(),
        },
        close,
    )
    .unwrap();
    let expired = actual_view(&f);
    assert_eq!(
        expired.projection.parents["TEST_CODE_EXPIRING"].status,
        ParentStatus::Expired
    );
    assert_eq!(
        expired.projection.parents["TEST_CODE_EXPIRING"]
            .reservation
            .cash_reserve,
        0
    );
    assert!(expired.projection.parents["TEST_CODE_EXPIRING"]
        .sell_claims
        .is_empty());
    assert_eq!(expired.projection.cash, view.projection.cash);
    let before = actual_rows(&f.original.db);
    let retry = apply_for_isolated_test(
        &f.original.db,
        &f.manifest.account_id,
        PaperV2Command::Expire {
            command_id: "TEST_CODE_EXPIRE".into(),
            expected: view.head,
            parent_id: "TEST_CODE_EXPIRING".into(),
        },
        close + chrono::Duration::hours(1),
    )
    .unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.head, receipt.head);
    assert_eq!(actual_rows(&f.original.db), before);
}

#[test]
fn paper_v2_execution_closed_expire_extreme_clock_replay_and_writer_leave_facts_unchanged() {
    let g = genesis(10_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut state = ExecutionProjection::initial(&g, &m.budget).unwrap();
    submit(&mut state, &m, "TEST_CODE_REPLAY_EXPIRE", Side::Buy, 100, 1).unwrap();
    let original = state.clone();
    let fact = Fact {
        request: CommandRecord::Expire {
            expected: head(),
            parent_id: "TEST_CODE_REPLAY_EXPIRE".into(),
            at: DateTime::<Utc>::MAX_UTC,
        },
        effect: Effect::Expired,
    };
    let raw = encode(&fact).unwrap();
    let recorded: Fact = decode(&raw).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_request(&mut state, &m, &recorded.request)
    }))
    .expect("full recorded command replay must not panic");
    assert!(
        matches!(result, Err(LedgerError::EvidenceUnavailable(ref why)) if why == "paper execution Shanghai clock exceeds supported range")
    );
    assert_eq!(state, original);
    assert!(matches!(
        state.marked_at(DateTime::<Utc>::MAX_UTC),
        Err(LedgerError::EvidenceUnavailable(_))
    ));
    let inherited = genesis(10_000_000_000, vec![lot("TEST_CODE_BAD_MARK_LOT", 100, 0)]);
    let allocation = policy(&inherited, 5_000_000_000, 10_000_000_000);
    let mut marked = ExecutionProjection::initial(&inherited, &allocation).unwrap();
    marked.account.marks.get_mut("600001").unwrap().observed_at = DateTime::<Utc>::MAX_UTC;
    let recorded: ExecutionProjection = decode(&encode(&marked).unwrap()).unwrap();
    let result = std::panic::catch_unwind(|| recorded.marked_at(at(1)))
        .expect("closed original mark clock must not panic");
    assert!(
        matches!(result, Err(LedgerError::EvidenceUnavailable(ref why)) if why == "paper execution Shanghai clock exceeds supported range")
    );
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_ACTUAL_EXPIRE", Side::Sell, 100, 2);
    let before = actual_rows(&f.original.db);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_for_isolated_test(
            &f.original.db,
            &f.manifest.account_id,
            PaperV2Command::Expire {
                command_id: "TEST_CODE_BAD_EXPIRE".into(),
                expected: actual_view(&f).head,
                parent_id: "TEST_CODE_ACTUAL_EXPIRE".into(),
            },
            DateTime::<Utc>::MAX_UTC,
        )
    }))
    .expect("actual writer must reject out-of-range closed clock without panic");
    assert!(
        matches!(result, Err(LedgerError::EvidenceUnavailable(ref why)) if why == "paper execution Shanghai clock exceeds supported range")
    );
    assert_eq!(actual_rows(&f.original.db), before);
}

fn fractional_fee() -> AShareFeePolicyV2 {
    AShareFeePolicyV2::new(
        fee().scope(),
        FeeRate::new(1, 3).unwrap(),
        0,
        FeeCoverage::initial_model(),
        "TEST_CODE_FRACTIONAL_REVIEWED_DESCRIPTOR",
    )
    .unwrap()
}
#[test]
fn paper_v2_execution_fee_reserve_bounds_every_partition_and_component_rounding() {
    use crate::performance::fee_policy::{
        a_share_stock_fill_fee_with_policy_v2, FeeCoverageRequirement,
    };
    let policy = fractional_fee();
    let one = fill_model::model(
        Side::Buy,
        100,
        10_000_000,
        10_000_000,
        &window("TEST_CODE_ONE_FEE", 1, 10_000_000, 100),
        &policy,
    )
    .unwrap();
    assert!(
        matches!(one, ModelOutcome::Fill(ref fill) if fill.commission_micro_cny == 333_333_333)
    );
    let bound = fill_model::worst_case_fee(Side::Buy, 300, 10_000_000, date(), &policy).unwrap();
    assert_eq!(bound, 1_000_000_002);
    let bulk = fill_model::model(
        Side::Buy,
        300,
        10_000_000,
        10_000_000,
        &window("TEST_CODE_BULK_FEE", 1, 10_000_000, 300),
        &policy,
    )
    .unwrap();
    assert!(
        matches!(bulk, ModelOutcome::Fill(ref fill) if fill.commission_micro_cny == 1_000_000_000)
    );
    // Enumerate all ordered partitions for small whole-lot quantities, each
    // tested for buy/sell, both date brackets, nondivisible prices, and minimum.
    fn partitions(left: u32, prefix: &mut Vec<u32>, all: &mut Vec<Vec<u32>>) {
        if left == 0 {
            all.push(prefix.clone());
            return;
        }
        for amount in 1..=left {
            prefix.push(amount);
            partitions(left - amount, prefix, all);
            prefix.pop();
        }
    }
    for policy in [fractional_fee(), fee()] {
        for day in [NaiveDate::from_ymd_opt(2023, 8, 25).unwrap(), date()] {
            for price in [10_000_000, 10_000_001] {
                for side in [Side::Buy, Side::Sell] {
                    for lots in 1..=6 {
                        let bound =
                            fill_model::worst_case_fee(side, lots * 100, price, day, &policy)
                                .unwrap();
                        let mut all = Vec::new();
                        partitions(lots, &mut Vec::new(), &mut all);
                        for parts in all {
                            let total: i64 = parts
                                .into_iter()
                                .map(|n| {
                                    a_share_stock_fill_fee_with_policy_v2(
                                        &policy,
                                        policy.scope(),
                                        if side == Side::Buy {
                                            crate::performance::fee_evidence::FillSide::Buy
                                        } else {
                                            crate::performance::fee_evidence::FillSide::Sell
                                        },
                                        budget::notional(price, n * 100).unwrap(),
                                        day,
                                        FeeCoverageRequirement::ModeledComponentsOnly,
                                    )
                                    .unwrap()
                                    .total_micro_cny
                                })
                                .sum();
                            assert!(
                                total <= bound,
                                "legal partition fees {total} exceed frozen {bound}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        fill_model::worst_case_fee(Side::Buy, 300, 10_000_000, date(), &fee()).unwrap(),
        15_000_000
    );
}

#[test]
fn paper_v2_execution_actual_global_fractional_bulk_and_remaining_partition_reserve_are_sufficient()
{
    for bulk in [true, false] {
        let f = actual_v6_fixture_with_fee(fractional_fee());
        actual_marks(&f, 2);
        actual_submit(&f, "TEST_CODE_FRACTIONAL_BUY", Side::Buy, 300, 3);
        let submitted = actual_view(&f);
        assert_eq!(
            submitted.projection.parents["TEST_CODE_FRACTIONAL_BUY"]
                .reservation
                .fee_reserve,
            1_000_000_002
        );
        assert_eq!(
            submitted.projection.parents["TEST_CODE_FRACTIONAL_BUY"]
                .reservation
                .cash_reserve,
            4_000_000_002
        );
        if bulk {
            actual_evaluate(
                &f,
                "TEST_CODE_FRACTIONAL_BUY",
                "TEST_CODE_FRACTIONAL_BULK",
                4,
                300,
            );
            let filled = actual_view(&f);
            assert_eq!(filled.projection.account.fees.micros(), 1_000_000_000);
            assert_eq!(filled.projection.cash.strategy_cash, 5_700_000_000);
            assert_eq!(
                filled.projection.parents["TEST_CODE_FRACTIONAL_BUY"].status,
                ParentStatus::Filled
            );
            assert_eq!(
                filled.projection.parents["TEST_CODE_FRACTIONAL_BUY"]
                    .reservation
                    .cash_reserve,
                0
            );
        } else {
            actual_evaluate(
                &f,
                "TEST_CODE_FRACTIONAL_BUY",
                "TEST_CODE_FRACTIONAL_FIRST",
                4,
                100,
            );
            let partial = actual_view(&f);
            assert_eq!(partial.projection.account.fees.micros(), 333_333_333);
            assert_eq!(
                partial.projection.parents["TEST_CODE_FRACTIONAL_BUY"]
                    .reservation
                    .fee_reserve,
                666_666_668
            );
            assert_eq!(
                partial.projection.parents["TEST_CODE_FRACTIONAL_BUY"]
                    .reservation
                    .cash_reserve,
                2_666_666_668
            );
            actual_evaluate(
                &f,
                "TEST_CODE_FRACTIONAL_BUY",
                "TEST_CODE_FRACTIONAL_REMAINING",
                5,
                200,
            );
            let filled = actual_view(&f);
            assert_eq!(filled.projection.account.fees.micros(), 1_000_000_000);
            assert_eq!(
                filled.projection.parents["TEST_CODE_FRACTIONAL_BUY"].status,
                ParentStatus::Filled
            );
            assert_eq!(
                filled.projection.parents["TEST_CODE_FRACTIONAL_BUY"]
                    .reservation
                    .cash_reserve,
                0
            );
        }
        assert_eq!(
            actual_view(&f).projection.cash.unassigned_cash,
            10_300_000_000
        );
        assert_eq!(
            original_payloads(&f.original.db, &f.manifest.account_id),
            f.original.old_payloads
        );
    }
}

#[test]
fn paper_v2_execution_actual_global_readonly_hook_stays_query_only_and_returns_exact_facts() {
    #[derive(QueryableByName)]
    struct ControlValue {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_READONLY_PARENT", Side::Sell, 100, 2);
    let original = actual_view(&f);
    let before = actual_rows(&f.original.db);
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let hit = hits.clone();
    let guard = install_test_hook(TestPhase::AfterRead, move |conn| {
        hit.set(hit.get() + 1);
        let control = diesel::sql_query("SELECT query_only AS value FROM pragma_query_only")
            .get_result::<ControlValue>(conn)
            .unwrap();
        assert_eq!(control.value, 1);
        assert!(
            diesel::sql_query("DELETE FROM paper_book_v2_execution_head")
                .execute(conn)
                .is_err()
        );
    });
    assert_eq!(actual_view(&f), original);
    drop(guard);
    assert_eq!(hits.get(), 1);
    assert_eq!(actual_rows(&f.original.db), before);
}


// Heap buffers belong to the original nonClone command, not to the movable
// outer frame's address. These observations create no authorization object.
fn retained_fixed_command_buffers(command: &PaperV2Command) -> (usize, usize, usize) {
    match command {
        PaperV2Command::Cancel { command_id, expected, parent_id } =>
            (command_id.as_ptr() as usize, parent_id.as_ptr() as usize,
                expected.event_hash.as_ptr() as usize),
        _ => panic!("this slice uses an actual fixed Cancel command"),
    }
}
fn retained_fixed_return_buffers(value: &RetainedExecutionAcquired) -> (usize, usize, usize, usize, usize) {
    let request_buffer = match value.fresh_request.as_ref() {
        Some(CommandRecord::Cancel { parent_id, .. }) => parent_id.as_ptr() as usize,
        None => 0,
        _ => panic!("this fixed return must preserve its actual Cancel record"),
    };
    (value.receipt.command_id.as_ptr() as usize, value.receipt.account_id.as_ptr() as usize,
        value.receipt.head.event_hash.as_ptr() as usize, value.binding.rows.events.as_ptr() as usize,
        request_buffer)
}
// Re-entry is a resource-retention check, never an SQL retry or reconstruction.
fn retained_fixed_assert_no_retry<'db, 'a>(
    frame: crate::database::global_schema_v1::paper_v6::RetainedPaperWrite<
        'db, RetainedExecutionInput<'a>, RetainedExecutionAcquired, LedgerError>,
) -> crate::database::global_schema_v1::paper_v6::RetainedPaperWrite<
    'db, RetainedExecutionInput<'a>, RetainedExecutionAcquired, LedgerError> {
    use crate::database::global_schema_v1::paper_v6::{RetainedPaperWriteOutcome, RetainedPaperWritePhase};
    let frame = Box::new(frame);
    let phase = frame.phase();
    let (input, value, work, error, faults) = frame.observe_fixed_execution_for_test();
    let input_buffers = retained_fixed_command_buffers(&input.command);
    let value_buffers = value.map(retained_fixed_return_buffers);
    let error_kind = error.map(std::mem::discriminant);
    let error_buffer = match error {
        Some(LedgerError::IntegrityFailure(reason)) => Some(reason.as_ptr() as usize),
        _ => None,
    };
    // The same Box placement is observed twice. No address is compared across
    // the consuming run_once move below; only remaining credit and owned heaps.
    assert_eq!(frame.observe_fixed_execution_for_test().2, work);
    let remaining = work.map(|(_, remaining)| remaining);
    let outcome = (*frame).run_once(
        |_, _, _, _| panic!("finished/refused fixed owner cannot run another callback"),
        |_, _, _, _, _| panic!("finished/refused fixed owner cannot run another tail"),
    );
    let frame = match (phase, outcome) {
        (RetainedPaperWritePhase::Complete, RetainedPaperWriteOutcome::Complete(frame)) => frame,
        (RetainedPaperWritePhase::WriterStopped | RetainedPaperWritePhase::Unopened,
            RetainedPaperWriteOutcome::Held(frame)) => frame,
        (RetainedPaperWritePhase::CommittedReadbackPending | RetainedPaperWritePhase::CommitUnknown,
            RetainedPaperWriteOutcome::Pending(frame)) => frame,
        _ => panic!("repeat must preserve the actual fixed owner's classification"),
    };
    let (input, value, work, error, after_faults) = frame.observe_fixed_execution_for_test();
    assert_eq!(frame.phase(), phase);
    assert_eq!(retained_fixed_command_buffers(&input.command), input_buffers);
    assert_eq!(value.map(retained_fixed_return_buffers), value_buffers);
    assert_eq!(work.map(|(_, remaining)| remaining), remaining);
    assert_eq!(after_faults, faults);
    assert_eq!(error.map(std::mem::discriminant), error_kind);
    assert_eq!(match error {
        Some(LedgerError::IntegrityFailure(reason)) => Some(reason.as_ptr() as usize),
        _ => None,
    }, error_buffer);
    frame
}

#[test]
fn paper_retained_fixed_actual_commit_and_readback_keep_owned_return() {
    use crate::database::global_schema_v1::paper_v6::{RetainedPaperWriteOutcome, RetainedPaperWritePhase};
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_RETAINED_COMPLETE_PARENT", Side::Sell, 100, 2);
    let expected = actual_view(&f).head;
    let before = actual_rows(&f.original.db);
    let command = PaperV2Command::Cancel {
        command_id: "TEST_CODE_RETAINED_COMPLETE_COMMAND".into(),
        expected: expected.clone(), parent_id: "TEST_CODE_RETAINED_COMPLETE_PARENT".into(),
    };
    let original = retained_fixed_command_buffers(&command);
    let frame = match apply_retained_for_isolated_test(&f.original.db, &f.manifest.account_id, command, at(7)) {
        RetainedPaperWriteOutcome::Complete(frame) => frame,
        _ => panic!("true fixed COMMIT and independent read-back must complete"),
    };
    assert_eq!(frame.phase(), RetainedPaperWritePhase::Complete);
    let (input, value, work, error, faults) = frame.observe_fixed_execution_for_test();
    assert_eq!(retained_fixed_command_buffers(&input.command), original);
    assert!(input.actual.is_some() && input.sampled_at == Some(at(7)));
    assert!(input.receipt.is_none() && input.binding.is_none() && input.record.is_none());
    assert!(error.is_none());
    assert_eq!(faults, (false, false, false));
    assert!(work.is_some_and(|(_, remaining)| remaining < 32 * 1024 * 1024));
    let value = value.expect("actual acquired return belongs to the complete frame");
    assert!(!value.receipt.replayed && !value.binding.rows.events.is_empty());
    assert_eq!(value.receipt.head.version, expected.version + 1);
    assert!(matches!(value.fresh_request.as_ref(), Some(CommandRecord::Cancel { parent_id, .. })
        if parent_id == "TEST_CODE_RETAINED_COMPLETE_PARENT"));
    let acquired_buffers = retained_fixed_return_buffers(value);
    let after = actual_rows(&f.original.db);
    assert_eq!(after.events.len(), before.events.len() + 1);
    assert_eq!(after, value.binding.rows);
    assert_eq!(actual_view(&f).projection.parents["TEST_CODE_RETAINED_COMPLETE_PARENT"].status, ParentStatus::Cancelled);
    let frame = retained_fixed_assert_no_retry(frame);
    assert_eq!(actual_rows(&f.original.db), after);
    let (input, value) = match frame.finish_complete() {
        Ok(owned) => owned,
        Err(_) => panic!("only observed complete releases the actual input and return"),
    };
    assert_eq!(retained_fixed_command_buffers(&input.command), original);
    assert_eq!(retained_fixed_return_buffers(&value), acquired_buffers);
    assert_eq!(value.binding.rows, after);
    assert_eq!(original_payloads(&f.original.db, &f.manifest.account_id), f.original.old_payloads);
    drop((input, value));

    // A real schema change after the true COMMIT/checkpoint rejects the fresh
    // reader. It is not a callback-supplied error pretending to be COMMIT Err.
    let pending = actual_v6_fixture();
    actual_submit(&pending, "TEST_CODE_RETAINED_PENDING_PARENT", Side::Sell, 100, 2);
    let expected = actual_view(&pending).head;
    let before = actual_rows(&pending.original.db);
    let path = pending.original.directory.path().join("TEST_CODE_paper_v5.db");
    let hook_path = path.clone();
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let hit = hits.clone();
    let guard = crate::database::install_retained_readback_after_checkpoint_hook(move || {
        hit.set(hit.get() + 1);
        let mut conn = SqliteConnection::establish(hook_path.to_str().unwrap()).unwrap();
        diesel::sql_query("CREATE TABLE TEST_CODE_retained_postcommit_cut(value INTEGER)")
            .execute(&mut conn).unwrap();
    });
    let command = PaperV2Command::Cancel {
        command_id: "TEST_CODE_RETAINED_PENDING_COMMAND".into(),
        expected: expected.clone(), parent_id: "TEST_CODE_RETAINED_PENDING_PARENT".into(),
    };
    let original = retained_fixed_command_buffers(&command);
    let frame = match apply_retained_for_isolated_test(&pending.original.db, &pending.manifest.account_id, command, at(7)) {
        RetainedPaperWriteOutcome::Pending(frame) => frame,
        _ => panic!("post-COMMIT catalog refusal must retain the actual acquired return"),
    };
    drop(guard);
    assert_eq!(hits.get(), 1);
    assert_eq!(frame.phase(), RetainedPaperWritePhase::CommittedReadbackPending);
    let (input, value, work, error, faults) = frame.observe_fixed_execution_for_test();
    assert_eq!(retained_fixed_command_buffers(&input.command), original);
    assert!(input.actual.is_some() && input.sampled_at == Some(at(7)));
    assert!(error.is_none());
    assert_eq!(faults, (false, false, true));
    assert!(work.is_some_and(|(_, remaining)| remaining < 32 * 1024 * 1024));
    let value = value.expect("the COMMIT return remains owned despite catalog refusal");
    assert_eq!(value.receipt.head.version, expected.version + 1);
    assert!(!value.receipt.replayed && value.fresh_request.is_some());
    let mut observation = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    let committed = sql_rows(&mut observation).unwrap();
    assert_eq!(committed.events.len(), before.events.len() + 1);
    assert_eq!(committed, value.binding.rows);
    // This ordinary fixture read is not a new qualified business reader.
    let frame = retained_fixed_assert_no_retry(frame);
    assert_eq!(sql_rows(&mut observation).unwrap(), committed);
    let frame = match frame.finish_complete() {
        Err(owner) => owner,
        Ok(_) => panic!("catalog refusal cannot release a success return"),
    };
    assert_eq!(frame.phase(), RetainedPaperWritePhase::CommittedReadbackPending);
    drop(frame); // Explicit fixture teardown, not recovery or automatic retry.
}

#[test]
fn paper_retained_fixed_stale_head_and_writer_tail_keep_first_fault() {
    use crate::database::global_schema_v1::paper_v6::{RetainedPaperWriteOutcome, RetainedPaperWritePhase};
    {
        // The original idempotent singleton init does not install an isolated manager.
        DatabaseManager::init(None).unwrap();
        let command = PaperV2Command::Cancel { command_id: "TEST_CODE_BRIDGE_REFUSED".into(),
            expected: HeadIdentity { version: 1, event_hash: "a".repeat(64) }, parent_id: "TEST_CODE_BRIDGE_PARENT".into() };
        let original = retained_fixed_command_buffers(&command);
        let frame = match apply_retained_for_isolated_test(DatabaseManager::get(), "TEST_CODE_BRIDGE_ACCOUNT", command, at(7)) {
            RetainedPaperWriteOutcome::Held(frame) => frame,
            _ => panic!("ordinary manager has no constructor-issued isolated origin"),
        };
        let (input, value, work, _, faults) = frame.observe_fixed_execution_for_test();
        assert_eq!(frame.phase(), RetainedPaperWritePhase::Unopened);
        assert_eq!(retained_fixed_command_buffers(&input.command), original);
        assert!(input.actual.is_none() && value.is_none() && work.is_none());
        assert_eq!(faults, (true, false, false));
        drop(retained_fixed_assert_no_retry(frame)); // No allocated Work witness.
    }
    let f = actual_v6_fixture();
    let stale = actual_view(&f).head;
    actual_submit(&f, "TEST_CODE_RETAINED_WRITER_PARENT", Side::Sell, 100, 2);
    actual_submit(&f, "TEST_CODE_RETAINED_EXTRA_PARENT", Side::Sell, 100, 3);
    let before = actual_rows(&f.original.db);
    let command = PaperV2Command::Cancel { command_id: "TEST_CODE_RETAINED_STALE".into(),
        expected: stale, parent_id: "TEST_CODE_RETAINED_WRITER_PARENT".into() };
    let original = retained_fixed_command_buffers(&command);
    let frame = match apply_retained_for_isolated_test(&f.original.db, &f.manifest.account_id, command, at(7)) {
        RetainedPaperWriteOutcome::Held(frame) => frame,
        _ => panic!("actual stale head must stop before a commit-ready return"),
    };
    let (input, value, work, error, faults) = frame.observe_fixed_execution_for_test();
    assert_eq!(frame.phase(), RetainedPaperWritePhase::WriterStopped);
    assert_eq!(retained_fixed_command_buffers(&input.command), original);
    assert!(input.actual.is_some() && input.sampled_at == Some(at(7)) && input.record.is_some());
    assert!(input.receipt.is_none() && value.is_none());
    assert!(matches!(error, Some(LedgerError::VersionChanged)));
    assert_eq!(faults, (true, true, false));
    assert!(work.is_some_and(|(_, remaining)| remaining < 32 * 1024 * 1024));
    let frame = retained_fixed_assert_no_retry(frame);
    assert_eq!(actual_rows(&f.original.db), before);
    drop(frame);

    let expected = actual_view(&f).head;
    let account = f.manifest.account_id.clone();
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let hit = hits.clone();
    let guard = install_test_hook(TestPhase::LastSqlBeforeCommit, move |conn| {
        hit.set(hit.get() + 1);
        append_hook_cancel(conn, &account, "TEST_CODE_RETAINED_EXTRA_PARENT", "TEST_CODE_RETAINED_EXTRA_EVENT");
    });
    let command = PaperV2Command::Cancel { command_id: "TEST_CODE_RETAINED_WRITER_TAIL".into(),
        expected: expected.clone(), parent_id: "TEST_CODE_RETAINED_WRITER_PARENT".into() };
    let original = retained_fixed_command_buffers(&command);
    let frame = match apply_retained_for_isolated_test(&f.original.db, &f.manifest.account_id, command, at(7)) {
        RetainedPaperWriteOutcome::Held(frame) => frame,
        _ => panic!("real legal writer mutation must stop at the fixed SQL-binding tail"),
    };
    drop(guard);
    assert_eq!(hits.get(), 1);
    let (input, value, _, error, faults) = frame.observe_fixed_execution_for_test();
    assert_eq!(frame.phase(), RetainedPaperWritePhase::WriterStopped);
    assert_eq!(retained_fixed_command_buffers(&input.command), original);
    assert!(input.actual.is_some() && input.sampled_at == Some(at(7)));
    assert!(input.receipt.is_none() && input.binding.is_none() && input.record.is_none());
    let value = value.expect("actual T was acquired before the writer tail failed");
    assert_eq!(value.receipt.head.version, expected.version + 1);
    assert!(!value.receipt.replayed && value.fresh_request.is_some());
    assert!(matches!(error, Some(LedgerError::IntegrityFailure(reason))
        if reason == "execution SQL binding changed after operation"));
    assert_eq!(faults, (true, true, false));
    assert_eq!(actual_rows(&f.original.db), before); // Real post-error rows; internal rollback remains unobserved.
    let frame = retained_fixed_assert_no_retry(frame);
    assert_eq!(actual_rows(&f.original.db), before);
    let frame = match frame.finish_complete() {
        Err(owner) => owner,
        Ok(_) => panic!("writer-tail refusal cannot release a success return"),
    };
    assert!(frame.observe_fixed_execution_for_test().1.is_some());
    assert_eq!(original_payloads(&f.original.db, &f.manifest.account_id), f.original.old_payloads);
    drop(frame);
}

#[test]
fn paper_retained_fixed_checkpoint_child_readback_keeps_both_commands() {
    use crate::database::global_schema_v1::paper_v6::{RetainedPaperWriteOutcome, RetainedPaperWritePhase};
    let f = actual_v6_fixture();
    actual_submit(&f, "TEST_CODE_RETAINED_PARENT_CANCEL", Side::Sell, 100, 2);
    actual_submit(&f, "TEST_CODE_RETAINED_CHILD_CANCEL", Side::Sell, 100, 3);
    let expected = actual_view(&f).head;
    let directory = f.original.directory.path().to_path_buf();
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let hit = hits.clone();
    let guard = crate::database::install_retained_readback_after_checkpoint_hook(move || {
        hit.set(hit.get() + 1);
        let name = "trading::paper_book_v2_execution::tests::TEST_CODE_paper_v2_execution_global_child";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", name, "--nocapture"])
            .env(PAPER_CHILD_PATH, directory.join("TEST_CODE_paper_v5.db"))
            .env(PAPER_CHILD_ACTION, "TEST_CODE_RETAINED_CHILD_CANCEL")
            .env(PAPER_CHILD_COMMAND, "TEST_CODE_RETAINED_COMMITTED_CHILD")
            .output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(output.status.success(), "actual retained child failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr));
        assert!(stdout.contains("running 1 test"));
        assert!(stdout.contains("TEST_CODE_PAPER_CHILD_COMMITTED TEST_CODE_RETAINED_COMMITTED_CHILD"));
    });
    let command = PaperV2Command::Cancel { command_id: "TEST_CODE_RETAINED_COMMITTED_PARENT".into(),
        expected: expected.clone(), parent_id: "TEST_CODE_RETAINED_PARENT_CANCEL".into() };
    let original = retained_fixed_command_buffers(&command);
    let frame = match apply_retained_for_isolated_test(&f.original.db, &f.manifest.account_id, command, at(7)) {
        RetainedPaperWriteOutcome::Pending(frame) => frame,
        _ => panic!("child COMMIT before fresh read-back must retain the original parent return"),
    };
    drop(guard);
    assert_eq!(hits.get(), 1);
    let (input, value, work, error, faults) = frame.observe_fixed_execution_for_test();
    assert_eq!(frame.phase(), RetainedPaperWritePhase::CommittedReadbackPending);
    assert_eq!(retained_fixed_command_buffers(&input.command), original);
    assert!(input.actual.is_some() && input.sampled_at == Some(at(7)));
    let value = value.expect("parent's actual receipt/binding remain owned");
    assert_eq!(value.receipt.head.version, expected.version + 1);
    assert!(!value.receipt.replayed && value.fresh_request.is_some());
    assert!(matches!(error, Some(LedgerError::IntegrityFailure(reason))
        if reason == "execution SQL binding changed after operation"));
    assert_eq!(faults, (true, false, true));
    assert!(work.is_some_and(|(_, remaining)| remaining < 32 * 1024 * 1024));
    let current = actual_view(&f);
    assert_eq!(current.head.version, expected.version + 2);
    for (parent, command, version) in [
        ("TEST_CODE_RETAINED_PARENT_CANCEL", "TEST_CODE_RETAINED_COMMITTED_PARENT", expected.version + 1),
        ("TEST_CODE_RETAINED_CHILD_CANCEL", "TEST_CODE_RETAINED_COMMITTED_CHILD", expected.version + 2),
    ] {
        assert_eq!(current.projection.parents[parent].status, ParentStatus::Cancelled);
        let observed = recover_command_for_isolated_test(&f.original.db, &f.manifest.account_id, command)
            .unwrap().unwrap();
        assert_eq!(observed.receipt().head.version, version);
        assert_eq!(observed.observed_head(), &current.head);
        assert!(matches!(observed.request(), CommandRecord::Cancel { parent_id, .. } if parent_id == parent));
    }
    let before_repeat = actual_rows(&f.original.db);
    assert_ne!(value.binding.rows, before_repeat); // The original T is not reminted from the new head.
    let frame = retained_fixed_assert_no_retry(frame);
    assert_eq!(actual_rows(&f.original.db), before_repeat);
    let frame = match frame.finish_complete() {
        Err(owner) => owner,
        Ok(_) => panic!("pending read-back cannot release a success return"),
    };
    assert_eq!(frame.phase(), RetainedPaperWritePhase::CommittedReadbackPending);
    assert!(frame.observe_fixed_execution_for_test().1.is_some());
    assert_eq!(original_payloads(&f.original.db, &f.manifest.account_id), f.original.old_payloads);
    drop(frame); // Explicit fixture teardown; no automatic replay/recovery.
}

fn decision_submit_complete(
    db: &DatabaseManager,
    account: &str,
    parent: &str,
    quantity: u32,
    expected: HeadIdentity,
    now: DateTime<Utc>,
) -> PaperV2Receipt {
    use crate::database::global_schema_v1::paper_v6::RetainedPaperWriteOutcome;
    let approved = actual_approval(db, account, parent, Side::Sell, quantity, 2);
    let frame =
        match submit_approved_decision_for_isolated_test(db, account, expected, approved, now) {
            RetainedPaperWriteOutcome::Complete(frame) => frame,
            _ => panic!("actual decision submission and independent readback must complete"),
        };
    let (_, acquired) = match frame.finish_complete() {
        Ok(owned) => owned,
        Err(_) => panic!("complete frame must retain its actual return"),
    };
    acquired.receipt().clone()
}

#[test]
fn paper_decision_submit_reuses_original_after_partial_fill_and_cold_reopen() {
    let f = actual_v6_fixture();
    let account = f.manifest.account_id.clone();
    let parent = "TEST_CODE_DECISION_RETRY_PARENT";
    let first = decision_submit_complete(
        &f.original.db,
        &account,
        parent,
        200,
        actual_view(&f).head,
        at(2),
    );
    assert!(!first.replayed);
    actual_evaluate(&f, parent, "TEST_CODE_DECISION_PARTIAL", 3, 100);
    let changed_head = actual_view(&f).head;
    assert!(changed_head.version > first.head.version);
    let before = actual_rows(&f.original.db);
    let retry = decision_submit_complete(
        &f.original.db,
        &account,
        parent,
        200,
        changed_head.clone(),
        at(4),
    );
    assert!(retry.replayed);
    assert_eq!(retry.command_id, first.command_id);
    assert_eq!(retry.head, first.head);
    assert_eq!(actual_rows(&f.original.db), before);
    assert_eq!(actual_view(&f).projection.parents[parent].filled, 100);
    assert_eq!(
        original_payloads(&f.original.db, &account),
        f.original.old_payloads
    );

    let path = f.original.directory.path().join("TEST_CODE_paper_v5.db");
    drop(f.original.db);
    let reopened = DatabaseManager::open_frozen_catalog_for_isolated_test(path).unwrap();
    // The recorded approval is unchanged; a later head/time is only a read
    // witness. Staleness must not turn this into a fresh fill or second order.
    let cold = decision_submit_complete(
        &reopened,
        &account,
        parent,
        200,
        changed_head,
        at(59) + chrono::Duration::minutes(5),
    );
    assert_eq!(cold, retry);
    assert_eq!(actual_rows(&reopened), before);
}

#[test]
fn paper_decision_submit_conflict_and_foreign_namespace_hold_original_approval() {
    use crate::database::global_schema_v1::paper_v6::RetainedPaperWriteOutcome;
    let f = actual_v6_fixture();
    let account = &f.manifest.account_id;
    let parent = "TEST_CODE_DECISION_CONFLICT_PARENT";
    decision_submit_complete(
        &f.original.db,
        account,
        parent,
        200,
        actual_view(&f).head,
        at(2),
    );
    let before = actual_rows(&f.original.db);
    let changed = actual_approval(&f.original.db, account, parent, Side::Sell, 100, 2);
    let original_buffer = changed.record().investment_decision_id.as_ptr();
    let held = match submit_approved_decision_for_isolated_test(
        &f.original.db,
        account,
        actual_view(&f).head,
        changed,
        at(3),
    ) {
        RetainedPaperWriteOutcome::Held(frame) => frame,
        _ => panic!("changed intent at the same decision must conflict"),
    };
    let (input, acquired, _, error, _) = held.observe_fixed_execution_for_test();
    assert!(matches!(error, Some(LedgerError::IdentityConflict)));
    assert!(acquired.is_none());
    assert!(
        matches!(&input.command, PaperV2Command::Submit { approved, .. }
        if approved.record().investment_decision_id.as_ptr() == original_buffer)
    );
    drop(held);
    assert_eq!(actual_rows(&f.original.db), before);

    // A new decision still consumes the initial CAS; only a matching prior
    // committed decision may ignore a later caller's observation head.
    let new_approval = actual_approval(
        &f.original.db,
        account,
        "TEST_CODE_DECISION_STALE_HEAD_PARENT",
        Side::Sell,
        100,
        2,
    );
    let held = match submit_approved_decision_for_isolated_test(
        &f.original.db,
        account,
        head(),
        new_approval,
        at(3),
    ) {
        RetainedPaperWriteOutcome::Held(frame) => frame,
        _ => panic!("first submission cannot bypass the original CAS"),
    };
    assert!(matches!(
        held.observe_fixed_execution_for_test().3,
        Some(LedgerError::VersionChanged)
    ));
    drop(held);
    assert_eq!(actual_rows(&f.original.db), before);

    let foreign = actual_v6_fixture();
    let approval = actual_approval(
        &foreign.original.db,
        &foreign.manifest.account_id,
        parent,
        Side::Sell,
        200,
        2,
    );
    let held = match submit_approved_decision_for_isolated_test(
        &f.original.db,
        account,
        actual_view(&f).head,
        approval,
        at(4),
    ) {
        RetainedPaperWriteOutcome::Held(frame) => frame,
        _ => panic!("identical values from another namespace cannot approve a retry"),
    };
    let (_, acquired, _, error, _) = held.observe_fixed_execution_for_test();
    assert!(matches!(error, Some(LedgerError::IntegrityFailure(why))
        if why == "actual intent owner/namespace differs"));
    assert!(acquired.is_none());
    drop(held);
    assert_eq!(actual_rows(&f.original.db), before);
}

#[test]
fn paper_decision_submit_command_identity_golden_and_namespace_scope() {
    let g = genesis(20_000_000_000, vec![]);
    let m = manifest(&g, 10_000_000_000, 10_000_000_000);
    let mut record = intent(&m, "TEST_CODE_GOLD_PARENT", Side::Buy, 100, 2);
    let id = decision_submission_command_id(&record);
    // Independently calculated SHA-256: fixed domain followed by three
    // big-endian u64 lengths and their UTF-8 account/epoch/decision values.
    assert_eq!(
        id,
        "paper-decision-submit-v1:7ec1c07b9273c98a185d5e82303c7d8f36f3f1c147c29bb1e08dd8a6246e5e6c"
    );
    record.quantity = 200;
    record.parent_id = "TEST_CODE_CHANGED_PARENT".into();
    assert_eq!(decision_submission_command_id(&record), id);
    record.epoch_id.push('2');
    assert_ne!(decision_submission_command_id(&record), id);
    record.epoch_id.pop();
    record.account_id.push('2');
    assert_ne!(decision_submission_command_id(&record), id);
    record.account_id.pop();
    record.investment_decision_id.push('2');
    assert_ne!(decision_submission_command_id(&record), id);
}

#[test]
fn paper_decision_submit_postcommit_pending_is_retained_without_resubmission() {
    use crate::database::global_schema_v1::paper_v6::{
        RetainedPaperWriteOutcome, RetainedPaperWritePhase,
    };
    let f = actual_v6_fixture();
    let account = &f.manifest.account_id;
    let parent = "TEST_CODE_DECISION_PENDING_PARENT";
    let approved = actual_approval(&f.original.db, account, parent, Side::Sell, 100, 2);
    let id = decision_submission_command_id(approved.record());
    let expected = actual_view(&f).head;
    let db_path = f.original.directory.path().join("TEST_CODE_paper_v5.db");
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let hit = hits.clone();
    let guard = crate::database::install_retained_readback_after_checkpoint_hook(move || {
        hit.set(hit.get() + 1);
        let mut conn = SqliteConnection::establish(db_path.to_str().unwrap()).unwrap();
        diesel::sql_query("CREATE TABLE TEST_CODE_decision_postcommit_cut(value INTEGER)")
            .execute(&mut conn)
            .unwrap();
    });
    let frame = match submit_approved_decision_for_isolated_test(
        &f.original.db,
        account,
        expected,
        approved,
        at(2),
    ) {
        RetainedPaperWriteOutcome::Pending(frame) => frame,
        _ => panic!("committed decision with failed fresh catalog check must remain Pending"),
    };
    drop(guard);
    assert_eq!(hits.get(), 1);
    assert_eq!(
        frame.phase(),
        RetainedPaperWritePhase::CommittedReadbackPending
    );
    let (input, acquired, _, _, _) = frame.observe_fixed_execution_for_test();
    assert!(
        matches!(&input.command, PaperV2Command::Submit { command_id, .. } if command_id == &id)
    );
    assert!(acquired.is_some_and(|value| !value.receipt.replayed && value.receipt.command_id == id));
    let frame = match frame.finish_complete() {
        Err(frame) => frame,
        Ok(_) => panic!("Pending cannot release a successful decision"),
    };
    let outcome = frame.run_once(
        |_, _, _, _| panic!("Pending must never execute again"),
        |_, _, _, _, _| panic!("Pending must never recreate readback"),
    );
    assert!(matches!(outcome, RetainedPaperWriteOutcome::Pending(_)));
    // Ordinary fixture inspection after the deliberately invalid catalog;
    // this connection does not issue a business reader or execution authority.
    let mut conn = f.original.db.get_conn().unwrap();
    let rows = sql_rows(&mut conn).unwrap();
    assert_eq!(rows.parents.len(), 1);
    assert_eq!(
        rows.events
            .iter()
            .filter(|event| event.command_id == id)
            .count(),
        1
    );
}

#[test]
fn paper_decision_outcomes_preserve_partial_cancel_lineage_and_modeled_costs() {
    use crate::performance::paper_decision_outcomes_v1::{
        summarize_recorded_execution, RecordedDecisionLinkageV1, StrategyVersionEvidenceV1,
    };
    let f = actual_v6_fixture();
    let account = &f.manifest.account_id;
    let parent = "TEST_CODE_LINEAGE_PARTIAL_PARENT";
    decision_submit_complete(
        &f.original.db,
        account,
        parent,
        200,
        actual_view(&f).head,
        at(2),
    );
    actual_evaluate(&f, parent, "TEST_CODE_LINEAGE_PARTIAL_FILL", 3, 100);
    apply_for_isolated_test(
        &f.original.db,
        account,
        PaperV2Command::Cancel {
            command_id: "TEST_CODE_LINEAGE_CANCEL".into(),
            expected: actual_view(&f).head,
            parent_id: parent.into(),
        },
        at(4),
    )
    .unwrap();
    actual_submit(&f, "TEST_CODE_LINEAGE_UNFILLED_PARENT", Side::Sell, 100, 5);
    let before = actual_rows(&f.original.db);
    let view = actual_view(&f);
    let report = summarize_recorded_execution(&view).unwrap();
    assert_eq!(report.orders.len(), 2);
    assert_eq!(report.fills.len(), 1);
    assert_eq!(report.observed_head_version, view.head.version);
    assert_eq!(report.observed_head_hash, view.head.event_hash);
    assert_eq!(
        report.decision_linkage,
        RecordedDecisionLinkageV1::ReferenceOnly
    );
    assert_eq!(
        report.strategy_version_evidence,
        StrategyVersionEvidenceV1::NotRecorded
    );
    let row = report
        .orders
        .iter()
        .find(|row| row.parent_id == parent)
        .unwrap();
    assert_eq!(
        row.investment_decision_reference,
        format!("TEST_CODE_DECISION_{parent}")
    );
    assert_eq!(row.status, "Cancelled");
    assert_eq!(
        (
            row.requested_quantity,
            row.filled_quantity,
            row.remaining_quantity,
            row.cancelled_quantity
        ),
        (200, 100, 0, 100)
    );
    assert_eq!(row.fill_count, 1);
    assert_eq!(row.filled_notional_micro_cny, 1_000_000_000);
    // The explicit Shanghai scenario charges 5 CNY commission and 0.5 CNY tax.
    assert_eq!(row.modeled_fee_micro_cny, 5_500_000);
    assert_eq!(
        row.inherited_buy_fee_micro_cny,
        view.projection.fills[0].inherited_buy_fee_micro_cny
    );
    assert_eq!(
        row.realized_pnl_micro_cny,
        view.projection.fills[0].realized_pnl_micro_cny
    );
    assert_eq!(report.fills[0].fill_id, view.projection.fills[0].fill_id);
    assert_eq!(report.fills[0].parent_id, parent);
    assert_eq!(
        report.fills[0].observation_id,
        "TEST_CODE_LINEAGE_PARTIAL_FILL"
    );
    let working = report
        .orders
        .iter()
        .find(|row| row.parent_id != parent)
        .unwrap();
    assert_eq!((working.fill_count, working.realized_pnl_micro_cny), (0, 0));
    assert_eq!(working.status, "Working");
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["decision_linkage"], "ReferenceOnly");
    assert_eq!(json["strategy_version_evidence"], "NotRecorded");
    assert_eq!(actual_rows(&f.original.db), before);
}

#[test]
fn paper_decision_outcomes_reject_orphans_duplicate_fills_and_hidden_quantity_mismatch() {
    use crate::performance::paper_decision_outcomes_v1::summarize_recorded_execution;
    let f = actual_v6_fixture();
    let parent = "TEST_CODE_LINEAGE_INTEGRITY_PARENT";
    actual_submit(&f, parent, Side::Sell, 200, 2);
    actual_evaluate(&f, parent, "TEST_CODE_LINEAGE_INTEGRITY_FILL", 3, 100);
    let mut view = actual_view(&f);
    view.projection.fills[0].parent_id = "TEST_CODE_ORPHAN_PARENT".into();
    assert!(summarize_recorded_execution(&view).is_err());
    view.projection.fills[0].parent_id = parent.into();
    let duplicate = view.projection.fills[0].clone();
    view.projection.fills.push(duplicate);
    assert!(summarize_recorded_execution(&view).is_err());
    view.projection.fills.pop();
    view.projection.parents.get_mut(parent).unwrap().filled = 200;
    view.projection.parents.get_mut(parent).unwrap().remaining = 0;
    assert!(summarize_recorded_execution(&view).is_err());
    view.projection.parents.get_mut(parent).unwrap().filled = 100;
    view.projection.parents.get_mut(parent).unwrap().remaining = 100;
    view.projection.fills[0].model.total_fee_micro_cny += 1;
    assert!(summarize_recorded_execution(&view).is_err());
    view.projection.fills[0].model.total_fee_micro_cny -= 1;
    assert!(summarize_recorded_execution(&view).is_ok());
}

#[test]
fn paper_decision_outcomes_check_full_report_bounds_and_preserve_empty_observation() {
    use crate::performance::paper_decision_outcomes_v1::summarize_recorded_execution;
    let f = actual_v6_fixture();
    let empty = summarize_recorded_execution(&actual_view(&f)).unwrap();
    assert!(empty.orders.is_empty() && empty.fills.is_empty());
    actual_submit(&f, "TEST_CODE_LINEAGE_BOUND_PARENT", Side::Sell, 100, 2);
    let mut view = actual_view(&f);
    view.projection
        .parents
        .values_mut()
        .next()
        .unwrap()
        .intent
        .investment_decision_id = "x".repeat(257);
    assert!(summarize_recorded_execution(&view).is_err());
    view = actual_view(&f);
    let parent = view.projection.parents.values().next().unwrap().clone();
    for index in 0..1024 {
        view.projection
            .parents
            .insert(format!("TEST_CODE_BOUND_{index}"), parent.clone());
    }
    assert!(summarize_recorded_execution(&view).is_err());
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
}

#[test]
fn paper_decision_recover_cold_without_new_approval_or_market_window() {
    let f = actual_v6_fixture();
    let account = f.manifest.account_id.clone();
    let epoch = f.manifest.epoch_id.clone();
    let parent = "TEST_CODE_COLD_DECISION_RECOVERY";
    let decision = format!("TEST_CODE_DECISION_{parent}");
    let submitted = decision_submit_complete(
        &f.original.db,
        &account,
        parent,
        200,
        actual_view(&f).head,
        at(2),
    );
    actual_evaluate(&f, parent, "TEST_CODE_COLD_RECOVERY_FILL", 3, 100);
    let current = actual_view(&f).head;
    let before = actual_rows(&f.original.db);
    let old_payloads = original_payloads(&f.original.db, &account);
    let path = f.original.directory.path().join("TEST_CODE_paper_v5.db");
    drop(f.original.db);
    let reopened = DatabaseManager::open_frozen_catalog_for_isolated_test(path).unwrap();

    // No issuer, facts acquisition, new market window or fixed clock is called
    // after restart. Only an ordinary recorded receipt is returned.
    let recovered =
        recover_decision_submission_for_isolated_test(&reopened, &account, &epoch, &decision)
            .unwrap()
            .unwrap();
    assert!(recovered.receipt().replayed);
    assert_eq!(recovered.receipt().command_id, submitted.command_id);
    assert_eq!(recovered.receipt().head, submitted.head);
    assert_eq!(recovered.observed_head(), &current);
    assert!(
        matches!(recovered.request(), CommandRecord::Submit { intent, .. }
        if intent.parent_id == parent && intent.investment_decision_id == decision)
    );
    assert!(recover_decision_submission_for_isolated_test(
        &reopened,
        &account,
        "TEST_CODE_WRONG_EPOCH",
        &decision
    )
    .unwrap()
    .is_none());
    assert!(recover_decision_submission_for_isolated_test(
        &reopened,
        &account,
        &epoch,
        "TEST_CODE_ABSENT_DECISION"
    )
    .unwrap()
    .is_none());
    assert!(
        recover_decision_submission_for_isolated_test(&reopened, &account, &epoch, "\0").is_err()
    );
    assert_eq!(actual_rows(&reopened), before);
    assert_eq!(original_payloads(&reopened, &account), old_payloads);
}
