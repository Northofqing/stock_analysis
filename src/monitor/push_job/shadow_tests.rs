use std::{cell::Cell, collections::BTreeSet, time::Duration};

use super::context::{context_fixture, ContextFixtureCase};
use super::facts::capture_fixture;
use super::policy::{disabled_fixture, fixture_policy_options, try_policy_fixture};
use super::projection::{projector_fixture, ProjectionBinding};
use super::*;

fn digest() -> Sha256Digest {
    Sha256Digest::parse("shadow fixture", &"a".repeat(64)).unwrap()
}

fn captured(context: &RunContext, empty: bool, payload: &[u8]) -> CapturedFacts {
    let contract = SourceContractId::try_new("auction-source".to_owned()).unwrap();
    let source = SourceRef::new(
        SourceRefId::try_new("source-shadow".to_owned()).unwrap(),
        SourceProvider::try_new("provider-shadow".to_owned()).unwrap(),
        ExternalId::try_new("external-shadow".to_owned()).unwrap(),
        contract.clone(),
        digest(),
    );
    CapturedFacts::try_new(
        contract.clone(),
        context.source_contract_version().clone(),
        vec![source.clone()],
        ExactBytes::new(payload.to_vec()),
        vec![SourceTime::observed_at(
            source.source_ref_id().clone(),
            None,
        )],
        if empty {
            FactsPresence::VerifiedEmpty(VerifiedEmptyEvidenceRef::new(
                context.occurrence().clone(),
                contract,
                digest(),
                context.captured_business_time(),
            ))
        } else {
            FactsPresence::Present
        },
        vec![ModelOutputRef::new(
            ModelId::try_new("secret-model-name".to_owned()).unwrap(),
            ModelVersion::try_new("model-v1".to_owned()).unwrap(),
            digest(),
            ExactBytes::new(b"secret-model-output".to_vec())
                .sha256()
                .clone(),
            ProtectedRef::try_new("vault://secret-model-output".to_owned()).unwrap(),
        )],
    )
    .unwrap()
}

fn fixture(empty: bool, payload: &[u8]) -> (PreparationCapture, PreparedFactsSnapshot) {
    let mut capture = capture_fixture().unwrap();
    let facts = capture
        .capture_once(|context| Ok(captured(context, empty, payload)))
        .unwrap();
    (capture, facts)
}

fn other_fixture(case: ContextFixtureCase) -> (PreparationCapture, PreparedFactsSnapshot) {
    let mut capture = PreparationCapture::new(
        context_fixture(case).unwrap(),
        SourceContractId::try_new("auction-source".to_owned()).unwrap(),
    );
    let facts = capture
        .capture_once(|context| Ok(captured(context, false, b"secret-facts")))
        .unwrap();
    (capture, facts)
}

fn projector(context: &RunContext) -> DecisionProjector {
    if context.unit_id().as_str() == "MU-auction" {
        projector_fixture(context).unwrap()
    } else {
        DecisionProjector::try_new(
            context,
            ProjectionBinding::new(
                context.unit_id().clone(),
                AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
                Some(MonitorKind::AuctionVolume),
                SubKind::none(),
                CompletionOwnerId::try_new("owner-other".to_owned()).unwrap(),
                CompletionPolicyId::try_new("auction-notification".to_owned()).unwrap(),
                CompletionPolicyVersion::try_new("policy-v1".to_owned()).unwrap(),
                TemplateId::try_new("auction-card".to_owned()).unwrap(),
            ),
        )
        .unwrap()
    }
}

fn semantics(severity: Severity) -> SemanticInput {
    SemanticInput::new(SubjectId::Global, severity, Suppression::eligible())
}

fn ready_parts(
    context: &RunContext,
    facts: &PreparedFactsSnapshot,
    text: &[u8],
    severity: Severity,
) -> (JobDecision, SemanticProjection) {
    let projection = projector(context)
        .project_semantics(facts, semantics(severity))
        .unwrap();
    let decision = projector(context)
        .prepare_ready(facts.clone(), semantics(severity))
        .unwrap()
        .render_once(|_| text.to_vec())
        .unwrap();
    (decision, projection)
}

fn completion() -> CompletionDirective {
    evaluate_completion(
        &try_policy_fixture(fixture_policy_options()).unwrap(),
        CompletionFact::ExplicitDisabled(disabled_fixture(ReasonCode::PolicyDisabled)),
    )
    .unwrap()
}

fn ready<'a>(context: &'a RunContext, facts: &'a PreparedFactsSnapshot) -> ShadowObservation<'a> {
    let (decision, projection) =
        ready_parts(context, facts, b"secret-rendered-text", Severity::Info);
    ShadowObservation::new(
        context,
        facts,
        decision,
        Some(projection),
        ReasonCode::IntentCreated,
        completion(),
        ShadowDiagnostics::default(),
    )
}

#[derive(PartialEq)]
struct BusinessRecord {
    symbol: String,
    price_bits: u64,
    hidden_raw_metric: i64,
}

#[derive(PartialEq)]
struct BusinessProposal {
    message: String,
    records: Vec<BusinessRecord>,
    notifications: BTreeSet<String>,
    ownership_marker: Box<u8>,
}

fn business_proposal() -> BusinessProposal {
    BusinessProposal {
        message: "sensitive-business-message".to_owned(),
        records: vec![
            BusinessRecord {
                symbol: "SENSITIVE-A".to_owned(),
                price_bits: 42.25_f64.to_bits(),
                hidden_raw_metric: 7_001,
            },
            BusinessRecord {
                symbol: "SENSITIVE-B".to_owned(),
                price_bits: 9.75_f64.to_bits(),
                hidden_raw_metric: 8_002,
            },
        ],
        notifications: BTreeSet::from([
            "sensitive-notification-a".to_owned(),
            "sensitive-notification-b".to_owned(),
        ]),
        ownership_marker: Box::new(17),
    }
}

