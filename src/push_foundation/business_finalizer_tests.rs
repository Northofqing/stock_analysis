use crate::monitor::push_job::{
    CursorDirective, DecisionId, IntentId, ReasonCode, RetryEligibility, ScheduleDirective,
    Sha256Digest, TerminalDisposition, TerminalRefId, UtcMicros,
};

use super::business_finalizer::{
    commit_accepted_finalization, commit_accepted_finalization_with_fault,
    commit_not_delivered_finalization, prepare_accepted_finalization,
    prepare_not_delivered_finalization, AcceptedFinalizationOutcome, AcceptedPreparationOutcome,
    AcceptedPreparationRequest, BusinessFinalizerError, FinalizerFault, FinalizerFence,
    NotDeliveredFinalizationOutcome, NotDeliveredPreparationOutcome,
    NotDeliveredPreparationRequest, PendingAcceptedFinalization, VerifiedOperatorAuditRef,
    VerifiedResolutionClearance,
};
use super::terminal_authority::{
    terminal_binding_sha256, AuthorityAttemptBinding, AuthorityQuery, TerminalAuthorityError,
};
use super::terminal_authority_tests::{digest, fixture, FakeAuthority, Fixture};
use super::{
    BusinessIntentStore, IntentSnapshot, IntentState, IntentStoreError, IntentTransitionCommand,
    LeaseAction, LeaseOwnerId, TransitionActor,
};

const DISPATCH_AT: i64 = 1_788_743_101_000_000;
const QUALIFY_VERIFIED_AT: i64 = 1_788_743_102_000_000;
const QUALIFY_AT: i64 = 1_788_743_103_000_000;
const FINAL_VERIFIED_AT: i64 = 1_788_743_104_000_000;
const FINAL_AT: i64 = 1_788_743_105_000_000;
const LEASE_UNTIL: i64 = 1_788_743_400_000_000;

fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}

fn dispatch(fixture: &Fixture, store: &mut BusinessIntentStore) -> IntentSnapshot {
    let command = IntentTransitionCommand::try_new(
        fixture.record.intent_id.clone(),
        IntentState::PendingDispatch,
        IntentState::AwaitingAuthority,
        fixture.snapshot.version(),
        TransitionActor::try_new("dispatcher-1".to_owned()).unwrap(),
        ReasonCode::IntentDispatchClaimed,
        micros(DISPATCH_AT),
        LeaseAction::Acquire {
            owner: LeaseOwnerId::try_new("finalizer-1".to_owned()).unwrap(),
            until: micros(LEASE_UNTIL),
        },
    )
    .unwrap();
    store.apply_nonterminal_transition(&command).unwrap();
    store.inspect(&fixture.record.intent_id).unwrap().unwrap()
}

fn fence(snapshot: &IntentSnapshot) -> FinalizerFence {
    FinalizerFence::new(
        LeaseOwnerId::try_new(snapshot.lease_owner().unwrap().to_owned()).unwrap(),
        snapshot.lease_generation(),
        snapshot.lease_until().unwrap(),
    )
}

fn request(snapshot: &IntentSnapshot, intent_id: IntentId) -> AcceptedPreparationRequest {
    AcceptedPreparationRequest::new(
        intent_id,
        snapshot.version(),
        TransitionActor::try_new("finalizer-1".to_owned()).unwrap(),
        fence(snapshot),
        micros(QUALIFY_VERIFIED_AT),
        micros(QUALIFY_AT),
    )
    .unwrap()
}

fn prepare_pending(
    store: &mut BusinessIntentStore,
    fixture: &Fixture,
    authority: &FakeAuthority,
    snapshot: &IntentSnapshot,
) -> PendingAcceptedFinalization {
    match prepare_accepted_finalization(
        store,
        request(snapshot, fixture.record.intent_id.clone()),
        &fixture.template,
        &fixture.policy,
        authority,
    )
    .unwrap()
    {
        AcceptedPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected pending finalization, got {other:?}"),
    }
}

