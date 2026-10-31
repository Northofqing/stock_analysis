use magic_market_core::{
    AssetClass, Exchange, InstrumentId, IsoDate, NonEmptyText, PositiveU32, Price, ProviderId,
    SourceEvidence, SourcedRecord, TargetPriceConsensus, TargetPriceObservation,
    TargetPriceRequest,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap()
}

fn evidence(batch: &str, source_at: &str) -> SourceEvidence {
    SourceEvidence::new(ProviderId::Eastmoney, "2026-07-27T10:00:00+08:00", batch)
        .unwrap()
        .with_source_at(source_at)
        .unwrap()
}

fn observation(
    report: &str,
    institution: &str,
    published_on: &str,
    source_t: f64,
    source_l: f64,
) -> TargetPriceObservation {
    TargetPriceObservation::new(
        instrument(),
        NonEmptyText::new("贵州茅台").unwrap(),
        NonEmptyText::new(report).unwrap(),
        NonEmptyText::new(institution).unwrap(),
        NonEmptyText::new(format!("机构{institution}")).unwrap(),
        IsoDate::new(published_on).unwrap(),
        Price::new(source_t).unwrap(),
        Price::new(source_l).unwrap(),
        evidence("target-batch", published_on),
    )
    .unwrap()
}

#[test]
fn observation_preserves_live_proven_lower_and_upper_source_fields() {
    let value = observation("R1", "O1", "2026-07-20", 1430.0, 1400.0);
    assert_eq!(value.source_indv_aim_price_t().get(), 1430.0);
    assert_eq!(value.instrument_name().as_str(), "贵州茅台");
    assert_eq!(value.source_indv_aim_price_l().get(), 1400.0);
    assert_eq!(value.normalized_low().get(), 1400.0);
    assert_eq!(value.normalized_high().get(), 1430.0);
    assert_eq!(
        serde_json::from_value::<TargetPriceObservation>(serde_json::to_value(&value).unwrap())
            .unwrap(),
        value
    );
    assert!(TargetPriceObservation::new(
        instrument(),
        NonEmptyText::new("贵州茅台").unwrap(),
        NonEmptyText::new("R2").unwrap(),
        NonEmptyText::new("O2").unwrap(),
        NonEmptyText::new("机构O2").unwrap(),
        IsoDate::new("2026-07-20").unwrap(),
        Price::new(1400.0).unwrap(),
        Price::new(1430.0).unwrap(),
        evidence("target-batch", "2026-07-20"),
    )
    .is_err());
}

#[test]
fn aggregate_derives_period_counts_range_mean_and_input_evidence() {
    let request = TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-01-01").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
    )
    .unwrap();
    let aggregate = TargetPriceConsensus::new(
        &request,
        vec![
            observation("R1", "O1", "2026-04-28", 1525.0, 1525.0),
            observation("R2", "O2", "2026-07-20", 1430.0, 1430.0),
            observation("R3", "O1", "2026-07-21", 1500.0, 1400.0),
        ],
        evidence("target-batch", "2026-07-21"),
    )
    .unwrap();
    assert_eq!(aggregate.observation_start().as_str(), "2026-04-28");
    assert_eq!(aggregate.instrument_name().as_str(), "贵州茅台");
    assert_eq!(aggregate.observation_end().as_str(), "2026-07-21");
    assert_eq!(aggregate.sample_count().get(), 3);
    assert_eq!(aggregate.contributor_count().get(), 2);
    assert_eq!(aggregate.low().get(), 1400.0);
    assert_eq!(aggregate.high().get(), 1525.0);
    assert_eq!(aggregate.mean().get(), (1525.0 + 1430.0 + 1450.0) / 3.0);
    assert_eq!(aggregate.input_evidence().len(), 3);
    assert_eq!(
        serde_json::from_value::<TargetPriceConsensus>(serde_json::to_value(&aggregate).unwrap())
            .unwrap(),
        aggregate
    );
}

