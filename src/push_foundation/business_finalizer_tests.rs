use crate::monitor::push_job::{
    CursorDirective, IntentId, ReasonCode, ScheduleDirective, TerminalDisposition, TerminalRefId,
    UtcMicros,
};

use super::business_finalizer::{
    commit_accepted_finalization, commit_accepted_finalization_with_fault,
    prepare_accepted_finalization, AcceptedFinalizationOutcome, AcceptedPreparationOutcome,
    AcceptedPreparationRequest, BusinessFinalizerError, FinalizerFault, FinalizerFence,
    PendingAcceptedFinalization,
};
use super::terminal_authority::{terminal_binding_sha256, AuthorityAttemptBinding};
use super::terminal_authority_tests::{fixture, FakeAuthority, Fixture};
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
