use magic_market_core::{
    AssetClass, BlockTrade, Board, BoardCategory, BoardFlow, CapitalCapabilities, DividendPlan,
    Exchange, FiniteNumber, FlowInterval, FlowScope, FundFlowPoint, FundFlowRequest, HolderCount,
    InstrumentDateRangeRequest, InstrumentId, IsoDate, LockupEvent, MarginBalance, Money,
    NonEmptyText, NorthboundChannel, NorthboundDailyRequest, NorthboundDailyStat,
    NorthboundQuotaBalance, NorthboundTopTurnover, PositiveU32, PostCloseFlow,
    PostCloseFlowRequest, Price, PriceLimitRule, ProviderId, Quantity, Ratio, RatioUnit,
    SourceEvidence, SourcedRecord,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

fn evidence() -> SourceEvidence {
    SourceEvidence::new(ProviderId::Eastmoney, "observed", "batch").unwrap()
}

fn post_close_evidence() -> SourceEvidence {
    SourceEvidence::new(ProviderId::Eastmoney, "2026-07-23T15:35:00+08:00", "batch").unwrap()
}

fn northbound_top_ten(channel: NorthboundChannel) -> Vec<NorthboundTopTurnover> {
    let (exchange, base) = match channel {
        NorthboundChannel::Shanghai => (Exchange::Shanghai, 600_000),
        NorthboundChannel::Shenzhen => (Exchange::Shenzhen, 1),
    };
    (1..=10)
        .map(|rank| {
            NorthboundTopTurnover::new(
                PositiveU32::new(rank).unwrap(),
                InstrumentId::new(exchange, format!("{:06}", base + rank), AssetClass::Equity)
                    .unwrap(),
                NonEmptyText::new(format!("stock-{rank}")).unwrap(),
                Money::new(f64::from(rank) * 1_000_000.0).unwrap(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn flow_series_distinguishes_missing_from_source_zero() {
    let point = FundFlowPoint {
        scope: FlowScope::Instrument(instrument()),
        interval: FlowInterval::Day1,
        period_at: NonEmptyText::new("2026-07-23").unwrap(),
        main_net: Some(Money::new(0.0).unwrap()),
        main_ratio: None,
        super_large_net: None,
        large_net: None,
        medium_net: None,
        small_net: None,
        evidence: evidence(),
    };
    let board = BoardFlow {
        board_code: NonEmptyText::new("BK0001").unwrap(),
        board_name: NonEmptyText::new("电力").unwrap(),
        category: BoardCategory::Industry,
        interval: FlowInterval::Day5,
        rank: PositiveU32::new(1).unwrap(),
        return_ratio: None,
        main_net: None,
        super_large_net: None,
        large_net: None,
        medium_net: None,
        small_net: None,
        leader_instrument: None,
        leader_name: None,
        leader_return_ratio: None,
        evidence: evidence(),
    };

    assert_eq!(point.main_net.unwrap().get(), 0.0);
    assert!(point.super_large_net.is_none());
    assert_eq!(board.provider_id(), ProviderId::Eastmoney);
}

#[test]
fn capital_records_round_trip_without_defaulting_absence() {
    let records = (
        MarginBalance {
            instrument: instrument(),
            trading_date: IsoDate::new("2026-07-23").unwrap(),
            financing_balance: None,
            financing_buy: Some(Money::new(0.0).unwrap()),
            financing_repayment: None,
            securities_lending_balance: None,
            securities_lending_sell: None,
            securities_lending_repayment: None,
            total_balance: None,
            evidence: evidence(),
        },
        BlockTrade {
            instrument: instrument(),
            trading_date: IsoDate::new("2026-07-23").unwrap(),
            traded_at: None,
            price: Price::new(4.0).unwrap(),
            close_price: None,
            premium_ratio: None,
            volume: Quantity::new(100.0).unwrap(),
            amount: Some(Money::new(400.0).unwrap()),
            buyer: None,
            seller: None,
            evidence: evidence(),
        },
        HolderCount {
            instrument: instrument(),
            report_date: IsoDate::new("2026-06-30").unwrap(),
            holders: Quantity::new(10_000.0).unwrap(),
            holder_change: None,
            change_ratio: Some(Ratio::new(-2.0, RatioUnit::Percent).unwrap()),
            average_shares_per_holder: None,
            evidence: evidence(),
        },
        LockupEvent {
            instrument: instrument(),
            listing_date: IsoDate::new("2026-08-01").unwrap(),
            share_type: NonEmptyText::new("首发原股东限售股份").unwrap(),
            shares: Quantity::new(1_000.0).unwrap(),
            able_shares: None,
            free_float_ratio: None,
            market_value: None,
            evidence: evidence(),
        },
        DividendPlan {
            instrument: instrument(),
            report_date: IsoDate::new("2025-12-31").unwrap(),
            ex_dividend_date: None,
            state: NonEmptyText::new("实施").unwrap(),
            cash_per_ten: Some(FiniteNumber::new(1.0).unwrap()),
            bonus_per_ten: None,
            transfer_per_ten: None,
            allotment_per_ten: None,
            reduction_ratio: None,
            evidence: evidence(),
        },
    );

    let json = serde_json::to_string(&records).unwrap();
    let decoded: (
        MarginBalance,
        BlockTrade,
        HolderCount,
        LockupEvent,
        DividendPlan,
    ) = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, records);
}

#[test]
fn range_requests_are_bounded_and_calendar_checked() {
    let request = InstrumentDateRangeRequest::new(instrument(), PositiveU32::new(100).unwrap())
        .unwrap()
        .with_range(
            IsoDate::new("2026-01-01").unwrap(),
            IsoDate::new("2026-07-23").unwrap(),
        )
        .unwrap();
    assert_eq!(request.instrument(), &instrument());
    assert_eq!(request.start().unwrap().as_str(), "2026-01-01");
    assert_eq!(request.end().unwrap().as_str(), "2026-07-23");
    assert_eq!(request.limit().get(), 100);
    assert_eq!(
        serde_json::from_str::<InstrumentDateRangeRequest>(
            &serde_json::to_string(&request).unwrap()
        )
        .unwrap(),
        request
    );
    let unbounded =
        InstrumentDateRangeRequest::new(instrument(), PositiveU32::new(1).unwrap()).unwrap();
    assert!(serde_json::from_str::<InstrumentDateRangeRequest>(
        &serde_json::to_string(&unbounded).unwrap()
    )
    .is_ok());
    let mut one_sided = serde_json::to_value(&unbounded).unwrap();
    one_sided["start"] = serde_json::json!("2026-01-01");
    assert!(serde_json::from_value::<InstrumentDateRangeRequest>(one_sided).is_err());
    assert!(
        InstrumentDateRangeRequest::new(instrument(), PositiveU32::new(10_001).unwrap()).is_err()
    );
    assert!(
        InstrumentDateRangeRequest::new(instrument(), PositiveU32::new(1).unwrap())
            .unwrap()
            .with_range(
                IsoDate::new("2026-07-23").unwrap(),
                IsoDate::new("2026-01-01").unwrap()
            )
            .is_err()
    );
    assert!(FundFlowRequest::new(
        FlowScope::Instrument(instrument()),
        FlowInterval::Day120,
        PositiveU32::new(10_001).unwrap()
    )
    .is_err());
    let flow = FundFlowRequest::new(
        FlowScope::Instrument(instrument()),
        FlowInterval::Day5,
        PositiveU32::new(5).unwrap(),
    )
    .unwrap();
    assert!(matches!(flow.scope(), FlowScope::Instrument(_)));
    assert_eq!(flow.interval(), FlowInterval::Day5);
    assert_eq!(flow.limit().get(), 5);
    assert_eq!(
        serde_json::from_str::<FundFlowRequest>(&serde_json::to_string(&flow).unwrap()).unwrap(),
        flow
    );
    assert!(serde_json::from_str::<FundFlowRequest>(
        r#"{"scope":{"Instrument":{"exchange":"Shanghai","code":"600396","asset_class":"Equity"}},"interval":"Day120","limit":10001}"#
    )
    .is_err());
}

#[test]
fn capital_requests_round_trip_only_complete_validated_filters() {
    let current =
        InstrumentDateRangeRequest::new(instrument(), PositiveU32::new(100).unwrap()).unwrap();
    let decoded: InstrumentDateRangeRequest =
        serde_json::from_value(serde_json::to_value(&current).unwrap()).unwrap();
    assert_eq!(decoded.instrument(), &instrument());
    assert!(decoded.start().is_none());
    assert!(decoded.end().is_none());

    let ranged = current
        .with_range(
            IsoDate::new("2026-01-01").unwrap(),
            IsoDate::new("2026-07-23").unwrap(),
        )
        .unwrap();
    let decoded: InstrumentDateRangeRequest =
        serde_json::from_value(serde_json::to_value(&ranged).unwrap()).unwrap();
    assert_eq!(decoded.start().unwrap().as_str(), "2026-01-01");
    assert_eq!(decoded.end().unwrap().as_str(), "2026-07-23");
    assert_eq!(decoded.limit().get(), 100);

    let partial = serde_json::json!({
        "instrument": {
            "exchange": "Shanghai",
            "code": "600396",
            "asset_class": "Equity"
        },
        "start": "2026-01-01",
        "end": null,
        "limit": 100
    });
    assert!(serde_json::from_value::<InstrumentDateRangeRequest>(partial).is_err());

    let flow = FundFlowRequest::new(
        FlowScope::Instrument(instrument()),
        FlowInterval::Day5,
        PositiveU32::new(20).unwrap(),
    )
    .unwrap();
    let decoded: FundFlowRequest =
        serde_json::from_value(serde_json::to_value(&flow).unwrap()).unwrap();
    assert_eq!(decoded.scope(), &FlowScope::Instrument(instrument()));
    assert_eq!(decoded.interval(), FlowInterval::Day5);
    assert_eq!(decoded.limit().get(), 20);

    let post_close = PostCloseFlowRequest::new(
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(10).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_value::<PostCloseFlowRequest>(serde_json::to_value(&post_close).unwrap())
            .unwrap(),
        post_close
    );
}

#[test]
fn post_close_flow_preserves_rank_and_source_backed_limit_metadata() {
    let source_evidence = post_close_evidence()
        .with_source_at("2026-07-23 15:00:00")
        .unwrap();
    let record = PostCloseFlow::new(
        instrument(),
        Some(NonEmptyText::new("华电辽能").unwrap()),
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        Some(Board::Main),
        Some(
            PriceLimitRule::new(
                Some(Ratio::new(10.0, RatioUnit::Percent).unwrap()),
                Some("source-rule".into()),
            )
            .unwrap(),
        ),
        source_evidence,
    )
    .unwrap();
    let request = PostCloseFlowRequest::new(
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(10).unwrap(),
    )
    .unwrap();
    assert_eq!(request.limit().get(), 10);
    assert_eq!(request.trading_date().as_str(), "2026-07-23");
    assert_eq!(record.instrument(), &instrument());
    assert_eq!(record.name().unwrap().as_str(), "华电辽能");
    assert_eq!(record.trading_date().as_str(), "2026-07-23");
    assert_eq!(record.rank().get(), 1);
    assert_eq!(record.close().get(), 4.36);
    assert_eq!(record.change().get(), 2.1);
    assert_eq!(record.main_net().get(), 12_000_000.0);
    assert_eq!(record.board(), Some(Board::Main));
    assert_eq!(
        record.price_limit_rule().unwrap().version(),
        Some("source-rule")
    );
    assert_eq!(record.evidence().provider(), ProviderId::Eastmoney);
    assert_eq!(
        serde_json::from_str::<PostCloseFlowRequest>(&serde_json::to_string(&request).unwrap())
            .unwrap(),
        request
    );
    assert_eq!(
        serde_json::from_str::<PostCloseFlow>(&serde_json::to_string(&record).unwrap()).unwrap(),
        record
    );
    assert_eq!(record.instrument(), &instrument());
    assert_eq!(record.name().unwrap().as_str(), "华电辽能");
    assert_eq!(record.trading_date().as_str(), "2026-07-23");
    assert_eq!(record.rank().get(), 1);
    assert_eq!(record.close().get(), 4.36);
    assert_eq!(record.change().get(), 2.1);
    assert_eq!(record.main_net().get(), 12_000_000.0);
    assert_eq!(record.main_net_ratio().get(), 3.2);
    assert_eq!(record.board(), Some(Board::Main));
    assert!(record.price_limit_rule().is_some());
    assert_eq!(record.evidence().provider(), ProviderId::Eastmoney);
    let mut missing_source_time = serde_json::to_value(&record).unwrap();
    missing_source_time["evidence"]["source_at"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<PostCloseFlow>(missing_source_time).is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        post_close_evidence(),
    )
    .is_err());
    for observed_at in [
        "2026-07-23T15:35+08:00",
        "2026-07-23Taa:35:00+08:00",
        "2026-07-23T25:00:00+08:00",
    ] {
        assert!(PostCloseFlow::new(
            instrument(),
            None,
            IsoDate::new("2026-07-23").unwrap(),
            PositiveU32::new(1).unwrap(),
            Price::new(4.36).unwrap(),
            Ratio::new(2.1, RatioUnit::Percent).unwrap(),
            Money::new(12_000_000.0).unwrap(),
            Ratio::new(3.2, RatioUnit::Percent).unwrap(),
            None,
            None,
            SourceEvidence::new(ProviderId::Eastmoney, observed_at, "batch")
                .unwrap()
                .with_source_at("2026-07-23 15:00:00")
                .unwrap(),
        )
        .is_err());
    }
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        post_close_evidence()
            .with_source_at("2026-07-23X15:00:00")
            .unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        SourceEvidence::new(ProviderId::Eastmoney, "2026-07-22T15:35:00+08:00", "batch",)
            .unwrap()
            .with_source_at("2026-07-23 15:00:00")
            .unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        evidence().with_source_at("short").unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        evidence().with_source_at("2026-07-23x15:35:00").unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        evidence().with_source_at("2026-07-22 15:35:00").unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        SourceEvidence::new(ProviderId::Eastmoney, "2026-07-23T15:34:59+08:00", "batch",)
            .unwrap()
            .with_source_at("2026-07-23 15:00:00")
            .unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        post_close_evidence()
            .with_source_at("2026-07-22 15:00:00")
            .unwrap(),
    )
    .is_err());
    assert!(PostCloseFlow::new(
        instrument(),
        None,
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(1).unwrap(),
        Price::new(4.36).unwrap(),
        Ratio::new(2.1, RatioUnit::Percent).unwrap(),
        Money::new(12_000_000.0).unwrap(),
        Ratio::new(3.2, RatioUnit::Percent).unwrap(),
        None,
        None,
        post_close_evidence().with_source_at("not-a-date").unwrap(),
    )
    .is_err());
    assert!(PostCloseFlowRequest::new(
        IsoDate::new("2026-07-23").unwrap(),
        PositiveU32::new(101).unwrap()
    )
    .is_err());
    let legacy_capabilities: CapitalCapabilities = serde_json::from_str(
        r#"{"fund_flow_series":true,"board_flow":true,"margin":true,"block_trades":true,"holder_count":true,"lockups":true,"dividends":true}"#,
    )
    .unwrap();
    assert!(!legacy_capabilities.post_close_flow);
}

#[test]
fn northbound_daily_statistics_are_lossless_checked_and_serde_safe() {
    let trading_date = IsoDate::new("2026-07-22").unwrap();
    let source_evidence = SourceEvidence::new(ProviderId::Hkex, "observed", "hkex-20260722")
        .unwrap()
        .with_source_at("2026-07-22")
        .unwrap();
    let record = NorthboundDailyStat::new(
        trading_date.clone(),
        NorthboundChannel::Shanghai,
        Money::new(174_764_630_000.0).unwrap(),
        Quantity::new(7_939_144.0).unwrap(),
        NorthboundQuotaBalance::Unavailable,
        Money::new(4_839_470_000.0).unwrap(),
        northbound_top_ten(NorthboundChannel::Shanghai),
        source_evidence,
    )
    .unwrap();
    let request = NorthboundDailyRequest::new(trading_date.clone(), NorthboundChannel::Shanghai);

    assert_eq!(record.trading_date(), &trading_date);
    assert_eq!(record.channel(), NorthboundChannel::Shanghai);
    assert_eq!(record.total_turnover().get(), 174_764_630_000.0);
    assert_eq!(record.total_trade_count().get(), 7_939_144.0);
    assert_eq!(record.quota_balance(), NorthboundQuotaBalance::Unavailable);
    assert_eq!(record.etf_turnover().get(), 4_839_470_000.0);
    assert_eq!(record.top_turnover().len(), 10);
    assert_eq!(record.top_turnover()[0].rank().get(), 1);
    assert_eq!(record.top_turnover()[0].instrument().code(), "600001");
    assert_eq!(record.top_turnover()[0].name().as_str(), "stock-1");
    assert_eq!(record.top_turnover()[0].total_turnover().get(), 1_000_000.0);
    assert_eq!(record.evidence().batch_id(), "hkex-20260722");
    assert_eq!(record.provider_id(), ProviderId::Hkex);
    assert_eq!(request.trading_date(), &trading_date);
    assert_eq!(request.channel(), NorthboundChannel::Shanghai);
    assert_eq!(
        serde_json::from_str::<NorthboundDailyStat>(&serde_json::to_string(&record).unwrap())
            .unwrap(),
        record
    );
    assert_eq!(
        serde_json::from_str::<NorthboundDailyRequest>(&serde_json::to_string(&request).unwrap())
            .unwrap(),
        request
    );
    assert!(serde_json::from_str::<NorthboundQuotaBalance>(r#"{"Amount":-1.0}"#).is_err());
    assert_eq!(
        serde_json::from_str::<NorthboundQuotaBalance>(r#"{"Amount":1.0}"#).unwrap(),
        NorthboundQuotaBalance::Amount(Money::new(1.0).unwrap())
    );
    assert_eq!(
        serde_json::from_str::<NorthboundQuotaBalance>(r#""Unavailable""#).unwrap(),
        NorthboundQuotaBalance::Unavailable
    );

    assert!(NorthboundTopTurnover::new(
        PositiveU32::new(11).unwrap(),
        instrument(),
        NonEmptyText::new("too-low").unwrap(),
        Money::new(1.0).unwrap(),
    )
    .is_err());
    assert!(NorthboundTopTurnover::new(
        PositiveU32::new(1).unwrap(),
        InstrumentId::new(Exchange::Shanghai, "110001", AssetClass::Bond).unwrap(),
        NonEmptyText::new("bond").unwrap(),
        Money::new(1.0).unwrap(),
    )
    .is_err());
    assert!(NorthboundTopTurnover::new(
        PositiveU32::new(1).unwrap(),
        instrument(),
        NonEmptyText::new("negative").unwrap(),
        Money::new(-1.0).unwrap(),
    )
    .is_err());

    let mut duplicate_rank = northbound_top_ten(NorthboundChannel::Shanghai);
    duplicate_rank[9] = duplicate_rank[0].clone();
    assert!(NorthboundDailyStat::new(
        trading_date.clone(),
        NorthboundChannel::Shanghai,
        Money::new(1.0).unwrap(),
        Quantity::new(1.0).unwrap(),
        NorthboundQuotaBalance::Amount(Money::new(1.0).unwrap()),
        Money::new(0.0).unwrap(),
        duplicate_rank,
        SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
            .unwrap()
            .with_source_at("2026-07-22")
            .unwrap(),
    )
    .is_err());
    let mut duplicate_instrument = northbound_top_ten(NorthboundChannel::Shanghai);
    duplicate_instrument[9] = NorthboundTopTurnover::new(
        PositiveU32::new(10).unwrap(),
        duplicate_instrument[0].instrument().clone(),
        NonEmptyText::new("duplicate").unwrap(),
        Money::new(10.0).unwrap(),
    )
    .unwrap();
    assert!(NorthboundDailyStat::new(
        trading_date.clone(),
        NorthboundChannel::Shanghai,
        Money::new(1.0).unwrap(),
        Quantity::new(1.0).unwrap(),
        NorthboundQuotaBalance::Unavailable,
        Money::new(0.0).unwrap(),
        duplicate_instrument,
        SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
            .unwrap()
            .with_source_at("2026-07-22")
            .unwrap(),
    )
    .is_err());
    for (turnover, etf, quota) in [
        (-1.0, 0.0, NorthboundQuotaBalance::Unavailable),
        (1.0, -1.0, NorthboundQuotaBalance::Unavailable),
        (
            1.0,
            0.0,
            NorthboundQuotaBalance::Amount(Money::new(-1.0).unwrap()),
        ),
    ] {
        assert!(NorthboundDailyStat::new(
            trading_date.clone(),
            NorthboundChannel::Shanghai,
            Money::new(turnover).unwrap(),
            Quantity::new(1.0).unwrap(),
            quota,
            Money::new(etf).unwrap(),
            northbound_top_ten(NorthboundChannel::Shanghai),
            SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
                .unwrap()
                .with_source_at("2026-07-22")
                .unwrap(),
        )
        .is_err());
    }
    assert!(NorthboundDailyStat::new(
        trading_date.clone(),
        NorthboundChannel::Shanghai,
        Money::new(1.0).unwrap(),
        Quantity::new(1.0).unwrap(),
        NorthboundQuotaBalance::Unavailable,
        Money::new(0.0).unwrap(),
        Vec::new(),
        SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
            .unwrap()
            .with_source_at("2026-07-22")
            .unwrap(),
    )
    .is_err());
    for source_at in [None, Some("x"), Some("2026-07-22X"), Some("not-a-date")] {
        let base = SourceEvidence::new(ProviderId::Hkex, "observed", "batch").unwrap();
        let source = match source_at {
            Some(value) => base.with_source_at(value).unwrap(),
            None => base,
        };
        assert!(NorthboundDailyStat::new(
            trading_date.clone(),
            NorthboundChannel::Shanghai,
            Money::new(1.0).unwrap(),
            Quantity::new(1.0).unwrap(),
            NorthboundQuotaBalance::Unavailable,
            Money::new(0.0).unwrap(),
            northbound_top_ten(NorthboundChannel::Shanghai),
            source,
        )
        .is_err());
    }
    assert!(NorthboundDailyStat::new(
        trading_date.clone(),
        NorthboundChannel::Shanghai,
        Money::new(1.0).unwrap(),
        Quantity::new(1.5).unwrap(),
        NorthboundQuotaBalance::Amount(Money::new(-1.0).unwrap()),
        Money::new(0.0).unwrap(),
        northbound_top_ten(NorthboundChannel::Shanghai),
        SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
            .unwrap()
            .with_source_at("2026-07-22")
            .unwrap(),
    )
    .is_err());
    assert!(NorthboundDailyStat::new(
        trading_date,
        NorthboundChannel::Shanghai,
        Money::new(1.0).unwrap(),
        Quantity::new(1.0).unwrap(),
        NorthboundQuotaBalance::Unavailable,
        Money::new(0.0).unwrap(),
        northbound_top_ten(NorthboundChannel::Shenzhen),
        SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
            .unwrap()
            .with_source_at("2026-07-21")
            .unwrap(),
    )
    .is_err());

    let legacy_capabilities: CapitalCapabilities = serde_json::from_str(
        r#"{"fund_flow_series":true,"board_flow":true,"margin":true,"block_trades":true,"holder_count":true,"lockups":true,"dividends":true}"#,
    )
    .unwrap();
    assert!(!legacy_capabilities.northbound_daily_statistics);
}

#[test]
fn northbound_domain_rejects_each_invalid_value_and_evidence_shape() {
    let trading_date = IsoDate::new("2026-07-22").unwrap();
    let valid_evidence = SourceEvidence::new(ProviderId::Hkex, "observed", "batch")
        .unwrap()
        .with_source_at("2026-07-22")
        .unwrap();

    let quota: NorthboundQuotaBalance = serde_json::from_str(r#"{"Amount":1000000.0}"#).unwrap();
    assert_eq!(
        quota,
        NorthboundQuotaBalance::Amount(Money::new(1_000_000.0).unwrap())
    );

    assert!(NorthboundTopTurnover::new(
        PositiveU32::new(11).unwrap(),
        instrument(),
        NonEmptyText::new("rank-too-large").unwrap(),
        Money::new(1.0).unwrap(),
    )
    .is_err());
    assert!(NorthboundTopTurnover::new(
        PositiveU32::new(1).unwrap(),
        InstrumentId::new(Exchange::Shanghai, "000001", AssetClass::Index).unwrap(),
        NonEmptyText::new("not-equity-or-fund").unwrap(),
        Money::new(1.0).unwrap(),
    )
    .is_err());
    assert!(NorthboundTopTurnover::new(
        PositiveU32::new(1).unwrap(),
        instrument(),
        NonEmptyText::new("negative-turnover").unwrap(),
        Money::new(-1.0).unwrap(),
    )
    .is_err());

    let first = northbound_top_ten(NorthboundChannel::Shanghai)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(first.name().as_str(), "stock-1");

    let make_record = |total_turnover: f64,
                       quota_balance: NorthboundQuotaBalance,
                       etf_turnover: f64,
                       top_turnover: Vec<NorthboundTopTurnover>,
                       evidence: SourceEvidence| {
        NorthboundDailyStat::new(
            trading_date.clone(),
            NorthboundChannel::Shanghai,
            Money::new(total_turnover).unwrap(),
            Quantity::new(1.0).unwrap(),
            quota_balance,
            Money::new(etf_turnover).unwrap(),
            top_turnover,
            evidence,
        )
    };

    assert!(make_record(
        -1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        valid_evidence.clone(),
    )
    .is_err());
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        -1.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        valid_evidence.clone(),
    )
    .is_err());
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Amount(Money::new(-1.0).unwrap()),
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        valid_evidence.clone(),
    )
    .is_err());
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai)
            .into_iter()
            .take(9)
            .collect(),
        valid_evidence.clone(),
    )
    .is_err());

    let mut duplicate_instrument = northbound_top_ten(NorthboundChannel::Shanghai);
    duplicate_instrument[9] = NorthboundTopTurnover::new(
        PositiveU32::new(10).unwrap(),
        duplicate_instrument[0].instrument().clone(),
        NonEmptyText::new("duplicate-instrument").unwrap(),
        Money::new(1.0).unwrap(),
    )
    .unwrap();
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        duplicate_instrument,
        valid_evidence.clone(),
    )
    .is_err());

    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        SourceEvidence::new(ProviderId::Hkex, "observed", "missing-source-at").unwrap(),
    )
    .is_err());
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        SourceEvidence::new(ProviderId::Hkex, "observed", "short-source-at")
            .unwrap()
            .with_source_at("short")
            .unwrap(),
    )
    .is_err());
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        SourceEvidence::new(ProviderId::Hkex, "observed", "bad-separator")
            .unwrap()
            .with_source_at("2026-07-22x15:00:00")
            .unwrap(),
    )
    .is_err());
    assert!(make_record(
        1.0,
        NorthboundQuotaBalance::Unavailable,
        0.0,
        northbound_top_ten(NorthboundChannel::Shanghai),
        SourceEvidence::new(ProviderId::Hkex, "observed", "wrong-date")
            .unwrap()
            .with_source_at("2026-07-21")
            .unwrap(),
    )
    .is_err());
}
