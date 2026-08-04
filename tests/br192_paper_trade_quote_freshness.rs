use std::fs;

fn source(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[test]
fn realtime_quote_freshness_is_checked_before_any_idempotency_reservation() {
    let paper_trade = source("src/trading/paper_trade.rs");
    let simulate = paper_trade
        .split("fn simulate_with_scope(")
        .nth(1)
        .expect("simulate_with_scope exists")
        .split("pub fn simulate(")
        .next()
        .expect("simulate_with_scope body");

    let freshness = simulate
        .find("validate_realtime_quote_freshness")
        .expect("realtime quote freshness gate");
    let reservation = simulate
        .find("reserve_business_order_id")
        .expect("idempotency reservation");
    assert!(
        freshness < reservation,
        "freshness rejection must happen before reservation/audit side effects"
    );
    assert!(simulate.contains("settled daily PaperTrade capability_unavailable"));
}

#[test]
fn realtime_freshness_contract_is_five_seconds_and_rejects_future_evidence() {
    let paper_trade = source("src/trading/paper_trade.rs");
    let validator = paper_trade
        .split("fn validate_realtime_quote_freshness(")
        .nth(1)
        .expect("freshness validator exists")
        .split("pub struct PaperTradeTerminalBindingV1")
        .next()
        .expect("freshness validator body");

    assert!(paper_trade.contains("const REALTIME_QUOTE_MAX_AGE_MILLIS: i64 = 5_000;"));
    assert!(validator.contains("REALTIME_QUOTE_MAX_AGE_MILLIS"));
    assert!(validator.contains("quote_observed_at > evaluated_at"));
    assert!(validator.contains("signed_duration_since"));
}

#[test]
fn terminal_binding_rechecks_the_same_realtime_freshness_evidence() {
    let paper_trade = source("src/trading/paper_trade.rs");
    let binding = paper_trade
        .split("impl PaperTradeTerminalBindingV1")
        .nth(1)
        .expect("terminal binding impl exists")
        .split("pub fn canonical_bytes")
        .next()
        .expect("terminal binding constructor");

    assert!(binding.contains("validate_realtime_quote_freshness(quote_observed_at, terminal_at)"));
}

#[test]
fn settled_daily_close_cannot_be_wrapped_as_a_realtime_execution_quote() {
    let paper_engine = source("src/trading/paper_engine.rs");

    assert!(paper_engine.contains("settled daily PaperTrade capability_unavailable"));
    assert!(!paper_engine.contains("fn load_latest_daily_close_quote("));
    assert!(!paper_engine.contains("using validated daily close"));
}