fn isolate_uncertain(
    fixture: &Fixture,
    store: &mut BusinessIntentStore,
    awaiting: &IntentSnapshot,
) -> IntentSnapshot {
    let command = IntentTransitionCommand::try_new(
        fixture.record.intent_id.clone(),
        IntentState::AwaitingAuthority,
        IntentState::ResolutionRequired,
        awaiting.version(),
        TransitionActor::try_new("transport-1".to_owned()).unwrap(),
        ReasonCode::TransportUncertain,
        micros(QUALIFY_AT),
        LeaseAction::Preserve,
    )
    .unwrap();
    store.apply_nonterminal_transition(&command).unwrap();
    store.inspect(&fixture.record.intent_id).unwrap().unwrap()
}

fn not_delivered_fixture() -> Fixture {
    let mut fixture = fixture();
    fixture.record.terminal_disposition = TerminalDisposition::ManualConfirmedNotDelivered;
    fixture.record.attempt_binding = AuthorityAttemptBinding::ValidatedManualWithoutAttempt;
    fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
    fixture
}

fn audit(
    fixture: &Fixture,
    snapshot: &IntentSnapshot,
    sha256: Sha256Digest,
) -> VerifiedOperatorAuditRef {
    VerifiedOperatorAuditRef::for_test(
        fixture.record.intent_id.clone(),
        fixture.record.decision_id.clone(),
        snapshot.version(),
        "operator-audit-42".to_owned(),
        sha256,
    )
    .unwrap()
}

#[test]
fn w10_accepted_requires_two_queries_and_commits_one_bound_terminal_event() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);

    let pending = match prepare_accepted_finalization(
        &mut store,
        request(&awaiting, fixture.record.intent_id.clone()),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        AcceptedPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected pending finalization, got {other:?}"),
    };
    assert_eq!(authority.calls.get(), 1);
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(qualified.state(), IntentState::AwaitingFinalizer);
    assert_eq!(qualified.version(), awaiting.version() + 1);
    assert_eq!(qualified.lease_owner(), Some("finalizer-1"));

    let outcome = commit_accepted_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    let (receipt, directive) = match outcome {
        AcceptedFinalizationOutcome::Applied { receipt, directive } => (receipt, directive),
        other => panic!("expected applied completion, got {other:?}"),
    };

    assert_eq!(authority.calls.get(), 2);
    assert_eq!(receipt.from_state(), IntentState::AwaitingFinalizer);
    assert_eq!(receipt.to_state(), IntentState::Completed);
    assert_eq!(receipt.reason(), ReasonCode::FinalizerCompleted);
    assert_eq!(
        receipt.terminal_ref_id(),
        Some(fixture.record.ref_id.as_str())
    );
    assert_eq!(
        receipt.terminal_binding_sha256(),
        Some(&fixture.record.binding_sha256)
    );
    assert_eq!(
        receipt.terminal_disposition(),
        Some(TerminalDisposition::Accepted)
    );
    assert_eq!(directive.schedule(), ScheduleDirective::CloseOnAccepted);
    assert_eq!(directive.cursor(), CursorDirective::AdvanceAccepted);

    let completed = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(completed.state(), IntentState::Completed);
    assert_eq!(completed.version(), awaiting.version() + 2);
    assert_eq!(completed.lease_owner(), None);
    assert_eq!(completed.lease_until(), None);
    let chain = store
        .inspect_transition_chain(&fixture.record.intent_id)
        .unwrap();
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[1].to_state(), IntentState::AwaitingFinalizer);
    assert_eq!(chain[2], receipt);
}