#[test]
fn business_proposal_equal_independent_outputs_match_and_return_original_legacy_value_once() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let expected_context = capture.context();
    let expected_facts = &facts;
    let calls = Cell::new([0_u8; 2]);
    let legacy = business_proposal();
    let legacy_marker = (&*legacy.ownership_marker) as *const u8;

    let mut execution = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |context, observed_facts, _| {
            calls.set([calls.get()[0] + 1, calls.get()[1]]);
            assert!(std::ptr::eq(expected_context, context));
            assert!(expected_facts.shares_instance_with(observed_facts));
            Ok(ShadowBusinessObservation::new(
                ready(context, observed_facts),
                Some(legacy),
            ))
        },
        |context, observed_facts, _| {
            calls.set([calls.get()[0], calls.get()[1] + 1]);
            assert!(std::ptr::eq(expected_context, context));
            assert!(expected_facts.shares_instance_with(observed_facts));
            Ok(ShadowBusinessObservation::new(
                ready(context, observed_facts),
                Some(business_proposal()),
            ))
        },
    );

    assert!(execution.report().is_match());
    assert_eq!(calls.get(), [1, 1]);
    let returned = execution.take_legacy_proposal().unwrap();
    assert_eq!((&*returned.ownership_marker) as *const u8, legacy_marker);
    assert!(execution.take_legacy_proposal().is_none());
    assert!(execution.report().is_match());
}

#[derive(Clone, Copy, Debug)]
enum Case {
    Ready,
    NoData,
    Disabled(ReasonCode),
    Blocked(ReasonCode, Option<UtcMicros>),
    Suppressed(ReasonCode, Option<UtcMicros>),
    Retryable(ReasonCode, Option<UtcMicros>),
    Permanent(ReasonCode),
}

fn business_ready<'a>(
    context: &'a RunContext,
    facts: &'a PreparedFactsSnapshot,
    proposal: Option<BusinessProposal>,
) -> ShadowBusinessObservation<'a, BusinessProposal> {
    ShadowBusinessObservation::new(ready(context, facts), proposal)
}

fn business_non_ready<'a>(
    context: &'a RunContext,
    facts: &'a PreparedFactsSnapshot,
    proposal: Option<BusinessProposal>,
) -> ShadowBusinessObservation<'a, BusinessProposal> {
    ShadowBusinessObservation::new(
        observe(context, facts, Case::Disabled(ReasonCode::PolicyDisabled)),
        proposal,
    )
}

fn business_invalid_no_data<'a>(
    context: &'a RunContext,
    facts: &'a PreparedFactsSnapshot,
    evidence_facts: &PreparedFactsSnapshot,
) -> ShadowBusinessObservation<'a, BusinessProposal> {
    ShadowBusinessObservation::new(
        ShadowObservation::new(
            context,
            facts,
            projector(context)
                .decide_no_data(evidence_facts, ReasonCode::IntentNoData)
                .unwrap(),
            None,
            ReasonCode::IntentNoData,
            completion(),
            ShadowDiagnostics::default(),
        ),
        None,
    )
}

fn observe<'a>(
    context: &'a RunContext,
    facts: &'a PreparedFactsSnapshot,
    case: Case,
) -> ShadowObservation<'a> {
    let projector = projector(context);
    let (decision, projection, reason) = match case {
        Case::Ready => return ready(context, facts),
        Case::NoData => (
            projector
                .decide_no_data(facts, ReasonCode::IntentNoData)
                .unwrap(),
            None,
            ReasonCode::IntentNoData,
        ),
        Case::Disabled(reason) => (projector.decide_disabled(reason).unwrap(), None, reason),
        Case::Blocked(reason, time) => (
            projector.decide_blocked_on_input(reason, time).unwrap(),
            None,
            reason,
        ),
        Case::Suppressed(reason, time) => {
            let input = || {
                SemanticInput::new(
                    SubjectId::Global,
                    Severity::Info,
                    Suppression::suppressed(reason, time),
                )
            };
            let projection = projector.project_semantics(facts, input()).unwrap();
            (
                projector.decide_suppressed(facts, input()).unwrap(),
                Some(projection),
                reason,
            )
        }
        Case::Retryable(reason, time) => (
            projector.decide_retryable_failure(reason, time).unwrap(),
            None,
            reason,
        ),
        Case::Permanent(reason) => (
            projector.decide_permanent_failure(reason).unwrap(),
            None,
            reason,
        ),
    };
    ShadowObservation::new(
        context,
        facts,
        decision,
        projection,
        reason,
        completion(),
        ShadowDiagnostics::default(),
    )
}

fn assert_invalid(report: &ShadowReport, path: ShadowPath, binding: ShadowInvalidBinding) {
    assert!(!report.is_match());
    assert_eq!(report.status(path), ShadowPathStatus::InvalidObservation);
    assert!(report
        .differences()
        .contains(&ShadowDifference::InvalidObservation { path, binding }));
    assert!(report.reasons().contains(&ReasonCode::ShadowSemanticDiff));
}

