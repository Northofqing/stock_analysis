use magic_market_core::{
    validate_provider_top_n_ranking_batch, AssetClass, DataBatch, Exchange, FiniteNumber,
    InstrumentId, IsoDate, MarketRankingCapabilities, MarketRankingKind, MarketRankingUnit,
    NonEmptyText, PositiveU32, Provenance, ProviderId, ProviderTopNRankingCapabilities,
    ProviderTopNRankingEntry, ProviderTopNRankingRequest, SignalCapabilities, SourceEvidence,
    SourcedRecord,
};

fn instrument(exchange: Exchange, code: &str) -> InstrumentId {
    InstrumentId::new(exchange, code, AssetClass::Equity).unwrap()
}

fn request(kind: MarketRankingKind, limit: u32) -> ProviderTopNRankingRequest {
    ProviderTopNRankingRequest::new(
        kind,
        IsoDate::new("2026-07-29").unwrap(),
        PositiveU32::new(limit).unwrap(),
        NonEmptyText::new("m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23").unwrap(),
    )
    .unwrap()
}

fn evidence(provider: ProviderId, observed_at: &str, batch_id: &str) -> SourceEvidence {
    SourceEvidence::new(provider, observed_at, batch_id).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn entry(
    kind: MarketRankingKind,
    ordinal: u32,
    exchange: Exchange,
    code: &str,
    value: f64,
    latest_trading_date: &str,
    filter_identity: &str,
    provider_declared_total: u32,
    inspected_row_count: u32,
    provider: ProviderId,
    observed_at: &str,
    batch_id: &str,
) -> ProviderTopNRankingEntry {
    let unit = match kind {
        MarketRankingKind::VolumeRatio => MarketRankingUnit::Multiple,
        MarketRankingKind::MainNetInflow => MarketRankingUnit::Yuan,
        _ => panic!("test helper accepts only admitted provider Top-N metrics"),
    };
    ProviderTopNRankingEntry::new(
        kind,
        PositiveU32::new(ordinal).unwrap(),
        instrument(exchange, code),
        NonEmptyText::new(format!("名称{code}")).unwrap(),
        FiniteNumber::new(value).unwrap(),
        unit,
        IsoDate::new(latest_trading_date).unwrap(),
        NonEmptyText::new(filter_identity).unwrap(),
        PositiveU32::new(provider_declared_total).unwrap(),
        PositiveU32::new(inspected_row_count).unwrap(),
        evidence(provider, observed_at, batch_id),
    )
    .unwrap()
}

fn valid_records() -> Vec<ProviderTopNRankingEntry> {
    let filter = "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23";
    vec![
        entry(
            MarketRankingKind::VolumeRatio,
            1,
            Exchange::Shanghai,
            "600001",
            12.5,
            "2026-07-29",
            filter,
            5_542,
            3,
            ProviderId::Eastmoney,
            "2026-07-29T15:35:01+08:00",
            "eastmoney-topn-1",
        ),
        entry(
            MarketRankingKind::VolumeRatio,
            2,
            Exchange::Shenzhen,
            "000002",
            8.0,
            "2026-07-29",
            filter,
            5_542,
            3,
            ProviderId::Eastmoney,
            "2026-07-29T15:35:01+08:00",
            "eastmoney-topn-1",
        ),
        entry(
            MarketRankingKind::VolumeRatio,
            3,
            Exchange::Beijing,
            "830001",
            3.0,
            "2026-07-29",
            filter,
            5_542,
            3,
            ProviderId::Eastmoney,
            "2026-07-29T15:35:01+08:00",
            "eastmoney-topn-1",
        ),
    ]
}

fn records_with_context(
    provider: ProviderId,
    observed_at: &str,
    batch_id: &str,
    inspected_row_count: u32,
) -> Vec<ProviderTopNRankingEntry> {
    valid_records()
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            entry(
                record.kind().clone(),
                u32::try_from(index + 1).unwrap(),
                record.instrument().exchange(),
                record.instrument().code(),
                record.value().get(),
                record.latest_trading_date().as_str(),
                record.filter_identity().as_str(),
                record.provider_declared_total().get(),
                inspected_row_count,
                provider,
                observed_at,
                batch_id,
            )
        })
        .collect()
}

fn valid_batch(records: Vec<ProviderTopNRankingEntry>) -> DataBatch<ProviderTopNRankingEntry> {
    DataBatch::strict(
        records,
        Provenance::new("eastmoney", "2026-07-29T15:35:01+08:00")
            .unwrap()
            .with_batch_id("eastmoney-topn-1")
            .unwrap(),
    )
}

