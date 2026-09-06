use super::{
    classify_durable_state, derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id,
    evaluate_completion, AdvanceEvent, AudienceId, AuthorityClass, BusinessDate, CalendarId,
    ChannelId, CompatId, CompatibilityEvidenceRef, CompletionEligibility, CompletionFact,
    CompletionOwnerId, CursorDirective, CursorPolicy, DeliveryAuthority, DeliveryResult,
    DeliveryResultView, DisabledPolicy, DurableStateProjection, FinalizerKind,
    IntentIdentityMaterial, ManualDirective, Namespace, NoDataPolicy, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ReasonCode, RetryEligibility,
    RetryPolicy, RunId, ScheduleDirective, ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId,
    Sha256Digest, SourceContractId, SourceContractVersion, SubjectId, TerminalDisposition, UnitId,
    UtcMicros, WeakOutcome, WeakOutcomeKind,
};

fn occurrence_material() -> OccurrenceIdentityMaterial {
    OccurrenceIdentityMaterial::new(
        BusinessDate::parse("2026-09-06").expect("valid date"),
        OccurrenceFamily::try_new("daily".to_owned()).expect("valid family"),
        OccurrenceKey::try_new("close".to_owned()).expect("valid key"),
    )
}

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse("fixture", &byte.to_string().repeat(64)).expect("valid sha")
}

fn channel(value: &str) -> ChannelId {
    ChannelId::try_new(value.to_owned()).expect("valid channel")
}

fn compatibility_evidence(
    configured: &[&str],
    attempted: &[&str],
    outcomes: &[(&str, WeakOutcomeKind)],
) -> super::Result<CompatibilityEvidenceRef> {
    CompatibilityEvidenceRef::try_new(
        CompatId::try_new("compat-1".to_owned())?,
        derive_intent_id(&IntentIdentityMaterial::new(
            Namespace::Production,
            UnitId::try_new("MU-cli".to_owned())?,
            CompletionOwnerId::try_new("owner-cli".to_owned())?,
            SourceContractId::try_new("cli-v1".to_owned())?,
            derive_occurrence_id(&occurrence_material()),
            SubjectId::Global,
            AudienceId::try_new("operator".to_owned())?,
        )),
        UnitId::try_new("MU-cli".to_owned())?,
        derive_occurrence_id(&occurrence_material()),
        configured.iter().map(|value| channel(value)).collect(),
        attempted.iter().map(|value| channel(value)).collect(),
        outcomes
            .iter()
            .map(|(name, kind)| WeakOutcome::new(channel(name), *kind, digest('a')))
            .collect(),
        digest('b'),
        UtcMicros::try_new(1_788_705_600_000_000)?,
    )
}

#[test]
fn w01_occurrence_golden_hash_is_stable() {
    let id = derive_occurrence_id(&occurrence_material());
    assert_eq!(
        id.as_str(),
        "5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a"
    );
}

#[test]
fn w01_identity_value_types_reject_invalid_input() {
    assert!(UnitId::try_new(String::new()).is_err());
    assert!(ProducerId::try_new(" value".to_owned()).is_err());
    assert!(AudienceId::try_new("value\0hidden".to_owned()).is_err());
    assert!(BusinessDate::parse("2026-9-6").is_err());
    assert!(Sha256Digest::parse("payload", "ABC").is_err());
    assert!(UtcMicros::try_new(-1).is_err());
}

#[test]
fn w01_test_namespace_and_subject_are_validated() {
    let one = Namespace::test(RunId::try_new("run-1".to_owned()).expect("valid run"));
    let two = Namespace::test(RunId::try_new("run-2".to_owned()).expect("valid run"));
    assert_ne!(one, two);
    assert_ne!(one, Namespace::Production);

    let entity = SubjectId::entity("000001.SZ".to_owned()).expect("valid subject");
    assert!(matches!(entity, SubjectId::Entity(ref value) if value.as_str() == "000001.SZ"));
    assert!(SubjectId::entity(" bad-subject".to_owned()).is_err());

    assert_eq!(
        SourceContractVersion::try_new("source-v1".to_owned())
            .expect("valid version")
            .as_str(),
        "source-v1"
    );
}

