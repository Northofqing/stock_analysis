use super::*;
use magic_market_core::{
    AssetClass, BarInterval, CorporateActionRequest, Exchange, InstrumentId, MinuteDataRequest,
    TradesRequest,
};
use std::cell::RefCell;
use std::collections::VecDeque;

type HistoryTransactionCall = (u8, String, u16, u16, u32);

fn instrument(exchange: Exchange, code: &str) -> InstrumentId {
    InstrumentId::new(exchange, code, AssetClass::Equity).unwrap()
}

#[derive(Default)]
struct ScriptedBlockingServiceQuery {
    count_calls: RefCell<Vec<u8>>,
    count_responses: RefCell<VecDeque<Result<u16, TdxError>>>,
    list_calls: RefCell<Vec<(u8, u16)>>,
    list_responses: RefCell<VecDeque<Result<Vec<SecurityInfo>, TdxError>>>,
    quote_calls: RefCell<Vec<Vec<(u8, String)>>>,
    quote_responses: RefCell<VecDeque<Result<Vec<SecurityQuote>, TdxError>>>,
    minute_calls: RefCell<Vec<(u8, String)>>,
    minute_responses: RefCell<VecDeque<Result<Vec<MinuteTimePrice>, TdxError>>>,
    history_minute_calls: RefCell<Vec<(u8, String, u32)>>,
    history_minute_responses: RefCell<VecDeque<Result<Vec<MinuteTimePrice>, TdxError>>>,
    transaction_calls: RefCell<Vec<(u8, String, u16, u16)>>,
    transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
    history_transaction_calls: RefCell<Vec<HistoryTransactionCall>>,
    history_transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
}

fn unconfigured<T>(operation: &str) -> Result<T, TdxError> {
    Err(TdxError::InvalidData(format!(
        "scripted {operation} response is not configured"
    )))
}

impl crate::adapter::BlockingTdxQuery for ScriptedBlockingServiceQuery {
    fn security_bars(
        &self,
        _category: u8,
        _market: u8,
        _code: &str,
        _start: u32,
        _count: u16,
        _adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        unconfigured("bars")
    }

    fn security_quotes(&self, instruments: &[(u8, &str)]) -> Result<Vec<SecurityQuote>, TdxError> {
        self.quote_calls.borrow_mut().push(
            instruments
                .iter()
                .map(|(market, code)| (*market, (*code).to_owned()))
                .collect(),
        );
        self.quote_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("quotes"))
    }

    fn minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>, TdxError> {
        self.minute_calls
            .borrow_mut()
            .push((market, code.to_owned()));
        self.minute_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("current minute"))
    }

    fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        self.history_minute_calls
            .borrow_mut()
            .push((market, code.to_owned(), date));
        self.history_minute_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("history minute"))
    }

    fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError> {
        self.transaction_calls
            .borrow_mut()
            .push((market, code.to_owned(), start, count));
        self.transaction_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("current transactions"))
    }

    fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError> {
        self.history_transaction_calls.borrow_mut().push((
            market,
            code.to_owned(),
            start,
            count,
            date,
        ));
        self.history_transaction_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("history transactions"))
    }

    fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        self.count_calls.borrow_mut().push(market);
        self.count_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("security count"))
    }

    fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        self.list_calls.borrow_mut().push((market, start));
        self.list_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("security list"))
    }
}

fn security(code: usize) -> SecurityInfo {
    SecurityInfo {
        code: format!("{code:06}"),
        volunit: 100,
        decimal_point: 2,
        name: format!("证券{code}"),
        pre_close: 10.0,
    }
}

