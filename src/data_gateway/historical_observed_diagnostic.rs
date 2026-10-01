//! A narrow cross-binary diagnostic façade. No rich evidence types are public.

use anyhow::{ensure, Result};
use chrono::{NaiveDate, Utc};
use std::path::Path;

use super::external_historical_bars::HistoricalWindowRequest;
use crate::market_domain::{AssetClass, Exchange, InstrumentId};

fn local_error_kind(error: &crate::grpc_client::errors::GrpcError) -> &'static str {
    use crate::grpc_client::errors::GrpcError;
    match error {
        GrpcError::InvalidArgument { .. } => "InvalidArgument",
        GrpcError::Unauthenticated { .. } => "Unauthenticated",
        GrpcError::PermissionDenied { .. } => "PermissionDenied",
        GrpcError::Unimplemented { .. } => "Unimplemented",
        GrpcError::ResourceExhausted { .. } => "ResourceExhausted",
        GrpcError::DeadlineExceeded { .. } => "DeadlineExceeded",
        GrpcError::Unavailable { .. } => "Unavailable",
        GrpcError::FailedPrecondition { .. } => "FailedPrecondition",
        GrpcError::Internal { .. } => "Internal",
        GrpcError::Unknown { .. } => "Unknown",
    }
}

fn diagnostic_request(
    exchange: &str,
    code: &str,
    from: &str,
    to: &str,
) -> Result<HistoricalWindowRequest> {
    let exchange = match exchange {
        "Shanghai" => Exchange::Shanghai,
        "Shenzhen" => Exchange::Shenzhen,
        _ => anyhow::bail!("historical diagnostic requires Shanghai or Shenzhen"),
    };
    ensure!(
        code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()),
        "historical diagnostic requires an exact six-digit equity code"
    );
    let canonical_date = |value: &str| -> Result<NaiveDate> {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("historical diagnostic requires canonical dates"))?;
        ensure!(
            date.format("%Y-%m-%d").to_string() == value,
            "historical diagnostic requires canonical dates"
        );
        Ok(date)
    };
    let instrument = InstrumentId::new(exchange, code, AssetClass::Equity)
        .map_err(|_| anyhow::anyhow!("historical diagnostic instrument invalid"))?;
    HistoricalWindowRequest::new(
        instrument,
        canonical_date(from)?,
        canonical_date(to)?,
        Utc::now(),
    )
    .map_err(|_| anyhow::anyhow!("historical diagnostic calendar/window unavailable or unfinished"))
}

/// Read one exact completed historical window using an explicit authenticated
/// bundle, then preserve its raw observation in an existing isolated directory.
/// Returns only secret-safe diagnostic JSON. Disk evidence cannot be supplied
/// as an input or reconstructed into a live/admitted data capability.
#[doc(hidden)]
pub async fn observed_history_diagnostic(
    bundle: &Path,
    output_root: &Path,
    exchange: &str,
    code: &str,
    from: &str,
    to: &str,
) -> Result<String> {
    let request = diagnostic_request(exchange, code, from, to)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (bundle, output_root, request);
        anyhow::bail!("historical diagnostic anchored store unsupported on this target");
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use super::external_historical_bars::ExternalHistoricalBarsGateway;
        use super::historical_observed_store::{HistoricalObservedStore, Publication};
        use super::historical_record_projection::project_observed_historical_records;
        let bundle_canonical = std::fs::canonicalize(bundle)
            .map_err(|_| anyhow::anyhow!("historical diagnostic bundle path unavailable"))?;
        let bundle_namespace = if bundle_canonical.is_dir() {
            bundle_canonical
        } else {
            bundle_canonical
                .parent()
                .ok_or_else(|| anyhow::anyhow!("historical diagnostic bundle namespace invalid"))?
                .to_path_buf()
        };
        let forbidden = [bundle_namespace];
        let store =
            HistoricalObservedStore::open_existing(output_root, &forbidden).map_err(|_| {
                anyhow::anyhow!(
                    "historical diagnostic output is not an isolated anchored private directory"
                )
            })?;
        let public_inputs = crate::grpc_client::build_identity::compiled_public_inputs()
            .map_err(|_| anyhow::anyhow!("historical diagnostic compiled public inputs invalid"))?;
        let mut gateway = ExternalHistoricalBarsGateway::connect_client_bundle(bundle)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "historical diagnostic connection failed ({})",
                    local_error_kind(&error)
                )
            })?;
        let capture = gateway.observe_once(request).await.map_err(|error| {
            anyhow::anyhow!(
                "historical diagnostic pre-capture qualification/read failed ({})",
                local_error_kind(&error)
            )
        })?;
        // Persist even negative observations before attempting row projection.
        let (artifact, publication) = store.persist(&capture).map_err(|_| {
            anyhow::anyhow!("historical diagnostic observed evidence publication failed")
        })?;
        let recorded = store.read_checked(&artifact).map_err(|_| {
            anyhow::anyhow!("historical diagnostic stored evidence verification failed")
        })?;
        ensure!(
            recorded.artifact() == &artifact,
            "historical diagnostic artifact verification mismatch"
        );
        let query_outcome = match &capture.observation().result {
            Ok(_) => serde_json::json!({"state":"EnvelopeObserved"}),
            Err(error) => {
                serde_json::json!({"state":"Rejected","kind":local_error_kind(error),"retryable":error.details().retryable})
            }
        };
        let (diagnostic_result, projection) = match project_observed_historical_records(&capture) {
            Ok(projection) => (
                "StoredObservedRows",
                serde_json::json!({
                    "state":"ParsedObservedRows",
                    "row_count":projection.rows().len(),
                    "observed_dates":projection.rows().iter().map(|row|row.source_date()).collect::<Vec<_>>(),
                    "missing_observed_dates":projection.missing_observed_dates(),
                    "all_requested_dates_observed":projection.all_requested_dates_observed(),
                    "server_complete_observation":projection.observed_complete_flag()
                }),
            ),
            Err(error) => (
                "StoredNegativeObservation",
                serde_json::json!({"state":"Rejected","reason":error.to_string()}),
            ),
        };
        // No raw remote message, status text, metadata, keys, bearer or bundle
        // contents are printed. Their allowed capture bytes stay in the store.
        serde_json::to_string(&serde_json::json!({
            "version":1,
            "scope":"ObservedOnly",
            "authority":"NotAdmitted",
            "coverage":"Unknown",
            "pit":"NotCertified",
            "diagnostic_result":diagnostic_result,
            "compiled_public_inputs":public_inputs,
            "request":{
                "instrument":capture.request().instrument(),
                "from":capture.request().from(),
                "to":capture.request().to(),
                "required_trading_dates":capture.request().required_trading_dates(),
                "wire_limit":capture.request().required_trading_dates().len(),
                "calendar_authority_sha256":capture.request().calendar_authority_hash()
            },
            "artifact":{
                "filename":artifact.filename(),
                "capture_sha256":artifact.capture_sha256(),
                "file_sha256":artifact.file_sha256(),
                "byte_length":artifact.byte_length(),
                "publication":match publication { Publication::Published=>"Published", Publication::ExistingExact=>"ExistingExact" }
            },
            "request_binding":if capture.request_binding_error().is_none() {"Matched"} else {"Rejected"},
            "query_outcome":query_outcome,
            "projection":projection
        })).map_err(|_| anyhow::anyhow!("historical diagnostic summary encoding failed"))
    }
}

#[cfg(test)]
#[path = "historical_observed_diagnostic_tests.rs"]
mod tests;
