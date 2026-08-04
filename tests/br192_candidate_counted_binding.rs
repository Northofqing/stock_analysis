//! BR-192 CandidateTriggered evidence-binding cutover guard.
//!
//! The production candidate assembler currently has real quote/statistics
//! batches but no durable Candidate -> Triggered lifecycle transition.  Until
//! that producer exists, T-07 must reject explicitly and must never fall
//! through to either the legacy generic dispatcher or a synthesized counted
//! binding.

fn function_body<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("function start");
    let rest = &source[start..];
    let end = rest.find(end).expect("function end");
    &rest[..end]
}

#[test]
fn candidate_triggered_fails_closed_until_durable_lifecycle_evidence_exists() {
    let source = include_str!("../src/bin/monitor/push_templates.rs");
    let contract = function_body(
        source,
        "/// BR-192 caller migration contract:",
        "pub async fn push_candidate_triggered(",
    );
    let candidate = function_body(
        source,
        "pub async fn push_candidate_triggered(",
        "pub async fn push_candidate_invalidated(",
    );

    for required in [
        "Candidate -> Triggered",
        "ordered selection-decision/source batch identities",
        "ticket scope",
    ] {
        assert!(
            contract.contains(required),
            "CandidateTriggered fail-closed contract is missing {required}"
        );
    }
    for required in [
        "Result<bool, String>",
        "Err(CANDIDATE_COUNTED_BINDING_UNAVAILABLE.to_string())",
    ] {
        assert!(
            candidate.contains(required),
            "CandidateTriggered fail-closed result is missing {required}"
        );
    }
    for forbidden in [
        "dispatch(",
        "push_counted_with_binding(",
        "CountedDeliveryBinding::new(",
        "chrono::Local::now(",
        "chrono::Utc::now(",
    ] {
        assert!(
            !candidate.contains(forbidden),
            "CandidateTriggered must not synthesize or dispatch counted evidence via {forbidden}"
        );
    }
}

#[test]
fn production_candidate_dispatcher_records_the_binding_rejection() {
    let source = include_str!("../src/bin/monitor/push_templates.rs");
    let dispatcher = function_body(
        source,
        "pub async fn dispatch_candidate_triggered_daily(",
        "async fn dispatch_holding_plan_daily_result(",
    );

    for required in [
        "match push_candidate_triggered(",
        "[P-03][BR-192] 候选计数投递拒绝",
        "log_dispatcher_attempt(\"P-03\", false, 1, &reason)",
    ] {
        assert!(
            dispatcher.contains(required),
            "P-03 must expose CandidateTriggered binding rejection through {required}"
        );
    }
}
