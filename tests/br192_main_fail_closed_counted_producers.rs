//! BR-192 static guard for non-evidence monitor producers.
//!
//! These producers must either fail closed before their private acquisition
//! work or use a semantically correct non-counted kind. They must never regain
//! a generic counted-governor call without a complete durable binding.

fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let start = source.find(start).expect("start marker");
    let remainder = &source[start..];
    let end = remainder.find(end).expect("end marker");
    &remainder[..end]
}

fn production_main_without_legacy_e2e_fixture(source: &str) -> String {
    let start_marker = "async fn run_review_only_inner(isolated_test_fixtures: bool)";
    let end_marker = "fn validate_announcement_watch_codes(";
    match (source.find(start_marker), source.find(end_marker)) {
        (Some(start), Some(end)) if start < end => {
            let fixture = &source[start..end];
            assert!(
                fixture.contains("run_review_only_inner(true)")
                    && fixture.contains("isolated test fixtures"),
                "excluded range must remain the isolated TEST_CODE E2E fixture"
            );
            assert_eq!(
                source[..end].matches("run_review_only_inner(").count(),
                2,
                "legacy E2E review may only have its definition and true-only fixture call"
            );
            format!("{}{}", &source[..start], &source[end..])
        }
        (None, Some(_)) | (None, None) => source.to_owned(),
        _ => panic!("partial legacy E2E fixture markers"),
    }
}

#[test]
fn production_main_has_no_generic_counted_governor_call() {
    let source = include_str!("../src/bin/monitor/main.rs");
    let production = production_main_without_legacy_e2e_fixture(source);
    let counted_kinds = [
        "HoldingPlan",
        "HoldingEvent",
        "T0Advice",
        "CandidateTriggered",
        "CloseCall",
        "ForbiddenOps",
        "PaperTrade",
        "ReviewMarket",
        "ReviewLhb",
        "ReviewSignal",
        "ReviewFailure",
        "TomorrowWatch",
        "EventCalendar",
        "ReviewProviderTopN",
        "FactorIC",
        "SectorTier",
        "CapitalVerify",
        "DailyReport",
    ];

    for call_start in [
        "notify::push_governor(",
        "notify::push_governor_v3(",
        "push_governor(",
        "push_governor_v3(",
    ] {
        for (index, _) in production.match_indices(call_start) {
            let call = production[index..]
                .split_once(';')
                .map(|(call, _)| call)
                .unwrap_or(&production[index..]);
            for kind in counted_kinds {
                assert!(
                    !call.contains(&format!("PushKind::{kind}")),
                    "production generic governor call regained counted PushKind::{kind}: {call}"
                );
            }
        }
    }
}

#[test]
fn retired_sub_kind_router_api_is_absent_but_durable_mapping_remains() {
    let notify = include_str!("../src/bin/monitor/notify.rs");
    let v14 = include_str!("../src/bin/monitor/v14_adapter.rs");
    let runtime = include_str!("../src/bin/monitor/durable_delivery_runtime.rs");

    for retired in [
        "daily_report_router",
        "push_governor_inner_with_sub_kind",
        "push_governor_v3_with_sub_kind",
    ] {
        assert!(
            !notify.contains(retired),
            "retired generic sub-kind route remains: {retired}"
        );
    }
    assert!(notify.contains("pub enum DailyReportSubKind"));
    assert!(notify.contains("pub async fn push_counted_with_binding("));
    assert!(v14.contains("pub fn v14_gate_with_sub_kind("));
    assert!(v14.contains("pub fn v14_gate_counted_binding("));
    for mapping in [
        "Some(DailyReportSubKind::FactorIC) => DeliverySubKind::FactorIC",
        "Some(DailyReportSubKind::SectorTier) => DeliverySubKind::SectorTier",
        "Some(DailyReportSubKind::CapitalVerify) => DeliverySubKind::CapitalVerify",
    ] {
        assert!(
            runtime.contains(mapping),
            "durable explicit sub-kind mapping missing: {mapping}"
        );
    }
}

