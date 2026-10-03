use super::*;
use super::super::{RowsSpecWork, ReplayResourceFailure, RowsSpecDebitFailure, ReplayTerminalState};
use crate::calendar::paid_replay_test as observed;
use std::sync::atomic::{AtomicBool, Ordering};

const LIMIT: u64 = 16 * 1024 * 1024;
#[derive(Clone, Copy)]
enum Case {
    OrdinaryRefusal, RuleRefusal, ChangedInput, StoredError, FreshError,
    ColdWarm, ColdExact, ColdShort, QueryShort, MoveReborrow, ArithmeticOrder,
    RepeatedErrors, Cumulative,
}
fn day() -> NaiveDate { NaiveDate::from_ymd_opt(2026, 10, 9).unwrap() }
fn terminal() -> ReplayTerminalState {
    super::super::super::target::test_replay_terminal()
}
fn terminal_error(error: ReplayCalendarCallFailure) -> ReplayTerminalFailure {
    match error {
        ReplayCalendarCallFailure::Terminal(failure) => failure,
        other => panic!("expected terminal, got {other:?}"),
    }
}
// Fixed test cases only: no caller amount/limit/callback/pin is exported.
fn fixture(case: Case) {
    let mut metadata = RowsSpecWork::new(LIMIT, 1, 1);
    let mut latch = terminal();
    let mut work = BorrowedReplayWork::test_borrow(&mut metadata, &mut latch);
    let mut payment = CalendarPaymentState::unpaid();
    let cold = cold_request().unwrap();
    match case {
        Case::OrdinaryRefusal => {
            let failure = work.codec_memory().err().unwrap();
            assert!(matches!(failure, ReplayTerminalFailure::CodecQualification(f)
                if f.kind == super::super::ReplayCodecFailureKind::PinUnavailable));
            assert_eq!(work.used(), 0);
            assert_eq!(work.finish(), Err(failure));
        }
        Case::RuleRefusal | Case::ChangedInput | Case::StoredError | Case::FreshError => {
            let result = match case {
                Case::RuleRefusal => super::super::layout_qualification::require_calendar_rules(None).map(|_| ()),
                Case::ChangedInput => observed::changed_input(),
                Case::StoredError => observed::stored_error_guard(),
                Case::FreshError => observed::fresh_error_guard(),
                _ => unreachable!(),
            };
            let reason = match case {
                Case::RuleRefusal => ReplayCalendarQualificationFailure::RuleUnavailable,
                Case::ChangedInput => ReplayCalendarQualificationFailure::InputMismatch,
                _ => ReplayCalendarQualificationFailure::ContradictoryStoredError,
            };
            let failure = checked_eligibility(&mut work, result).unwrap_err();
            assert_eq!(failure, ReplayTerminalFailure::CalendarQualification(reason));
            assert_eq!(work.used(), 0);
            assert_eq!(terminal_error(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())).unwrap_err()), failure);
            assert_eq!(work.used(), 0);
            assert_eq!(observed::counts(), (0, 0, 0));
            drop(work);
            let mut render = BorrowedReplayWork::test_borrow(&mut metadata, &mut latch);
            assert_eq!(render.finish(), Err(failure));
            assert_eq!(render.reserve(ReplaySite::CalendarQuery, QUERY_REQUEST).err(), Some(failure));
        }
        Case::ColdShort | Case::QueryShort => {
            let remaining = if matches!(case, Case::ColdShort) { cold - 1 } else { cold + QUERY_REQUEST - 1 };
            work.reserve(ReplaySite::StateCopy, LIMIT - remaining).unwrap().consume();
            let failure = terminal_error(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())).unwrap_err());
            let site = if matches!(case, Case::ColdShort) { ReplaySite::CalendarCold } else { ReplaySite::CalendarQuery };
            assert_eq!(failure, ReplayTerminalFailure::Resource(ReplayResourceFailure {
                site, cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded), used: LIMIT + 1,
            }));
            assert_eq!(matches!(payment, CalendarPaymentState::Paid), matches!(case, Case::QueryShort));
            assert_eq!(observed::counts(), (0, 0, 0));
            assert!(observed::cold());
            let after = work.used();
            assert_eq!(terminal_error(call_paid(&mut work, &mut payment, CalendarRequest::Next(day())).unwrap_err()), failure);
            assert_eq!(work.used(), after);
            drop(work);
            let mut next = BorrowedReplayWork::test_borrow(&mut metadata, &mut latch);
            let mut new_payment = CalendarPaymentState::unpaid();
            assert_eq!(terminal_error(call_paid(&mut next, &mut new_payment, CalendarRequest::Day(day())).unwrap_err()), failure);
            assert_eq!(next.used(), LIMIT + 1);
            assert_eq!(latch.finish(), Err(failure));
        }
        Case::ColdExact => {
            work.reserve(ReplaySite::StateCopy, LIMIT - cold - QUERY_REQUEST).unwrap().consume();
            assert_eq!(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())), Ok(CalendarResponse::Day(true)));
            assert_eq!(work.used(), LIMIT);
            assert_eq!(observed::counts(), (1, 1, 1));
            let failure = terminal_error(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())).unwrap_err());
            assert_eq!(failure, ReplayTerminalFailure::Resource(ReplayResourceFailure {
                site: ReplaySite::CalendarQuery, cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded), used: LIMIT + QUERY_REQUEST,
            }));
            assert_eq!(observed::counts(), (1, 1, 1));
        }
        Case::ColdWarm | Case::MoveReborrow => {
            assert_eq!(observed::counts(), (0, 0, 0));
            assert!(observed::cold());
            assert_eq!(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())), Ok(CalendarResponse::Day(true)));
            assert_eq!(work.used(), cold + QUERY_REQUEST);
            assert_eq!(observed::counts(), (1, 1, 1));
            let hash = observed::authority_hash().unwrap();
            assert_eq!(hash, crate::calendar::verified_a_share_calendar_authority_hash(day()).unwrap());
            let mut moved = payment;
            {
                let reborrow = &mut work;
                assert_eq!(call_paid(reborrow, &mut moved, CalendarRequest::Next(day())),
                    Ok(CalendarResponse::Date(NaiveDate::from_ymd_opt(2026, 10, 12).unwrap())));
            }
            assert_eq!(work.used(), cold + 2 * QUERY_REQUEST);
            assert_eq!(observed::counts(), (1, 2, 2));
            if matches!(case, Case::MoveReborrow) {
                drop(work);
                let mut render = BorrowedReplayWork::test_borrow(&mut metadata, &mut latch);
                let mut new_payment = CalendarPaymentState::unpaid();
                assert_eq!(call_paid(&mut render, &mut new_payment, CalendarRequest::Prev(day())),
                    Ok(CalendarResponse::Date(crate::calendar::verified_prev_a_share_trading_day(day()).unwrap())));
                assert_eq!(render.used(), 2 * cold + 3 * QUERY_REQUEST);
                render.finish().unwrap();
            }
        }
        Case::ArithmeticOrder => {
            let expected_prev = crate::calendar::verified_prev_a_share_trading_day(NaiveDate::MIN).unwrap_err();
            let expected_next = crate::calendar::verified_next_a_share_trading_day(NaiveDate::MAX).unwrap_err();
            assert!(observed::cold());
            assert_eq!(call_paid(&mut work, &mut payment, CalendarRequest::Prev(NaiveDate::MIN)), Err(ReplayCalendarCallFailure::Historical(expected_prev)));
            assert_eq!(call_paid(&mut work, &mut payment, CalendarRequest::Next(NaiveDate::MAX)), Err(ReplayCalendarCallFailure::Historical(expected_next)));
            assert_eq!(work.used(), cold + 2 * QUERY_REQUEST);
            assert_eq!(observed::counts(), (0, 2, 0));
            assert!(observed::cold());
            assert_eq!(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())), Ok(CalendarResponse::Day(true)));
            assert_eq!(work.used(), cold + 3 * QUERY_REQUEST);
            assert_eq!(observed::counts(), (1, 3, 1));
            work.finish().unwrap();
        }
        Case::RepeatedErrors => {
            let outside = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
            for index in 1..=3 {
                let error = call_paid(&mut work, &mut payment, CalendarRequest::Day(outside)).unwrap_err();
                assert_eq!(error, ReplayCalendarCallFailure::Historical("checked-in A-share trading-calendar coverage unavailable for 2027".to_owned()));
                assert_eq!(work.used(), cold + index * QUERY_REQUEST);
                work.finish().unwrap();
            }
            assert_eq!(observed::counts(), (1, 3, 3));
            for (date, expected) in [(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(), false),
                (NaiveDate::from_ymd_opt(2026, 10, 10).unwrap(), false), (day(), true)] {
                assert_eq!(call_paid(&mut work, &mut payment, CalendarRequest::Day(date)), Ok(CalendarResponse::Day(expected)));
                assert_eq!(crate::calendar::verified_a_share_trading_day(date), Ok(expected));
            }
            assert_eq!(work.used(), cold + 6 * QUERY_REQUEST);
        }
        Case::Cumulative => {
            let mut successes = 0_u64;
            loop {
                match call_paid(&mut work, &mut payment, CalendarRequest::Day(day())) {
                    Ok(CalendarResponse::Day(true)) => successes += 1,
                    Err(error) => {
                        let failure = terminal_error(error);
                        let total = cold + (successes + 1) * QUERY_REQUEST;
                        assert!(total > LIMIT);
                        assert_eq!(work.used(), total);
                        assert_eq!(failure, ReplayTerminalFailure::Resource(ReplayResourceFailure {
                            site: ReplaySite::CalendarQuery, cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded), used: total,
                        }));
                        let counts = observed::counts();
                        assert_eq!(counts, (1, successes as usize, successes as usize));
                        assert_eq!(terminal_error(call_paid(&mut work, &mut payment, CalendarRequest::Day(day())).unwrap_err()), failure);
                        assert_eq!(observed::counts(), counts);
                        assert_eq!(work.used(), total);
                        break;
                    }
                    other => panic!("unexpected result {other:?}"),
                }
            }
        }
    }
}