#[test]
fn provider_top_n_capabilities_and_request_are_independent_from_full_market() {
    let capabilities = ProviderTopNRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    assert!(capabilities.supports(&MarketRankingKind::VolumeRatio));
    assert!(!capabilities.supports(&MarketRankingKind::MainNetInflow));
    assert!(!capabilities.supports(&MarketRankingKind::Popularity));
    assert!(!capabilities.all_admitted());

    assert_eq!(
        MarketRankingCapabilities::default(),
        MarketRankingCapabilities {
            volume_ratio: false,
            main_net_inflow: false,
        }
    );
    assert!(!SignalCapabilities::default().market_rankings);

    let request = request(MarketRankingKind::VolumeRatio, 100);
    assert_eq!(request.kind(), &MarketRankingKind::VolumeRatio);
    assert_eq!(request.trading_date().as_str(), "2026-07-29");
    assert_eq!(request.limit().get(), 100);
    assert_eq!(
        request.filter_identity().as_str(),
        "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23"
    );

    assert!(ProviderTopNRankingRequest::new(
        MarketRankingKind::VolumeRatio,
        IsoDate::new("2026-07-29").unwrap(),
        PositiveU32::new(101).unwrap(),
        NonEmptyText::new("A-share-equities").unwrap(),
    )
    .is_err());
    assert!(ProviderTopNRankingRequest::new(
        MarketRankingKind::Popularity,
        IsoDate::new("2026-07-29").unwrap(),
        PositiveU32::new(20).unwrap(),
        NonEmptyText::new("A-share-equities").unwrap(),
    )
    .is_err());
}

#[test]
fn provider_top_n_entry_is_narrow_and_never_claims_source_time_or_coverage() {
    let record = valid_records().remove(0);
    assert_eq!(record.kind(), &MarketRankingKind::VolumeRatio);
    assert_eq!(record.source_order_ordinal().get(), 1);
    assert_eq!(record.instrument().code(), "600001");
    assert_eq!(record.label().as_str(), "名称600001");
    assert_eq!(record.value().get(), 12.5);
    assert_eq!(record.unit(), &MarketRankingUnit::Multiple);
    assert_eq!(record.latest_trading_date().as_str(), "2026-07-29");
    assert_eq!(
        record.filter_identity().as_str(),
        "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23"
    );
    assert_eq!(record.provider_declared_total().get(), 5_542);
    assert_eq!(record.inspected_row_count().get(), 3);
    assert_eq!(record.evidence().source_at(), None);
    assert_eq!(record.provider_id(), ProviderId::Eastmoney);
    assert_eq!(record.evidence_batch_id(), "eastmoney-topn-1");
    assert_eq!(record.evidence_source_at(), None);
    assert_eq!(
        record.evidence_observed_at(),
        Some("2026-07-29T15:35:01+08:00")
    );

    let with_source_time =
        SourceEvidence::new(ProviderId::Eastmoney, "2026-07-29T15:35:01+08:00", "bad")
            .unwrap()
            .with_source_at("2026-07-29T15:35:00+08:00")
            .unwrap();
    assert!(ProviderTopNRankingEntry::new(
        MarketRankingKind::VolumeRatio,
        PositiveU32::new(1).unwrap(),
        instrument(Exchange::Shanghai, "600001"),
        NonEmptyText::new("名称").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        MarketRankingUnit::Multiple,
        IsoDate::new("2026-07-29").unwrap(),
        NonEmptyText::new("A-share-equities").unwrap(),
        PositiveU32::new(5_542).unwrap(),
        PositiveU32::new(1).unwrap(),
        with_source_time,
    )
    .is_err());
}

