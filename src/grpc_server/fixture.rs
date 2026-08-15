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
        _ => None,
    }
}
