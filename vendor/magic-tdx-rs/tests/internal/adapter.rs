use super::*;
use magic_market_core::Adjustment;
use std::cell::RefCell;
use std::collections::VecDeque;

type SecurityBarsCall = (u8, u8, String, u32, u16, u8);
type HistoryTransactionCall = (u8, String, u16, u16, u32);

#[test]
fn rejects_bar_ranges_instead_of_silently_ignoring_them() {
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 5)
        .unwrap()
        .with_range("2026-07-01", "2026-07-22")
        .unwrap();
    assert!(matches!(
        reject_unsupported_bar_range(&request),
        Err(TdxError::Unsupported(_))
    ));
}

#[test]
fn maps_standard_one_minute_monthly_and_yearly_categories_exactly() {
    assert_eq!(category(BarInterval::Minute1).unwrap(), 8);
    assert_eq!(category(BarInterval::Month).unwrap(), 6);
    assert_eq!(category(BarInterval::Year).unwrap(), 11);
}

#[test]
fn pagination_and_preflight_helpers_reject_impossible_terminal_states() {
    assert!(ensure_nonempty::<u8>(&[]).is_err());

    let mut complete = HistoricalBarPagination::new(1);
    complete.accept_page(vec![source_bar()]).unwrap();
    assert!(complete.accept_page(Vec::new()).is_err());

    assert!(HistoricalBarPagination::new(1).finish().is_err());
    assert!(HistoricalBarPagination {
        expected: 1,
        remaining: 0,
        offset: 0,
        pages: Vec::new(),
    }
    .finish()
    .is_err());

    assert_eq!(normalize_ipo_date(0).unwrap(), None);
    assert!(normalize_ipo_date(u32::MAX).is_err());

    let option = InstrumentId::new(Exchange::Shanghai, "10000001", AssetClass::Option).unwrap();
    assert!(matches!(
        validate_corporate_action_request(
            &CorporateActionRequest::new(option),
            &IsoDate::new("2026-07-27").unwrap()
        ),
        Err(TdxError::Unsupported(_))
    ));
    let future = CorporateActionRequest::new(instrument("600001"))
        .with_range(
            IsoDate::new("2026-07-28").unwrap(),
            IsoDate::new("2026-07-29").unwrap(),
        )
        .unwrap();
    assert!(
        validate_corporate_action_request(&future, &IsoDate::new("2026-07-27").unwrap()).is_err()
    );
}

#[test]
fn order_book_levels_preserve_absence_atomically() {
    let absent = book_level(0.0, 0.0).unwrap();
    assert!(absent.price().is_none());
    assert!(absent.quantity().is_none());
    let half_present = book_level(0.0, 1.0).unwrap();
    assert!(half_present.price().is_none());
    assert!(half_present.quantity().is_none());
    assert!(book_level(10.0, -1.0).is_err());
}
use magic_market_core::{AssetClass, Exchange};

fn instrument(code: &str) -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap()
}

fn normalize_corporate_actions(
    source: &str,
    request: &CorporateActionRequest,
    records: Vec<XdXrInfo>,
) -> Result<DataBatch<CorporateAction>, TdxError> {
    super::normalize_corporate_actions(
        source,
        request,
        records,
        &IsoDate::new("2026-07-27").unwrap(),
    )
}

fn source_bar() -> SecurityBar {
    SecurityBar {
        open: 10.0,
        close: 11.0,
        high: 12.0,
        low: 9.0,
        vol: 100.0,
        amount: 1_000.0,
        year: 2026,
        month: 7,
        day: 23,
        hour: 0,
        minute: 0,
        datetime: "2026-07-23".into(),
    }
}

fn indexed_daily_bar(index: usize) -> SecurityBar {
    let year = 2020 + index / (12 * 28);
    let month = 1 + (index / 28) % 12;
    let day = 1 + index % 28;
    let datetime = format!("{year:04}-{month:02}-{day:02}");
    SecurityBar {
        year: u32::try_from(year).unwrap(),
        month: u32::try_from(month).unwrap(),
        day: u32::try_from(day).unwrap(),
        datetime,
        ..source_bar()
    }
}

fn indexed_daily_bars(start: usize, count: usize) -> Vec<SecurityBar> {
    (start..start + count).map(indexed_daily_bar).collect()
}

fn source_finance(market: u8, code: &str, ipo_date: u32) -> FinanceInfo {
    FinanceInfo {
        market,
        code: code.into(),
        liutongguben: 0.0,
        province: 0,
        industry: 0,
        updated_date: 20260727,
        ipo_date,
        zongguben: 0.0,
        guojiagu: 0.0,
        faqirenfarengu: 0.0,
        farengu: 0.0,
        bgu: 0.0,
        hgu: 0.0,
        zhigonggu: 0.0,
        zongzichan: 0.0,
        liudongzichan: 0.0,
        gudingzichan: 0.0,
        wuxingzichan: 0.0,
        gudongrenshu: 0.0,
        liudongfuzhai: 0.0,
        changqifuzhai: 0.0,
        zibengongjijin: 0.0,
        jingzichan: 0.0,
        zhuyingshouru: 0.0,
        zhuyinglirun: 0.0,
        yingshouzhangkuan: 0.0,
        yingyelirun: 0.0,
        touzishouyu: 0.0,
        jingyingxianjinliu: 0.0,
        zongxianjinliu: 0.0,
        cunhuo: 0.0,
        lirunzonghe: 0.0,
        shuihoulirun: 0.0,
        jinglirun: 0.0,
        weifenpeilirun: 0.0,
        meigujingzichan: 0.0,
    }
}

fn source_action(date: (u32, u32, u32), category: u32, value: f64) -> XdXrInfo {
    let capital_structure = (2..=10).contains(&category);
    XdXrInfo {
        year: date.0,
        month: date.1,
        day: date.2,
        category,
        name: "fixture".into(),
        fenhong: (category == 1).then_some(value),
        peigujia: (category == 1).then_some(12.0),
        songzhuangu: (category == 1).then_some(1.0),
        peigu: (category == 1).then_some(0.5),
        suogu: matches!(category, 11 | 12).then_some(value),
        panqianliutong: capital_structure.then_some(100.0),
        panhouliutong: capital_structure.then_some(100.0 + value),
        qianzongguben: capital_structure.then_some(200.0),
        houzongguben: capital_structure.then_some(200.0 + value),
        fenshu: matches!(category, 13 | 14).then_some(value),
        xingquanjia: matches!(category, 13 | 14).then_some(30.3),
    }
}

#[derive(Default)]
struct ScriptedBarsQuery {
    calls: RefCell<Vec<SecurityBarsCall>>,
    responses: RefCell<VecDeque<Result<Vec<SecurityBar>, TdxError>>>,
    quote_calls: RefCell<Vec<Vec<(u8, String)>>>,
    quote_response: RefCell<Option<Result<Vec<SecurityQuote>, TdxError>>>,
    minute_calls: RefCell<Vec<(u8, String)>>,
    minute_response: RefCell<Option<Result<Vec<MinuteTimePrice>, TdxError>>>,
    history_minute_calls: RefCell<Vec<(u8, String, u32)>>,
    history_minute_response: RefCell<Option<Result<Vec<MinuteTimePrice>, TdxError>>>,
    transaction_calls: RefCell<Vec<(u8, String, u16, u16)>>,
    transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
    history_transaction_calls: RefCell<Vec<HistoryTransactionCall>>,
    history_transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
    security_count_calls: RefCell<Vec<u8>>,
    security_count_responses: RefCell<VecDeque<Result<u16, TdxError>>>,
    security_list_calls: RefCell<Vec<(u8, u16)>>,
    security_list_responses: RefCell<VecDeque<Result<Vec<SecurityInfo>, TdxError>>>,
    finance_calls: RefCell<Vec<(u8, String)>>,
    finance_responses: RefCell<VecDeque<Result<FinanceInfo, TdxError>>>,
    xdxr_calls: RefCell<Vec<(u8, String)>>,
    xdxr_responses: RefCell<VecDeque<Result<Vec<XdXrInfo>, TdxError>>>,
}

impl BlockingTdxQuery for ScriptedBarsQuery {
    fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        self.calls
            .borrow_mut()
            .push((category, market, code.to_owned(), start, count, adjust));
        self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(TdxError::InvalidData(
                "scripted bars response is not configured".into(),
            ))
        })
    }

    fn security_quotes(&self, instruments: &[(u8, &str)]) -> Result<Vec<SecurityQuote>, TdxError> {
        self.quote_calls.borrow_mut().push(
            instruments
                .iter()
                .map(|(market, code)| (*market, (*code).to_owned()))
                .collect(),
        );
        self.quote_response.borrow_mut().take().unwrap_or_else(|| {
            Err(TdxError::InvalidData(
                "scripted quote response is not configured".into(),
            ))
        })
    }

    fn minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>, TdxError> {
        self.minute_calls
            .borrow_mut()
            .push((market, code.to_owned()));
        self.minute_response.borrow_mut().take().unwrap_or_else(|| {
            Err(TdxError::InvalidData(
                "scripted current minute response is not configured".into(),
            ))
        })
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
        self.history_minute_response
            .borrow_mut()
            .take()
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted history minute response is not configured".into(),
                ))
            })
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
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted current transaction response is not configured".into(),
                ))
            })
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
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted history transaction response is not configured".into(),
                ))
            })
    }

    fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        self.security_count_calls.borrow_mut().push(market);
        self.security_count_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted security count response is not configured".into(),
                ))
            })
    }

    fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        self.security_list_calls.borrow_mut().push((market, start));
        self.security_list_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted security list response is not configured".into(),
                ))
            })
    }

    fn finance_info(&self, market: u8, code: &str) -> Result<FinanceInfo, TdxError> {
        self.finance_calls
            .borrow_mut()
            .push((market, code.to_owned()));
        self.finance_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted finance response is not configured".into(),
                ))
            })
    }

    fn xdxr_info(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>, TdxError> {
        self.xdxr_calls.borrow_mut().push((market, code.to_owned()));
        self.xdxr_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted XDXR response is not configured".into(),
                ))
            })
    }
}

