use magic_market_core::{AssetClass, BarInterval, BarsRequest, Exchange, InstrumentId};
#[test]
fn request_rejects_zero_limit() {
    let id = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    assert!(BarsRequest::new(id, BarInterval::Day, 0).is_err());
}

#[test]
fn request_validates_date_range() {
    let id = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let req = BarsRequest::new(id, BarInterval::Day, 1).unwrap();
    assert!(req.clone().with_range("2026-07-22", "2026-07-21").is_err());
    assert!(req.clone().with_range("2026-07-21", "2026-07-22").is_ok());
    assert!(req.clone().with_range("2026-99-01", "2026-99-02").is_err());
    assert!(req.clone().with_range("2026-02-31", "2026-03-01").is_err());
    assert!(req.clone().with_range("0000-01-01", "2026-03-01").is_err());
    assert!(req.clone().with_range("1899-12-31", "2026-03-01").is_err());
    assert!(req.clone().with_range("2026/01/01", "2026-03-01").is_err());
    assert!(req.with_range("2024-02-29", "2024-03-01").is_ok());
}

#[test]
fn request_exposes_validated_values_through_accessors() {
    let id = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let req = BarsRequest::new(id.clone(), BarInterval::Day, 5)
        .unwrap()
        .with_range("2026-07-21", "2026-07-22")
        .unwrap();

    assert_eq!(req.instrument(), &id);
    assert_eq!(req.interval(), BarInterval::Day);
    assert_eq!(req.start(), Some("2026-07-21"));
    assert_eq!(req.end(), Some("2026-07-22"));
    assert_eq!(req.limit(), 5);
    assert_eq!(
        serde_json::from_str::<BarsRequest>(&serde_json::to_string(&req).unwrap()).unwrap(),
        req
    );

    let unbounded = BarsRequest::new(id, BarInterval::Day, 1).unwrap();
    assert_eq!(
        serde_json::from_str::<BarsRequest>(&serde_json::to_string(&unbounded).unwrap()).unwrap(),
        unbounded
    );
    let mut one_sided = serde_json::to_value(&unbounded).unwrap();
    one_sided["start"] = serde_json::json!("2026-07-21");
    assert!(serde_json::from_value::<BarsRequest>(one_sided).is_err());
}
