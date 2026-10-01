use super::*;
use std::cell::{Cell, RefCell};
use stock_analysis::opportunity::candidate_panel::{CandidateEntry, CandidateSource, EvidenceTier};

pub(in crate::push_templates) fn entry(
    code: &str,
    tier: EvidenceTier,
    price: Option<f64>,
    heat: Option<f64>,
) -> CandidateEntry {
    CandidateEntry {
        code: code.to_owned(),
        name: format!("TEST_CODE {code}"),
        sources: vec![CandidateSource::StockPick],
        tier,
        evidence: vec!["TEST_CODE observed fixture; no source authority".to_owned()],
        current_price: price,
        change_pct: Some(1.0),
        heat_score: heat,
    }
}

/// Plain observed DTO fixture, without a provider admission capability.
pub(in crate::push_templates) fn batch(entries: Vec<CandidateEntry>) -> RealCandidateBatch {
    RealCandidateBatch {
        entries,
        quotes: Default::default(),
        themes: Default::default(),
        quote_evidence: None,
        statistics_evidence: None,
        p5_files: Vec::new(),
        p5_candidate_refs: Vec::new(),
        chain_query: p05_chain_witness::project_same_query(Vec::new())
            .unwrap()
            .witness,
        chain_candidate_refs: Vec::new(),
    }
}

fn captured() -> chrono::DateTime<chrono::FixedOffset> {
    "2026-09-23T10:30:59+08:00".parse().unwrap()
}

#[tokio::test]
async fn p05_shared_unit_invalid_slot_precedes_acquisition_and_all_effects() {
    let calls = Cell::new(0);
    let observation = dispatch_with(
        "2026-09-22",
        captured(),
        || {
            calls.set(calls.get() + 1);
            std::future::ready(Ok(batch(Vec::new())))
        },
        |_| -> std::future::Ready<PushOutcome> { panic!("invalid slot cannot deliver") },
        || panic!("invalid slot cannot read snapshot"),
        |_| panic!("invalid slot cannot persist snapshot"),
    )
    .await;
    assert_eq!(calls.get(), 0);
    assert!(matches!(
        observation.auction_repush,
        CandidateChildObservation::NotPrepared(_)
    ));
    assert!(matches!(
        observation.candidate_board,
        CandidateChildObservation::NotPrepared(_)
    ));
    assert!(matches!(
        observation.invalidated,
        InvalidatedObservation::NotPrepared(_)
    ));
}

#[tokio::test]
async fn p05_shared_unit_source_failure_does_not_prepare_any_child() {
    let calls = Cell::new(0);
    let observation = dispatch_with(
        "2026-09-23",
        captured(),
        || {
            calls.set(calls.get() + 1);
            std::future::ready(Err("TEST_CODE original qualification failure".to_owned()))
        },
        |_| -> std::future::Ready<PushOutcome> { panic!("source failure cannot deliver") },
        || panic!("source failure cannot read snapshot"),
        |_| panic!("source failure cannot persist snapshot"),
    )
    .await;
    assert_eq!(calls.get(), 1);
    assert_eq!(
        observation,
        CandidateUnitDispatchObservation::unavailable("p05_source_unavailable".to_owned())
    );
}

#[tokio::test]
async fn p05_shared_unit_empty_batch_is_not_prepared_without_snapshot_effects() {
    let calls = Cell::new(0);
    let observation = dispatch_with(
        "2026-09-23",
        captured(),
        || {
            calls.set(calls.get() + 1);
            std::future::ready(Ok(batch(Vec::new())))
        },
        |_| -> std::future::Ready<PushOutcome> { panic!("empty batch cannot deliver") },
        || panic!("empty batch cannot read snapshot"),
        |_| panic!("empty batch cannot advance snapshot"),
    )
    .await;
    assert_eq!(calls.get(), 1);
    assert!(!observation.auction_repush.was_pushed());
    assert!(!observation.candidate_board.was_pushed());
    assert_eq!(
        observation.invalidated,
        InvalidatedObservation::NotPrepared("p05_no_candidates".to_owned())
    );
}

#[tokio::test]
async fn p05_shared_unit_auction_preserves_top5_stable_ties_filter_and_raw_outcome() {
    let entries = vec![
        entry(
            "reference-hot",
            EvidenceTier::Reference,
            Some(20.0),
            Some(999.0),
        ),
        entry(
            "strong-tie-first",
            EvidenceTier::Strong,
            Some(10.0),
            Some(80.0),
        ),
        entry(
            "strong-tie-second",
            EvidenceTier::Strong,
            Some(11.0),
            Some(80.0),
        ),
        entry("strong-hot", EvidenceTier::Strong, Some(12.0), Some(90.0)),
        entry(
            "reference-cold",
            EvidenceTier::Reference,
            Some(13.0),
            Some(60.0),
        ),
        entry("theme-last", EvidenceTier::Theme, Some(14.0), Some(50.0)),
        entry("no-price", EvidenceTier::Strong, None, Some(100.0)),
        entry("zero-price", EvidenceTier::Strong, Some(0.0), Some(100.0)),
        entry(
            "infinite-price",
            EvidenceTier::Strong,
            Some(f64::INFINITY),
            Some(100.0),
        ),
        entry("nan-heat", EvidenceTier::Strong, Some(15.0), Some(f64::NAN)),
    ];
    let original: Vec<_> = entries.iter().map(|entry| entry.code.clone()).collect();
    let batch = batch(entries);
    let ranked = auction_top5(&batch);
    assert_eq!(
        ranked
            .iter()
            .map(|entry| entry.code.as_str())
            .collect::<Vec<_>>(),
        vec![
            "strong-hot",
            "strong-tie-first",
            "strong-tie-second",
            "reference-hot",
            "reference-cold"
        ]
    );
    let rendered = render_auction_repush("10:30:59", &ranked);
    let calls = Cell::new(0);
    let denied = PushOutcome::Denied("TEST_CODE original auction admission reason".to_owned());
    let observation =
        dispatch_auction_from_batch(captured().date_naive(), "10:30:59", &batch, &mut |child| {
            calls.set(calls.get() + 1);
            assert_eq!(child.token.descriptor().push_kind, PushKind::AuctionRepush);
            assert_eq!(child.text, rendered);
            assert_eq!(
                child.binding.schedule_occurrence_identity(),
                "auction-repush:2026-09-23:10:30:59"
            );
            assert!(matches!(
                child.binding.scope(),
                crate::durable_delivery_runtime::CountedDeliveryScope::Global
            ));
            assert!(!child.binding.retry_authorized());
            std::future::ready(denied.clone())
        })
        .await;
    assert_eq!(calls.get(), 1);
    assert_eq!(observation, CandidateChildObservation::Observed(denied));
    assert!(!observation.was_pushed());
    assert_eq!(
        batch
            .entries
            .iter()
            .map(|entry| entry.code.clone())
            .collect::<Vec<_>>(),
        original
    );
}