#[test]
fn blocking_bar_seam_uses_decoded_records_and_exact_request_parameters() {
    let query = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([Ok((19..=23)
            .map(|day| source_bar_at(day, 10.0))
            .collect())])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 5).unwrap();

    let batch = historical_bars_with(&query, "tdx", &request).unwrap();

    assert_eq!(
        *query.calls.borrow(),
        vec![(KLINE_DAILY, 1, "600396".into(), 0, 5, 0)]
    );
    assert_eq!(batch.records().len(), 5);
    assert_eq!(batch.provenance().source(), "tdx");
    assert_eq!(batch.provenance().source_at(), Some("2026-07-23"));
}

#[test]
fn blocking_historical_bars_page_at_801_and_restore_ascending_provider_order() {
    let query = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Ok(indexed_daily_bars(0, 1)),
        ])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 801).unwrap();

    let batch = historical_bars_with(&query, "tdx", &request).unwrap();

    assert_eq!(
        *query.calls.borrow(),
        vec![
            (KLINE_DAILY, 1, "600396".into(), 0, 800, 0),
            (KLINE_DAILY, 1, "600396".into(), 800, 1, 0),
        ]
    );
    assert_eq!(batch.records().len(), 801);
    assert_eq!(batch.records()[0].bar_start(), "2020-01-01");
    assert_eq!(batch.records()[800].bar_start(), "2022-05-17");
    assert_eq!(batch.provenance().source_at(), Some("2022-05-17"));
}

#[test]
fn blocking_historical_bars_use_one_exact_page_at_800() {
    let query = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([Ok(indexed_daily_bars(0, 800))])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 800).unwrap();

    let batch = historical_bars_with(&query, "tdx", &request).unwrap();

    assert_eq!(
        *query.calls.borrow(),
        vec![(KLINE_DAILY, 1, "600396".into(), 0, 800, 0)]
    );
    assert_eq!(batch.records().len(), 800);
    assert_eq!(batch.records()[0].bar_start(), "2020-01-01");
    assert_eq!(batch.records()[799].bar_start(), "2022-05-16");
}

#[test]
fn blocking_historical_bars_fetch_more_than_two_exact_pages() {
    let query = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(801, 800)),
            Ok(indexed_daily_bars(1, 800)),
            Ok(indexed_daily_bars(0, 1)),
        ])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 1_601).unwrap();

    let batch = historical_bars_with(&query, "tdx-smart", &request).unwrap();

    assert_eq!(
        *query.calls.borrow(),
        vec![
            (KLINE_DAILY, 1, "600396".into(), 0, 800, 0),
            (KLINE_DAILY, 1, "600396".into(), 800, 800, 0),
            (KLINE_DAILY, 1, "600396".into(), 1_600, 1, 0),
        ]
    );
    assert_eq!(batch.records().len(), 1_601);
    assert_eq!(batch.records()[0].bar_start(), "2020-01-01");
    assert_eq!(batch.records()[1_600].bar_start(), "2024-10-05");
    assert_eq!(batch.provenance().source(), "tdx-smart");
}

#[test]
fn blocking_historical_bars_reject_second_page_failure_and_short_page_atomically() {
    let failing = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Err(TdxError::Connection("second page failed".into())),
        ])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 801).unwrap();

    let failure = historical_bars_with(&failing, "tdx-direct", &request);

    assert!(matches!(
        failure,
        Err(TdxError::Connection(message)) if message == "second page failed"
    ));
    assert_eq!(failing.calls.borrow().len(), 2);

    let short = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Ok(Vec::new()),
        ])),
        ..Default::default()
    };

    let failure = historical_bars_with(&short, "tdx", &request);

    assert!(matches!(
        failure,
        Err(TdxError::HistoricalBarCardinality {
            offset: 800,
            actual: 0,
            expected_page: 1,
            requested_total: 801,
        })
    ));
    assert_eq!(short.calls.borrow().len(), 2);
}

#[test]
fn blocking_paginated_historical_bars_keep_atomic_sequence_and_structure_validation() {
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 801).unwrap();

    let duplicate = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(800, 800)),
            Ok(indexed_daily_bars(800, 1)),
        ])),
        ..Default::default()
    };
    assert!(matches!(
        historical_bars_with(&duplicate, "tdx", &request),
        Err(TdxError::InvalidData(message)) if message.contains("duplicate or non-increasing")
    ));

    let reversed = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(0, 800)),
            Ok(indexed_daily_bars(900, 1)),
        ])),
        ..Default::default()
    };
    assert!(matches!(
        historical_bars_with(&reversed, "tdx", &request),
        Err(TdxError::InvalidData(message)) if message.contains("duplicate or non-increasing")
    ));

    let mut invalid_timestamp = indexed_daily_bar(0);
    invalid_timestamp.datetime = "2020-01-02".into();
    let invalid_timestamp = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Ok(vec![invalid_timestamp]),
        ])),
        ..Default::default()
    };
    assert!(matches!(
        historical_bars_with(&invalid_timestamp, "tdx", &request),
        Err(TdxError::InvalidData(message)) if message.contains("contradicts decoded components")
    ));

    let mut invalid_amount = indexed_daily_bar(0);
    invalid_amount.amount = -1.0;
    let invalid_amount = ScriptedBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Ok(vec![invalid_amount]),
        ])),
        ..Default::default()
    };
    assert!(matches!(
        historical_bars_with(&invalid_amount, "tdx", &request),
        Err(TdxError::InvalidData(message)) if message.contains("amount is invalid")
    ));
}

#[test]
fn blocking_quote_seam_restores_order_and_preserves_source_label() {
    let query = ScriptedBarsQuery {
        quote_response: RefCell::new(Some(Ok(vec![
            source_quote("600002", 102.0),
            source_quote("600001", 101.0),
        ]))),
        ..Default::default()
    };
    let instruments = [instrument("600001"), instrument("600002")];

    let batch = realtime_quotes_with(&query, "tdx-smart", &instruments).unwrap();

    assert_eq!(
        *query.quote_calls.borrow(),
        vec![vec![(1, "600001".into()), (1, "600002".into())]]
    );
    assert_eq!(batch.records()[0].instrument().code(), "600001");
    assert_eq!(batch.records()[0].price().get(), 101.0);
    assert_eq!(batch.records()[1].instrument().code(), "600002");
    assert_eq!(batch.provenance().source(), "tdx-smart");
}

#[test]
fn blocking_historical_minute_seam_uses_explicit_source_date() {
    let query = ScriptedBarsQuery {
        history_minute_response: RefCell::new(Some(Ok(vec![MinuteTimePrice {
            time: "09:31".into(),
            price: 15.4,
            avg_price: 15.4,
            vol: 10.0,
        }]))),
        ..Default::default()
    };
    let request = MinuteDataRequest::new(instrument("600396"))
        .with_date("2026-07-23")
        .unwrap();

    let batch = minute_data_with(&query, "tdx", &request).unwrap();

    assert!(query.minute_calls.borrow().is_empty());
    assert_eq!(
        *query.history_minute_calls.borrow(),
        vec![(1, "600396".into(), 20260723)]
    );
    assert_eq!(batch.records()[0].minute_at(), "2026-07-23 09:31");
    assert_eq!(batch.provenance().source(), "tdx");
    assert_eq!(
        batch.provenance().source_at(),
        Some("2026-07-23T09:31:00+08:00")
    );
}

#[test]
fn blocking_current_minute_rejects_before_transport_outside_session() {
    let query = ScriptedBarsQuery::default();
    let request = MinuteDataRequest::new(instrument("600396"));

    let result = minute_data_with_session(&query, "tdx", &request, |_| {
        Err(TdxError::InvalidData("outside session".into()))
    });

    assert!(matches!(
        result,
        Err(TdxError::InvalidData(message)) if message == "outside session"
    ));
    assert!(query.minute_calls.borrow().is_empty());
    assert!(query.history_minute_calls.borrow().is_empty());
}

#[test]
fn blocking_trade_seam_uses_historical_query_and_stops_on_short_page() {
    let query = ScriptedBarsQuery {
        history_transaction_responses: RefCell::new(VecDeque::from([Ok(vec![
            source_trade(0, 0),
            source_trade(1, 1),
        ])])),
        ..Default::default()
    };
    let request = TradesRequest::new(instrument("600519"), 3)
        .unwrap()
        .with_date("2026-07-21")
        .unwrap();

    let batch = trades_with(&query, "tdx-current", "tdx-history", &request).unwrap();

    assert!(query.transaction_calls.borrow().is_empty());
    assert_eq!(
        *query.history_transaction_calls.borrow(),
        vec![(1, "600519".into(), 0, 3, 20260721)]
    );
    assert_eq!(batch.records().len(), 2);
    assert_eq!(batch.records()[0].trade_at(), "2026-07-21 10:00:00");
    assert_eq!(batch.provenance().source(), "tdx-history");
}

