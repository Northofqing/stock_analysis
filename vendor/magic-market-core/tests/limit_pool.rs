use magic_market_core::{
    AssetClass, Exchange, InstrumentId, IsoDate, LimitPoolEntry, LimitPoolKind, LimitPoolRequest,
    Money, NonEmptyText, PositiveU32, Price, ProviderId, Quantity, Ratio, RatioUnit,
    SourceEvidence, SourcedRecord,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

#[test]
fn all_four_limit_pool_kinds_are_explicit() {
    let kinds = [
        LimitPoolKind::Upper,
        LimitPoolKind::Broken,
        LimitPoolKind::Lower,
        LimitPoolKind::PreviousUpper,
    ];
    assert_eq!(kinds.len(), 4);
    assert!(LimitPoolRequest::new(
        LimitPoolKind::Upper,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1_001).unwrap()
    )
    .is_err());

    let request = LimitPoolRequest::new(
        LimitPoolKind::Broken,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(100).unwrap(),
    )
    .unwrap();
    assert_eq!(request.kind(), LimitPoolKind::Broken);
    assert_eq!(request.trading_date().as_str(), "2026-07-23");
    assert_eq!(request.limit().get(), 100);
    assert_eq!(
        serde_json::from_value::<LimitPoolRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );
}

#[test]
fn limit_entry_round_trips_with_optional_reason() {
    let entry = LimitPoolEntry {
        kind: LimitPoolKind::Upper,
        instrument: instrument(),
        trading_date: IsoDate::new("2026-07-23").unwrap(),
        price: Price::new(4.0).unwrap(),
        change: Ratio::new(10.0, RatioUnit::Percent).unwrap(),
        volume: Some(Quantity::new(100.0).unwrap()),
        turnover: None,
        sealed_amount: Some(Money::new(0.0).unwrap()),
        first_seal_at: None,
        last_seal_at: None,
        break_count: None,
        streak: Some(PositiveU32::new(2).unwrap()),
        industry: None,
        board_name: None,
        seal_state: None,
        reseal_count: None,
        reason: Some(NonEmptyText::new("电力").unwrap()),
        evidence: SourceEvidence::new(ProviderId::Eastmoney, "observed", "pool").unwrap(),
    };
    assert_eq!(entry.provider_id(), ProviderId::Eastmoney);
    assert_eq!(entry.evidence_batch_id(), "pool");
    assert_eq!(
        serde_json::from_str::<LimitPoolEntry>(&serde_json::to_string(&entry).unwrap()).unwrap(),
        entry
    );
}