#[test]
fn one_capture_shares_context_arc_and_model_output_with_both_paths() {
    let captures = Cell::new(0);
    let calls = Cell::new(0);
    let mut capture = capture_fixture().unwrap();
    let facts = capture
        .capture_once(|context| {
            captures.set(captures.get() + 1);
            Ok(captured(context, false, b"secret-facts"))
        })
        .unwrap();
    let context = capture.context();
    let callback = |observed_context, observed_facts, _: &ShadowDeniedEffects| {
        calls.set(calls.get() + 1);
        let observation = ready(observed_context, observed_facts);
        assert!(std::ptr::eq(context, observed_context));
        assert!(facts.shares_instance_with(observed_facts));
        assert!(std::ptr::eq(
            facts.facts().model_output_refs(),
            observed_facts.facts().model_output_refs()
        ));
        assert_eq!(observed_facts.facts().model_output_refs().len(), 1);
        Ok(observation)
    };
    let report = execute_shadow(context, &facts, callback, callback);
    assert!(report.is_match());
    assert_eq!(captures.get(), 1);
    assert_eq!(capture.attempt_count(), 1);
    assert_eq!(calls.get(), 2);
    assert_eq!(report.unit_id(), context.unit_id());
    assert_eq!(report.run_context_sha256(), &context.canonical_sha256());
    assert_eq!(
        report.prepared_facts_sha256(),
        &facts.facts().canonical_sha256()
    );
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_eq!(report.status(path), ShadowPathStatus::Completed);
        assert!(report.counts(path).is_zero());
    }
}

#[test]
fn context_facts_mismatch_executes_neither_path_including_changed_business_time() {
    let (_, facts) = fixture(false, b"secret-facts");
    for case in [
        ContextFixtureCase::ValidEvent,
        ContextFixtureCase::ValidOtherCapturedTime,
    ] {
        let context = context_fixture(case).unwrap();
        let report = execute_shadow(
            &context,
            &facts,
            |_, _, _| panic!("old must not execute"),
            |_, _, _| panic!("new must not execute"),
        );
        assert!(!report.is_match());
        assert_eq!(
            report.differences(),
            &[ShadowDifference::ContextFactsBinding]
        );
        for path in [ShadowPath::Old, ShadowPath::New] {
            assert_eq!(report.status(path), ShadowPathStatus::NotExecuted);
            assert!(report.counts(path).is_zero());
        }
    }
}

#[test]
fn equal_independent_context_and_capture_instances_are_rejected_but_arc_clone_is_valid() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let (other_capture, other_facts) = fixture(false, b"secret-facts");
    assert_eq!(capture.context(), other_capture.context());
    assert_eq!(
        facts.facts().canonical_sha256(),
        other_facts.facts().canonical_sha256()
    );
    assert!(!facts.shares_instance_with(&other_facts));
    let report = execute_shadow(
        capture.context(),
        &facts,
        |_, _, _| Ok(ready(other_capture.context(), &other_facts)),
        |context, facts, _| Ok(ready(context, facts)),
    );
    assert_invalid(
        &report,
        ShadowPath::Old,
        ShadowInvalidBinding::ContextInstance,
    );
    assert_invalid(
        &report,
        ShadowPath::Old,
        ShadowInvalidBinding::FactsInstance,
    );
    let cloned_facts = facts.clone();
    let report = execute_shadow(
        capture.context(),
        &facts,
        |context, _, _| Ok(ready(context, &cloned_facts)),
        |context, facts, _| Ok(ready(context, facts)),
    );
    assert!(report.is_match());
}

#[test]
fn all_seven_real_decision_branches_compare_equal_and_detect_other_branches() {
    let cases = [
        Case::Ready,
        Case::NoData,
        Case::Disabled(ReasonCode::PolicyDisabled),
        Case::Blocked(ReasonCode::InputSourceUnavailable, None),
        Case::Suppressed(ReasonCode::PolicyCooldownActive, None),
        Case::Retryable(ReasonCode::InputSourceUnready, None),
        Case::Permanent(ReasonCode::InputNamespaceViolation),
    ];
    for case in cases {
        let (capture, facts) = fixture(matches!(case, Case::NoData), b"secret-facts");
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(observe(context, facts, case)),
            |context, facts, _| Ok(observe(context, facts, case)),
        );
        assert!(report.is_match(), "{case:?}: {report:?}");
        let other = if matches!(case, Case::Disabled(_)) {
            Case::Permanent(ReasonCode::InputEvidenceInvalid)
        } else {
            Case::Disabled(ReasonCode::PolicyDisabled)
        };
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(observe(context, facts, case)),
            |context, facts, _| Ok(observe(context, facts, other)),
        );
        assert!(!report.is_match());
        assert!(report
            .differences()
            .contains(&ShadowDifference::JobDecision));
    }
}

#[test]
fn non_ready_reason_retry_and_suppression_business_time_fields_are_exact() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let time = Some(UtcMicros::try_new(200).unwrap());
    let cases = [
        (
            Case::Disabled(ReasonCode::PolicyDisabled),
            Case::Disabled(ReasonCode::PolicyOptInDisabled),
        ),
        (
            Case::Blocked(ReasonCode::InputSourceUnavailable, None),
            Case::Blocked(ReasonCode::InputSourceUnready, None),
        ),
        (
            Case::Blocked(ReasonCode::InputSourceUnavailable, None),
            Case::Blocked(ReasonCode::InputSourceUnavailable, time),
        ),
        (
            Case::Suppressed(ReasonCode::PolicyCooldownActive, None),
            Case::Suppressed(ReasonCode::PolicyDailyBudgetFull, None),
        ),
        (
            Case::Suppressed(ReasonCode::PolicyCooldownActive, None),
            Case::Suppressed(ReasonCode::PolicyCooldownActive, time),
        ),
        (
            Case::Retryable(ReasonCode::InputSourceUnavailable, None),
            Case::Retryable(ReasonCode::InputSourceUnready, None),
        ),
        (
            Case::Retryable(ReasonCode::InputSourceUnavailable, None),
            Case::Retryable(ReasonCode::InputSourceUnavailable, time),
        ),
        (
            Case::Permanent(ReasonCode::InputNamespaceViolation),
            Case::Permanent(ReasonCode::InputEvidenceInvalid),
        ),
    ];
    for (old, new) in cases {
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(observe(context, facts, old)),
            |context, facts, _| Ok(observe(context, facts, new)),
        );
        assert!(!report.is_match());
        assert!(
            report
                .differences()
                .contains(&ShadowDifference::JobDecision),
            "{report:?}"
        );
    }
}