#[test]
fn blocking_current_trades_reject_before_transport_outside_session() {
    let query = ScriptedBarsQuery::default();
    let request = TradesRequest::new(instrument("600519"), 3).unwrap();

    let result = trades_with_session(&query, "tdx-current", "tdx-history", &request, |_| {
        Err(TdxError::InvalidData("outside session".into()))
    });

    assert!(matches!(
        result,
        Err(TdxError::InvalidData(message)) if message == "outside session"
    ));
    assert!(query.transaction_calls.borrow().is_empty());
    assert!(query.history_transaction_calls.borrow().is_empty());
}

#[test]
fn blocking_metadata_seam_uses_declared_count_and_source_backed_page() {
    let query = ScriptedBarsQuery {
        security_count_responses: RefCell::new(VecDeque::from([Ok(2)])),
        security_list_responses: RefCell::new(VecDeque::from([Ok(vec![
            SecurityInfo {
                code: "600001".into(),
                volunit: 100,
                decimal_point: 2,
                name: "甲公司".into(),
                pre_close: 10.0,
            },
            SecurityInfo {
                code: "600002".into(),
                volunit: 100,
                decimal_point: 2,
                name: "乙公司".into(),
                pre_close: 20.0,
            },
        ])])),
        finance_responses: RefCell::new(VecDeque::from([
            Ok(source_finance(1, "600002", 20000102)),
            Ok(source_finance(1, "600001", 20000101)),
        ])),
        ..Default::default()
    };
    let instruments = [instrument("600002"), instrument("600001")];

    let batch = security_metadata_with(&query, "tdx", &instruments).unwrap();

    assert_eq!(*query.security_count_calls.borrow(), vec![1]);
    assert_eq!(*query.security_list_calls.borrow(), vec![(1, 0)]);
    assert_eq!(batch.records()[0].instrument().code(), "600002");
    assert_eq!(batch.records()[0].name(), Some("乙公司"));
    assert_eq!(batch.records()[0].listed_on(), Some("2000-01-02"));
    assert_eq!(batch.records()[1].name(), Some("甲公司"));
    assert_eq!(batch.provenance().source(), "tdx");
}

#[test]
fn blocking_order_book_seam_normalizes_five_levels_once() {
    let query = ScriptedBarsQuery {
        quote_response: RefCell::new(Some(Ok(vec![source_quote("600001", 101.0)]))),
        ..Default::default()
    };
    let instruments = [instrument("600001")];

    let batch = order_books_with(&query, "TDX smart", "tdx-smart", &instruments).unwrap();

    let book = &batch.records()[0];
    assert_eq!(book.bids()[0].price().unwrap().get(), 101.9);
    assert_eq!(book.asks()[4].quantity().unwrap().get(), 19.0);
    assert_eq!(book.total_bid_quantity().unwrap().get(), 60.0);
    assert_eq!(book.total_ask_quantity().unwrap().get(), 85.0);
    assert_eq!(book.status(), DataStatus::Unavailable);
    assert_eq!(book.provider(), ProviderId::Tdx);
    assert_eq!(batch.provenance().source(), "tdx-smart");
    assert_eq!(batch.quality().issues().len(), 1);
}

#[derive(Default)]
struct ScriptedAsyncBarsQuery {
    calls: RefCell<Vec<SecurityBarsCall>>,
    responses: RefCell<VecDeque<Result<Vec<SecurityBar>, TdxError>>>,
    quote_calls: RefCell<Vec<Vec<(u8, String)>>>,
    quote_response: RefCell<Option<Result<Vec<SecurityQuote>, TdxError>>>,
    transaction_calls: RefCell<Vec<(u8, String, u16, u16)>>,
    transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
    history_transaction_calls: RefCell<Vec<HistoryTransactionCall>>,
    history_transaction_responses: RefCell<VecDeque<Result<Vec<TickData>, TdxError>>>,
}

impl AsyncTdxQuery for ScriptedAsyncBarsQuery {
    async fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        self.calls
            .borrow_mut()
            .push((category, market, code.to_owned(), start, count, adjust));
        self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(TdxError::InvalidData(
                "scripted async bars response is not configured".into(),
            ))
        })
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
        self.quote_response.borrow_mut().take().unwrap_or_else(|| {
            Err(TdxError::InvalidData(
                "scripted async quote response is not configured".into(),
            ))
        })
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
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted async current transaction response is not configured".into(),
                ))
            })
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
            .unwrap_or_else(|| {
                Err(TdxError::InvalidData(
                    "scripted async history transaction response is not configured".into(),
                ))
            })
    }

    async fn security_count(&self, _market: u8) -> Result<u16, TdxError> {
        Err(TdxError::InvalidData(
            "scripted async security count response is not configured".into(),
        ))
    }

    async fn security_list(&self, _market: u8, _start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        Err(TdxError::InvalidData(
            "scripted async security list response is not configured".into(),
        ))
    }

    async fn minute_time_data(
        &self,
        _market: u8,
        _code: &str,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        Err(TdxError::InvalidData(
            "scripted async minute response is not configured".into(),
        ))
    }

    async fn history_minute_time_data(
        &self,
        _market: u8,
        _code: &str,
        _date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        Err(TdxError::InvalidData(
            "scripted async history minute response is not configured".into(),
        ))
    }
}

#[tokio::test]
async fn async_bar_seam_uses_decoded_records_and_exact_source_label() {
    let query = ScriptedAsyncBarsQuery {
        responses: RefCell::new(VecDeque::from([Ok((19..=23)
            .map(|day| source_bar_at(day, 10.0))
            .collect())])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 5).unwrap();

    let batch = historical_bars_async_with(&query, "tdx-async", &request)
        .await
        .unwrap();

    assert_eq!(
        *query.calls.borrow(),
        vec![(KLINE_DAILY, 1, "600396".into(), 0, 5, 0)]
    );
    assert_eq!(batch.provenance().source(), "tdx-async");
    assert_eq!(batch.provenance().source_at(), Some("2026-07-23"));
}

#[tokio::test]
async fn async_historical_bars_page_at_801_and_reject_second_page_failure() {
    let query = ScriptedAsyncBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Ok(indexed_daily_bars(0, 1)),
        ])),
        ..Default::default()
    };
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 801).unwrap();

    let batch = historical_bars_async_with(&query, "tdx-async", &request)
        .await
        .unwrap();

    assert_eq!(
        *query.calls.borrow(),
        vec![
            (KLINE_DAILY, 1, "600396".into(), 0, 800, 0),
            (KLINE_DAILY, 1, "600396".into(), 800, 1, 0),
        ]
    );
    assert_eq!(batch.records().len(), 801);
    assert_eq!(batch.records()[0].bar_start(), "2020-01-01");
    assert_eq!(batch.records()[800].bar_start(), "2022-05-17");

    let failing = ScriptedAsyncBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Err(TdxError::Disconnected),
        ])),
        ..Default::default()
    };

    let failure = historical_bars_async_with(&failing, "tdx-async", &request).await;

    assert!(matches!(failure, Err(TdxError::Disconnected)));
    assert_eq!(failing.calls.borrow().len(), 2);

    let short = ScriptedAsyncBarsQuery {
        responses: RefCell::new(VecDeque::from([
            Ok(indexed_daily_bars(1, 800)),
            Ok(Vec::new()),
        ])),
        ..Default::default()
    };

    let failure = historical_bars_async_with(&short, "tdx-async", &request).await;

    assert!(matches!(
        failure,
        Err(TdxError::HistoricalBarCardinality {
            offset: 800,
            actual: 0,
            expected_page: 1,
            requested_total: 801,
        })
    ));
    assert_eq!(short.calls.borrow().len(), 2);
}

#[tokio::test]
async fn async_quote_seam_reorders_decoded_records() {
    let query = ScriptedAsyncBarsQuery {
        quote_response: RefCell::new(Some(Ok(vec![
            source_quote("600002", 102.0),
            source_quote("600001", 101.0),
        ]))),
        ..Default::default()
    };
    let instruments = [instrument("600001"), instrument("600002")];

    let batch = realtime_quotes_async_with(&query, "tdx-async", &instruments)
        .await
        .unwrap();

    assert_eq!(
        *query.quote_calls.borrow(),
        vec![vec![(1, "600001".into()), (1, "600002".into())]]
    );
    assert_eq!(batch.records()[0].instrument().code(), "600001");
    assert_eq!(batch.records()[1].instrument().code(), "600002");
    assert_eq!(batch.provenance().source(), "tdx-async");
}

#[tokio::test]
async fn async_trade_seam_uses_historical_query_and_short_terminal_page() {
    let query = ScriptedAsyncBarsQuery {
        history_transaction_responses: RefCell::new(VecDeque::from([Ok(vec![
            source_trade(0, 0),
            source_trade(1, 1),
        ])])),
        ..Default::default()
    };
    let request = TradesRequest::new(instrument("600519"), 3)
        .unwrap()
        .with_date("2026-07-21")
        .unwrap();

    let batch = trades_async_with(&query, &request).await.unwrap();

    assert!(query.transaction_calls.borrow().is_empty());
    assert_eq!(
        *query.history_transaction_calls.borrow(),
        vec![(1, "600519".into(), 0, 3, 20260721)]
    );
    assert_eq!(batch.records().len(), 2);
    assert_eq!(batch.provenance().source(), "tdx-async-history");
}

