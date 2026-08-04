//! BR-192 T0Advice counted-delivery cutover guard.

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("start marker");
    let remainder = &source[start..];
    let end = remainder.find(end).expect("end marker");
    &remainder[..end]
}

#[test]
fn production_t0_preserves_snapshot_and_complete_market_evidence_into_one_binding() {
    let source = include_str!("../src/bin/monitor/main.rs");
    let prepare = between(
        source,
        "struct PreparedT0Advice",
        "fn t0_delivery_outcomes_confirmed",
    );
    for required in [
        "text: String",
        "binding: durable_delivery_runtime::CountedDeliveryBinding",
        "T0PositionSnapshotBindingV1::new(",
        "T0PlanDecisionBindingV1::new(",
        "source_binding_canonical",
        "CountedDeliveryScope::Ticket",
        "instrument: decision_binding.instrument().clone()",
        "ordered_batch_ids: vec![decision_binding.evidence_batch_id().to_owned()]",
    ] {
        assert!(
            prepare.contains(required),
            "T0 prepare path must preserve {required}"
        );
    }

    let dispatch = between(
        source,
        "if last_t0_scan.elapsed().as_secs() >= 30",
        "// 产业链扫描已统一",
    );
    assert!(dispatch.contains("notify::push_counted_with_binding("));
    assert!(!dispatch.contains("notify::push_governor_v3("));
    assert!(!dispatch.contains("chrono::Utc::now()"));
    assert!(!dispatch.contains("chrono::Local::now()"));
}

#[test]
fn generic_t0_wrappers_and_synthetic_e2e_delivery_are_absent() {
    let source = include_str!("../src/bin/monitor/push_templates.rs");
    assert!(!source.contains("pub async fn push_t0_advice("));
    assert!(!source.contains("pub async fn push_t0_forbid("));

    for retired_synthetic_path in [
        "[v12-E2E-T05]",
        "[v12-E2E-T06]",
        "fail_msgs.push(\"T-05\"",
        "fail_msgs.push(\"T-06\"",
    ] {
        assert!(
            !source.contains(retired_synthetic_path),
            "synthetic T0 delivery path must remain absent: {retired_synthetic_path}"
        );
    }
}

#[test]
fn t0_batch_identity_is_frozen_only_after_complete_normalized_evidence() {
    let source = include_str!("../src/data_gateway/magic_tdx_t0.rs");
    let finalize = between(
        source,
        "fn finalize_t0_batch(",
        "pub fn fetch_magic_tdx_t0_batch(",
    );
    for required in [
        "requested_at",
        "source_at",
        "observed_at",
        "requested_instruments",
        "records: &fresh_records",
        "rejections: &rejections",
        "complete_batch_id(&binding)",
    ] {
        assert!(
            finalize.contains(required),
            "complete T0 batch binding must include {required}"
        );
    }
    assert!(!source.contains("fn stable_batch_id("));
}