#[test]
fn ready_whitespace_and_semantic_changes_have_typed_deterministic_differences() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for (text, severity, expected) in [
        (
            &b"secret-rendered-text "[..],
            Severity::Info,
            vec![
                ShadowDifference::JobDecision,
                ShadowDifference::RenderedSha256,
                ShadowDifference::RenderedBytes,
            ],
        ),
        (
            &b"secret-rendered-text"[..],
            Severity::Important,
            vec![
                ShadowDifference::JobDecision,
                ShadowDifference::SemanticProjection,
            ],
        ),
    ] {
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(ready(context, facts)),
            |context, facts, _| {
                let (decision, projection) = ready_parts(context, facts, text, severity);
                Ok(ShadowObservation::new(
                    context,
                    facts,
                    decision,
                    Some(projection),
                    ReasonCode::IntentCreated,
                    completion(),
                    ShadowDiagnostics::default(),
                ))
            },
        );
        assert!(!report.is_match());
        assert_eq!(report.differences(), expected);
        assert_eq!(report.reasons(), [ReasonCode::ShadowSemanticDiff]);
    }
}

#[test]
fn no_data_from_other_facts_cannot_pass_even_when_both_decisions_agree() {
    let (capture, facts) = fixture(true, b"empty-facts");
    let (_, other_facts) = fixture(true, b"other-empty-facts");
    let callback = |context, facts, _: &ShadowDeniedEffects| {
        let decision = projector(context)
            .decide_no_data(&other_facts, ReasonCode::IntentNoData)
            .unwrap();
        Ok(ShadowObservation::new(
            context,
            facts,
            decision,
            None,
            ReasonCode::IntentNoData,
            completion(),
            ShadowDiagnostics::default(),
        ))
    };
    let report = execute_shadow(capture.context(), &facts, callback, callback);
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_invalid(&report, path, ShadowInvalidBinding::NoDataEvidence);
    }
}

#[test]
fn ready_rejects_missing_or_mismatched_actual_projection_even_when_both_agree() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for missing in [true, false] {
        let callback = |context, facts, _: &ShadowDeniedEffects| {
            let (decision, _) = ready_parts(context, facts, b"same", Severity::Info);
            let projection = if missing {
                None
            } else {
                Some(
                    projector(context)
                        .project_semantics(facts, semantics(Severity::Important))
                        .unwrap(),
                )
            };
            Ok(ShadowObservation::new(
                context,
                facts,
                decision,
                projection,
                ReasonCode::IntentCreated,
                completion(),
                ShadowDiagnostics::default(),
            ))
        };
        let report = execute_shadow(capture.context(), &facts, callback, callback);
        for path in [ShadowPath::Old, ShadowPath::New] {
            assert_invalid(
                &report,
                path,
                if missing {
                    ShadowInvalidBinding::ReadyProjectionMissing
                } else {
                    ShadowInvalidBinding::ReadyProjectionMismatch
                },
            );
        }
    }
}

#[test]
fn projection_presence_and_independent_reason_are_compared() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for change_projection in [true, false] {
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(ready(context, facts)),
            |context, facts, _| {
                let (decision, projection) =
                    ready_parts(context, facts, b"secret-rendered-text", Severity::Info);
                Ok(ShadowObservation::new(
                    context,
                    facts,
                    decision,
                    if change_projection {
                        None
                    } else {
                        Some(projection)
                    },
                    if change_projection {
                        ReasonCode::IntentCreated
                    } else {
                        ReasonCode::IntentDispatchClaimed
                    },
                    completion(),
                    ShadowDiagnostics::default(),
                ))
            },
        );
        assert!(!report.is_match());
        assert!(report.differences().contains(&if change_projection {
            ShadowDifference::SemanticProjection
        } else {
            ShadowDifference::ReasonCode
        }));
    }
}

#[test]
fn non_ready_self_reported_reason_cannot_disagree_with_actual_decision() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let callback = |context, facts, _: &ShadowDeniedEffects| {
        let decision = projector(context)
            .decide_disabled(ReasonCode::PolicyDisabled)
            .unwrap();
        Ok(ShadowObservation::new(
            context,
            facts,
            decision,
            None,
            ReasonCode::PolicyOptInDisabled,
            completion(),
            ShadowDiagnostics::default(),
        ))
    };
    let report = execute_shadow(capture.context(), &facts, callback, callback);
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_invalid(&report, path, ShadowInvalidBinding::DecisionReason);
    }
}