#[tokio::test]
async fn async_historical_trades_paginate_at_protocol_limit() {
    let query = ScriptedAsyncBarsQuery {
        history_transaction_responses: RefCell::new(VecDeque::from([
            Ok(vec![
                source_trade(0, 0);
                usize::from(HISTORICAL_TRADE_PAGE_SIZE)
            ]),
            Ok(vec![source_trade(1, 1)]),
        ])),
        ..Default::default()
    };
    let request = TradesRequest::new(instrument("600519"), HISTORICAL_TRADE_PAGE_SIZE + 1)
        .unwrap()
        .with_date("2026-07-21")
        .unwrap();

    let batch = trades_async_with(&query, &request).await.unwrap();

    assert!(query.transaction_calls.borrow().is_empty());
    assert_eq!(
        *query.history_transaction_calls.borrow(),
        vec![
            (1, "600519".into(), 0, HISTORICAL_TRADE_PAGE_SIZE, 20260721),
            (1, "600519".into(), HISTORICAL_TRADE_PAGE_SIZE, 1, 20260721),
        ]
    );
    assert_eq!(
        batch.records().len(),
        usize::from(HISTORICAL_TRADE_PAGE_SIZE) + 1
    );
    assert_eq!(batch.provenance().source(), "tdx-async-history");
    assert_eq!(batch.provenance().source_at(), Some("2026-07-21 10:00:01"));
}

#[tokio::test]
async fn async_historical_trades_reject_oversized_page() {
    let query = ScriptedAsyncBarsQuery {
        history_transaction_responses: RefCell::new(VecDeque::from([Ok(vec![
            source_trade(0, 0),
            source_trade(1, 1),
        ])])),
        ..Default::default()
    };
    let request = TradesRequest::new(instrument("600519"), 1)
        .unwrap()
        .with_date("2026-07-21")
        .unwrap();

    let error = trades_async_with(&query, &request).await.unwrap_err();

    assert!(matches!(
        error,
        TdxError::InvalidData(message)
            if message == "TDX async trade page exceeds requested cardinality"
    ));
    assert_eq!(
        *query.history_transaction_calls.borrow(),
        vec![(1, "600519".into(), 0, 1, 20260721)]
    );
}

#[tokio::test]
async fn async_current_trades_reject_before_transport_outside_session() {
    let query = ScriptedAsyncBarsQuery::default();
    let request = TradesRequest::new(instrument("600519"), 3).unwrap();

    let result = trades_async_with_session(&query, &request, |_| {
        Err(TdxError::InvalidData("outside session".into()))
    })
    .await;

    assert!(matches!(
        result,
        Err(TdxError::InvalidData(message)) if message == "outside session"
    ));
    assert!(query.transaction_calls.borrow().is_empty());
    assert!(query.history_transaction_calls.borrow().is_empty());
}

