use crate::monitor::push_job::{
    derive_occurrence_id, evaluate_completion, w09_completion_policy_fixture, AuthorityClass,
    BusinessDate, CalendarId, CompletionFact, CompletionOwnerId, MachineCatalog, Namespace,
    OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, PhaseEpic, ProducerId, ReasonCode,
    ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest, SourceContractId,
    UnitId, UtcMicros, VerifiedEmptyEvidenceRef,
};

use super::phase_scheduler::{
    CatchUpPolicy, MarketObservation, NextEligibleSessionRef, PhaseSchedule, PhaseScheduler,
    PhaseSchedulerError, ScheduleOccurrenceSnapshot, ScheduleStatus, ScheduleStep, ScheduleWindow,
    WindowPosition,
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

fn created(schedule: &PhaseSchedule, observed_at: i64) -> ScheduleOccurrenceSnapshot {
    let observation = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
        micros(observed_at),
    );
    match PhaseScheduler::tick(schedule, None, &observation).expect("TEST_CODE create Expected") {
        ScheduleStep::CreateExpected(snapshot) => snapshot,
        other => panic!("TEST_CODE expected creation, got {other:?}"),
    }
}

fn hydrated(
    schedule: &PhaseSchedule,
    status: ScheduleStatus,
    version: u64,
    reason: ReasonCode,
    next: Option<NextEligibleSessionRef>,
) -> ScheduleOccurrenceSnapshot {
    ScheduleOccurrenceSnapshot::try_hydrate(
        schedule.clone(),
        status,
        version,
        reason,
        micros(WINDOW_START),
        micros(WINDOW_START + 1),
        next,
    )
    .expect("TEST_CODE valid hydrated occurrence")
}