#[test]
fn w10_manual_acceptance_stays_distinct_but_can_complete_when_policy_allows_it() {
    let mut fixture = fixture();
    fixture.record.terminal_disposition = TerminalDisposition::ManualConfirmedAccepted;
    fixture.record.attempt_binding = AuthorityAttemptBinding::ValidatedManualWithoutAttempt;
    fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let pending = match prepare_accepted_finalization(
        &mut store,
        request(&awaiting, fixture.record.intent_id.clone()),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        AcceptedPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected pending manual acceptance, got {other:?}"),
    };

    let outcome = commit_accepted_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    let (receipt, directive) = match outcome {
        AcceptedFinalizationOutcome::Applied { receipt, directive } => (receipt, directive),
        other => panic!("expected applied manual acceptance, got {other:?}"),
    };
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(
        receipt.terminal_disposition(),
        Some(TerminalDisposition::ManualConfirmedAccepted)
    );
    assert_eq!(directive.cursor(), CursorDirective::AdvanceManualAccepted);
}

#[test]
fn w10_rejected_uncertain_and_not_delivered_cannot_enter_accepted_finalizer() {
    for disposition in [
        TerminalDisposition::Rejected,
        TerminalDisposition::Uncertain,
        TerminalDisposition::ManualConfirmedNotDelivered,
    ] {
        let mut fixture = fixture();
        fixture.record.terminal_disposition = disposition;
        if disposition == TerminalDisposition::ManualConfirmedNotDelivered {
            fixture.record.attempt_binding = AuthorityAttemptBinding::ValidatedManualWithoutAttempt;
        }
        fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
        let authority = FakeAuthority::terminal(fixture.record.clone());
        let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
        let awaiting = dispatch(&fixture, &mut store);

        assert!(matches!(
            prepare_accepted_finalization(
                &mut store,
                request(&awaiting, fixture.record.intent_id.clone()),
                &fixture.template,
                &fixture.policy,
                &authority,
            ),
            Err(BusinessFinalizerError::DispositionNotCompletable { actual }) if actual == disposition
        ));
        assert_eq!(authority.calls.get(), 1);
        let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
        assert_eq!(current.state(), IntentState::AwaitingAuthority);
        assert_eq!(current.version(), awaiting.version());
        assert_eq!(
            store
                .inspect_transition_chain(&fixture.record.intent_id)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn w10_stale_finalizer_cas_is_reread_and_isolated_as_resolution_required() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let pending = prepare_pending(&mut store, &fixture, &authority, &awaiting);
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();

    let competitor = IntentTransitionCommand::try_new(
        fixture.record.intent_id.clone(),
        IntentState::AwaitingFinalizer,
        IntentState::AwaitingFinalizer,
        qualified.version(),
        TransitionActor::try_new("finalizer-competitor".to_owned()).unwrap(),
        ReasonCode::IntentLeaseHeld,
        micros(FINAL_VERIFIED_AT),
        LeaseAction::Preserve,
    )
    .unwrap();
    store.apply_nonterminal_transition(&competitor).unwrap();

    let outcome = commit_accepted_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    let receipt = match outcome {
        AcceptedFinalizationOutcome::ResolutionRequired { receipt } => receipt,
        other => panic!("expected conflict isolation, got {other:?}"),
    };
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(receipt.from_state(), IntentState::AwaitingFinalizer);
    assert_eq!(receipt.to_state(), IntentState::ResolutionRequired);
    assert_eq!(receipt.reason(), ReasonCode::FinalizerCasConflict);
    assert_eq!(
        store
            .inspect(&fixture.record.intent_id)
            .unwrap()
            .unwrap()
            .state(),
        IntentState::ResolutionRequired
    );
}

#[test]
fn w10_identical_competing_finalize_returns_the_only_completed_event() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let first = prepare_pending(&mut store, &fixture, &authority, &awaiting);
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    let second = prepare_pending(&mut store, &fixture, &authority, &qualified);

    let first_receipt = match commit_accepted_finalization(
        &mut store,
        first,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap()
    {
        AcceptedFinalizationOutcome::Applied { receipt, .. } => receipt,
        other => panic!("expected first completion, got {other:?}"),
    };
    let second_receipt = match commit_accepted_finalization(
        &mut store,
        second,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap()
    {
        AcceptedFinalizationOutcome::AlreadyCommitted { receipt, .. } => receipt,
        other => panic!("expected idempotent completion, got {other:?}"),
    };

    assert_eq!(first_receipt, second_receipt);
    assert_eq!(authority.calls.get(), 3);
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .iter()
            .filter(|event| event.to_state() == IntentState::Completed)
            .count(),
        1
    );
}

#[test]
fn w10_same_version_with_different_terminal_is_conflict_not_idempotency() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut changed_record = fixture.record.clone();
    changed_record.ref_id = TerminalRefId::try_new("disposition-conflict".to_owned()).unwrap();
    changed_record.binding_sha256 = terminal_binding_sha256(&changed_record);
    let conflicting_authority = FakeAuthority::terminal(changed_record);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let first = prepare_pending(&mut store, &fixture, &authority, &awaiting);
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    let conflicting = prepare_pending(&mut store, &fixture, &conflicting_authority, &qualified);

    commit_accepted_finalization(
        &mut store,
        first,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    let outcome = commit_accepted_finalization(
        &mut store,
        conflicting,
        &fixture.template,
        &fixture.policy,
        &conflicting_authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        AcceptedFinalizationOutcome::ResolutionRequired { .. }
    ));
    assert_eq!(conflicting_authority.calls.get(), 1);
    let chain = store
        .inspect_transition_chain(&fixture.record.intent_id)
        .unwrap();
    assert!(chain
        .iter()
        .any(|event| event.to_state() == IntentState::Completed));
    assert_eq!(
        chain.last().unwrap().to_state(),
        IntentState::ResolutionRequired
    );
}

#[test]
fn w10_terminal_cas_and_append_faults_roll_back_the_complete_transaction() {
    for fault in [FinalizerFault::AfterCas, FinalizerFault::AfterAppend] {
        let fixture = fixture();
        let authority = FakeAuthority::terminal(fixture.record.clone());
        let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
        let awaiting = dispatch(&fixture, &mut store);
        let pending = prepare_pending(&mut store, &fixture, &authority, &awaiting);
        let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
        let chain_before = store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap();

        assert!(matches!(
            commit_accepted_finalization_with_fault(
                &mut store,
                pending,
                &fixture.template,
                &fixture.policy,
                &authority,
                micros(FINAL_VERIFIED_AT),
                micros(FINAL_AT),
                fault,
            ),
            Err(BusinessFinalizerError::Store(
                IntentStoreError::InjectedFault { .. }
            ))
        ));
        let after = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
        assert_eq!(after.state(), IntentState::AwaitingFinalizer);
        assert_eq!(after.version(), qualified.version());
        assert_eq!(
            store
                .inspect_transition_chain(&fixture.record.intent_id)
                .unwrap(),
            chain_before
        );
    }
}

#[test]
fn w10_commit_ack_loss_recovers_the_existing_completion_without_another_query() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let retry_request = request(&awaiting, fixture.record.intent_id.clone());
    let pending = prepare_pending(&mut store, &fixture, &authority, &awaiting);

    assert!(matches!(
        commit_accepted_finalization_with_fault(
            &mut store,
            pending,
            &fixture.template,
            &fixture.policy,
            &authority,
            micros(FINAL_VERIFIED_AT),
            micros(FINAL_AT),
            FinalizerFault::AfterCommitAckLost,
        ),
        Err(BusinessFinalizerError::Store(
            IntentStoreError::InjectedFault {
                point: "after_transition_commit_ack_lost"
            }
        ))
    ));
    assert_eq!(authority.calls.get(), 2);

    let receipt = match prepare_accepted_finalization(
        &mut store,
        retry_request,
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        AcceptedPreparationOutcome::AlreadyFinalized(receipt) => receipt,
        other => panic!("expected recovered completion, got {other:?}"),
    };
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(receipt.to_state(), IntentState::Completed);
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn w10_resolution_requires_exact_clearance_before_accepted_qualification() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let resolution = isolate_uncertain(&fixture, &mut store, &awaiting);
    let chain = store
        .inspect_transition_chain(&fixture.record.intent_id)
        .unwrap();
    let conflict_sha256 = chain.last().unwrap().canonical_sha256().clone();

    assert!(matches!(
        prepare_accepted_finalization(
            &mut store,
            request(&resolution, fixture.record.intent_id.clone()),
            &fixture.template,
            &fixture.policy,
            &authority,
        ),
        Err(BusinessFinalizerError::ResolutionClearanceRequired)
    ));
    assert_eq!(authority.calls.get(), 0);

    let stale = VerifiedResolutionClearance::for_test(
        fixture.record.intent_id.clone(),
        resolution.version() - 1,
        conflict_sha256,
    );
    assert!(matches!(
        prepare_accepted_finalization(
            &mut store,
            request(&resolution, fixture.record.intent_id.clone()).with_resolution_clearance(stale),
            &fixture.template,
            &fixture.policy,
            &authority,
        ),
        Err(BusinessFinalizerError::ResolutionClearanceMismatch)
    ));
    assert_eq!(authority.calls.get(), 0);
    assert_eq!(
        store
            .inspect(&fixture.record.intent_id)
            .unwrap()
            .unwrap()
            .state(),
        IntentState::ResolutionRequired
    );
}

