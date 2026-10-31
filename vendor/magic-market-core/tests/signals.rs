use magic_market_core::{
    AssetClass, BoardCategory, BoardMembership, ConceptHit, DragonTigerDisclosure,
    DragonTigerEntry, DragonTigerSeat, DragonTigerSide, Exchange, FiniteNumber, InstrumentId,
    InstrumentSignalRequest, IsoDate, MarketDragonTigerRequest, MarketRankingEntry,
    MarketRankingKind, MarketRankingUnit, MarketSession, Money, NonEmptyText, PopularityRank,
    PositiveU32, ProviderId, Ratio, RatioUnit, SourceEvidence, SourcedRecord, StrongStockReason,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

fn evidence(provider: ProviderId, batch: &str) -> SourceEvidence {
    SourceEvidence::new(provider, "observed", batch).unwrap()
}

fn dated_evidence(provider: ProviderId, batch: &str) -> SourceEvidence {
    evidence(provider, batch)
        .with_source_at("2026-07-23")
        .unwrap()
}

#[test]
fn board_and_reason_contracts_preserve_unknown_categories() {
    let board = BoardMembership {
        instrument: instrument(),
        board_code: NonEmptyText::new("BK0001").unwrap(),
        board_name: NonEmptyText::new("混合板块").unwrap(),
        category: BoardCategory::Unknown,
        evidence: evidence(ProviderId::Eastmoney, "board"),
    };
    let reason = StrongStockReason {
        instrument: instrument(),
        trading_date: IsoDate::new("2026-07-23").unwrap(),
        reason: NonEmptyText::new("电力板块走强").unwrap(),
        subjects: vec![NonEmptyText::new("电力").unwrap()],
        limit_state: None,
        evidence: evidence(ProviderId::Tonghuashun, "reason"),
    };

    assert_eq!(board.category, BoardCategory::Unknown);
    assert_eq!(board.provider_id(), ProviderId::Eastmoney);
    assert_eq!(board.evidence_batch_id(), "board");
    assert_eq!(reason.provider_id(), ProviderId::Tonghuashun);
    assert_eq!(reason.evidence_batch_id(), "reason");
    assert_eq!(
        serde_json::from_str::<BoardMembership>(&serde_json::to_string(&board).unwrap()).unwrap(),
        board
    );
}

#[test]
fn dragon_tiger_missing_amounts_remain_absent() {
    let entry = DragonTigerEntry::new(
        NonEmptyText::new("entry-1").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        None,
        Some(Money::new(0.0).unwrap()),
        None,
        None,
        None,
        dated_evidence(ProviderId::Eastmoney, "entry"),
    )
    .unwrap();
    let seat = DragonTigerSeat::new(
        NonEmptyText::new("entry-1").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        DragonTigerSide::Buy,
        PositiveU32::new(1).unwrap(),
        NonEmptyText::new("机构专用").unwrap(),
        Money::new(0.0).unwrap(),
        Some(Money::new(0.0).unwrap()),
        None,
        None,
        dated_evidence(ProviderId::Eastmoney, "seat"),
    )
    .unwrap();

    assert_eq!(entry.buy_amount().unwrap().get(), 0.0);
    assert!(entry.sell_amount().is_none());
    assert_eq!(seat.rank().get(), 1);
    assert_eq!(seat.instrument(), &instrument());
    assert_eq!(seat.trading_date().as_str(), "2026-07-23");
    assert_eq!(entry.provider_id(), ProviderId::Eastmoney);
    assert_eq!(entry.evidence_batch_id(), "entry");
    assert_eq!(seat.provider_id(), ProviderId::Eastmoney);
    assert_eq!(seat.evidence_batch_id(), "seat");
    assert_eq!(
        serde_json::from_value::<DragonTigerEntry>(serde_json::to_value(&entry).unwrap()).unwrap(),
        entry
    );
    assert_eq!(
        serde_json::from_value::<DragonTigerSeat>(serde_json::to_value(&seat).unwrap()).unwrap(),
        seat
    );
    assert!(DragonTigerSeat::new(
        NonEmptyText::new("entry-1").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        DragonTigerSide::Buy,
        PositiveU32::new(1).unwrap(),
        NonEmptyText::new("机构专用").unwrap(),
        Money::new(100.0).unwrap(),
        None,
        None,
        None,
        dated_evidence(ProviderId::Eastmoney, "missing-side-amount"),
    )
    .is_err());
}

#[test]
fn dragon_tiger_checked_deserialization_rejects_semantic_bypasses() {
    let entry = DragonTigerEntry::new(
        NonEmptyText::new("entry-1").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        None,
        Some(Money::new(100.0).unwrap()),
        Some(Money::new(40.0).unwrap()),
        Some(Money::new(60.0).unwrap()),
        None,
        dated_evidence(ProviderId::Eastmoney, "entry"),
    )
    .unwrap();
    let seat = DragonTigerSeat::new(
        NonEmptyText::new("entry-1").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        DragonTigerSide::Buy,
        PositiveU32::new(1).unwrap(),
        NonEmptyText::new("机构专用").unwrap(),
        Money::new(100.0).unwrap(),
        Some(Money::new(100.0).unwrap()),
        Some(Money::new(40.0).unwrap()),
        Some(Money::new(60.0).unwrap()),
        dated_evidence(ProviderId::Eastmoney, "seat"),
    )
    .unwrap();

    let mut wrong_date = serde_json::to_value(&entry).unwrap();
    wrong_date["evidence"]["source_at"] = serde_json::json!("2026-07-22");
    assert!(serde_json::from_value::<DragonTigerEntry>(wrong_date).is_err());

    let mut inconsistent_net = serde_json::to_value(&entry).unwrap();
    inconsistent_net["net_amount"] = serde_json::json!(61.0);
    assert!(serde_json::from_value::<DragonTigerEntry>(inconsistent_net).is_err());

    let mut rank_six = serde_json::to_value(&seat).unwrap();
    rank_six["rank"] = serde_json::json!(6);
    assert!(serde_json::from_value::<DragonTigerSeat>(rank_six).is_err());

    let mut wrong_side_amount = serde_json::to_value(&seat).unwrap();
    wrong_side_amount["amount"] = serde_json::json!(99.0);
    assert!(serde_json::from_value::<DragonTigerSeat>(wrong_side_amount).is_err());

    let mut large_amount_bypass = serde_json::to_value(&seat).unwrap();
    large_amount_bypass["amount"] = serde_json::json!(1_000_000_000_000_000.0);
    large_amount_bypass["buy_amount"] = serde_json::json!(1_000_000_000_000_100.0);
    large_amount_bypass["sell_amount"] = serde_json::Value::Null;
    large_amount_bypass["net_amount"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<DragonTigerSeat>(large_amount_bypass).is_err());
}

#[test]
fn popularity_join_retains_both_evidence_records() {
    let rank = PopularityRank {
        instrument: instrument(),
        rank: PositiveU32::new(3).unwrap(),
        price: None,
        name: None,
        rank_change: None,
        return_ratio: None,
        heat: None,
        concepts: vec![],
        tag: None,
        quote_evidence: Some(evidence(ProviderId::Tencent, "quote")),
        evidence: evidence(ProviderId::Eastmoney, "rank"),
    };
    let hit = ConceptHit {
        instrument: instrument(),
        concept: NonEmptyText::new("绿色电力").unwrap(),
        detail: None,
        evidence: evidence(ProviderId::Eastmoney, "concept"),
    };
    let ranking = MarketRankingEntry::new(
        MarketRankingKind::Popularity,
        PositiveU32::new(1).unwrap(),
        Some(instrument()),
        NonEmptyText::new("热度").unwrap(),
        FiniteNumber::new(99.0).unwrap(),
        MarketRankingUnit::Score,
        IsoDate::new("2026-07-23").unwrap(),
        MarketSession::Continuous,
        NonEmptyText::new("A-share-equities").unwrap(),
        PositiveU32::new(1).unwrap(),
        PositiveU32::new(1).unwrap(),
        0,
        dated_evidence(ProviderId::Eastmoney, "ranking"),
    )
    .unwrap();

    assert_eq!(rank.provider_id(), ProviderId::Eastmoney);
    assert_eq!(rank.evidence_batch_id(), "rank");
    assert_eq!(
        rank.quote_evidence.as_ref().unwrap().provider(),
        ProviderId::Tencent
    );
    let legacy_json = serde_json::to_string(&rank)
        .unwrap()
        .replace(",\"concepts\":[]", "");
    let restored: PopularityRank = serde_json::from_str(&legacy_json).unwrap();
    assert!(restored.concepts.is_empty());
    assert_eq!(ranking.provider_id(), ProviderId::Eastmoney);
    assert_eq!(ranking.evidence_batch_id(), "ranking");
    assert_eq!(hit.provider_id(), ProviderId::Eastmoney);
    assert_eq!(hit.evidence_batch_id(), "concept");
}

#[test]
fn signal_request_round_trip_preserves_optional_trading_date() {
    let request = InstrumentSignalRequest::new(instrument(), PositiveU32::new(100).unwrap())
        .unwrap()
        .with_trading_date(IsoDate::new("2026-07-23").unwrap());
    assert_eq!(request.instrument(), &instrument());
    assert_eq!(request.trading_date().unwrap().as_str(), "2026-07-23");
    assert_eq!(request.limit().get(), 100);
    assert_eq!(
        serde_json::from_value::<InstrumentSignalRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );
    assert!(InstrumentSignalRequest::new(instrument(), PositiveU32::new(10_001).unwrap()).is_err());
}

fn disclosure_entry() -> DragonTigerEntry {
    DragonTigerEntry::new(
        NonEmptyText::new("600396:2026-07-23:1001").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        Some(NonEmptyText::new("日涨幅偏离值达到7%").unwrap()),
        Some(Money::new(100.0).unwrap()),
        Some(Money::new(40.0).unwrap()),
        Some(Money::new(60.0).unwrap()),
        None,
        dated_evidence(ProviderId::Eastmoney, "market-lhb"),
    )
    .unwrap()
}

fn disclosure_seats(side: DragonTigerSide, count: u32) -> Vec<DragonTigerSeat> {
    (1..=count)
        .map(|rank| {
            let amount = Money::new(f64::from(rank) * 10.0).unwrap();
            let (buy, sell) = match side {
                DragonTigerSide::Buy => (Some(amount), None),
                DragonTigerSide::Sell => (None, Some(amount)),
            };
            DragonTigerSeat::new(
                NonEmptyText::new("600396:2026-07-23:1001").unwrap(),
                instrument(),
                IsoDate::new("2026-07-23").unwrap(),
                side,
                PositiveU32::new(rank).unwrap(),
                NonEmptyText::new(format!("seat-{side:?}-{rank}")).unwrap(),
                amount,
                buy,
                sell,
                None,
                dated_evidence(ProviderId::Eastmoney, "market-lhb"),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn market_dragon_tiger_disclosure_requires_exact_buy_five_sell_five() {
    let request = MarketDragonTigerRequest::new(
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(5).unwrap(),
    )
    .unwrap();
    assert_eq!(request.trading_date().as_str(), "2026-07-23");
    assert_eq!(request.limit().get(), 5);
    assert!(MarketDragonTigerRequest::new(
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(101).unwrap()
    )
    .is_err());

    let mut complete = disclosure_seats(DragonTigerSide::Buy, 5);
    complete.extend(disclosure_seats(DragonTigerSide::Sell, 5));
    let disclosure = DragonTigerDisclosure::new(disclosure_entry(), complete.clone()).unwrap();
    assert_eq!(
        disclosure.entry().entry_id().as_str(),
        "600396:2026-07-23:1001"
    );
    assert_eq!(disclosure.seats().len(), 10);
    assert_eq!(disclosure.provider_id(), ProviderId::Eastmoney);
    assert_eq!(disclosure.evidence_batch_id(), "market-lhb");

    let mut mismatched = complete.clone();
    let mut mismatched_seat = serde_json::to_value(&mismatched[0]).unwrap();
    mismatched_seat["evidence"]["observed_at"] = serde_json::json!("different-observation");
    mismatched[0] = serde_json::from_value(mismatched_seat).unwrap();
    assert!(DragonTigerDisclosure::new(disclosure_entry(), mismatched).is_err());

    complete.pop();
    assert!(DragonTigerDisclosure::new(disclosure_entry(), complete).is_err());
}

#[test]
fn dragon_tiger_entry_rejects_invalid_turnover_and_evidence_dates() {
    let make = |turnover_rate, evidence| {
        DragonTigerEntry::new(
            NonEmptyText::new("entry-validation").unwrap(),
            instrument(),
            IsoDate::new("2026-07-23").unwrap(),
            None,
            None,
            None,
            None,
            turnover_rate,
            evidence,
        )
    };

    assert!(make(
        Some(Ratio::new(0.5, RatioUnit::Decimal).unwrap()),
        dated_evidence(ProviderId::Eastmoney, "entry-validation")
    )
    .is_err());
    assert!(make(
        Some(Ratio::new(-0.5, RatioUnit::Percent).unwrap()),
        dated_evidence(ProviderId::Eastmoney, "entry-validation")
    )
    .is_err());
    assert!(DragonTigerEntry::new(
        NonEmptyText::new("entry-negative").unwrap(),
        instrument(),
        IsoDate::new("2026-07-23").unwrap(),
        None,
        Some(Money::new(-1.0).unwrap()),
        None,
        None,
        None,
        dated_evidence(ProviderId::Eastmoney, "entry-negative")
    )
    .is_err());
    assert!(make(None, evidence(ProviderId::Eastmoney, "entry-validation")).is_err());
    assert!(make(
        None,
        evidence(ProviderId::Eastmoney, "entry-validation")
            .with_source_at("2026-07-22")
            .unwrap()
    )
    .is_err());
    assert!(make(
        None,
        evidence(ProviderId::Eastmoney, "entry-validation")
            .with_source_at("2026-07-23X")
            .unwrap()
    )
    .is_err());

    let named = make(
        Some(Ratio::new(0.5, RatioUnit::Percent).unwrap()),
        evidence(ProviderId::Eastmoney, "entry-validation")
            .with_source_at("2026-07-23 16:00:00")
            .unwrap(),
    )
    .unwrap()
    .with_instrument_name(NonEmptyText::new("华电辽能").unwrap());
    assert_eq!(named.instrument_name().unwrap().as_str(), "华电辽能");
    assert_eq!(named.turnover_rate().unwrap().get(), 0.5);
    assert!(named.reason().is_none());
    assert!(named.net_amount().is_none());
}

#[test]
fn dragon_tiger_disclosure_rejects_identity_and_duplicate_rank_bypasses() {
    let mut complete = disclosure_seats(DragonTigerSide::Buy, 5);
    complete.extend(disclosure_seats(DragonTigerSide::Sell, 5));

    let mut wrong_identity = complete.clone();
    let mismatched_instrument =
        InstrumentId::new(Exchange::Shanghai, "600703", AssetClass::Equity).unwrap();
    wrong_identity[0] = DragonTigerSeat::new(
        wrong_identity[0].entry_id().clone(),
        mismatched_instrument,
        wrong_identity[0].trading_date().clone(),
        wrong_identity[0].side(),
        wrong_identity[0].rank(),
        wrong_identity[0].seat_name().clone(),
        wrong_identity[0].amount(),
        wrong_identity[0].buy_amount(),
        wrong_identity[0].sell_amount(),
        wrong_identity[0].net_amount(),
        wrong_identity[0].evidence().clone(),
    )
    .unwrap();
    assert!(DragonTigerDisclosure::new(disclosure_entry(), wrong_identity).is_err());

    let mut duplicate_rank = complete.clone();
    duplicate_rank[9] = duplicate_rank[8].clone();
    assert!(DragonTigerDisclosure::new(disclosure_entry(), duplicate_rank).is_err());

    let valid = DragonTigerDisclosure::new(disclosure_entry(), complete).unwrap();
    let restored: DragonTigerDisclosure =
        serde_json::from_value(serde_json::to_value(&valid).unwrap()).unwrap();
    assert_eq!(restored, valid);
}

#[test]
fn market_dragon_tiger_request_checked_deserialization_revalidates_limit() {
    let request: MarketDragonTigerRequest =
        serde_json::from_str(r#"{"trading_date":"2026-07-23","limit":100}"#).unwrap();
    assert_eq!(request.trading_date().as_str(), "2026-07-23");
    assert_eq!(request.limit().get(), 100);
    assert!(serde_json::from_str::<MarketDragonTigerRequest>(
        r#"{"trading_date":"2026-07-23","limit":101}"#
    )
    .is_err());
}
