use super::*;
use crate::durable_delivery::tests::{
    envelope, fixture_coordinator_arc, now, prepare_reserved, receipt, reconcile_terminal,
    rejection, uncertainty, Fixture, MemoryAppendPort, StaticSink,
};
use crate::durable_delivery::DeliverySubKind;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};

fn candidate(label: &str) -> DeliveryEnvelope {
    envelope(
        label,
        PushKind::HoldingPlan,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    )
}
fn instrument(e: &DeliveryEnvelope) -> InstrumentId {
    InstrumentId::new(
        Exchange::Shanghai,
        e.scope_key.rsplit(':').next().unwrap(),
        AssetClass::Equity,
    )
    .unwrap()
}
fn observed(c: &DurableDeliveryCoordinator, e: &DeliveryEnvelope) -> HoldingPlanOwnedOccurrence {
    match c
        .inspect_holding_plan_occurrence(
            NaiveDate::parse_from_str(&e.business_date, "%Y-%m-%d").unwrap(),
            &instrument(e),
        )
        .unwrap()
    {
        HoldingPlanOccurrenceObservation::Owned(owner) => owner,
        HoldingPlanOccurrenceObservation::Missing => panic!("actual T03 owner missing"),
    }
}
fn changed_card(e: &DeliveryEnvelope) -> DeliveryEnvelope {
    let mut text = e.rendered_content.clone();
    text.extend_from_slice(b" TEST_CODE_LATER_QUOTE");
    DeliveryEnvelope::new(
        e.business_date.clone(),
        e.push_kind,
        e.sub_kind,
        e.scope_key.clone(),
        e.schedule_occurrence_identity.clone(),
        e.source_evidence_fingerprint.clone(),
        e.source_binding_canonical.clone(),
        e.delivery_subject_hash.clone(),
        text,
        e.retry_authorized,
        e.task_binding.clone(),
    )
    .unwrap()
}
fn counts(f: &Fixture) -> (i64, i64, i64, i64) {
    (
        f.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox"),
        f.query_i64("SELECT COUNT(*) FROM daily_budget_reservations"),
        f.query_i64("SELECT COUNT(*) FROM delivery_disposition_payloads"),
    )
}

#[test]
fn t03_exact_owner_missing_reader_is_read_only_and_does_not_create_legacy_table() {
    let f = Fixture::new("T03_MISSING");
    let e = candidate("T03_MISSING");
    let before = counts(&f);
    assert_eq!(
        f.coordinator
            .inspect_holding_plan_occurrence(
                NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
                &instrument(&e)
            )
            .unwrap(),
        HoldingPlanOccurrenceObservation::Missing
    );
    assert_eq!(counts(&f), before);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM sqlite_master WHERE name='holding_plan_daily'"),
        0
    );
}

