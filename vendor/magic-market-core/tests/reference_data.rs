use magic_market_core::{
    CurrencyCode, EconomicRevision, EconomicRevisionKind, FiniteNumber, IsoDate, NonEmptyText,
    OfficialFxFixing, OfficialFxFixingIdentity, OfficialFxFixingRequest, PositiveU32, ProviderId,
    RatioUnit, ReferenceRateIdentity, ReferenceRateKind, ReferenceRateObservation,
    ReferenceRateRequest, ReferenceTenor, SourceEvidence, SourcedRecord,
};

#[test]
fn currency_and_request_identities_are_checked() {
    assert_eq!(CurrencyCode::new("cny").unwrap().as_str(), "CNY");
    assert!(CurrencyCode::new("CN").is_err());
    assert!(CurrencyCode::new("C1Y").is_err());
    let pair = OfficialFxFixingIdentity::new(
        ProviderId::Cfets,
        CurrencyCode::new("USD").unwrap(),
        CurrencyCode::new("CNY").unwrap(),
    )
    .unwrap();
    assert!(OfficialFxFixingRequest::new(
        vec![pair.clone(), pair],
        IsoDate::new("2026-07-01").unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        PositiveU32::new(50).unwrap(),
    )
    .is_err());
}

#[test]
fn official_fixing_requires_positive_value_and_quotation_base() {
    let evidence =
        SourceEvidence::new(ProviderId::Cfets, "2026-07-29T02:00:00Z", "cfets-1").unwrap();
    assert!(OfficialFxFixing::new(
        CurrencyCode::new("USD").unwrap(),
        CurrencyCode::new("CNY").unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(0.0).unwrap(),
        PositiveU32::new(1).unwrap(),
        None,
        None,
        evidence,
    )
    .is_err());
}

#[test]
fn serde_cannot_bypass_reference_data_invariants() {
    let zero = r#"{
      "identity":{"provider":"Cfets","base":"USD","quote":"CNY"},
      "fixing_date":"2026-07-29","value":0.0,"quotation_base":1,
      "published_at":null,"revision":null,
      "evidence":{"provider":"Cfets","source_at":null,
        "observed_at":"2026-07-29T02:00:00Z","batch_id":"cfets-1"}
    }"#;
    assert!(serde_json::from_str::<OfficialFxFixing>(zero).is_err());

    let duplicate = r#"{
      "rates":[
        {"provider":"Cfets","kind":{"Shibor":"OneMonth"}},
        {"provider":"Cfets","kind":{"Shibor":"OneMonth"}}
      ],
      "start":"2026-07-01","end":"2026-07-29","max_rows":50
    }"#;
    assert!(serde_json::from_str::<ReferenceRateRequest>(duplicate).is_err());

    let invalid_lpr = r#"{"provider":"Cfets","kind":{"LoanPrimeRate":"OneMonth"}}"#;
    assert!(serde_json::from_str::<ReferenceRateIdentity>(invalid_lpr).is_err());

    let valid = ReferenceRateIdentity::new(
        ProviderId::Cfets,
        ReferenceRateKind::Shibor(ReferenceTenor::Overnight),
    )
    .unwrap();
    assert_eq!(valid.provider(), ProviderId::Cfets);
}

#[test]
fn reference_records_reject_provider_and_publication_evidence_conflicts() {
    let identity = ReferenceRateIdentity::new(
        ProviderId::Cfets,
        ReferenceRateKind::Shibor(ReferenceTenor::Overnight),
    )
    .unwrap();
    let foreign = SourceEvidence::new(ProviderId::Fred, "2026-07-29T03:00:00Z", "fred-1").unwrap();
    assert!(ReferenceRateObservation::new(
        identity.clone(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        RatioUnit::Percent,
        None,
        None,
        foreign,
    )
    .is_err());

    let sourced = SourceEvidence::new(ProviderId::Cfets, "2026-07-29T03:00:00Z", "cfets-1")
        .unwrap()
        .with_source_at("2026-07-29T02:00:00Z")
        .unwrap();
    assert!(ReferenceRateObservation::new(
        identity,
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        RatioUnit::Percent,
        None,
        None,
        sourced,
    )
    .is_err());

    let fixing_mismatch = r#"{
      "identity":{"provider":"Fred","base":"USD","quote":"CNY"},
      "fixing_date":"2026-07-29","value":6.8,"quotation_base":1,
      "published_at":null,"revision":null,
      "evidence":{"provider":"Cfets","source_at":null,
        "observed_at":"2026-07-29T03:00:00Z","batch_id":"cfets-1"}
    }"#;
    assert!(serde_json::from_str::<OfficialFxFixing>(fixing_mismatch).is_err());
}

#[test]
fn reference_rate_success_path_accessors_and_serde_are_complete() {
    assert!(ReferenceRateIdentity::new(
        ProviderId::Cfets,
        ReferenceRateKind::Shibor(ReferenceTenor::OverFiveYears)
    )
    .is_err());

    let published_at = NonEmptyText::new("2026-07-29T03:00:00Z").unwrap();
    let evidence = SourceEvidence::new(ProviderId::Cfets, "2026-07-29T04:00:00Z", "rates")
        .unwrap()
        .with_source_at(published_at.as_str())
        .unwrap();
    let revision = EconomicRevision {
        kind: EconomicRevisionKind::Final,
        label: Some(NonEmptyText::new("official").unwrap()),
    };
    let observation = ReferenceRateObservation::new(
        ReferenceRateIdentity::new(
            ProviderId::Cfets,
            ReferenceRateKind::Shibor(ReferenceTenor::OneWeek),
        )
        .unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(1.5).unwrap(),
        RatioUnit::Percent,
        Some(published_at),
        Some(revision),
        evidence,
    )
    .unwrap();
    assert_eq!(observation.rate().get(), 1.5);
    assert_eq!(observation.published_at(), Some("2026-07-29T03:00:00Z"));
    assert!(observation.revision().is_some());
    assert_eq!(observation.evidence().batch_id(), "rates");
    let round_trip: ReferenceRateObservation =
        serde_json::from_str(&serde_json::to_string(&observation).unwrap()).unwrap();
    assert_eq!(round_trip, observation);
}