fn source_quote(code: &str, price: f64) -> SecurityQuote {
    SecurityQuote {
        market: 1,
        code: code.into(),
        active1: 0,
        price,
        last_close: 100.0,
        open: 101.0,
        high: 103.0,
        low: 99.0,
        servertime: "10:00:01".into(),
        vol: 1_000.0,
        cur_vol: 10.0,
        amount: 102_000.0,
        s_vol: 400.0,
        b_vol: 600.0,
        bid1: 101.9,
        bid_vol1: 10.0,
        bid2: 101.8,
        bid_vol2: 11.0,
        bid3: 101.7,
        bid_vol3: 12.0,
        bid4: 101.6,
        bid_vol4: 13.0,
        bid5: 101.5,
        bid_vol5: 14.0,
        ask1: 102.1,
        ask_vol1: 15.0,
        ask2: 102.2,
        ask_vol2: 16.0,
        ask3: 102.3,
        ask_vol3: 17.0,
        ask4: 102.4,
        ask_vol4: 18.0,
        ask5: 102.5,
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

#[test]
fn order_book_quotes_are_keyed_by_market_and_code() {
    let instruments = [instrument("600001"), instrument("600002")];
    let ordered = ordered_order_book_quotes(
        &instruments,
        vec![source_quote("600002", 102.0), source_quote("600001", 101.0)],
        "test",
    )
    .unwrap();
    assert_eq!(ordered[0].0.code(), "600001");
    assert_eq!(ordered[0].1.price, 101.0);
    assert_eq!(ordered[1].0.code(), "600002");
    assert_eq!(ordered[1].1.price, 102.0);

    assert!(ordered_order_book_quotes(&[], Vec::new(), "test").is_err());
    assert!(ordered_order_book_quotes(
        &[instrument("600001"), instrument("600001")],
        vec![source_quote("600001", 101.0)],
        "test",
    )
    .is_err());
    assert!(ordered_order_book_quotes(
        &instruments,
        vec![source_quote("600001", 101.0), source_quote("600001", 102.0)],
        "test",
    )
    .is_err());
    assert!(ordered_order_book_quotes(
        &instruments,
        vec![source_quote("600001", 101.0), source_quote("600003", 103.0)],
        "test",
    )
    .is_err());
    assert!(
        ordered_order_book_quotes(&instruments, vec![source_quote("600001", 101.0)], "test",)
            .is_err()
    );
}

fn source_trade(index: u32, side: u32) -> TickData {
    TickData {
        time: format!("10:00:{index:02}"),
        price: 1_300.0 + f64::from(index),
        vol: 100.0 + f64::from(index),
        num: index + 1,
        buyorsell: side,
        reserved: 0,
    }
}

#[test]
fn normalized_quotes_restore_request_order_and_mark_missing_name() {
    let instruments = [instrument("600001"), instrument("600002")];
    let batch = normalize_quotes(
        "test",
        &instruments,
        vec![source_quote("600002", 101.0), source_quote("600001", 102.0)],
    )
    .unwrap();
    assert_eq!(batch.records()[0].instrument().code(), "600001");
    assert_eq!(batch.records()[0].price(), Price::new(102.0).unwrap());
    assert_eq!(
        batch.records()[0].change_percent(),
        Some(Ratio::new(2.0, RatioUnit::Percent).unwrap())
    );
    assert_eq!(batch.records()[0].status(), DataStatus::Unavailable);
    assert!(batch.records()[0].name().is_none());
    assert!(batch.records()[0].source_at().is_none());
    assert!(batch.provenance().source_at().is_none());
    assert_eq!(batch.quality().issues().len(), 6);
}

#[test]
fn normalized_quotes_reject_duplicates_and_missing_records() {
    let duplicated = [instrument("600001"), instrument("600001")];
    assert!(normalize_quotes("test", &duplicated, Vec::new()).is_err());

    let requested = [instrument("600001"), instrument("600002")];
    assert!(normalize_quotes("test", &requested, vec![source_quote("600001", 102.0)]).is_err());
}

#[test]
fn normalizes_only_source_backed_security_metadata() {
    let star = instrument("688001");
    let chinext = InstrumentId::new(Exchange::Shenzhen, "300001", AssetClass::Equity).unwrap();
    let records = vec![
        (
            0,
            SecurityInfo {
                code: "300001".into(),
                volunit: 100,
                decimal_point: 2,
                name: "*ST示例".into(),
                pre_close: 10.0,
            },
        ),
        (
            1,
            SecurityInfo {
                code: "688001".into(),
                volunit: 100,
                decimal_point: 2,
                name: "科创示例".into(),
                pre_close: 20.0,
            },
        ),
    ];

    let batch = normalize_security_metadata("test", &[star, chinext], records).unwrap();
    assert_eq!(batch.records()[0].board(), Some(Board::Star));
    assert_eq!(batch.records()[0].is_st(), Some(false));
    assert_eq!(batch.records()[1].board(), Some(Board::ChiNext));
    assert_eq!(batch.records()[1].is_st(), Some(true));
    assert!(batch
        .records()
        .iter()
        .all(|record| record.listed_on().is_none()
            && record.price_limit().percent().is_none()
            && record.price_limit().version().is_none()
            && record.status() == DataStatus::Unavailable));
    assert!(!batch.quality().is_complete());
}

#[test]
fn enriches_listing_date_only_from_matching_verified_finance_records() {
    let requested = [instrument("600001")];
    let records = vec![(1, security_info("600001", "甲公司"))];
    let batch = normalize_security_metadata_with_finance(
        "test",
        &requested,
        records.clone(),
        vec![source_finance(1, "600001", 19991231)],
    )
    .unwrap();
    assert_eq!(batch.records()[0].listed_on(), Some("1999-12-31"));
    assert!(!batch
        .quality()
        .issues()
        .iter()
        .any(|issue| issue.contains("listing date unavailable")));

    assert!(normalize_security_metadata_with_finance(
        "test",
        &requested,
        records.clone(),
        vec![source_finance(0, "600001", 19991231)],
    )
    .is_err());
    assert!(normalize_security_metadata_with_finance(
        "test",
        &requested,
        records,
        vec![source_finance(1, "600001", 99991231)],
    )
    .is_err());
}

#[test]
fn normalizes_corporate_actions_in_range_order_with_shared_evidence() {
    let request = CorporateActionRequest::new(instrument("600001"))
        .with_range(
            IsoDate::new("2024-01-01").unwrap(),
            IsoDate::new("2026-07-27").unwrap(),
        )
        .unwrap();
    let batch = normalize_corporate_actions(
        "test",
        &request,
        vec![
            source_action((2023, 6, 1), 11, 2.0),
            source_action((2024, 6, 1), 11, 2.0),
            source_action((2025, 6, 1), 1, 2.0),
            source_action((2026, 6, 1), 12, 0.5),
        ],
    )
    .unwrap();
    assert_eq!(batch.records().len(), 3);
    assert_eq!(batch.records()[0].effective_on().as_str(), "2024-06-01");
    assert_eq!(
        batch.records()[1].category(),
        CorporateActionCategory::Distribution
    );
    assert_eq!(
        batch.records()[2].category(),
        CorporateActionCategory::NonTradableReverseSplit
    );
    let batch_id = batch.provenance().batch_id().unwrap();
    assert!(batch
        .records()
        .iter()
        .all(|record| record.evidence().batch_id() == batch_id
            && record.evidence().source_at().is_none()));
}

#[test]
fn corporate_action_normalization_accepts_monotonic_source_directions_but_rejects_reversals() {
    let request = CorporateActionRequest::new(instrument("600001"));
    let descending = normalize_corporate_actions(
        "test",
        &request,
        vec![
            source_action((2026, 6, 1), 12, 0.5),
            source_action((2025, 6, 1), 1, 2.0),
            source_action((2024, 6, 1), 11, 2.0),
        ],
    )
    .unwrap();
    assert_eq!(
        descending.records()[0].effective_on().as_str(),
        "2024-06-01"
    );

    assert!(normalize_corporate_actions(
        "test",
        &request,
        vec![
            source_action((2026, 6, 1), 12, 0.5),
            source_action((2024, 6, 1), 11, 2.0),
            source_action((2025, 6, 1), 1, 2.0),
        ],
    )
    .is_err());
}

#[test]
fn corporate_action_normalization_preserves_verified_empty_and_rejects_bad_rows() {
    let request = CorporateActionRequest::new(instrument("600001"));
    let empty = normalize_corporate_actions("test", &request, Vec::new()).unwrap();
    assert!(empty.records().is_empty());
    assert!(empty.quality().is_complete());

    let duplicate = source_action((2025, 6, 1), 1, 2.0);
    assert!(
        normalize_corporate_actions("test", &request, vec![duplicate.clone(), duplicate],).is_err()
    );
    assert!(normalize_corporate_actions(
        "test",
        &request,
        vec![source_action((2025, 6, 1), 99, 2.0)],
    )
    .is_err());
    assert!(normalize_corporate_actions(
        "test",
        &request,
        vec![source_action((2025, 2, 31), 11, 2.0)],
    )
    .is_err());
}

#[test]
fn corporate_action_normalization_validates_all_categories_before_range_projection() {
    let request = CorporateActionRequest::new(instrument("600001"))
        .with_range(
            IsoDate::new("2024-01-01").unwrap(),
            IsoDate::new("2024-12-31").unwrap(),
        )
        .unwrap();

    let inside_supported =
        normalize_corporate_actions("test", &request, vec![source_action((2024, 6, 1), 2, 1.0)])
            .unwrap();
    assert_eq!(
        inside_supported.records()[0].category(),
        CorporateActionCategory::BonusRightsListing
    );

    let outside_supported =
        normalize_corporate_actions("test", &request, vec![source_action((2023, 6, 1), 2, 1.0)])
            .unwrap();
    assert!(outside_supported.records().is_empty());

    let outside_unknown =
        normalize_corporate_actions("test", &request, vec![source_action((2023, 6, 1), 99, 0.0)])
            .unwrap_err();
    assert!(matches!(outside_unknown, TdxError::InvalidData(_)));
    assert!(outside_unknown.to_string().contains("category 99"));

    let mut outside_bad_schema = source_action((2023, 6, 1), 11, 2.0);
    outside_bad_schema.suogu = None;
    let error =
        normalize_corporate_actions("test", &request, vec![outside_bad_schema]).unwrap_err();
    assert!(matches!(error, TdxError::InvalidData(_)));
    assert!(error.to_string().contains("source ratio"));

    let mut outside_bad_capital = source_action((2023, 6, 1), 2, 1.0);
    outside_bad_capital.houzongguben = None;
    let error =
        normalize_corporate_actions("test", &request, vec![outside_bad_capital]).unwrap_err();
    assert!(matches!(error, TdxError::InvalidData(_)));
    assert!(error.to_string().contains("total-after"));
}

#[test]
fn corporate_action_normalization_maps_every_tdx_protocol_category() {
    let request = CorporateActionRequest::new(instrument("600001"));
    let records = (1..=14)
        .map(|category| {
            let value = if category == 12 { 0.5 } else { 2.0 };
            source_action((2020, 1, category), category, value)
        })
        .collect();
    let batch = normalize_corporate_actions("test", &request, records).unwrap();
    assert_eq!(batch.records().len(), 14);
    assert_eq!(
        batch
            .records()
            .iter()
            .map(|record| record.category())
            .collect::<Vec<_>>(),
        vec![
            CorporateActionCategory::Distribution,
            CorporateActionCategory::BonusRightsListing,
            CorporateActionCategory::NonTradableShareListing,
            CorporateActionCategory::UnknownCapitalChange,
            CorporateActionCategory::CapitalChange,
            CorporateActionCategory::AdditionalIssuance,
            CorporateActionCategory::ShareRepurchase,
            CorporateActionCategory::AdditionalIssuanceListing,
            CorporateActionCategory::TransferredAllotmentListing,
            CorporateActionCategory::ConvertibleBondListing,
            CorporateActionCategory::CapitalRescaling,
            CorporateActionCategory::NonTradableReverseSplit,
            CorporateActionCategory::SubscriptionWarrantGrant,
            CorporateActionCategory::PutWarrantGrant,
        ]
    );
}

#[test]
fn corporate_action_normalization_rejects_incomplete_or_malformed_extended_terms() {
    let request = CorporateActionRequest::new(instrument("600001"));

    for category in 2..=10 {
        for field in 0..4 {
            let mut record = source_action((2020, 1, category), category, 2.0);
            match field {
                0 => record.panqianliutong = None,
                1 => record.panhouliutong = None,
                2 => record.qianzongguben = None,
                3 => record.houzongguben = None,
                _ => unreachable!(),
            }
            assert!(
                normalize_corporate_actions("test", &request, vec![record]).is_err(),
                "category {category} accepted missing field {field}"
            );
        }
    }

    let mut negative_capital = source_action((2020, 1, 2), 2, 2.0);
    negative_capital.panqianliutong = Some(-1.0);
    assert!(normalize_corporate_actions("test", &request, vec![negative_capital]).is_err());

    let mut non_finite_capital = source_action((2020, 1, 2), 2, 2.0);
    non_finite_capital.houzongguben = Some(f64::NAN);
    assert!(normalize_corporate_actions("test", &request, vec![non_finite_capital]).is_err());

    for category in [13, 14] {
        let mut missing_price = source_action((2020, 1, category), category, 2.0);
        missing_price.xingquanjia = None;
        assert!(normalize_corporate_actions("test", &request, vec![missing_price]).is_err());

        let mut missing_quantity = source_action((2020, 1, category), category, 2.0);
        missing_quantity.fenshu = None;
        assert!(normalize_corporate_actions("test", &request, vec![missing_quantity]).is_err());

        let zero_quantity = source_action((2020, 1, category), category, 0.0);
        assert!(normalize_corporate_actions("test", &request, vec![zero_quantity]).is_err());

        let mut non_finite_price = source_action((2020, 1, category), category, 2.0);
        non_finite_price.xingquanjia = Some(f64::INFINITY);
        assert!(normalize_corporate_actions("test", &request, vec![non_finite_price]).is_err());
    }
}

#[test]
fn corporate_action_provider_response_echoes_exact_empty_request_coverage() {
    let request = CorporateActionRequest::new(instrument("600001"))
        .with_range(
            IsoDate::new("1900-01-01").unwrap(),
            IsoDate::new("1900-12-31").unwrap(),
        )
        .unwrap();
    let query = ScriptedBarsQuery::default();
    query.xdxr_responses.borrow_mut().push_back(Ok(Vec::new()));

    let response = corporate_actions_with(&query, "test", &request).unwrap();
    assert_eq!(response.coverage(), &request);
    assert_eq!(response.evidence().source_at(), None);
    assert_eq!(
        response.evidence().batch_id(),
        response.batch().provenance().batch_id().unwrap()
    );
    assert!(response.batch().records().is_empty());
    assert!(response.batch().quality().is_complete());
    assert_eq!(
        query.xdxr_calls.borrow().as_slice(),
        &[(1, "600001".to_owned())]
    );
}

#[test]
fn corporate_action_response_rejects_missing_provenance_batch_id() {
    let request = CorporateActionRequest::new(instrument("600001"));
    let provenance: magic_market_core::Provenance = serde_json::from_value(serde_json::json!({
        "source": "test",
        "source_at": null,
        "fetched_at": "2026-07-27T10:00:00+08:00",
        "batch_id": null
    }))
    .unwrap();
    let batch = DataBatch::<CorporateAction>::strict(Vec::new(), provenance);

    let error = corporate_action_response(&request, batch, IsoDate::new("2026-07-27").unwrap())
        .unwrap_err();

    assert!(matches!(
        error,
        TdxError::InvalidData(message)
            if message == "TDX corporate-action batch omitted its batch ID"
    ));
}

#[test]
fn corporate_action_response_propagates_provenance_source_time() {
    let request = CorporateActionRequest::new(instrument("600001"));
    let provenance = magic_market_core::Provenance::new("test", "2026-07-27T10:00:00+08:00")
        .unwrap()
        .with_source_at("2026-07-26")
        .unwrap();
    let batch_id = provenance.batch_id().unwrap().to_owned();
    let batch = DataBatch::<CorporateAction>::strict(Vec::new(), provenance);

    let response =
        corporate_action_response(&request, batch, IsoDate::new("2026-07-27").unwrap()).unwrap();

    assert_eq!(response.evidence().provider(), ProviderId::Tdx);
    assert_eq!(response.evidence().source_at(), Some("2026-07-26"));
    assert_eq!(
        response.evidence().observed_at(),
        "2026-07-27T10:00:00+08:00"
    );
    assert_eq!(response.evidence().batch_id(), batch_id);
}

#[test]
fn beijing_uses_the_live_verified_tdx_market_number() {
    let beijing = InstrumentId::new(Exchange::Beijing, "920001", AssetClass::Equity).unwrap();
    assert_eq!(market(&beijing).unwrap(), 2);
    assert_eq!(market(&instrument("600001")).unwrap(), 1);
    let shenzhen = InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity).unwrap();
    assert_eq!(market(&shenzhen).unwrap(), 0);
}

#[test]
fn rejects_beijing_security_metadata_before_transport() {
    let beijing = InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity).unwrap();
    let error = validate_security_metadata_request(&[beijing]).unwrap_err();
    assert!(matches!(error, TdxError::Unsupported(_)));
    assert!(error.to_string().contains("security-list"));
}

