use super::super::model_change_draft_store_v1::*;
use super::*;
use crate::evidence_retention::outbox_v1::{
    LocalDisposition, OutboxFault, OutboxFixture, RecoveredPresence, TestCommitObservation,
    UnverifiedOutbox,
};
use crate::evidence_retention::{TrustState, ValueError};

fn at(day: u32) -> DateTime<Utc> {
    format!("2026-01-{day:02}T00:00:00Z").parse().unwrap()
}

fn window(from: u32, to: u32) -> DeclaredEvaluationWindowV1 {
    DeclaredEvaluationWindowV1 {
        from: at(from),
        to: at(to),
    }
}

fn version_ref(v: &str) -> DeclaredStrategyVersionV1 {
    DeclaredStrategyVersionV1 {
        strategy_id: "TEST_CODE_strategy".into(),
        strategy_version: v.into(),
        model_id: "TEST_CODE_model".into(),
        model_version: v.into(),
        declared_git_commit: "1".repeat(40),
        config_sha256: "2".repeat(64),
    }
}

fn request() -> ModelChangeDraftRequestV1 {
    // Synthetic declarations only; no real budget, runtime or approval.
    ModelChangeDraftRequestV1 {
        champion: version_ref("v1"),
        challenger: version_ref("v2"),
        rollback: version_ref("v1"),
        champion_paper_book_id: "TEST_CODE_champion_book".into(),
        challenger_paper_book_id: "TEST_CODE_challenger_book".into(),
        declared_registered_at: at(3),
        training: window(1, 3),
        validation: window(3, 6),
        prospective: window(6, 20),
        review_deadline: at(22),
        comparison: SharedComparisonContractV1 {
            universe_manifest_sha256: "3".repeat(64),
            benchmark_policy_version: "TEST_CODE_benchmark_v1".into(),
            input_alignment_policy_version: "TEST_CODE_same_input_v1".into(),
            data_health_policy_version: "TEST_CODE_health_v1".into(),
            account_snapshot_policy_version: "TEST_CODE_account_v1".into(),
            fill_model_version: "TEST_CODE_fill_v1".into(),
            cost_policy_sha256: "4".repeat(64),
        },
        sample_policy: DeclaredSampleSufficiencyPolicyV1 {
            policy_version: "TEST_CODE_samples_v1".into(),
            minimum_decisions: 100,
            minimum_mature_t1: 80,
            minimum_mature_t5: 50,
            minimum_mature_t20: 20,
            required_regimes: vec![
                RequiredRegimeSamplesV1 {
                    regime_id: "TEST_CODE_down".into(),
                    minimum_decisions: 20,
                },
                RequiredRegimeSamplesV1 {
                    regime_id: "TEST_CODE_up".into(),
                    minimum_decisions: 20,
                },
            ],
            minimum_net_benchmark_excess_bps: 25,
            maximum_drawdown_bps: 1000,
            maximum_tail_loss_bps: 500,
            minimum_data_availability_bps: 9900,
            maximum_independent_trials: 10,
            multiple_testing_policy_version: "TEST_CODE_holm_v1".into(),
            minimum_capacity_micro_cny: 100_000_000_000,
            maximum_participation_bps: 100,
        },
        rationale: DeclaredChangeRationaleV1 {
            hypothesis: "TEST_CODE hypothesis".into(),
            proposed_change: "TEST_CODE change".into(),
            expected_benefit: "TEST_CODE benefit".into(),
            risk_impact: "TEST_CODE risk".into(),
            counterfactual_design: "TEST_CODE same inputs and separate books".into(),
        },
    }
}