#[test]
fn reference_requests_cover_empty_mixed_reversed_and_oversized_bounds() {
    let start = IsoDate::new("2026-07-01").unwrap();
    let end = IsoDate::new("2026-07-29").unwrap();
    let limit = PositiveU32::new(10).unwrap();
    assert!(ReferenceRateRequest::new(vec![], start.clone(), end.clone(), limit).is_err());
    let cfets = ReferenceRateIdentity::new(
        ProviderId::Cfets,
        ReferenceRateKind::Shibor(ReferenceTenor::Overnight),
    )
    .unwrap();
    let foreign = ReferenceRateIdentity::new(
        ProviderId::Pbc,
        ReferenceRateKind::SourceDefined(NonEmptyText::new("rate").unwrap()),
    )
    .unwrap();
    assert!(ReferenceRateRequest::new(
        vec![cfets.clone(), foreign],
        start.clone(),
        end.clone(),
        limit
    )
    .is_err());
    assert!(
        ReferenceRateRequest::new(vec![cfets.clone()], end.clone(), start.clone(), limit).is_err()
    );
    assert!(
        ReferenceRateRequest::new(vec![cfets], start, end, PositiveU32::new(10_001).unwrap())
            .is_err()
    );
}

#[test]
fn official_fx_success_path_and_request_serde_cover_all_accessors() {
    let usd = CurrencyCode::new("USD").unwrap();
    let cny = CurrencyCode::new("CNY").unwrap();
    assert!(OfficialFxFixingIdentity::new(ProviderId::Cfets, usd.clone(), usd.clone()).is_err());

    let published_at = NonEmptyText::new("2026-07-29T01:00:00Z").unwrap();
    let evidence = SourceEvidence::new(ProviderId::Cfets, "2026-07-29T02:00:00Z", "fx")
        .unwrap()
        .with_source_at(published_at.as_str())
        .unwrap();
    let revision = EconomicRevision {
        kind: EconomicRevisionKind::Revised,
        label: None,
    };
    let fixing = OfficialFxFixing::new(
        usd.clone(),
        cny.clone(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(7.1).unwrap(),
        PositiveU32::new(1).unwrap(),
        Some(published_at),
        Some(revision),
        evidence,
    )
    .unwrap();
    assert_eq!(fixing.published_at(), Some("2026-07-29T01:00:00Z"));
    assert!(fixing.revision().is_some());
    assert_eq!(fixing.evidence().batch_id(), "fx");

    let pair = OfficialFxFixingIdentity::new(ProviderId::Cfets, usd, cny).unwrap();
    let request = OfficialFxFixingRequest::new(
        vec![pair],
        IsoDate::new("2026-07-01").unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        PositiveU32::new(50).unwrap(),
    )
    .unwrap();
    let wire = serde_json::to_string(&request).unwrap();
    assert_eq!(
        serde_json::from_str::<OfficialFxFixingRequest>(&wire).unwrap(),
        request
    );

    let mismatched = SourceEvidence::new(ProviderId::Cfets, "2026-07-29T02:00:00Z", "fx").unwrap();
    assert!(OfficialFxFixing::new(
        CurrencyCode::new("USD").unwrap(),
        CurrencyCode::new("CNY").unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(7.1).unwrap(),
        PositiveU32::new(1).unwrap(),
        Some(NonEmptyText::new("2026-07-29T01:00:00Z").unwrap()),
        None,
        mismatched,
    )
    .is_err());
}

#[test]
fn reference_records_expose_all_source_evidence() {
    let published_at = "2026-07-29T02:00:00Z";
    let observed_at = "2026-07-29T03:00:00Z";
    let evidence = SourceEvidence::new(ProviderId::Cfets, observed_at, "cfets-evidence")
        .unwrap()
        .with_source_at(published_at)
        .unwrap();
    let rate = ReferenceRateObservation::new(
        ReferenceRateIdentity::new(
            ProviderId::Cfets,
            ReferenceRateKind::Shibor(ReferenceTenor::Overnight),
        )
        .unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(1.0).unwrap(),
        RatioUnit::Percent,
        Some(magic_market_core::NonEmptyText::new(published_at).unwrap()),
        None,
        evidence.clone(),
    )
    .unwrap();
    let fixing = OfficialFxFixing::new(
        CurrencyCode::new("USD").unwrap(),
        CurrencyCode::new("CNY").unwrap(),
        IsoDate::new("2026-07-29").unwrap(),
        FiniteNumber::new(6.8).unwrap(),
        PositiveU32::new(1).unwrap(),
        Some(magic_market_core::NonEmptyText::new(published_at).unwrap()),
        None,
        evidence,
    )
    .unwrap();

    for record in [&rate as &dyn SourcedRecord, &fixing as &dyn SourcedRecord] {
        assert_eq!(record.provider_id(), ProviderId::Cfets);
        assert_eq!(record.evidence_batch_id(), "cfets-evidence");
        assert_eq!(record.evidence_source_at(), Some(published_at));
        assert_eq!(record.evidence_observed_at(), Some(observed_at));
    }
}