#[test]
fn w01_outer_identities_bind_source_contract_without_changing_raw_occurrence() {
    let occurrence = occurrence_material();
    let raw_id = derive_occurrence_id(&occurrence);
    let schedule = |source: &str| {
        derive_schedule_occurrence_id(&ScheduleOccurrenceIdentityMaterial::new(
            Namespace::Production,
            UnitId::try_new("MU-close".to_owned()).expect("valid unit"),
            ProducerId::try_new("close-scheduled".to_owned()).expect("valid producer"),
            ScheduleOrTriggerId::try_new("schedule-close".to_owned()).expect("valid schedule"),
            CalendarId::try_new("a-share-calendar".to_owned()).expect("valid calendar"),
            occurrence.clone(),
            CompletionOwnerId::try_new("owner-close".to_owned()).expect("valid owner"),
            SourceContractId::try_new(source.to_owned()).expect("valid source"),
        ))
    };
    let intent = |source: &str| {
        derive_intent_id(&IntentIdentityMaterial::new(
            Namespace::Production,
            UnitId::try_new("MU-close".to_owned()).expect("valid unit"),
            CompletionOwnerId::try_new("owner-close".to_owned()).expect("valid owner"),
            SourceContractId::try_new(source.to_owned()).expect("valid source"),
            raw_id.clone(),
            SubjectId::Global,
            AudienceId::try_new("portfolio-owner".to_owned()).expect("valid audience"),
        ))
    };

    assert_ne!(schedule("close-v1"), schedule("close-v2"));
    assert_ne!(intent("close-v1"), intent("close-v2"));
    assert_eq!(raw_id, derive_occurrence_id(&occurrence));
}

#[test]
fn w02_compatibility_evidence_rejects_channel_shape_conflicts() {
    let valid = compatibility_evidence(
        &["wechat", "feishu"],
        &["feishu"],
        &[("feishu", WeakOutcomeKind::Accepted)],
    )
    .expect("valid compatibility evidence");
    assert_eq!(valid.configured_channels()[0].as_str(), "wechat");
    assert_eq!(valid.configured_channels()[1].as_str(), "feishu");

    assert!(compatibility_evidence(
        &["feishu"],
        &["wechat"],
        &[("wechat", WeakOutcomeKind::Unknown)]
    )
    .is_err());
    assert!(compatibility_evidence(
        &["feishu", "feishu"],
        &["feishu"],
        &[("feishu", WeakOutcomeKind::Accepted)]
    )
    .is_err());
    assert!(compatibility_evidence(
        &["feishu", "wechat"],
        &["feishu", "wechat"],
        &[("feishu", WeakOutcomeKind::Rejected)]
    )
    .is_err());
}

#[test]
fn w02_compatibility_results_enforce_matrix_and_never_finalize() {
    let all_accepted = compatibility_evidence(
        &["feishu", "wechat"],
        &["feishu", "wechat"],
        &[
            ("feishu", WeakOutcomeKind::Accepted),
            ("wechat", WeakOutcomeKind::Accepted),
        ],
    )
    .expect("valid all-accepted evidence");
    let best = DeliveryResult::best_effort_accepted(all_accepted).expect("best-effort matrix");
    assert_eq!(best.authority_class(), DeliveryAuthority::Compat);
    assert_eq!(best.completion_eligibility(), CompletionEligibility::Never);
    assert!(matches!(
        best.view(),
        DeliveryResultView::BestEffortAccepted(_)
    ));

    let mixed = compatibility_evidence(
        &["feishu", "wechat"],
        &["feishu", "wechat"],
        &[
            ("feishu", WeakOutcomeKind::Accepted),
            ("wechat", WeakOutcomeKind::Rejected),
        ],
    )
    .expect("valid mixed evidence");
    assert!(DeliveryResult::best_effort_accepted(mixed.clone()).is_err());
    assert!(DeliveryResult::partially_accepted(mixed).is_ok());

    let none = DeliveryResult::no_channel_configured();
    assert_eq!(
        none.reason_code(),
        Some(ReasonCode::TransportNoChannelConfigured)
    );
    assert_eq!(none.authority_class(), DeliveryAuthority::None);
    assert_eq!(none.completion_eligibility(), CompletionEligibility::Never);
}