#[test]
fn model_change_draft_identity_freezes_policy_versions_windows_and_rollback() {
    let r = request();
    let first = build_model_change_draft_v1(&r).unwrap();
    assert_eq!(first, build_model_change_draft_v1(&r).unwrap());
    assert_eq!(first.request(), &r);
    let edits: &[fn(&mut ModelChangeDraftRequestV1)] = &[
        |r| r.challenger.declared_git_commit = "5".repeat(40),
        |r| r.challenger.config_sha256 = "6".repeat(64),
        |r| r.challenger.strategy_version = "v3".into(),
        |r| r.challenger.model_version = "m3".into(),
        |r| r.rollback = version_ref("v0"),
        |r| r.champion_paper_book_id = "TEST_CODE_another_book".into(),
        |r| r.sample_policy.minimum_net_benchmark_excess_bps += 1,
        |r| r.sample_policy.minimum_mature_t20 += 1,
        |r| r.sample_policy.maximum_independent_trials += 1,
        |r| r.sample_policy.required_regimes[0].minimum_decisions += 1,
        |r| r.sample_policy.minimum_capacity_micro_cny += 1,
        |r| r.comparison.cost_policy_sha256 = "7".repeat(64),
        |r| r.comparison.input_alignment_policy_version = "TEST_CODE_input_v2".into(),
        |r| r.prospective.to = at(21),
        |r| r.review_deadline = at(23),
        |r| r.rationale.counterfactual_design = "TEST_CODE revised counterfactual".into(),
    ];
    for edit in edits {
        let mut changed = r.clone();
        edit(&mut changed);
        assert_ne!(
            first.draft_id(),
            build_model_change_draft_v1(&changed).unwrap().draft_id()
        );
    }
    assert!(std::str::from_utf8(first.canonical_bytes())
        .unwrap()
        .contains("DeclaredReferencesOnly"));
}

#[test]
fn model_change_draft_cold_decode_preserves_original_and_refuses_promotion_or_rewrite() {
    let draft = build_model_change_draft_v1(&request()).unwrap();
    assert_eq!(
        draft,
        recover_model_change_draft_v1(draft.canonical_bytes(), draft.draft_id()).unwrap()
    );
    let mut spaced = draft.canonical_bytes().to_vec();
    spaced.push(b' ');
    assert_eq!(
        recover_model_change_draft_v1(&spaced, draft.draft_id()),
        Err(ModelChangeDraftErrorV1::InvalidCanonical)
    );
    let original = std::str::from_utf8(draft.canonical_bytes()).unwrap();
    for altered in [
        original.replace("\"phase\":\"Draft\"", "\"phase\":\"Promoted\""),
        original.replacen("{", "{\"human_approved\":true,", 1),
        original.replace("\"DeclaredReferencesOnly\"", "\"Qualified\""),
    ] {
        assert_eq!(
            recover_model_change_draft_v1(altered.as_bytes(), draft.draft_id()),
            Err(ModelChangeDraftErrorV1::InvalidCanonical)
        );
    }
    let mut changed = request();
    changed.sample_policy.maximum_drawdown_bps += 1;
    let rewrite = build_model_change_draft_v1(&changed).unwrap();
    assert_eq!(
        recover_model_change_draft_v1(rewrite.canonical_bytes(), draft.draft_id()),
        Err(ModelChangeDraftErrorV1::IdentityMismatch)
    );
    let missing = original.replace("\"minimum_mature_t20\":20,", "");
    assert_eq!(
        recover_model_change_draft_v1(missing.as_bytes(), draft.draft_id()),
        Err(ModelChangeDraftErrorV1::InvalidCanonical)
    );
}

#[test]
fn model_change_draft_requires_preregistration_disjoint_windows_and_distinct_books() {
    let invalid: &[(fn(&mut ModelChangeDraftRequestV1), ModelChangeDraftErrorV1)] = &[
        (
            |r| r.declared_registered_at = at(4),
            ModelChangeDraftErrorV1::InvalidWindow,
        ),
        (
            |r| r.training.to = at(4),
            ModelChangeDraftErrorV1::InvalidWindow,
        ),
        (
            |r| r.validation.to = at(7),
            ModelChangeDraftErrorV1::InvalidWindow,
        ),
        (
            |r| r.prospective.to = r.prospective.from,
            ModelChangeDraftErrorV1::InvalidWindow,
        ),
        (
            |r| r.review_deadline = r.prospective.to,
            ModelChangeDraftErrorV1::InvalidWindow,
        ),
        (
            |r| r.challenger_paper_book_id = r.champion_paper_book_id.clone(),
            ModelChangeDraftErrorV1::SharedPaperBook,
        ),
        (
            |r| r.challenger.strategy_version = r.champion.strategy_version.clone(),
            ModelChangeDraftErrorV1::AmbiguousVersion,
        ),
        (
            |r| r.rollback.config_sha256 = "5".repeat(64),
            ModelChangeDraftErrorV1::AmbiguousVersion,
        ),
        (
            |r| r.rollback = r.challenger.clone(),
            ModelChangeDraftErrorV1::AmbiguousVersion,
        ),
    ];
    for (edit, expected) in invalid {
        let mut r = request();
        edit(&mut r);
        assert_eq!(&build_model_change_draft_v1(&r).unwrap_err(), expected);
    }
}

