//! fixture 数据 (离线确定性测试): GRPC_GATEWAY_TEST_FIXTURE=1 时 handler 返回这些数据。
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState, CanonicalPayload, Operation, QueryResponse,
};

pub fn fixture_response(op: Operation, schema: &str, version: u32) -> Option<QueryResponse> {
    let payload = |data: &str| CanonicalPayload {
        schema: schema.to_string(),
        schema_version: version,
        content_type: "application/json; charset=utf-8".to_string(),
        data: data.as_bytes().to_vec(),
    };
    let resp = |request_id: &str, records: Vec<CanonicalPayload>| QueryResponse {
        request_id: request_id.to_string(),
        operation: op as i32,
        admission: AdmissionState::Admitted as i32,
        // 桥路径 (grpc_source::convert::parse_provider) 只认 ProviderId Debug 名;
        // "fixture" 不在表内 → 会 invalid_evidence。fixture 是测试路径, 用 Tdx 代表。
        selected_provider: "Tdx".to_string(),
        batch_id: "fixture-b1".to_string(),
        complete: true,
        observed_at: "2026-08-13T10:00:00+08:00".to_string(),
        source_at: "2026-08-13T10:00:00+08:00".to_string(),
        records,
        source: "fixture".to_string(),
    };
    match op {
        Operation::RealtimeQuotes => Some(resp(
            "fixture-rq",
            vec![payload(
                r#"[{"code":"600519","name":"贵州茅台","price":1500.0,"change_pct":2.34,"previous_close":1490.0}]"#,
            )],
        )),
        Operation::HistoricalBars => Some(resp(
            "fixture-hb",
            vec![payload(
                r#"[{"code":"600519","date":"2026-08-13","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":12345.0,"amount":1.85e9,"pct_chg":2.34,"settled":true}]"#,
            )],
        )),
        Operation::MinuteData => Some(resp(
            "fixture-md",
            vec![payload(
                r#"[{"code":"600519","minute_at":"2026-08-13T09:35:00+08:00","price":1500.0,"cumulative_quantity":12345.0,"cumulative_amount":1.85e9,"source_at":"2026-08-13T09:35:00+08:00"}]"#,
            )],
        )),
        Operation::Announcements => Some(resp(
            "fixture-ann",
            vec![payload(
                r#"[{"announcement_id":"fixture-a1","code":"600519","category":"分红派息","title":"贵州茅台:关于2026年中期分红的公告","published_at":"2026-08-13T09:00:00+08:00","url":"https://example.com/a1"}]"#,
            )],
        )),
        Operation::GlobalNews => Some(resp(
            "fixture-news",
            vec![payload(
                r#"[{"item_id":"fixture-n1","title":"央行开展逆回购操作","summary":"央行今日开展逆回购操作","content":"为维护银行体系流动性合理充裕,央行今日开展逆回购操作。","publisher":"央行","url":"https://example.com/n1","published_at":"2026-08-13T08:30:00+08:00","instruments":[],"topics":["流动性"],"language":"zh"}]"#,
            )],
        )),
        Operation::EconomicCalendar => Some(resp(
            "fixture-ec",
            vec![payload(
                r#"[{"event_id":"fixture-e1","indicator_id":123,"country":"CN","name":"中国 CPI 同比","period":"2026-07","scheduled_at":"2026-08-09T09:30:00+08:00","released_at":"2026-08-09T09:30:00+08:00","previous":0.6,"consensus":0.7,"actual":0.8,"revised":null,"unit":"%","importance":3,"impact":"利好"}]"#,
            )],
        )),
        Operation::OrderBooks => Some(resp(
            "fixture-ob",
            vec![payload(
                r#"[{"code":"600519","bids":[{"price":1499.0,"quantity":100.0},{"price":1498.0,"quantity":200.0}],"asks":[{"price":1501.0,"quantity":150.0},{"price":1502.0,"quantity":300.0}],"total_bid_quantity":300.0,"total_ask_quantity":450.0,"source_at":"2026-08-13T10:00:00+08:00"}]"#,
            )],
        )),
        Operation::MoneyFlows => Some(resp(
            "fixture-mf",
            vec![payload(
                r#"[{"code":"600519","main_net":5e7,"super_large_net":4e7,"large_net":1e7,"medium_net":-2e6,"small_net":-3e6,"source_at":"2026-08-13T10:00:00+08:00"}]"#,
            )],
        )),
        Operation::ForeignExchange => Some(resp(
            "fixture-fx",
            vec![payload(
                r#"[{"pair":"USDCNY","name":"美元/人民币","rate":7.15,"change":-0.01,"change_percent":-0.14,"source_at":"2026-08-13T10:00:00+08:00"}]"#,
            )],
        )),
        Operation::FuturesDelivery => Some(resp(
            "fixture-fd",
            vec![payload(
                r#"[{"contract_code":"IF2608","product_code":"IF","last_trading_date":"2026-08-21","delivery_date":"2026-08-21","notice_url":"https://example.com/fd1"}]"#,
            )],
        )),
        Operation::SecurityMetadata => Some(resp(
            "fixture-sec",
            vec![payload(
                r#"[{"code":"600519","name":"贵州茅台","board":"Main","is_st":false,"listed_on":"2001-08-27","price_limit_percent":10.0,"source_at":"2026-08-13T10:00:00+08:00"}]"#,
            )],
        )),
        // M1 扩展 (P4): 6 个新 op fixture (字段形状与 delegate 视图一致;
        // fixture 仅 GRPC_GATEWAY_TEST_FIXTURE=1 时生效, 属测试路径)。
        Operation::IndexQuotes => Some(resp(
            "fixture-idx",
            vec![payload(
                r#"[{"code":"sh000001","name":"上证指数","current":3500.5,"change":12.3,"change_percent":0.35,"open":3490.0,"high":3510.0,"low":3485.0,"previous_close":3488.2,"volume":3.2e9,"amount":4.5e11,"source_at":"2026-08-15T10:00:00+08:00"}]"#,
            )],
        )),
        Operation::InstrumentNews => Some(resp(
            "fixture-in",
            vec![payload(
                r#"[{"code":"600519","title":"贵州茅台:关于2026年半年度报告的公告","summary":"公司发布2026年半年度报告","url":"https://example.com/in1","source":"Sina","source_name":"新浪财经","category":"个股新闻","external_id":"in1","published_at":"2026-08-15T09:00:00+08:00","fetched_at":"2026-08-15T09:00:01+08:00","content_hash":"fixture-hash-in1"}]"#,
            )],
        )),
        Operation::IntradayShape => Some(resp(
            "fixture-shape",
            vec![payload(
                r#"[{"date":"2026-08-15","pre_close":1500.0,"open_pct":0.2,"high_pct":2.1,"low_pct":-0.8,"close_pct":1.3,"amplitude":2.9,"tail_30m_pct":0.5,"shape_label":"稳步推高"}]"#,
            )],
        )),
        Operation::T0Evidence => Some(resp(
            "fixture-t0",
            vec![payload(
                r#"{"records":[{"instrument":"SH600519","code":"600519","requested_at":"2026-08-15T09:35:00+08:00","source_at":"2026-08-15T09:35:00+08:00","observed_at":"2026-08-15T09:35:00+08:00","batch_id":"fixture-t0","quote":{"price":1500.0,"last_close":1490.0,"open":1495.0,"high":1505.0,"low":1490.0,"volume":1e6,"amount":1.5e9,"bids":[{"price":1499.0,"volume":100.0},{"price":1498.0,"volume":200.0},{"price":1497.0,"volume":300.0},{"price":1496.0,"volume":400.0},{"price":1495.0,"volume":500.0}],"asks":[{"price":1501.0,"volume":100.0},{"price":1502.0,"volume":200.0},{"price":1503.0,"volume":300.0},{"price":1504.0,"volume":400.0},{"price":1505.0,"volume":500.0}]},"settled_daily":[],"completed_five_minute":[],"intraday_average_price":1498.0}],"rejections":[]}"#,
            )],
        )),
        Operation::OutcomeDailyBars => Some(resp(
            "fixture-ob",
            vec![payload(
                r#"[{"market_date":"2026-08-14","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":123456,"amount":1.85e9}]"#,
            )],
        )),
        Operation::UpperLimitPoolReview => Some(resp(
            "fixture-ulp",
            vec![payload(
                r#"[{"code":"600519","trading_date":"2026-08-15","theme":"白酒","streak":2}]"#,
            )],
        )),
        // M3 扩展 (P4): 剩余网关钩子的 op fixture (字段形状与 convert.rs 解析一致)。
        Operation::DragonTiger => Some(resp(
            "fixture-dt",
            vec![payload(
                r#"[{"exchange":"Shanghai","code":"600519","ranking_net_amount_yuan":1.5e8,"disclosures":[{"entry_id":"e1","trade_id":"t1","reason":"日涨幅偏离值达7%","buy_amount_yuan":8e7,"sell_amount_yuan":3e7,"net_amount_yuan":5e7,"turnover_rate_pct":1.2,"seats":[{"side":"Buy","rank":1,"seat_name":"机构专用","amount_yuan":4e7,"buy_amount_yuan":4e7,"sell_amount_yuan":0.0,"net_amount_yuan":4e7}]}]}]"#,
            )],
        )),
        Operation::BlockTrades => Some(resp(
            "fixture-bt",
            vec![payload(
                r#"[{"code":"600519","traded_at":"2026-08-15T15:05:00+08:00","price":1490.0,"close_price":1500.0,"premium_ratio":-0.67,"volume":1e5,"amount":1.49e8,"buyer":"机构专用","seller":"中信证券"}]"#,
            )],
        )),
        Operation::Consensus => Some(resp(
            "fixture-cons",
            vec![payload(
                r#"[{"report_count":12,"broker_count":8,"eps_this_year_avg":42.5,"eps_next_year_avg":48.0,"eps_next2_year_avg":55.0,"rating_distribution":{"买入":5,"增持":3}}]"#,
            )],
        )),
        Operation::BoardDirectory => Some(resp(
            "fixture-bdir",
            vec![payload(
                r#"[{"code":"BK0475","name":"白酒","kind":"Concept","member_count":32}]"#,
            )],
        )),
        Operation::BoardConstituents => Some(resp(
            "fixture-bcons",
            vec![payload(
                r#"[{"instrument_code":"600519","board_code":"BK0475","board_name":"白酒","kind":"Concept"}]"#,
            )],
        )),
        Operation::BoardFlows => Some(resp(
            "fixture-bf",
            vec![payload(
                r#"[{"code":"BK0475","name":"白酒","kind":"Concept","rank":1,"return_pct":2.1,"main_net_yuan":3.2e8,"leader_code":"600519","leader_name":"贵州茅台"}]"#,
            )],
        )),
        Operation::MarketRankings => Some(resp(
            "fixture-mr",
            vec![payload(
                r#"[{"code":"BK0475","name":"白酒","change_pct":2.1,"main_inflow":3.2e8,"leader_name":"贵州茅台","vol_ratio":1.5,"turnover":3.2,"day1_ratio":1.1,"day5_ratio":2.3}]"#,
            )],
        )),
        Operation::ConceptHits => Some(resp(
            "fixture-ch",
            vec![payload(
                r#"[{"code":"BK0475","name":"白酒","change_pct":2.1,"main_inflow":3.2e8,"leader_name":"贵州茅台","vol_ratio":1.5,"turnover":3.2,"day1_ratio":1.1,"day5_ratio":2.3}]"#,
            )],
        )),
        Operation::MarketStatistics => Some(resp(
            "fixture-ms",
            vec![payload(
                r#"[{"code":"600519","turnover_rate":0.42,"trailing_pe":28.5,"static_pe":26.0,"pb":9.2,"total_market_cap":1.88e12,"floating_market_cap":1.88e12,"upper_limit":1650.0,"lower_limit":1350.0,"volume_ratio":1.1}]"#,
            )],
        )),
        Operation::TechnicalBars => Some(resp(
            "fixture-tb",
            vec![payload(
                r#"[{"open":1490.0,"close":1500.0,"high":1505.0,"low":1488.0,"vol":1234567.0,"amount":1.85e9,"at":"2026-08-15T10:30:00+08:00"}]"#,
            )],
        )),
        // P4 M4b: 批次 1A 新增 fixture (视图字段与 delegate fetch_* 一一对应)。
        Operation::ResearchReports => Some(resp(
            "fixture-rr",
            vec![payload(
                r#"[{"code":"600519","report_id":"fixture-r1","title":"贵州茅台:2026年中报点评","organization":"国泰君安","rating":"增持","published_at":"2026-08-13T09:00:00+08:00","canonical_url":"https://example.com/r1","target_price_upper":1600.0,"target_price_lower":1500.0}]"#,
            )],
        )),
        Operation::NorthboundDaily => Some(resp(
            "fixture-nbd",
            vec![payload(
                r#"[{"trading_date":"2026-08-13","channel":"Shanghai","total_turnover":5.2e10,"total_trade_count":4200.0,"quota_balance":8e9,"etf_turnover":1.2e9,"top_turnover":[{"rank":1,"code":"600519","name":"贵州茅台","total_turnover":1.8e9}]}]"#,
            )],
        )),
        Operation::FinancialStatements => Some(resp(
            "fixture-fs",
            vec![payload(
                r#"[{"instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},"kind":"Balance","report_period":"2026-06-30","announced_on":"2026-08-15","currency":"CNY","lines":[{"key":"total_assets","source_label":"总资产","value":3.2e11,"unit":"元"}],"evidence":{"provider":"Tdx","source_at":"2026-08-13T10:00:00+08:00","observed_at":"2026-08-13T10:00:00+08:00","batch_id":"fixture-b1"}}]"#,
            )],
        )),
        Operation::FundFlowSeries => Some(resp(
            "fixture-ffs",
            vec![payload(
                r#"[{"code":"600519","interval":"Day1","period_at":"2026-08-13","main_net":5e7,"main_ratio_percent":12.3,"super_large_net":4e7,"large_net":1e7,"medium_net":-2e6,"small_net":-3e6}]"#,
            )],
        )),
        Operation::ProviderTopNRankings => Some(QueryResponse {
            // 该 op 客户端双路 audit 锚定 Eastmoney (request evidence 语义) →
            // selected_provider 必须 = Eastmoney, 否则 audit_gateway_result 判
            // "batch provider differs from the admitted provider"。
            request_id: "fixture-ptr".to_string(),
            operation: Operation::ProviderTopNRankings as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "Eastmoney".to_string(),
            batch_id: "fixture-b1".to_string(),
            complete: true,
            observed_at: "2026-08-13T10:00:00+08:00".to_string(),
            source_at: "2026-08-13T10:00:00+08:00".to_string(),
            records: vec![payload(
                r#"[{"metric":"VolumeRatio","ordinal":1,"code":"600519","label":"贵州茅台","value":3.2,"unit":"Multiple","trading_date":"2026-08-13","filter_identity":"volume_ratio_top20","provider_declared_total":20,"inspected_row_count":20},{"metric":"MainNetInflow","ordinal":1,"code":"600519","label":"贵州茅台","value":1.5e9,"unit":"Yuan","trading_date":"2026-08-13","filter_identity":"main_net_inflow_top20","provider_declared_total":20,"inspected_row_count":20}]"#,
            )],
            source: "fixture".to_string(),
        }),
        _ => None,
    }
}