#[test]
fn w02_all_fourteen_durable_states_have_exact_routes() {
    use crate::durable_delivery::DecisionState::*;
    let cases = [
        (Reserved, DurableStateProjection::BlockedBeforeAttempt),
        (
            AttemptInFlight,
            DurableStateProjection::BlockedAwaitingReconciliation,
        ),
        (
            AcceptedAuditPending,
            DurableStateProjection::BlockedAwaitingAuthoritySeal,
        ),
        (
            AcceptedTaskTransitionPending,
            DurableStateProjection::BlockedAwaitingAuthoritySeal,
        ),
        (
            Delivered,
            DurableStateProjection::RequiresVerifiedAcceptedOrAlreadyTerminal,
        ),
        (
            RejectedAuditPending,
            DurableStateProjection::BlockedAwaitingReconciliation,
        ),
        (
            RejectedTaskTransitionPending,
            DurableStateProjection::BlockedAwaitingReconciliation,
        ),
        (
            RejectedDurable,
            DurableStateProjection::RequiresVerifiedRejectedOrAlreadyTerminal,
        ),
        (
            UncertainAuditPending,
            DurableStateProjection::BlockedAwaitingAuthoritySeal,
        ),
        (
            UncertainTaskTransitionPending,
            DurableStateProjection::BlockedAwaitingAuthoritySeal,
        ),
        (
            UncertainManualReview,
            DurableStateProjection::RequiresVerifiedUncertainOrAlreadyTerminal,
        ),
        (
            ManualRejectedAuditPending,
            DurableStateProjection::BlockedAwaitingAuthoritySeal,
        ),
        (
            ManualRejectedTaskTransitionPending,
            DurableStateProjection::BlockedAwaitingAuthoritySeal,
        ),
        (
            ManualResolvedRejected,
            DurableStateProjection::RequiresVerifiedNotDeliveredTerminal,
        ),
    ];
    assert_eq!(cases.len(), 14);
    for (state, expected) in cases {
        assert_eq!(classify_durable_state(state), expected, "state={state:?}");
    }
}

#[test]
fn w02_verified_terminal_disposition_controls_strong_result_permissions() {
    use super::delivery::verified_terminal_fixture;

    let accepted = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Accepted,
    ));
    assert_eq!(accepted.authority_class(), DeliveryAuthority::Strong);
    assert_eq!(
        accepted.completion_eligibility(),
        CompletionEligibility::PolicyBound
    );
    assert!(matches!(
        accepted.view(),
        DeliveryResultView::TransportAccepted(_)
    ));

    let rejected = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Rejected,
    ));
    assert_eq!(
        rejected.completion_eligibility(),
        CompletionEligibility::Never
    );
    assert_eq!(rejected.reason_code(), Some(ReasonCode::TransportRejected));

    let uncertain = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Uncertain,
    ));
    assert_eq!(
        uncertain.completion_eligibility(),
        CompletionEligibility::Never
    );
    assert!(uncertain.requires_manual_quarantine());

    let manual = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::ManualConfirmedAccepted,
    ));
    assert!(matches!(
        manual.view(),
        DeliveryResultView::AlreadyTerminal(_)
    ));
    assert_eq!(
        manual.completion_eligibility(),
        CompletionEligibility::PolicyBound
    );
}