// The parent may run with unrelated tests. Cold assertions run only in an exact
// fresh child, with no fixture prewarming and no successful qualification token.
fn fresh_child(name: &str) -> bool {
    const FLAG: &str = "TEST_CODE_PAID_CALENDAR_CHILD";
    if std::env::var(FLAG).ok().as_deref() == Some(name) { return true; }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg(format!("database::global_schema_v1::replay_work::paid_calendar::tests::{name}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(FLAG, name).output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed; 0 failed"), "child did not execute its exact case: {stdout}");
    false
}

#[test]
fn request_formula_uses_real_public_layouts_and_fixed_trace() {
    assert_eq!(URL_REQUEST, 190);
    assert_eq!(QUERY_REQUEST, (2 * 61_u64).max(48).max(43));
    assert_eq!(date_node_requests(), Ok(233));
    let (_, internal) = btree_node_bounds::<NaiveDate, ()>().unwrap();
    let thread = calendar_wait_thread_request_upper().unwrap();
    assert_eq!(cold_request().unwrap(), 1240 + 233 * internal.bytes() + thread);
    // Conditional primitive branch, not a guessed private std sizeof.
    if FieldLayout::of::<Box<[u8]>>() == (FieldLayout { size: 16, align: 8 })
        && FieldLayout::of::<NonZeroU64>() == (FieldLayout { size: 8, align: 8 })
        && FieldLayout::of::<*mut c_void>() == (FieldLayout { size: 8, align: 8 })
        && FieldLayout::of::<AtomicUsize>() == (FieldLayout { size: 8, align: 8 })
        && FieldLayout::of::<AtomicI8>() == (FieldLayout { size: 1, align: 1 })
        && FieldLayout::of::<u128>() == (FieldLayout { size: 16, align: 16 }) {
        assert_eq!(thread, 144);
    }
}
#[test]
fn checked_formula_rejects_overflow_address_space_and_alignment() {
    assert_eq!(compose_cold(u64::MAX, 0), Err(LayoutFailure::Overflow));
    assert_eq!(compose_cold(0, u64::MAX), Err(LayoutFailure::Overflow));
    assert_eq!(layout(isize::MAX as u64, 8), Err(LayoutFailure::AddressSpace));
    assert_eq!(layout(1, 3), Err(LayoutFailure::InvalidAlignment));
}

