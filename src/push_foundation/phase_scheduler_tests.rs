use crate::monitor::push_job::{
    BusinessDate, CalendarId, CompletionOwnerId, MachineCatalog, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, PhaseEpic, ProducerId,
    ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, SourceContractId, UnitId, UtcMicros,
};

use super::phase_scheduler::{
    CatchUpPolicy, MarketObservation, PhaseSchedule, PhaseScheduler, PhaseSchedulerError,
    ScheduleStatus, ScheduleStep, ScheduleWindow, WindowPosition,
};
use super::reconciler::w14_recovery_barrier_fixture;

const WINDOW_START: i64 = 1_788_739_200_000_000;
const WINDOW_END: i64 = 1_788_740_100_000_000;

fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).expect("TEST_CODE valid UTC micros")
}

fn identity(
    producer: &str,
    unit: &str,
    owner: &str,
    family: &str,
) -> ScheduleOccurrenceIdentityMaterial {
    ScheduleOccurrenceIdentityMaterial::new(
        Namespace::Production,
        UnitId::try_new(unit.to_owned()).expect("TEST_CODE valid unit"),
        ProducerId::try_new(producer.to_owned()).expect("TEST_CODE valid producer"),
        ScheduleOrTriggerId::try_new("p01-0900".to_owned()).expect("TEST_CODE valid schedule"),
        CalendarId::try_new("a-share-sse-2026".to_owned()).expect("TEST_CODE valid calendar"),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE valid business date"),
            OccurrenceFamily::try_new(family.to_owned()).expect("TEST_CODE valid family"),
            OccurrenceKey::try_new("p01:2026-09-07".to_owned())
                .expect("TEST_CODE valid occurrence key"),
        ),
        CompletionOwnerId::try_new(owner.to_owned()).expect("TEST_CODE valid owner"),
        SourceContractId::try_new("p01-source-v1".to_owned())
            .expect("TEST_CODE valid source contract"),
    )
}

fn p01_identity() -> ScheduleOccurrenceIdentityMaterial {
    identity(
        "p01-scheduled",
        "MU-p01",
        "business_date_once_claims(business_date,PreopenNewsHot,None,GLOBAL) → immutable decision / occurrence=p01:{business_date}",
        "p01:{business_date}",
    )
}

fn schedule(policy: CatchUpPolicy) -> PhaseSchedule {
    PhaseSchedule::try_bind(
        &MachineCatalog::bundled().expect("TEST_CODE bundled catalog"),
        p01_identity(),
        PhaseEpic::Preopen,
        ScheduleWindow::try_new(micros(WINDOW_START), micros(WINDOW_END))
            .expect("TEST_CODE valid schedule window"),
        policy,
    )
    .expect("TEST_CODE valid P01 schedule binding")
}

#[test]
fn w14_schedule_window_is_start_inclusive_and_end_exclusive() {
    let window = ScheduleWindow::try_new(micros(WINDOW_START), micros(WINDOW_END))
        .expect("TEST_CODE valid schedule window");

    assert_eq!(
        window.position(micros(WINDOW_START - 1)),
        WindowPosition::Before
    );
    assert_eq!(window.position(micros(WINDOW_START)), WindowPosition::Open);
    assert_eq!(
        window.position(micros(WINDOW_END - 1)),
        WindowPosition::Open
    );
    assert_eq!(window.position(micros(WINDOW_END)), WindowPosition::Expired);
    assert_eq!(
        ScheduleWindow::try_new(micros(WINDOW_START), micros(WINDOW_START)),
        Err(PhaseSchedulerError::InvalidWindow)
    );
    assert_eq!(
        ScheduleWindow::try_new(micros(WINDOW_END), micros(WINDOW_START)),
        Err(PhaseSchedulerError::InvalidWindow)
    );
}