fn source_quote(code: &str) -> SecurityQuote {
    SecurityQuote {
        market: 1,
        code: code.into(),
        active1: 0,
        price: 10.0,
        last_close: 9.0,
        open: 9.5,
        high: 10.5,
        low: 8.5,
        servertime: "10:00:01".into(),
        vol: 1_000.0,
        cur_vol: 10.0,
        amount: 10_000.0,
        s_vol: 400.0,
        b_vol: 600.0,
        bid1: 9.9,
        bid_vol1: 10.0,
        bid2: 9.8,
        bid_vol2: 11.0,
        bid3: 9.7,
        bid_vol3: 12.0,
        bid4: 9.6,
        bid_vol4: 13.0,
        bid5: 9.5,
        bid_vol5: 14.0,
        ask1: 10.1,
        ask_vol1: 15.0,
        ask2: 10.2,
        ask_vol2: 16.0,
        ask3: 10.3,
        ask_vol3: 17.0,
        ask4: 10.4,
        ask_vol4: 18.0,
        ask5: 10.5,
        ask_vol5: 19.0,
        reversed_bytes0: 0,
        reversed_bytes1: 0,
        reversed_bytes2: 0,
        reversed_bytes3: 0,
        reversed_bytes4: 0,
        reversed_bytes5: 0,
        reversed_bytes6: 0,
        reversed_bytes7: 0,
        reversed_bytes8: 0,
        reversed_bytes9: 0,
        active2: 0,
    }
}

fn minute(time: &str) -> MinuteTimePrice {
    MinuteTimePrice {
        time: time.into(),
        price: 10.0,
        avg_price: 10.0,
        vol: 100.0,
    }
}

fn tick(time: &str) -> TickData {
    TickData {
        time: time.into(),
        price: 10.0,
        vol: 100.0,
        num: 1,
        buyorsell: 0,
        reserved: 0,
    }
}

#[test]
fn blocking_service_security_list_seam_assembles_declared_count_atomically() {
    let query = ScriptedBlockingServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(2001)])),
        list_responses: RefCell::new(VecDeque::from([
            Ok((0..1000).map(security).collect()),
            Ok((1000..2000).map(security).collect()),
            Ok(vec![security(2000)]),
        ])),
        ..Default::default()
    };

    let records = security_list_all_with(&query, 1).unwrap();

    assert_eq!(records.len(), 2001);
    assert_eq!(*query.count_calls.borrow(), vec![1]);
    assert_eq!(
        *query.list_calls.borrow(),
        vec![(1, 0), (1, 1000), (1, 2000)]
    );
    assert_eq!(records[2000].code, "002000");
}

#[test]
fn blocking_service_security_list_seam_rejects_declared_count_mismatches() {
    let premature_end = ScriptedBlockingServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(1)])),
        list_responses: RefCell::new(VecDeque::from([Ok(Vec::new())])),
        ..Default::default()
    };
    assert!(matches!(
        security_list_all_with(&premature_end, 1),
        Err(TdxError::InvalidData(_))
    ));

    let oversized_page = ScriptedBlockingServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(1)])),
        list_responses: RefCell::new(VecDeque::from([Ok(vec![security(0), security(1)])])),
        ..Default::default()
    };
    assert!(matches!(
        security_list_all_with(&oversized_page, 1),
        Err(TdxError::InvalidData(_))
    ));

    let empty_market = ScriptedBlockingServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(0)])),
        ..Default::default()
    };
    assert!(security_list_all_with(&empty_market, 1).unwrap().is_empty());
    assert!(empty_market.list_calls.borrow().is_empty());
}

#[test]
fn blocking_service_quote_seam_chunks_at_sixty_and_restores_order() {
    let instruments: Vec<_> = (0..61)
        .map(|index| instrument(Exchange::Shanghai, &format!("6{index:05}")))
        .collect();
    let mut first_page: Vec<_> = instruments[..60]
        .iter()
        .rev()
        .map(|id| source_quote(id.code()))
        .collect();
    let second_page = vec![source_quote(instruments[60].code())];
    let query = ScriptedBlockingServiceQuery {
        quote_responses: RefCell::new(VecDeque::from([
            Ok(std::mem::take(&mut first_page)),
            Ok(second_page),
        ])),
        ..Default::default()
    };

    let batch = quotes_chunked_with(&query, &instruments).unwrap();

    assert_eq!(query.quote_calls.borrow()[0].len(), 60);
    assert_eq!(query.quote_calls.borrow()[1].len(), 1);
    assert_eq!(batch.records().len(), 61);
    for (record, expected) in batch.records().iter().zip(&instruments) {
        assert_eq!(record.instrument(), expected);
    }
    assert_eq!(batch.provenance().source(), "tdx-smart-chunked");
}

