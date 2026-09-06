use super::{
    classify_durable_state, derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id,
    evaluate_completion, AdvanceEvent, AudienceId, AuthorityClass, BusinessDate, CalendarId,
    ChannelId, CompatId, CompatibilityEvidenceRef, CompletionEligibility, CompletionFact,
    CompletionOwnerId, CursorDirective, CursorPolicy, DeliveryAuthority, DeliveryResult,
    DeliveryResultView, DisabledPolicy, DurableStateProjection, ExternalId, FinalizerKind,
    IntentIdentityMaterial, ManualDirective, Namespace, NoDataPolicy, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ReasonCode, RetryEligibility,
    RetryPolicy, RunId, ScheduleDirective, ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId,
    Sha256Digest, SourceContractId, SourceContractVersion, SourceProvider, SourceRef, SourceRefId,
    SubjectId, TerminalDisposition, UnitId, UtcMicros, WeakOutcome, WeakOutcomeKind,
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
    use super::identity::occurrence_preimage_fixture;

    assert_eq!(
        occurrence_preimage_fixture(&occurrence_material()),
        b"OccurrenceId/v1\0{\"business_date\":\"2026-09-06\",\"occurrence_family\":\"daily\",\"occurrence_key\":\"close\"}"
    );
    let id = derive_occurrence_id(&occurrence_material());
    assert_eq!(
        id.as_str(),
        "5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a"
    );
}

