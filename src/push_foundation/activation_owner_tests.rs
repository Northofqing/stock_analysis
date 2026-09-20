use crate::monitor::push_job::{GitSha40, Sha256Digest, UnitId};

use super::activation::{
    ActivationManifest, DesiredActivationState as State, PromotionAction as Action,
    PromotionJournalEntry,
};
use super::activation_codec::{journal_digest, manifest_digest, promotion_event_id};
use super::activation_facts::{ActivationReconciliation, UnitActivationFacts};
use super::activation_owner::{
    project_owner_admission, AdmissionProjectionError, NewWorkAdmissionClaim,
};

fn digest() -> Sha256Digest {
    Sha256Digest::parse("fixture", &"a".repeat(64)).expect("TEST_CODE digest")
}

/// The projection consumes validated T1 values; these unit fixtures model that boundary, not
/// operator authentication or SQLite validation (covered by the real-store tests separately).
fn empty_history() -> UnitActivationFacts {
    UnitActivationFacts {
        unit_id: UnitId::try_new("MU-p01".into()).expect("TEST_CODE unit"),
        manifests: Vec::new(),
        journal: Vec::new(),
        reconciliation: ActivationReconciliation::Unregistered,
    }
}

fn append(facts: &mut UnitActivationFacts, action: Action, owner: &str, target: Option<usize>) {
    let generation = facts.manifests.len() as u64 + 1;
    let state = match action {
        Action::Initialize | Action::Disable => State::Disabled,
        Action::EnterShadow => State::Shadow,
        Action::Activate => State::Active,
        Action::Drain => State::Draining,
        Action::Rollback => {
            facts.manifests[target.expect("TEST_CODE rollback target")].desired_state()
        }
    };
    let mut manifest = ActivationManifest {
        manifest_sha256: digest(),
        unit_id: facts.unit_id.clone(),
        generation,
        previous_manifest_sha256: facts.manifests.last().map(|m| m.manifest_sha256().clone()),
        desired_state: state,
        physical_owner: owner.into(),
        build_commit: GitSha40::parse(&"b".repeat(40)).expect("TEST_CODE Git"),
        build_sha256: digest(),
        catalog_sha256: digest(),
        business_schema_sha256: digest(),
        durable_schema_sha256: digest(),
        template_sha256: digest(),
        source_contract_sha256: digest(),
        evidence_sha256: digest(),
        approved_by: "fixture-approver-not-authenticated".into(),
        approved_at: generation * 10,
        window_start: generation * 10,
        window_end: generation * 10 + 9,
        rollback_target_sha256: target
            .map(|index| facts.manifests[index].manifest_sha256().clone()),
        created_at: generation * 10 + 1,
    };
    manifest.manifest_sha256 = manifest_digest(&manifest);
    let mut entry = PromotionJournalEntry {
        event_id: promotion_event_id(facts.unit_id.as_str(), generation),
        unit_id: facts.unit_id.clone(),
        generation,
        from_manifest_sha256: manifest.previous_manifest_sha256.clone(),
        to_manifest_sha256: manifest.manifest_sha256.clone(),
        actor: manifest.approved_by.clone(),
        action,
        reason: "activation.applied".into(),
        window_start: manifest.window_start,
        window_end: manifest.window_end,
        evidence_sha256: manifest.evidence_sha256.clone(),
        rollback_target_sha256: manifest.rollback_target_sha256.clone(),
        previous_sha256: facts
            .journal
            .last()
            .map(|entry| entry.canonical_sha256().clone()),
        canonical_sha256: digest(),
        occurred_at: generation * 10 + 2,
    };
    entry.canonical_sha256 = journal_digest(&entry);
    facts.manifests.push(manifest);
    facts.journal.push(entry);
    facts.reconciliation = ActivationReconciliation::CaughtUp { generation };
}