#[tokio::test]
async fn p05_shared_unit_unpriced_auction_never_invents_a_delivered_child() {
    let batch = batch(vec![entry(
        "TEST_CODE unpriced",
        EvidenceTier::Strong,
        None,
        Some(80.0),
    )]);
    let observation = dispatch_auction_from_batch(
        captured().date_naive(),
        "10:30:59",
        &batch,
        &mut |_| -> std::future::Ready<PushOutcome> { panic!("unpriced auction cannot deliver") },
    )
    .await;
    assert_eq!(
        observation,
        CandidateChildObservation::NotPrepared("p05_no_priced_candidates".to_owned())
    );
}

#[tokio::test]
async fn p05_shared_unit_invalidated_retains_sink_error_and_existing_ticket_retry_identity() {
    let failure = PushOutcome::SinkError("TEST_CODE exact original sink uncertainty".to_owned());
    let observation = dispatch_invalidated_with(
        captured().date_naive(),
        "600009",
        "10:30:59",
        "600009",
        "候选",
        "从候选台消失",
        &mut |child| {
            assert_eq!(
                child.token.descriptor().push_kind,
                PushKind::CandidateInvalidated
            );
            assert_eq!(
                child.text,
                render_candidate_invalidated(
                    "10:30:59",
                    "600009",
                    "600009",
                    "候选",
                    "从候选台消失"
                )
            );
            assert_eq!(
                child.binding.schedule_occurrence_identity(),
                "candidate-invalidated:2026-09-23:600009"
            );
            assert_eq!(child.binding.governance_code(), Some("600009"));
            assert!(child.binding.retry_authorized());
            std::future::ready(failure.clone())
        },
    )
    .await;
    assert_eq!(observation, CandidateChildObservation::Observed(failure));
    assert!(!observation.was_pushed());
    let invalid = dispatch_invalidated_with(
        captured().date_naive(),
        "TEST_CODE invalid",
        "10:30:59",
        "test",
        "候选",
        "从候选台消失",
        &mut |_| -> std::future::Ready<PushOutcome> { panic!("unresolved ticket cannot deliver") },
    )
    .await;
    assert!(matches!(invalid, CandidateChildObservation::NotPrepared(_)));
}

#[tokio::test]
async fn p05_shared_unit_partial_observation_survives_board_preparation_failure() {
    let events = RefCell::new(Vec::new());
    let calls = Cell::new(0);
    let observation = dispatch_with(
        "2026-09-23",
        captured(),
        || {
            calls.set(calls.get() + 1);
            std::future::ready(Ok(batch(vec![
                entry("600010", EvidenceTier::Reference, Some(10.0), Some(80.0)),
                entry(
                    "TEST_CODE invalid sample",
                    EvidenceTier::Strong,
                    Some(10.0),
                    Some(f64::NAN),
                ),
            ])))
        },
        |child| {
            let kind = child.token.descriptor().push_kind;
            events.borrow_mut().push(kind);
            std::future::ready(match kind {
                PushKind::AuctionRepush => PushOutcome::Deduped,
                PushKind::CandidateInvalidated => {
                    PushOutcome::Denied("TEST_CODE raw T08 denied".to_owned())
                }
                _ => panic!("invalid Strong sample cannot deliver board"),
            })
        },
        || Some(["600009".to_owned()].into()),
        |_| panic!("board preparation failure cannot advance snapshot"),
    )
    .await;
    assert_eq!(calls.get(), 1);
    assert_eq!(
        *events.borrow(),
        vec![PushKind::AuctionRepush, PushKind::CandidateInvalidated]
    );
    assert_eq!(
        observation.auction_repush,
        CandidateChildObservation::Observed(PushOutcome::Deduped)
    );
    assert_eq!(
        observation.invalidated,
        InvalidatedObservation::Items(vec![(
            "600009".to_owned(),
            CandidateChildObservation::Observed(PushOutcome::Denied(
                "TEST_CODE raw T08 denied".to_owned()
            ))
        )])
    );
    assert!(matches!(
        observation.candidate_board,
        CandidateChildObservation::NotPrepared(_)
    ));
    assert!(!observation.auction_repush.was_pushed() && !observation.candidate_board.was_pushed());
}