#[test]
fn normalizes_tdx_minute_rows_into_cumulative_chronological_points() {
    let records = vec![
        MinuteTimePrice {
            time: "09:32".into(),
            price: 15.6,
            avg_price: 15.5,
            vol: 20.0,
        },
        MinuteTimePrice {
            time: "09:31".into(),
            price: 15.4,
            avg_price: 15.4,
            vol: 10.0,
        },
    ];
    let batch =
        normalize_minute_records("test", &instrument("600396"), "2026-07-23", records).unwrap();
    assert_eq!(batch.records()[0].minute_at(), "2026-07-23 09:31");
    assert_eq!(batch.records()[0].cumulative_quantity().get(), 10.0);
    assert_eq!(batch.records()[1].cumulative_quantity().get(), 30.0);
    assert_eq!(
        batch.records()[1].source_at(),
        Some("2026-07-23T09:32:00+08:00")
    );
    assert!(batch.records()[1].cumulative_amount().is_none());
}

#[test]
fn paginates_and_normalizes_historical_trades() {
    let request = TradesRequest::new(instrument("600519"), 5)
        .unwrap()
        .with_date("2026-07-21")
        .unwrap();
    let mut calls = Vec::new();
    let batch = paginate_trades("test", &request, 2, |start, count| {
        calls.push((start, count));
        Ok((start..start + count)
            .map(|index| source_trade(u32::from(index), u32::from(index % 3)))
            .collect())
    })
    .unwrap();
    assert_eq!(calls, vec![(0, 2), (2, 2), (4, 1)]);
    assert_eq!(batch.records().len(), 5);
    assert_eq!(batch.records()[0].trade_at(), "2026-07-21 10:00:00");
    assert_eq!(batch.records()[0].source_at(), Some("2026-07-21 10:00:00"));
    assert_eq!(batch.records()[0].side(), TradeSide::Buy);
    assert_eq!(batch.records()[1].side(), TradeSide::Sell);
    assert_eq!(batch.records()[2].side(), TradeSide::Neutral);
}

#[test]
fn marks_unknown_trade_side_without_dropping_the_record() {
    let request = TradesRequest::new(instrument("600519"), 1).unwrap();
    let batch = normalize_trade_records("test", &request, vec![source_trade(0, 9)]).unwrap();
    assert_eq!(batch.records()[0].side(), TradeSide::Unknown(9));
    assert_eq!(batch.records()[0].status(), DataStatus::Unavailable);
    assert_eq!(batch.quality().issues().len(), 1);
}

#[test]
fn maps_every_core_bar_interval_and_accepts_requests_without_ranges() {
    let cases = [
        (BarInterval::Minute1, KLINE_1MIN),
        (BarInterval::Minute5, KLINE_5MIN),
        (BarInterval::Minute15, KLINE_15MIN),
        (BarInterval::Minute30, KLINE_30MIN),
        (BarInterval::Hour1, KLINE_1HOUR),
        (BarInterval::Day, KLINE_DAILY),
        (BarInterval::Week, KLINE_WEEKLY),
        (BarInterval::Month, KLINE_MONTHLY),
        (BarInterval::Year, KLINE_YEARLY),
    ];
    for (interval, expected) in cases {
        assert_eq!(category(interval).unwrap(), expected);
    }
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 1).unwrap();
    assert!(reject_unsupported_bar_range(&request).is_ok());
}

#[test]
fn normalized_daily_bar_preserves_units_and_complete_evidence() {
    let request = BarsRequest::new(instrument("600396"), BarInterval::Day, 1).unwrap();
    let batch = normalize_bars("tdx", &request, vec![source_bar()]).unwrap();
    let record = &batch.records()[0];
    let batch_id = batch.provenance().batch_id().unwrap();

    assert_eq!(batch.records().len(), 1);
    assert_eq!(batch.provenance().source(), "tdx");
    assert_eq!(batch.provenance().source_at(), Some("2026-07-23"));
    assert_eq!(record.instrument(), request.instrument());
    assert_eq!(record.interval(), BarInterval::Day);
    assert_eq!(record.bar_start(), "2026-07-23");
    assert_eq!(record.bar_end(), "2026-07-23");
    assert_eq!(record.open(), Price::new(10.0).unwrap());
    assert_eq!(record.high(), Price::new(12.0).unwrap());
    assert_eq!(record.low(), Price::new(9.0).unwrap());
    assert_eq!(record.close(), Price::new(11.0).unwrap());
    assert_eq!(record.volume(), Quantity::new(1.0).unwrap());
    assert_eq!(record.amount(), Some(Money::new(1_000.0).unwrap()));
    assert_eq!(record.adjustment(), Adjustment::Unadjusted);
    assert_eq!(record.source_at(), Some("2026-07-23"));
    assert_eq!(record.provider(), ProviderId::Tdx);
    assert_eq!(record.batch_id(), batch_id);
}

#[test]
fn normalized_intraday_bar_canonicalizes_only_core_bar_time() {
    let request = BarsRequest::new(instrument("600396"), BarInterval::Minute5, 1).unwrap();
    let mut source = source_bar();
    source.hour = 9;
    source.minute = 35;
    source.datetime = "2026-07-23 09:35".into();

    let batch = normalize_bars("tdx-smart", &request, vec![source]).unwrap();
    let record = &batch.records()[0];
    assert_eq!(record.bar_start(), "2026-07-23 09:35:00");
    assert_eq!(record.bar_end(), "2026-07-23 09:35:00");
    assert_eq!(record.source_at(), Some("2026-07-23 09:35"));
    assert_eq!(batch.provenance().source_at(), Some("2026-07-23 09:35"));
}

fn source_bar_at(day: u32, close: f64) -> SecurityBar {
    SecurityBar {
        open: close,
        close,
        high: close,
        low: close,
        vol: 100.0,
        amount: close * 10_000.0,
        year: 2026,
        month: 7,
        day,
        hour: 0,
        minute: 0,
        datetime: format!("2026-07-{day:02}"),
    }
}

#[test]
fn normalized_bar_batches_reject_incomplete_or_ambiguous_sequences() {
    let one = BarsRequest::new(instrument("600396"), BarInterval::Day, 1).unwrap();
    let two = BarsRequest::new(instrument("600396"), BarInterval::Day, 2).unwrap();

    assert!(matches!(
        normalize_bars("tdx", &one, Vec::new()),
        Err(TdxError::HistoricalBarCardinality {
            offset: 0,
            actual: 0,
            expected_page: 1,
            requested_total: 1,
        })
    ));
    assert!(normalize_bars("legacy-tdx", &one, vec![source_bar()]).is_err());
    assert!(matches!(
        normalize_bars("tdx", &two, vec![source_bar()]),
        Err(TdxError::HistoricalBarCardinality {
            offset: 0,
            actual: 1,
            expected_page: 2,
            requested_total: 2,
        })
    ));
    assert!(normalize_bars(
        "tdx",
        &one,
        vec![source_bar_at(22, 10.0), source_bar_at(23, 10.0)],
    )
    .is_err());
    assert!(normalize_bars(
        "tdx",
        &two,
        vec![source_bar_at(22, 10.0), source_bar_at(22, 10.0)],
    )
    .is_err());
    assert!(normalize_bars(
        "tdx",
        &two,
        vec![source_bar_at(23, 10.0), source_bar_at(22, 10.0)],
    )
    .is_err());

    let mut mismatched = source_bar();
    mismatched.day = 22;
    assert!(normalize_bars("tdx", &one, vec![mismatched]).is_err());

    let mut future = source_bar();
    future.year = 2099;
    future.datetime = "2099-07-23".into();
    assert!(normalize_bars("tdx", &one, vec![future]).is_err());

    let mut invalid_calendar = source_bar();
    invalid_calendar.month = 2;
    invalid_calendar.day = 30;
    invalid_calendar.datetime = "2026-02-30".into();
    assert!(normalize_bars("tdx", &one, vec![invalid_calendar]).is_err());

    let intraday = BarsRequest::new(instrument("600396"), BarInterval::Minute5, 1).unwrap();
    let mut invalid_intraday = source_bar();
    invalid_intraday.hour = 24;
    invalid_intraday.minute = 35;
    invalid_intraday.datetime = "2026-07-23 24:35".into();
    assert!(normalize_bars("tdx", &intraday, vec![invalid_intraday]).is_err());

    let mut invalid_daily = source_bar();
    invalid_daily.hour = 1;
    assert!(normalize_bars("tdx", &one, vec![invalid_daily]).is_err());
}