#[test]
fn w03_reason_code_registry_is_exact_and_round_trips() {
    let expected = [
        "schedule.not_trading_day",
        "schedule.window_not_open",
        "schedule.window_expired",
        "schedule.occurrence_closed",
        "schedule.occurrence_conflict",
        "schedule.window_open",
        "schedule.deferred",
        "input.source_recovered",
        "activation.ready",
        "input.source_unavailable",
        "input.source_unready",
        "input.evidence_invalid",
        "input.no_verified_batch",
        "input.account_snapshot_missing",
        "input.namespace_violation",
        "policy.disabled",
        "policy.starved",
        "policy.opt_in_disabled",
        "policy.cooldown_active",
        "policy.daily_budget_full",
        "policy.suppressed",
        "intent.payload_conflict",
        "intent.expected_version_conflict",
        "intent.lease_held",
        "intent.transition_conflict",
        "transport.rejected",
        "transport.uncertain",
        "transport.no_channel_configured",
        "transport.all_channels_failed",
        "transport.partially_accepted",
        "finalizer.terminal_ref_invalid",
        "finalizer.binding_mismatch",
        "finalizer.cas_conflict",
        "finalizer.deadline_exceeded",
        "finalizer.transition_append_failed",
        "activation.manifest_mismatch",
        "activation.generation_conflict",
        "activation.owner_conflict",
        "activation.core_unready",
        "activation.producer_unready",
        "shadow.semantic_diff",
        "shadow.side_effect_attempted",
        "operator.not_delivered",
        "operator.unauthorized",
        "operator.evidence_invalid",
        "operator.resolution_conflict",
        "intent.created",
        "intent.no_data",
        "intent.dispatch_claimed",
        "intent.authority_verified",
        "finalizer.completed",
        "activation.applied",
    ];
    assert_eq!(ReasonCode::ALL.len(), 52);
    assert_eq!(
        ReasonCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    for code in ReasonCode::ALL {
        assert_eq!(ReasonCode::try_from(code.as_str()), Ok(code));
        assert!(code.as_str().is_ascii());
        assert!(!code.as_str().contains('\0'));
    }
    assert!(ReasonCode::try_from("transport.accepted").is_err());
}

#[test]
fn w03_retry_directive_preserves_reason_and_never_retries_uncertain() {
    use std::num::NonZeroU32;

    let now = UtcMicros::try_new(100).expect("valid time");
    let not_before = UtcMicros::try_new(200).expect("valid time");
    let waiting = RetryPolicy::InputBackoff { not_before }.evaluate(
        now,
        0,
        false,
        ReasonCode::InputSourceUnavailable,
    );
    assert_eq!(waiting.reason(), ReasonCode::InputSourceUnavailable);
    assert_eq!(
        waiting.eligibility(),
        RetryEligibility::NotBefore(not_before)
    );

    let eligible = RetryPolicy::InputBackoff { not_before }.evaluate(
        not_before,
        0,
        false,
        ReasonCode::InputSourceUnavailable,
    );
    assert_eq!(eligible.eligibility(), RetryEligibility::EligibleInputRetry);

    let uncertain = RetryPolicy::InputBackoff { not_before }.evaluate(
        now,
        1,
        true,
        ReasonCode::TransportUncertain,
    );
    assert_eq!(uncertain.eligibility(), RetryEligibility::Never);
    assert_eq!(uncertain.reason(), ReasonCode::TransportUncertain);

    let rejected = RetryPolicy::AuthorizedRejected {
        not_before,
        max_attempts: NonZeroU32::new(2).expect("nonzero attempts"),
    };
    assert_eq!(
        rejected
            .evaluate(not_before, 0, false, ReasonCode::TransportRejected)
            .eligibility(),
        RetryEligibility::RejectedAuthorizationRequired
    );
    assert_eq!(
        rejected
            .evaluate(not_before, 2, true, ReasonCode::TransportRejected)
            .eligibility(),
        RetryEligibility::AttemptsExhausted
    );
    assert_eq!(
        rejected
            .evaluate(not_before, 1, true, ReasonCode::TransportRejected)
            .eligibility(),
        RetryEligibility::EligibleAuthorizedRejected
    );
}

#[test]
fn w03_input_backoff_cannot_retry_post_attempt_or_contract_failures() {
    let now = UtcMicros::try_new(200).expect("valid time");
    let policy = RetryPolicy::InputBackoff {
        not_before: UtcMicros::try_new(100).expect("valid time"),
    };

    assert_eq!(
        policy
            .evaluate(now, 0, true, ReasonCode::TransportRejected)
            .eligibility(),
        RetryEligibility::Never
    );
    assert_eq!(
        policy
            .evaluate(now, 0, false, ReasonCode::FinalizerCasConflict)
            .eligibility(),
        RetryEligibility::Never
    );
    assert_eq!(
        policy
            .evaluate(now, 0, false, ReasonCode::InputSourceUnready)
            .eligibility(),
        RetryEligibility::EligibleInputRetry
    );
}

#[test]
fn w03_no_data_disabled_and_uncertain_have_distinct_completion() {
    use super::delivery::verified_terminal_fixture;
    use super::policy::{disabled_fixture, policy_fixture, verified_empty_fixture};

    let policy = policy_fixture(
        NoDataPolicy::CloseVerifiedOccurrence,
        DisabledPolicy::CloseDisabledOccurrence,
        CursorPolicy::AcceptedBoundOnly,
        RetryPolicy::Never,
    );

    let no_data = evaluate_completion(
        &policy,
        CompletionFact::VerifiedNoData(verified_empty_fixture()),
    )
    .expect("valid no-data decision");
    assert_eq!(no_data.schedule(), ScheduleDirective::CloseVerifiedNoData);
    assert_eq!(no_data.cursor(), CursorDirective::Never);

    let disabled = evaluate_completion(
        &policy,
        CompletionFact::ExplicitDisabled(disabled_fixture(ReasonCode::PolicyDisabled)),
    )
    .expect("valid disabled decision");
    assert_eq!(
        disabled.schedule(),
        ScheduleDirective::CloseExplicitDisabled
    );
    assert_eq!(disabled.cursor(), CursorDirective::Never);

    let uncertain_delivery = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Uncertain,
    ));
    let uncertain = evaluate_completion(&policy, CompletionFact::Delivery(&uncertain_delivery))
        .expect("valid uncertain decision");
    assert_eq!(uncertain.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(uncertain.cursor(), CursorDirective::Never);
    assert_eq!(uncertain.retry().eligibility(), RetryEligibility::Never);
    assert_eq!(
        uncertain.manual(),
        ManualDirective::QuarantineThenVerifiedManual
    );
}