fn full_cycle() -> UnitActivationFacts {
    let mut facts = empty_history();
    for (action, owner) in [
        (Action::Initialize, "legacy"),
        (Action::EnterShadow, "legacy"),
        (Action::Activate, "new"),
        (Action::Drain, "new"),
        (Action::Disable, "new"),
    ] {
        append(&mut facts, action, owner, None);
    }
    facts
}

#[test]
fn initial_disabled_and_shadow_reference_baseline_without_granting_permission() {
    for owner in ["legacy", "None"] {
        let mut facts = empty_history();
        append(&mut facts, Action::Initialize, owner, None);
        let baseline = facts.manifests[0].manifest_sha256().clone();
        let first = project_owner_admission(&facts).expect("TEST_CODE initial projection");
        assert_eq!(
            first.new_work,
            NewWorkAdmissionClaim::InitializationBaseline {
                manifest_sha256: baseline.clone()
            }
        );
        append(&mut facts, Action::EnterShadow, owner, None);
        let shadow = project_owner_admission(&facts).expect("TEST_CODE shadow projection");
        assert_eq!(shadow.new_work, first.new_work);
        assert_eq!(shadow.physical_owner, owner);
        assert_eq!(shadow.generation, 2);
        assert_ne!(shadow.manifest_sha256, first.manifest_sha256);
        assert!(shadow.rollback_approval_manifests.is_empty());
    }
}

#[test]
fn activation_replaces_baseline_and_drain_disable_shadow_remain_closed() {
    let mut facts = empty_history();
    append(&mut facts, Action::Initialize, "legacy", None);
    append(&mut facts, Action::EnterShadow, "legacy", None);
    append(&mut facts, Action::Activate, "new", None);
    let active = project_owner_admission(&facts).expect("TEST_CODE active");
    assert_eq!(
        active.new_work,
        NewWorkAdmissionClaim::ActivationApproval {
            manifest_sha256: facts.manifests[2].manifest_sha256().clone(),
        }
    );
    for action in [Action::Drain, Action::Disable, Action::EnterShadow] {
        append(&mut facts, action, "new", None);
        let result = project_owner_admission(&facts).expect("TEST_CODE post-drain projection");
        assert_eq!(result.new_work, NewWorkAdmissionClaim::Closed);
        assert_eq!(result.physical_owner, "new");
    }
}

#[test]
fn rollback_to_each_exact_target_restores_its_claim_under_the_new_generation() {
    let original = full_cycle();
    for target in 0..5 {
        let mut facts = original.clone();
        let owner = facts.manifests[target].physical_owner().to_owned();
        append(&mut facts, Action::Rollback, &owner, Some(target));
        let result = project_owner_admission(&facts).expect("TEST_CODE rollback projection");
        let expected = match target {
            0 | 1 => NewWorkAdmissionClaim::InitializationBaseline {
                manifest_sha256: facts.manifests[0].manifest_sha256().clone(),
            },
            2 => NewWorkAdmissionClaim::ActivationApproval {
                manifest_sha256: facts.manifests[2].manifest_sha256().clone(),
            },
            _ => NewWorkAdmissionClaim::Closed,
        };
        assert_eq!(result.new_work, expected);
        assert_eq!(result.generation, 6);
        assert_eq!(result.physical_owner, owner);
        assert_eq!(result.manifest_sha256, facts.manifests[5].manifest_sha256);
        assert_eq!(
            result.rollback_approval_manifests,
            vec![facts.manifests[5].manifest_sha256.clone()]
        );
    }
}

