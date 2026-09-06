use crate::monitor::push_job::{
    CursorDirective, IntentId, ReasonCode, ScheduleDirective, TerminalDisposition, UtcMicros,
};

use super::business_finalizer::{
    commit_accepted_finalization, prepare_accepted_finalization, AcceptedFinalizationOutcome,
    AcceptedPreparationOutcome, AcceptedPreparationRequest, BusinessFinalizerError, FinalizerFence,
};
use super::terminal_authority::{terminal_binding_sha256, AuthorityAttemptBinding};
use super::terminal_authority_tests::{fixture, FakeAuthority, Fixture};
use super::{
    BusinessIntentStore, IntentSnapshot, IntentState, IntentTransitionCommand, LeaseAction,
    LeaseOwnerId, TransitionActor,
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
