use magic_market_core::{
    AssetClass, AsyncHistoricalBars, AsyncRealtimeQuotes, AsyncTrades, BarsRequest, Exchange,
    HistoricalBars, InstrumentId, MinuteData, SecurityMetadataProvider, Trades,
};
use magic_tdx_rs::{TdxDirectClient, TdxHqClient, TdxSmartClient};
#[test]
fn tdx_client_implements_core_bars_contract() {
    fn accepts<P: HistoricalBars<Bar = magic_market_core::Bar, Error = magic_tdx_rs::TdxError>>(
        _: &P,
    ) {
    }
    let _ = BarsRequest::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        magic_market_core::BarInterval::Day,
        1,
    )
    .unwrap();
    accepts(&TdxHqClient::new());
    accepts(&TdxSmartClient::new());
    accepts(&TdxDirectClient::new("127.0.0.1", 7709, 1.0));

    fn accepts_trades<P: Trades<Error = magic_tdx_rs::TdxError>>(_: &P) {}
    accepts_trades(&TdxHqClient::new());
    accepts_trades(&TdxSmartClient::new());
    accepts_trades(&TdxDirectClient::new("127.0.0.1", 7709, 1.0));

    fn accepts_metadata<P: SecurityMetadataProvider<Error = magic_tdx_rs::TdxError>>(_: &P) {}
    accepts_metadata(&TdxHqClient::new());
    accepts_metadata(&TdxSmartClient::new());

    fn accepts_minute<P: MinuteData<Error = magic_tdx_rs::TdxError>>(_: &P) {}
    accepts_minute(&TdxHqClient::new());
    accepts_minute(&TdxSmartClient::new());
}

#[test]
fn async_tdx_client_implements_core_contracts() {
    fn accepts_bars<
        P: AsyncHistoricalBars<Bar = magic_market_core::Bar, Error = magic_tdx_rs::TdxError>,
    >(
        _: &P,
    ) {
    }
    fn accepts_quotes<
        P: AsyncRealtimeQuotes<Quote = magic_market_core::Quote, Error = magic_tdx_rs::TdxError>,
    >(
        _: &P,
    ) {
    }
    fn accepts_trades<P: AsyncTrades<Error = magic_tdx_rs::TdxError>>(_: &P) {}
    let client = magic_tdx_rs::AsyncTdxHqClient::new();
    accepts_bars(&client);
    accepts_quotes(&client);
    accepts_trades(&client);
}

#[test]
fn batch_provenance_distinguishes_source_and_fetch_time() {
    let p = magic_market_core::Provenance::new("tdx", "1700000000")
        .unwrap()
        .with_source_at("2026-07-15")
        .unwrap();
    assert_eq!(p.source_at(), Some("2026-07-15"));
    assert_eq!(p.fetched_at(), "1700000000");
}
