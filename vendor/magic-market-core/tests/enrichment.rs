use magic_market_core::{
    Adjustment, AssetClass, Bar, BarInterval, Exchange, FiniteNumber, InstrumentId,
    MarketStatistics, Money, Price, ProviderId, Quantity, Ratio, SourceEvidence, SourcedRecord,
    TechnicalBar,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

fn evidence() -> SourceEvidence {
    SourceEvidence::new(ProviderId::Tencent, "observed", "tencent:600396").unwrap()
}

#[test]
fn market_statistics_preserve_optional_values_and_evidence() {
    let statistics = MarketStatistics::new(
        instrument(),
        Some(Ratio::new(2.5, magic_market_core::RatioUnit::Percent).unwrap()),
        Some(FiniteNumber::new(12.3).unwrap()),
        None,
        Some(FiniteNumber::new(1.4).unwrap()),
        Some(Money::new(6_000_000_000.0).unwrap()),
        None,
        Some(Price::new(4.4).unwrap()),
        Some(Price::new(3.6).unwrap()),
        Some(FiniteNumber::new(0.9).unwrap()),
        evidence(),
    )
    .unwrap();

    assert_eq!(statistics.instrument(), &instrument());
    assert_eq!(statistics.turnover_rate().unwrap().get(), 2.5);
    assert_eq!(statistics.trailing_pe().unwrap().get(), 12.3);
    assert!(statistics.static_pe().is_none());
    assert_eq!(statistics.pb().unwrap().get(), 1.4);
    assert_eq!(
        statistics.total_market_cap().unwrap().get(),
        6_000_000_000.0
    );
    assert!(statistics.floating_market_cap().is_none());
    assert_eq!(statistics.upper_limit().unwrap().get(), 4.4);
    assert_eq!(statistics.lower_limit().unwrap().get(), 3.6);
    assert_eq!(statistics.volume_ratio().unwrap().get(), 0.9);
    assert_eq!(statistics.evidence().provider(), ProviderId::Tencent);
    assert_eq!(statistics.provider_id(), ProviderId::Tencent);
    assert_eq!(statistics.evidence_batch_id(), "tencent:600396");

    let json = serde_json::to_string(&statistics).unwrap();
    assert_eq!(
        serde_json::from_str::<MarketStatistics>(&json).unwrap(),
        statistics
    );
}

#[test]
fn market_statistics_reject_negative_capitalization_through_serde() {
    let json = r#"{
        "instrument":{"exchange":"Shanghai","code":"600396","asset_class":"Equity"},
        "turnover_rate":null,"trailing_pe":null,"static_pe":null,"pb":null,
        "total_market_cap":-1.0,"floating_market_cap":null,
        "upper_limit":null,"lower_limit":null,"volume_ratio":null,
        "evidence":{"provider":"Tencent","source_at":null,"observed_at":"observed","batch_id":"batch"}
    }"#;
    assert!(serde_json::from_str::<MarketStatistics>(json).is_err());
}

#[test]
fn technical_bar_distinguishes_source_mas_from_raw_bar() {
    let bar = Bar::new(
        instrument(),
        BarInterval::Day,
        "2026-07-23",
        "2026-07-23",
        Price::new(4.0).unwrap(),
        Price::new(4.2).unwrap(),
        Price::new(3.9).unwrap(),
        Price::new(4.1).unwrap(),
        Quantity::new(100.0).unwrap(),
        None,
        Adjustment::Unadjusted,
        ProviderId::Baidu,
        "baidu:600396",
    )
    .unwrap();
    let technical = TechnicalBar::new(
        bar.clone(),
        Some(Price::new(4.0).unwrap()),
        None,
        None,
        SourceEvidence::new(ProviderId::Baidu, "observed", "baidu:600396").unwrap(),
    )
    .unwrap();

    assert_eq!(technical.ma5().unwrap().get(), 4.0);
    assert!(technical.ma10().is_none());
    assert!(technical.ma20().is_none());
    assert_eq!(technical.bar(), &bar);
    assert_eq!(technical.evidence().provider(), ProviderId::Baidu);
    assert_eq!(technical.provider_id(), ProviderId::Baidu);
    assert_eq!(technical.evidence_batch_id(), "baidu:600396");
    assert_eq!(
        serde_json::from_value::<TechnicalBar>(serde_json::to_value(&technical).unwrap()).unwrap(),
        technical
    );
    assert!(TechnicalBar::new(
        bar.clone(),
        None,
        None,
        None,
        SourceEvidence::new(ProviderId::Tencent, "observed", "baidu:600396").unwrap()
    )
    .is_err());
    assert!(TechnicalBar::new(
        bar,
        None,
        None,
        None,
        SourceEvidence::new(ProviderId::Baidu, "observed", "wrong-batch").unwrap()
    )
    .is_err());
}
