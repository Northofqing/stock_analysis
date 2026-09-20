//! QueryRequest/QueryResponse 信封 (合同 §5/§6)。
//! request_id: 调用方生成的非空唯一请求 ID; 同一业务重试保留原 ID。
use crate::grpc_client::pb::magic::market::v1::Operation;
use crate::grpc_client::pb::magic::market::v1::{
    CanonicalPayload, QueryRequest, QueryResponse, RequestContext,
};
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
    #[error("响应 operation 不匹配: 期望 {0} 实际 {1}")]
    OperationMismatch(i32, i32),
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
    let data = serde_json::to_vec(&payload).map_err(|e| EnvelopeError::Serialize(e.to_string()))?;
    Ok(QueryRequest {
        context: Some(RequestContext {
            protocol_version: PROTOCOL_VERSION,
            request_id: new_request_id(),
        }),
        // 合同 §5: 普通调用保持 preferred_provider 为空, 由服务端 Composition 选择。
        preferred_provider: String::new(),
        // BR-238: production gateway requests never opt into diagnostic data.
        // A separately named operator probe may build an explicit diagnostic
        // request, but the shared production constructor stays fail-closed.
        allow_unadmitted: false,
        payload: Some(CanonicalPayload {
            schema: schema.schema_name.to_string(),
            schema_version: schema.schema_version,
            content_type: "application/json; charset=utf-8".to_string(),
            data,
        }),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryAdmission {
    Unspecified,
    Admitted,
    Unadmitted,
}

impl From<crate::grpc_client::pb::magic::market::v1::AdmissionState> for QueryAdmission {
    fn from(value: crate::grpc_client::pb::magic::market::v1::AdmissionState) -> Self {
        use crate::grpc_client::pb::magic::market::v1::AdmissionState;
        match value {
            AdmissionState::Admitted => Self::Admitted,
            AdmissionState::Unadmitted => Self::Unadmitted,
            AdmissionState::Unspecified => Self::Unspecified,
        }
    }
}

impl PartialEq<crate::grpc_client::pb::magic::market::v1::AdmissionState> for QueryAdmission {
    fn eq(&self, other: &crate::grpc_client::pb::magic::market::v1::AdmissionState) -> bool {
        *self == QueryAdmission::from(*other)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalRecord {
    pub schema: String,
    pub schema_version: u32,
    pub content_type: String,
    pub data: Vec<u8>,
}

impl From<CanonicalPayload> for CanonicalRecord {
    fn from(value: CanonicalPayload) -> Self {
        Self {
            schema: value.schema,
            schema_version: value.schema_version,
            content_type: value.content_type,
            data: value.data,
        }
    }
}

impl PartialEq<CanonicalPayload> for CanonicalRecord {
    fn eq(&self, other: &CanonicalPayload) -> bool {
        self.schema == other.schema
            && self.schema_version == other.schema_version
            && self.content_type == other.content_type
            && self.data == other.data
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcquisitionProvenance {
    Missing,
    LocalWireSource(String),
    ExternalMtlsAuthority(String),
}

impl AcquisitionProvenance {
    pub fn source(&self) -> &str {
        match self {
            Self::Missing => "",
            Self::LocalWireSource(value) | Self::ExternalMtlsAuthority(value) => value,
        }
    }
}

#[derive(Debug)] // unwrap_err 需要 Ok 类型 Debug
pub struct QueryResult {
    pub admission: QueryAdmission,
    pub selected_provider: String,
    pub batch_id: String,
    pub complete: bool,
    pub observed_at: String,
    pub source_at: String,
    pub records: Vec<CanonicalRecord>,
    pub provenance: AcquisitionProvenance,
    /// 非空表示服务端执行的是诊断读取；生产消费者即使收到 records 也必须拒绝。
    pub diagnostic_blocker: String,
}

impl QueryResult {
    pub fn source(&self) -> &str {
        self.provenance.source()
    }
}

pub fn parse_query_response(
    expected_request_id: &str,
    expected_operation: Operation,
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
    if resp.operation != expected_operation as i32 {
        return Err(EnvelopeError::OperationMismatch(
            expected_operation as i32,
            resp.operation,
        ));
    }
    Ok(QueryResult {
        // prost 0.14 from_i32 deprecated → try_from (语义等价, 未知值回落 Unspecified)。
        admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::try_from(
            resp.admission,
        )
        .unwrap_or(crate::grpc_client::pb::magic::market::v1::AdmissionState::Unspecified)
        .into(),
        selected_provider: resp.selected_provider,
        batch_id: resp.batch_id,
        complete: resp.complete,
        observed_at: resp.observed_at,
        source_at: resp.source_at,
        records: resp.records.into_iter().map(Into::into).collect(),
        provenance: if resp.source.is_empty() {
            AcquisitionProvenance::Missing
        } else {
            AcquisitionProvenance::LocalWireSource(resp.source)
        },
        diagnostic_blocker: resp.diagnostic_blocker,
    })
}

pub(crate) fn parse_external_query_response(
    expected_request_id: &str,
    expected_operation: Operation,
    authority: &str,
    resp: crate::grpc_client::external_pb::magic::market::v1::QueryResponse,
) -> Result<QueryResult, EnvelopeError> {
    use crate::grpc_client::external_pb::magic::market::v1::{
        AdmissionState as ExternalAdmission, Operation as ExternalOperation,
    };
    if resp.request_id.is_empty() {
        return Err(EnvelopeError::MissingRequestId);
    }
    if resp.request_id != expected_request_id {
        return Err(EnvelopeError::RequestIdMismatch(
            expected_request_id.to_owned(),
            resp.request_id,
        ));
    }
    let expected_external = match expected_operation {
        Operation::SecurityMetadata => ExternalOperation::SecurityMetadata,
        Operation::GlobalNews => ExternalOperation::GlobalNews,
        Operation::InstrumentNews => ExternalOperation::InstrumentNews,
        _ => return Err(EnvelopeError::OperationMismatch(expected_operation as i32, resp.operation)),
    };
    if resp.operation != expected_external as i32 {
        return Err(EnvelopeError::OperationMismatch(
            expected_external as i32,
            resp.operation,
        ));
    }
    let admission = match ExternalAdmission::try_from(resp.admission)
        .unwrap_or(ExternalAdmission::Unspecified)
    {
        ExternalAdmission::Admitted => QueryAdmission::Admitted,
        ExternalAdmission::Unadmitted => QueryAdmission::Unadmitted,
        ExternalAdmission::Unspecified => QueryAdmission::Unspecified,
    };
    Ok(QueryResult {
        admission,
        selected_provider: resp.selected_provider,
        batch_id: resp.batch_id,
        complete: resp.complete,
        observed_at: resp.observed_at,
        source_at: resp.source_at,
        records: resp
            .records
            .into_iter()
            .map(|record| CanonicalRecord {
                schema: record.schema,
                schema_version: record.schema_version,
                content_type: record.content_type,
                data: record.data,
            })
            .collect(),
        provenance: AcquisitionProvenance::ExternalMtlsAuthority(authority.to_owned()),
        diagnostic_blocker: resp.diagnostic_blocker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::AdmissionState;

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
        assert!(
            !req.allow_unadmitted,
            "BR-238 production requests must not opt into diagnostic data"
        );
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
            operation: Operation::RealtimeQuotes as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "tdx-dev".to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "t1".to_string(),
            source_at: "t2".to_string(),
            records: vec![],
            source: "tdx".to_string(),
            diagnostic_blocker: "TEST_CODE_diagnostic_blocker".to_string(),
        };
        let result = parse_query_response("r-1", Operation::RealtimeQuotes, resp).unwrap();
        assert_eq!(result.admission, AdmissionState::Admitted);
        assert!(result.complete);
        assert_eq!(result.selected_provider, "tdx-dev");
        assert_eq!(result.source(), "tdx");
        assert_eq!(result.diagnostic_blocker, "TEST_CODE_diagnostic_blocker");
    }

    #[test]
    fn rejects_mismatched_request_id() {
        let resp = QueryResponse {
            request_id: "other".to_string(),
            operation: Operation::RealtimeQuotes as i32,
            admission: 1,
            selected_provider: "".to_string(),
            batch_id: "".to_string(),
            complete: false,
            observed_at: "".to_string(),
            source_at: "".to_string(),
            records: vec![],
            source: String::new(),
            diagnostic_blocker: String::new(),
        };
        let err = parse_query_response("r-1", Operation::RealtimeQuotes, resp).unwrap_err();
        assert!(matches!(err, EnvelopeError::RequestIdMismatch(_, _)));
    }

    #[test]
    fn br238_rejects_response_for_a_different_operation() {
        let resp = QueryResponse {
            request_id: "r-1".to_string(),
            operation: Operation::OrderBooks as i32,
            admission: AdmissionState::Admitted as i32,
            selected_provider: "TEST_CODE_provider".to_string(),
            batch_id: "TEST_CODE_batch".to_string(),
            complete: true,
            observed_at: "2026-08-17T09:20:01+08:00".to_string(),
            source_at: "2026-08-17T09:20:00+08:00".to_string(),
            records: vec![],
            source: "TEST_CODE_source".to_string(),
            diagnostic_blocker: String::new(),
        };
        let error = parse_query_response("r-1", Operation::RealtimeQuotes, resp)
            .expect_err("a response for a different operation must fail closed");
        assert_eq!(
            error,
            EnvelopeError::OperationMismatch(
                Operation::RealtimeQuotes as i32,
                Operation::OrderBooks as i32,
            )
        );
    }

    #[test]
    fn rejects_missing_request_id() {
        let resp = QueryResponse {
            request_id: String::new(),
            operation: Operation::RealtimeQuotes as i32,
            admission: 1,
            selected_provider: "".to_string(),
            batch_id: "".to_string(),
            complete: false,
            observed_at: "".to_string(),
            source_at: "".to_string(),
            records: vec![],
            source: String::new(),
            diagnostic_blocker: String::new(),
        };
        assert_eq!(
            parse_query_response("r-1", Operation::RealtimeQuotes, resp).unwrap_err(),
            EnvelopeError::MissingRequestId
        );
    }
}
