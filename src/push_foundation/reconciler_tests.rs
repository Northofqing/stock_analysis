use std::cell::{Cell, RefCell};

use crate::monitor::push_job::{
    AudienceId, BusinessDate, CompletionOwnerId, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, ReasonCode, Sha256Digest, SourceContractId,
    SubjectId, TerminalDisposition, UnitId, UtcMicros,
};

use super::business_finalizer::{
    prepare_accepted_finalization, AcceptedPreparationOutcome, AcceptedPreparationRequest,
    FinalizerFence,
};
use super::intent_store::AttestedReadyIntent;
use super::reconciler::{
    reconcile_startup, RecoveryBindingError, RecoveryBindings, RecoveryBindingsPort,
    RecoveryBoundary, RecoveryConfig, RecoveryError,
};
use super::terminal_authority::{
    terminal_binding_sha256, AuthorityAttemptBinding, AuthorityQuery, AuthorityQueryFailure,
};
use super::terminal_authority_tests::{fixture as terminal_fixture, FakeAuthority, Fixture};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    IntentSnapshot, IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId,
    TransitionActor,
};

const CREATED_AT: i64 = 1_788_700_000_000_000;
const FIRST_CLAIM_AT: i64 = 1_788_700_100_000_000;
const RECOVERY_AT: i64 = 1_788_700_300_000_000;
const RECOVERY_UNTIL: i64 = 1_788_700_600_000_000;

fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse("w11 fixture", &byte.to_string().repeat(64)).unwrap()
}

struct RecoveryFixture {
    _root: tempfile::TempDir,
    store: BusinessIntentStore,
}

impl RecoveryFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("business.sqlite3");
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&database)
            .unwrap();
        let store = BusinessIntentStore::open(&database).unwrap();
        Self { _root: root, store }
    }

    fn insert_ready(
        &mut self,
        business_date: &str,
        occurrence_key: &str,
        created_at: i64,
    ) -> IntentSnapshot {
        insert_ready_into_store(&mut self.store, business_date, occurrence_key, created_at)
    }
}

fn insert_ready_into_store(
    store: &mut BusinessIntentStore,
    business_date: &str,
    occurrence_key: &str,
    created_at: i64,
) -> IntentSnapshot {
    let identity = InitialIntentIdentity::new(
        Namespace::Production,
        UnitId::try_new("MU-auction".to_owned()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse(business_date).unwrap(),
            OccurrenceFamily::try_new("auction-session".to_owned()).unwrap(),
            OccurrenceKey::try_new(occurrence_key.to_owned()).unwrap(),
        ),
        CompletionOwnerId::try_new("owner-auction".to_owned()).unwrap(),
        SourceContractId::try_new("auction-source".to_owned()).unwrap(),
        SubjectId::entity(format!("{occurrence_key}.SZ")).unwrap(),
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    );
    let draft = InitialIntentDraft::ready_for_recovery_test(
        identity,
        format!("prepared:{business_date}:{occurrence_key}").into_bytes(),
        format!("rendered:{business_date}:{occurrence_key}").into_bytes(),
        digest('e'),
        digest('f'),
        micros(created_at),
    )
    .unwrap();
    let intent_id = draft.intent_id().clone();
    store.record_initial(&draft).unwrap();
    store.inspect(&intent_id).unwrap().unwrap()
}

struct NoBindings {
    calls: Cell<usize>,
}

impl NoBindings {
    fn new() -> Self {
        Self {
            calls: Cell::new(0),
        }
    }
}

impl RecoveryBindingsPort for NoBindings {
    fn resolve<'a>(
        &'a self,
        _intent: &AttestedReadyIntent,
    ) -> Result<RecoveryBindings<'a>, RecoveryBindingError> {
        self.calls.set(self.calls.get() + 1);
        Err(RecoveryBindingError::Unavailable)
    }
}

fn config(owner: &str, page_size: usize) -> RecoveryConfig {
    config_at(owner, page_size, RECOVERY_AT, RECOVERY_UNTIL, 8)
}