#[test]
fn model_change_draft_checks_policy_and_material_bounds_without_defaults() {
    let edits: &[fn(&mut ModelChangeDraftRequestV1)] = &[
        |r| r.sample_policy.minimum_decisions = 0,
        |r| r.sample_policy.minimum_mature_t20 = 0,
        |r| r.sample_policy.minimum_mature_t5 = 101,
        |r| r.sample_policy.required_regimes.clear(),
        |r| r.sample_policy.required_regimes[1] = r.sample_policy.required_regimes[0].clone(),
        |r| r.sample_policy.required_regimes.reverse(),
        |r| r.sample_policy.maximum_drawdown_bps = 10_001,
        |r| r.sample_policy.maximum_tail_loss_bps = 10_001,
        |r| r.sample_policy.minimum_data_availability_bps = 0,
        |r| r.sample_policy.maximum_participation_bps = 10_001,
        |r| r.sample_policy.maximum_independent_trials = 0,
        |r| r.sample_policy.minimum_capacity_micro_cny = 0,
    ];
    for edit in edits {
        let mut r = request();
        edit(&mut r);
        assert_eq!(
            build_model_change_draft_v1(&r),
            Err(ModelChangeDraftErrorV1::InvalidSamplePolicy)
        );
    }
    let mut r = request();
    r.rationale.hypothesis = "x".repeat(2049);
    assert_eq!(
        build_model_change_draft_v1(&r),
        Err(ModelChangeDraftErrorV1::InvalidRationale)
    );
    r = request();
    r.challenger.config_sha256 = "0".repeat(64);
    assert_eq!(
        build_model_change_draft_v1(&r),
        Err(ModelChangeDraftErrorV1::InvalidReference)
    );
    assert_eq!(
        recover_model_change_draft_v1(&vec![b' '; MAX_BYTES + 1], "TEST_CODE_id"),
        Err(ModelChangeDraftErrorV1::TooLarge)
    );
}

#[test]
fn model_change_draft_deadline_is_exclusive_and_cannot_auto_promote() {
    let draft = build_model_change_draft_v1(&request()).unwrap();
    for (day, expected) in [
        (2, DraftReviewTimeV1::FutureDeclaration),
        (3, DraftReviewTimeV1::BeforeObservation),
        (6, DraftReviewTimeV1::ObservationWindow),
        (19, DraftReviewTimeV1::ObservationWindow),
        (20, DraftReviewTimeV1::AwaitingReview),
        (21, DraftReviewTimeV1::AwaitingReview),
        (22, DraftReviewTimeV1::Expired),
        (23, DraftReviewTimeV1::Expired),
    ] {
        assert_eq!(draft.review_time_at(at(day)).unwrap(), expected);
    }
    assert_eq!(
        draft.review_time_at(DateTime::from_timestamp(-1, 0).unwrap()),
        Err(ModelChangeDraftErrorV1::InvalidWindow)
    );
    assert_eq!(
        draft,
        recover_model_change_draft_v1(draft.canonical_bytes(), draft.draft_id()).unwrap()
    );
}

fn material_command(original: ModelChangeDraftV1) -> DraftMaterialCommand {
    match prepare_draft_material(original) {
        Ok(v) => v,
        Err(v) => panic!("material preparation held {:?}", v.first_fault()),
    }
}

fn material_open(f: &OutboxFixture) -> UnverifiedOutbox {
    match f.open() {
        Ok(v) => v,
        Err(v) => panic!("actual material open held {:?}", v.first_fault()),
    }
}

fn material_close(v: UnverifiedOutbox) {
    if let Err(v) = v.close() {
        panic!("actual material close held {:?}", v.first_fault());
    }
}

fn material_stored(outcome: DraftMaterialOutcome) -> StoredDraftMaterial {
    match outcome {
        DraftMaterialOutcome::Stored(v) => v,
        DraftMaterialOutcome::Held(v) => panic!("material Held {:?}", v.first_fault()),
        DraftMaterialOutcome::Pending(v) => panic!("material Pending {:?}", v.first_fault()),
        _ => panic!("unexpected recovered material"),
    }
}

fn material_held(outcome: DraftMaterialOutcome) -> HeldDraftMaterial {
    match outcome {
        DraftMaterialOutcome::Held(v) => v,
        _ => panic!("expected actual Held"),
    }
}

