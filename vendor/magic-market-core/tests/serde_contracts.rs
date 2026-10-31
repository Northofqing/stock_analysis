use magic_market_core::{
    Adjustment, AssetClass, AuctionSnapshot, Bar, BarInterval, BarsRequest, Board, BookLevel,
    Capabilities, DataBatch, DataStatus, Exchange, InstrumentId, MinuteDataRequest, MinutePoint,
    Money, MoneyFlow, OrderBook, Price, PriceLimitRule, Provenance, ProviderId, Quantity, Quote,
    Ratio, RatioUnit, SecurityMetadata, Trade, TradeSide, TradesRequest,
};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;

fn quote() -> Quote {
    Quote::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        Price::new(1_308.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        None,
        "2026-07-22T10:00:00+08:00",
        ProviderId::Tdx,
        "batch-1",
    )
    .unwrap()
}

#[test]
fn normalized_batch_round_trips_without_losing_evidence() {
    let provenance = Provenance::new("tdx", "2026-07-22T10:00:01+08:00")
        .unwrap()
        .with_source_at("2026-07-22T10:00:00+08:00")
        .unwrap()
        .with_batch_id("batch-1")
        .unwrap();
    let batch = DataBatch::strict(vec![quote()], provenance);

    let json = serde_json::to_string(&batch).unwrap();
    let decoded: DataBatch<Quote> = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.records(), batch.records());
    assert_eq!(decoded.provenance(), batch.provenance());
    assert!(decoded.quality().is_complete());
}