#[test]
fn w01_canonical_json_escaping_is_infallible_and_exact() {
    use super::identity::canonical_string_preimage_fixture;

    let input = "quote\" slash\\ line\n tab\t 中文 \u{001f}";
    let expected = "Fixture/v1\0{\"value\":\"quote\\\" slash\\\\ line\\n tab\\t 中文 \\u001f\"}";
    assert_eq!(
        canonical_string_preimage_fixture(input),
        expected.as_bytes()
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
    assert!(UnitId::try_new("x".repeat(513)).is_err());
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
fn w01_outer_identities_bind_unit_producer_owner_and_namespace() {
    let occurrence = occurrence_material();
    let raw_id = derive_occurrence_id(&occurrence);
    let schedule = |namespace: Namespace, unit: &str, producer: &str, owner: &str| {
        derive_schedule_occurrence_id(&ScheduleOccurrenceIdentityMaterial::new(
            namespace,
            UnitId::try_new(unit.to_owned()).expect("valid unit"),
            ProducerId::try_new(producer.to_owned()).expect("valid producer"),
            ScheduleOrTriggerId::try_new("schedule-close".to_owned()).expect("valid schedule"),
            CalendarId::try_new("a-share-calendar".to_owned()).expect("valid calendar"),
            occurrence.clone(),
            CompletionOwnerId::try_new(owner.to_owned()).expect("valid owner"),
            SourceContractId::try_new("close-v1".to_owned()).expect("valid source"),
        ))
    };
    let production = schedule(
        Namespace::Production,
        "MU-close",
        "close-scheduled",
        "owner-close",
    );
    assert_ne!(
        production,
        schedule(
            Namespace::Production,
            "MU-other",
            "close-scheduled",
            "owner-close"
        )
    );
    assert_ne!(
        production,
        schedule(
            Namespace::Production,
            "MU-close",
            "other-producer",
            "owner-close"
        )
    );
    assert_ne!(
        production,
        schedule(
            Namespace::Production,
            "MU-close",
            "close-scheduled",
            "other-owner"
        )
    );
    assert_ne!(
        production,
        schedule(
            Namespace::test(RunId::try_new("run-1".to_owned()).expect("valid run")),
            "MU-close",
            "close-scheduled",
            "owner-close"
        )
    );

    let intent = |namespace: Namespace, unit: &str, owner: &str| {
        derive_intent_id(&IntentIdentityMaterial::new(
            namespace,
            UnitId::try_new(unit.to_owned()).expect("valid unit"),
            CompletionOwnerId::try_new(owner.to_owned()).expect("valid owner"),
            SourceContractId::try_new("close-v1".to_owned()).expect("valid source"),
            raw_id.clone(),
            SubjectId::Global,
            AudienceId::try_new("portfolio-owner".to_owned()).expect("valid audience"),
        ))
    };
    let production_intent = intent(Namespace::Production, "MU-close", "owner-close");
    assert_ne!(
        production_intent,
        intent(Namespace::Production, "MU-other", "owner-close")
    );
    assert_ne!(
        production_intent,
        intent(Namespace::Production, "MU-close", "other-owner")
    );
    assert_ne!(
        production_intent,
        intent(
            Namespace::test(RunId::try_new("run-1".to_owned()).expect("valid run")),
            "MU-close",
            "owner-close"
        )
    );
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
    let partial = DeliveryResult::partially_accepted(mixed).expect("partial matrix");
    assert_eq!(partial.authority_class(), DeliveryAuthority::Compat);
    assert_eq!(
        partial.completion_eligibility(),
        CompletionEligibility::Never
    );

    let none = DeliveryResult::no_channel_configured();
    assert_eq!(
        none.reason_code(),
        Some(ReasonCode::TransportNoChannelConfigured)
    );
    assert_eq!(none.authority_class(), DeliveryAuthority::Compat);
    assert_eq!(none.completion_eligibility(), CompletionEligibility::Never);

    let unknown = compatibility_evidence(
        &["feishu"],
        &["feishu"],
        &[("feishu", WeakOutcomeKind::Unknown)],
    )
    .expect("valid unknown evidence");
    let failed = DeliveryResult::all_channels_failed(unknown).expect("all-failed matrix");
    assert_eq!(failed.authority_class(), DeliveryAuthority::Compat);
    assert_eq!(
        failed.completion_eligibility(),
        CompletionEligibility::Never
    );
    assert!(matches!(
        failed.view(),
        DeliveryResultView::AllChannelsFailed(evidence)
            if evidence.weak_outcomes()[0].kind() == WeakOutcomeKind::Unknown
    ));
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
    use std::collections::BTreeSet;

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
    assert_eq!(
        ReasonCode::ALL
            .iter()
            .map(|code| code.as_str().split('.').next().expect("reason namespace"))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "activation",
            "finalizer",
            "input",
            "intent",
            "operator",
            "policy",
            "schedule",
            "shadow",
            "transport",
        ])
    );
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
            .evaluate(now, 1, true, ReasonCode::TransportRejected)
            .eligibility(),
        RetryEligibility::NotBefore(not_before)
    );
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
    assert_eq!(
        rejected
            .evaluate(not_before, 0, true, ReasonCode::FinalizerCasConflict)
            .eligibility(),
        RetryEligibility::Never
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

    let deliveries = [
        DeliveryResult::best_effort_accepted(
            compatibility_evidence(
                &["feishu"],
                &["feishu"],
                &[("feishu", WeakOutcomeKind::Accepted)],
            )
            .expect("valid all-accepted evidence"),
        )
        .expect("valid best-effort result"),
        DeliveryResult::partially_accepted(
            compatibility_evidence(
                &["feishu", "wechat"],
                &["feishu", "wechat"],
                &[
                    ("feishu", WeakOutcomeKind::Accepted),
                    ("wechat", WeakOutcomeKind::Rejected),
                ],
            )
            .expect("valid partial evidence"),
        )
        .expect("valid partial result"),
        DeliveryResult::no_channel_configured(),
        DeliveryResult::all_channels_failed(
            compatibility_evidence(
                &["feishu"],
                &["feishu"],
                &[("feishu", WeakOutcomeKind::Unknown)],
            )
            .expect("valid failed evidence"),
        )
        .expect("valid all-failed result"),
    ];

    let bound = try_policy_fixture(fixture_policy_options()).expect("valid bound policy");
    let mut observation = fixture_policy_options();
    observation.cursor_policy = CursorPolicy::Never;
    observation.allowed_authority.clear();
    observation.finalizer_kind = FinalizerKind::CompatibilityObservation;
    observation.close_all_schedule_branches = false;
    let observation = try_policy_fixture(observation).expect("valid observation policy");

    for delivery in &deliveries {
        assert!(evaluate_completion(&bound, CompletionFact::Delivery(delivery)).is_err());
        let directive = evaluate_completion(&observation, CompletionFact::Delivery(delivery))
            .expect("compatibility observation is allowed");
        assert_eq!(directive.schedule(), ScheduleDirective::KeepOpen);
        assert_eq!(directive.cursor(), CursorDirective::Never);
        assert_eq!(directive.retry().eligibility(), RetryEligibility::Never);
        assert_eq!(directive.manual(), ManualDirective::None);
    }
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

#[test]
fn w04_run_context_golden_hash_is_stable() {
    use super::context::{context_fixture, run_context_preimage_fixture, ContextFixtureCase};
    use super::{PhaseEpic, TriggerView};

    let context =
        context_fixture(ContextFixtureCase::ValidScheduled).expect("valid catalog-bound context");

    assert_eq!(context.schema_version(), 1);
    assert_eq!(context.run_id().as_str(), "run-20260907-090500");
    assert_eq!(context.unit_id().as_str(), "MU-auction");
    assert_eq!(context.namespace(), &Namespace::Production);
    assert_eq!(context.business_date().as_str(), "2026-09-07");
    assert_eq!(context.calendar_date().as_str(), "2026-09-07");
    assert_eq!(context.phase(), PhaseEpic::Auction);
    assert!(matches!(
        context.trigger(),
        TriggerView::Scheduled { schedule_id } if schedule_id.as_str() == "auction-main"
    ));
    assert_eq!(
        context.occurrence().as_str(),
        "d5881448142d550c9e73bd4c7d61ed8da16587a6beee0f6dced16c5420b11e0d"
    );
    assert_eq!(
        context.captured_business_time().get(),
        1_788_743_100_000_000
    );
    assert_eq!(context.activation_generation(), 7);
    assert_eq!(
        context.build_commit().as_str(),
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(context.catalog_sha256(), &digest('c'));
    assert_eq!(
        context.source_contract_version().as_str(),
        "auction-source-v2"
    );
    assert_eq!(context.template_version().as_str(), "auction-card-v3");

    let expected = concat!(
        "RunContext/v1\0{",
        "\"activation_generation\":7,",
        "\"build_commit\":\"0123456789abcdef0123456789abcdef01234567\",",
        "\"business_date\":\"2026-09-07\",",
        "\"calendar_date\":\"2026-09-07\",",
        "\"captured_business_time\":1788743100000000,",
        "\"catalog_sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",",
        "\"namespace\":{\"kind\":\"Production\",\"run_id\":null},",
        "\"occurrence\":\"d5881448142d550c9e73bd4c7d61ed8da16587a6beee0f6dced16c5420b11e0d\",",
        "\"phase\":\"Auction\",",
        "\"run_id\":\"run-20260907-090500\",",
        "\"schema_version\":1,",
        "\"source_contract_version\":\"auction-source-v2\",",
        "\"template_version\":\"auction-card-v3\",",
        "\"trigger\":{\"kind\":\"Scheduled\",\"schedule_id\":\"auction-main\"},",
        "\"unit_id\":\"MU-auction\"}"
    );
    assert_eq!(run_context_preimage_fixture(&context), expected.as_bytes());
    assert_eq!(
        context.canonical_sha256().as_str(),
        "ced27f93ce01baa5c775beef415f73fda5b065727ee9d246c91ba9807d7276b8"
    );
}

#[test]
fn w04_run_context_exposes_only_the_captured_trigger_branch() {
    use super::context::{context_fixture, ContextFixtureCase};
    use super::TriggerView;

    let event = context_fixture(ContextFixtureCase::ValidEvent).expect("valid event context");
    assert!(matches!(
        event.trigger(),
        TriggerView::Event {
            producer_id,
            source_ref,
        } if producer_id.as_str() == "auction-event"
            && source_ref.source_ref_id().as_str() == "source-event-1"
            && source_ref.source_contract_id().as_str() == "auction-source"
    ));

    let manual = context_fixture(ContextFixtureCase::ValidManual).expect("valid manual context");
    assert!(matches!(
        manual.trigger(),
        TriggerView::Manual {
            command_id,
            authenticated_operator_ref,
        } if command_id.as_str() == "command-1"
            && authenticated_operator_ref.as_str() == "operator-session-1"
    ));
}

#[test]
fn w04_run_context_rejects_invalid_values_and_catalog_mismatches() {
    use super::context::{context_fixture, ContextFixtureCase};
    use super::{CalendarDate, GitSha40};

    assert!(CalendarDate::parse("2026-9-7").is_err());
    assert!(GitSha40::parse("ABCDEF0123456789ABCDEF0123456789ABCDEF01").is_err());
    assert!(GitSha40::parse("0123").is_err());

    for case in [
        ContextFixtureCase::WrongSchedule,
        ContextFixtureCase::WrongEventProducer,
        ContextFixtureCase::WrongEventSourceContract,
        ContextFixtureCase::WrongOccurrenceFamily,
        ContextFixtureCase::WrongTestNamespaceRun,
    ] {
        assert!(
            context_fixture(case).is_err(),
            "case {case:?} must fail closed"
        );
    }
}

fn w04_source_ref(id: &str, contract: &str, content: char) -> SourceRef {
    SourceRef::new(
        SourceRefId::try_new(id.to_owned()).expect("valid source ref id"),
        SourceProvider::try_new("fixture-provider".to_owned()).expect("valid provider"),
        ExternalId::try_new(format!("external-{id}")).expect("valid external id"),
        SourceContractId::try_new(contract.to_owned()).expect("valid source contract"),
        digest(content),
    )
}

fn w04_model_ref(model: &str, output: char) -> super::ModelOutputRef {
    super::ModelOutputRef::new(
        super::ModelId::try_new(model.to_owned()).expect("valid model"),
        super::ModelVersion::try_new("2026-09".to_owned()).expect("valid model version"),
        digest('1'),
        digest(output),
        super::ProtectedRef::try_new(format!("vault://model/{model}/{output}"))
            .expect("valid protected ref"),
    )
}

fn w04_present_facts(
    source_contract_id: &str,
    source_contract_version: &str,
    source_refs: Vec<SourceRef>,
    source_times: Vec<super::SourceTime>,
    model_output_refs: Vec<super::ModelOutputRef>,
) -> super::Result<super::CapturedFacts> {
    super::CapturedFacts::try_new(
        SourceContractId::try_new(source_contract_id.to_owned())?,
        SourceContractVersion::try_new(source_contract_version.to_owned())?,
        source_refs,
        super::ExactBytes::new(br#"{"items":[{"code":"000001.SZ"}]}"#.to_vec()),
        source_times,
        super::FactsPresence::Present,
        model_output_refs,
    )
}

#[test]
fn w04_exact_bytes_hash_the_original_payload_without_rewriting() {
    use super::ExactBytes;

    let compact = ExactBytes::new(br#"{"a":1}"#.to_vec());
    let spaced = ExactBytes::new(br#"{ "a": 1 }"#.to_vec());
    assert_eq!(compact.as_bytes(), br#"{"a":1}"#);
    assert_eq!(spaced.as_bytes(), br#"{ "a": 1 }"#);
    assert_ne!(compact.sha256(), spaced.sha256());

    let non_utf8 = ExactBytes::new(vec![0xff, 0x00, 0x41]);
    assert_eq!(non_utf8.as_bytes(), &[0xff, 0x00, 0x41]);
    assert_eq!(non_utf8.len(), 3);
    assert_eq!(
        non_utf8.sha256().as_str(),
        "0fa3e62511779f0398b77cad37b3cc4763bb96253b91fcd61500f8a979ad9920"
    );

    let sensitive = ExactBytes::new(b"portfolio-secret-fact".to_vec());
    let debug = format!("{sensitive:?}");
    assert!(!debug.contains("portfolio-secret-fact"));
    assert!(debug.contains("len"));
    assert!(debug.contains(sensitive.sha256().as_str()));
}

#[test]
fn w04_source_times_are_total_ordered_and_preserve_unknown() {
    use super::{SourceRefId, SourceTime, SourceTimeKind};

    let one = w04_source_ref("source-1", "auction-source", 'a');
    let two = w04_source_ref("source-2", "auction-source", 'b');
    let at = UtcMicros::try_new(1_788_743_100_000_001).expect("valid observed time");
    let valid = w04_present_facts(
        "auction-source",
        "auction-source-v2",
        vec![one.clone(), two.clone()],
        vec![
            SourceTime::observed_at(one.source_ref_id().clone(), Some(at)),
            SourceTime::as_of(two.source_ref_id().clone(), None),
        ],
        Vec::new(),
    )
    .expect("total ordered source times");
    assert_eq!(
        valid.provider_observed_at()[0].kind(),
        SourceTimeKind::ObservedAt
    );
    assert_eq!(valid.provider_observed_at()[0].value(), Some(at));
    assert_eq!(valid.provider_observed_at()[1].kind(), SourceTimeKind::AsOf);
    assert_eq!(valid.provider_observed_at()[1].value(), None);

    let malformed = [
        vec![SourceTime::observed_at(
            one.source_ref_id().clone(),
            Some(at),
        )],
        vec![
            SourceTime::as_of(two.source_ref_id().clone(), None),
            SourceTime::observed_at(one.source_ref_id().clone(), Some(at)),
        ],
        vec![
            SourceTime::observed_at(one.source_ref_id().clone(), Some(at)),
            SourceTime::as_of(
                SourceRefId::try_new("unknown-source".to_owned()).expect("valid id"),
                None,
            ),
        ],
    ];
    for source_times in malformed {
        assert!(w04_present_facts(
            "auction-source",
            "auction-source-v2",
            vec![one.clone(), two.clone()],
            source_times,
            Vec::new(),
        )
        .is_err());
    }
}

#[test]
fn w04_source_and_model_references_are_ordered_unique_and_frozen() {
    use super::facts::capture_fixture;
    use super::SourceTime;

    let one = w04_source_ref("source-1", "auction-source", 'a');
    let two = w04_source_ref("source-2", "auction-source", 'b');
    let first_model = w04_model_ref("model-a", '2');
    let second_model = w04_model_ref("model-b", '3');

    assert!(w04_present_facts(
        "auction-source",
        "auction-source-v2",
        vec![one.clone(), one.clone()],
        vec![
            SourceTime::observed_at(one.source_ref_id().clone(), None),
            SourceTime::observed_at(one.source_ref_id().clone(), None),
        ],
        Vec::new(),
    )
    .is_err());
    assert!(w04_present_facts(
        "auction-source",
        "auction-source-v2",
        vec![one.clone()],
        vec![SourceTime::observed_at(one.source_ref_id().clone(), None)],
        vec![first_model.clone(), first_model.clone()],
    )
    .is_err());

    let mut capture = capture_fixture().expect("valid capture capability");
    let snapshot = capture
        .capture_once(|_| {
            Ok(w04_present_facts(
                "auction-source",
                "auction-source-v2",
                vec![two.clone(), one.clone()],
                vec![
                    SourceTime::as_of(two.source_ref_id().clone(), None),
                    SourceTime::observed_at(one.source_ref_id().clone(), None),
                ],
                vec![second_model.clone(), first_model.clone()],
            )
            .expect("valid captured facts"))
        })
        .expect("first capture succeeds");
    let facts = snapshot.facts();
    assert_eq!(facts.source_refs(), &[two, one]);
    assert_eq!(facts.model_output_refs(), &[second_model, first_model]);
    assert_eq!(facts.source_contract_id().as_str(), "auction-source");
    assert_eq!(
        facts.source_contract_version().as_str(),
        "auction-source-v2"
    );
    assert_eq!(
        facts.run_context_sha256(),
        &capture.context().canonical_sha256()
    );
    assert_eq!(facts.facts_sha256(), facts.canonical_facts().sha256());
    assert!(!facts.verified_empty());
}

#[test]
fn w04_capture_rejects_source_contract_id_and_version_drift() {
    use super::facts::capture_fixture;
    use super::SourceTime;

    for (source_contract_id, source_contract_version) in [
        ("other-source", "auction-source-v2"),
        ("auction-source", "auction-source-v3"),
    ] {
        let source = w04_source_ref("source-1", source_contract_id, 'a');
        let mut capture = capture_fixture().expect("valid capture capability");
        let result = capture.capture_once(|_| {
            Ok(w04_present_facts(
                source_contract_id,
                source_contract_version,
                vec![source.clone()],
                vec![SourceTime::observed_at(
                    source.source_ref_id().clone(),
                    None,
                )],
                Vec::new(),
            )
            .expect("locally consistent facts"))
        });
        assert!(matches!(
            result,
            Err(super::PreparationError::InvalidCapturedFacts(_))
        ));
        assert_eq!(capture.state(), super::CaptureStateView::Failed);
    }
}

#[test]
fn w04_verified_empty_requires_evidence_bound_to_context_and_source() {
    use super::facts::capture_fixture;
    use super::{CapturedFacts, ExactBytes, FactsPresence, SourceTime};

    let mut capture = capture_fixture().expect("valid capture capability");
    let wrong_occurrence = derive_occurrence_id(&occurrence_material());
    let wrong_evidence = super::VerifiedEmptyEvidenceRef::new(
        wrong_occurrence,
        SourceContractId::try_new("auction-source".to_owned()).expect("valid source"),
        digest('e'),
        UtcMicros::try_new(1_788_743_100_000_002).expect("valid verified time"),
    );
    let source = w04_source_ref("source-empty", "auction-source", 'e');
    let result = capture.capture_once(|_| {
        Ok(CapturedFacts::try_new(
            SourceContractId::try_new("auction-source".to_owned()).expect("valid source"),
            SourceContractVersion::try_new("auction-source-v2".to_owned())
                .expect("valid source version"),
            vec![source.clone()],
            ExactBytes::new(br#"{"items":[]}"#.to_vec()),
            vec![SourceTime::observed_at(
                source.source_ref_id().clone(),
                None,
            )],
            FactsPresence::VerifiedEmpty(wrong_evidence),
            Vec::new(),
        )
        .expect("locally consistent empty facts"))
    });
    assert!(matches!(
        result,
        Err(super::PreparationError::InvalidCapturedFacts(_))
    ));

    let mut capture = capture_fixture().expect("valid capture capability");
    let source = w04_source_ref("source-empty", "auction-source", 'e');
    let snapshot = capture
        .capture_once(|context| {
            let evidence = super::VerifiedEmptyEvidenceRef::new(
                context.occurrence().clone(),
                SourceContractId::try_new("auction-source".to_owned()).expect("valid source"),
                digest('e'),
                UtcMicros::try_new(1_788_743_100_000_002).expect("valid verified time"),
            );
            Ok(CapturedFacts::try_new(
                SourceContractId::try_new("auction-source".to_owned()).expect("valid source"),
                SourceContractVersion::try_new("auction-source-v2".to_owned())
                    .expect("valid source version"),
                vec![source.clone()],
                ExactBytes::new(br#"{"items":[]}"#.to_vec()),
                vec![SourceTime::observed_at(
                    source.source_ref_id().clone(),
                    None,
                )],
                FactsPresence::VerifiedEmpty(evidence),
                Vec::new(),
            )
            .expect("valid verified empty facts"))
        })
        .expect("bound verified empty succeeds");
    assert!(snapshot.facts().verified_empty());
}

#[test]
fn w04_active_and_shadow_share_the_same_immutable_snapshot() {
    use super::facts::capture_fixture;
    use super::SourceTime;

    let source = w04_source_ref("source-1", "auction-source", 'a');
    let model = w04_model_ref("model-a", '2');
    let mut capture = capture_fixture().expect("valid capture capability");
    let active = capture
        .capture_once(|_| {
            Ok(w04_present_facts(
                "auction-source",
                "auction-source-v2",
                vec![source.clone()],
                vec![SourceTime::observed_at(
                    source.source_ref_id().clone(),
                    None,
                )],
                vec![model.clone()],
            )
            .expect("valid facts"))
        })
        .expect("first capture succeeds");
    let shadow = active.clone();
    assert!(active.shares_instance_with(&shadow));
    assert_eq!(active, shadow);
    assert_eq!(
        active.facts().model_output_refs(),
        std::slice::from_ref(&model)
    );
    assert_eq!(
        shadow.facts().model_output_refs(),
        std::slice::from_ref(&model)
    );

    let mut separate_capture = capture_fixture().expect("valid separate capability");
    let separate = separate_capture
        .capture_once(|_| {
            Ok(w04_present_facts(
                "auction-source",
                "auction-source-v2",
                vec![source.clone()],
                vec![SourceTime::observed_at(
                    source.source_ref_id().clone(),
                    None,
                )],
                vec![model.clone()],
            )
            .expect("same value facts"))
        })
        .expect("separate capture succeeds");
    assert_eq!(
        active.facts().canonical_sha256(),
        separate.facts().canonical_sha256()
    );
    assert!(!active.shares_instance_with(&separate));
    assert_ne!(active, separate);
}

#[test]
fn w04_second_capture_is_rejected_before_the_external_call() {
    use std::cell::Cell;

    use super::facts::capture_fixture;
    use super::SourceTime;

    let calls = Cell::new(0_u64);
    let source = w04_source_ref("source-1", "auction-source", 'a');
    let mut capture = capture_fixture().expect("valid capture capability");
    capture
        .capture_once(|_| {
            calls.set(calls.get() + 1);
            Ok(w04_present_facts(
                "auction-source",
                "auction-source-v2",
                vec![source.clone()],
                vec![SourceTime::observed_at(
                    source.source_ref_id().clone(),
                    None,
                )],
                Vec::new(),
            )
            .expect("valid facts"))
        })
        .expect("first capture succeeds");
    let second = capture.capture_once(|_| {
        calls.set(calls.get() + 1);
        panic!("second external acquisition must not be called")
    });
    assert!(matches!(
        second,
        Err(super::PreparationError::AlreadyAttempted {
            state: super::CaptureStateView::Sealed,
        })
    ));
    assert_eq!(calls.get(), 1);
    assert_eq!(capture.attempt_count(), 1);
    assert_eq!(capture.rejected_count(), 1);
    assert_eq!(capture.state(), super::CaptureStateView::Sealed);
}

#[test]
fn w04_failed_capture_is_single_use_and_preserves_the_typed_reason() {
    use std::cell::Cell;

    use super::facts::capture_fixture;

    let calls = Cell::new(0_u64);
    let mut capture = capture_fixture().expect("valid capture capability");
    let first = capture.capture_once(|_| {
        calls.set(calls.get() + 1);
        Err(super::PreparationError::AcquisitionFailed {
            reason: ReasonCode::InputSourceUnavailable,
        })
    });
    assert!(matches!(
        first,
        Err(super::PreparationError::AcquisitionFailed {
            reason: ReasonCode::InputSourceUnavailable,
        })
    ));
    let second = capture.capture_once(|_| {
        calls.set(calls.get() + 1);
        panic!("failed capture must not call the provider again")
    });
    assert!(matches!(
        second,
        Err(super::PreparationError::AlreadyAttempted {
            state: super::CaptureStateView::Failed,
        })
    ));
    assert_eq!(calls.get(), 1);
    assert_eq!(capture.attempt_count(), 1);
    assert_eq!(capture.rejected_count(), 1);
}

#[test]
fn w04_capture_unwind_does_not_reopen_the_capability() {
    use std::cell::Cell;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use super::facts::capture_fixture;

    let calls = Cell::new(0_u64);
    let mut capture = capture_fixture().expect("valid capture capability");
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _: std::result::Result<super::PreparedFactsSnapshot, super::PreparationError> = capture
            .capture_once(|_| {
                calls.set(calls.get() + 1);
                panic!("fixture acquisition panic")
            });
    }));
    assert!(unwind.is_err());
    assert_eq!(capture.state(), super::CaptureStateView::Capturing);

    let second = capture.capture_once(|_| {
        calls.set(calls.get() + 1);
        panic!("capture must stay closed after unwind")
    });
    assert!(matches!(
        second,
        Err(super::PreparationError::AlreadyAttempted {
            state: super::CaptureStateView::Capturing,
        })
    ));
    assert_eq!(calls.get(), 1);
    assert_eq!(capture.attempt_count(), 1);
    assert_eq!(capture.rejected_count(), 1);
}

fn w05_projection_snapshot(
    source_refs: Vec<SourceRef>,
    source_times: Vec<super::SourceTime>,
    model_output_refs: Vec<super::ModelOutputRef>,
) -> (super::DecisionProjector, super::PreparedFactsSnapshot) {
    use super::facts::capture_fixture;
    use super::projection::projector_fixture;

    let mut capture = capture_fixture().expect("valid W05 capture capability");
    let projector = projector_fixture(capture.context()).expect("valid catalog-bound projector");
    let snapshot = capture
        .capture_once(|_| {
            Ok(w04_present_facts(
                "auction-source",
                "auction-source-v2",
                source_refs,
                source_times,
                model_output_refs,
            )
            .expect("valid W05 facts"))
        })
        .expect("W05 facts captured once");
    (projector, snapshot)
}

fn w05_semantic_input() -> super::SemanticInput {
    super::SemanticInput::new(
        SubjectId::entity("000001.SZ".to_owned()).expect("valid subject"),
        super::Severity::Important,
        super::Suppression::eligible(),
    )
}

#[test]
fn w05_monitor_kind_is_the_exact_catalog_closed_set() {
    use std::collections::BTreeSet;

    use super::MonitorKind;

    let expected = [
        "HoldingEvent",
        "DailyReport",
        "Announcement",
        "AuctionVolume",
        "VirtualWatch",
        "LimitBoards",
        "SectorTop",
        "FundInflow",
        "AuctionRepush",
        "FactorIC",
        "SectorTier",
        "CapitalVerify",
        "WeeklySOP",
        "StockPick",
        "IndustryChain",
        "TurnoverTop",
        "CandidateBoard",
        "NewsRanked",
        "AccountMode",
        "DataMode",
        "HoldingPlan",
        "T0Advice",
        "CandidateTriggered",
        "ForbiddenOps",
        "PaperTrade",
        "PaperSell",
        "SnapshotStale",
        "AttributionDaily",
        "G5bAttribution",
        "CloseCall",
        "ReviewMarket",
        "ReviewLhb",
        "ReviewSignal",
        "ReviewFailure",
        "TomorrowWatch",
        "EventCalendar",
        "ReviewProviderTopN",
        "PositionReview",
        "ReviewBacktest",
        "WatchlistTracking",
        "PreopenNewsHot",
        "IntradayMarket",
        "NewsCatalyst",
        "SectorAnomaly",
        "NewsToIdea",
        "CatalystReview",
        "IndustryChainIntraday",
        "PostFixedPriceOrder",
        "PostFixedPriceFill",
        "StPriceLimitChanged",
        "EtfClosingCallAuction",
        "BlockTradeIntradayConfirm",
        "BlockTradePriceRange",
        "PaperReview",
        "CandidateInvalidated",
        "IpoListingApproval",
        "IpoProspectus",
        "IpoCatalyst",
        "PolicyHit",
        "EarningsBeat",
        "EarningsMiss",
        "AnalystUpgrade",
        "MarketActionAlert",
        "NewsFlashCritical",
        "NewsFlashAggregated",
    ];
    assert_eq!(MonitorKind::ALL.len(), 65);
    assert_eq!(
        MonitorKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        expected.iter().copied().collect::<BTreeSet<_>>().len(),
        expected.len()
    );
    for (expected, kind) in expected.iter().zip(MonitorKind::ALL) {
        assert_eq!(MonitorKind::try_from(*expected).expect("known kind"), kind);
    }
    assert!(MonitorKind::try_from("UnknownKind").is_err());
}

#[test]
fn w05_semantic_projection_is_deterministic_and_context_bound() {
    let source = w04_source_ref("source-1", "auction-source", 'a');
    let model = w04_model_ref("model-a", '2');
    let (projector, snapshot) = w05_projection_snapshot(
        vec![source.clone()],
        vec![super::SourceTime::observed_at(
            source.source_ref_id().clone(),
            None,
        )],
        vec![model],
    );

    let first = projector
        .project_semantics(&snapshot, w05_semantic_input())
        .expect("pure projection");
    let rebuilt = projector
        .project_semantics(&snapshot, w05_semantic_input())
        .expect("same pure projection");
    assert_eq!(first, rebuilt);
    assert_eq!(first.canonical_bytes(), rebuilt.canonical_bytes());
    assert_eq!(first.sha256(), rebuilt.sha256());
    assert_eq!(first.sha256(), first.canonical_bytes().sha256());
    assert_eq!(first.audience().as_str(), "portfolio-owner");
    assert_eq!(
        first.monitor_kind(),
        Some(super::MonitorKind::AuctionVolume)
    );
    assert_eq!(first.sub_kind(), &super::SubKind::None);
    assert_eq!(first.occurrence(), projector.occurrence());
    assert_eq!(
        first.business_subject(),
        &SubjectId::entity("000001.SZ".to_owned()).unwrap()
    );
    assert_eq!(first.severity(), super::Severity::Important);
    assert_eq!(first.suppression(), &super::Suppression::Eligible);
    assert_eq!(
        first.completion_policy_id().as_str(),
        "auction-notification"
    );
    assert_eq!(first.completion_policy_version().as_str(), "policy-v1");
    assert_eq!(first.template_id().as_str(), "auction-card");
    assert_eq!(first.template_version().as_str(), "auction-card-v3");
    assert_eq!(
        first.sha256().as_str(),
        "W05_SEMANTIC_PROJECTION_GOLDEN_TO_BE_REPLACED"
    );
}

#[test]
fn w05_evidence_fingerprint_preserves_source_and_model_order() {
    let one = w04_source_ref("source-1", "auction-source", 'a');
    let two = w04_source_ref("source-2", "auction-source", 'b');
    let model_one = w04_model_ref("model-a", '2');
    let model_two = w04_model_ref("model-b", '3');
    let (first_projector, first_snapshot) = w05_projection_snapshot(
        vec![one.clone(), two.clone()],
        vec![
            super::SourceTime::observed_at(one.source_ref_id().clone(), None),
            super::SourceTime::as_of(two.source_ref_id().clone(), None),
        ],
        vec![model_one.clone(), model_two.clone()],
    );
    let (reversed_projector, reversed_snapshot) = w05_projection_snapshot(
        vec![two.clone(), one.clone()],
        vec![
            super::SourceTime::as_of(two.source_ref_id().clone(), None),
            super::SourceTime::observed_at(one.source_ref_id().clone(), None),
        ],
        vec![model_two, model_one],
    );

    let first = first_projector
        .project_semantics(&first_snapshot, w05_semantic_input())
        .unwrap();
    let reversed = reversed_projector
        .project_semantics(&reversed_snapshot, w05_semantic_input())
        .unwrap();
    assert_ne!(
        first.evidence_fingerprint(),
        reversed.evidence_fingerprint()
    );
    assert_ne!(first.sha256(), reversed.sha256());
}

#[test]
fn w05_projection_rejects_facts_from_another_context_before_render() {
    use super::context::{context_fixture, ContextFixtureCase};
    use super::projection::projector_fixture;

    let source = w04_source_ref("source-1", "auction-source", 'a');
    let (_, snapshot) = w05_projection_snapshot(
        vec![source.clone()],
        vec![super::SourceTime::observed_at(
            source.source_ref_id().clone(),
            None,
        )],
        Vec::new(),
    );
    let other_context = context_fixture(ContextFixtureCase::ValidEvent).unwrap();
    let other_projector = projector_fixture(&other_context).unwrap();
    assert!(matches!(
        other_projector.project_semantics(&snapshot, w05_semantic_input()),
        Err(super::ProjectionError::ContextFactsMismatch)
    ));
}

#[test]
fn w05_projection_value_types_are_closed_and_validated() {
    assert!(super::SubKind::try_registered(String::new()).is_err());
    assert_eq!(super::SubKind::none(), super::SubKind::None);
    assert_eq!(super::Severity::ALL.len(), 4);
    assert_eq!(
        super::Suppression::suppressed(
            ReasonCode::PolicyCooldownActive,
            Some(UtcMicros::try_new(1_788_743_200_000_000).unwrap()),
        ),
        super::Suppression::Suppressed {
            reason: ReasonCode::PolicyCooldownActive,
            eligible_after: Some(UtcMicros::try_new(1_788_743_200_000_000).unwrap()),
        }
    );
}