fn material_recovered(outcome: DraftMaterialOutcome) -> RecoveredDraftMaterial {
    match outcome {
        DraftMaterialOutcome::Recovered(v) => v,
        _ => panic!("expected actual closed readback"),
    }
}

fn original_material(raw: &[u8], id: &str) -> DraftMaterialCommand {
    material_command(recover_model_change_draft_v1(raw, id).unwrap())
}

#[test]
fn model_draft_material_cold_reuse_and_family_conflict_keep_original_bytes() {
    let r = request();
    let original = build_model_change_draft_v1(&r).unwrap();
    let raw = original.canonical_bytes().to_vec();
    let id = original.draft_id().to_owned();
    let pointer = original.canonical_bytes().as_ptr();
    let command = material_command(original);
    assert_eq!(command.original().canonical_bytes().as_ptr(), pointer);
    let slot = command.family_slot().to_owned();
    let envelope: serde_json::Value =
        serde_json::from_slice(command.test_package().as_canonical_bytes()).unwrap();
    assert_eq!(envelope["owner_domain"], "Attribution");
    assert_eq!(
        envelope["owner_schema_claim"],
        "model-change-draft-material-v1"
    );
    assert_eq!(envelope["trust"], "Unverified");
    assert_eq!(envelope["business_day_claim"], "1970-01-01");
    assert_eq!(
        envelope["window_start_claim"],
        serde_json::json!({"unix_seconds":0,"nanosecond":0})
    );
    assert_eq!(
        envelope["window_end_exclusive_claim"],
        serde_json::json!({"unix_seconds":1,"nanosecond":0})
    );
    assert_eq!(envelope["claimed_record_count"], 1);
    for field in [
        "source_chain_before_claim",
        "source_chain_after_claim",
        "artifact_sha256_claim",
        "activation_id_claim",
    ] {
        assert!(envelope[field].is_null());
    }
    assert_eq!(
        hex::decode(envelope["body_hex"].as_str().unwrap()).unwrap(),
        raw
    );
    let f = OutboxFixture::new();
    let first = material_stored(command.persist(material_open(&f), 0));
    assert_eq!(
        (first.generation(), first.disposition(), first.trust()),
        (1, LocalDisposition::Stored, TrustState::Unverified)
    );
    assert_eq!(first.original().canonical_bytes().as_ptr(), pointer);
    assert_eq!(first.family_slot(), slot);
    let cold =
        material_recovered(original_material(&raw, &id).observe_previous(material_open(&f), 1));
    assert_eq!(cold.presence(), RecoveredPresence::ExactUnverified);
    assert_eq!(
        (cold.original().canonical_bytes(), cold.trust()),
        (raw.as_slice(), TrustState::Unverified)
    );
    assert_eq!(
        cold.original_fault(),
        DraftMaterialFault::Outbox(OutboxFault::CommitUnknown)
    );
    let exact = material_stored(original_material(&raw, &id).persist(material_open(&f), 1));
    assert_eq!(
        (exact.generation(), exact.disposition()),
        (2, LocalDisposition::ExactReuse)
    );
    let edits: &[fn(&mut ModelChangeDraftRequestV1)] = &[
        |r| r.sample_policy.minimum_net_benchmark_excess_bps += 1,
        |r| r.prospective.to = at(21),
        |r| r.challenger.config_sha256 = "9".repeat(64),
        |r| r.challenger.model_version = "TEST_CODE_model_v3".into(),
    ];
    for (i, edit) in edits.iter().enumerate() {
        let mut changed = r.clone();
        edit(&mut changed);
        let command = material_command(build_model_change_draft_v1(&changed).unwrap());
        assert_eq!(command.family_slot(), slot);
        assert_ne!(command.original().canonical_bytes(), raw);
        let stored = material_stored(command.persist(material_open(&f), i as i64 + 2));
        assert_eq!(stored.disposition(), LocalDisposition::Conflict);
    }
    let mut count = material_open(&f);
    assert_eq!(
        (
            count.generation(),
            count.test_material_count(),
            count.test_conflict_count()
        ),
        (6, 5, 10)
    );
    material_close(count);
    let old =
        material_recovered(original_material(&raw, &id).observe_previous(material_open(&f), 1));
    assert_eq!(old.presence(), RecoveredPresence::ExactUnverified);
    assert_eq!(old.original().canonical_bytes(), raw);
}

