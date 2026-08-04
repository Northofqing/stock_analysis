//! BR-100/BR-192 PaperTrade terminal-evidence counted-delivery cutover guard.

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("start marker");
    let remainder = &source[start..];
    let end = remainder.find(end).expect("end marker");
    &remainder[..end]
}

#[test]
fn paper_trade_terminal_binding_preserves_plan_and_immutable_audit_receipt() {
    let source = include_str!("../src/trading/paper_trade.rs");
    let binding = between(
        source,
        "pub struct PaperTradeTerminalBindingV1",
        "pub fn evaluate(",
    );
    for required in [
        "plan_id",
        "instrument: InstrumentId",
        "business_date: NaiveDate",
        "order_audit_id",
        "audit_previous_hash",
        "audit_record_hash",
        "quote_observed_at",
        "terminal_at",
        "canonical_bytes",
        "terminal_transition_id",
        "delivery_subject_hash",
    ] {
        assert!(
            binding.contains(required),
            "terminal binding must preserve {required}"
        );
    }
}

#[test]
fn persisted_daily_projection_requires_one_exact_terminal_audit_chain_row() {
    let source = include_str!("../src/bin/monitor/push_templates.rs");
    let loader = between(source, "struct PaperTradeDispatchRow", "/// BR-100: 从当日");
    for required in [
        "plan_id",
        "order_audit_id: Option<i64>",
        "audit_previous_hash: Option<String>",
        "audit_record_hash: Option<String>",
        "LEFT JOIN order_audit",
        "LEFT JOIN order_audit_chain",
        "PaperTradeTerminalBindingV1::new(",
        "terminal evidence unavailable",
        "terminal evidence ambiguous",
    ] {
        assert!(
            loader.contains(required),
            "daily projection must require {required}"
        );
    }
}

#[test]
fn production_paper_trade_dispatch_carries_text_and_explicit_counted_binding_only() {
    let source = include_str!("../src/bin/monitor/push_templates.rs");
    let dispatch = between(
        source,
        "struct PreparedPaperTrade",
        "/// v39: P-03 候选触发",
    );
    for required in [
        "text: String",
        "binding: crate::durable_delivery_runtime::CountedDeliveryBinding",
        "CountedDeliveryScope::Ticket",
        "CountedDeliveryOrigin::InternalDurable",
        "push_counted_with_binding(",
    ] {
        assert!(
            dispatch.contains(required),
            "PaperTrade dispatch must preserve {required}"
        );
    }
    assert!(!dispatch.contains("push_governor_v3("));
    assert!(!dispatch.contains("pub async fn push_paper_trade("));
    assert!(!dispatch.contains("pub async fn dispatch_paper_trade_one("));
}

#[test]
fn monitor_has_one_terminal_paper_trade_call_site_without_local_occurrence_identity() {
    let source = include_str!("../src/bin/monitor/main.rs");
    let call = between(source, "// BR-100: P-04", "// 9:15-9:20");
    assert_eq!(call.matches("dispatch_paper_trade_daily(").count(), 1);
    assert!(!call.contains("push_governor"));
    assert!(!call.contains("Utc::now"));
    assert!(!call.contains("Local::now"));
}