fn close_directive() -> crate::monitor::push_job::CompletionDirective {
    let policy = w09_completion_policy_fixture(
        "MU-p01",
        "business_date_once_claims(business_date,PreopenNewsHot,None,GLOBAL) → immutable decision / occurrence=p01:{business_date}",
        vec![AuthorityClass::GenericCounted],
    );
    let occurrence = derive_occurrence_id(&OccurrenceIdentityMaterial::new(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
        OccurrenceFamily::try_new("p01:{business_date}".to_owned())
            .expect("TEST_CODE valid family"),
        OccurrenceKey::try_new("p01:2026-09-07".to_owned()).expect("TEST_CODE valid key"),
    ));
    let empty = VerifiedEmptyEvidenceRef::new(
        occurrence,
        SourceContractId::try_new("p01-source-v1".to_owned()).expect("TEST_CODE valid source"),
        Sha256Digest::parse("TEST_CODE evidence", &"e".repeat(64)).expect("TEST_CODE valid digest"),
        micros(WINDOW_END),
    );
    evaluate_completion(&policy, CompletionFact::VerifiedNoData(&empty))
        .expect("TEST_CODE close completion directive")
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
fn w14_non_trading_authority_preserves_existing_occurrences_without_eligibility() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let current = created(&schedule, WINDOW_START);
    let barrier = w14_recovery_barrier_fixture();
    for observed_at in [WINDOW_START + 1, WINDOW_END] {
        let observation = MarketObservation::non_trading_day(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
            micros(observed_at),
        );
        let expected = ScheduleStep::NoChange {
            occurrence_id: current.occurrence_id().clone(),
            status: ScheduleStatus::Expected,
            version: 0,
            reason: ReasonCode::ScheduleNotTradingDay,
        };
        assert_eq!(
            PhaseScheduler::tick(&schedule, Some(&current), &observation)
                .expect("TEST_CODE preserve existing non-trading fact"),
            expected,
        );
        assert_eq!(
            PhaseScheduler::startup_catch_up(&barrier, &schedule, Some(&current), &observation)
                .expect("TEST_CODE non-trading recovery does not authorize new work"),
            expected,
        );
    }
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

#[test]
fn w14_expire_and_same_day_policies_use_the_exact_end_boundary() {
    for policy in [
        CatchUpPolicy::ExpireWithoutCatchUp,
        CatchUpPolicy::SameBusinessDayBeforeDeadline,
    ] {
        let schedule = schedule(policy);
        let occurrence = created(&schedule, WINDOW_START);
        let before_end = MarketObservation::trading_day(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
            micros(WINDOW_END - 1),
        );
        let at_end = MarketObservation::trading_day(
            BusinessDate::parse("2026-09-07").expect("TEST_CODE valid date"),
            micros(WINDOW_END),
        );

        let eligible = match PhaseScheduler::tick(&schedule, Some(&occurrence), &before_end)
            .expect("TEST_CODE end minus one remains open")
        {
            ScheduleStep::TransitionProposal(proposal) => proposal,
            other => panic!("TEST_CODE expected Eligible proposal, got {other:?}"),
        };
        assert_eq!(eligible.to_status(), ScheduleStatus::Eligible);

        let missed = match PhaseScheduler::tick(&schedule, Some(&occurrence), &at_end)
            .expect("TEST_CODE exact end is expired")
        {
            ScheduleStep::TransitionProposal(proposal) => proposal,
            other => panic!("TEST_CODE expected Missed proposal, got {other:?}"),
        };
        assert_eq!(missed.to_status(), ScheduleStatus::Missed);
        assert_eq!(missed.reason(), ReasonCode::ScheduleWindowExpired);
    }
}

#[test]
fn w14_deferred_occurrence_resumes_in_next_half_open_session_without_new_identity() {
    let schedule = schedule(CatchUpPolicy::DeferToNextEligibleSession);
    let occurrence = created(&schedule, WINDOW_START);
    let next_window = ScheduleWindow::try_new(
        micros(WINDOW_END + 86_400_000_000),
        micros(WINDOW_END + 86_400_900_000),
    )
    .expect("TEST_CODE next window");
    let next = NextEligibleSessionRef::try_new(
        &schedule,
        BusinessDate::parse("2026-09-08").expect("TEST_CODE next business date"),
        next_window,
    )
    .expect("TEST_CODE next session reference");
    assert_eq!(next.business_date().as_str(), "2026-09-08");
    assert_eq!(next.window(), next_window);
    let expired = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        micros(WINDOW_END),
    )
    .with_next_eligible(next.clone());

    let deferred_proposal = match PhaseScheduler::tick(&schedule, Some(&occurrence), &expired)
        .expect("TEST_CODE defer expired occurrence")
    {
        ScheduleStep::TransitionProposal(proposal) => proposal,
        other => panic!("TEST_CODE expected Deferred proposal, got {other:?}"),
    };
    assert_eq!(deferred_proposal.to_status(), ScheduleStatus::Deferred);
    assert_eq!(deferred_proposal.next_eligible(), Some(&next));
    let deferred = occurrence
        .apply_proposal(&deferred_proposal)
        .expect("TEST_CODE apply deferred proposal");
    assert_eq!(deferred.status(), ScheduleStatus::Deferred);
    assert_eq!(deferred.occurrence_id(), occurrence.occurrence_id());
    assert_eq!(deferred.business_date().as_str(), "2026-09-07");
    assert_eq!(deferred.next_eligible(), Some(&next));

    let before_next = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        micros(next_window.start().get() - 1),
    );
    assert!(matches!(
        PhaseScheduler::tick(&schedule, Some(&deferred), &before_next)
            .expect("TEST_CODE wait for next session"),
        ScheduleStep::NoChange {
            status: ScheduleStatus::Deferred,
            reason: ReasonCode::ScheduleWindowNotOpen,
            ..
        }
    ));

    let next_start = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        next_window.start(),
    );
    let eligible = match PhaseScheduler::tick(&schedule, Some(&deferred), &next_start)
        .expect("TEST_CODE recover deferred occurrence")
    {
        ScheduleStep::TransitionProposal(proposal) => proposal,
        other => panic!("TEST_CODE expected Eligible proposal, got {other:?}"),
    };
    assert_eq!(eligible.from_status(), ScheduleStatus::Deferred);
    assert_eq!(eligible.to_status(), ScheduleStatus::Eligible);
    assert_eq!(eligible.occurrence_id(), occurrence.occurrence_id());
}

#[test]
fn w14_blocked_and_prepared_states_are_not_reopened_by_time_ticks() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let blocked = hydrated(
        &schedule,
        ScheduleStatus::BlockedOnInput,
        2,
        ReasonCode::InputSourceUnavailable,
        None,
    );
    let open = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        micros(WINDOW_START + 2),
    );
    assert!(matches!(
        PhaseScheduler::tick(&schedule, Some(&blocked), &open)
            .expect("TEST_CODE blocked remains blocked"),
        ScheduleStep::NoChange {
            status: ScheduleStatus::BlockedOnInput,
            reason: ReasonCode::InputSourceUnavailable,
            ..
        }
    ));

    let expired = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        micros(WINDOW_END),
    );
    let missed = match PhaseScheduler::tick(&schedule, Some(&blocked), &expired)
        .expect("TEST_CODE blocked expiration")
    {
        ScheduleStep::TransitionProposal(proposal) => proposal,
        other => panic!("TEST_CODE expected Missed proposal, got {other:?}"),
    };
    assert_eq!(missed.to_status(), ScheduleStatus::Missed);

    let prepared = hydrated(
        &schedule,
        ScheduleStatus::Prepared,
        3,
        ReasonCode::IntentCreated,
        None,
    );
    assert!(matches!(
        PhaseScheduler::tick(&schedule, Some(&prepared), &expired)
            .expect("TEST_CODE prepared remains prepared"),
        ScheduleStep::NoChange {
            status: ScheduleStatus::Prepared,
            reason: ReasonCode::IntentCreated,
            ..
        }
    ));
}