#[test]
fn blocking_service_quote_seam_rejects_incomplete_and_duplicate_chunks_atomically() {
    let instruments = [
        instrument(Exchange::Shanghai, "600001"),
        instrument(Exchange::Shanghai, "600002"),
    ];
    let incomplete = ScriptedBlockingServiceQuery {
        quote_responses: RefCell::new(VecDeque::from([Ok(vec![source_quote("600001")])])),
        ..Default::default()
    };
    assert!(matches!(
        quotes_chunked_with(&incomplete, &instruments),
        Err(TdxError::InvalidData(_))
    ));

    let duplicate = ScriptedBlockingServiceQuery {
        quote_responses: RefCell::new(VecDeque::from([Ok(vec![
            source_quote("600001"),
            source_quote("600001"),
        ])])),
        ..Default::default()
    };
    assert!(matches!(
        quotes_chunked_with(&duplicate, &instruments),
        Err(TdxError::InvalidData(_))
    ));

    let unexpected = ScriptedBlockingServiceQuery {
        quote_responses: RefCell::new(VecDeque::from([Ok(vec![
            source_quote("600001"),
            source_quote("600003"),
        ])])),
        ..Default::default()
    };
    assert!(matches!(
        quotes_chunked_with(&unexpected, &instruments),
        Err(TdxError::InvalidData(_))
    ));
}

#[test]
fn blocking_service_raw_query_seam_preserves_all_passthrough_parameters() {
    let query = ScriptedBlockingServiceQuery {
        minute_responses: RefCell::new(VecDeque::from([Ok(vec![minute("09:31")])])),
        history_minute_responses: RefCell::new(VecDeque::from([Ok(vec![minute("09:32")])])),
        transaction_responses: RefCell::new(VecDeque::from([Ok(vec![tick("09:33:00")])])),
        history_transaction_responses: RefCell::new(VecDeque::from([Ok(vec![tick("09:34:00")])])),
        ..Default::default()
    };

    assert_eq!(
        minute_data_with(&query, 1, "600396").unwrap()[0].time,
        "09:31"
    );
    assert_eq!(
        history_minute_data_with(&query, 1, "600396", 20260723).unwrap()[0].time,
        "09:32"
    );
    assert_eq!(
        transactions_with(&query, 1, "600396", 5, 10).unwrap()[0].time,
        "09:33:00"
    );
    assert_eq!(
        history_transactions_with(&query, 1, "600396", 15, 20, 20260722).unwrap()[0].time,
        "09:34:00"
    );

    assert_eq!(*query.minute_calls.borrow(), vec![(1, "600396".into())]);
    assert_eq!(
        *query.history_minute_calls.borrow(),
        vec![(1, "600396".into(), 20260723)]
    );
    assert_eq!(
        *query.transaction_calls.borrow(),
        vec![(1, "600396".into(), 5, 10)]
    );
    assert_eq!(
        *query.history_transaction_calls.borrow(),
        vec![(1, "600396".into(), 15, 20, 20260722)]
    );
}

#[derive(Default)]
struct ScriptedAsyncServiceQuery {
    count_calls: RefCell<Vec<u8>>,
    count_responses: RefCell<VecDeque<Result<u16, TdxError>>>,
    list_calls: RefCell<Vec<(u8, u16)>>,
    list_responses: RefCell<VecDeque<Result<Vec<SecurityInfo>, TdxError>>>,
    quote_calls: RefCell<Vec<Vec<(u8, String)>>>,
    quote_responses: RefCell<VecDeque<Result<Vec<SecurityQuote>, TdxError>>>,
    minute_calls: RefCell<Vec<(u8, String)>>,
    minute_responses: RefCell<VecDeque<Result<Vec<MinuteTimePrice>, TdxError>>>,
    history_minute_calls: RefCell<Vec<(u8, String, u32)>>,
    history_minute_responses: RefCell<VecDeque<Result<Vec<MinuteTimePrice>, TdxError>>>,
    transaction_calls: RefCell<Vec<(u8, String, u16, u16)>>,
    transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
    history_transaction_calls: RefCell<Vec<HistoryTransactionCall>>,
    history_transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
}

impl crate::adapter::AsyncTdxQuery for ScriptedAsyncServiceQuery {
    async fn security_bars(
        &self,
        _category: u8,
        _market: u8,
        _code: &str,
        _start: u32,
        _count: u16,
        _adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        unconfigured("async bars")
    }