fn config_at(
    owner: &str,
    page_size: usize,
    now: i64,
    lease_until: i64,
    max_iterations: usize,
) -> RecoveryConfig {
    RecoveryConfig::try_new(
        LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
        TransitionActor::try_new(owner.to_owned()).unwrap(),
        micros(now),
        micros(lease_until),
        page_size,
        max_iterations,
    )
    .unwrap()
}

fn acquire(
    store: &mut BusinessIntentStore,
    snapshot: &IntentSnapshot,
    owner: &str,
    at: i64,
    until: i64,
) -> IntentSnapshot {
    let intent = snapshot.attested_ready_binding().unwrap();
    let command = IntentTransitionCommand::try_new(
        intent.intent_id.clone(),
        snapshot.state(),
        snapshot.state(),
        snapshot.version(),
        TransitionActor::try_new(owner.to_owned()).unwrap(),
        ReasonCode::IntentDispatchClaimed,
        micros(at),
        LeaseAction::Acquire {
            owner: LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
            until: micros(until),
        },
    )
    .unwrap();
    store.apply_nonterminal_transition(&command).unwrap();
    store
        .inspect(&intent.intent_id)
        .unwrap()
        .expect("claimed intent remains readable")
}

struct StaticBindings<'a> {
    fixture: &'a Fixture,
    authority: &'a FakeAuthority,
    calls: Cell<usize>,
}

impl<'a> StaticBindings<'a> {
    fn new(fixture: &'a Fixture, authority: &'a FakeAuthority) -> Self {
        Self {
            fixture,
            authority,
            calls: Cell::new(0),
        }
    }
}

impl RecoveryBindingsPort for StaticBindings<'_> {
    fn resolve<'a>(
        &'a self,
        intent: &AttestedReadyIntent,
    ) -> Result<RecoveryBindings<'a>, RecoveryBindingError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(intent.unit_id, self.fixture.record.unit_id);
        assert_eq!(intent.intent_id, self.fixture.record.intent_id);
        Ok(RecoveryBindings::new(
            &self.fixture.template,
            &self.fixture.policy,
            self.authority,
        ))
    }
}

struct RenewFenceOnceBindings<'a> {
    fixture: &'a Fixture,
    authority: &'a FakeAuthority,
    competitor: RefCell<BusinessIntentStore>,
    owner: LeaseOwnerId,
    renewed_until: UtcMicros,
    renewed_at: UtcMicros,
    calls: Cell<usize>,
}

impl RecoveryBindingsPort for RenewFenceOnceBindings<'_> {
    fn resolve<'a>(
        &'a self,
        intent: &AttestedReadyIntent,
    ) -> Result<RecoveryBindings<'a>, RecoveryBindingError> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 0 {
            let mut competitor = self.competitor.borrow_mut();
            let current = competitor
                .inspect(&intent.intent_id)
                .unwrap()
                .expect("competitor sees the recovery intent");
            let command = IntentTransitionCommand::try_new(
                intent.intent_id.clone(),
                current.state(),
                current.state(),
                current.version(),
                TransitionActor::try_new("same-owner-winner".to_owned()).unwrap(),
                ReasonCode::IntentDispatchClaimed,
                self.renewed_at,
                LeaseAction::Acquire {
                    owner: self.owner.clone(),
                    until: self.renewed_until,
                },
            )
            .unwrap();
            competitor.apply_nonterminal_transition(&command).unwrap();
        }
        Ok(RecoveryBindings::new(
            &self.fixture.template,
            &self.fixture.policy,
            self.authority,
        ))
    }
}

fn dispatch_terminal_fixture(
    fixture: &Fixture,
    store: &mut BusinessIntentStore,
    owner: &str,
    at: i64,
    until: i64,
) -> IntentSnapshot {
    let command = IntentTransitionCommand::try_new(
        fixture.record.intent_id.clone(),
        IntentState::PendingDispatch,
        IntentState::AwaitingAuthority,
        fixture.snapshot.version(),
        TransitionActor::try_new(owner.to_owned()).unwrap(),
        ReasonCode::IntentDispatchClaimed,
        micros(at),
        LeaseAction::Acquire {
            owner: LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
            until: micros(until),
        },
    )
    .unwrap();
    store.apply_nonterminal_transition(&command).unwrap();
    store.inspect(&fixture.record.intent_id).unwrap().unwrap()
}