#[test]
fn normalized_bar_batches_reject_bad_values_and_admit_structurally_valid_large_moves() {
    let one = BarsRequest::new(instrument("600396"), BarInterval::Day, 1).unwrap();
    let two = BarsRequest::new(instrument("600396"), BarInterval::Day, 2).unwrap();

    for mutate in [
        |bar: &mut SecurityBar| bar.open = f64::NAN,
        |bar: &mut SecurityBar| bar.close = 0.0,
        |bar: &mut SecurityBar| bar.high = 9.0,
        |bar: &mut SecurityBar| bar.vol = -1.0,
        |bar: &mut SecurityBar| bar.amount = -1.0,
        |bar: &mut SecurityBar| bar.amount = 0.0,
    ] {
        let mut bar = source_bar();
        mutate(&mut bar);
        assert!(normalize_bars("tdx", &one, vec![bar]).is_err());
    }

    let positive_jump = normalize_bars(
        "tdx",
        &two,
        vec![source_bar_at(22, 10.0), source_bar_at(23, 13.0)],
    )
    .unwrap();
    let negative_jump = normalize_bars(
        "tdx",
        &two,
        vec![source_bar_at(22, 10.0), source_bar_at(23, 7.5)],
    )
    .unwrap();

    for (batch, expected_close) in [(positive_jump, 13.0), (negative_jump, 7.5)] {
        let batch_id = batch.provenance().batch_id().unwrap();
        assert_eq!(batch.provenance().source(), "tdx");
        assert_eq!(batch.provenance().source_at(), Some("2026-07-23"));
        assert_eq!(
            batch.records()[1].close(),
            Price::new(expected_close).unwrap()
        );
        assert_eq!(batch.records()[1].provider(), ProviderId::Tdx);
        assert_eq!(batch.records()[1].source_at(), Some("2026-07-23"));
        assert_eq!(batch.records()[1].batch_id(), batch_id);
    }
}

#[test]
fn date_and_session_helpers_reject_ambiguous_values() {
    assert_eq!(compact_date("2026-07-23").unwrap(), 20260723);
    assert_eq!(compact_date("20260723").unwrap(), 20260723);
    assert!(compact_date("2026/07/23").is_err());
    assert!(compact_date("2026-7-23").is_err());
    assert_eq!(display_date(20260723).unwrap(), "2026-07-23");
    assert!(display_date(u32::MAX).is_err());

    for value in ["09:31", "11:30", "13:01", "15:00"] {
        assert!(valid_tdx_minute(value));
    }
    for value in ["09:30", "11:31", "13:00", "15:01", "bad"] {
        assert!(!valid_tdx_minute(value));
    }
}

#[test]
fn normalized_current_session_gate_accepts_only_active_weekday_windows() {
    for unix_seconds in [
        1_784_856_600, // 2026-07-24 Friday 09:30:00 Asia/Shanghai
        1_784_863_800, // 2026-07-24 Friday 11:30:00 Asia/Shanghai
        1_784_869_200, // 2026-07-24 Friday 13:00:00 Asia/Shanghai
        1_784_876_400, // 2026-07-24 Friday 15:00:00 Asia/Shanghai
    ] {
        assert!(ensure_current_session_at(unix_seconds, "minute data").is_ok());
    }
    for unix_seconds in [
        1_784_856_599, // Friday 09:29:59
        1_784_863_801, // Friday 11:30:01
        1_784_944_800, // Saturday 10:00:00
    ] {
        assert!(matches!(
            ensure_current_session_at(unix_seconds, "minute data"),
            Err(TdxError::InvalidData(message))
                if message
                    == "TDX normalized current minute data is unavailable outside an active A-share weekday session"
        ));
    }
}

fn minute(time: &str, price: f64, volume: f64) -> MinuteTimePrice {
    MinuteTimePrice {
        time: time.into(),
        price,
        avg_price: price,
        vol: volume,
    }
}

#[test]
fn minute_normalization_rejects_empty_oversized_duplicate_and_bad_numeric_data() {
    let id = instrument("600396");
    assert!(normalize_minute_records("test", &id, "2026-07-23", Vec::new()).is_err());
    assert!(normalize_minute_records(
        "test",
        &id,
        "2026-07-23",
        vec![minute("09:31", 10.0, 1.0); 241],
    )
    .is_err());
    assert!(
        normalize_minute_records("test", &id, "2026-07-23", vec![minute("09:30", 10.0, 1.0)],)
            .is_err()
    );
    assert!(normalize_minute_records(
        "test",
        &id,
        "2026-07-23",
        vec![minute("09:31", 10.0, 1.0), minute("09:31", 10.0, 1.0)],
    )
    .is_err());
    for volume in [-1.0, f64::NAN] {
        assert!(normalize_minute_records(
            "test",
            &id,
            "2026-07-23",
            vec![minute("09:31", 10.0, volume)],
        )
        .is_err());
    }
    assert!(normalize_minute_records(
        "test",
        &id,
        "2026-07-23",
        vec![
            minute("09:31", 10.0, f64::MAX),
            minute("09:32", 10.0, f64::MAX),
        ],
    )
    .is_err());
    assert!(
        normalize_minute_records("test", &id, "2026-07-23", vec![minute("09:31", 0.0, 1.0)],)
            .is_err()
    );
}

#[test]
fn optional_quote_prices_and_quote_numeric_fields_are_validated() {
    assert!(optional_quote_price(0.0, "field").unwrap().is_none());
    assert_eq!(
        optional_quote_price(10.0, "field").unwrap(),
        Some(Price::new(10.0).unwrap())
    );
    assert!(optional_quote_price(-1.0, "field").is_err());
    assert!(optional_quote_price(f64::NAN, "field").is_err());

    let id = instrument("600396");
    assert!(normalize_quotes("test", &[], Vec::new()).is_err());

    let mut invalid = source_quote("600396", 0.0);
    assert!(normalize_quotes("test", std::slice::from_ref(&id), vec![invalid.clone()]).is_err());
    invalid.price = 10.0;
    invalid.last_close = -1.0;
    assert!(normalize_quotes("test", std::slice::from_ref(&id), vec![invalid.clone()]).is_err());
    invalid.last_close = 9.0;
    invalid.vol = -1.0;
    assert!(normalize_quotes("test", std::slice::from_ref(&id), vec![invalid.clone()]).is_err());
    invalid.vol = 1.0;
    invalid.amount = -1.0;
    assert!(normalize_quotes("test", std::slice::from_ref(&id), vec![invalid]).is_err());

    let ids = [instrument("600001"), instrument("600002")];
    assert!(normalize_quotes(
        "test",
        &ids,
        vec![source_quote("600001", 10.0), source_quote("600001", 11.0)],
    )
    .is_err());
}

#[test]
fn trade_dates_records_and_pages_fail_explicitly() {
    assert_eq!(tdx_trade_date("2026-07-23").unwrap(), 20260723);
    for value in ["20260723", "2026/07/23", "2026-0x-23"] {
        assert!(tdx_trade_date(value).is_err());
    }

    let request = TradesRequest::new(instrument("600519"), 2).unwrap();
    assert!(normalize_trade_records("test", &request, Vec::new()).is_err());
    let mut invalid = source_trade(0, 0);
    invalid.time.clear();
    assert!(normalize_trade_records("test", &request, vec![invalid]).is_err());
    let mut invalid = source_trade(0, 0);
    invalid.price = 0.0;
    assert!(normalize_trade_records("test", &request, vec![invalid]).is_err());
    let mut invalid = source_trade(0, 0);
    invalid.vol = -1.0;
    assert!(normalize_trade_records("test", &request, vec![invalid]).is_err());

    let oversized = paginate_trades("test", &request, 1, |_, _| Ok(vec![source_trade(0, 0); 2]));
    assert!(oversized.is_err());
    let empty = paginate_trades("test", &request, 2, |_, _| Ok(Vec::new()));
    assert!(empty.is_err());
    let short = paginate_trades("test", &request, 2, |_, _| Ok(vec![source_trade(0, 0)])).unwrap();
    assert_eq!(short.records().len(), 1);
}

#[test]
fn board_and_st_classification_covers_every_supported_shape() {
    assert_eq!(board(&instrument("689001")), Board::Star);
    let chinext = InstrumentId::new(Exchange::Shenzhen, "301001", AssetClass::Equity).unwrap();
    assert_eq!(board(&chinext), Board::ChiNext);
    assert_eq!(board(&instrument("600396")), Board::Main);
    let beijing = InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity).unwrap();
    assert_eq!(board(&beijing), Board::Beijing);
    let fund = InstrumentId::new(Exchange::Shanghai, "510050", AssetClass::Fund).unwrap();
    assert_eq!(board(&fund), Board::Unknown);

    assert_eq!(st_flag(""), None);
    for name in ["ST示例", "*ST示例", "S*ST示例", "SST示例"] {
        assert_eq!(st_flag(name), Some(true));
    }
    assert_eq!(st_flag("普通公司"), Some(false));
}

fn security_info(code: &str, name: &str) -> SecurityInfo {
    SecurityInfo {
        code: code.into(),
        volunit: 100,
        decimal_point: 2,
        name: name.into(),
        pre_close: 10.0,
    }
}

#[test]
fn security_list_fetching_and_metadata_normalization_reject_bad_cardinality() {
    assert!(fetch_security_records(&[], |_| Ok(0), |_, _| Ok(Vec::new())).is_err());
    let duplicate = [instrument("600396"), instrument("600396")];
    assert!(fetch_security_records(&duplicate, |_| Ok(1), |_, _| Ok(Vec::new())).is_err());

    let requested = [instrument("600396")];
    let records = fetch_security_records(
        &requested,
        |_| Ok(2),
        |_, start| {
            assert_eq!(start, 0);
            Ok(vec![
                security_info("600396", "示例"),
                security_info("600397", "其他"),
            ])
        },
    )
    .unwrap();
    assert_eq!(records.len(), 1);
    assert!(fetch_security_records(&requested, |_| Ok(1), |_, _| Ok(Vec::new())).is_err());
    assert!(fetch_security_records(
        &requested,
        |_| Ok(1),
        |_, _| Ok(vec![
            security_info("600396", "示例"),
            security_info("600397", "其他")
        ]),
    )
    .is_err());

    assert!(validate_security_metadata_request(&requested).is_ok());
    assert!(normalize_security_metadata(
        "test",
        &requested,
        vec![
            (1, security_info("600396", "示例")),
            (1, security_info("600396", "重复")),
        ],
    )
    .is_err());
    assert!(normalize_security_metadata("test", &requested, Vec::new()).is_err());
    assert!(normalize_security_metadata(
        "test",
        &requested,
        vec![
            (1, security_info("600396", "示例")),
            (1, security_info("600397", "额外")),
        ],
    )
    .is_err());

    let fund = InstrumentId::new(Exchange::Shanghai, "510050", AssetClass::Fund).unwrap();
    let batch = normalize_security_metadata(
        "test",
        std::slice::from_ref(&fund),
        vec![(1, security_info("510050", "   "))],
    )
    .unwrap();
    assert_eq!(batch.records()[0].board(), Some(Board::Unknown));
    assert_eq!(batch.records()[0].is_st(), None);
    assert!(batch.records()[0].name().is_none());
}