#[test]
fn w14_completion_closes_prepared_once_and_closed_never_reopens() {
    let schedule = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let prepared = hydrated(
        &schedule,
        ScheduleStatus::Prepared,
        3,
        ReasonCode::IntentCreated,
        None,
    );
    let directive = close_directive();
    let close = match PhaseScheduler::completion(&prepared, directive, micros(WINDOW_END))
        .expect("TEST_CODE close prepared occurrence")
    {
        ScheduleStep::TransitionProposal(proposal) => proposal,
        other => panic!("TEST_CODE expected close proposal, got {other:?}"),
    };
    assert_eq!(close.from_status(), ScheduleStatus::Prepared);
    assert_eq!(close.to_status(), ScheduleStatus::Closed);
    assert_eq!(close.reason(), ReasonCode::ScheduleOccurrenceClosed);
    let closed = prepared
        .apply_proposal(&close)
        .expect("TEST_CODE apply close proposal");

    let rewound = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        micros(WINDOW_START - 1),
    );
    let barrier = w14_recovery_barrier_fixture();
    for step in [
        PhaseScheduler::tick(&schedule, Some(&closed), &rewound)
            .expect("TEST_CODE closed normal tick"),
        PhaseScheduler::startup_catch_up(&barrier, &schedule, Some(&closed), &rewound)
            .expect("TEST_CODE closed catch-up"),
        PhaseScheduler::completion(&closed, directive, micros(WINDOW_END + 1))
            .expect("TEST_CODE repeated completion"),
    ] {
        assert!(matches!(
            step,
            ScheduleStep::NoChange {
                status: ScheduleStatus::Closed,
                reason: ReasonCode::ScheduleOccurrenceClosed,
                ..
            }
        ));
    }
}

#[test]
fn w14_invalid_next_snapshot_binding_and_version_overflow_fail_closed() {
    let defer_schedule = schedule(CatchUpPolicy::DeferToNextEligibleSession);
    let invalid_next_window =
        ScheduleWindow::try_new(micros(WINDOW_START + 1), micros(WINDOW_END + 1))
            .expect("TEST_CODE structurally valid but overlapping window");
    assert_eq!(
        NextEligibleSessionRef::try_new(
            &defer_schedule,
            BusinessDate::parse("2026-09-07").expect("TEST_CODE same date"),
            invalid_next_window,
        ),
        Err(PhaseSchedulerError::InvalidNextEligibleSession)
    );
    assert_eq!(
        ScheduleOccurrenceSnapshot::try_hydrate(
            defer_schedule.clone(),
            ScheduleStatus::Deferred,
            1,
            ReasonCode::ScheduleDeferred,
            micros(WINDOW_START),
            micros(WINDOW_START + 1),
            None,
        ),
        Err(PhaseSchedulerError::NextEligibleSessionRequired)
    );
    assert_eq!(
        ScheduleOccurrenceSnapshot::try_hydrate(
            defer_schedule.clone(),
            ScheduleStatus::Expected,
            0,
            ReasonCode::ScheduleWindowNotOpen,
            micros(WINDOW_START + 1),
            micros(WINDOW_START),
            None,
        ),
        Err(PhaseSchedulerError::InvalidSnapshot {
            check: "updated_at_before_created_at",
        })
    );

    let overflow = ScheduleOccurrenceSnapshot::try_hydrate(
        defer_schedule.clone(),
        ScheduleStatus::Expected,
        u64::MAX,
        ReasonCode::ScheduleWindowOpen,
        micros(WINDOW_START),
        micros(WINDOW_START),
        None,
    )
    .expect("TEST_CODE overflow fixture");
    let open = MarketObservation::trading_day(
        BusinessDate::parse("2026-09-07").expect("TEST_CODE original date"),
        micros(WINDOW_START),
    );
    assert_eq!(
        PhaseScheduler::tick(&defer_schedule, Some(&overflow), &open),
        Err(PhaseSchedulerError::VersionOverflow)
    );

    let other = schedule(CatchUpPolicy::SameBusinessDayBeforeDeadline);
    let current = created(&defer_schedule, WINDOW_START);
    assert_eq!(
        PhaseScheduler::tick(&other, Some(&current), &open),
        Err(PhaseSchedulerError::OccurrenceBindingMismatch)
    );
}