#[test]
fn t03_exact_owner_two_coordinator_race_freezes_one_original_without_loser_side_effects() {
    let f = Fixture::new("T03_RACE");
    let first = candidate("T03_RACE");
    let second = changed_card(&first);
    let a = fixture_coordinator_arc(&f);
    let b = f.second_coordinator("T03_RACE_B");
    let barrier = Arc::new(Barrier::new(2));
    let handles = [(a, first.clone()), (b, second.clone())]
        .into_iter()
        .map(|(c, e)| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                c.prepare_holding_plan_occurrence(&e, 1, now()).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    let winners = outcomes
        .iter()
        .filter_map(|o| match o {
            HoldingPlanPrepareOutcome::Prepared(p) => Some(p.decision_identity.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(winners.len(), 1);
    let loser = outcomes
        .iter()
        .find_map(|o| match o {
            HoldingPlanPrepareOutcome::AlreadyOwned(o) => Some(o),
            _ => None,
        })
        .unwrap();
    assert_eq!(loser.envelope().decision_identity, winners[0]);
    let original = if winners[0] == first.decision_identity {
        &first
    } else {
        &second
    };
    assert_eq!(
        loser.envelope().canonical_bytes().unwrap(),
        original.canonical_bytes().unwrap()
    );
    assert_eq!(counts(&f).0, 1);
    assert_eq!(counts(&f).2, 1);
    assert_eq!(counts(&f).3, 0);
    let before = counts(&f);
    let competitor = if original == &first { &second } else { &first };
    assert!(
        matches!(f.coordinator.prepare(competitor,0,now()),Err(DurableDeliveryError::PolicyMismatch(reason)) if reason=="holding_plan_exact_occurrence_already_owned")
    );
    assert_eq!(
        counts(&f),
        before,
        "generic conflict must not freeze sink-count denial or new audit/budget"
    );
    assert_eq!(
        f.coordinator
            .prepare(original, 1, now())
            .unwrap()
            .decision_identity,
        winners[0]
    );
    assert_eq!(counts(&f), before);
}

#[test]
fn t03_exact_owner_first_insert_race_cannot_write_unguarded_incoming_conflict_audit() {
    let f = Fixture::new("T03_FIRST_INSERT_ROUTING");
    let owner = candidate("T03_FIRST_INSERT_ROUTING");
    let mut incoming = owner.clone();
    incoming.push_kind = PushKind::HoldingEvent;
    // As in the original G5b regression, conflict input is raw observation:
    // it need not qualify as a newly admitted envelope under its claimed kind.
    let second = f.second_coordinator("T03_FIRST_INSERT_OWNER");
    let new_owner = owner.clone();
    f.coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterMutationRoutingBeforeDateFence,
            move || {
                second.prepare(&new_owner, 1, now())?;
                Ok(())
            },
        )
        .unwrap();
    assert!(matches!(
        f.coordinator.prepare(&incoming, 1, now()),
        Err(DurableDeliveryError::PolicyMismatch(reason))
            if reason.contains("mutation routing changed after preflight")
    ));
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 1);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"),
        0,
        "the old None Holding context must fail closed without audit writes"
    );
    assert_eq!(
        f.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
        owner.canonical_bytes().unwrap()
    );
    let before = counts(&f);
    let actual = fixture_coordinator_arc(&f);
    arm_fault(
        &actual,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        OperationPostvalidationTestFault::HoldingPlanExtraAudit,
    );
    // A new explicit call reads the stored Holding route. Its final SQL hook
    // must reject extra valid audit bytes and roll back the original conflict.
    let error = actual.prepare(&incoming, 1, now()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("holding_plan_exact_owner_sql_witness_changed"),
        "{error}"
    );
    assert_eq!(counts(&f), before);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"),
        0
    );
    // This is a further caller-initiated prepare, not an automatic retry or
    // new delivery authorization. Preserve the original conflict audit API.
    assert!(matches!(
        actual.prepare(&incoming, 1, now()),
        Err(DurableDeliveryError::DecisionIdentityConflict { .. })
    ));
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox WHERE audit_kind='DecisionIdentityConflict'"),
        1
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 1);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM daily_budget_reservations"),
        before.2
    );
    assert_eq!(
        f.query_blob("SELECT envelope_canonical FROM delivery_decisions"),
        owner.canonical_bytes().unwrap()
    );
}