#[test]
fn model_draft_material_actual_cas_busy_commit_unknown_and_close_keep_owners() {
    let original = build_model_change_draft_v1(&request()).unwrap();
    let raw = original.canonical_bytes();
    let id = original.draft_id();
    let f = OutboxFixture::new();
    let a = material_open(&f);
    let b = material_open(&f);
    assert_eq!((a.generation(), b.generation()), (0, 0));
    material_stored(original_material(raw, id).persist(a, 0));
    let command = original_material(raw, id);
    let pointer = command.original().canonical_bytes().as_ptr();
    let stale = material_held(command.persist(b, 0));
    assert_eq!(
        stale.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::StaleGeneration)
    );
    assert_eq!(stale.original().canonical_bytes().as_ptr(), pointer);
    assert!(stale.test_outbox().unwrap().test_command_retained());
    drop(stale.drain_resources_once());
    let mut a = material_open(&f);
    let b = material_open(&f);
    a.test_hold_transaction().unwrap();
    let busy = material_held(original_material(raw, id).persist(b, 1));
    assert_eq!(
        busy.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::Busy)
    );
    assert!(busy.test_outbox().unwrap().test_connection_retained());
    drop(busy.drain_resources_once());
    if let Err(v) = a.test_rollback() {
        panic!("actual rollback {:?}", v.first_fault());
    }

    let unknown = OutboxFixture::new();
    let command = original_material(raw, id);
    let pointer = command.original().canonical_bytes().as_ptr();
    let pending = match command.persist(
        material_open(&unknown).test_observation(TestCommitObservation::LoseResponse),
        0,
    ) {
        DraftMaterialOutcome::Pending(v) => v,
        _ => panic!("genuine response loss"),
    };
    assert_eq!(
        pending.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::CommitUnknown)
    );
    assert_eq!(pending.original().canonical_bytes().as_ptr(), pointer);
    assert_eq!(pending.trust(), TrustState::Unverified);
    assert!(pending.test_actual_owner_retained());
    let observed = material_recovered(pending.observe());
    assert_eq!(observed.presence(), RecoveredPresence::ExactUnverified);
    assert_eq!(observed.original().canonical_bytes().as_ptr(), pointer);
    assert_eq!(
        observed.original_fault(),
        DraftMaterialFault::Outbox(OutboxFault::CommitUnknown)
    );
    let mut count = material_open(&unknown);
    assert_eq!((count.generation(), count.test_material_count()), (1, 1));
    material_close(count);

    let close = OutboxFixture::new();
    let command = original_material(raw, id);
    let pointer = command.original().canonical_bytes().as_ptr();
    let failed = material_held(command.persist(material_open(&close).test_busy_vm(), 0));
    assert_eq!(
        failed.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::CloseHeld)
    );
    assert_eq!(failed.original().canonical_bytes().as_ptr(), pointer);
    assert!(failed.test_outbox().unwrap().test_connection_retained());
    let failed = failed.test_finalize_then_drain();
    assert_eq!(
        failed.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::CloseHeld)
    );
    assert!(!failed.test_outbox().unwrap().test_connection_retained());
    assert_eq!(
        failed
            .drain_resources_once()
            .original()
            .canonical_bytes()
            .as_ptr(),
        pointer
    );
    let cold =
        material_recovered(original_material(raw, id).observe_previous(material_open(&close), 1));
    assert_eq!(cold.presence(), RecoveredPresence::ExactUnverified);
}