#[test]
fn request_range_identity_duplicates_and_aggregate_serde_are_checked() {
    assert!(TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-07-27").unwrap(),
        IsoDate::new("2026-01-01").unwrap(),
    )
    .is_err());
    let request = TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-01-01").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
    )
    .unwrap();
    let duplicate = observation("R1", "O1", "2026-07-20", 1430.0, 1430.0);
    assert!(TargetPriceConsensus::new(
        &request,
        vec![duplicate.clone(), duplicate],
        evidence("target-batch", "2026-07-20"),
    )
    .is_err());

    let aggregate = TargetPriceConsensus::new(
        &request,
        vec![observation("R1", "O1", "2026-07-20", 1430.0, 1430.0)],
        evidence("target-batch", "2026-07-20"),
    )
    .unwrap();
    let mut wrong_mean = serde_json::to_value(aggregate).unwrap();
    wrong_mean["mean"] = serde_json::json!(1.0);
    assert!(serde_json::from_value::<TargetPriceConsensus>(wrong_mean).is_err());
}

#[test]
fn sample_and_contributor_counts_remain_positive_by_construction() {
    let request = TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-01-01").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
    )
    .unwrap();
    assert!(TargetPriceConsensus::new(
        &request,
        Vec::new(),
        evidence("target-batch", "2026-07-20")
    )
    .is_err());
    assert!(PositiveU32::new(0).is_err());
}

#[test]
fn request_and_observation_accessors_are_checked_through_serde() {
    let request = TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-01-01").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
    )
    .unwrap();
    assert_eq!(request.instrument(), &instrument());
    assert_eq!(request.from().as_str(), "2026-01-01");
    assert_eq!(request.through().as_str(), "2026-07-27");
    let encoded_request = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<TargetPriceRequest>(encoded_request.clone()).unwrap(),
        request
    );
    let mut reversed_request = encoded_request;
    reversed_request["from"] = serde_json::json!("2026-07-28");
    assert!(serde_json::from_value::<TargetPriceRequest>(reversed_request).is_err());

    let value = observation("R1", "O1", "2026-07-20", 1430.0, 1400.0);
    assert_eq!(value.instrument(), &instrument());
    assert_eq!(value.report_id().as_str(), "R1");
    assert_eq!(value.institution_id().as_str(), "O1");
    assert_eq!(value.institution_name().as_str(), "机构O1");
    assert_eq!(value.published_on().as_str(), "2026-07-20");
    assert_eq!(value.evidence().batch_id(), "target-batch");
    assert_eq!(value.provider_id(), ProviderId::Eastmoney);
    assert_eq!(value.evidence_batch_id(), "target-batch");

    let mut contradictory_range = serde_json::to_value(&value).unwrap();
    contradictory_range["normalized_low"] = serde_json::json!(1399.0);
    assert!(serde_json::from_value::<TargetPriceObservation>(contradictory_range).is_err());
}

#[test]
fn target_observation_rejects_missing_or_mismatched_source_dates() {
    let make = |source_evidence| {
        TargetPriceObservation::new(
            instrument(),
            NonEmptyText::new("贵州茅台").unwrap(),
            NonEmptyText::new("R1").unwrap(),
            NonEmptyText::new("O1").unwrap(),
            NonEmptyText::new("机构O1").unwrap(),
            IsoDate::new("2026-07-20").unwrap(),
            Price::new(1430.0).unwrap(),
            Price::new(1400.0).unwrap(),
            source_evidence,
        )
    };
    assert!(make(
        SourceEvidence::new(
            ProviderId::Eastmoney,
            "2026-07-27T10:00:00+08:00",
            "target-batch"
        )
        .unwrap()
    )
    .is_err());
    assert!(make(evidence("target-batch", "2026-07-19")).is_err());
    assert!(make(evidence("target-batch", "2026-07-20X")).is_err());
}