    async fn security_quotes(
        &self,
        instruments: &[(u8, &str)],
    ) -> Result<Vec<SecurityQuote>, TdxError> {
        self.quote_calls.borrow_mut().push(
            instruments
                .iter()
                .map(|(market, code)| (*market, (*code).to_owned()))
                .collect(),
        );
        self.quote_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async quotes"))
    }

    async fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError> {
        self.transaction_calls
            .borrow_mut()
            .push((market, code.to_owned(), start, count));
        self.transaction_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async current transactions"))
    }

    async fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError> {
        self.history_transaction_calls.borrow_mut().push((
            market,
            code.to_owned(),
            start,
            count,
            date,
        ));
        self.history_transaction_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async history transactions"))
    }

    async fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        self.count_calls.borrow_mut().push(market);
        self.count_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async security count"))
    }

    async fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        self.list_calls.borrow_mut().push((market, start));
        self.list_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async security list"))
    }

    async fn minute_time_data(
        &self,
        market: u8,
        code: &str,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        self.minute_calls
            .borrow_mut()
            .push((market, code.to_owned()));
        self.minute_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async current minute"))
    }

    async fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        self.history_minute_calls
            .borrow_mut()
            .push((market, code.to_owned(), date));
        self.history_minute_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| unconfigured("async history minute"))
    }
}

#[tokio::test]
async fn async_service_security_list_seam_assembles_declared_count_atomically() {
    let query = ScriptedAsyncServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(2001)])),
        list_responses: RefCell::new(VecDeque::from([
            Ok((0..1000).map(security).collect()),
            Ok((1000..2000).map(security).collect()),
            Ok(vec![security(2000)]),
        ])),
        ..Default::default()
    };

    let records = security_list_all_async_with(&query, 1).await.unwrap();

    assert_eq!(records.len(), 2001);
    assert_eq!(*query.count_calls.borrow(), vec![1]);
    assert_eq!(
        *query.list_calls.borrow(),
        vec![(1, 0), (1, 1000), (1, 2000)]
    );
}

#[tokio::test]
async fn async_service_security_list_seam_rejects_declared_count_mismatches() {
    let premature_end = ScriptedAsyncServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(1)])),
        list_responses: RefCell::new(VecDeque::from([Ok(Vec::new())])),
        ..Default::default()
    };
    assert!(matches!(
        security_list_all_async_with(&premature_end, 1).await,
        Err(TdxError::InvalidData(_))
    ));

    let oversized_page = ScriptedAsyncServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(1)])),
        list_responses: RefCell::new(VecDeque::from([Ok(vec![security(0), security(1)])])),
        ..Default::default()
    };
    assert!(matches!(
        security_list_all_async_with(&oversized_page, 1).await,
        Err(TdxError::InvalidData(_))
    ));

    let empty_market = ScriptedAsyncServiceQuery {
        count_responses: RefCell::new(VecDeque::from([Ok(0)])),
        ..Default::default()
    };
    assert!(security_list_all_async_with(&empty_market, 1)
        .await
        .unwrap()
        .is_empty());
    assert!(empty_market.list_calls.borrow().is_empty());
}

#[tokio::test]
async fn async_service_order_book_seam_reuses_shared_normalization() {
    let query = ScriptedAsyncServiceQuery {
        quote_responses: RefCell::new(VecDeque::from([Ok(vec![source_quote("600396")])])),
        ..Default::default()
    };
    let instruments = [instrument(Exchange::Shanghai, "600396")];

    let batch = order_books_async_with(&query, &instruments).await.unwrap();

    assert_eq!(
        *query.quote_calls.borrow(),
        vec![vec![(1, "600396".into())]]
    );
    let book = &batch.records()[0];
    assert_eq!(book.total_bid_quantity().unwrap().get(), 60.0);
    assert_eq!(book.total_ask_quantity().unwrap().get(), 85.0);
    assert_eq!(book.provider(), magic_market_core::ProviderId::Tdx);
    assert_eq!(batch.provenance().source(), "tdx-async");
}