#[test]
fn provider_top_n_entry_rejects_invalid_metric_identity_value_and_counts() {
    let base_evidence = || evidence(ProviderId::Eastmoney, "2026-07-29T15:35:01+08:00", "batch");
    let make = |kind, instrument, value, unit, ordinal, inspected| {
        ProviderTopNRankingEntry::new(
            kind,
            PositiveU32::new(ordinal).unwrap(),
            instrument,
            NonEmptyText::new("名称").unwrap(),
            FiniteNumber::new(value).unwrap(),
            unit,
            IsoDate::new("2026-07-29").unwrap(),
            NonEmptyText::new("A-share-equities").unwrap(),
            PositiveU32::new(5_542).unwrap(),
            PositiveU32::new(inspected).unwrap(),
            base_evidence(),
        )
    };

    assert!(make(
        MarketRankingKind::VolumeRatio,
        instrument(Exchange::Shanghai, "600001"),
        1.0,
        MarketRankingUnit::Yuan,
        1,
        1,
    )
    .is_err());
    assert!(make(
        MarketRankingKind::VolumeRatio,
        instrument(Exchange::Shanghai, "600001"),
        -0.1,
        MarketRankingUnit::Multiple,
        1,
        1,
    )
    .is_err());
    assert!(make(
        MarketRankingKind::Popularity,
        instrument(Exchange::Shanghai, "600001"),
        1.0,
        MarketRankingUnit::Score,
        1,
        1,
    )
    .is_err());
    assert!(make(
        MarketRankingKind::VolumeRatio,
        InstrumentId::new(Exchange::Shanghai, "000001", AssetClass::Index).unwrap(),
        1.0,
        MarketRankingUnit::Multiple,
        1,
        1,
    )
    .is_err());
    assert!(make(
        MarketRankingKind::VolumeRatio,
        instrument(Exchange::Shanghai, "000001"),
        1.0,
        MarketRankingUnit::Multiple,
        1,
        1,
    )
    .is_err());
    assert!(make(
        MarketRankingKind::VolumeRatio,
        instrument(Exchange::Shanghai, "600001"),
        1.0,
        MarketRankingUnit::Multiple,
        2,
        1,
    )
    .is_err());
    assert!(ProviderTopNRankingEntry::new(
        MarketRankingKind::VolumeRatio,
        PositiveU32::new(1).unwrap(),
        instrument(Exchange::Shanghai, "600001"),
        NonEmptyText::new("名称").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        MarketRankingUnit::Multiple,
        IsoDate::new("2026-07-29").unwrap(),
        NonEmptyText::new("A-share-equities").unwrap(),
        PositiveU32::new(1).unwrap(),
        PositiveU32::new(2).unwrap(),
        base_evidence(),
    )
    .is_err());

    assert!(make(
        MarketRankingKind::VolumeRatio,
        instrument(Exchange::Beijing, "930001"),
        1.0,
        MarketRankingUnit::Multiple,
        1,
        1,
    )
    .is_ok());
}

#[test]
fn provider_top_n_batch_requires_one_complete_exact_post_close_response() {
    let request = request(MarketRankingKind::VolumeRatio, 3);
    let capabilities = ProviderTopNRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    validate_provider_top_n_ranking_batch(
        &valid_batch(valid_records()),
        &request,
        capabilities,
        ProviderId::Eastmoney,
        &NonEmptyText::new("eastmoney").unwrap(),
    )
    .unwrap();

    let unavailable = ProviderTopNRankingCapabilities::default();
    assert!(validate_provider_top_n_ranking_batch(
        &valid_batch(valid_records()),
        &request,
        unavailable,
        ProviderId::Eastmoney,
        &NonEmptyText::new("eastmoney").unwrap(),
    )
    .is_err());

    let incomplete = DataBatch::best_effort(
        valid_records(),
        Provenance::new("eastmoney", "2026-07-29T15:35:01+08:00")
            .unwrap()
            .with_batch_id("eastmoney-topn-1")
            .unwrap(),
        vec!["missing selected metric".into()],
    )
    .unwrap();
    assert!(validate_provider_top_n_ranking_batch(
        &incomplete,
        &request,
        capabilities,
        ProviderId::Eastmoney,
        &NonEmptyText::new("eastmoney").unwrap(),
    )
    .is_err());

    let sourced_batch = DataBatch::strict(
        valid_records(),
        Provenance::new("eastmoney", "2026-07-29T15:35:01+08:00")
            .unwrap()
            .with_source_at("2026-07-29T15:35:00+08:00")
            .unwrap()
            .with_batch_id("eastmoney-topn-1")
            .unwrap(),
    );
    assert!(validate_provider_top_n_ranking_batch(
        &sourced_batch,
        &request,
        capabilities,
        ProviderId::Eastmoney,
        &NonEmptyText::new("eastmoney").unwrap(),
    )
    .is_err());
}