#[test]
fn nested_rollback_retains_every_scope_approval_on_the_selected_target_path() {
    let mut facts = full_cycle();
    append(&mut facts, Action::Rollback, "legacy", Some(0));
    append(&mut facts, Action::EnterShadow, "legacy", None);
    append(&mut facts, Action::Rollback, "legacy", Some(6));
    let result = project_owner_admission(&facts).expect("TEST_CODE nested rollback");
    assert_eq!(
        result.new_work,
        NewWorkAdmissionClaim::InitializationBaseline {
            manifest_sha256: facts.manifests[0].manifest_sha256.clone()
        }
    );
    assert_eq!(
        result.rollback_approval_manifests,
        vec![
            facts.manifests[5].manifest_sha256.clone(),
            facts.manifests[7].manifest_sha256.clone()
        ]
    );
    assert_eq!(result.generation, 8);
    append(&mut facts, Action::Activate, "next", None);
    let fresh = project_owner_admission(&facts).expect("TEST_CODE fresh activation");
    assert!(fresh.rollback_approval_manifests.is_empty());
    assert_eq!(
        fresh.new_work,
        NewWorkAdmissionClaim::ActivationApproval {
            manifest_sha256: facts.manifests[8].manifest_sha256.clone()
        }
    );
}

#[test]
fn owner_preserving_actions_reject_changes_even_outside_the_selected_rollback_path() {
    for broken_action in [Action::EnterShadow, Action::Drain, Action::Disable] {
        let mut facts = empty_history();
        let mut owner = "legacy";
        for action in [
            Action::Initialize,
            Action::EnterShadow,
            Action::Activate,
            Action::Drain,
            Action::Disable,
        ] {
            if action == Action::Activate {
                owner = "new";
            }
            if action == broken_action {
                owner = "unapproved-replacement";
            }
            append(&mut facts, action, owner, None);
        }
        append(&mut facts, Action::Rollback, "legacy", Some(0));
        assert_eq!(
            project_owner_admission(&facts),
            Err(AdmissionProjectionError::OwnerNotPreserved)
        );
    }
}

#[test]
fn unregistered_and_pending_history_never_fall_back_to_old_admission() {
    let mut facts = empty_history();
    assert_eq!(
        project_owner_admission(&facts),
        Err(AdmissionProjectionError::Unregistered)
    );
    append(&mut facts, Action::Initialize, "legacy", None);
    facts.journal.clear();
    facts.reconciliation = ActivationReconciliation::Pending {
        executed_generation: None,
        pending_generation: 1,
    };
    assert_eq!(
        project_owner_admission(&facts),
        Err(AdmissionProjectionError::ReconciliationRequired)
    );
    let mut applied = full_cycle();
    append(&mut applied, Action::Rollback, "legacy", Some(0));
    applied.journal.pop();
    applied.reconciliation = ActivationReconciliation::Pending {
        executed_generation: Some(5),
        pending_generation: 6,
    };
    assert_eq!(
        project_owner_admission(&applied),
        Err(AdmissionProjectionError::ReconciliationRequired)
    );
}

#[test]
fn rollback_without_an_earlier_matching_target_is_rejected() {
    for malformed in 0..4 {
        let mut facts = full_cycle();
        append(&mut facts, Action::Rollback, "legacy", Some(0));
        let current_hash = facts.manifests[5].manifest_sha256.clone();
        match malformed {
            0 => facts.manifests[5].rollback_target_sha256 = None,
            1 => facts.manifests[5].rollback_target_sha256 = Some(current_hash),
            2 => facts.manifests[5].physical_owner = "wrong-owner".into(),
            _ => facts.manifests[5].desired_state = State::Active,
        }
        assert_eq!(
            project_owner_admission(&facts),
            Err(AdmissionProjectionError::InvalidHistory)
        );
    }
}

#[test]
fn projection_does_not_modify_history_or_disclose_owner_in_debug() {
    let mut facts = empty_history();
    append(&mut facts, Action::Initialize, "private-owner-secret", None);
    let before = facts.clone();
    let result = project_owner_admission(&facts).expect("TEST_CODE projection");
    assert_eq!(facts, before);
    assert!(!format!("{result:?}").contains("private-owner-secret"));
    assert!(!format!("{result:?}").contains("fixture-approver"));
}