#[tokio::test]
async fn async_service_order_book_seam_rejects_incomplete_decoded_batches() {
    let query = ScriptedAsyncServiceQuery {
        quote_responses: RefCell::new(VecDeque::from([Ok(Vec::new())])),
        ..Default::default()
    };
    let instruments = [instrument(Exchange::Shanghai, "600396")];

    assert!(matches!(
        order_books_async_with(&query, &instruments).await,
        Err(TdxError::InvalidData(_))
    ));
    assert_eq!(
        *query.quote_calls.borrow(),
        vec![vec![(1, "600396".into())]]
    );
}

#[tokio::test]
async fn async_service_raw_query_seam_preserves_all_passthrough_parameters() {
    let query = ScriptedAsyncServiceQuery {
        minute_responses: RefCell::new(VecDeque::from([Ok(vec![minute("09:31")])])),
        history_minute_responses: RefCell::new(VecDeque::from([Ok(vec![minute("09:32")])])),
        transaction_responses: RefCell::new(VecDeque::from([Ok(vec![tick("09:33:00")])])),
        history_transaction_responses: RefCell::new(VecDeque::from([Ok(vec![tick("09:34:00")])])),
        ..Default::default()
    };

    assert_eq!(
        minute_data_async_with(&query, 1, "600396").await.unwrap()[0].time,
        "09:31"
    );
    assert_eq!(
        history_minute_data_async_with(&query, 1, "600396", 20260723)
            .await
            .unwrap()[0]
            .time,
        "09:32"
    );
    assert_eq!(
        transactions_async_with(&query, 1, "600396", 5, 10)
            .await
            .unwrap()[0]
            .time,
        "09:33:00"
    );
    assert_eq!(
        history_transactions_async_with(&query, 1, "600396", 15, 20, 20260722)
            .await
            .unwrap()[0]
            .time,
        "09:34:00"
    );
    assert_eq!(*query.minute_calls.borrow(), vec![(1, "600396".into())]);
    assert_eq!(
        *query.history_minute_calls.borrow(),
        vec![(1, "600396".into(), 20260723)]
    );
    assert_eq!(
        *query.transaction_calls.borrow(),
        vec![(1, "600396".into(), 5, 10)]
    );
    assert_eq!(
        *query.history_transaction_calls.borrow(),
        vec![(1, "600396".into(), 15, 20, 20260722)]
    );
}

#[test]
fn service_market_mapping_and_construction_are_explicit() {
    assert_eq!(
        market(&instrument(Exchange::Shanghai, "600396")).unwrap(),
        1
    );
    assert_eq!(
        market(&instrument(Exchange::Shenzhen, "000001")).unwrap(),
        0
    );
    assert_eq!(market(&instrument(Exchange::Beijing, "920118")).unwrap(), 2);
    let blocking = TdxService::default();
    let _ = blocking.client();
    let asynchronous = AsyncTdxService::default();
    let _ = asynchronous.client();
}

#[test]
fn blocking_service_rejects_requests_before_any_transport_call() {
    let service = TdxService::new();
    service.client().inner().set_auto_retry(false);
    let ranged = BarsRequest::new(
        instrument(Exchange::Shanghai, "600396"),
        BarInterval::Day,
        1,
    )
    .unwrap()
    .with_range("2026-07-01", "2026-07-23")
    .unwrap();
    assert!(matches!(
        service.bars(&ranged),
        Err(TdxError::Unsupported(_))
    ));
    assert!(service.quotes(&[]).is_err());
    assert!(service.quotes_chunked(&[]).is_err());

    let beijing = instrument(Exchange::Beijing, "920118");
    assert!(matches!(
        service.security_metadata(std::slice::from_ref(&beijing)),
        Err(TdxError::Unsupported(_))
    ));
}

#[test]
fn blocking_service_facade_preserves_every_explicit_unsupported_capability() {
    let service = TdxService::new();
    let id = instrument(Exchange::Beijing, "920118");

    assert!(matches!(
        service.money_flows(std::slice::from_ref(&id)),
        Err(TdxError::Unsupported(_))
    ));
    assert!(matches!(
        service.auction_snapshots(std::slice::from_ref(&id)),
        Err(TdxError::Unsupported(_))
    ));
}

