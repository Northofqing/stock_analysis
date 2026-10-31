use magic_market_core::{
    BoardCategory, BoardConstituentRequest, BoardDefinition, BoardDirectoryRequest,
    DragonTigerDiscoveryRequest, Exchange, IsoDate, MarketDiscoveryCapabilities, NonEmptyText,
    PositiveU32, ProviderId, SourceEvidence, SourcedRecord,
};

#[test]
fn discovery_requests_are_explicit_bounded_and_serde_checked() {
    let date = IsoDate::new("2026-07-24").unwrap();
    let request = DragonTigerDiscoveryRequest::new(date.clone(), PositiveU32::new(10_000).unwrap())
        .unwrap()
        .with_exchange(Exchange::Beijing);
    assert_eq!(request.trading_date(), &date);
    assert_eq!(request.exchange(), Some(Exchange::Beijing));
    assert_eq!(request.limit().get(), 10_000);
    assert_eq!(
        serde_json::from_value::<DragonTigerDiscoveryRequest>(
            serde_json::to_value(&request).unwrap()
        )
        .unwrap()
        .exchange(),
        Some(Exchange::Beijing)
    );
    let all_exchanges =
        DragonTigerDiscoveryRequest::new(date.clone(), PositiveU32::new(1).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_value::<DragonTigerDiscoveryRequest>(
            serde_json::to_value(&all_exchanges).unwrap()
        )
        .unwrap()
        .exchange(),
        None
    );
    assert!(DragonTigerDiscoveryRequest::new(date, PositiveU32::new(10_001).unwrap()).is_err());

    let mut oversized = serde_json::to_value(&request).unwrap();
    oversized["limit"] = serde_json::json!(10_001);
    assert!(serde_json::from_value::<DragonTigerDiscoveryRequest>(oversized).is_err());

    let directory =
        BoardDirectoryRequest::new(BoardCategory::Concept, PositiveU32::new(200).unwrap()).unwrap();
    assert_eq!(directory.category(), BoardCategory::Concept);
    assert_eq!(directory.limit().get(), 200);

    let constituents = BoardConstituentRequest::new(
        NonEmptyText::new("tdx:concept:人工智能").unwrap(),
        PositiveU32::new(400).unwrap(),
    )
    .unwrap();
    assert_eq!(constituents.board_code().as_str(), "tdx:concept:人工智能");
    assert_eq!(constituents.limit().get(), 400);

    let mut oversized = serde_json::to_value(&directory).unwrap();
    oversized["limit"] = serde_json::json!(10_001);
    assert!(serde_json::from_value::<BoardDirectoryRequest>(oversized).is_err());

    let mut oversized = serde_json::to_value(&constituents).unwrap();
    oversized["limit"] = serde_json::json!(10_001);
    assert!(serde_json::from_value::<BoardConstituentRequest>(oversized).is_err());
}

#[test]
fn board_definition_is_sourced_and_serde_checked() {
    let evidence = SourceEvidence::new(ProviderId::Tdx, "observed", "batch").unwrap();
    let board = BoardDefinition::new(
        NonEmptyText::new("tdx:industry:电力").unwrap(),
        NonEmptyText::new("电力").unwrap(),
        BoardCategory::Industry,
        PositiveU32::new(42).unwrap(),
        evidence,
    )
    .unwrap();
    assert_eq!(board.board_code().as_str(), "tdx:industry:电力");
    assert_eq!(board.board_name().as_str(), "电力");
    assert_eq!(board.category(), BoardCategory::Industry);
    assert_eq!(board.member_count().get(), 42);
    assert_eq!(board.evidence().provider(), ProviderId::Tdx);
    assert_eq!(board.provider_id(), ProviderId::Tdx);
    assert_eq!(board.evidence_batch_id(), "batch");

    let json = serde_json::to_string(&board).unwrap();
    assert_eq!(
        serde_json::from_str::<BoardDefinition>(&json).unwrap(),
        board
    );
}

#[test]
fn discovery_capabilities_default_to_false() {
    assert_eq!(
        MarketDiscoveryCapabilities::default(),
        MarketDiscoveryCapabilities {
            dragon_tiger_discovery: false,
            board_directory: false,
            board_memberships: false,
            board_constituents: false,
        }
    );
}