#[test]
fn available_records_expose_complete_source_backed_fields() {
    let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let source_at = "2026-07-22T10:00:00+08:00";
    let observed_at = "2026-07-22T10:00:01+08:00";

    let money_flow = MoneyFlow::new(
        instrument.clone(),
        Some(Money::new(500.0).unwrap()),
        Some(Money::new(100.0).unwrap()),
        Some(Money::new(200.0).unwrap()),
        Some(Money::new(-50.0).unwrap()),
        Some(Money::new(250.0).unwrap()),
        DataStatus::Available,
        Some(source_at.into()),
        observed_at,
        ProviderId::Eastmoney,
        "flow-batch",
    )
    .unwrap();
    assert_eq!(money_flow.instrument(), &instrument);
    assert_eq!(money_flow.main_net().unwrap().get(), 500.0);
    assert_eq!(money_flow.super_large_net().unwrap().get(), 100.0);
    assert_eq!(money_flow.large_net().unwrap().get(), 200.0);
    assert_eq!(money_flow.medium_net().unwrap().get(), -50.0);
    assert_eq!(money_flow.small_net().unwrap().get(), 250.0);
    assert_eq!(money_flow.status(), DataStatus::Available);
    assert_eq!(money_flow.source_at(), Some(source_at));
    assert_eq!(money_flow.observed_at(), observed_at);
    assert_eq!(money_flow.provider(), ProviderId::Eastmoney);
    assert_eq!(money_flow.batch_id(), "flow-batch");

    let level = BookLevel::new(
        Some(Price::new(1_308.0).unwrap()),
        Some(Quantity::new(10.0).unwrap()),
    )
    .unwrap();
    let order_book = OrderBook::new(
        instrument.clone(),
        [level; 5],
        [level; 5],
        Some(Quantity::new(50.0).unwrap()),
        Some(Quantity::new(50.0).unwrap()),
        DataStatus::Available,
        Some(source_at.into()),
        observed_at,
        ProviderId::Tdx,
        "book-batch",
    )
    .unwrap();
    assert_eq!(order_book.instrument(), &instrument);
    assert_eq!(order_book.bids()[0].price().unwrap().get(), 1_308.0);
    assert_eq!(order_book.asks()[0].quantity().unwrap().get(), 10.0);
    assert_eq!(order_book.total_bid_quantity().unwrap().get(), 50.0);
    assert_eq!(order_book.total_ask_quantity().unwrap().get(), 50.0);
    assert_eq!(order_book.status(), DataStatus::Available);
    assert_eq!(order_book.source_at(), Some(source_at));
    assert_eq!(order_book.observed_at(), observed_at);
    assert_eq!(order_book.provider(), ProviderId::Tdx);
    assert_eq!(order_book.batch_id(), "book-batch");

    let auction = AuctionSnapshot::new(
        instrument.clone(),
        Some("贵州茅台".into()),
        Some(Price::new(1_308.0).unwrap()),
        Some(Price::new(1_300.0).unwrap()),
        Some(Ratio::new(0.62, RatioUnit::Percent).unwrap()),
        Some(Quantity::new(100.0).unwrap()),
        Some(Money::new(130_800.0).unwrap()),
        Some(Quantity::new(20.0).unwrap()),
        Some(Quantity::new(30.0).unwrap()),
        Some(Ratio::new(1.25, RatioUnit::Decimal).unwrap()),
        DataStatus::Available,
        Some(source_at.into()),
        observed_at,
        ProviderId::Eastmoney,
        "auction-batch",
    )
    .unwrap();
    assert_eq!(auction.instrument(), &instrument);
    assert_eq!(auction.name(), Some("贵州茅台"));
    assert_eq!(auction.matched_price().unwrap().get(), 1_308.0);
    assert_eq!(auction.previous_close().unwrap().get(), 1_300.0);
    assert_eq!(auction.change_percent().unwrap().get(), 0.62);
    assert_eq!(auction.matched_quantity().unwrap().get(), 100.0);
    assert_eq!(auction.matched_amount().unwrap().get(), 130_800.0);
    assert_eq!(auction.unmatched_bid_quantity().unwrap().get(), 20.0);
    assert_eq!(auction.unmatched_ask_quantity().unwrap().get(), 30.0);
    assert_eq!(auction.volume_ratio().unwrap().get(), 1.25);
    assert_eq!(auction.status(), DataStatus::Available);
    assert_eq!(auction.source_at(), Some(source_at));
    assert_eq!(auction.observed_at(), observed_at);
    assert_eq!(auction.provider(), ProviderId::Eastmoney);
    assert_eq!(auction.batch_id(), "auction-batch");

    let trade = Trade::new(
        instrument.clone(),
        "2026-07-22 10:00:00",
        Price::new(1_308.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        Some(2),
        TradeSide::Buy,
        DataStatus::Available,
        Some(source_at.into()),
        observed_at,
        ProviderId::Tdx,
        "trade-batch",
    )
    .unwrap();
    assert_eq!(trade.instrument(), &instrument);
    assert_eq!(trade.trade_at(), "2026-07-22 10:00:00");
    assert_eq!(trade.price().get(), 1_308.0);
    assert_eq!(trade.quantity().get(), 100.0);
    assert_eq!(trade.trade_count(), Some(2));
    assert_eq!(trade.side(), TradeSide::Buy);
    assert_eq!(trade.status(), DataStatus::Available);
    assert_eq!(trade.source_at(), Some(source_at));
    assert_eq!(trade.observed_at(), observed_at);
    assert_eq!(trade.provider(), ProviderId::Tdx);
    assert_eq!(trade.batch_id(), "trade-batch");

    let metadata = SecurityMetadata::new(
        instrument.clone(),
        Some("贵州茅台".into()),
        Some(Board::Main),
        Some(false),
        Some("2001-08-27".into()),
        PriceLimitRule::new(
            Some(Ratio::new(10.0, RatioUnit::Percent).unwrap()),
            Some("main-v1".into()),
        )
        .unwrap(),
        DataStatus::Available,
        Some(source_at.into()),
        observed_at,
        ProviderId::Tdx,
        "metadata-batch",
    )
    .unwrap();
    assert_eq!(metadata.instrument(), &instrument);
    assert_eq!(metadata.name(), Some("贵州茅台"));
    assert_eq!(metadata.board(), Some(Board::Main));
    assert_eq!(metadata.is_st(), Some(false));
    assert_eq!(metadata.listed_on(), Some("2001-08-27"));
    assert_eq!(metadata.price_limit().percent().unwrap().get(), 10.0);
    assert_eq!(metadata.price_limit().version(), Some("main-v1"));
    assert_eq!(metadata.status(), DataStatus::Available);
    assert_eq!(metadata.source_at(), Some(source_at));
    assert_eq!(metadata.observed_at(), observed_at);
    assert_eq!(metadata.provider(), ProviderId::Tdx);
    assert_eq!(metadata.batch_id(), "metadata-batch");

    let minute = MinutePoint::new(
        instrument.clone(),
        "2026-07-22 10:00",
        Price::new(1_308.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        Some(Money::new(130_800.0).unwrap()),
        DataStatus::Available,
        Some(source_at.into()),
        observed_at,
        ProviderId::Tencent,
        "minute-batch",
    )
    .unwrap();
    assert_eq!(minute.instrument(), &instrument);
    assert_eq!(minute.minute_at(), "2026-07-22 10:00");
    assert_eq!(minute.price().get(), 1_308.0);
    assert_eq!(minute.cumulative_quantity().get(), 100.0);
    assert_eq!(minute.cumulative_amount().unwrap().get(), 130_800.0);
    assert_eq!(minute.status(), DataStatus::Available);
    assert_eq!(minute.source_at(), Some(source_at));
    assert_eq!(minute.observed_at(), observed_at);
    assert_eq!(minute.provider(), ProviderId::Tencent);
    assert_eq!(minute.batch_id(), "minute-batch");
}

fn assert_round_trip<T>(value: T)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
}

#[test]
fn every_normalized_record_family_round_trips_through_checked_serde() {
    let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let level = BookLevel::unavailable();
    assert_round_trip(quote());
    assert_round_trip(
        MoneyFlow::new(
            instrument.clone(),
            None,
            None,
            None,
            None,
            None,
            DataStatus::Unavailable,
            None,
            "observed",
            ProviderId::Eastmoney,
            "batch-1",
        )
        .unwrap(),
    );
    assert_round_trip(
        OrderBook::new(
            instrument.clone(),
            [level; 5],
            [level; 5],
            None,
            None,
            DataStatus::Unavailable,
            None,
            "observed",
            ProviderId::Tdx,
            "batch-1",
        )
        .unwrap(),
    );
    assert_round_trip(
        AuctionSnapshot::new(
            instrument.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            DataStatus::Unavailable,
            None,
            "observed",
            ProviderId::Eastmoney,
            "batch-1",
        )
        .unwrap(),
    );
    assert_round_trip(
        Trade::new(
            instrument.clone(),
            "10:00:00",
            Price::new(1_308.0).unwrap(),
            Quantity::new(100.0).unwrap(),
            Some(1),
            TradeSide::Unknown(5),
            DataStatus::Unavailable,
            Some("10:00:00".into()),
            "observed",
            ProviderId::Tdx,
            "batch-1",
        )
        .unwrap(),
    );
    assert_round_trip(
        SecurityMetadata::new(
            instrument.clone(),
            Some("贵州茅台".into()),
            Some(Board::Main),
            Some(false),
            None,
            PriceLimitRule::new(None, None).unwrap(),
            DataStatus::Unavailable,
            None,
            "observed",
            ProviderId::Tdx,
            "batch-1",
        )
        .unwrap(),
    );
    assert_round_trip(
        Bar::new(
            instrument,
            BarInterval::Day,
            "2026-07-22",
            "2026-07-22",
            Price::new(100.0).unwrap(),
            Price::new(101.0).unwrap(),
            Price::new(99.0).unwrap(),
            Price::new(100.0).unwrap(),
            Quantity::new(100.0).unwrap(),
            None,
            Adjustment::Unadjusted,
            ProviderId::Tdx,
            "batch-1",
        )
        .unwrap(),
    );
}

#[test]
fn deserialization_cannot_bypass_checked_value_types() {
    assert!(serde_json::from_str::<Price>("0.0").is_err());
    assert!(serde_json::from_str::<Quantity>("-1.0").is_err());
}

#[test]
fn deserialization_cannot_create_an_empty_instrument_code() {
    let json = r#"{"exchange":"Shanghai","code":"   ","asset_class":"Equity"}"#;
    assert!(serde_json::from_str::<InstrumentId>(json).is_err());
}

#[test]
fn deserialization_rejects_contradictory_quality_state() {
    let json = r#"{
        "records": [],
        "provenance": {
            "source": "fixture",
            "source_at": null,
            "fetched_at": "2026-07-22T10:00:00+08:00",
            "batch_id": null
        },
        "quality": {"complete": true, "issues": ["missing page"]}
    }"#;

    assert!(serde_json::from_str::<DataBatch<Quote>>(json).is_err());
}

