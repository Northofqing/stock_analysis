//! Closed ExternalV1 request-contract adapter (BR-231).
//!
//! The client bundle delivered on 2026-08-17 is not a complete wire contract.
//! This module therefore admits only operations whose request schema and JSON
//! shape are explicitly delivered and fixture-proven. Local Rust domain types
//! are never used to guess an upstream payload.

use crate::grpc_client::pb::magic::market::v1::{
    CanonicalPayload, Operation, QueryRequest, RequestContext,
};
use chrono::NaiveDate;
use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExternalContractError {
    #[error("external-v1 operation contract was not delivered")]
    UndeliveredOperation,
    #[error("external-v1 request parameters are invalid")]
    InvalidParameters,
    #[error("external-v1 request serialization failed")]
    Serialize,
}

pub fn build_external_query_request(
    operation: Operation,
    params: Value,
) -> Result<QueryRequest, ExternalContractError> {
    let (schema, data) = match operation {
        Operation::SecurityMetadata => {
            ensure_only_keys(&params, &["codes"])?;
            let codes = required_codes(&params)?;
            (
                "magic.market.security_metadata.request",
                serde_json::json!({"instruments": instruments(&codes)?}),
            )
        }
        Operation::InstrumentNews => {
            ensure_only_keys(&params, &["codes", "start", "end", "limit"])?;
            let codes = required_codes(&params)?;
            let [code] = codes.as_slice() else {
                return Err(ExternalContractError::InvalidParameters);
            };
            let limit = params.get("limit").map_or(Ok(100_u64), |value| {
                value
                    .as_u64()
                    .filter(|limit| (1..=10_000).contains(limit))
                    .ok_or(ExternalContractError::InvalidParameters)
            })?;
            let mut request = Map::new();
            request.insert("instrument".to_string(), instrument(code)?);
            request.insert("limit".to_string(), Value::from(limit));
            match (params.get("start"), params.get("end")) {
                (None, None) => {}
                (Some(start), Some(end)) => {
                    let start = parse_iso_date(start)?;
                    let end = parse_iso_date(end)?;
                    if start > end {
                        return Err(ExternalContractError::InvalidParameters);
                    }
                    request.insert("start".to_string(), Value::from(start.to_string()));
                    request.insert("end".to_string(), Value::from(end.to_string()));
                }
                _ => return Err(ExternalContractError::InvalidParameters),
            }
            (
                "magic.market.instrument_news.request",
                Value::Object(request),
            )
        }
        _ => return Err(ExternalContractError::UndeliveredOperation),
    };

    let data = serde_json::to_vec(&data).map_err(|_| ExternalContractError::Serialize)?;
    Ok(QueryRequest {
        context: Some(RequestContext {
            protocol_version: 1,
            request_id: crate::grpc_client::envelope::new_request_id(),
        }),
        preferred_provider: String::new(),
        allow_unadmitted: false,
        payload: Some(CanonicalPayload {
            schema: schema.to_string(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".to_string(),
            data,
        }),
    })
}

fn ensure_only_keys(params: &Value, allowed: &[&str]) -> Result<(), ExternalContractError> {
    let object = params
        .as_object()
        .ok_or(ExternalContractError::InvalidParameters)?;
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(ExternalContractError::InvalidParameters)
    }
}

fn required_codes(params: &Value) -> Result<Vec<String>, ExternalContractError> {
    let codes = params
        .get("codes")
        .and_then(Value::as_array)
        .ok_or(ExternalContractError::InvalidParameters)?;
    if codes.is_empty() {
        return Err(ExternalContractError::InvalidParameters);
    }
    let mut seen = std::collections::HashSet::with_capacity(codes.len());
    codes
        .iter()
        .map(|value| {
            let code = value
                .as_str()
                .filter(|code| code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
                .ok_or(ExternalContractError::InvalidParameters)?;
            if !seen.insert(code) {
                return Err(ExternalContractError::InvalidParameters);
            }
            Ok(code.to_string())
        })
        .collect()
}

fn instruments(codes: &[String]) -> Result<Vec<Value>, ExternalContractError> {
    codes.iter().map(|code| instrument(code)).collect()
}

fn instrument(code: &str) -> Result<Value, ExternalContractError> {
    let exchange = match code.as_bytes().first() {
        Some(b'6') => "Shanghai",
        Some(b'0' | b'3') => "Shenzhen",
        Some(b'4' | b'8' | b'9') => "Beijing",
        _ => return Err(ExternalContractError::InvalidParameters),
    };
    Ok(serde_json::json!({
        "exchange": exchange,
        "code": code,
        "asset_class": "Equity",
    }))
}

fn parse_iso_date(value: &Value) -> Result<NaiveDate, ExternalContractError> {
    let value = value
        .as_str()
        .ok_or(ExternalContractError::InvalidParameters)?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ExternalContractError::InvalidParameters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn payload_json(request: QueryRequest) -> (String, Value, bool) {
        let payload = request.payload.expect("external request payload");
        let data = serde_json::from_slice(&payload.data).expect("external request JSON");
        (payload.schema, data, request.allow_unadmitted)
    }

    #[test]
    fn security_metadata_uses_delivered_schema_and_canonical_instruments() {
        let request =
            build_external_query_request(Operation::SecurityMetadata, json!({"codes": ["600396"]}))
                .expect("delivered SecurityMetadata contract");

        let (schema, data, allow_unadmitted) = payload_json(request);
        assert_eq!(schema, "magic.market.security_metadata.request");
        assert_eq!(
            data,
            json!({
                "instruments": [{
                    "exchange": "Shanghai",
                    "code": "600396",
                    "asset_class": "Equity"
                }]
            })
        );
        assert!(!allow_unadmitted);
    }

    #[test]
    fn instrument_news_uses_delivered_single_instrument_contract() {
        let request = build_external_query_request(
            Operation::InstrumentNews,
            json!({"codes": ["000001"], "limit": 100}),
        )
        .expect("delivered InstrumentNews contract");

        let (schema, data, allow_unadmitted) = payload_json(request);
        assert_eq!(schema, "magic.market.instrument_news.request");
        assert_eq!(
            data,
            json!({
                "instrument": {
                    "exchange": "Shenzhen",
                    "code": "000001",
                    "asset_class": "Equity"
                },
                "limit": 100
            })
        );
        assert!(!allow_unadmitted);
    }

    #[test]
    fn rejects_live_but_undelivered_external_contracts_before_io() {
        for operation in [
            Operation::RealtimeQuotes,
            Operation::BoardConstituents,
            Operation::UpperLimitPoolReview,
        ] {
            assert_eq!(
                build_external_query_request(operation, json!({})).unwrap_err(),
                ExternalContractError::UndeliveredOperation,
                "{operation:?} must not be inferred from local types"
            );
        }
    }

    #[test]
    fn rejects_duplicate_or_ambiguous_instrument_requests() {
        assert_eq!(
            build_external_query_request(
                Operation::SecurityMetadata,
                json!({"codes": ["600396", "600396"]}),
            )
            .unwrap_err(),
            ExternalContractError::InvalidParameters
        );
        assert_eq!(
            build_external_query_request(
                Operation::InstrumentNews,
                json!({"codes": ["000001"], "from_days": 30}),
            )
            .unwrap_err(),
            ExternalContractError::InvalidParameters
        );
    }
}
