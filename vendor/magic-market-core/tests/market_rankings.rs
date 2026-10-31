use magic_market_core::{
    validate_market_ranking_batch, AssetClass, Exchange, FiniteNumber, InstrumentId, IsoDate,
    MarketBreadthRequest, MarketBreadthSnapshot, MarketRankingCapabilities, MarketRankingEntry,
    MarketRankingKind, MarketRankingUnit, MarketSession, NonEmptyText, PositiveU32, ProviderId,
    Ratio, SourceEvidence, SourcedRecord,
};

fn instrument(code: &str) -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap()
}

#[test]
fn ranking_capabilities_are_independent_per_source_metric() {
    let capabilities = MarketRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    assert!(capabilities.supports(&MarketRankingKind::VolumeRatio));
    assert!(!capabilities.supports(&MarketRankingKind::MainNetInflow));
    assert!(!capabilities.all_admitted());
    assert!(!capabilities.supports(&MarketRankingKind::Popularity));
}

fn evidence(provider: ProviderId, batch: &str, source_at: &str) -> SourceEvidence {
    SourceEvidence::new(provider, "2026-07-27T10:00:01+08:00", batch)
        .unwrap()
        .with_source_at(source_at)
        .unwrap()
}

fn ranking(
    kind: MarketRankingKind,
    rank: u32,
    code: &str,
    value: f64,
    unit: MarketRankingUnit,
) -> MarketRankingEntry {
    MarketRankingEntry::new(
        kind,
        PositiveU32::new(rank).unwrap(),
        Some(instrument(code)),
        NonEmptyText::new(format!("名称{code}")).unwrap(),
        FiniteNumber::new(value).unwrap(),
        unit,
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        NonEmptyText::new("A-share-equities").unwrap(),
        PositiveU32::new(3).unwrap(),
        PositiveU32::new(3).unwrap(),
        850,
        evidence(
            ProviderId::Eastmoney,
            "ranking-batch",
            "2026-07-27T10:00:00+08:00",
        ),
    )
    .unwrap()
}

#[test]
fn typed_rankings_require_the_metric_unit_code_name_session_and_source_date() {
    let volume = ranking(
        MarketRankingKind::VolumeRatio,
        1,
        "600001",
        12.5,
        MarketRankingUnit::Multiple,
    );
    assert_eq!(volume.instrument().unwrap().code(), "600001");
    assert_eq!(volume.label().as_str(), "名称600001");
    assert_eq!(volume.unit(), &MarketRankingUnit::Multiple);
    assert_eq!(volume.source_date().as_str(), "2026-07-27");
    assert_eq!(volume.source_session(), MarketSession::Continuous);
    assert_eq!(volume.coverage_ratio().get(), 1.0);
    assert_eq!(volume.max_source_skew_millis(), 850);
    assert_eq!(volume.evidence().batch_id(), "ranking-batch");
    assert_eq!(volume.provider_id(), ProviderId::Eastmoney);

    let wrong_unit = MarketRankingEntry::new(
        MarketRankingKind::MainNetInflow,
        PositiveU32::new(1).unwrap(),
        Some(instrument("600001")),
        NonEmptyText::new("名称").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        MarketRankingUnit::Multiple,
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        NonEmptyText::new("A-share-equities").unwrap(),
        PositiveU32::new(1).unwrap(),
        PositiveU32::new(1).unwrap(),
        0,
        evidence(
            ProviderId::Eastmoney,
            "ranking",
            "2026-07-27T10:00:00+08:00",
        ),
    );
    assert!(wrong_unit.is_err());

    let missing_code = MarketRankingEntry::new(
        MarketRankingKind::VolumeRatio,
        PositiveU32::new(1).unwrap(),
        None,
        NonEmptyText::new("名称").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        MarketRankingUnit::Multiple,
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        NonEmptyText::new("A-share-equities").unwrap(),
        PositiveU32::new(1).unwrap(),
        PositiveU32::new(1).unwrap(),
        0,
        evidence(
            ProviderId::Eastmoney,
            "ranking",
            "2026-07-27T10:00:00+08:00",
        ),
    );
    assert!(missing_code.is_err());
}

