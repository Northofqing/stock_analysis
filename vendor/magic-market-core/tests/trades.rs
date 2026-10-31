use magic_market_core::{
    AssetClass, DataStatus, Exchange, InstrumentId, Price, ProviderId, Quantity, Trade, TradeSide,
    TradesRequest,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap()
}

#[test]
fn trade_request_validates_limit_and_calendar_date() {
    assert!(TradesRequest::new(instrument(), 0).is_err());
    assert!(TradesRequest::new(instrument(), 20)
        .unwrap()
        .with_date("2026-02-29")
        .is_err());
    let request = TradesRequest::new(instrument(), 2_000)
        .unwrap()
        .with_date("2024-02-29")
        .unwrap();
    assert_eq!(request.date(), Some("2024-02-29"));
    assert_eq!(request.limit(), 2_000);
    assert_eq!(request.instrument(), &instrument());
    assert_eq!(
        serde_json::from_str::<TradesRequest>(&serde_json::to_string(&request).unwrap()).unwrap(),
        request
    );
    let current = TradesRequest::new(instrument(), 1).unwrap();
    assert_eq!(
        serde_json::from_str::<TradesRequest>(&serde_json::to_string(&current).unwrap()).unwrap(),
        current
    );

    assert!(Trade::new(
        instrument(),
        "10:00:00",
        Price::new(10.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        Some(0),
        TradeSide::Buy,
        DataStatus::Available,
        Some("source".into()),
        "observed",
        ProviderId::Tdx,
        "batch",
    )
    .is_err());
    let trade = Trade::new(
        instrument(),
        "10:00:00",
        Price::new(10.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        Some(2),
        TradeSide::Buy,
        DataStatus::Available,
        Some("source".into()),
        "observed",
        ProviderId::Tdx,
        "batch",
    )
    .unwrap();
    assert_eq!(trade.instrument(), &instrument());
    assert_eq!(trade.trade_at(), "10:00:00");
    assert_eq!(trade.price().get(), 10.0);
    assert_eq!(trade.quantity().get(), 100.0);
    assert_eq!(trade.trade_count(), Some(2));
    assert_eq!(trade.side(), TradeSide::Buy);
    assert_eq!(trade.status(), DataStatus::Available);
    assert_eq!(trade.source_at(), Some("source"));
    assert_eq!(trade.observed_at(), "observed");
    assert_eq!(trade.provider(), ProviderId::Tdx);
    assert_eq!(trade.batch_id(), "batch");
}