#[test]
fn w10_exact_resolution_clearance_is_consumed_by_two_query_accepted_flow() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let resolution = isolate_uncertain(&fixture, &mut store, &awaiting);
    let conflict_sha256 = store
        .inspect_transition_chain(&fixture.record.intent_id)
        .unwrap()
        .last()
        .unwrap()
        .canonical_sha256()
        .clone();
    let clearance = VerifiedResolutionClearance::for_test(
        fixture.record.intent_id.clone(),
        resolution.version(),
        conflict_sha256,
    );
    let pending = match prepare_accepted_finalization(
        &mut store,
        request(&resolution, fixture.record.intent_id.clone()).with_resolution_clearance(clearance),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        AcceptedPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected cleared pending finalization, got {other:?}"),
    };
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(qualified.state(), IntentState::AwaitingFinalizer);
    assert_eq!(
        qualified.previous_state(),
        Some(IntentState::ResolutionRequired)
    );

    let outcome = commit_accepted_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        AcceptedFinalizationOutcome::Applied { .. }
    ));
    assert_eq!(authority.calls.get(), 2);
}

#[test]
fn w10_not_delivered_requires_two_queries_and_persists_the_independent_audit() {
    let fixture = not_delivered_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let audit_sha256 = digest('9');
    let pending = match prepare_not_delivered_finalization(
        &mut store,
        NotDeliveredPreparationRequest::new(
            fixture.record.intent_id.clone(),
            awaiting.version(),
            TransitionActor::try_new("operator-1".to_owned()).unwrap(),
            fence(&awaiting),
            micros(QUALIFY_VERIFIED_AT),
            audit(&fixture, &awaiting, audit_sha256.clone()),
        )
        .unwrap(),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        NotDeliveredPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected pending not-delivered finalization, got {other:?}"),
    };
    let outcome = commit_not_delivered_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    let (receipt, directive) = match outcome {
        NotDeliveredFinalizationOutcome::Applied { receipt, directive } => (receipt, directive),
        other => panic!("expected applied not-delivered, got {other:?}"),
    };
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(receipt.to_state(), IntentState::NotDelivered);
    assert_eq!(receipt.reason(), ReasonCode::OperatorNotDelivered);
    assert_eq!(
        receipt.terminal_decision_id(),
        Some(fixture.record.decision_id.as_str())
    );
    assert_eq!(receipt.operator_audit_ref(), Some("operator-audit-42"));
    assert_eq!(receipt.operator_audit_sha256(), Some(&audit_sha256));
    assert_eq!(directive.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(directive.cursor(), CursorDirective::Never);
    assert_eq!(directive.retry().reason(), ReasonCode::OperatorNotDelivered);
    assert_eq!(directive.retry().eligibility(), RetryEligibility::Never);
    assert_eq!(
        store
            .inspect(&fixture.record.intent_id)
            .unwrap()
            .unwrap()
            .lease_owner(),
        None
    );
}

#[test]
fn w10_not_delivered_allows_only_the_latest_uncertain_resolution_origin() {
    let fixture = not_delivered_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let resolution = isolate_uncertain(&fixture, &mut store, &awaiting);
    let pending = match prepare_not_delivered_finalization(
        &mut store,
        NotDeliveredPreparationRequest::new(
            fixture.record.intent_id.clone(),
            resolution.version(),
            TransitionActor::try_new("operator-1".to_owned()).unwrap(),
            fence(&resolution),
            micros(QUALIFY_VERIFIED_AT),
            audit(&fixture, &resolution, digest('8')),
        )
        .unwrap(),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        NotDeliveredPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected eligible resolution pending, got {other:?}"),
    };
    let outcome = commit_not_delivered_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        NotDeliveredFinalizationOutcome::Applied { .. }
    ));
    assert_eq!(authority.calls.get(), 2);
}

