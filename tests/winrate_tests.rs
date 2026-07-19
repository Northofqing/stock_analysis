//! BR-017 verified-row win-rate calculation tests.

use stock_analysis::opportunity::winrate::*;

fn outcome(hit: Option<bool>, change: Option<f64>) -> VerifiedOutcome {
    VerifiedOutcome {
        hit,
        actual_change: change,
        special_case: None,
    }
}

#[test]
fn insufficient_verified_rows_have_no_gated_score() {
    let rows = vec![outcome(Some(true), Some(1.0)); 199];
    let summary = summarize_verified_rows(&rows);
    assert!(!summary.sufficient);
    assert_eq!(summary.gated_score(), None);
}

#[test]
fn threshold_uses_verified_hits_and_returns() {
    let mut rows = vec![outcome(Some(true), Some(2.0)); 150];
    rows.extend(vec![outcome(Some(false), Some(-1.0)); 50]);
    let summary = summarize_verified_rows(&rows);
    assert!(summary.sufficient);
    assert_eq!(summary.total, 200);
    assert_eq!(summary.wins, 150);
    assert_eq!(summary.losses, 50);
    assert_eq!(summary.winrate, Some(0.75));
    assert_eq!(summary.mean_return, Some(1.25));
    assert_eq!(summary.gated_score(), Some(0.75));
}

#[test]
fn unresolved_and_suspended_rows_are_excluded() {
    let rows = vec![
        outcome(Some(true), Some(1.0)),
        outcome(Some(false), Some(-1.0)),
        VerifiedOutcome {
            hit: None,
            actual_change: None,
            special_case: Some("suspended".to_string()),
        },
    ];
    let summary = summarize_verified_rows(&rows);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.winrate, Some(0.5));
}

#[test]
fn verified_negative_signal_maps_to_zero() {
    let mut rows = vec![outcome(Some(true), Some(1.0)); 80];
    rows.extend(vec![outcome(Some(false), Some(-1.0)); 120]);
    let summary = summarize_verified_rows(&rows);
    assert_eq!(summary.winrate, Some(0.4));
    assert_eq!(summary.gated_score(), Some(0.0));
}
