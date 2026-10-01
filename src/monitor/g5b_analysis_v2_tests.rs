#![cfg(unix)]
use super::*;
use crate::llm::ReceiptBearingJson;
use crate::monitor::alert_log::AlertLog;
use crate::monitor::g5b_selection_v2::G5bSelectionV2Candidate;

const CONTENT: &str = "{\n  \"risk_note\": \"证据有限\",\n  \"confidence\": \"low\",\n  \"capital_logic\": \"观测资金\",\n  \"catalyst_chain\": [\"第一条\"],\n  \"main_reason\": \"模型原文\"\n}";
fn clock() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-28T15:10:00+08:00")
        .unwrap()
        .with_timezone(&Utc)
}

// Only low-authority codec observations here. Work creation is exercised with
// the real attested coordinator in g5b_analysis_v2_behavior_tests.rs.
fn original() -> (Attempt, String) {
    let root = tempfile::tempdir().unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    let log = AlertLog::for_test(root.path()).unwrap();
    log.initialize_date_input_head(date).unwrap();
    let raw = "{\"origin\":\"production\",\"triggered_at\":\"2026-09-28T15:00:00+08:00\",\"code\":\"600001\",\"name\":\"fixture\",\"level\":\"重要\",\"category\":\"fixture\",\"message\":\"same\",\"t1_locked\":false}\n";
    log.append_test_date_raw_production_fixture(date, raw.as_bytes())
        .unwrap();
    let fence = log.acquire_date_writer_fence(date).unwrap();
    let prefix = log.inspect_date_input_prefix_locked(date, &fence).unwrap();
    let candidate = G5bSelectionV2Candidate::from_locked_prefix(&prefix).unwrap();
    let evidence = G5bSelectionEvidence::decode(candidate.canonical_bytes()).unwrap();
    let record = candidate.selected()[0].record().clone();
    let prompt = deep_attribution_prompt(&DeepAttributionRequest {
        record,
        as_of: clock(),
    });
    let attempt = Attempt {
        schema: ATTEMPT_SCHEMA.to_owned(),
        policy: ATTEMPT_POLICY.to_owned(),
        member: member(&evidence, 0).unwrap(),
        selection_canonical: evidence.canonical().to_vec(),
        selection_sha256: hash(evidence.canonical()),
        request_as_of: clock(),
        configured_provider: "TEST_CODE_REAL_RECEIPT".to_owned(),
        configured_model: "configured-model".to_owned(),
        system_prompt: G5B_SYSTEM_PROMPT_V1.to_owned(),
        system_sha256: hash(G5B_SYSTEM_PROMPT_V1.as_bytes()),
        user_sha256: hash(prompt.as_bytes()),
        user_prompt: prompt,
    };
    (attempt, "opaque-original-intent".to_owned())
}
fn completed(attempt: &Attempt, identity: &str) -> Core {
    let response = ReceiptBearingJson::test_fixture(
        "TEST_CODE_REAL_RECEIPT",
        "upstream-reported-model",
        Some("actual-request"),
        "actual-response",
        &attempt.system_prompt,
        &attempt.user_prompt,
        CONTENT,
        clock(),
        clock() + chrono::Duration::milliseconds(12),
    );
    let (_, content, actual_receipt) = response.into_parts();
    let receipt = Receipt::from(&actual_receipt);
    let result = parse_content(content.as_bytes()).unwrap();
    let row = row_for(attempt, &result, &receipt, 12).unwrap();
    let row_bytes = canonical(&row).unwrap();
    let result_bytes = canonical(&result).unwrap();
    let attempt_bytes = canonical(attempt).unwrap();
    let summary = render_deep_attribution_summary(&row).into_bytes();
    Core {
        schema: CORE_SCHEMA.to_owned(),
        member: attempt.member.clone(),
        attempt_identity: identity.to_owned(),
        attempt_sha256: hash(&attempt_bytes),
        attempt_canonical: attempt_bytes,
        model_content_utf8: content.into_bytes(),
        model_content_sha256: actual_receipt.response_sha256().to_owned(),
        receipt,
        result_sha256: hash(&result_bytes),
        result_canonical: result_bytes,
        row_sha256: hash(&row_bytes),
        row_canonical: row_bytes,
        elapsed_ms: 12,
        summary_sha256: hash(&summary),
        summary_utf8: summary,
    }
}

#[test]
fn g5b_analysis_v2_codec_retains_actual_content_and_reported_receipt_in_closed_handoff() {
    let (attempt, identity) = original();
    let bytes = canonical(&attempt).unwrap();
    let frozen = build_handoff(completed(&attempt, &identity)).unwrap();
    let core = validate_handoff(&frozen, &bytes, &identity).unwrap();
    assert_eq!(core.model_content_utf8, CONTENT.as_bytes());
    assert_ne!(
        core.model_content_utf8,
        canonical(&parse_content(CONTENT.as_bytes()).unwrap()).unwrap()
    );
    assert_eq!(core.receipt.response_sha256, hash(CONTENT.as_bytes()));
    assert_eq!(core.receipt.model, "upstream-reported-model");
    let handoff: Handoff = decode(&frozen).unwrap();
    let source: Source = decode(&handoff.source_canonical).unwrap();
    assert_eq!(source.schema, "g5b-attribution-v2");
    assert_eq!(source.frozen_core_sha256, hash(&handoff.core_canonical));
    let envelope: DeliveryEnvelope = decode(&handoff.envelope_canonical).unwrap();
    assert_eq!(
        envelope.schedule_occurrence_identity,
        attempt.member.occurrence_identity
    );
    assert_eq!(envelope.source_binding_canonical, handoff.source_canonical);
}