#[test]
fn ranking_batch_requires_full_coverage_continuous_unique_ranks_and_ordering() {
    let records = vec![
        ranking(
            MarketRankingKind::VolumeRatio,
            1,
            "600001",
            12.5,
            MarketRankingUnit::Multiple,
        ),
        ranking(
            MarketRankingKind::VolumeRatio,
            2,
            "600002",
            8.0,
            MarketRankingUnit::Multiple,
        ),
        ranking(
            MarketRankingKind::VolumeRatio,
            3,
            "600003",
            3.0,
            MarketRankingUnit::Multiple,
        ),
    ];
    validate_market_ranking_batch(
        &records,
        &MarketRankingKind::VolumeRatio,
        PositiveU32::new(3).unwrap(),
    )
    .unwrap();

    let mut duplicate_rank = records.clone();
    duplicate_rank[1] = ranking(
        MarketRankingKind::VolumeRatio,
        1,
        "600002",
        8.0,
        MarketRankingUnit::Multiple,
    );
    assert!(validate_market_ranking_batch(
        &duplicate_rank,
        &MarketRankingKind::VolumeRatio,
        PositiveU32::new(3).unwrap()
    )
    .is_err());

    let mut wrong_order = records;
    wrong_order[1] = ranking(
        MarketRankingKind::VolumeRatio,
        2,
        "600002",
        13.0,
        MarketRankingUnit::Multiple,
    );
    assert!(validate_market_ranking_batch(
        &wrong_order,
        &MarketRankingKind::VolumeRatio,
        PositiveU32::new(3).unwrap()
    )
    .is_err());
}

#[test]
fn ranking_checked_serde_rejects_semantic_bypasses() {
    let ranking = ranking(
        MarketRankingKind::MainNetInflow,
        1,
        "600001",
        10_000.0,
        MarketRankingUnit::Yuan,
    );
    let encoded = serde_json::to_value(&ranking).unwrap();
    assert_eq!(
        serde_json::from_value::<MarketRankingEntry>(encoded.clone()).unwrap(),
        ranking
    );

    let mut wrong_date = encoded.clone();
    wrong_date["source_date"] = serde_json::json!("2026-07-26");
    assert!(serde_json::from_value::<MarketRankingEntry>(wrong_date).is_err());

    let mut incomplete = encoded;
    incomplete["covered_count"] = serde_json::json!(2);
    assert!(serde_json::from_value::<MarketRankingEntry>(incomplete).is_err());
}

#[test]
fn breadth_contract_checks_partition_limits_coverage_skew_identity_and_evidence() {
    let input_evidence = vec![
        evidence(ProviderId::Tencent, "quote-a", "2026-07-27T10:00:00+08:00"),
        evidence(ProviderId::Tencent, "quote-b", "2026-07-27T10:00:01+08:00"),
    ];
    let snapshot = MarketBreadthSnapshot::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        100,
        90,
        40,
        30,
        20,
        5,
        3,
        1_000,
        input_evidence,
        evidence(
            ProviderId::LocalAnalysis,
            "breadth",
            "2026-07-27T10:00:01+08:00",
        ),
    )
    .unwrap();
    assert_eq!(snapshot.valid(), 90);
    assert_eq!(snapshot.coverage().get(), 0.9);
    assert_eq!(snapshot.max_source_skew_millis(), 1_000);
    assert_eq!(snapshot.input_evidence().len(), 2);

    let invalid_partition = MarketBreadthSnapshot::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        100,
        90,
        40,
        30,
        19,
        5,
        3,
        1_000,
        vec![evidence(
            ProviderId::Tencent,
            "quote",
            "2026-07-27T10:00:00+08:00",
        )],
        evidence(
            ProviderId::LocalAnalysis,
            "breadth",
            "2026-07-27T10:00:01+08:00",
        ),
    );
    assert!(invalid_partition.is_err());
    assert!(MarketBreadthSnapshot::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        u32::MAX,
        u32::MAX,
        u32::MAX,
        1,
        0,
        0,
        0,
        1_000,
        vec![evidence(
            ProviderId::Tencent,
            "overflow",
            "2026-07-27T10:00:00+08:00",
        )],
        evidence(
            ProviderId::LocalAnalysis,
            "breadth-overflow",
            "2026-07-27T10:00:01+08:00",
        ),
    )
    .is_err());
}

#[test]
fn breadth_request_enforces_decimal_coverage_and_source_skew_bounds() {
    let request = MarketBreadthRequest::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        Ratio::decimal(0.95).unwrap(),
        5_000,
    )
    .unwrap();
    assert_eq!(request.minimum_coverage().get(), 0.95);
    assert_eq!(request.maximum_source_skew_millis(), 5_000);
    assert!(MarketBreadthRequest::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        Ratio::new(95.0, magic_market_core::RatioUnit::Percent).unwrap(),
        5_000,
    )
    .is_err());
}