#[test]
fn deserialization_rejects_empty_evidence_strings() {
    let mut json = serde_json::to_value(quote()).unwrap();
    json["observed_at"] = serde_json::json!(" ");
    assert!(serde_json::from_value::<Quote>(json).is_err());
    assert!(Quote::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        Price::new(1_308.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        None,
        " ",
        ProviderId::Tdx,
        "batch-1",
    )
    .is_err());
}

#[test]
fn quote_deserialization_rejects_available_with_missing_fields() {
    let mut json = serde_json::to_value(quote()).unwrap();
    json["status"] = serde_json::json!("Available");

    assert!(serde_json::from_value::<Quote>(json).is_err());
}

#[test]
fn order_book_deserialization_rejects_half_present_levels() {
    let json = serde_json::json!({"price": null, "quantity": 0.0});
    assert!(serde_json::from_value::<BookLevel>(json).is_err());
}

#[test]
fn normalized_records_reject_contradictory_aggregates_and_counts() {
    let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let available = BookLevel::new(
        Some(Price::new(1_308.0).unwrap()),
        Some(Quantity::new(10.0).unwrap()),
    )
    .unwrap();
    let mut partial = [BookLevel::unavailable(); 5];
    partial[0] = available;

    assert!(OrderBook::new(
        instrument.clone(),
        partial,
        [BookLevel::unavailable(); 5],
        None,
        None,
        DataStatus::Unavailable,
        None,
        "observed",
        ProviderId::Tdx,
        "batch",
    )
    .is_err());
    assert!(OrderBook::new(
        instrument.clone(),
        [available; 5],
        [available; 5],
        Some(Quantity::new(49.0).unwrap()),
        Some(Quantity::new(50.0).unwrap()),
        DataStatus::Available,
        Some("2026-07-22T10:00:00+08:00".into()),
        "observed",
        ProviderId::Tdx,
        "batch",
    )
    .is_err());
    assert!(Trade::new(
        instrument,
        "10:00:00",
        Price::new(1_308.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        Some(0),
        TradeSide::Buy,
        DataStatus::Available,
        Some("2026-07-22T10:00:00+08:00".into()),
        "observed",
        ProviderId::Tdx,
        "batch",
    )
    .is_err());
}

#[test]
fn request_round_trips_preserve_absent_and_present_filters() {
    let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();

    let bars = BarsRequest::new(instrument.clone(), BarInterval::Day, 5).unwrap();
    let decoded: BarsRequest =
        serde_json::from_value(serde_json::to_value(&bars).unwrap()).unwrap();
    assert!(decoded.start().is_none());
    let ranged = bars.with_range("2026-07-21", "2026-07-22").unwrap();
    assert_eq!(
        serde_json::from_value::<BarsRequest>(serde_json::to_value(&ranged).unwrap()).unwrap(),
        ranged
    );

    let current_minute = MinuteDataRequest::new(instrument.clone());
    let decoded: MinuteDataRequest =
        serde_json::from_value(serde_json::to_value(&current_minute).unwrap()).unwrap();
    assert_eq!(decoded.instrument(), &instrument);
    assert!(decoded.date().is_none());

    let current_trades = TradesRequest::new(instrument.clone(), 20).unwrap();
    let decoded: TradesRequest =
        serde_json::from_value(serde_json::to_value(&current_trades).unwrap()).unwrap();
    assert_eq!(decoded.instrument(), &instrument);
    assert!(decoded.date().is_none());
    let historical_trades = current_trades.with_date("2026-07-22").unwrap();
    assert_eq!(
        serde_json::from_value::<TradesRequest>(serde_json::to_value(&historical_trades).unwrap())
            .unwrap(),
        historical_trades
    );

    assert_eq!(Capabilities::default(), Capabilities::new());
}

#[test]
fn records_reject_negative_turnover_but_money_flow_allows_negative_net_values() {
    let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let negative = Money::new(-1.0).unwrap();

    assert!(Quote::new(
        instrument.clone(),
        Price::new(100.0).unwrap(),
        Quantity::new(10.0).unwrap(),
        Some(negative),
        "observed",
        ProviderId::Tdx,
        "batch-1",
    )
    .is_err());
    assert!(AuctionSnapshot::new(
        instrument.clone(),
        None,
        None,
        None,
        None,
        None,
        Some(negative),
        None,
        None,
        None,
        DataStatus::Unavailable,
        None,
        "observed",
        ProviderId::Eastmoney,
        "batch-1",
    )
    .is_err());
    assert!(Bar::new(
        instrument.clone(),
        BarInterval::Day,
        "2026-07-22",
        "2026-07-22",
        Price::new(100.0).unwrap(),
        Price::new(101.0).unwrap(),
        Price::new(99.0).unwrap(),
        Price::new(100.0).unwrap(),
        Quantity::new(10.0).unwrap(),
        Some(negative),
        Adjustment::Unadjusted,
        ProviderId::Tdx,
        "batch-1",
    )
    .is_err());
    assert!(MoneyFlow::new(
        instrument,
        Some(negative),
        None,
        None,
        None,
        None,
        DataStatus::Unavailable,
        None,
        "observed",
        ProviderId::Eastmoney,
        "batch-1",
    )
    .is_ok());
}

#[test]
fn provenance_deserialization_does_not_fabricate_a_missing_batch_id() {
    let json = r#"{
        "source": "fixture",
        "source_at": null,
        "fetched_at": "2026-07-22T10:00:00+08:00",
        "batch_id": null
    }"#;
    let provenance: Provenance = serde_json::from_str(json).unwrap();

    assert_eq!(provenance.batch_id(), None);
    assert_eq!(
        serde_json::to_value(provenance).unwrap()["batch_id"],
        serde_json::Value::Null
    );
}