#[test]
fn model_draft_material_same_work_and_changed_root_refuse_without_retry() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let original = build_model_change_draft_v1(&request()).unwrap();
    let raw = original.canonical_bytes();
    let id = original.draft_id();
    let f = OutboxFixture::new();
    let mut short = material_open(&f);
    assert_eq!(
        short.test_spend_owned(8 * 1024 * 1024),
        Err(OutboxFault::Work(ValueError::AllocationLimit))
    );
    let command = original_material(raw, id);
    let pointer = command.original().canonical_bytes().as_ptr();
    let failed = material_held(command.persist(short, 0));
    assert_eq!(
        failed.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::Work(ValueError::AllocationLimit))
    );
    assert_eq!(failed.original().canonical_bytes().as_ptr(), pointer);
    assert!(failed.test_outbox().unwrap().test_command_retained());
    drop(failed.drain_resources_once());
    let mut pending = match original_material(raw, id).persist(
        material_open(&f).test_observation(TestCommitObservation::LoseResponse),
        0,
    ) {
        DraftMaterialOutcome::Pending(v) => v,
        _ => panic!("actual pending"),
    };
    pending.test_exhaust_same_work();
    let failed = material_held(pending.observe());
    assert_eq!(
        failed.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::CommitUnknown)
    );
    assert_eq!(failed.original().canonical_bytes(), raw);
    assert!(failed.test_outbox().unwrap().test_connection_retained());
    drop(failed.drain_resources_once());
    let cold =
        material_recovered(original_material(raw, id).observe_previous(material_open(&f), 1));
    assert_eq!(cold.presence(), RecoveredPresence::ExactUnverified);

    let foreign = OutboxFixture::new();
    let outbox = material_open(&foreign);
    fs::set_permissions(foreign.directory(), fs::Permissions::from_mode(0o755)).unwrap();
    let failed = material_held(original_material(raw, id).persist(outbox, 0));
    assert_eq!(
        failed.first_fault(),
        DraftMaterialFault::Outbox(OutboxFault::RootBinding)
    );
    assert_eq!(
        (failed.original().canonical_bytes(), failed.trust()),
        (raw, TrustState::Unverified)
    );
    assert!(failed.test_outbox().unwrap().test_command_retained());
    drop(failed.drain_resources_once());
    fs::set_permissions(foreign.directory(), fs::Permissions::from_mode(0o700)).unwrap();
    let mut untouched = material_open(&foreign);
    assert_eq!(
        (untouched.generation(), untouched.test_material_count()),
        (0, 0)
    );
    material_close(untouched);
}

#[test]
fn model_draft_material_family_golden_and_absence_are_not_approval() {
    let r = request();
    let command = material_command(build_model_change_draft_v1(&r).unwrap());
    assert_eq!(command.family_slot(), "model-change-draft-family-v1:sha256:525296a78e6e1c84caa6752eb7d50fa4ab567d0283e6b03483d1f74b4161826f");
    let slot = command.family_slot().to_owned();
    let mut scoped = r.clone();
    scoped.challenger_paper_book_id = "TEST_CODE_second_book".into();
    assert_ne!(
        material_command(build_model_change_draft_v1(&scoped).unwrap()).family_slot(),
        slot
    );
    scoped = r.clone();
    scoped.challenger.strategy_version = "v3".into();
    assert_ne!(
        material_command(build_model_change_draft_v1(&scoped).unwrap()).family_slot(),
        slot
    );
    let f = OutboxFixture::new();
    let missing = material_recovered(command.observe_previous(material_open(&f), 1));
    assert_eq!(missing.presence(), RecoveredPresence::MissingFactsUnknown);
    assert_eq!(missing.trust(), TrustState::Unverified);
    let mut empty = material_open(&f);
    assert_eq!((empty.generation(), empty.test_material_count()), (0, 0));
    material_close(empty);
    assert_eq!(missing.original().request(), &r);
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
}

fn material_read_ok(value: Result<DraftMaterialRead, HeldDraftMaterialRead>) -> DraftMaterialRead {
    match value {
        Ok(v) => v,
        Err(v) => panic!("material read Held {:?}", v.first_fault()),
    }
}
fn material_read_held(
    value: Result<DraftMaterialRead, HeldDraftMaterialRead>,
) -> HeldDraftMaterialRead {
    match value {
        Err(v) => v,
        Ok(_) => panic!("expected actual read Held"),
    }
}

#[test]
fn model_draft_read_cold_uses_saved_ids_and_reports_other_family_material() {
    let r = request();
    let f = OutboxFixture::new();
    let stored = material_stored(
        material_command(build_model_change_draft_v1(&r).unwrap()).persist(material_open(&f), 0),
    );
    let package_id = stored.package_id().to_owned();
    let draft_id = stored.original().draft_id().to_owned();
    let expected = stored.original().canonical_bytes().to_vec();
    drop(stored); // No Draft object or raw bytes are passed to the reader.
    let cold = material_read_ok(read_draft_material(
        material_open(&f),
        package_id.clone(),
        draft_id.clone(),
    ));
    assert_eq!(cold.original().unwrap().canonical_bytes(), expected);
    assert_eq!(cold.original().unwrap().request(), &r);
    assert_eq!(
        (
            cold.observed_generation(),
            cold.other_family_materials(),
            cold.trust()
        ),
        (1, 0, TrustState::Unverified)
    );
    let mut changed = r;
    changed.sample_policy.maximum_drawdown_bps += 1;
    let conflict = material_stored(
        material_command(build_model_change_draft_v1(&changed).unwrap())
            .persist(material_open(&f), 1),
    );
    assert_eq!(conflict.disposition(), LocalDisposition::Conflict);
    let new_id = conflict.package_id().to_owned();
    let new_draft_id = conflict.original().draft_id().to_owned();
    drop(conflict);
    let old = material_read_ok(read_draft_material(material_open(&f), package_id, draft_id));
    assert_eq!(old.original().unwrap().canonical_bytes(), expected);
    assert_eq!(
        (old.observed_generation(), old.other_family_materials()),
        (2, 1)
    );
    let conflicted = material_read_ok(read_draft_material(material_open(&f), new_id, new_draft_id));
    assert_eq!(conflicted.original().unwrap().request(), &changed);
    assert_eq!(conflicted.other_family_materials(), 1);
    let mut unchanged = material_open(&f);
    assert_eq!(
        (
            unchanged.generation(),
            unchanged.test_material_count(),
            unchanged.test_conflict_count()
        ),
        (2, 2, 1)
    );
    material_close(unchanged);
}