#[test]
fn w14_schedule_binding_exactly_matches_w06_catalog_authority() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    let window = ScheduleWindow::try_new(micros(WINDOW_START), micros(WINDOW_END))
        .expect("TEST_CODE valid schedule window");
    let valid = PhaseSchedule::try_bind(
        &catalog,
        p01_identity(),
        PhaseEpic::Preopen,
        window,
        CatchUpPolicy::SameBusinessDayBeforeDeadline,
    )
    .expect("TEST_CODE exact catalog binding");

    assert_eq!(valid.phase(), PhaseEpic::Preopen);
    assert_eq!(valid.business_date().as_str(), "2026-09-07");
    assert_eq!(valid.window().start(), micros(WINDOW_START));
    assert_eq!(valid.window().end(), micros(WINDOW_END));
    assert_eq!(
        valid.catch_up_policy(),
        CatchUpPolicy::SameBusinessDayBeforeDeadline
    );

    for (candidate, expected) in [
        (
            identity(
                "TEST_CODE_unknown-producer",
                "MU-p01",
                "business_date_once_claims(business_date,PreopenNewsHot,None,GLOBAL) → immutable decision / occurrence=p01:{business_date}",
                "p01:{business_date}",
            ),
            PhaseSchedulerError::CatalogProducerMissing,
        ),
        (
            identity(
                "p01-scheduled",
                "TEST_CODE_wrong-unit",
                "business_date_once_claims(business_date,PreopenNewsHot,None,GLOBAL) → immutable decision / occurrence=p01:{business_date}",
                "p01:{business_date}",
            ),
            PhaseSchedulerError::CatalogMismatch { field: "unit_id" },
        ),
        (
            identity(
                "p01-scheduled",
                "MU-p01",
                "TEST_CODE_wrong-owner",
                "p01:{business_date}",
            ),
            PhaseSchedulerError::CatalogMismatch {
                field: "completion_owner",
            },
        ),
        (
            identity(
                "p01-scheduled",
                "MU-p01",
                "business_date_once_claims(business_date,PreopenNewsHot,None,GLOBAL) → immutable decision / occurrence=p01:{business_date}",
                "TEST_CODE_wrong-family",
            ),
            PhaseSchedulerError::CatalogMismatch {
                field: "occurrence_family",
            },
        ),
    ] {
        assert_eq!(
            PhaseSchedule::try_bind(
                &catalog,
                candidate,
                PhaseEpic::Preopen,
                window,
                CatchUpPolicy::SameBusinessDayBeforeDeadline,
            ),
            Err(expected)
        );
    }

    assert_eq!(
        PhaseSchedule::try_bind(
            &catalog,
            p01_identity(),
            PhaseEpic::Intraday,
            window,
            CatchUpPolicy::SameBusinessDayBeforeDeadline,
        ),
        Err(PhaseSchedulerError::CatalogMismatch { field: "phase" })
    );
}

#[test]
fn w14_non_trading_day_does_not_create_an_occurrence() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let observation = MarketObservation::non_trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
        micros(WINDOW_START),
    );

    assert_eq!(
        PhaseScheduler::tick(&schedule, None, &observation)
            .expect("TEST_CODE non-trading classification"),
        ScheduleStep::NoOccurrence {
            reason: crate::monitor::push_job::ReasonCode::ScheduleNotTradingDay,
        }
    );
}

#[test]
fn w14_recover_persisted_only_never_creates_new_work() {
    let schedule = schedule(CatchUpPolicy::RecoverPersistedOnly);
    let observation = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
        micros(WINDOW_START),
    );

    let step = PhaseScheduler::tick(&schedule, None, &observation)
        .expect("TEST_CODE recovery-only classification");
    assert_eq!(
        step,
        ScheduleStep::RecoveryOnly {
            occurrence_id: schedule.occurrence_id().clone(),
        }
    );
}

