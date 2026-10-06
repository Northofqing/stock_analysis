use super::*;

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
