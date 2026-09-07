use std::cell::Cell;

use crate::monitor::push_job::{
    AudienceId, BusinessDate, CompletionOwnerId, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, ReasonCode, Sha256Digest, SourceContractId,
    SubjectId, UnitId, UtcMicros,
};

use super::intent_store::AttestedReadyIntent;
use super::reconciler::{
    reconcile_startup, RecoveryBindingError, RecoveryBindings, RecoveryBindingsPort,
    RecoveryBoundary, RecoveryConfig,
};
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
        self.store.record_initial(&draft).unwrap();
        self.store.inspect(&intent_id).unwrap().unwrap()
    }
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
    RecoveryConfig::try_new(
        LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
        TransitionActor::try_new(owner.to_owned()).unwrap(),
        micros(RECOVERY_AT),
        micros(RECOVERY_UNTIL),
        page_size,
        8,
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