#[test]
fn w03_strong_terminal_matrix_separates_schedule_cursor_retry_and_manual() {
    use std::num::NonZeroU32;

    use super::delivery::verified_terminal_fixture;
    use super::policy::{fixture_policy_options, try_policy_fixture};

    let accepted_policy = try_policy_fixture(fixture_policy_options()).expect("valid policy");
    assert_eq!(
        accepted_policy
            .completion_owner()
            .completion_owner()
            .as_str(),
        "fixture-owner"
    );
    assert_eq!(
        accepted_policy.completion_owner().unit_id().as_str(),
        "MU-fixture"
    );
    let accepted_delivery = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Accepted,
    ));
    let accepted = evaluate_completion(
        &accepted_policy,
        CompletionFact::Delivery(&accepted_delivery),
    )
    .expect("accepted terminal is allowed");
    assert_eq!(accepted.schedule(), ScheduleDirective::CloseOnAccepted);
    assert_eq!(accepted.cursor(), CursorDirective::AdvanceAccepted);
    assert_eq!(accepted.retry().eligibility(), RetryEligibility::Never);
    assert_eq!(accepted.manual(), ManualDirective::None);

    let manual_delivery = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::ManualConfirmedAccepted,
    ));
    let manual = evaluate_completion(&accepted_policy, CompletionFact::Delivery(&manual_delivery))
        .expect("manual accepted terminal is allowed");
    assert_eq!(manual.schedule(), ScheduleDirective::CloseOnAccepted);
    assert_eq!(manual.cursor(), CursorDirective::AdvanceManualAccepted);

    let mut accepted_only = fixture_policy_options();
    accepted_only.advance_event = AdvanceEvent::AcceptedBound;
    let accepted_only = try_policy_fixture(accepted_only).expect("valid accepted-only policy");
    let manual = evaluate_completion(&accepted_only, CompletionFact::Delivery(&manual_delivery))
        .expect("manual terminal remains observable");
    assert_eq!(manual.schedule(), ScheduleDirective::CloseOnAccepted);
    assert_eq!(manual.cursor(), CursorDirective::Never);

    let not_delivered = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::ManualConfirmedNotDelivered,
    ));
    let not_delivered =
        evaluate_completion(&accepted_policy, CompletionFact::Delivery(&not_delivered))
            .expect("manual not-delivered terminal is allowed");
    assert_eq!(not_delivered.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(not_delivered.cursor(), CursorDirective::Never);

    let mut rejected_options = fixture_policy_options();
    rejected_options.retry_policy = RetryPolicy::AuthorizedRejected {
        not_before: UtcMicros::try_new(200).expect("valid time"),
        max_attempts: NonZeroU32::new(2).expect("nonzero attempts"),
    };
    let rejected_policy = try_policy_fixture(rejected_options).expect("valid rejected policy");
    let rejected_delivery = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Rejected,
    ));
    let rejected = evaluate_completion(
        &rejected_policy,
        CompletionFact::Delivery(&rejected_delivery),
    )
    .expect("rejected terminal is allowed");
    assert_eq!(rejected.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(rejected.cursor(), CursorDirective::Never);
    assert_eq!(
        rejected.retry().eligibility(),
        RetryEligibility::RejectedAuthorizationRequired
    );
}