fn observation_with_identity(
    returned_instrument: InstrumentId,
    instrument_name: &str,
    report: &str,
    institution: &str,
    published_on: &str,
    source_evidence: SourceEvidence,
) -> TargetPriceObservation {
    TargetPriceObservation::new(
        returned_instrument,
        NonEmptyText::new(instrument_name).unwrap(),
        NonEmptyText::new(report).unwrap(),
        NonEmptyText::new(institution).unwrap(),
        NonEmptyText::new(format!("机构{institution}")).unwrap(),
        IsoDate::new(published_on).unwrap(),
        Price::new(1430.0).unwrap(),
        Price::new(1400.0).unwrap(),
        source_evidence,
    )
    .unwrap()
}

#[test]
fn aggregate_rejects_identity_name_range_and_batch_disagreement() {
    let request = TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-01-01").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
    )
    .unwrap();
    let aggregate_evidence = evidence("target-batch", "2026-07-20");
    let other = InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity).unwrap();

    assert!(TargetPriceConsensus::new(
        &request,
        vec![observation_with_identity(
            other,
            "贵州茅台",
            "R1",
            "O1",
            "2026-07-20",
            evidence("target-batch", "2026-07-20"),
        )],
        aggregate_evidence.clone(),
    )
    .is_err());
    assert!(TargetPriceConsensus::new(
        &request,
        vec![
            observation_with_identity(
                instrument(),
                "贵州茅台",
                "R1",
                "O1",
                "2026-07-20",
                evidence("target-batch", "2026-07-20"),
            ),
            observation_with_identity(
                instrument(),
                "茅台",
                "R2",
                "O2",
                "2026-07-20",
                evidence("target-batch", "2026-07-20"),
            ),
        ],
        aggregate_evidence.clone(),
    )
    .is_err());
    assert!(TargetPriceConsensus::new(
        &request,
        vec![observation_with_identity(
            instrument(),
            "贵州茅台",
            "R1",
            "O1",
            "2025-12-31",
            evidence("target-batch", "2025-12-31"),
        )],
        aggregate_evidence.clone(),
    )
    .is_err());
    assert!(TargetPriceConsensus::new(
        &request,
        vec![observation_with_identity(
            instrument(),
            "贵州茅台",
            "R1",
            "O1",
            "2026-07-20",
            evidence("other-batch", "2026-07-20"),
        )],
        aggregate_evidence,
    )
    .is_err());
}

#[test]
fn aggregate_accessors_and_evidence_are_source_preserving() {
    let request = TargetPriceRequest::new(
        instrument(),
        IsoDate::new("2026-01-01").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
    )
    .unwrap();
    let aggregate = TargetPriceConsensus::new(
        &request,
        vec![observation("R1", "O1", "2026-07-20", 1430.0, 1400.0)],
        evidence("target-batch", "2026-07-20"),
    )
    .unwrap();
    assert_eq!(aggregate.instrument(), &instrument());
    assert_eq!(aggregate.requested_from().as_str(), "2026-01-01");
    assert_eq!(aggregate.requested_through().as_str(), "2026-07-27");
    assert_eq!(aggregate.observations().len(), 1);
    assert_eq!(aggregate.evidence().batch_id(), "target-batch");
    assert_eq!(aggregate.provider_id(), ProviderId::Eastmoney);
    assert_eq!(aggregate.evidence_batch_id(), "target-batch");

    let mut reversed_request = serde_json::to_value(&aggregate).unwrap();
    reversed_request["requested_from"] = serde_json::json!("2026-07-28");
    assert!(serde_json::from_value::<TargetPriceConsensus>(reversed_request).is_err());

    let missing_source_time = SourceEvidence::new(
        ProviderId::Eastmoney,
        "2026-07-27T10:00:00+08:00",
        "target-batch",
    )
    .unwrap();
    assert!(TargetPriceConsensus::new(
        &request,
        vec![observation("R1", "O1", "2026-07-20", 1430.0, 1400.0)],
        missing_source_time,
    )
    .is_err());
}