#[test]
fn ready_validates_unit_occurrence_context_business_time_and_facts_binding() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for (case, binding) in [
        (
            ContextFixtureCase::ValidOtherUnit,
            ShadowInvalidBinding::ReadyUnit,
        ),
        (
            ContextFixtureCase::ValidOtherOccurrence,
            ShadowInvalidBinding::ReadyOccurrence,
        ),
        (
            ContextFixtureCase::ValidOtherCapturedTime,
            ShadowInvalidBinding::ReadyContext,
        ),
        (
            ContextFixtureCase::ValidEvent,
            ShadowInvalidBinding::ReadyContext,
        ),
    ] {
        let (other_capture, other_facts) = other_fixture(case);
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(ready(context, facts)),
            |context, facts, _| {
                let (decision, projection) = ready_parts(
                    other_capture.context(),
                    &other_facts,
                    b"secret-rendered-text",
                    Severity::Info,
                );
                Ok(ShadowObservation::new(
                    context,
                    facts,
                    decision,
                    Some(projection),
                    ReasonCode::IntentCreated,
                    completion(),
                    ShadowDiagnostics::default(),
                ))
            },
        );
        assert_invalid(&report, ShadowPath::New, binding);
        assert_invalid(&report, ShadowPath::New, ShadowInvalidBinding::ReadyFacts);
    }
    let (_, other_facts) = fixture(false, b"other-facts");
    let report = execute_shadow(
        capture.context(),
        &facts,
        |context, facts, _| Ok(ready(context, facts)),
        |context, facts, _| {
            let (decision, projection) = ready_parts(
                context,
                &other_facts,
                b"secret-rendered-text",
                Severity::Info,
            );
            Ok(ShadowObservation::new(
                context,
                facts,
                decision,
                Some(projection),
                ReasonCode::IntentCreated,
                completion(),
                ShadowDiagnostics::default(),
            ))
        },
    );
    assert_invalid(&report, ShadowPath::New, ShadowInvalidBinding::ReadyFacts);
}

#[test]
fn only_three_diagnostic_fields_can_change_without_affecting_match() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for changed in 0..3 {
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| Ok(ready(context, facts)),
            |context, facts, _| {
                let (decision, projection) =
                    ready_parts(context, facts, b"secret-rendered-text", Severity::Info);
                let diagnostics = ShadowDiagnostics {
                    attempt_id: (changed == 0)
                        .then(|| AttemptId::try_new("secret-attempt".to_owned()).unwrap()),
                    latency: (changed == 1).then(|| Duration::from_millis(10)),
                    diagnostic_timestamp: (changed == 2).then(|| UtcMicros::try_new(50).unwrap()),
                };
                let observation = ShadowObservation::new(
                    context,
                    facts,
                    decision,
                    Some(projection),
                    ReasonCode::IntentCreated,
                    completion(),
                    diagnostics,
                );
                assert_eq!(observation.diagnostics().latency.is_some(), changed == 1);
                Ok(observation)
            },
        );
        assert!(report.is_match(), "{report:?}");
    }
}

#[test]
fn completion_schedule_cursor_retry_and_manual_proposals_are_compared() {
    use super::delivery::verified_terminal_fixture;
    let policy = try_policy_fixture(fixture_policy_options()).unwrap();
    let mut open_options = fixture_policy_options();
    open_options.close_all_schedule_branches = false;
    let open_policy = try_policy_fixture(open_options).unwrap();
    let accepted = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Accepted,
    ));
    let manual = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::ManualConfirmedAccepted,
    ));
    let uncertain = DeliveryResult::from_verified_terminal(verified_terminal_fixture(
        TerminalDisposition::Uncertain,
    ));
    let evaluate = |fact| evaluate_completion(&policy, fact).unwrap();
    let pairs = [
        (
            completion(),
            evaluate_completion(
                &open_policy,
                CompletionFact::ExplicitDisabled(disabled_fixture(ReasonCode::PolicyDisabled)),
            )
            .unwrap(),
            ShadowDifference::CompletionSchedule,
        ),
        (
            evaluate(CompletionFact::Delivery(&accepted)),
            evaluate(CompletionFact::Delivery(&manual)),
            ShadowDifference::CompletionCursor,
        ),
        (
            evaluate(CompletionFact::Suppressed {
                reason: ReasonCode::PolicyCooldownActive,
                eligible_after: None,
            }),
            evaluate(CompletionFact::Suppressed {
                reason: ReasonCode::PolicyCooldownActive,
                eligible_after: Some(UtcMicros::try_new(100).unwrap()),
            }),
            ShadowDifference::CompletionRetry,
        ),
        (
            evaluate(CompletionFact::Delivery(&accepted)),
            evaluate(CompletionFact::Delivery(&uncertain)),
            ShadowDifference::CompletionManual,
        ),
    ];
    let (capture, facts) = fixture(false, b"secret-facts");
    for (old, new, expected) in pairs {
        let callback = |context, facts, proposal| {
            let (decision, projection) = ready_parts(context, facts, b"same", Severity::Info);
            Ok(ShadowObservation::new(
                context,
                facts,
                decision,
                Some(projection),
                ReasonCode::IntentCreated,
                proposal,
                ShadowDiagnostics::default(),
            ))
        };
        let report = execute_shadow(
            capture.context(),
            &facts,
            |context, facts, _| callback(context, facts, old),
            |context, facts, _| callback(context, facts, new),
        );
        assert!(!report.is_match());
        assert!(report.differences().contains(&expected));
        assert!(!report
            .differences()
            .contains(&ShadowDifference::JobDecision));
    }
}

#[test]
fn each_denied_capability_is_counted_and_never_executes_the_requested_action() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for effect in ShadowEffect::ALL {
        for attempted_path in [ShadowPath::Old, ShadowPath::New] {
            let actions = Cell::new(0);
            let callback = |context, facts, effects: &ShadowDeniedEffects, path| {
                if path == attempted_path {
                    // The action would run only if the capability granted it. The adapter then
                    // deliberately ignores denial and still returns an equal real decision.
                    let result = effects.request(effect);
                    if result.is_ok() {
                        actions.set(actions.get() + 1);
                    }
                    assert_eq!(result.unwrap_err().effect(), effect);
                }
                Ok(ready(context, facts))
            };
            let report = execute_shadow(
                capture.context(),
                &facts,
                |context, facts, effects| callback(context, facts, effects, ShadowPath::Old),
                |context, facts, effects| callback(context, facts, effects, ShadowPath::New),
            );
            assert!(!report.is_match());
            assert!(report.differences().is_empty());
            assert_eq!(report.reasons(), [ReasonCode::ShadowSideEffectAttempted]);
            assert_eq!(actions.get(), 0);
            assert_eq!(capture.attempt_count(), 1);
            for path in [ShadowPath::Old, ShadowPath::New] {
                assert_eq!(report.status(path), ShadowPathStatus::Completed);
                for counted in ShadowEffect::ALL {
                    assert_eq!(
                        report.counts(path).get(counted),
                        u64::from(path == attempted_path && counted == effect)
                    );
                }
            }
        }
    }
}

