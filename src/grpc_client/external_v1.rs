//! Closed ExternalV1 request-contract adapter (BR-238).
//!
//! The client bundle delivered on 2026-08-17 is not a complete wire contract.
//! This module therefore admits only operations whose request schema and JSON
//! shape are explicitly delivered and fixture-proven. Local Rust domain types
//! are never used to guess an upstream payload.

use crate::grpc_client::pb::magic::market::v1::{
    CanonicalPayload, Operation, QueryRequest, RequestContext,
};
use crate::market_domain::{AssetClass, InstrumentId};
use chrono::{DateTime, FixedOffset, NaiveDate};
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
    let (schema, schema_version, preferred_provider, data) = match operation {
        Operation::SecurityMetadata => {
            ensure_only_keys(&params, &["instruments"])?;
            let instruments = required_instruments(&params)?;
            (
                "magic.market.security_metadata.request",
                1,
                String::new(),
                serde_json::json!({"instruments": instruments}),
            )
        }
        Operation::GlobalNews => {
            ensure_only_keys(&params, &["provider", "limit"])?;
            let provider = params
                .get("provider")
                .and_then(Value::as_str)
                .filter(|provider| {
                    matches!(
                        *provider,
                        "Eastmoney" | "Cailianpress" | "Jin10" | "ThePaper"
                    )
                })
                .ok_or(ExternalContractError::InvalidParameters)?;
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .filter(|limit| (1..=20).contains(limit))
                .ok_or(ExternalContractError::InvalidParameters)?;
            (
                "magic.market.global_news.request",
                2,
                provider.to_owned(),
                serde_json::json!({"limit": limit}),
            )
        }
        Operation::InstrumentNews => {
            ensure_only_keys(
                &params,
                &["instrument", "start", "end", "limit", "captured_through"],
            )?;
            let instrument = required_instrument(&params)?;
            let captured_through = params
                .get("captured_through")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or(ExternalContractError::InvalidParameters)?;
            let captured_at = DateTime::parse_from_rfc3339(captured_through)
                .map_err(|_| ExternalContractError::InvalidParameters)?;
            let shanghai = FixedOffset::east_opt(8 * 60 * 60)
                .ok_or(ExternalContractError::InvalidParameters)?;
            let captured_date = captured_at.with_timezone(&shanghai).date_naive();
            let limit = params.get("limit").map_or(Ok(100_u64), |value| {
                value
                    .as_u64()
                    .filter(|limit| (1..=10_000).contains(limit))
                    .ok_or(ExternalContractError::InvalidParameters)
            })?;
            let mut request = Map::new();
            request.insert("instrument".to_string(), instrument);
            request.insert("limit".to_string(), Value::from(limit));
            match (params.get("start"), params.get("end")) {
                (None, None) => {}
                (Some(start), Some(end)) => {
                    let start = parse_iso_date(start)?;
                    let end = parse_iso_date(end)?;
                    if start > end {
                        return Err(ExternalContractError::InvalidParameters);
                    }
                    if end != captured_date {
                        return Err(ExternalContractError::InvalidParameters);
                    }
                    request.insert("start".to_string(), Value::from(start.to_string()));
                    request.insert("end".to_string(), Value::from(end.to_string()));
                }
                _ => return Err(ExternalContractError::InvalidParameters),
            }
            request.insert(
                "captured_through".to_string(),
                Value::from(captured_through),
            );
            (
                "magic.market.instrument_news.request",
                2,
                String::new(),
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
        preferred_provider,
        allow_unadmitted: false,
        payload: Some(CanonicalPayload {
            schema: schema.to_string(),
            schema_version,
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

fn required_instruments(params: &Value) -> Result<Vec<Value>, ExternalContractError> {
    let instruments = params
        .get("instruments")
        .and_then(Value::as_array)
        .ok_or(ExternalContractError::InvalidParameters)?;
    if instruments.is_empty() {
        return Err(ExternalContractError::InvalidParameters);
    }
    let mut seen = std::collections::HashSet::with_capacity(instruments.len());
    instruments
        .iter()
        .map(|value| {
            let instrument = canonical_instrument(value)?;
            let identity =
                serde_json::to_string(&instrument).map_err(|_| ExternalContractError::Serialize)?;
            if !seen.insert(identity) {
                return Err(ExternalContractError::InvalidParameters);
            }
            Ok(instrument)
        })
        .collect()
}

fn required_instrument(params: &Value) -> Result<Value, ExternalContractError> {
    params
        .get("instrument")
        .ok_or(ExternalContractError::InvalidParameters)
        .and_then(canonical_instrument)
}

fn canonical_instrument(value: &Value) -> Result<Value, ExternalContractError> {
    ensure_only_keys(value, &["exchange", "code", "asset_class"])?;
    let instrument: InstrumentId = serde_json::from_value(value.clone())
        .map_err(|_| ExternalContractError::InvalidParameters)?;
    if instrument.asset_class() != AssetClass::Equity {
        return Err(ExternalContractError::InvalidParameters);
    }
    serde_json::to_value(instrument).map_err(|_| ExternalContractError::Serialize)
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
        let request = build_external_query_request(
            Operation::SecurityMetadata,
            json!({
                "instruments": [{
                    "exchange": "Shanghai",
                    "code": "600396",
                    "asset_class": "Equity"
                }]
            }),
        )
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
    fn instrument_news_v2_requires_captured_through() {
        let error = build_external_query_request(
            Operation::InstrumentNews,
            json!({
                "instrument": {
                    "exchange": "Shenzhen",
                    "code": "000001",
                    "asset_class": "Equity"
                },
                "limit": 100
            }),
        )
        .expect_err("InstrumentNews v2 must bind an exact caller-captured upper bound");
        assert_eq!(error, ExternalContractError::InvalidParameters);
    }

    #[test]
    fn instrument_news_v2_binds_captured_through() {
        let request = build_external_query_request(
            Operation::InstrumentNews,
            json!({
                "instrument": {
                    "exchange": "Shenzhen",
                    "code": "TEST_CODE_000001",
                    "asset_class": "Equity"
                },
                "start": "2026-08-19",
                "end": "2026-08-19",
                "limit": 20,
                "captured_through": "2026-08-19T16:15:37+08:00"
            }),
        )
        .expect("delivered InstrumentNews v2 contract");

        let payload = request.payload.expect("external request payload");
        let data: Value = serde_json::from_slice(&payload.data).expect("external request JSON");
        assert_eq!(payload.schema, "magic.market.instrument_news.request");
        assert_eq!(payload.schema_version, 2);
        assert_eq!(
            data,
            json!({
                "instrument": {
                    "exchange": "Shenzhen",
                    "code": "TEST_CODE_000001",
                    "asset_class": "Equity"
                },
                "start": "2026-08-19",
                "end": "2026-08-19",
                "limit": 20,
                "captured_through": "2026-08-19T16:15:37+08:00"
            })
        );
        assert!(!request.allow_unadmitted);
    }

    #[test]
    fn instrument_news_v2_rejects_range_end_that_differs_from_shanghai_capture_date() {
        let error = build_external_query_request(
            Operation::InstrumentNews,
            json!({
                "instrument": {
                    "exchange": "Shenzhen",
                    "code": "TEST_CODE_000001",
                    "asset_class": "Equity"
                },
                "start": "2026-08-18",
                "end": "2026-08-18",
                "limit": 20,
                "captured_through": "2026-08-19T00:15:37+08:00"
            }),
        )
        .expect_err("request range must share the caller-captured Shanghai date");
        assert_eq!(error, ExternalContractError::InvalidParameters);
    }

    #[test]
    fn global_news_v2_routes_closed_provider_outside_business_payload() {
        let request = build_external_query_request(
            Operation::GlobalNews,
            json!({"provider": "Cailianpress", "limit": 20}),
        )
        .expect("delivered GlobalNews v2 contract");

        let payload = request.payload.expect("external request payload");
        let data: Value = serde_json::from_slice(&payload.data).expect("external request JSON");
        assert_eq!(request.preferred_provider, "Cailianpress");
        assert_eq!(payload.schema, "magic.market.global_news.request");
        assert_eq!(payload.schema_version, 2);
        assert_eq!(data, json!({"limit": 20}));
        assert!(!request.allow_unadmitted);
    }

    #[test]
    fn global_news_v2_rejects_unknown_provider_extra_fields_and_invalid_limits() {
        for params in [
            json!({"provider": "cailianpress", "limit": 1}),
            json!({"provider": "Jin10", "limit": 0}),
            json!({"provider": "Jin10", "limit": 21}),
            json!({"provider": "Jin10", "limit": 1, "url": "https://example.com"}),
        ] {
            assert_eq!(
                build_external_query_request(Operation::GlobalNews, params).unwrap_err(),
                ExternalContractError::InvalidParameters
            );
        }
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
                json!({
                    "instruments": [
                        {
                            "exchange": "Shanghai",
                            "code": "600396",
                            "asset_class": "Equity"
                        },
                        {
                            "exchange": "Shanghai",
                            "code": "600396",
                            "asset_class": "Equity"
                        }
                    ]
                }),
            )
            .unwrap_err(),
            ExternalContractError::InvalidParameters
        );
        assert_eq!(
            build_external_query_request(
                Operation::InstrumentNews,
                json!({
                    "instrument": {
                        "exchange": "Shenzhen",
                        "code": "000001",
                        "asset_class": "Equity"
                    },
                    "from_days": 30
                }),
            )
            .unwrap_err(),
            ExternalContractError::InvalidParameters
        );
    }

    #[test]
    fn rejects_bare_codes_instead_of_inferring_exchange() {
        for operation in [Operation::SecurityMetadata, Operation::InstrumentNews] {
            assert_eq!(
                build_external_query_request(operation, json!({"codes": ["600396"]})).unwrap_err(),
                ExternalContractError::InvalidParameters
            );
        }
    }
}
