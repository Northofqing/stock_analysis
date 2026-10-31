use magic_market_core::{
    AssetClass, DataStatus, Exchange, InstrumentId, MinuteDataRequest, MinutePoint, Money, Price,
    ProviderId, Quantity,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

#[test]
fn minute_point_round_trips_through_checked_serde() {
    let point = MinutePoint::new(
        instrument(),
        "2026-07-23 09:31",
        Price::new(15.5).unwrap(),
        Quantity::new(100.0).unwrap(),
        Some(Money::new(155_000.0).unwrap()),
        DataStatus::Available,
        Some("2026-07-23T09:31:00+08:00".into()),
        "observed",
        ProviderId::Tencent,
        "batch",
    )
    .unwrap();
    let json = serde_json::to_string(&point).unwrap();
    let decoded: MinutePoint = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, point);
    assert_eq!(decoded.instrument(), &instrument());
    assert_eq!(decoded.minute_at(), "2026-07-23 09:31");
    assert_eq!(decoded.price().get(), 15.5);
    assert_eq!(decoded.cumulative_quantity().get(), 100.0);
    assert_eq!(decoded.cumulative_amount().unwrap().get(), 155_000.0);
    assert_eq!(decoded.status(), DataStatus::Available);
    assert_eq!(decoded.source_at(), Some("2026-07-23T09:31:00+08:00"));
    assert_eq!(decoded.observed_at(), "observed");
    assert_eq!(decoded.provider(), ProviderId::Tencent);
    assert_eq!(decoded.batch_id(), "batch");
}

#[test]
fn minute_point_rejects_invalid_time_amount_and_status() {
    let base = |minute_at: &str, amount: Option<Money>, status, source_at| {
        MinutePoint::new(
            instrument(),
            minute_at,
            Price::new(15.5).unwrap(),
            Quantity::new(100.0).unwrap(),
            amount,
            status,
            source_at,
            "observed",
            ProviderId::Tencent,
            "batch",
        )
    };
    assert!(base("2026-02-30 09:31", None, DataStatus::Unavailable, None).is_err());
    assert!(base("2026-07-23 24:00", None, DataStatus::Unavailable, None).is_err());
    assert!(base(
        "2026-07-23 09:31",
        Money::new(-1.0).ok(),
        DataStatus::Unavailable,
        None
    )
    .is_err());
    assert!(base("2026-07-23 09:31", None, DataStatus::Available, None).is_err());
}

#[test]
fn minute_request_validates_and_round_trips_date() {
    let request = MinuteDataRequest::new(instrument())
        .with_date("2026-07-23")
        .unwrap();
    assert_eq!(request.date(), Some("2026-07-23"));
    assert_eq!(request.instrument(), &instrument());
    let json = serde_json::to_string(&request).unwrap();
    let decoded: MinuteDataRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, request);
    assert!(MinuteDataRequest::new(instrument())
        .with_date("2026-02-30")
        .is_err());
    assert!(serde_json::from_str::<MinuteDataRequest>(
        r#"{"instrument":{"exchange":"Shanghai","code":"600396","asset_class":"Equity"},"date":"2026-02-30"}"#
    )
    .is_err());
    let current = MinuteDataRequest::new(instrument());
    assert_eq!(
        serde_json::from_str::<MinuteDataRequest>(&serde_json::to_string(&current).unwrap())
            .unwrap(),
        current
    );
}