#[test]
fn model_draft_read_rejects_wrong_ids_scope_time_size_and_family_after_actual_read() {
    use crate::evidence_retention::outbox_v1::EnqueueOutcome;
    use crate::evidence_retention::{
        draft_from_claims, DraftClaimsRef, OwnerDomain, UtcInstantClaim,
    };
    let original = build_model_change_draft_v1(&request()).unwrap();
    let raw = original.canonical_bytes();
    let draft_id = original.draft_id();
    let command = material_command(build_model_change_draft_v1(&request()).unwrap());
    let slot = command.family_slot().to_owned();
    drop(command);
    // Each package is genuinely stored through the same native outbox; these
    // are ordinary declarations, never foreign approval capabilities.
    for case in 0..5 {
        let large = vec![b'a'; 64 * 1024 + 1];
        let claims = DraftClaimsRef {
            owner_domain: if case == 0 {
                OwnerDomain::Data
            } else {
                OwnerDomain::Attribution
            },
            owner_schema_claim: "model-change-draft-material-v1",
            logical_slot_claim: if case == 3 {
                "TEST_CODE_wrong_family"
            } else {
                &slot
            },
            business_day_claim: "1970-01-01",
            window_start_claim: UtcInstantClaim {
                unix_seconds: 0,
                nanosecond: 0,
            },
            window_end_exclusive_claim: UtcInstantClaim {
                unix_seconds: if case == 1 { 2 } else { 1 },
                nanosecond: 0,
            },
            claimed_record_count: Some(1),
            source_chain_before_claim: None,
            source_chain_after_claim: None,
            artifact_sha256_claim: None,
            activation_id_claim: None,
        };
        let package = draft_from_claims(claims, if case == 2 { &large } else { raw }).unwrap();
        let package_id = package.id().to_owned();
        let f = OutboxFixture::new();
        match material_open(&f).enqueue(package, 0) {
            EnqueueOutcome::Stored(_) => (),
            _ => panic!("actual ordinary material store"),
        }
        let requested_id = if case == 4 {
            format!("model-change-draft-v1:sha256:{}", "0".repeat(64))
        } else {
            draft_id.to_owned()
        };
        let held = material_read_held(read_draft_material(
            material_open(&f),
            package_id.clone(),
            requested_id.clone(),
        ));
        assert_eq!(held.package_id(), package_id);
        assert_eq!(held.draft_id(), requested_id);
        assert_eq!(held.test_observed_material().unwrap().id(), package_id);
        if case == 4 {
            assert_eq!(
                held.first_fault(),
                &DraftMaterialReadFault::Draft(ModelChangeDraftErrorV1::IdentityMismatch)
            );
        } else {
            assert_eq!(held.first_fault(), &DraftMaterialReadFault::Envelope);
        }
        if case == 3 {
            assert_eq!(held.test_original().unwrap().canonical_bytes(), raw);
        } else {
            assert!(held.test_original().is_none());
        }
        drop(held.drain_resources_once());
        let mut unchanged = material_open(&f);
        assert_eq!(
            (unchanged.generation(), unchanged.test_material_count()),
            (1, 1)
        );
        material_close(unchanged);
    }
}