#[test]
fn daily_report_producers_skip_before_private_acquisition() {
    let source = include_str!("../src/bin/monitor/main.rs");

    let premarket = between(
        source,
        "let (_positions, targets) = match TieredScanner::load_portfolio_targets()",
        "prediction::verify_predictions().await;",
    );
    assert!(premarket
        .contains("capability_unavailable=premarket_daily_report_counted_binding_unavailable"));
    for forbidden in [
        "is_t1_locked(",
        "build_pre_market_checklist(",
        "PushKind::DailyReport",
    ] {
        assert!(
            !premarket.contains(forbidden),
            "premarket fail-closed region must not use {forbidden}"
        );
    }

    let close = between(
        source,
        "// BR-192: the legacy close summary",
        "\"[持仓汇总][BR-192]",
    );
    assert!(close
        .contains("capability_unavailable=close_summary_daily_report_counted_binding_unavailable"));
    for forbidden in [
        "fetch_sh_index_change",
        "portfolio::get_positions",
        "is_t1_locked(",
        "build_close_summary(",
        "PushKind::DailyReport",
    ] {
        assert!(
            !close.contains(forbidden),
            "close-summary fail-closed region must not use {forbidden}"
        );
    }

    let position_and_close_review = between(source, "\"[持仓汇总][BR-192]", "// 盘后独立维度");
    for required in [
        "capability_unavailable=position_summary_binding_unavailable",
        "capability_unavailable=close_review_account_binding_unavailable",
    ] {
        assert!(
            position_and_close_review.contains(required),
            "missing fail-closed marker {required}"
        );
    }
    for forbidden in [
        "render_real_paper_position_summary",
        "build_close_review_report().await",
        "PushKind::DailyReport",
        "push_governor",
    ] {
        assert!(
            !position_and_close_review.contains(forbidden),
            "unbound position/close report must not use {forbidden}"
        );
    }

    let virtual_t1 = between(
        source,
        "// BR-192: the mutable daily virtual snapshot",
        "// v18/v19: 多轮 AI",
    );
    assert!(virtual_t1
        .contains("capability_unavailable=virtual_t1_daily_report_counted_binding_unavailable"));
    for forbidden in [
        "load_latest_prior_virtual_snapshot",
        "fetch_t1_close_map",
        "required_daily_bars",
        "push_governor",
        "PushKind::DailyReport",
    ] {
        assert!(
            !virtual_t1.contains(forbidden),
            "virtual T+1 fail-closed region must not use {forbidden}"
        );
    }
}

#[test]
fn review_ai_is_disabled_before_model_and_quote_acquisition() {
    let source = include_str!("../src/bin/monitor/main.rs");
    let scheduler = between(
        source,
        "// BR-192: the legacy multi-round AI result",
        "if state.as_ref().map(review_batch::ReviewScheduleState::date)",
    );
    assert!(scheduler.contains("capability_unavailable=review_signal_counted_binding_unavailable"));
    for forbidden in [
        "run_multi_agent_analysis",
        "fetch_position_quotes",
        "run_review_deep_analysis",
        "PushKind::ReviewSignal",
        "push_governor",
    ] {
        assert!(
            !scheduler.contains(forbidden),
            "AI fail-closed scheduler must not use {forbidden}"
        );
        assert!(
            !source.contains(&format!("fn {forbidden}")),
            "retired AI acquisition helper unexpectedly remains: {forbidden}"
        );
    }
}

#[test]
fn holding_event_and_alert_paths_expose_unavailable_without_counted_delivery() {
    let source = include_str!("../src/bin/monitor/main.rs");

    let board_break = between(
        source,
        "// BR-192: the in-memory board-break transition",
        "if resonance.abs() > 30.0",
    );
    assert!(
        board_break.contains("capability_unavailable=holding_event_counted_binding_unavailable")
    );
    assert!(!board_break.contains("PushKind::HoldingEvent"));
    assert!(!board_break.contains("push_governor"));

    for retired in [
        "last_health_summary",
        "health_state_hash",
        "holding_health_state_unchanged",
        "commit_holding_health_state",
        "📊 持仓健康度 (",
    ] {
        assert!(
            !source.contains(retired),
            "retired HoldingEvent summary path remains: {retired}"
        );
    }
    assert!(source.contains("summary disabled before rendering"));

    let alert = between(
        source,
        "fn reject_unbound_alert_delivery(",
        "fn build_price_map(",
    );
    assert!(alert.contains("capability_unavailable=alert_daily_report_counted_binding_unavailable"));
    for forbidden in [
        "apply_attribution",
        "alert_log",
        "format_alert",
        "PushKind::DailyReport",
        "push_governor",
    ] {
        assert!(
            !alert.contains(forbidden),
            "unbound alert rejection must not use {forbidden}"
        );
    }
}

#[test]
fn board_flow_uses_the_existing_non_counted_intraday_market_semantics() {
    let main = include_str!("../src/bin/monitor/main.rs");
    let notify = include_str!("../src/bin/monitor/notify.rs");
    let runtime = include_str!("../src/bin/monitor/durable_delivery_runtime.rs");

    let board_flow = between(
        main,
        "stock_analysis::data_gateway::BoardDataGateway::new()",
        "// v34: I-03 涨停扩散与板块补涨",
    );
    assert!(board_flow.contains("notify::PushKind::IntradayMarket"));
    assert!(!board_flow.contains("notify::PushKind::ReviewSignal"));

    assert!(notify.contains("PushKind::IntradayMarket => \"盘中轮动\""));
    let counted_mapping = between(
        runtime,
        "fn durable_kind_and_sub_kind_with_override(",
        "fn owner_instance_identity()",
    );
    assert!(
        !counted_mapping.contains("K::IntradayMarket"),
        "IntradayMarket must remain outside the BR-192 counted catalog"
    );
}
