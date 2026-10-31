use super::*;

#[test]
fn global_requests_reject_empty_and_duplicate_identities() {
    assert!(GlobalIndexRequest::new(Vec::new()).is_err());
    assert!(GlobalIndexRequest::new(vec![GlobalIndexCode::Sp500, GlobalIndexCode::Sp500]).is_err());
    assert!(FxRequest::new(vec![FxPair::UsdCny, FxPair::UsdCny]).is_err());
}

#[test]
fn global_requests_revalidate_deserialized_values() {
    let duplicate = r#"{"indices":["DowJones","DowJones"]}"#;
    assert!(serde_json::from_str::<GlobalIndexRequest>(duplicate).is_err());
}

#[test]
fn global_and_fx_requests_round_trip_and_expose_identities() {
    let indices = vec![GlobalIndexCode::DowJones, GlobalIndexCode::Sp500];
    let request = GlobalIndexRequest::new(indices.clone()).unwrap();
    assert_eq!(request.indices(), indices);
    assert_eq!(
        serde_json::from_str::<GlobalIndexRequest>(r#"{"indices":["DowJones","Sp500"]}"#).unwrap(),
        request
    );

    let pairs = vec![FxPair::UsdCny, FxPair::EurUsd];
    let fx_request = FxRequest::new(pairs.clone()).unwrap();
    assert_eq!(fx_request.pairs(), pairs);
    assert_eq!(
        serde_json::from_str::<FxRequest>(r#"{"pairs":["UsdCny","EurUsd"]}"#).unwrap(),
        fx_request
    );
}

#[test]
fn global_and_fx_requests_enforce_empty_and_maximum_cardinality() {
    assert!(FxRequest::new(Vec::new()).is_err());
    assert!(GlobalIndexRequest::new(vec![GlobalIndexCode::Sp500; 21]).is_err());
    assert!(FxRequest::new(vec![FxPair::UsdCny; 21]).is_err());
}

#[test]
fn global_records_expose_source_identity() {
    let evidence =
        SourceEvidence::new(crate::ProviderId::Sina, "observed", "global-batch").unwrap();
    let index = GlobalIndexQuote {
        index: GlobalIndexCode::Sp500,
        name: NonEmptyText::new("S&P 500").unwrap(),
        value: Price::new(6_400.0).unwrap(),
        change: FiniteNumber::new(5.0).unwrap(),
        change_percent: Ratio::new(0.08, crate::RatioUnit::Percent).unwrap(),
        evidence: evidence.clone(),
    };
    assert_eq!(index.provider_id(), crate::ProviderId::Sina);
    assert_eq!(index.evidence_batch_id(), "global-batch");

    let fx = FxQuote {
        pair: FxPair::UsdCny,
        name: NonEmptyText::new("美元人民币").unwrap(),
        rate: Price::new(7.16).unwrap(),
        change: None,
        change_percent: None,
        evidence,
    };
    assert_eq!(fx.provider_id(), crate::ProviderId::Sina);
    assert_eq!(fx.evidence_batch_id(), "global-batch");
}