#[test]
fn model_draft_read_close_changed_root_and_same_work_keep_actual_query_and_material() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let f = OutboxFixture::new();
    let stored = material_stored(
        material_command(build_model_change_draft_v1(&request()).unwrap())
            .persist(material_open(&f), 0),
    );
    let package_id = stored.package_id().to_owned();
    let draft_id = stored.original().draft_id().to_owned();
    let close = material_open(&f).test_busy_vm();
    let held = material_read_held(read_draft_material(
        close,
        package_id.clone(),
        draft_id.clone(),
    ));
    assert_eq!(
        held.first_fault(),
        &DraftMaterialReadFault::Outbox(OutboxFault::CloseHeld)
    );
    assert!(held.test_outbox().unwrap().test_connection_retained());
    assert_eq!(
        held.test_outbox().unwrap().test_read_id(),
        Some(package_id.as_str())
    );
    assert_eq!(
        held.test_outbox()
            .unwrap()
            .test_selected_material()
            .unwrap()
            .id(),
        package_id
    );
    let held = held.test_finalize_then_drain();
    assert_eq!(
        held.first_fault(),
        &DraftMaterialReadFault::Outbox(OutboxFault::CloseHeld)
    );
    assert!(!held.test_outbox().unwrap().test_connection_retained());
    assert_eq!(
        held.test_outbox()
            .unwrap()
            .test_selected_material()
            .unwrap()
            .id(),
        package_id
    );
    drop(held.drain_resources_once());
    let mut short = material_open(&f);
    assert_eq!(
        short.test_spend_owned(8 * 1024 * 1024),
        Err(OutboxFault::Work(ValueError::AllocationLimit))
    );
    let held = material_read_held(read_draft_material(
        short,
        package_id.clone(),
        draft_id.clone(),
    ));
    assert_eq!(
        held.first_fault(),
        &DraftMaterialReadFault::Outbox(OutboxFault::Work(ValueError::AllocationLimit))
    );
    assert_eq!(
        held.test_outbox().unwrap().test_read_id(),
        Some(package_id.as_str())
    );
    drop(held.drain_resources_once());
    let outbox = material_open(&f);
    fs::set_permissions(f.directory(), fs::Permissions::from_mode(0o755)).unwrap();
    let held = material_read_held(read_draft_material(
        outbox,
        package_id.clone(),
        draft_id.clone(),
    ));
    assert_eq!(
        held.first_fault(),
        &DraftMaterialReadFault::Outbox(OutboxFault::RootBinding)
    );
    assert!(held.test_outbox().unwrap().test_connection_retained());
    drop(held.drain_resources_once());
    fs::set_permissions(f.directory(), fs::Permissions::from_mode(0o700)).unwrap();
    let original = material_read_ok(read_draft_material(material_open(&f), package_id, draft_id));
    assert_eq!(
        original.original().unwrap().canonical_bytes(),
        stored.original().canonical_bytes()
    );
}

#[test]
fn model_draft_read_invalid_input_and_absence_do_not_reconstruct_or_approve() {
    let f = OutboxFixture::new();
    let draft_id = build_model_change_draft_v1(&request())
        .unwrap()
        .draft_id()
        .to_owned();
    let absent_id = format!("retention-package-draft-v1:{}", "0".repeat(64));
    let missing = material_read_ok(read_draft_material(
        material_open(&f),
        absent_id,
        draft_id.clone(),
    ));
    assert!(missing.original().is_none());
    assert_eq!(
        (
            missing.observed_generation(),
            missing.other_family_materials(),
            missing.trust()
        ),
        (0, 0, TrustState::Unverified)
    );
    let invalid = "TEST_CODE_invalid".repeat(4096);
    let pointer = invalid.as_ptr();
    let held = material_read_held(read_draft_material(
        material_open(&f).test_busy_vm(),
        invalid,
        draft_id,
    ));
    assert_eq!(held.first_fault(), &DraftMaterialReadFault::Input);
    assert_eq!(held.package_id().as_ptr(), pointer);
    let held = held.drain_resources_once();
    assert_eq!(held.first_fault(), &DraftMaterialReadFault::Input);
    assert!(held.test_outbox().unwrap().test_connection_retained());
    let held = held.test_finalize_then_drain();
    assert_eq!(held.first_fault(), &DraftMaterialReadFault::Input);
    assert_eq!(held.package_id().as_ptr(), pointer);
    drop(held.drain_resources_once());
    let mut unchanged = material_open(&f);
    assert_eq!(
        (unchanged.generation(), unchanged.test_material_count()),
        (0, 0)
    );
    material_close(unchanged);
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
}
