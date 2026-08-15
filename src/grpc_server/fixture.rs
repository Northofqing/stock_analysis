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
        selected_provider: "fixture".to_string(),
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
                r#"[{"code":"600519","name":"贵州茅台","price":1500.0,"change_pct":2.34,"volume":12345,"amount":1.85e9}]"#,
            )],
        )),
        Operation::HistoricalBars => Some(resp(
            "fixture-hb",
            vec![payload(
                r#"[{"code":"600519","date":"2026-08-13","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":12345}]"#,
            )],
        )),
        Operation::MinuteData => Some(resp(
            "fixture-md",
            vec![payload(
                r#"[{"code":"600519","time":"09:35","open":1490.0,"high":1505.0,"low":1488.0,"close":1500.0,"volume":1200}]"#,
            )],
        )),
        Operation::Announcements => Some(resp(
            "fixture-ann",
            vec![payload(
                r#"[{"code":"600519","title":"贵州茅台:关于2026年中期分红的公告","published_at":"2026-08-13T09:00:00+08:00","url":"https://example.com/a1"}]"#,
            )],
        )),
        Operation::GlobalNews => Some(resp(
            "fixture-news",
            vec![payload(
                r#"[{"title":"央行开展逆回购操作","source":"fixture-news","published_at":"2026-08-13T08:30:00+08:00","url":"https://example.com/n1"}]"#,
            )],
        )),
        Operation::SecurityMetadata => Some(resp(
            "fixture-sec",
            vec![payload(
                r#"[{"code":"600519","name":"贵州茅台","market":"SH","industry":"白酒","list_date":"2001-08-27"}]"#,
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
                r#"[{"code":"600519","title":"贵州茅台:关于2026年半年度报告的公告","summary":"公司发布2026年半年度报告","url":"https://example.com/in1","source_name":"新浪财经","published_at":"2026-08-15T09:00:00+08:00"}]"#,
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
                r#"{"records":[{"instrument":"SH600519","code":"600519","requested_at":"2026-08-15T09:35:00+08:00","source_at":"2026-08-15T09:35:00+08:00","observed_at":"2026-08-15T09:35:00+08:00","batch_id":"fixture-t0","quote":{"price":1500.0,"last_close":1490.0,"open":1495.0,"high":1505.0,"low":1490.0,"volume":1e6,"amount":1.5e9,"bids":[],"asks":[]},"settled_daily":[],"completed_five_minute":[],"intraday_average_price":1498.0}],"rejections":[]}"#,
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
        _ => None,
    }
}