#[test]
fn ranking_records_reject_negative_incomplete_and_out_of_range_contexts() {
    let build = |rank, value, universe_size, covered_count, source_evidence| {
        MarketRankingEntry::new(
            MarketRankingKind::VolumeRatio,
            PositiveU32::new(rank).unwrap(),
            Some(instrument("600001")),
            NonEmptyText::new("名称600001").unwrap(),
            FiniteNumber::new(value).unwrap(),
            MarketRankingUnit::Multiple,
            IsoDate::new("2026-07-27").unwrap(),
            MarketSession::Continuous,
            NonEmptyText::new("A-share-equities").unwrap(),
            PositiveU32::new(universe_size).unwrap(),
            PositiveU32::new(covered_count).unwrap(),
            0,
            source_evidence,
        )
    };

    assert!(build(
        1,
        -0.1,
        1,
        1,
        evidence(
            ProviderId::Eastmoney,
            "negative",
            "2026-07-27T10:00:00+08:00"
        )
    )
    .is_err());
    assert!(build(
        1,
        1.0,
        2,
        1,
        evidence(
            ProviderId::Eastmoney,
            "partial",
            "2026-07-27T10:00:00+08:00"
        )
    )
    .is_err());
    assert!(build(
        3,
        1.0,
        2,
        2,
        evidence(ProviderId::Eastmoney, "rank", "2026-07-27T10:00:00+08:00")
    )
    .is_err());
    assert!(build(
        1,
        1.0,
        1,
        1,
        SourceEvidence::new(
            ProviderId::Eastmoney,
            "2026-07-27T10:00:01+08:00",
            "missing-source-time"
        )
        .unwrap()
    )
    .is_err());
    assert!(build(
        1,
        1.0,
        1,
        1,
        evidence(ProviderId::Eastmoney, "wrong-source-date", "2026-07-27X")
    )
    .is_err());
}

#[test]
fn ranking_batch_rejects_empty_wrong_kind_cardinality_and_duplicate_instruments() {
    let limit = PositiveU32::new(3).unwrap();
    assert!(validate_market_ranking_batch(&[], &MarketRankingKind::VolumeRatio, limit).is_err());

    let records = vec![
        ranking(
            MarketRankingKind::VolumeRatio,
            1,
            "600001",
            12.5,
            MarketRankingUnit::Multiple,
        ),
        ranking(
            MarketRankingKind::VolumeRatio,
            2,
            "600002",
            8.0,
            MarketRankingUnit::Multiple,
        ),
        ranking(
            MarketRankingKind::VolumeRatio,
            3,
            "600003",
            3.0,
            MarketRankingUnit::Multiple,
        ),
    ];
    assert!(
        validate_market_ranking_batch(&records, &MarketRankingKind::MainNetInflow, limit).is_err()
    );
    assert!(
        validate_market_ranking_batch(&records[..2], &MarketRankingKind::VolumeRatio, limit)
            .is_err()
    );

    let duplicate_instrument = vec![
        records[0].clone(),
        ranking(
            MarketRankingKind::VolumeRatio,
            2,
            "600001",
            8.0,
            MarketRankingUnit::Multiple,
        ),
        records[2].clone(),
    ];
    assert!(validate_market_ranking_batch(
        &duplicate_instrument,
        &MarketRankingKind::VolumeRatio,
        limit
    )
    .is_err());
}

#[test]
fn non_instrument_ranking_allows_absent_security_identity() {
    let industry = MarketRankingEntry::new(
        MarketRankingKind::Industry,
        PositiveU32::new(1).unwrap(),
        None,
        NonEmptyText::new("电力行业").unwrap(),
        FiniteNumber::new(2.5).unwrap(),
        MarketRankingUnit::Percent,
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        NonEmptyText::new("all-industries").unwrap(),
        PositiveU32::new(1).unwrap(),
        PositiveU32::new(1).unwrap(),
        0,
        evidence(
            ProviderId::Eastmoney,
            "industry-ranking",
            "2026-07-27T10:00:00+08:00",
        ),
    )
    .unwrap();
    assert!(industry.instrument().is_none());
    validate_market_ranking_batch(
        &[industry],
        &MarketRankingKind::Industry,
        PositiveU32::new(1).unwrap(),
    )
    .unwrap();
}

