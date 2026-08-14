//! QueryRequest/QueryResponse 信封 (合同 §5/§6)。
//! request_id: 调用方生成的非空唯一请求 ID; 同一业务重试保留原 ID。
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState, CanonicalPayload, QueryRequest, QueryResponse, RequestContext,
};
use crate::grpc_client::pb::magic::market::v1::Operation;
use crate::grpc_contract::schema::schema_for;
use std::sync::atomic::{AtomicU64, Ordering};

const PROTOCOL_VERSION: u32 = 1;
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn new_request_id() -> String {
    let ms = chrono::Utc::now().timestamp_millis();
    let pid = std::process::id();
    let n = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{ms}-{pid}-{n}")
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EnvelopeError {
    #[error("request_id 不匹配: 期望 {0} 实际 {1}")]
    RequestIdMismatch(String, String),
    #[error("响应缺 request_id")]
    MissingRequestId,
    #[error("schema 未冻结, 无法构造请求")]
    SchemaNotFrozen,
    #[error("payload 序列化失败: {0}")]
    Serialize(String),
}

pub fn build_query_request(
    op: Operation,
    payload: serde_json::Value,
) -> Result<QueryRequest, EnvelopeError> {
    let schema = schema_for(op).ok_or(EnvelopeError::SchemaNotFrozen)?;
    let data = serde_json::to_vec(&payload)
        .map_err(|e| EnvelopeError::Serialize(e.to_string()))?;
    Ok(QueryRequest {
        context: Some(RequestContext {
            protocol_version: PROTOCOL_VERSION,
            request_id: new_request_id(),
        }),
        // 合同 §5: 普通调用保持 preferred_provider 为空, 由服务端 Composition 选择。
        preferred_provider: String::new(),
        payload: Some(CanonicalPayload {
            schema: schema.schema_name.to_string(),
            schema_version: schema.schema_version,
            content_type: "application/json; charset=utf-8".to_string(),
            data,
        }),
    })
}

#[derive(Debug)] // unwrap_err 需要 Ok 类型 Debug
pub struct QueryResult {
    pub admission: AdmissionState,
    pub selected_provider: String,
    pub batch_id: String,
    pub complete: bool,
    pub observed_at: String,
    pub source_at: String,
    pub records: Vec<CanonicalPayload>,
}

pub fn parse_query_response(
    expected_request_id: &str,
    resp: QueryResponse,
) -> Result<QueryResult, EnvelopeError> {
    if resp.request_id.is_empty() {
        return Err(EnvelopeError::MissingRequestId);
    }
    if resp.request_id != expected_request_id {
        return Err(EnvelopeError::RequestIdMismatch(
            expected_request_id.to_string(),
            resp.request_id,
        ));
    }
    Ok(QueryResult {
        // prost 0.14 from_i32 deprecated → try_from (语义等价, 未知值回落 Unspecified)。
        admission: AdmissionState::try_from(resp.admission)
            .unwrap_or(AdmissionState::Unspecified),
        selected_provider: resp.selected_provider,
        batch_id: resp.batch_id,
        complete: resp.complete,
        observed_at: resp.observed_at,
        source_at: resp.source_at,
        records: resp.records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_unique_and_nonempty() {
        let a = new_request_id();
        let b = new_request_id();
        assert!(!a.is_empty() && !b.is_empty());
        assert_ne!(a, b);
    }

    #[test]
    fn builds_query_request_with_frozen_schema() {
        let req = build_query_request(
            Operation::RealtimeQuotes,
            serde_json::json!({"codes": ["600519"]}),
        )
        .unwrap();
        let ctx = req.context.unwrap();
        assert_eq!(ctx.protocol_version, 1);
        assert!(!ctx.request_id.is_empty());
        assert_eq!(req.preferred_provider, "");
        let payload = req.payload.unwrap();
        assert_eq!(payload.schema, "market.realtime_quotes");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
    }

    #[test]
    fn rejects_query_request_for_unfrozen_schema() {
        let err = build_query_request(
            Operation::OptionData, // 不在 SCHEMAS (未实现 op)
            serde_json::json!({}),
        )
        .unwrap_err();
        assert_eq!(err, EnvelopeError::SchemaNotFrozen);
    }

    #[test]
    fn parses_query_response_with_matching_request_id() {
        let resp = QueryResponse {
            request_id: "r-1".to_string(),
            operation: 3,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "t1".to_string(),
            source_at: "t2".to_string(),
            records: vec![],
        };
        let result = parse_query_response("r-1", resp).unwrap();
        assert_eq!(result.admission, AdmissionState::Admitted);
        assert!(result.complete);
        assert_eq!(result.selected_provider, "tdx-dev");
    }

    #[test]
    fn rejects_mismatched_request_id() {
        let resp = QueryResponse {
            request_id: "other".to_string(),
            operation: 3,
            admission: 1,
            selected_provider: "".to_string(),
            batch_id: "".to_string(),
            complete: false,
            observed_at: "".to_string(),
            source_at: "".to_string(),
            records: vec![],
        };
        let err = parse_query_response("r-1", resp).unwrap_err();
        assert!(matches!(err, EnvelopeError::RequestIdMismatch(_, _)));
    }

    #[test]
    fn rejects_missing_request_id() {
        let resp = QueryResponse {
            request_id: String::new(),
            operation: 3,
            admission: 1,
            selected_provider: "".to_string(),
            batch_id: "".to_string(),
            complete: false,
            observed_at: "".to_string(),
            source_at: "".to_string(),
            records: vec![],
        };
        assert_eq!(parse_query_response("r-1", resp).unwrap_err(), EnvelopeError::MissingRequestId);
    }
}