#[test]
fn bar_deserialization_rechecks_ohlc_invariants() {
    let bar = Bar::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        BarInterval::Day,
        "2026-07-21",
        "2026-07-21",
        Price::new(100.0).unwrap(),
        Price::new(105.0).unwrap(),
        Price::new(95.0).unwrap(),
        Price::new(101.0).unwrap(),
        Quantity::new(100.0).unwrap(),
        None,
        Adjustment::Unadjusted,
        ProviderId::Tdx,
        "batch-1",
    )
    .unwrap();
    let mut json = serde_json::to_value(bar).unwrap();
    json["high"] = serde_json::json!(99.0);

    assert!(serde_json::from_value::<Bar>(json).is_err());
}

#[test]
fn price_limit_deserialization_rechecks_business_rules() {
    let json = serde_json::json!({
        "percent": {"value": -10.0, "unit": "Percent"},
        "version": "rule-v1"
    });
    assert!(serde_json::from_value::<PriceLimitRule>(json).is_err());
    assert!(PriceLimitRule::new(Some(Ratio::decimal(0.1).unwrap()), None).is_err());
}

#[test]
fn request_deserialization_reuses_constructor_validation() {
    let request = BarsRequest::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        BarInterval::Day,
        5,
    )
    .unwrap();
    let mut json = serde_json::to_value(request).unwrap();
    json["limit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<BarsRequest>(json).is_err());

    let json = serde_json::json!({
        "instrument": {"exchange":"Shanghai","code":"600519","asset_class":"Equity"},
        "interval": "Day",
        "start": "2026-07-21",
        "end": null,
        "limit": 5
    });
    assert!(serde_json::from_value::<BarsRequest>(json).is_err());
}

#[test]
fn security_metadata_deserialization_rejects_invalid_listing_date() {
    let metadata = SecurityMetadata::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        Some("贵州茅台".into()),
        Some(Board::Main),
        Some(false),
        None,
        PriceLimitRule::new(None, None).unwrap(),
        DataStatus::Unavailable,
        None,
        "2026-07-22T10:00:00+08:00",
        ProviderId::Tdx,
        "batch-1",
    )
    .unwrap();
    let mut json = serde_json::to_value(metadata).unwrap();
    json["listed_on"] = serde_json::json!("2026-02-30");

    assert!(serde_json::from_value::<SecurityMetadata>(json).is_err());
}