#[test]
fn t03_exact_owner_reopen_physical_accepted_is_original_receipt_not_delivered_bool() {
    let mut f = Fixture::new("T03_REOPEN");
    let e = candidate("T03_REOPEN");
    let append = MemoryAppendPort::default();
    prepare_reserved(&f, &e, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    assert_eq!(
        observed(&f.coordinator, &e).receipt_kind(),
        HoldingPlanReceiptKind::Pending
    );
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    let original = observed(&f.coordinator, &e);
    assert_eq!(
        original.receipt_kind(),
        HoldingPlanReceiptKind::PhysicalAccepted
    );
    assert!(original.local_drained());
    assert!(original.terminal_ref().is_some());
    assert!(original.evidence_sha256().is_some());
    let raw =
        f.query_blob("SELECT result_canonical FROM sink_results WHERE result_kind='Accepted'");
    assert_eq!(original.evidence_sha256(), Some(sha256_hex(&raw).as_str()));
    drop(f.coordinator.take());
    let reopened = f.second_coordinator("T03_REOPEN_ACTUAL");
    assert_eq!(observed(&reopened, &e), original);
    assert_eq!(
        reopened
            .resume_deliverable(&e.decision_identity, &sinks, now())
            .unwrap()
            .sink_calls,
        0
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    drop(reopened);
}

struct UnavailableAppend;
impl ImmutableAppendPort for UnavailableAppend {
    fn append_exact(&self, _kind: &str, _id: &str, _bytes: &[u8], _sha: &str) -> Result<String> {
        Err(DurableDeliveryError::Io(std::io::Error::other(
            "TEST_CODE_T03_APPEND_UNAVAILABLE",
        )))
    }
}
#[test]
fn t03_exact_owner_audit_io_keeps_accepted_raw_and_restores_without_second_sink() {
    let f = Fixture::new("T03_AUDIT_IO");
    let e = candidate("T03_AUDIT_IO");
    let append = MemoryAppendPort::default();
    prepare_reserved(&f, &e, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    let raw =
        f.query_blob("SELECT result_canonical FROM sink_results WHERE result_kind='Accepted'");
    assert!(f
        .coordinator
        .reconcile_all_pending(&UnavailableAppend, now())
        .is_err());
    let owner = observed(&f.coordinator, &e);
    assert_eq!(owner.receipt_kind(), HoldingPlanReceiptKind::Pending);
    assert!(!owner.local_drained());
    assert_eq!(
        f.coordinator
            .resume_deliverable(&e.decision_identity, &sinks, now())
            .unwrap()
            .sink_calls,
        0
    );
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    assert_eq!(
        f.query_blob("SELECT result_canonical FROM sink_results WHERE result_kind='Accepted'"),
        raw
    );
    assert_eq!(
        observed(&f.coordinator, &e).receipt_kind(),
        HoldingPlanReceiptKind::PhysicalAccepted
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn t03_exact_owner_uncertain_restart_and_manual_accepted_never_become_physical() {
    let mut f = Fixture::new("T03_MANUAL");
    let e = candidate("T03_MANUAL");
    let append = MemoryAppendPort::default();
    prepare_reserved(&f, &e, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(
        &f,
        &append,
        DecisionState::UncertainManualReview,
        &e.decision_identity,
    );
    let original = observed(&f.coordinator, &e);
    assert_eq!(original.receipt_kind(), HoldingPlanReceiptKind::Uncertain);
    drop(f.coordinator.take());
    let reopened = f.second_coordinator("T03_MANUAL_REOPEN");
    assert_eq!(observed(&reopened, &e), original);
    assert_eq!(
        reopened
            .resume_deliverable(&e.decision_identity, &sinks, now())
            .unwrap()
            .sink_calls,
        0
    );
    reopened
        .resolve_uncertain(
            &ManualResolutionCommand {
                decision_identity: e.decision_identity.clone(),
                disposition: ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                },
                operator_identity: "TEST_CODE_T03_OPERATOR_0123456789".into(),
                reason: "TEST_CODE_T03_VERIFIED_MANUAL_ACCEPTANCE".into(),
                external_evidence: b"TEST_CODE_T03_EXTERNAL_MANUAL_EVIDENCE".to_vec(),
                resolved_at: now(),
            },
            &append,
        )
        .unwrap();
    reopened.reconcile_all_pending(&append, now()).unwrap();
    let manual = observed(&reopened, &e);
    assert_eq!(manual.state(), DecisionState::Delivered);
    assert_eq!(
        manual.receipt_kind(),
        HoldingPlanReceiptKind::ManualAccepted
    );
    assert!(manual.local_drained());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM sink_results WHERE result_kind='Accepted'"),
        0
    );
    drop(reopened);
}

#[test]
fn t03_exact_owner_rejection_retry_preserves_real_authorization_and_original_envelope() {
    for initially_authorized in [false, true] {
        let label = if initially_authorized {
            "T03_RETRY_YES"
        } else {
            "T03_RETRY_NO"
        };
        let f = Fixture::new(label);
        let e = candidate(label);
        let append = MemoryAppendPort::default();
        prepare_reserved(&f, &e, &append);
        let rejected = StaticSink::new(AuthoritativeSinkResult::Rejected(rejection(
            now(),
            initially_authorized,
        )));
        let sinks: Vec<AuthoritativeSink> = vec![rejected.clone()];
        f.coordinator
            .resume_deliverable(&e.decision_identity, &sinks, now())
            .unwrap();
        reconcile_terminal(
            &f,
            &append,
            DecisionState::RejectedDurable,
            &e.decision_identity,
        );
        let original = observed(&f.coordinator, &e);
        assert_eq!(original.receipt_kind(), HoldingPlanReceiptKind::Rejected);
        assert_eq!(original.retry_authorized(), initially_authorized);
        let accepted = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(
            now() + Duration::seconds(1),
        )));
        let accepts: Vec<AuthoritativeSink> = vec![accepted.clone()];
        if !initially_authorized {
            assert_eq!(
                f.coordinator
                    .resume_deliverable(&e.decision_identity, &accepts, now())
                    .unwrap()
                    .sink_calls,
                0
            );
            assert_eq!(accepted.calls.load(Ordering::SeqCst), 0);
            f.coordinator
                .authorize_rejected_retry(&e.decision_identity)
                .unwrap();
            let authorized = observed(&f.coordinator, &e);
            assert!(authorized.retry_authorized());
            assert_eq!(authorized.envelope(), &e);
            assert_eq!(authorized.receipt_kind(), HoldingPlanReceiptKind::Rejected);
            assert_eq!(authorized.terminal_ref(), None);
            assert_eq!(authorized.evidence_sha256(), None);
        }
        f.coordinator
            .resume_deliverable(&e.decision_identity, &accepts, now() + Duration::seconds(1))
            .unwrap();
        reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
        assert_eq!(accepted.calls.load(Ordering::SeqCst), 1);
        assert_eq!(rejected.calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 1);
        let completed = observed(&f.coordinator, &e);
        assert_eq!(completed.envelope(), &e);
        assert_eq!(
            completed.receipt_kind(),
            HoldingPlanReceiptKind::PhysicalAccepted
        );
        assert!(completed.terminal_ref().is_some());
    }
}

#[test]
fn t03_exact_owner_fixed_shanghai_date_and_legacy_unknown_cannot_open_a_new_owner() {
    let f = Fixture::new("T03_DATE");
    let e = candidate("T03_DATE");
    let mut source: serde_json::Value =
        serde_json::from_slice(&e.source_binding_canonical).unwrap();
    source["observed_at"] = json!("2026-07-30T17:00:00Z"); // Shanghai July 31.
    let bytes = serde_json::to_vec(&source).unwrap();
    let wrong = DeliveryEnvelope::new(
        e.business_date.clone(),
        e.push_kind,
        e.sub_kind,
        e.scope_key.clone(),
        e.schedule_occurrence_identity.clone(),
        e.source_evidence_fingerprint.clone(),
        bytes.clone(),
        sha256_hex(&bytes),
        e.rendered_content.clone(),
        true,
        None,
    )
    .unwrap();
    assert!(f.coordinator.prepare(&wrong, 1, now()).is_err());
    assert_eq!(counts(&f), (0, 0, 0, 0));
    let legacy = DeliveryEnvelope::new(
        e.business_date.clone(),
        e.push_kind,
        e.sub_kind,
        e.scope_key.clone(),
        "TEST_CODE_OLD_T03_UNKNOWN_OCCURRENCE",
        e.source_evidence_fingerprint.clone(),
        b"TEST_CODE_OLD_SOURCE".to_vec(),
        "TEST_CODE_OLD_SUBJECT",
        e.rendered_content.clone(),
        true,
        None,
    )
    .unwrap();
    // An actual old row fixture, not a factory that can admit/send it.
    f.coordinator
        .with_immediate_transaction(|tx| {
            let raw = legacy.canonical_bytes()?;
            insert_new_decision(
                tx,
                &legacy,
                &raw,
                &sha256_hex(&raw),
                DecisionState::Reserved,
                now(),
            )?;
            record_state_transition(
                tx,
                &legacy.decision_identity,
                None,
                DecisionState::Reserved,
                "prepare",
                None,
                canonical_json(
                    &json!({"envelope_sha256":sha256_hex(&raw),"reservation_generation":1}),
                )?,
                now(),
            )?;
            let policy = load_policy(tx, legacy.push_kind, legacy.sub_kind)?;
            f.coordinator
                .reserve_generation(tx, &legacy, &policy, 1, now())
        })
        .unwrap();
    let before = counts(&f);
    assert!(
        matches!(f.coordinator.inspect_holding_plan_occurrence(NaiveDate::from_ymd_opt(2026,7,30).unwrap(),&instrument(&e)),Err(DurableDeliveryError::PolicyMismatch(reason)) if reason.contains("legacy_unknown"))
    );
    assert!(f
        .coordinator
        .prepare_holding_plan_occurrence(&e, 1, now())
        .is_err());
    assert_eq!(counts(&f), before);
    // Original unknown evidence can still be preserved by its stored route.
    f.coordinator
        .reconcile_all_pending(&MemoryAppendPort::default(), now())
        .unwrap();
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions"), 1);
}

fn arm_fault(
    c: &Arc<DurableDeliveryCoordinator>,
    phase: DatabaseOperationTestPhase,
    fault: OperationPostvalidationTestFault,
) {
    let actual = Arc::clone(c);
    c.install_database_operation_test_hook(phase, move || {
        actual.install_operation_postvalidation_test_fault(fault)
    })
    .unwrap();
}
#[test]
fn t03_exact_owner_all_sql_phases_changed_nochange_reject_extra_audit_and_membership() {
    for phase in [
        DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
    ] {
        for already_prepared in [false, true] {
            for fault in [
                OperationPostvalidationTestFault::HoldingPlanExtraAudit,
                OperationPostvalidationTestFault::HoldingPlanDuplicateOwner,
            ] {
                let f = Fixture::new("T03_SQL_FAULT");
                let e = candidate("T03_SQL_FAULT");
                if already_prepared {
                    f.coordinator.prepare(&e, 1, now()).unwrap();
                }
                let before = counts(&f);
                let actual = fixture_coordinator_arc(&f);
                arm_fault(&actual, phase, fault);
                let error = actual.prepare(&e, 1, now()).unwrap_err();
                assert!(error.to_string().contains("holding_plan_"), "{error}");
                assert_eq!(
                    counts(&f),
                    before,
                    "all own writes and injected rows must roll back"
                );
                if already_prepared {
                    assert_eq!(observed(&actual, &e).envelope(), &e);
                }
            }
        }
    }
}

fn external_fault(
    c: &Arc<DurableDeliveryCoordinator>,
    path: &Path,
    phase: DatabaseOperationTestPhase,
    fault: OperationPostvalidationTestFault,
) {
    let actual = Arc::clone(c);
    let path = path.to_owned();
    c.install_database_operation_test_hook(phase, move || {
        let mut external = Connection::open(path)?;
        external.pragma_update(None, "foreign_keys", "ON")?;
        let tx = external.transaction_with_behavior(TransactionBehavior::Immediate)?;
        inject_sql_fault_for_test(&actual, &tx, fault)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
}
#[test]
fn t03_exact_owner_true_postcommit_keeps_committed_owner_but_does_not_return_success() {
    for fault in [
        OperationPostvalidationTestFault::HoldingPlanExtraAudit,
        OperationPostvalidationTestFault::HoldingPlanDuplicateOwner,
    ] {
        let f = Fixture::new("T03_POSTCOMMIT");
        let e = candidate("T03_POSTCOMMIT");
        let actual = fixture_coordinator_arc(&f);
        external_fault(
            &actual,
            &f.database_path,
            DatabaseOperationTestPhase::AfterCommitBeforePostValidation,
            fault,
        );
        let error = actual
            .prepare_holding_plan_occurrence(&e, 1, now())
            .unwrap_err();
        assert!(
            error.to_string().contains("after COMMIT succeeded"),
            "{error}"
        );
        let stored=f.query_blob("SELECT envelope_canonical FROM delivery_decisions ORDER BY created_at,decision_identity LIMIT 1");
        assert!(f.query_i64("SELECT COUNT(*) FROM delivery_decisions") >= 1);
        assert!(!stored.is_empty());
        assert_eq!(
            f.query_i64(&format!(
                "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity='{}'",
                e.decision_identity
            )),
            1,
            "the successful COMMIT cannot be described as rolled back"
        );
        if fault == OperationPostvalidationTestFault::HoldingPlanDuplicateOwner {
            let before = counts(&f);
            assert!(actual
                .inspect_holding_plan_occurrence(
                    NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
                    &instrument(&e)
                )
                .is_err());
            assert_eq!(
                counts(&f),
                before,
                "duplicate history is not healed or first-picked"
            );
        }
    }
}

#[test]
fn t03_exact_owner_reader_rechecks_membership_after_all_core_hooks() {
    for fault in [
        OperationPostvalidationTestFault::HoldingPlanExtraAudit,
        OperationPostvalidationTestFault::HoldingPlanDuplicateOwner,
    ] {
        let f = Fixture::new("T03_READ_HOOK");
        let e = candidate("T03_READ_HOOK");
        f.coordinator.prepare(&e, 1, now()).unwrap();
        let actual = fixture_coordinator_arc(&f);
        external_fault(
            &actual,
            &f.database_path,
            DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
            fault,
        );
        assert!(actual
            .inspect_holding_plan_occurrence(
                NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
                &instrument(&e)
            )
            .is_err());
        assert!(
            f.query_i64("SELECT COUNT(*) FROM immutable_audit_outbox") > 0,
            "hook committed original evidence remains"
        );
    }
}

#[test]
fn t03_exact_owner_different_date_ticket_kind_keep_original_rolling_policy() {
    let f = Fixture::new("T03_POLICY");
    let e = candidate("T03_POLICY");
    let append = MemoryAppendPort::default();
    prepare_reserved(&f, &e, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    let next = envelope(
        "T03_POLICY",
        PushKind::HoldingPlan,
        DeliverySubKind::None,
        "2026-07-31",
        false,
    );
    assert_eq!(
        f.coordinator
            .prepare(&next, 1, now() + Duration::days(1))
            .unwrap()
            .state,
        DecisionState::Reserved
    );
    let other = candidate("T03_OTHER_TICKET");
    assert_eq!(
        f.coordinator.prepare(&other, 1, now()).unwrap().state,
        DecisionState::Reserved
    );
    let unrelated = envelope(
        "T03_UNRELATED",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    assert_eq!(
        f.coordinator.prepare(&unrelated, 1, now()).unwrap().state,
        DecisionState::Reserved
    );
    assert_eq!(
        f.query_i64("SELECT COUNT(*) FROM business_date_once_claims WHERE push_kind='HoldingPlan'"),
        0
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM cooldown_reservations WHERE push_kind='HoldingPlan' AND window_mode='Rolling' AND effective_cooldown_secs=1800"),3);
}

#[test]
fn t03_exact_owner_begin_heartbeat_and_raw_mutators_keep_exact_sql_boundary() {
    let f = Fixture::new("T03_MUTATORS");
    let e = candidate("T03_MUTATORS");
    let append = MemoryAppendPort::default();
    prepare_reserved(&f, &e, &append);
    let actual = fixture_coordinator_arc(&f);
    let before = counts(&f);
    arm_fault(
        &actual,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        OperationPostvalidationTestFault::HoldingPlanExtraAudit,
    );
    assert!(actual
        .begin_attempt(&e.decision_identity, 1, now())
        .is_err());
    assert_eq!(counts(&f), before);
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM delivery_attempts"), 0);
    assert_eq!(
        actual.decision_state(&e.decision_identity).unwrap(),
        DecisionState::Reserved
    );
    let lease = actual
        .begin_attempt(&e.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    // An identical heartbeat is a real NoChange; after-SQL extra rows must
    // still reject rather than being allowed because no body effect occurred.
    let before = counts(&f);
    arm_fault(
        &actual,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        OperationPostvalidationTestFault::HoldingPlanExtraAudit,
    );
    assert!(actual
        .heartbeat_attempt(
            &e.decision_identity,
            &lease.attempt_identity,
            lease.fence_token,
            now()
        )
        .is_err());
    assert_eq!(counts(&f), before);
    let result = AuthoritativeSinkResult::Accepted(receipt(now()));
    arm_fault(
        &actual,
        DatabaseOperationTestPhase::AfterSqlBeforePostValidation,
        OperationPostvalidationTestFault::HoldingPlanExtraAudit,
    );
    assert!(actual
        .record_sink_result(
            &lease.attempt_identity,
            lease.fence_token,
            result.clone(),
            now()
        )
        .is_err());
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    assert_eq!(
        actual.decision_state(&e.decision_identity).unwrap(),
        DecisionState::AttemptInFlight
    );
    // This is the very same observed raw receipt, never a new sink call.
    actual
        .record_sink_result(&lease.attempt_identity, lease.fence_token, result, now())
        .unwrap();
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    assert_eq!(
        observed(&actual, &e).receipt_kind(),
        HoldingPlanReceiptKind::PhysicalAccepted
    );
    assert_eq!(f.query_i64("SELECT COUNT(*) FROM sink_results"), 1);
}

#[test]
fn t03_exact_owner_delivered_with_pending_local_evidence_is_not_drained_completion() {
    let f = Fixture::new("T03_DRAIN");
    let e = candidate("T03_DRAIN");
    let append = MemoryAppendPort::default();
    prepare_reserved(&f, &e, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    f.coordinator
        .resume_deliverable(&e.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(&f, &append, DecisionState::Delivered, &e.decision_identity);
    let original = observed(&f.coordinator, &e);
    // Preserve a new pending local fact without editing any accepted bytes.
    f.coordinator
        .with_immediate_transaction(|tx| {
            inject_sql_fault_for_test(
                &f.coordinator,
                tx,
                OperationPostvalidationTestFault::HoldingPlanExtraAudit,
            )
        })
        .unwrap();
    let pending = observed(&f.coordinator, &e);
    assert_eq!(pending.state(), DecisionState::Delivered);
    assert_eq!(
        pending.receipt_kind(),
        HoldingPlanReceiptKind::PhysicalAccepted
    );
    assert_eq!(pending.evidence_sha256(), original.evidence_sha256());
    assert!(!pending.local_drained());
    f.coordinator.reconcile_all_pending(&append, now()).unwrap();
    assert!(observed(&f.coordinator, &e).local_drained());
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}