#[test]
fn w10_duplicate_not_delivered_returns_one_terminal_event_without_a_third_requery() {
    let fixture = not_delivered_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let build_request = || {
        NotDeliveredPreparationRequest::new(
            fixture.record.intent_id.clone(),
            awaiting.version(),
            TransitionActor::try_new("operator-1".to_owned()).unwrap(),
            fence(&awaiting),
            micros(QUALIFY_VERIFIED_AT),
            audit(&fixture, &awaiting, digest('2')),
        )
        .unwrap()
    };
    let first = match prepare_not_delivered_finalization(
        &mut store,
        build_request(),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        NotDeliveredPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected first pending not-delivered, got {other:?}"),
    };
    let second = match prepare_not_delivered_finalization(
        &mut store,
        build_request(),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap()
    {
        NotDeliveredPreparationOutcome::Pending(pending) => pending,
        other => panic!("expected second pending not-delivered, got {other:?}"),
    };
    let first_receipt = match commit_not_delivered_finalization(
        &mut store,
        first,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap()
    {
        NotDeliveredFinalizationOutcome::Applied { receipt, .. } => receipt,
        other => panic!("expected applied not-delivered, got {other:?}"),
    };
    let second_receipt = match commit_not_delivered_finalization(
        &mut store,
        second,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap()
    {
        NotDeliveredFinalizationOutcome::AlreadyCommitted { receipt, .. } => receipt,
        other => panic!("expected idempotent not-delivered, got {other:?}"),
    };
    assert_eq!(first_receipt, second_receipt);
    assert_eq!(authority.calls.get(), 3);
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .iter()
            .filter(|event| event.to_state() == IntentState::NotDelivered)
            .count(),
        1
    );
}

#[test]
fn w10_not_delivered_rejects_missing_or_replayed_audit_before_authority_query() {
    let fixture = not_delivered_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);

    let missing = NotDeliveredPreparationRequest::without_audit_for_test(
        fixture.record.intent_id.clone(),
        awaiting.version(),
        TransitionActor::try_new("operator-1".to_owned()).unwrap(),
        fence(&awaiting),
        micros(QUALIFY_VERIFIED_AT),
    );
    assert!(matches!(
        prepare_not_delivered_finalization(
            &mut store,
            missing,
            &fixture.template,
            &fixture.policy,
            &authority,
        ),
        Err(BusinessFinalizerError::OperatorAuditRequired)
    ));

    let replayed = VerifiedOperatorAuditRef::for_test(
        fixture.record.intent_id.clone(),
        fixture.record.decision_id.clone(),
        awaiting.version() + 1,
        "operator-audit-replayed".to_owned(),
        digest('7'),
    )
    .unwrap();
    let request = NotDeliveredPreparationRequest::new(
        fixture.record.intent_id.clone(),
        awaiting.version(),
        TransitionActor::try_new("operator-1".to_owned()).unwrap(),
        fence(&awaiting),
        micros(QUALIFY_VERIFIED_AT),
        replayed,
    )
    .unwrap();
    assert!(matches!(
        prepare_not_delivered_finalization(
            &mut store,
            request,
            &fixture.template,
            &fixture.policy,
            &authority,
        ),
        Err(BusinessFinalizerError::OperatorAuditMismatch)
    ));

    let wrong_decision = VerifiedOperatorAuditRef::for_test(
        fixture.record.intent_id.clone(),
        DecisionId::try_new("0".repeat(64)).unwrap(),
        awaiting.version(),
        "operator-audit-wrong-decision".to_owned(),
        digest('4'),
    )
    .unwrap();
    let request = NotDeliveredPreparationRequest::new(
        fixture.record.intent_id.clone(),
        awaiting.version(),
        TransitionActor::try_new("operator-1".to_owned()).unwrap(),
        fence(&awaiting),
        micros(QUALIFY_VERIFIED_AT),
        wrong_decision,
    )
    .unwrap();
    assert!(matches!(
        prepare_not_delivered_finalization(
            &mut store,
            request,
            &fixture.template,
            &fixture.policy,
            &authority,
        ),
        Err(BusinessFinalizerError::OperatorAuditMismatch)
    ));
    assert_eq!(authority.calls.get(), 0);
}

#[test]
fn w10_not_delivered_rejects_accepted_history_and_wrong_disposition_without_writes() {
    let mut accepted_fixture = fixture();
    accepted_fixture.record.terminal_disposition = TerminalDisposition::ManualConfirmedAccepted;
    accepted_fixture.record.attempt_binding =
        AuthorityAttemptBinding::ValidatedManualWithoutAttempt;
    accepted_fixture.record.binding_sha256 = terminal_binding_sha256(&accepted_fixture.record);
    let accepted_authority = FakeAuthority::terminal(accepted_fixture.record.clone());
    let mut store = BusinessIntentStore::open(&accepted_fixture.database).unwrap();
    let awaiting = dispatch(&accepted_fixture, &mut store);
    let accepted_pending = prepare_pending(
        &mut store,
        &accepted_fixture,
        &accepted_authority,
        &awaiting,
    );
    let qualified = store
        .inspect(&accepted_fixture.record.intent_id)
        .unwrap()
        .unwrap();

    let mut not_delivered_record = accepted_fixture.record.clone();
    not_delivered_record.terminal_disposition = TerminalDisposition::ManualConfirmedNotDelivered;
    not_delivered_record.binding_sha256 = terminal_binding_sha256(&not_delivered_record);
    let not_delivered_authority = FakeAuthority::terminal(not_delivered_record);
    let request = NotDeliveredPreparationRequest::new(
        accepted_fixture.record.intent_id.clone(),
        qualified.version(),
        TransitionActor::try_new("operator-1".to_owned()).unwrap(),
        fence(&qualified),
        micros(FINAL_VERIFIED_AT),
        audit(&accepted_fixture, &qualified, digest('6')),
    )
    .unwrap();
    assert!(matches!(
        prepare_not_delivered_finalization(
            &mut store,
            request,
            &accepted_fixture.template,
            &accepted_fixture.policy,
            &not_delivered_authority,
        ),
        Err(BusinessFinalizerError::NotDeliveredHistoryIneligible)
    ));
    assert_eq!(not_delivered_authority.calls.get(), 0);

    let completed_fence = fence(&qualified);
    commit_accepted_finalization(
        &mut store,
        accepted_pending,
        &accepted_fixture.template,
        &accepted_fixture.policy,
        &accepted_authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    )
    .unwrap();
    let completed = store
        .inspect(&accepted_fixture.record.intent_id)
        .unwrap()
        .unwrap();
    let request = NotDeliveredPreparationRequest::new(
        accepted_fixture.record.intent_id.clone(),
        completed.version(),
        TransitionActor::try_new("operator-1".to_owned()).unwrap(),
        completed_fence,
        micros(FINAL_AT),
        audit(&accepted_fixture, &completed, digest('3')),
    )
    .unwrap();
    assert!(matches!(
        prepare_not_delivered_finalization(
            &mut store,
            request,
            &accepted_fixture.template,
            &accepted_fixture.policy,
            &not_delivered_authority,
        ),
        Err(BusinessFinalizerError::NotDeliveredHistoryIneligible)
    ));
    assert_eq!(not_delivered_authority.calls.get(), 0);

    let other_fixture = fixture();
    let authority = FakeAuthority::terminal(other_fixture.record.clone());
    let mut other_store = BusinessIntentStore::open(&other_fixture.database).unwrap();
    let other_awaiting = dispatch(&other_fixture, &mut other_store);
    let request = NotDeliveredPreparationRequest::new(
        other_fixture.record.intent_id.clone(),
        other_awaiting.version(),
        TransitionActor::try_new("operator-1".to_owned()).unwrap(),
        fence(&other_awaiting),
        micros(QUALIFY_VERIFIED_AT),
        audit(&other_fixture, &other_awaiting, digest('5')),
    )
    .unwrap();
    assert!(matches!(
        prepare_not_delivered_finalization(
            &mut other_store,
            request,
            &other_fixture.template,
            &other_fixture.policy,
            &authority,
        ),
        Err(BusinessFinalizerError::DispositionNotCompletable {
            actual: TerminalDisposition::Accepted
        })
    ));
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(
        other_store
            .inspect(&other_fixture.record.intent_id)
            .unwrap()
            .unwrap()
            .state(),
        IntentState::AwaitingAuthority
    );
}

#[test]
fn w10_failed_final_requery_appends_nonterminal_invalid_evidence() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let pending = prepare_pending(&mut store, &fixture, &authority, &awaiting);
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    *authority.result.borrow_mut() = Ok(AuthorityQuery::PendingSeal);

    let receipt = match commit_accepted_finalization(
        &mut store,
        pending,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(FINAL_VERIFIED_AT),
        micros(FINAL_AT),
    ) {
        Err(BusinessFinalizerError::TerminalInvalid {
            source: TerminalAuthorityError::TerminalPendingSeal,
            receipt,
        }) => receipt,
        other => panic!("expected persisted terminal-invalid result, got {other:?}"),
    };
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(receipt.from_state(), IntentState::AwaitingFinalizer);
    assert_eq!(receipt.to_state(), IntentState::AwaitingFinalizer);
    assert_eq!(receipt.reason(), ReasonCode::FinalizerTerminalRefInvalid);
    assert_eq!(receipt.terminal_disposition(), None);
    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::AwaitingFinalizer);
    assert_eq!(current.version(), qualified.version() + 1);
}

#[test]
fn w10_existing_resolution_is_reported_without_another_isolation_write() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let awaiting = dispatch(&fixture, &mut store);
    let pending = prepare_pending(&mut store, &fixture, &authority, &awaiting);
    let qualified = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    let isolate = IntentTransitionCommand::try_new(
        fixture.record.intent_id.clone(),
        IntentState::AwaitingFinalizer,
        IntentState::ResolutionRequired,
        qualified.version(),
        TransitionActor::try_new("operator-2".to_owned()).unwrap(),
        ReasonCode::OperatorResolutionConflict,
        micros(FINAL_VERIFIED_AT),
        LeaseAction::Preserve,
    )
    .unwrap();
    store.apply_nonterminal_transition(&isolate).unwrap();
    let isolated = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    let chain_len = store
        .inspect_transition_chain(&fixture.record.intent_id)
        .unwrap()
        .len();

    assert!(matches!(
        commit_accepted_finalization(
            &mut store,
            pending,
            &fixture.template,
            &fixture.policy,
            &authority,
            micros(FINAL_VERIFIED_AT),
            micros(FINAL_AT),
        ),
        Err(BusinessFinalizerError::ConflictUnresolved { current })
            if *current == isolated
    ));
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .len(),
        chain_len
    );
}