#[test]
fn provider_top_n_batch_rejects_bad_cardinality_order_identity_and_context() {
    let request = request(MarketRankingKind::VolumeRatio, 3);
    let capabilities = ProviderTopNRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    let validate = |records| {
        validate_provider_top_n_ranking_batch(
            &valid_batch(records),
            &request,
            capabilities,
            ProviderId::Eastmoney,
            &NonEmptyText::new("eastmoney").unwrap(),
        )
    };

    assert!(validate(valid_records()[..2].to_vec()).is_err());

    let mut wrong_order = valid_records();
    wrong_order[1] = entry(
        MarketRankingKind::VolumeRatio,
        2,
        Exchange::Shenzhen,
        "000002",
        13.0,
        "2026-07-29",
        "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23",
        5_542,
        3,
        ProviderId::Eastmoney,
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    );
    assert!(validate(wrong_order).is_err());

    let mut duplicate = valid_records();
    duplicate[1] = entry(
        MarketRankingKind::VolumeRatio,
        2,
        Exchange::Shanghai,
        "600001",
        8.0,
        "2026-07-29",
        "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23",
        5_542,
        3,
        ProviderId::Eastmoney,
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    );
    assert!(validate(duplicate).is_err());

    let mut mixed_date = valid_records();
    mixed_date[1] = entry(
        MarketRankingKind::VolumeRatio,
        2,
        Exchange::Shenzhen,
        "000002",
        8.0,
        "2026-07-28",
        "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23",
        5_542,
        3,
        ProviderId::Eastmoney,
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    );
    assert!(validate(mixed_date).is_err());

    let mut mixed_filter = valid_records();
    mixed_filter[1] = entry(
        MarketRankingKind::VolumeRatio,
        2,
        Exchange::Shenzhen,
        "000002",
        8.0,
        "2026-07-29",
        "different-filter",
        5_542,
        3,
        ProviderId::Eastmoney,
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    );
    assert!(validate(mixed_filter).is_err());
}

#[test]
fn provider_top_n_batch_admits_later_capture_but_rejects_pre_close_and_earlier_date() {
    let request = request(MarketRankingKind::VolumeRatio, 3);
    let capabilities = ProviderTopNRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    let validate = |records, observed_at| {
        let batch = DataBatch::strict(
            records,
            Provenance::new("eastmoney", observed_at)
                .unwrap()
                .with_batch_id("eastmoney-topn-1")
                .unwrap(),
        );
        validate_provider_top_n_ranking_batch(
            &batch,
            &request,
            capabilities,
            ProviderId::Eastmoney,
            &NonEmptyText::new("eastmoney").unwrap(),
        )
    };

    let pre_close = valid_records()
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            entry(
                record.kind().clone(),
                u32::try_from(index + 1).unwrap(),
                record.instrument().exchange(),
                record.instrument().code(),
                record.value().get(),
                "2026-07-29",
                record.filter_identity().as_str(),
                5_542,
                3,
                ProviderId::Eastmoney,
                "2026-07-29T15:34:59+08:00",
                "eastmoney-topn-1",
            )
        })
        .collect();
    assert!(validate(pre_close, "2026-07-29T15:34:59+08:00").is_err());

    let next_date = valid_records()
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            entry(
                record.kind().clone(),
                u32::try_from(index + 1).unwrap(),
                record.instrument().exchange(),
                record.instrument().code(),
                record.value().get(),
                "2026-07-29",
                record.filter_identity().as_str(),
                5_542,
                3,
                ProviderId::Eastmoney,
                "2026-07-30T00:00:01+08:00",
                "eastmoney-topn-1",
            )
        })
        .collect();
    validate(next_date, "2026-07-30T00:00:01+08:00").unwrap();

    let previous_date = valid_records()
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            entry(
                record.kind().clone(),
                u32::try_from(index + 1).unwrap(),
                record.instrument().exchange(),
                record.instrument().code(),
                record.value().get(),
                "2026-07-29",
                record.filter_identity().as_str(),
                5_542,
                3,
                ProviderId::Eastmoney,
                "2026-07-28T23:59:59+08:00",
                "eastmoney-topn-1",
            )
        })
        .collect();
    assert!(validate(previous_date, "2026-07-28T23:59:59+08:00").is_err());
}

