//! BR-192: `monitor --test` may exercise isolated non-counted smoke paths, but it
//! must not manufacture counted delivery decisions from TEST_CODE constants or
//! dispatch-time account/quote projections.

#[test]
fn strict_review_entry_has_no_legacy_fixture_or_generic_counted_producer() {
    let source = include_str!("../src/bin/monitor/main.rs");
    assert!(
        !source.contains("run_review_only_inner(isolated_test_fixtures"),
        "retired isolated review fixture must not be restored"
    );
    let review_entry = source
        .split("\nasync fn run_review_only() -> Result<(), String> {\n")
        .nth(1)
        .and_then(|tail| {
            tail.split("\n#[cfg(test)]\nmod tests_br212_review_cli_completion")
                .next()
        })
        .expect("strict review entry");
    let strict_runner = source
        .split("\nasync fn run_strict_review_only_inner(\n")
        .nth(1)
        .and_then(|tail| tail.split("\nfn post_session_review_window_open(").next())
        .expect("strict dependency-partitioned runner");

    assert!(
        review_entry.contains("run_strict_review_only_inner(&due, context)")
            && strict_runner.contains("push_templates::dispatch_post_session_review(context, due)"),
        "review must delegate directly to the strict dependency-partitioned dispatcher"
    );
    for forbidden in [
        "stock_analysis::portfolio::get_positions()",
        "notify::PushKind::DailyReport",
        "notify::PushKind::TomorrowWatch",
        "PushKind::DailyReport",
        "PushKind::TomorrowWatch",
        "required_monitor_daily_bars(",
        "TEST_CODE_",
    ] {
        assert!(
            !review_entry.contains(forbidden) && !strict_runner.contains(forbidden),
            "strict review entry retained legacy/private producer {forbidden}"
        );
    }
}

#[test]
fn v70_constant_fixture_does_not_fabricate_counted_review_cards() {
    let source = include_str!("../src/bin/monitor/main.rs");
    let e2e = source
        .split("async fn e2e_all_templates_run(")
        .nth(1)
        .and_then(|tail| {
            tail.split("#[cfg(test)]\nmod tests_br196_monitor_test_acceptance")
                .next()
        })
        .expect("v70 isolated E2E entry");
    let fixture = source
        .split("async fn push_e2e_14x_templates(")
        .nth(1)
        .and_then(|tail| tail.split("fn validate_announcement_watch_codes(").next())
        .expect("v70 isolated template fixture");

    assert!(
        fixture.contains("capability_unavailable=review_lhb_counted_binding_unavailable")
            && fixture.contains("capability_unavailable=review_signal_counted_binding_unavailable"),
        "v70 must report counted review capabilities as explicitly unavailable"
    );
    assert!(
        fixture.contains(
            "log_dispatcher_attempt(\n        \"R-04\",\n        false,\n        0,\n        \"review_lhb_counted_binding_unavailable\","
        ) && fixture.contains(
            "log_dispatcher_attempt(\n        \"R-05\",\n        false,\n        0,\n        \"review_signal_counted_binding_unavailable\","
        ),
        "v70 counted capability boundaries require namespaced dispatcher audit"
    );
    for forbidden in [
        "render_review_lhb(",
        "render_review_signal(",
        "notify::PushKind::ReviewLhb",
        "notify::PushKind::ReviewSignal",
    ] {
        assert!(
            !fixture.contains(forbidden),
            "v70 retained fabricated counted review producer {forbidden}"
        );
    }
    assert!(
        !e2e.contains("dispatch_all_for_test("),
        "isolated E2E must not indirectly enter live/counted template dispatchers"
    );
    assert!(
        !source.contains("seed_e2e_data_via_sqlite("),
        "retired synthetic E2E database seed must not be restored"
    );
    assert!(
        !source.contains("TEST_CODE_LHB_"),
        "retired synthetic R-04 disclosure rows must not remain in the E2E seed"
    );
}