macro_rules! fresh_case {
    ($name:ident, $case:ident) => {
        #[test]
        fn $name() { if fresh_child(stringify!($name)) { fixture(Case::$case); } }
    };
}
fresh_case!(ordinary_build_refuses_without_calendar_or_codec_pin, OrdinaryRefusal);
fresh_case!(absent_calendar_rule_latches_before_payment, RuleRefusal);
fresh_case!(changed_literal_refuses_before_force, ChangedInput);
fresh_case!(stored_parser_error_is_borrowed_and_latched, StoredError);
fresh_case!(fresh_contradiction_guard_never_clones_into_historical_error, FreshError);
fresh_case!(fresh_original_lazy_then_same_loan_warm_query, ColdWarm);
fresh_case!(exact_cold_plus_query_then_next_request_refuses, ColdExact);
fresh_case!(cold_one_short_stops_before_original_initializer, ColdShort);
fresh_case!(query_one_short_retains_cold_debit_without_force, QueryShort);
fresh_case!(move_short_reborrow_and_new_loan_keep_cumulative_pool, MoveReborrow);
fresh_case!(prev_next_arithmetic_errors_precede_original_force, ArithmeticOrder);
fresh_case!(warm_historical_errors_and_closed_weekend_queries_pay_each_time, RepeatedErrors);
fresh_case!(repeated_calendar_calls_exhaust_same_pool_and_latch, Cumulative);

