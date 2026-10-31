use magic_market_core::{
    AssetClass, Board, DataStatus, Exchange, InstrumentId, PriceLimitRule, ProviderId,
    SecurityMetadata,
};

#[test]
fn beijing_is_a_first_class_exchange() {
    let instrument = InstrumentId::new(Exchange::Beijing, "920001", AssetClass::Equity).unwrap();
    assert_eq!(instrument.exchange(), Exchange::Beijing);
}

#[test]
fn security_metadata_preserves_unavailable_source_fields() {
    let metadata = SecurityMetadata::new(
        InstrumentId::new(Exchange::Shanghai, "688001", AssetClass::Equity).unwrap(),
        Some("示例证券".into()),
        Some(Board::Star),
        Some(false),
        None,
        PriceLimitRule::new(None, None).unwrap(),
        DataStatus::Unavailable,
        None,
        "observed",
        ProviderId::Tdx,
        "batch-1",
    )
    .unwrap();

    assert_eq!(metadata.board(), Some(Board::Star));
    assert!(metadata.listed_on().is_none());
    assert!(metadata.price_limit().percent().is_none());
    assert!(metadata.price_limit().version().is_none());
    assert_eq!(metadata.status(), DataStatus::Unavailable);
}