fn complete_breadth_snapshot() -> MarketBreadthSnapshot {
    MarketBreadthSnapshot::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        100,
        90,
        40,
        30,
        20,
        5,
        3,
        1_000,
        vec![
            evidence(ProviderId::Tencent, "quote-a", "2026-07-27T10:00:00+08:00"),
            evidence(ProviderId::Tencent, "quote-b", "2026-07-27T10:00:01+08:00"),
        ],
        evidence(
            ProviderId::LocalAnalysis,
            "breadth",
            "2026-07-27T10:00:01+08:00",
        ),
    )
    .unwrap()
}

#[test]
fn breadth_accessors_and_checked_serde_preserve_derived_coverage() {
    let request = MarketBreadthRequest::new(
        NonEmptyText::new("A-share-equities").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        MarketSession::Continuous,
        Ratio::decimal(0.9).unwrap(),
        1_000,
    )
    .unwrap();
    assert_eq!(request.universe().as_str(), "A-share-equities");
    assert_eq!(request.source_date().as_str(), "2026-07-27");
    assert_eq!(request.source_session(), MarketSession::Continuous);
    assert_eq!(
        serde_json::from_value::<MarketBreadthRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );

    let snapshot = complete_breadth_snapshot();
    assert_eq!(snapshot.universe().as_str(), "A-share-equities");
    assert_eq!(snapshot.source_date().as_str(), "2026-07-27");
    assert_eq!(snapshot.source_session(), MarketSession::Continuous);
    assert_eq!(snapshot.total(), 100);
    assert_eq!(snapshot.up(), 40);
    assert_eq!(snapshot.down(), 30);
    assert_eq!(snapshot.flat(), 20);
    assert_eq!(snapshot.limit_up(), 5);
    assert_eq!(snapshot.limit_down(), 3);
    assert_eq!(snapshot.evidence().provider(), ProviderId::LocalAnalysis);
    let encoded = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        serde_json::from_value::<MarketBreadthSnapshot>(encoded.clone()).unwrap(),
        snapshot
    );
    let mut contradictory_coverage = encoded;
    contradictory_coverage["coverage"]["value"] = serde_json::json!(0.8);
    assert!(serde_json::from_value::<MarketBreadthSnapshot>(contradictory_coverage).is_err());
}

#[test]
fn breadth_rejects_invalid_totals_limits_and_input_evidence() {
    let build = |total, valid, up, down, flat, limit_up, limit_down, inputs, aggregate| {
        MarketBreadthSnapshot::new(
            NonEmptyText::new("A-share-equities").unwrap(),
            IsoDate::new("2026-07-27").unwrap(),
            MarketSession::Continuous,
            total,
            valid,
            up,
            down,
            flat,
            limit_up,
            limit_down,
            1_000,
            inputs,
            aggregate,
        )
    };
    let input = evidence(ProviderId::Tencent, "quote", "2026-07-27T10:00:00+08:00");
    let aggregate = evidence(
        ProviderId::LocalAnalysis,
        "breadth",
        "2026-07-27T10:00:01+08:00",
    );

    assert!(build(0, 0, 0, 0, 0, 0, 0, vec![input.clone()], aggregate.clone()).is_err());
    assert!(build(1, 2, 1, 1, 0, 0, 0, vec![input.clone()], aggregate.clone()).is_err());
    assert!(build(3, 3, 1, 1, 1, 2, 0, vec![input.clone()], aggregate.clone()).is_err());
    assert!(build(3, 3, 1, 1, 1, 0, 2, vec![input.clone()], aggregate.clone()).is_err());
    assert!(build(3, 3, 1, 1, 1, 0, 0, Vec::new(), aggregate.clone()).is_err());
    assert!(build(
        3,
        3,
        1,
        1,
        1,
        0,
        0,
        vec![input.clone(), input],
        aggregate.clone()
    )
    .is_err());
    assert!(build(
        3,
        3,
        1,
        1,
        1,
        0,
        0,
        vec![SourceEvidence::new(
            ProviderId::Tencent,
            "2026-07-27T10:00:01+08:00",
            "missing-source-time"
        )
        .unwrap()],
        aggregate.clone()
    )
    .is_err());
    assert!(build(
        3,
        3,
        1,
        1,
        1,
        0,
        0,
        vec![evidence(
            ProviderId::Tencent,
            "quote",
            "2026-07-27T10:00:00+08:00"
        )],
        SourceEvidence::new(
            ProviderId::LocalAnalysis,
            "2026-07-27T10:00:01+08:00",
            "missing-aggregate-source-time"
        )
        .unwrap()
    )
    .is_err());
}
