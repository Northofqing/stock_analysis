//! BR-188 static ownership regression for the board consumer cutover.

fn source(relative: &str) -> String {
    std::fs::read_to_string(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

#[test]
fn br188_target_consumers_do_not_call_legacy_board_facades() {
    for relative in [
        "src/bin/monitor/main.rs",
        "src/monitor/news_monitor.rs",
        "src/decision/exclusion.rs",
    ] {
        let text = source(relative);
        for forbidden in [
            "fetch_board_ranking(",
            "fetch_board_components(",
            "search_board_code_by_keyword(",
        ] {
            assert!(
                !text.contains(forbidden),
                "{relative} must not call legacy board facade {forbidden}"
            );
        }
    }
}

#[test]
fn br188_consumers_use_gateway_and_removed_unserviceable_legacy_screener() {
    let main = source("src/bin/monitor/main.rs");
    assert!(main.contains("day1_flows_blocking("));
    assert!(main.contains("render_board_flow_market_view("));
    assert!(!main.contains("run_stock_screener"));

    let news = source("src/monitor/news_monitor.rs");
    assert!(news.contains("memberships_blocking("));

    let exclusion = source("src/decision/exclusion.rs");
    assert!(exclusion.contains("memberships_blocking("));

    let bootstrap = source("src/app/bootstrap.rs");
    assert!(!bootstrap.contains("detect_resonance_sectors("));
    assert!(!bootstrap.contains("detect_unexplained_moves("));
}