enum Gate { Initializer, BothForces }
fn wait_for(gate: Gate) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let ready = match gate {
            Gate::Initializer => observed::counts().0 == 1,
            Gate::BothForces => observed::counts().2 == 2,
        };
        if ready { break; }
        assert!(std::time::Instant::now() < deadline, "calendar fixture gate timed out");
        std::thread::yield_now();
    }
}
// Test-only finite orchestration never exposes a work/pin/calendar permit.
static WAITER_DONE: AtomicBool = AtomicBool::new(false);
fn contender(denied: bool) -> (u64, Result<CalendarResponse, ReplayCalendarCallFailure>) {
    let mut metadata = RowsSpecWork::new(LIMIT, 1, 1);
    let mut latch = terminal();
    let mut work = BorrowedReplayWork::test_borrow(&mut metadata, &mut latch);
    if denied { work.reserve(ReplaySite::StateCopy, LIMIT - cold_request().unwrap() + 1).unwrap().consume(); }
    let mut payment = CalendarPaymentState::unpaid();
    let result = call_paid(&mut work, &mut payment, CalendarRequest::Day(day()));
    (work.used(), result)
}
#[test]
fn fresh_original_lazy_contenders_each_pay_before_force() {
    if !fresh_child("fresh_original_lazy_contenders_each_pay_before_force") { return; }
    assert!(observed::cold());
    observed::hold_initializer();
    let first = std::thread::spawn(|| contender(false));
    wait_for(Gate::Initializer);
    let second = std::thread::spawn(|| {
        let result = contender(false);
        WAITER_DONE.store(true, Ordering::SeqCst);
        result
    });
    wait_for(Gate::BothForces);
    // Both calls entered the fixed force boundary while the original initializer
    // is held. This is not observation of an unregistered-thread System Arc.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(!WAITER_DONE.load(Ordering::SeqCst));
    observed::release_initializer();
    for result in [first.join().unwrap(), second.join().unwrap()] {
        assert_eq!(result, (cold_request().unwrap() + QUERY_REQUEST, Ok(CalendarResponse::Day(true))));
    }
    assert_eq!(observed::counts(), (1, 2, 2));
}
#[test]
fn short_contender_refuses_while_original_initializer_is_held() {
    if !fresh_child("short_contender_refuses_while_original_initializer_is_held") { return; }
    assert!(observed::cold());
    observed::hold_initializer();
    let first = std::thread::spawn(|| contender(false));
    wait_for(Gate::Initializer);
    let rejected = std::thread::spawn(|| contender(true)).join().unwrap();
    assert_eq!(rejected.0, LIMIT + 1);
    assert!(matches!(rejected.1, Err(ReplayCalendarCallFailure::Terminal(ReplayTerminalFailure::Resource(f))) if f.site == ReplaySite::CalendarCold));
    assert_eq!(observed::counts(), (1, 1, 1));
    observed::release_initializer();
    assert_eq!(first.join().unwrap().1, Ok(CalendarResponse::Day(true)));
}