#[test]
fn order_book_helpers_validate_levels_depth_and_request_identity() {
    assert!(book_level(f64::NAN, 1.0).is_err());
    assert!(book_level(1.0, f64::INFINITY).is_err());
    let complete = book_level(10.0, 2.0).unwrap();
    assert_eq!(complete.price(), Some(Price::new(10.0).unwrap()));
    assert_eq!(complete.quantity(), Some(Quantity::new(2.0).unwrap()));

    let unavailable = std::array::from_fn(|_| BookLevel::unavailable());
    assert_eq!(book_depth(&unavailable).unwrap(), None);
    let filled = std::array::from_fn(|_| book_level(10.0, 2.0).unwrap());
    assert_eq!(
        book_depth(&filled).unwrap(),
        Some(Quantity::new(10.0).unwrap())
    );
    let overflowing = std::array::from_fn(|_| book_level(10.0, f64::MAX).unwrap());
    assert!(book_depth(&overflowing).is_err());

    let ids = [instrument("600001"), instrument("600002")];
    assert_eq!(
        order_book_pairs(&ids, "test").unwrap(),
        vec![(1, "600001"), (1, "600002")]
    );

    let id = instrument("600396");
    let mut incomplete = source_quote("600396", 10.0);
    incomplete.bid5 = 0.0;
    let batch =
        normalize_order_books("test", "test", std::slice::from_ref(&id), vec![incomplete]).unwrap();
    assert!(!batch.quality().is_complete());
}

#[test]
fn disconnected_clients_still_execute_all_deterministic_preflight_failures() {
    let client = TdxHqClient::new();
    client.set_auto_retry(false);
    let ranged = BarsRequest::new(instrument("600396"), BarInterval::Day, 1)
        .unwrap()
        .with_range("2026-07-01", "2026-07-23")
        .unwrap();
    assert!(matches!(
        <TdxHqClient as HistoricalBars>::historical_bars(&client, &ranged),
        Err(TdxError::Unsupported(_))
    ));
    assert!(<TdxHqClient as OrderBooks>::order_books(&client, &[]).is_err());

    let beijing = InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity).unwrap();
    assert!(matches!(
        <TdxHqClient as SecurityMetadataProvider>::security_metadata(&client, &[beijing]),
        Err(TdxError::Unsupported(_))
    ));

    assert!(matches!(
        <TdxHqClient as MoneyFlows>::money_flows(&client, &[]),
        Err(TdxError::Unsupported(_))
    ));
    assert!(matches!(
        <TdxHqClient as Auctions>::auction_snapshots(&client, &[]),
        Err(TdxError::Unsupported(_))
    ));

    let smart = crate::TdxSmartClient::new();
    smart.inner().set_auto_retry(false);
    assert!(matches!(
        <crate::TdxSmartClient as HistoricalBars>::historical_bars(&smart, &ranged),
        Err(TdxError::Unsupported(_))
    ));
    assert!(<crate::TdxSmartClient as OrderBooks>::order_books(&smart, &[]).is_err());
    assert!(matches!(
        <crate::TdxSmartClient as MoneyFlows>::money_flows(&smart, &[]),
        Err(TdxError::Unsupported(_))
    ));
    assert!(matches!(
        <crate::TdxSmartClient as Auctions>::auction_snapshots(&smart, &[]),
        Err(TdxError::Unsupported(_))
    ));

    let direct = crate::TdxDirectClient::new("127.0.0.1", 9, 0.001);
    assert!(matches!(
        <crate::TdxDirectClient as HistoricalBars>::historical_bars(&direct, &ranged),
        Err(TdxError::Unsupported(_))
    ));
}

fn assert_blocking_query_rejects_block_codes(query: &impl BlockingTdxQuery) {
    assert!(query
        .security_bars(KLINE_DAILY, 1, "880001", 0, 1, 0)
        .is_err());
    assert!(query.security_quotes(&[(1, "880001")]).is_err());
    let _ = query.minute_time_data(1, "880001");
    let _ = query.history_minute_time_data(1, "880001", 20260723);
    let _ = query.transaction_data(1, "880001", 0, 1);
    let _ = query.history_transaction_data(1, "880001", 0, 1, 20260723);
    let _ = query.security_count(9);
    let _ = query.security_list(9, 0);
    let _ = query.finance_info(1, "880001");
    let _ = query.xdxr_info(1, "880001");
}

#[test]
fn concrete_blocking_query_delegates_every_family_and_preserves_preflight_errors() {
    let client = TdxHqClient::new();
    client.set_auto_retry(false);
    assert_blocking_query_rejects_block_codes(&client);

    let smart = crate::TdxSmartClient::new();
    smart.inner().set_auto_retry(false);
    assert_blocking_query_rejects_block_codes(&smart);

    let direct = crate::TdxDirectClient::new("127.0.0.1", 9, 0.001);
    assert_blocking_query_rejects_block_codes(&direct);
}

#[test]
fn every_blocking_provider_facade_returns_real_batches_or_explicit_errors() {
    fn assert_real_or_explicit<T>(result: Result<DataBatch<T>, TdxError>, expected_source: &str) {
        match result {
            Ok(batch) => {
                assert!(!batch.records().is_empty());
                assert_eq!(batch.provenance().source(), expected_source);
            }
            Err(error) => assert!(!error.to_string().trim().is_empty()),
        }
    }

    let id = instrument("600396");
    let bars = BarsRequest::new(id.clone(), BarInterval::Day, 1).unwrap();
    let minute = MinuteDataRequest::new(id.clone());
    let trades = TradesRequest::new(id.clone(), 1).unwrap();

    let client = TdxHqClient::new();
    client.set_auto_retry(false);
    assert_real_or_explicit(
        <TdxHqClient as HistoricalBars>::historical_bars(&client, &bars),
        "tdx",
    );
    assert_real_or_explicit(
        <TdxHqClient as MinuteData>::minute_data(&client, &minute),
        "tdx",
    );
    assert_real_or_explicit(
        <TdxHqClient as RealtimeQuotes>::realtime_quotes(&client, std::slice::from_ref(&id)),
        "tdx",
    );
    assert_real_or_explicit(
        <TdxHqClient as Trades>::trades(&client, &trades),
        "tdx-current",
    );
    assert_real_or_explicit(
        <TdxHqClient as SecurityMetadataProvider>::security_metadata(
            &client,
            std::slice::from_ref(&id),
        ),
        "tdx",
    );
    assert_real_or_explicit(
        <TdxHqClient as OrderBooks>::order_books(&client, std::slice::from_ref(&id)),
        "tdx",
    );

    let direct = crate::TdxDirectClient::new("127.0.0.1", 9, 0.001);
    assert!(<crate::TdxDirectClient as HistoricalBars>::historical_bars(&direct, &bars).is_err());
    assert!(<crate::TdxDirectClient as RealtimeQuotes>::realtime_quotes(
        &direct,
        std::slice::from_ref(&id)
    )
    .is_err());
    assert!(<crate::TdxDirectClient as Trades>::trades(&direct, &trades).is_err());
}

#[tokio::test]
async fn async_client_rejects_unsupported_ranges_before_transport() {
    let client = crate::AsyncTdxHqClient::new();
    let ranged = BarsRequest::new(instrument("600396"), BarInterval::Day, 1)
        .unwrap()
        .with_range("2026-07-01", "2026-07-23")
        .unwrap();
    assert!(matches!(
        <crate::AsyncTdxHqClient as AsyncHistoricalBars>::historical_bars_async(&client, &ranged)
            .await,
        Err(TdxError::Unsupported(_))
    ));
}

#[tokio::test]
async fn disconnected_async_client_preserves_transport_failures_for_each_mapping() {
    let client = crate::AsyncTdxHqClient::new();
    let id = instrument("600396");
    let bars = BarsRequest::new(id.clone(), BarInterval::Day, 1).unwrap();
    assert!(
        <crate::AsyncTdxHqClient as AsyncHistoricalBars>::historical_bars_async(&client, &bars)
            .await
            .is_err()
    );
    assert!(
        <crate::AsyncTdxHqClient as AsyncRealtimeQuotes>::realtime_quotes_async(
            &client,
            std::slice::from_ref(&id),
        )
        .await
        .is_err()
    );
    let current = TradesRequest::new(id.clone(), 1).unwrap();
    assert!(
        <crate::AsyncTdxHqClient as AsyncTrades>::trades_async(&client, &current)
            .await
            .is_err()
    );
    let historical = TradesRequest::new(id, 1)
        .unwrap()
        .with_date("2026-07-23")
        .unwrap();
    assert!(
        <crate::AsyncTdxHqClient as AsyncTrades>::trades_async(&client, &historical)
            .await
            .is_err()
    );
}
