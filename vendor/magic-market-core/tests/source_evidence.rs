use magic_market_core::{ProviderId, SourceEvidence};

#[test]
fn source_evidence_keeps_source_and_observation_times_separate() {
    let evidence = SourceEvidence::new(
        ProviderId::Eastmoney,
        "2026-07-23T10:00:01+08:00",
        "eastmoney:600396:1",
    )
    .unwrap()
    .with_source_at("2026-07-23T10:00:00+08:00")
    .unwrap();

    assert_eq!(evidence.provider(), ProviderId::Eastmoney);
    assert_eq!(evidence.source_at(), Some("2026-07-23T10:00:00+08:00"));
    assert_eq!(evidence.observed_at(), "2026-07-23T10:00:01+08:00");
    assert_eq!(evidence.batch_id(), "eastmoney:600396:1");

    let json = serde_json::to_string(&evidence).unwrap();
    assert_eq!(
        serde_json::from_str::<SourceEvidence>(&json).unwrap(),
        evidence
    );
}

#[test]
fn source_evidence_cannot_be_forged_through_serde() {
    assert!(SourceEvidence::new(ProviderId::Tencent, " ", "batch").is_err());
    assert!(SourceEvidence::new(ProviderId::Tencent, "observed", " ").is_err());
    assert!(serde_json::from_str::<SourceEvidence>(
        r#"{"provider":"Tencent","source_at":null,"observed_at":"","batch_id":"batch"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<SourceEvidence>(
        r#"{"provider":"Tencent","source_at":"bad\nvalue","observed_at":"observed","batch_id":"batch"}"#
    )
    .is_err());
}
