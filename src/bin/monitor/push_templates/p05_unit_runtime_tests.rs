//! Bin observation/orchestration regressions. These DTO callbacks do not mint
//! an actual Unit, consumer permission, physical acceptance or completion.
//! The library's attested paired-database tests cover those owner boundaries.
use super::tests::{batch, entry};
use super::*;
use std::cell::{Cell, RefCell};
use stock_analysis::opportunity::candidate_panel::EvidenceTier;

fn captured() -> chrono::DateTime<chrono::FixedOffset> {
    "2026-09-23T09:20:37+08:00".parse().unwrap()
}

#[tokio::test]
async fn p05_unit_bin_one_acquisition_preserves_both_original_renderers_before_owner() {
    let calls = Cell::new(0);
    let stages = RefCell::new(Vec::new());
    let entries = vec![
        entry(
            "reference-hot",
            EvidenceTier::Reference,
            Some(20.0),
            Some(999.0),
        ),
        entry("strong-first", EvidenceTier::Strong, Some(10.0), Some(80.0)),
        entry(
            "strong-second",
            EvidenceTier::Strong,
            Some(11.0),
            Some(80.0),
        ),
        entry("no-price", EvidenceTier::Strong, None, Some(100.0)),
    ];
    let expected_board =
        stock_analysis::opportunity::candidate_panel::format_candidate_board(&entries);
    let expected_auction =
        render_auction_repush("09:20:37", &auction_top5(&batch(entries.clone())));
    let prepared = prepare_once_with(
        "2026-09-23",
        captured(),
        || {
            calls.set(calls.get() + 1);
            stages.borrow_mut().push("single original acquisition");
            std::future::ready(Ok(batch(entries.clone())))
        },
        |prepared| {
            // Both exact original renderer outputs must already exist at the
            // immutable owner's boundary. This callback is not a fake owner.
            assert_eq!(prepared.auction_rendered, expected_auction.as_bytes());
            assert_eq!(prepared.board_rendered, expected_board.as_bytes());
            stages.borrow_mut().push("complete observed intent input");
            std::future::ready(Ok(prepared))
        },
    )
    .await
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        *stages.borrow(),
        vec![
            "single original acquisition",
            "complete observed intent input"
        ]
    );
    assert_eq!(
        prepared
            .entries
            .iter()
            .map(|entry| entry.code.as_str())
            .collect::<Vec<_>>(),
        vec!["reference-hot", "strong-first", "strong-second", "no-price"]
    );
    let auction = String::from_utf8(prepared.auction_rendered).unwrap();
    assert!(auction.find("strong-first").unwrap() < auction.find("strong-second").unwrap());
    assert!(!auction.contains("no-price"));
}

#[tokio::test]
async fn p05_unit_bin_invalid_slot_source_failure_and_unpriced_batch_never_reach_owner() {
    let acquisitions = Cell::new(0);
    let persists = Cell::new(0);
    for case in 0..4 {
        let date = if case == 0 {
            "2026-09-22"
        } else {
            "2026-09-23"
        };
        let result = prepare_once_with(
            date,
            captured(),
            || {
                acquisitions.set(acquisitions.get() + 1);
                std::future::ready(match case {
                    1 => Err("TEST_CODE source unavailable".to_owned()),
                    2 => Ok(batch(Vec::new())),
                    _ => Ok(batch(vec![entry(
                        "no-price",
                        EvidenceTier::Strong,
                        None,
                        Some(90.0),
                    )])),
                })
            },
            |_| {
                persists.set(persists.get() + 1);
                std::future::ready(Ok(()))
            },
        )
        .await;
        assert!(
            result.is_err(),
            "case {case} cannot create an immutable intent"
        );
    }
    assert_eq!(
        acquisitions.get(),
        3,
        "invalid date rejected before the loader"
    );
    assert_eq!(persists.get(), 0);
}

#[tokio::test]
async fn p05_unit_bin_saved_unknown_does_not_hide_later_unit_or_enable_reacquisition() {
    let dates = ["2026-09-22".to_owned(), "2026-09-23".to_owned()];
    let seen = RefCell::new(Vec::new());
    let result = recover_saved_with(dates, |date| {
        seen.borrow_mut().push(date.clone());
        std::future::ready(if date == "2026-09-22" {
            Err("TEST_CODE original Started freeze absent".to_owned())
        } else {
            Ok(())
        })
    })
    .await;
    assert_eq!(*seen.borrow(), vec!["2026-09-22", "2026-09-23"]);
    let error = result.unwrap_err();
    assert!(error.contains("2026-09-22") && error.contains("Started freeze absent"));
    // Production propagates this error before restore-current/fresh acquisition;
    // the original Unknown is never replaced by None or a new model invocation.
}

#[tokio::test]
async fn p05_unit_bin_saved_recovery_does_not_need_fresh_window_or_source_provider() {
    assert!(!fresh_auction_window("2026-09-23T17:00:00+08:00".parse().unwrap()).unwrap());
    let seen = RefCell::new(Vec::new());
    let dates = recover_saved_with(["2026-09-22".to_owned()], |date| {
        seen.borrow_mut().push(date);
        std::future::ready(Ok(()))
    })
    .await
    .unwrap();
    assert!(dates.contains("2026-09-22"));
    assert_eq!(*seen.borrow(), vec!["2026-09-22"]);
    assert!(!fresh_auction_window("2026-09-23T09:19:59+08:00".parse().unwrap()).unwrap());
    assert!(fresh_auction_window("2026-09-23T09:20:00+08:00".parse().unwrap()).unwrap());
    assert!(!fresh_auction_window("2026-09-23T09:25:00+08:00".parse().unwrap()).unwrap());
}