#[test]
fn mixed_attempts_have_exact_per_path_counts_and_new_runs_cannot_erase_prior_failure() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let failed = execute_shadow(
        capture.context(),
        &facts,
        |context, facts, effects| {
            for (index, effect) in ShadowEffect::ALL.into_iter().enumerate() {
                for _ in 0..=index {
                    let _ = effects.request(effect);
                }
            }
            Ok(ready(context, facts))
        },
        |context, facts, effects| {
            for _ in 0..3 {
                let _ = effects.request(ShadowEffect::TransportSend);
            }
            let (decision, projection) = ready_parts(context, facts, b"different", Severity::Info);
            Ok(ShadowObservation::new(
                context,
                facts,
                decision,
                Some(projection),
                ReasonCode::IntentCreated,
                completion(),
                ShadowDiagnostics::default(),
            ))
        },
    );
    let before = format!("{failed:?}");
    for (index, effect) in ShadowEffect::ALL.into_iter().enumerate() {
        assert_eq!(
            failed.counts(ShadowPath::Old).get(effect),
            (index + 1) as u64
        );
        assert_eq!(
            failed.counts(ShadowPath::New).get(effect),
            if effect == ShadowEffect::TransportSend {
                3
            } else {
                0
            }
        );
    }
    assert_eq!(
        failed.reasons(),
        [
            ReasonCode::ShadowSemanticDiff,
            ReasonCode::ShadowSideEffectAttempted
        ]
    );
    let clean = execute_shadow(
        capture.context(),
        &facts,
        |context, facts, _| Ok(ready(context, facts)),
        |context, facts, _| Ok(ready(context, facts)),
    );
    assert!(clean.is_match());
    assert!(!failed.is_match());
    assert_eq!(before, format!("{failed:?}"));
}

#[test]
fn explicit_callback_failure_preserves_both_statuses_and_side_effect_failure() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for both_fail in [false, true] {
        let report = execute_shadow(
            capture.context(),
            &facts,
            |_, _, effects| {
                let _ = effects.request(ShadowEffect::BusinessDbWrite);
                Err(ShadowCallbackFailure)
            },
            |context, facts, _| {
                if both_fail {
                    Err(ShadowCallbackFailure)
                } else {
                    Ok(ready(context, facts))
                }
            },
        );
        assert!(!report.is_match());
        assert_eq!(
            report.status(ShadowPath::Old),
            ShadowPathStatus::CallbackFailed
        );
        assert_eq!(
            report.status(ShadowPath::New),
            if both_fail {
                ShadowPathStatus::CallbackFailed
            } else {
                ShadowPathStatus::Completed
            }
        );
        assert!(report
            .differences()
            .contains(&ShadowDifference::CallbackFailed(ShadowPath::Old)));
        assert_eq!(
            report
                .counts(ShadowPath::Old)
                .get(ShadowEffect::BusinessDbWrite),
            1
        );
        assert_eq!(
            report.reasons(),
            [
                ReasonCode::ShadowSemanticDiff,
                ReasonCode::ShadowSideEffectAttempted
            ]
        );
    }
}

#[test]
fn debug_and_error_surfaces_never_expose_input_rendered_model_or_callback_content() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let observation = ready(capture.context(), &facts);
    let report = execute_shadow(
        capture.context(),
        &facts,
        |context, facts, _| Ok(ready(context, facts)),
        |_, _, effects| {
            let denied = effects.request(ShadowEffect::LlmRecompute).unwrap_err();
            assert_eq!(denied.to_string(), "shadow effect denied: LlmRecompute");
            let _private_callback_error = "secret-callback-error";
            Err(ShadowCallbackFailure)
        },
    );
    let debug = format!(
        "{observation:?} {report:?} {:?} {}",
        ShadowDiagnostics {
            attempt_id: Some(AttemptId::try_new("secret-attempt".to_owned()).unwrap()),
            ..ShadowDiagnostics::default()
        },
        ShadowCallbackFailure
    );
    for secret in [
        "secret-facts",
        "secret-rendered-text",
        "secret-model-name",
        "secret-model-output",
        "secret-attempt",
        "secret-callback-error",
    ] {
        assert!(!debug.contains(secret), "leaked {secret}");
    }
    for effect in ShadowEffect::ALL {
        assert!(debug.contains(effect.as_str()));
    }
}

#[test]
fn business_proposal_all_complete_fields_and_record_order_are_compared() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for change in 0..5 {
        let mut execution = execute_shadow_with_proposals(
            capture.context(),
            &facts,
            |context, facts, _| Ok(business_ready(context, facts, Some(business_proposal()))),
            |context, facts, _| {
                let mut proposal = business_proposal();
                match change {
                    0 => proposal.message = "different-business-message".to_owned(),
                    1 => proposal.records[0].price_bits = 42.5_f64.to_bits(),
                    2 => proposal.records[0].hidden_raw_metric = 7_002,
                    3 => proposal.records.swap(0, 1),
                    4 => {
                        assert!(proposal.notifications.remove("sensitive-notification-b"));
                    }
                    _ => unreachable!(),
                }
                Ok(business_ready(context, facts, Some(proposal)))
            },
        );
        assert_eq!(
            execution.report().differences(),
            &[ShadowBusinessDifference::BusinessProposal],
            "change {change}"
        );
        for path in [ShadowPath::Old, ShadowPath::New] {
            assert_eq!(execution.report().status(path), ShadowPathStatus::Completed);
        }
        assert_eq!(
            execution.report().reasons(),
            [ReasonCode::ShadowSemanticDiff]
        );
        assert!(execution.take_legacy_proposal().is_some());
    }
}

