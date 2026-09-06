use super::{
    classify_durable_state, derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id,
    AudienceId, BusinessDate, CalendarId, ChannelId, CompatId, CompatibilityEvidenceRef,
    CompletionEligibility, CompletionOwnerId, DeliveryAuthority, DeliveryResult,
    DeliveryResultView, DurableStateProjection, IntentIdentityMaterial, Namespace,
    OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, ReasonCode, RunId,
    ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest, SourceContractId,
    SourceContractVersion, SubjectId, TerminalDisposition, UnitId, UtcMicros, WeakOutcome,
    WeakOutcomeKind,
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