#[test]
fn p05_unit_bin_retained_source_encodings_preserve_original_independent_times() {
    let mut observed = batch(vec![entry(
        "TEST_CODE_SOURCE",
        EvidenceTier::Strong,
        Some(10.0),
        Some(80.0),
    )]);
    observed.quote_evidence = Some(stock_analysis::data_gateway::BatchEvidence {
        provider: stock_analysis::market_domain::ProviderId::HithinkFinance,
        source: "TEST_CODE quote observation".into(),
        source_at: Some("2026-09-23T09:19:58+08:00".into()),
        observed_at: "2026-09-23T09:20:01+08:00".into(),
        batch_id: "TEST_CODE_QUOTE_BATCH".into(),
    });
    observed.statistics_evidence = Some(stock_analysis::data_gateway::BatchEvidence {
        provider: stock_analysis::market_domain::ProviderId::HithinkFinance,
        source: "TEST_CODE statistic observation".into(),
        source_at: None,
        observed_at: "2026-09-23T09:20:04+08:00".into(),
        batch_id: "TEST_CODE_STAT_BATCH".into(),
    });
    let bytes = observed_source_bytes(&observed).unwrap();
    let quote: serde_json::Value =
        serde_json::from_slice(bytes.quote_evidence.as_ref().unwrap()).unwrap();
    let stats: serde_json::Value =
        serde_json::from_slice(bytes.statistics_evidence.as_ref().unwrap()).unwrap();
    assert_eq!(quote["source_at"], "2026-09-23T09:19:58+08:00");
    assert_eq!(quote["observed_at"], "2026-09-23T09:20:01+08:00");
    assert!(stats["source_at"].is_null());
    assert_eq!(stats["observed_at"], "2026-09-23T09:20:04+08:00");
    assert_ne!(quote["batch_id"], stats["batch_id"]);
    let query: serde_json::Value = serde_json::from_slice(&bytes.chain_query).unwrap();
    assert_eq!(
        query["ordered_rows_sha256"],
        observed.chain_query.ordered_rows_sha256
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes.p5_file_witnesses).unwrap(),
        serde_json::json!([])
    );
}

#[tokio::test]
async fn p05_unit_bin_legacy_single_card_entries_cannot_open_generic_consumer() {
    assert!(!super::super::dispatch_auction_repush("09:20").await);
    assert!(!super::super::dispatch_candidate_board("2026-09-23").await);
    use stock_analysis::durable_delivery::PushKind as Kind;
    assert_eq!(
        presentation_for_kind(Kind::AuctionRepush)
            .unwrap()
            .descriptor()
            .push_kind,
        PushKind::AuctionRepush
    );
    assert_eq!(
        presentation_for_kind(Kind::CandidateBoard)
            .unwrap()
            .descriptor()
            .push_kind,
        PushKind::CandidateBoard
    );
    assert_eq!(
        presentation_for_kind(Kind::CandidateInvalidated)
            .unwrap()
            .descriptor()
            .push_kind,
        PushKind::CandidateInvalidated
    );
    assert!(presentation_for_kind(Kind::HoldingPlan).is_err());
}

#[test]
fn p05_unit_bin_resident_and_daily_call_order_supplements_owner_behavior() {
    let main = include_str!("../main.rs");
    let resident = main
        .find("} else if !selection_cli.requires_service_enablement()")
        .unwrap();
    let entry = &main[resident..main.find("let intraday_loop = async").unwrap()];
    assert!(
        entry
            .find("dispatch_auction_candidate_unit_tick(false)")
            .unwrap()
            < entry.find("initialize_p05_family_before_window()").unwrap()
    );
    assert!(
        entry.find("initialize_p05_family_before_window()").unwrap()
            < entry.find("spawn_dryrun_reporter").unwrap()
    );
    let all_day = &main[main.find("let intraday_loop = async").unwrap()
        ..main.find("let market_loop = async").unwrap()];
    assert!(
        all_day
            .find("dispatch_auction_candidate_unit_tick(true)")
            .unwrap()
            < all_day.find("let risk_context").unwrap()
    );
    let daily = &main[main.find("let market_loop = async").unwrap()..];
    assert!(
        daily.find("initialize_p05_family_before_window()").unwrap()
            < daily.find("TieredScanner::load_portfolio_targets").unwrap()
    );
    assert!(!main.contains("post_close_candidates_notified"));
    assert!(!main.contains("dispatch_auction_candidate_unit("));
    let producer = include_str!("p05_shared_unit.rs");
    let tick = &producer[producer.find("pub(super) async fn tick(").unwrap()
        ..producer.find("async fn recover_saved_with").unwrap()];
    assert!(tick.find("recover_saved_with").unwrap() < tick.find("fresh_auction_window").unwrap());
    assert!(!tick.contains("candidate_snapshot_"));
    let runtime = include_str!("../durable_delivery_runtime/p05_unit.rs");
    let child = &runtime[runtime.find("fn deliver_child_blocking").unwrap()
        ..runtime.find("pub(super) fn resume_owned").unwrap()];
    assert!(
        child.find("counted_delivery_critical_section").unwrap()
            < child.find(".prepare_child(1)").unwrap()
    );
    assert!(
        child.find(".prepare_child(1)").unwrap()
            < child.find("advance_prepared_envelope_with_resume").unwrap()
    );
    assert!(!child.contains(".coordinator.prepare("));
    assert!(!child.contains(".dispatch_view("));
}