#[test]
fn business_proposal_invalid_presence_is_path_specific_even_when_both_paths_agree() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let mut ready_missing = execute_shadow_with_proposals::<BusinessProposal, _, _>(
        capture.context(),
        &facts,
        |context, facts, _| Ok(business_ready(context, facts, None)),
        |context, facts, _| Ok(business_ready(context, facts, None)),
    );
    assert!(!ready_missing.report().is_match());
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_eq!(
            ready_missing.report().status(path),
            ShadowPathStatus::InvalidObservation
        );
        assert!(ready_missing.report().differences().contains(
            &ShadowBusinessDifference::InvalidObservation {
                path,
                binding: ShadowBusinessInvalidBinding::ReadyProposalMissing,
            }
        ));
    }
    assert!(ready_missing.take_legacy_proposal().is_none());

    let mut non_ready_present = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |context, facts, _| {
            Ok(business_non_ready(
                context,
                facts,
                Some(business_proposal()),
            ))
        },
        |context, facts, _| {
            Ok(business_non_ready(
                context,
                facts,
                Some(business_proposal()),
            ))
        },
    );
    assert!(!non_ready_present.report().is_match());
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_eq!(
            non_ready_present.report().status(path),
            ShadowPathStatus::InvalidObservation
        );
        assert!(non_ready_present.report().differences().contains(
            &ShadowBusinessDifference::InvalidObservation {
                path,
                binding: ShadowBusinessInvalidBinding::NonReadyProposalPresent,
            }
        ));
    }
    assert!(non_ready_present.take_legacy_proposal().is_none());
}

#[test]
fn business_proposal_callback_failures_run_both_paths_and_apply_legacy_retention_rules() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let calls = Cell::new([0_u8; 2]);
    let mut new_failed = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |context, facts, _| {
            calls.set([calls.get()[0] + 1, calls.get()[1]]);
            Ok(business_ready(context, facts, Some(business_proposal())))
        },
        |_, _, _| {
            calls.set([calls.get()[0], calls.get()[1] + 1]);
            Err(ShadowCallbackFailure)
        },
    );
    assert_eq!(calls.get(), [1, 1]);
    assert_eq!(
        new_failed.report().status(ShadowPath::New),
        ShadowPathStatus::CallbackFailed
    );
    assert!(new_failed
        .report()
        .differences()
        .contains(&ShadowBusinessDifference::Shadow(
            ShadowDifference::CallbackFailed(ShadowPath::New)
        )));
    assert!(new_failed.take_legacy_proposal().is_some());

    let calls = Cell::new([0_u8; 2]);
    let mut old_failed = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |_, _, _| {
            calls.set([calls.get()[0] + 1, calls.get()[1]]);
            Err(ShadowCallbackFailure)
        },
        |context, facts, _| {
            calls.set([calls.get()[0], calls.get()[1] + 1]);
            Ok(business_ready(context, facts, Some(business_proposal())))
        },
    );
    assert_eq!(calls.get(), [1, 1]);
    assert!(old_failed.take_legacy_proposal().is_none());

    let calls = Cell::new([0_u8; 2]);
    let both_failed = execute_shadow_with_proposals::<BusinessProposal, _, _>(
        capture.context(),
        &facts,
        |_, _, _| {
            calls.set([calls.get()[0] + 1, calls.get()[1]]);
            Err(ShadowCallbackFailure)
        },
        |_, _, _| {
            calls.set([calls.get()[0], calls.get()[1] + 1]);
            Err(ShadowCallbackFailure)
        },
    );
    assert_eq!(calls.get(), [1, 1]);
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_eq!(
            both_failed.report().status(path),
            ShadowPathStatus::CallbackFailed
        );
        assert!(both_failed
            .report()
            .differences()
            .contains(&ShadowBusinessDifference::Shadow(
                ShadowDifference::CallbackFailed(path)
            )));
    }
}