#[test]
fn blocking_service_maps_disconnected_failures_for_raw_facade_families() {
    let service = TdxService::new();
    service.client().inner().set_auto_retry(false);
    let id = instrument(Exchange::Shanghai, "600396");
    let historical_trades = TradesRequest::new(id.clone(), 1)
        .unwrap()
        .with_date("2026-07-23")
        .unwrap();

    for (operation, failed) in [
        (
            "historical trades",
            service.trades(&historical_trades).is_err(),
        ),
        (
            "security metadata",
            service
                .security_metadata(std::slice::from_ref(&id))
                .is_err(),
        ),
        ("security count", service.security_count(1).is_err()),
        ("security list", service.security_list(1, 0).is_err()),
        (
            "complete security list",
            service.security_list_all(1).is_err(),
        ),
        ("minute data", service.minute_data(1, "600396").is_err()),
        (
            "historical minute data",
            service.history_minute_data(1, "600396", 20260723).is_err(),
        ),
        (
            "transactions",
            service.transactions(1, "600396", 0, 1).is_err(),
        ),
        (
            "historical transactions",
            service
                .history_transactions(1, "600396", 0, 1, 20260723)
                .is_err(),
        ),
        ("finance", service.finance(1, "600396").is_err()),
        (
            "corporate actions",
            service.corporate_actions(1, "600396").is_err(),
        ),
    ] {
        assert!(failed, "{operation} must preserve the disconnected failure");
    }

    let actions = CorporateActionRequest::new(id);
    assert!(service.normalized_corporate_actions(&actions).is_err());
}

#[test]
fn blocking_query_defaults_keep_optional_metadata_explicitly_unsupported() {
    let query = ScriptedBlockingServiceQuery::default();
    assert!(matches!(
        crate::adapter::BlockingTdxQuery::finance_info(&query, 1, "600396"),
        Err(TdxError::Unsupported(_))
    ));
    assert!(matches!(
        crate::adapter::BlockingTdxQuery::xdxr_info(&query, 1, "600396"),
        Err(TdxError::Unsupported(_))
    ));
}

#[tokio::test]
async fn async_service_maps_disconnected_failures_for_every_facade_family() {
    let service = AsyncTdxService::new();
    let id = instrument(Exchange::Shanghai, "600396");
    let ranged = BarsRequest::new(id.clone(), BarInterval::Day, 1)
        .unwrap()
        .with_range("2026-07-01", "2026-07-23")
        .unwrap();
    assert!(matches!(
        service.bars(&ranged).await,
        Err(TdxError::Unsupported(_))
    ));
    assert!(service.order_books(&[]).await.is_err());

    let trades = TradesRequest::new(id.clone(), 1).unwrap();
    let historical_trades = TradesRequest::new(id.clone(), 1)
        .unwrap()
        .with_date("2026-07-23")
        .unwrap();
    let minute_request = MinuteDataRequest::new(id.clone());
    assert_eq!(minute_request.instrument(), &id);

    let (
        quotes,
        current_trades,
        historical_trade_batch,
        count,
        list,
        all,
        minute,
        history_minute,
        transactions,
        history_transactions,
        finance,
        actions,
    ) = tokio::join!(
        service.quotes(std::slice::from_ref(&id)),
        service.trades(&trades),
        service.trades(&historical_trades),
        service.security_count(1),
        service.security_list(1, 0),
        service.security_list_all(1),
        service.minute_data(1, "600396"),
        service.history_minute_data(1, "600396", 20260723),
        service.transactions(1, "600396", 0, 1),
        service.history_transactions(1, "600396", 0, 1, 20260723),
        service.finance(1, "600396"),
        service.corporate_actions(1, "600396"),
    );
    for failed in [
        quotes.is_err(),
        current_trades.is_err(),
        historical_trade_batch.is_err(),
        count.is_err(),
        list.is_err(),
        all.is_err(),
        minute.is_err(),
        history_minute.is_err(),
        transactions.is_err(),
        history_transactions.is_err(),
        finance.is_err(),
        actions.is_err(),
    ] {
        assert!(failed);
    }

    let actions = CorporateActionRequest::new(instrument(Exchange::Beijing, "920118"));
    assert!(service
        .normalized_corporate_actions(&actions)
        .await
        .is_err());
}