fn authority_times() -> (i64, i64, i64) {
    (
        1_788_743_101_000_000,
        1_788_743_110_000_000,
        1_788_743_600_000_000,
    )
}

#[test]
fn w11_scans_every_business_date_with_keyset_pages_and_never_dispatches() {
    let mut fixture = RecoveryFixture::new();
    let oldest = fixture.insert_ready("2026-08-31", "000001", CREATED_AT);
    let middle = fixture.insert_ready("2026-09-03", "000002", CREATED_AT + 1);
    let newest = fixture.insert_ready("2026-09-07", "000003", CREATED_AT + 2);
    let bindings = NoBindings::new();

    let report = reconcile_startup(&mut fixture.store, &config("recovery-1", 1), &bindings)
        .expect("all-date local recovery reaches a fixed point");

    assert_eq!(report.iterations(), 2);
    assert_eq!(report.transition_count(), 3);
    assert_eq!(report.entries().len(), 3);
    assert_eq!(
        report
            .entries()
            .iter()
            .map(|entry| (entry.business_date(), entry.intent_id(), entry.boundary()))
            .collect::<Vec<_>>(),
        vec![
            (
                "2026-08-31",
                oldest.intent_id(),
                RecoveryBoundary::DispatchPending,
            ),
            (
                "2026-09-03",
                middle.intent_id(),
                RecoveryBoundary::DispatchPending,
            ),
            (
                "2026-09-07",
                newest.intent_id(),
                RecoveryBoundary::DispatchPending,
            ),
        ]
    );
    assert_eq!(
        bindings.calls.get(),
        0,
        "PendingDispatch never queries authority"
    );
    for original in [&oldest, &middle, &newest] {
        let current = fixture
            .store
            .inspect(&original.attested_ready_binding().unwrap().intent_id)
            .unwrap()
            .unwrap();
        assert_eq!(current.state(), IntentState::PendingDispatch);
        assert_eq!(current.lease_owner(), Some("recovery-1"));
        assert_eq!(current.lease_generation(), 1);
        assert_eq!(
            fixture
                .store
                .inspect_transition_chain(&original.attested_ready_binding().unwrap().intent_id)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn w11_reclaims_only_missing_or_expired_lease_and_preserves_live_fences() {
    let mut fixture = RecoveryFixture::new();
    let expired = fixture.insert_ready("2026-09-01", "000011", CREATED_AT);
    let foreign = fixture.insert_ready("2026-09-02", "000012", CREATED_AT + 1);
    let same_owner = fixture.insert_ready("2026-09-03", "000013", CREATED_AT + 2);
    let expired = acquire(
        &mut fixture.store,
        &expired,
        "old-owner",
        FIRST_CLAIM_AT,
        RECOVERY_AT,
    );
    let foreign = acquire(
        &mut fixture.store,
        &foreign,
        "foreign-owner",
        FIRST_CLAIM_AT,
        RECOVERY_UNTIL + 10,
    );
    let same_owner = acquire(
        &mut fixture.store,
        &same_owner,
        "recovery-1",
        FIRST_CLAIM_AT,
        RECOVERY_UNTIL + 20,
    );
    let bindings = NoBindings::new();

    let report = reconcile_startup(&mut fixture.store, &config("recovery-1", 2), &bindings)
        .expect("lease recovery reaches fixed point");

    let expired_current = fixture
        .store
        .inspect(&expired.attested_ready_binding().unwrap().intent_id)
        .unwrap()
        .unwrap();
    assert_eq!(expired_current.lease_owner(), Some("recovery-1"));
    assert_eq!(expired_current.lease_until(), Some(micros(RECOVERY_UNTIL)));
    assert_eq!(expired_current.lease_generation(), 2);
    assert_eq!(expired_current.version(), expired.version() + 1);

    let foreign_current = fixture
        .store
        .inspect(&foreign.attested_ready_binding().unwrap().intent_id)
        .unwrap()
        .unwrap();
    assert_eq!(foreign_current, foreign);
    assert_eq!(
        report
            .entry(foreign.intent_id())
            .expect("foreign boundary is reported")
            .boundary(),
        RecoveryBoundary::LiveForeignLease
    );

    let same_owner_current = fixture
        .store
        .inspect(&same_owner.attested_ready_binding().unwrap().intent_id)
        .unwrap()
        .unwrap();
    assert_eq!(same_owner_current, same_owner);
    assert_eq!(
        report
            .entry(same_owner.intent_id())
            .expect("same owner boundary is reported")
            .lease_generation(),
        1
    );
    assert_eq!(report.transition_count(), 1);
    assert_eq!(bindings.calls.get(), 0);
}

#[test]
fn w11_reconciles_accepted_authority_to_completed_with_no_dispatch_seam() {
    let fixture = terminal_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let bindings = StaticBindings::new(&fixture, &authority);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    let awaiting = dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "recovery-accepted",
        dispatch_at,
        lease_until,
    );

    let report = reconcile_startup(
        &mut store,
        &config_at("recovery-accepted", 1, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("accepted terminal reaches business completion");

    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::Completed);
    assert_eq!(current.version(), awaiting.version() + 2);
    assert_eq!(current.lease_owner(), None);
    assert_eq!(authority.calls.get(), 2, "W10 owns both exact queries");
    assert_eq!(bindings.calls.get(), 1);
    assert_eq!(report.transition_count(), 2);
    assert_eq!(
        report.entry(current.intent_id()).unwrap().boundary(),
        RecoveryBoundary::Finalized
    );
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn w11_restart_from_awaiting_finalizer_requeries_instead_of_reusing_memory() {
    let fixture = terminal_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    let awaiting = dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "recovery-finalizer",
        dispatch_at,
        lease_until,
    );
    let pending = prepare_accepted_finalization(
        &mut store,
        AcceptedPreparationRequest::new(
            fixture.record.intent_id.clone(),
            awaiting.version(),
            TransitionActor::try_new("recovery-finalizer".to_owned()).unwrap(),
            FinalizerFence::new(
                LeaseOwnerId::try_new("recovery-finalizer".to_owned()).unwrap(),
                awaiting.lease_generation(),
                awaiting.lease_until().unwrap(),
            ),
            micros(dispatch_at + 1),
            micros(dispatch_at + 2),
        )
        .unwrap(),
        &fixture.template,
        &fixture.policy,
        &authority,
    )
    .unwrap();
    assert!(matches!(pending, AcceptedPreparationOutcome::Pending(_)));
    assert_eq!(authority.calls.replace(0), 1);
    let bindings = StaticBindings::new(&fixture, &authority);

    let report = reconcile_startup(
        &mut store,
        &config_at("recovery-finalizer", 2, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("restart reconstitutes finalization from persisted facts");

    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::Completed);
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(report.transition_count(), 1);
    assert_eq!(
        report.entry(current.intent_id()).unwrap().boundary(),
        RecoveryBoundary::Finalized
    );
}

#[test]
fn w11_rejected_is_recorded_once_and_never_becomes_retry_permission() {
    let mut fixture = terminal_fixture();
    fixture.record.terminal_disposition = TerminalDisposition::Rejected;
    fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let bindings = StaticBindings::new(&fixture, &authority);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    let awaiting = dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "recovery-rejected",
        dispatch_at,
        lease_until,
    );

    let report = reconcile_startup(
        &mut store,
        &config_at("recovery-rejected", 1, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("rejected reaches a stable authorization boundary");

    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::AwaitingAuthority);
    assert_eq!(current.reason(), ReasonCode::TransportRejected);
    assert_eq!(current.version(), awaiting.version() + 1);
    assert_eq!(report.transition_count(), 1);
    assert_eq!(authority.calls.get(), 2, "second pass is read-only");
    assert_eq!(
        report.entry(current.intent_id()).unwrap().boundary(),
        RecoveryBoundary::RejectedAuthorizationRequired
    );
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn w11_uncertain_is_quarantined_and_not_queried_after_resolution_required() {
    let mut fixture = terminal_fixture();
    fixture.record.terminal_disposition = TerminalDisposition::Uncertain;
    fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let bindings = StaticBindings::new(&fixture, &authority);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "recovery-uncertain",
        dispatch_at,
        lease_until,
    );

    let report = reconcile_startup(
        &mut store,
        &config_at("recovery-uncertain", 2, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("uncertain is isolated without resend");

    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::ResolutionRequired);
    assert_eq!(current.reason(), ReasonCode::TransportUncertain);
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(bindings.calls.get(), 1);
    assert_eq!(report.transition_count(), 1);
    assert_eq!(
        report.entry(current.intent_id()).unwrap().boundary(),
        RecoveryBoundary::ManualResolutionRequired
    );
}

#[test]
fn w11_not_delivered_terminal_stays_pending_without_operator_audit() {
    let mut fixture = terminal_fixture();
    fixture.record.terminal_disposition = TerminalDisposition::ManualConfirmedNotDelivered;
    fixture.record.attempt_binding = AuthorityAttemptBinding::ValidatedManualWithoutAttempt;
    fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let bindings = StaticBindings::new(&fixture, &authority);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    let awaiting = dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "recovery-not-delivered",
        dispatch_at,
        lease_until,
    );

    let report = reconcile_startup(
        &mut store,
        &config_at("recovery-not-delivered", 2, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("manual not-delivered waits for W18 audit capability");

    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current, awaiting);
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(report.transition_count(), 0);
    assert_eq!(
        report.entry(current.intent_id()).unwrap().boundary(),
        RecoveryBoundary::OperatorAuditRequired
    );
}

#[test]
fn w11_authority_blocker_appends_only_one_event_before_fixed_point() {
    for query in [
        Ok(AuthorityQuery::Missing),
        Ok(AuthorityQuery::PendingSeal),
        Err(AuthorityQueryFailure),
    ] {
        let fixture = terminal_fixture();
        let authority = FakeAuthority::terminal(fixture.record.clone());
        *authority.result.borrow_mut() = query;
        let bindings = StaticBindings::new(&fixture, &authority);
        let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
        let (dispatch_at, recovery_at, lease_until) = authority_times();
        let awaiting = dispatch_terminal_fixture(
            &fixture,
            &mut store,
            "recovery-blocked",
            dispatch_at,
            lease_until,
        );

        let report = reconcile_startup(
            &mut store,
            &config_at("recovery-blocked", 1, recovery_at, lease_until, 8),
            &bindings,
        )
        .expect("stable authority blocker reaches fixed point");

        let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
        assert_eq!(current.state(), IntentState::AwaitingAuthority);
        assert_eq!(current.reason(), ReasonCode::FinalizerTerminalRefInvalid);
        assert_eq!(current.version(), awaiting.version() + 1);
        assert_eq!(report.transition_count(), 1);
        assert_eq!(authority.calls.get(), 2);
        assert_eq!(
            report.entry(current.intent_id()).unwrap().boundary(),
            RecoveryBoundary::AuthorityBlocked
        );
        assert_eq!(
            store
                .inspect_transition_chain(&fixture.record.intent_id)
                .unwrap()
                .len(),
            2
        );
    }
}

#[test]
fn w11_keyset_scan_retains_mixed_date_boundaries_while_an_intent_completes() {
    let fixture = terminal_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let bindings = StaticBindings::new(&fixture, &authority);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let older = insert_ready_into_store(
        &mut store,
        "2026-09-01",
        "older-pending",
        1_788_600_000_000_000,
    );
    let newer = insert_ready_into_store(
        &mut store,
        "2026-09-08",
        "newer-pending",
        1_788_743_100_000_001,
    );
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "mixed-recovery",
        dispatch_at,
        lease_until,
    );

    let report = reconcile_startup(
        &mut store,
        &config_at("mixed-recovery", 1, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("keyset scan survives state removal and reaches a fixed point");

    assert_eq!(report.iterations(), 2);
    assert_eq!(report.transition_count(), 4);
    assert_eq!(report.entries().len(), 3);
    assert_eq!(
        report
            .entries()
            .iter()
            .map(|entry| (entry.business_date(), entry.boundary()))
            .collect::<Vec<_>>(),
        vec![
            ("2026-09-01", RecoveryBoundary::DispatchPending),
            ("2026-09-07", RecoveryBoundary::Finalized),
            ("2026-09-08", RecoveryBoundary::DispatchPending),
        ]
    );
    assert_eq!(authority.calls.get(), 2);
    assert_eq!(bindings.calls.get(), 1);
    for pending in [&older, &newer] {
        assert_eq!(
            store
                .inspect(&pending.attested_ready_binding().unwrap().intent_id)
                .unwrap()
                .unwrap()
                .state(),
            IntentState::PendingDispatch
        );
    }
    let debug = format!("{report:?}");
    assert!(!debug.contains("prepared:"));
    assert!(!debug.contains("rendered:"));
    assert!(!debug.contains("000001.SZ"));
}

#[test]
fn w11_same_owner_fence_renewal_is_reread_before_authority_use() {
    let fixture = terminal_fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    let awaiting = dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "race-recovery",
        dispatch_at,
        lease_until,
    );
    let renewed_until = micros(lease_until + 10_000);
    let bindings = RenewFenceOnceBindings {
        fixture: &fixture,
        authority: &authority,
        competitor: RefCell::new(BusinessIntentStore::open(&fixture.database).unwrap()),
        owner: LeaseOwnerId::try_new("race-recovery".to_owned()).unwrap(),
        renewed_until,
        renewed_at: micros(recovery_at),
        calls: Cell::new(0),
    };

    let report = reconcile_startup(
        &mut store,
        &config_at("race-recovery", 1, recovery_at, lease_until, 8),
        &bindings,
    )
    .expect("a stale in-memory fence is reread and reevaluated");

    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.state(), IntentState::Completed);
    assert_eq!(current.version(), awaiting.version() + 3);
    assert_eq!(current.lease_generation(), awaiting.lease_generation() + 1);
    assert_eq!(current.lease_owner(), None);
    assert_eq!(bindings.calls.get(), 2);
    assert_eq!(
        authority.calls.get(),
        2,
        "the stale fence performs no query"
    );
    assert_eq!(report.iterations(), 3);
    assert_eq!(
        report.transition_count(),
        2,
        "the competing write is not ours"
    );
    assert_eq!(
        report.entry(current.intent_id()).unwrap().boundary(),
        RecoveryBoundary::Finalized
    );
}

#[test]
fn w11_progress_at_iteration_cap_fails_closed_instead_of_claiming_fixed_point() {
    let mut fixture = terminal_fixture();
    fixture.record.terminal_disposition = TerminalDisposition::Rejected;
    fixture.record.binding_sha256 = terminal_binding_sha256(&fixture.record);
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let bindings = StaticBindings::new(&fixture, &authority);
    let mut store = BusinessIntentStore::open(&fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    dispatch_terminal_fixture(
        &fixture,
        &mut store,
        "capped-recovery",
        dispatch_at,
        lease_until,
    );

    let error = reconcile_startup(
        &mut store,
        &config_at("capped-recovery", 1, recovery_at, lease_until, 1),
        &bindings,
    )
    .expect_err("progress without a confirming pass must fail the startup gate");

    assert_eq!(error, RecoveryError::IterationLimitExceeded);
    let current = store.inspect(&fixture.record.intent_id).unwrap().unwrap();
    assert_eq!(current.reason(), ReasonCode::TransportRejected);
    assert_eq!(authority.calls.get(), 1);
    assert_eq!(
        store
            .inspect_transition_chain(&fixture.record.intent_id)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn w11_repeated_startups_do_not_extend_stable_blocker_or_resolution_chains() {
    let mut rejected_fixture = terminal_fixture();
    rejected_fixture.record.terminal_disposition = TerminalDisposition::Rejected;
    rejected_fixture.record.binding_sha256 = terminal_binding_sha256(&rejected_fixture.record);
    let rejected_authority = FakeAuthority::terminal(rejected_fixture.record.clone());
    let rejected_bindings = StaticBindings::new(&rejected_fixture, &rejected_authority);
    let mut rejected_store = BusinessIntentStore::open(&rejected_fixture.database).unwrap();
    let (dispatch_at, recovery_at, lease_until) = authority_times();
    dispatch_terminal_fixture(
        &rejected_fixture,
        &mut rejected_store,
        "repeat-rejected",
        dispatch_at,
        lease_until,
    );
    let repeated_config = config_at("repeat-rejected", 1, recovery_at, lease_until, 8);
    reconcile_startup(&mut rejected_store, &repeated_config, &rejected_bindings).unwrap();
    let rejected_chain_len = rejected_store
        .inspect_transition_chain(&rejected_fixture.record.intent_id)
        .unwrap()
        .len();
    let second =
        reconcile_startup(&mut rejected_store, &repeated_config, &rejected_bindings).unwrap();
    assert_eq!(second.transition_count(), 0);
    assert_eq!(
        rejected_store
            .inspect_transition_chain(&rejected_fixture.record.intent_id)
            .unwrap()
            .len(),
        rejected_chain_len
    );

    let blocked_fixture = terminal_fixture();
    let blocked_authority = FakeAuthority::terminal(blocked_fixture.record.clone());
    *blocked_authority.result.borrow_mut() = Ok(AuthorityQuery::Missing);
    let blocked_bindings = StaticBindings::new(&blocked_fixture, &blocked_authority);
    let mut blocked_store = BusinessIntentStore::open(&blocked_fixture.database).unwrap();
    dispatch_terminal_fixture(
        &blocked_fixture,
        &mut blocked_store,
        "repeat-blocked",
        dispatch_at,
        lease_until,
    );
    let blocked_config = config_at("repeat-blocked", 1, recovery_at, lease_until, 8);
    reconcile_startup(&mut blocked_store, &blocked_config, &blocked_bindings).unwrap();
    let blocked_chain_len = blocked_store
        .inspect_transition_chain(&blocked_fixture.record.intent_id)
        .unwrap()
        .len();
    let second = reconcile_startup(&mut blocked_store, &blocked_config, &blocked_bindings).unwrap();
    assert_eq!(second.transition_count(), 0);
    assert_eq!(
        blocked_store
            .inspect_transition_chain(&blocked_fixture.record.intent_id)
            .unwrap()
            .len(),
        blocked_chain_len
    );

    let mut uncertain_fixture = terminal_fixture();
    uncertain_fixture.record.terminal_disposition = TerminalDisposition::Uncertain;
    uncertain_fixture.record.binding_sha256 = terminal_binding_sha256(&uncertain_fixture.record);
    let uncertain_authority = FakeAuthority::terminal(uncertain_fixture.record.clone());
    let uncertain_bindings = StaticBindings::new(&uncertain_fixture, &uncertain_authority);
    let mut uncertain_store = BusinessIntentStore::open(&uncertain_fixture.database).unwrap();
    dispatch_terminal_fixture(
        &uncertain_fixture,
        &mut uncertain_store,
        "repeat-uncertain",
        dispatch_at,
        lease_until,
    );
    let uncertain_config = config_at("repeat-uncertain", 1, recovery_at, lease_until, 8);
    reconcile_startup(&mut uncertain_store, &uncertain_config, &uncertain_bindings).unwrap();
    let uncertain_calls = uncertain_authority.calls.get();
    let uncertain_chain_len = uncertain_store
        .inspect_transition_chain(&uncertain_fixture.record.intent_id)
        .unwrap()
        .len();
    let second =
        reconcile_startup(&mut uncertain_store, &uncertain_config, &uncertain_bindings).unwrap();
    assert_eq!(second.transition_count(), 0);
    assert_eq!(uncertain_authority.calls.get(), uncertain_calls);
    assert_eq!(
        uncertain_store
            .inspect_transition_chain(&uncertain_fixture.record.intent_id)
            .unwrap()
            .len(),
        uncertain_chain_len
    );
}