#[test]
fn g5b_analysis_v2_codec_rejects_unknown_duplicate_and_noncanonical_evidence() {
    let (attempt, identity) = original();
    let bytes = build_handoff(completed(&attempt, &identity)).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    let unknown = format!("{{\"caller_authority\":true,{}", &text[1..]);
    assert!(decode::<Handoff>(unknown.as_bytes()).is_err());
    let duplicate = format!("{{\"schema\":\"{}\",{}", HANDOFF_SCHEMA, &text[1..]);
    assert!(decode::<Handoff>(duplicate.as_bytes()).is_err());
    let mut whitespace = bytes.clone();
    whitespace.push(b'\n');
    assert!(decode::<Handoff>(&whitespace).is_err());
    let text = std::str::from_utf8(&canonical(&attempt).unwrap())
        .unwrap()
        .to_owned();
    let duplicate = format!(
        "{{\"request_as_of\":\"2026-09-28T07:10:00Z\",{}",
        &text[1..]
    );
    assert!(decode::<Attempt>(duplicate.as_bytes()).is_err());
    assert!(parse_content(b"{\"main_reason\":\"one\",\"main_reason\":\"two\",\"catalyst_chain\":[],\"capital_logic\":\"c\",\"confidence\":\"low\",\"risk_note\":\"r\"}").is_err());
    assert!(parse_content(b"{\"main_reason\":\"one\",\"catalyst_chain\":[],\"capital_logic\":\"c\",\"confidence\":\"low\",\"risk_note\":\"r\",\"extra\":1}").is_err());
}

#[test]
fn g5b_analysis_v2_codec_recomputed_outer_hash_does_not_rebind_raw_content_or_actual_attempt() {
    let (attempt, identity) = original();
    let actual = canonical(&attempt).unwrap();
    // Rebuild all outer hashes to ensure this is semantic verification, not
    // merely detecting an accidentally stale outer SHA.
    let mut core = completed(&attempt, &identity);
    core.model_content_utf8.push(b' ');
    core.model_content_sha256 = hash(&core.model_content_utf8);
    assert!(validate_handoff(&build_handoff(core).unwrap(), &actual, &identity).is_err());
    let mut core = completed(&attempt, &identity);
    let mut changed = attempt.clone();
    changed.request_as_of += chrono::Duration::seconds(1);
    let record = validate_attempt(&attempt).unwrap();
    changed.user_prompt = deep_attribution_prompt(&DeepAttributionRequest {
        record,
        as_of: changed.request_as_of,
    });
    changed.user_sha256 = hash(changed.user_prompt.as_bytes());
    core.attempt_canonical = canonical(&changed).unwrap();
    core.attempt_sha256 = hash(&core.attempt_canonical);
    core.receipt.user_sha256 = changed.user_sha256;
    assert!(validate_handoff(&build_handoff(core).unwrap(), &actual, &identity).is_err());
    assert!(validate_handoff(
        &build_handoff(completed(&attempt, &identity)).unwrap(),
        &actual,
        "other-original-intent"
    )
    .is_err());
}

#[test]
fn g5b_analysis_v2_codec_receipt_prompt_provider_time_and_required_upstream_identity_are_bound() {
    let (attempt, identity) = original();
    let actual = canonical(&attempt).unwrap();
    for mode in 0..5 {
        let mut core = completed(&attempt, &identity);
        match mode {
            0 => core.receipt.system_sha256 = hash(b"other-system"),
            1 => core.receipt.provider = "other-provider".to_owned(),
            2 => core.receipt.upstream_response_id = None,
            3 => core.receipt.completed_at = core.receipt.started_at - chrono::Duration::seconds(1),
            _ => core.receipt.started_at = attempt.request_as_of - chrono::Duration::seconds(1),
        }
        let bytes = build_handoff(core).unwrap();
        assert!(
            validate_handoff(&bytes, &actual, &identity).is_err(),
            "receipt mode {mode}"
        );
    }
}

#[test]
fn g5b_analysis_v2_codec_occurrence_source_and_rendered_envelope_must_match() {
    let (attempt, identity) = original();
    let actual = canonical(&attempt).unwrap();
    let frozen = build_handoff(completed(&attempt, &identity)).unwrap();
    let mut handoff: Handoff = decode(&frozen).unwrap();
    let mut source: Source = decode(&handoff.source_canonical).unwrap();
    source.member.line_ordinal += 1;
    handoff.source_canonical = canonical(&source).unwrap();
    handoff.source_sha256 = hash(&handoff.source_canonical);
    assert!(validate_handoff(&canonical(&handoff).unwrap(), &actual, &identity).is_err());
    let mut handoff: Handoff = decode(&frozen).unwrap();
    let mut envelope: DeliveryEnvelope = decode(&handoff.envelope_canonical).unwrap();
    envelope.rendered_content = b"caller-rendered-text".to_vec();
    handoff.envelope_canonical = canonical(&envelope).unwrap();
    handoff.envelope_sha256 = hash(&handoff.envelope_canonical);
    assert!(validate_handoff(&canonical(&handoff).unwrap(), &actual, &identity).is_err());
}