#[test]
fn business_proposal_context_and_observation_bindings_cannot_be_hidden_by_equal_payloads() {
    let (_, facts) = fixture(false, b"secret-facts");
    let wrong_context = context_fixture(ContextFixtureCase::ValidEvent).unwrap();
    let calls = Cell::new(0);
    let mut mismatch = execute_shadow_with_proposals::<BusinessProposal, _, _>(
        &wrong_context,
        &facts,
        |_, _, _| {
            calls.set(calls.get() + 1);
            unreachable!()
        },
        |_, _, _| {
            calls.set(calls.get() + 1);
            unreachable!()
        },
    );
    assert_eq!(calls.get(), 0);
    assert_eq!(
        mismatch.report().differences(),
        &[ShadowBusinessDifference::Shadow(
            ShadowDifference::ContextFactsBinding
        )]
    );
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert_eq!(
            mismatch.report().status(path),
            ShadowPathStatus::NotExecuted
        );
    }
    assert!(mismatch.take_legacy_proposal().is_none());

    let (capture, facts) = fixture(false, b"secret-facts");
    let (other_capture, other_facts) = fixture(false, b"secret-facts");
    let mut wrong_instances = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |_, _, _| {
            Ok(business_ready(
                other_capture.context(),
                &other_facts,
                Some(business_proposal()),
            ))
        },
        |context, facts, _| Ok(business_ready(context, facts, Some(business_proposal()))),
    );
    for binding in [
        ShadowInvalidBinding::ContextInstance,
        ShadowInvalidBinding::FactsInstance,
    ] {
        assert!(wrong_instances.report().differences().contains(
            &ShadowBusinessDifference::Shadow(ShadowDifference::InvalidObservation {
                path: ShadowPath::Old,
                binding,
            })
        ));
    }
    assert!(wrong_instances.take_legacy_proposal().is_none());

    let mut invalid_ready = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |context, facts, _| {
            let (decision, _) = ready_parts(context, facts, b"same", Severity::Info);
            Ok(ShadowBusinessObservation::new(
                ShadowObservation::new(
                    context,
                    facts,
                    decision,
                    None,
                    ReasonCode::IntentCreated,
                    completion(),
                    ShadowDiagnostics::default(),
                ),
                Some(business_proposal()),
            ))
        },
        |context, facts, _| Ok(business_ready(context, facts, Some(business_proposal()))),
    );
    assert!(invalid_ready
        .report()
        .differences()
        .contains(&ShadowBusinessDifference::Shadow(
            ShadowDifference::InvalidObservation {
                path: ShadowPath::Old,
                binding: ShadowInvalidBinding::ReadyProjectionMissing,
            }
        )));
    assert!(invalid_ready.take_legacy_proposal().is_none());

    let (empty_capture, empty_facts) = fixture(true, b"empty-facts");
    let (_, other_empty_facts) = fixture(true, b"other-empty-facts");
    let invalid_no_data = execute_shadow_with_proposals(
        empty_capture.context(),
        &empty_facts,
        |context, facts, _| Ok(business_invalid_no_data(context, facts, &other_empty_facts)),
        |context, facts, _| Ok(business_invalid_no_data(context, facts, &other_empty_facts)),
    );
    for path in [ShadowPath::Old, ShadowPath::New] {
        assert!(invalid_no_data.report().differences().contains(
            &ShadowBusinessDifference::Shadow(ShadowDifference::InvalidObservation {
                path,
                binding: ShadowInvalidBinding::NoDataEvidence,
            })
        ));
    }
}

#[test]
fn business_proposal_all_denied_capabilities_are_counted_before_rejection() {
    let (capture, facts) = fixture(false, b"secret-facts");
    for effect in ShadowEffect::ALL {
        for attempted_path in [ShadowPath::Old, ShadowPath::New] {
            let mut execution = execute_shadow_with_proposals(
                capture.context(),
                &facts,
                |context, facts, effects| {
                    if attempted_path == ShadowPath::Old {
                        assert_eq!(effects.request(effect).unwrap_err().effect(), effect);
                    }
                    Ok(business_ready(context, facts, Some(business_proposal())))
                },
                |context, facts, effects| {
                    if attempted_path == ShadowPath::New {
                        assert_eq!(effects.request(effect).unwrap_err().effect(), effect);
                    }
                    Ok(business_ready(context, facts, Some(business_proposal())))
                },
            );
            assert!(!execution.report().is_match());
            assert!(execution.report().differences().is_empty());
            assert_eq!(
                execution.report().reasons(),
                [ReasonCode::ShadowSideEffectAttempted]
            );
            for path in [ShadowPath::Old, ShadowPath::New] {
                for counted in ShadowEffect::ALL {
                    assert_eq!(
                        execution.report().counts(path).get(counted),
                        u64::from(path == attempted_path && counted == effect)
                    );
                }
            }
            assert!(execution.take_legacy_proposal().is_some());
        }
    }
}

#[test]
fn business_proposal_valid_non_ready_observations_match_without_legacy_output() {
    let (capture, facts) = fixture(false, b"secret-facts");
    let mut execution = execute_shadow_with_proposals::<BusinessProposal, _, _>(
        capture.context(),
        &facts,
        |context, facts, _| Ok(business_non_ready(context, facts, None)),
        |context, facts, _| Ok(business_non_ready(context, facts, None)),
    );
    assert!(execution.report().is_match());
    assert!(execution.report().differences().is_empty());
    assert!(execution.take_legacy_proposal().is_none());
}

#[test]
fn business_proposal_debug_and_errors_do_not_require_or_expose_payload_debug() {
    let (capture, facts) = fixture(false, b"sensitive-input-payload");
    let observation = business_ready(capture.context(), &facts, Some(business_proposal()));
    let observation_debug = format!("{observation:?}");
    let mut execution = execute_shadow_with_proposals(
        capture.context(),
        &facts,
        |context, facts, _| Ok(business_ready(context, facts, Some(business_proposal()))),
        |context, facts, effects| {
            let denied = effects.request(ShadowEffect::TransportSend).unwrap_err();
            assert_eq!(denied.to_string(), "shadow effect denied: TransportSend");
            let _sensitive_callback_error = "sensitive-callback-error";
            Ok(business_ready(context, facts, Some(business_proposal())))
        },
    );
    let before_take = format!("{execution:?}");
    assert!(execution.take_legacy_proposal().is_some());
    let after_take = format!("{execution:?}");
    assert_eq!(
        execution.report().reasons(),
        [ReasonCode::ShadowSideEffectAttempted]
    );
    for debug in [observation_debug, before_take, after_take] {
        for secret in [
            "sensitive-business-message",
            "SENSITIVE-A",
            "SENSITIVE-B",
            "sensitive-notification-a",
            "sensitive-notification-b",
            "sensitive-input-payload",
            "sensitive-callback-error",
            &42.25_f64.to_bits().to_string(),
            "7001",
        ] {
            assert!(!debug.contains(secret), "leaked {secret}");
        }
    }
}