#[test]
fn w03_compatibility_results_require_observation_policy_and_never_complete() {
    use super::policy::{fixture_policy_options, try_policy_fixture};

    let evidence = compatibility_evidence(
        &["feishu"],
        &["feishu"],
        &[("feishu", WeakOutcomeKind::Accepted)],
    )
    .expect("valid compatibility evidence");
    let delivery = DeliveryResult::best_effort_accepted(evidence).expect("valid weak result");

    let bound = try_policy_fixture(fixture_policy_options()).expect("valid bound policy");
    assert!(evaluate_completion(&bound, CompletionFact::Delivery(&delivery)).is_err());

    let mut observation = fixture_policy_options();
    observation.cursor_policy = CursorPolicy::Never;
    observation.allowed_authority.clear();
    observation.finalizer_kind = FinalizerKind::CompatibilityObservation;
    observation.close_all_schedule_branches = false;
    let observation = try_policy_fixture(observation).expect("valid observation policy");
    let directive = evaluate_completion(&observation, CompletionFact::Delivery(&delivery))
        .expect("compatibility observation is allowed");
    assert_eq!(directive.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(directive.cursor(), CursorDirective::Never);
    assert_eq!(directive.retry().eligibility(), RetryEligibility::Never);
    assert_eq!(directive.manual(), ManualDirective::None);
}

#[test]
fn w03_policy_registration_rejects_owner_cursor_authority_conflicts() {
    use super::policy::{fixture_policy_options, try_policy_fixture};

    let mut owner_mismatch = fixture_policy_options();
    owner_mismatch.catalog_completion_owner = "other-owner";
    assert!(try_policy_fixture(owner_mismatch).is_err());

    let mut bound_without_cursor = fixture_policy_options();
    bound_without_cursor.cursor_policy = CursorPolicy::Never;
    assert!(try_policy_fixture(bound_without_cursor).is_err());

    let mut bound_without_authority = fixture_policy_options();
    bound_without_authority.allowed_authority.clear();
    assert!(try_policy_fixture(bound_without_authority).is_err());

    let mut duplicate_authority = fixture_policy_options();
    duplicate_authority.allowed_authority = vec![
        AuthorityClass::GenericCounted,
        AuthorityClass::GenericCounted,
    ];
    assert!(try_policy_fixture(duplicate_authority).is_err());

    let mut observation_with_authority = fixture_policy_options();
    observation_with_authority.cursor_policy = CursorPolicy::Never;
    observation_with_authority.finalizer_kind = FinalizerKind::CompatibilityObservation;
    assert!(try_policy_fixture(observation_with_authority).is_err());
}

#[test]
fn w03_non_terminal_facts_keep_cursor_closed_and_preserve_typed_retry() {
    use super::policy::{fixture_policy_options, try_policy_fixture};

    let mut options = fixture_policy_options();
    options.retry_policy = RetryPolicy::InputBackoff {
        not_before: UtcMicros::try_new(200).expect("valid time"),
    };
    let policy = try_policy_fixture(options).expect("valid input policy");

    let blocked = evaluate_completion(
        &policy,
        CompletionFact::BlockedOnInput {
            reason: ReasonCode::InputSourceUnavailable,
            retry_after: Some(UtcMicros::try_new(250).expect("valid time")),
        },
    )
    .expect("valid blocked input");
    assert_eq!(blocked.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(blocked.cursor(), CursorDirective::Never);
    assert_eq!(
        blocked.retry().eligibility(),
        RetryEligibility::NotBefore(UtcMicros::try_new(250).expect("valid time"))
    );

    let suppressed = evaluate_completion(
        &policy,
        CompletionFact::Suppressed {
            reason: ReasonCode::PolicyCooldownActive,
            eligible_after: Some(UtcMicros::try_new(300).expect("valid time")),
        },
    )
    .expect("valid suppression");
    assert_eq!(
        suppressed.schedule(),
        ScheduleDirective::CloseSuppressedOccurrence
    );
    assert_eq!(suppressed.cursor(), CursorDirective::Never);
    assert_eq!(
        suppressed.retry().eligibility(),
        RetryEligibility::NotBefore(UtcMicros::try_new(300).expect("valid time"))
    );

    let retryable = evaluate_completion(
        &policy,
        CompletionFact::RetryableFailure {
            reason: ReasonCode::InputSourceUnready,
            retry_after: None,
        },
    )
    .expect("valid retryable preparation failure");
    assert_eq!(retryable.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(retryable.cursor(), CursorDirective::Never);
    assert_eq!(
        retryable.retry().eligibility(),
        RetryEligibility::NotBefore(UtcMicros::try_new(200).expect("valid time"))
    );

    let permanent = evaluate_completion(
        &policy,
        CompletionFact::PermanentFailure {
            reason: ReasonCode::InputNamespaceViolation,
        },
    )
    .expect("valid permanent failure");
    assert_eq!(permanent.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(permanent.cursor(), CursorDirective::Never);
    assert_eq!(permanent.retry().eligibility(), RetryEligibility::Never);

    let delivery_blocked = DeliveryResult::blocked(ReasonCode::FinalizerBindingMismatch);
    let delivery_blocked =
        evaluate_completion(&policy, CompletionFact::Delivery(&delivery_blocked))
            .expect("valid blocked delivery");
    assert_eq!(delivery_blocked.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(delivery_blocked.cursor(), CursorDirective::Never);
    assert_eq!(
        delivery_blocked.retry().eligibility(),
        RetryEligibility::Never
    );
}