#[test]
fn provider_top_n_checked_serde_rejects_semantic_bypasses() {
    let request = request(MarketRankingKind::VolumeRatio, 3);
    let request_json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<ProviderTopNRankingRequest>(request_json.clone()).unwrap(),
        request
    );
    let mut oversized_request = request_json;
    oversized_request["limit"] = serde_json::json!(101);
    assert!(serde_json::from_value::<ProviderTopNRankingRequest>(oversized_request).is_err());

    let record = valid_records().remove(0);
    let encoded = serde_json::to_value(&record).unwrap();
    assert_eq!(
        serde_json::from_value::<ProviderTopNRankingEntry>(encoded.clone()).unwrap(),
        record
    );

    let mut source_time = encoded.clone();
    source_time["evidence"]["source_at"] = serde_json::json!("2026-07-29T15:35:00+08:00");
    assert!(serde_json::from_value::<ProviderTopNRankingEntry>(source_time).is_err());

    let mut ordinal = encoded;
    ordinal["source_order_ordinal"] = serde_json::json!(4);
    assert!(serde_json::from_value::<ProviderTopNRankingEntry>(ordinal).is_err());
}

#[test]
fn provider_top_n_batch_rejects_provider_batch_and_observation_evidence_drift() {
    let request = request(MarketRankingKind::VolumeRatio, 3);
    let capabilities = ProviderTopNRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    let validate = |records, observed_at: &str, batch_id: &str| {
        validate_provider_top_n_ranking_batch(
            &DataBatch::strict(
                records,
                Provenance::new("eastmoney", observed_at)
                    .unwrap()
                    .with_batch_id(batch_id)
                    .unwrap(),
            ),
            &request,
            capabilities,
            ProviderId::Eastmoney,
            &NonEmptyText::new("eastmoney").unwrap(),
        )
    };

    assert!(validate(
        records_with_context(
            ProviderId::Tencent,
            "2026-07-29T15:35:01+08:00",
            "eastmoney-topn-1",
            3,
        ),
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    )
    .is_err());
    assert!(validate(
        records_with_context(
            ProviderId::Eastmoney,
            "2026-07-29T15:35:01+08:00",
            "different-record-batch",
            3,
        ),
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    )
    .is_err());
    assert!(validate(
        records_with_context(
            ProviderId::Eastmoney,
            "2026-07-29T15:35:02+08:00",
            "eastmoney-topn-1",
            3,
        ),
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    )
    .is_err());
    assert!(validate(
        records_with_context(
            ProviderId::Eastmoney,
            "2026-07-29T15:35:01+08:00",
            "eastmoney-topn-1",
            4,
        ),
        "2026-07-29T15:35:01+08:00",
        "eastmoney-topn-1",
    )
    .is_err());
}

#[test]
fn provider_top_n_batch_accepts_valid_fractional_observation_time() {
    let observed_at = "2026-07-29T15:35:01.123+08:00";
    let batch = DataBatch::strict(
        records_with_context(ProviderId::Eastmoney, observed_at, "fractional-batch", 3),
        Provenance::new("eastmoney", observed_at)
            .unwrap()
            .with_batch_id("fractional-batch")
            .unwrap(),
    );
    validate_provider_top_n_ranking_batch(
        &batch,
        &request(MarketRankingKind::VolumeRatio, 3),
        ProviderTopNRankingCapabilities {
            volume_ratio: true,
            main_net_inflow: false,
        },
        ProviderId::Eastmoney,
        &NonEmptyText::new("eastmoney").unwrap(),
    )
    .unwrap();
}

#[test]
fn provider_top_n_batch_rejects_malformed_fraction_and_invalid_clock_values() {
    let capabilities = ProviderTopNRankingCapabilities {
        volume_ratio: true,
        main_net_inflow: false,
    };
    for observed_at in [
        "2026-07-29 15:35:01+08:00",
        "2026-02-30T15:35:01+08:00",
        "2026-07-29T15:35:01.+08:00",
        "2026-07-29T15:35:01.a+08:00",
        "2026-07-29T24:00:00+08:00",
        "2026-07-29T15:60:00+08:00",
        "2026-07-29T15:35:60+08:00",
    ] {
        let batch = DataBatch::strict(
            records_with_context(ProviderId::Eastmoney, observed_at, "invalid-clock-batch", 3),
            Provenance::new("eastmoney", observed_at)
                .unwrap()
                .with_batch_id("invalid-clock-batch")
                .unwrap(),
        );
        assert!(validate_provider_top_n_ranking_batch(
            &batch,
            &request(MarketRankingKind::VolumeRatio, 3),
            capabilities,
            ProviderId::Eastmoney,
            &NonEmptyText::new("eastmoney").unwrap(),
        )
        .is_err());
    }
}