#[test]
fn w14_new_occurrence_is_created_expected_then_proposed_eligible() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let observation = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
        micros(WINDOW_START),
    );

    let created = match PhaseScheduler::tick(&schedule, None, &observation)
        .expect("TEST_CODE create Expected")
    {
        ScheduleStep::CreateExpected(snapshot) => snapshot,
        other => panic!("TEST_CODE expected creation, got {other:?}"),
    };
    assert_eq!(created.occurrence_id(), schedule.occurrence_id());
    assert_eq!(created.business_date().as_str(), "2026-09-07");
    assert_eq!(created.status(), ScheduleStatus::Expected);
    assert_eq!(created.version(), 0);
    assert_eq!(
        created.reason(),
        crate::monitor::push_job::ReasonCode::ScheduleWindowOpen
    );
    assert_eq!(created.created_at(), micros(WINDOW_START));
    assert_eq!(created.updated_at(), micros(WINDOW_START));

    let proposal = match PhaseScheduler::tick(&schedule, Some(&created), &observation)
        .expect("TEST_CODE propose eligibility")
    {
        ScheduleStep::TransitionProposal(proposal) => proposal,
        other => panic!("TEST_CODE expected transition, got {other:?}"),
    };
    assert_eq!(proposal.occurrence_id(), schedule.occurrence_id());
    assert_eq!(proposal.from_status(), ScheduleStatus::Expected);
    assert_eq!(proposal.to_status(), ScheduleStatus::Eligible);
    assert_eq!(proposal.expected_version(), 0);
    assert_eq!(proposal.result_version(), 1);
    assert_eq!(
        proposal.reason(),
        crate::monitor::push_job::ReasonCode::ScheduleWindowOpen
    );
    assert_eq!(proposal.observed_at(), micros(WINDOW_START));
}

#[test]
fn w14_normal_tick_and_startup_catch_up_are_exactly_replayable() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let observation = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
        micros(WINDOW_START + 1),
    );
    let barrier = w14_recovery_barrier_fixture();

    let normal_create =
        PhaseScheduler::tick(&schedule, None, &observation).expect("TEST_CODE normal creation");
    let catch_up_create = PhaseScheduler::startup_catch_up(&barrier, &schedule, None, &observation)
        .expect("TEST_CODE catch-up creation");
    assert_eq!(normal_create, catch_up_create);

    let created = match normal_create {
        ScheduleStep::CreateExpected(snapshot) => snapshot,
        other => panic!("TEST_CODE expected creation, got {other:?}"),
    };
    let normal_transition = PhaseScheduler::tick(&schedule, Some(&created), &observation)
        .expect("TEST_CODE normal transition");
    let catch_up_transition =
        PhaseScheduler::startup_catch_up(&barrier, &schedule, Some(&created), &observation)
            .expect("TEST_CODE catch-up transition");
    assert_eq!(normal_transition, catch_up_transition);
    assert!(matches!(
        normal_transition,
        ScheduleStep::TransitionProposal(_)
    ));
}

#[test]
fn w14_late_catch_up_keeps_original_business_date_and_rejects_date_drift() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let initial = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid original date"),
        micros(WINDOW_START),
    );
    let created = match PhaseScheduler::tick(&schedule, None, &initial)
        .expect("TEST_CODE create original occurrence")
    {
        ScheduleStep::CreateExpected(snapshot) => snapshot,
        other => panic!("TEST_CODE expected creation, got {other:?}"),
    };
    let barrier = w14_recovery_barrier_fixture();
    let late = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original business date"),
        micros(WINDOW_END + 86_400_000_000),
    );

    let first = PhaseScheduler::startup_catch_up(&barrier, &schedule, Some(&created), &late)
        .expect("TEST_CODE late catch-up");
    let replay = PhaseScheduler::startup_catch_up(&barrier, &schedule, Some(&created), &late)
        .expect("TEST_CODE deterministic replay");
    assert_eq!(first, replay);
    let proposal = match first {
        ScheduleStep::TransitionProposal(proposal) => proposal,
        other => panic!("TEST_CODE expected Missed proposal, got {other:?}"),
    };
    assert_eq!(proposal.occurrence_id(), schedule.occurrence_id());
    assert_eq!(proposal.to_status(), ScheduleStatus::Missed);
    assert_eq!(created.business_date().as_str(), "2026-09-07");

    let drifted = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-08").expect("TEST_CODE different business date"),
        micros(WINDOW_END + 86_400_000_000),
    );
    assert_eq!(
        PhaseScheduler::startup_catch_up(&barrier, &schedule, Some(&created), &drifted),
        Err(PhaseSchedulerError::BusinessDateMismatch)
    );
}
