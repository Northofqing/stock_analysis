//! canonical payload 校验 (合同 §5: 未知 schema/version 必须停止解析)。
use crate::grpc_client::pb::magic::market::v1::Operation;
use crate::grpc_contract::schema::schema_for;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SchemaError {
    #[error("未知 schema: {0} (不允许忽略或猜字段)")]
    UnknownSchema(String),
    #[error("schema {0} 版本不支持: {1}")]
    UnsupportedVersion(String, u32),
    #[error("payload 不是合法 UTF-8 JSON: {0}")]
    NotJson(String),
}

/// 校验 schema/version 并解析 data 为 JSON。失败必须拒绝, 不返回部分结果。
pub fn validate_payload(
    operation: Operation,
    schema: &str,
    version: u32,
    data: &[u8],
) -> Result<serde_json::Value, SchemaError> {
    let frozen =
        schema_for(operation).ok_or_else(|| SchemaError::UnknownSchema(schema.to_string()))?;
    if frozen.schema_name != schema {
        return Err(SchemaError::UnknownSchema(schema.to_string()));
    }
    if frozen.schema_version != version {
        return Err(SchemaError::UnsupportedVersion(schema.to_string(), version));
    }
    serde_json::from_slice(data).map_err(|e| SchemaError::NotJson(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::Operation;

    #[test]
    fn rejects_unknown_schema() {
        let err =
            validate_payload(Operation::RealtimeQuotes, "not.a.schema", 1, b"[]").unwrap_err();
        assert_eq!(err, SchemaError::UnknownSchema("not.a.schema".to_string()));
    }

    #[test]
    fn rejects_wrong_schema_for_operation() {
        // Announcements 的 schema 名不能用于 RealtimeQuotes。
        let err = validate_payload(Operation::RealtimeQuotes, "news.announcements", 1, b"[]")
            .unwrap_err();
        assert!(matches!(err, SchemaError::UnknownSchema(_)));
    }

    #[test]
    fn rejects_unsupported_version() {
        let err = validate_payload(
            Operation::RealtimeQuotes,
            "market.realtime_quotes",
            99,
            b"[]",
        )
        .unwrap_err();
        assert_eq!(
            err,
            SchemaError::UnsupportedVersion("market.realtime_quotes".to_string(), 99)
        );
    }

    #[test]
    fn rejects_non_json_data() {
        let err = validate_payload(
            Operation::RealtimeQuotes,
            "market.realtime_quotes",
            1,
            b"not json",
        )
        .unwrap_err();
        assert!(matches!(err, SchemaError::NotJson(_)));
    }

    #[test]
    fn parses_valid_json() {
        let value = validate_payload(
            Operation::RealtimeQuotes,
            "market.realtime_quotes",
            1,
            br#"[{"code":"600519"}]"#,
        )
        .unwrap();
        assert_eq!(value[0]["code"], "600519");
    }
}
