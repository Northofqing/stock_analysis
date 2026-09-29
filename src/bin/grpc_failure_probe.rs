//! One read-only LocalBridgeV1 flow query and one GetHealth on an explicit loopback endpoint.
//! The local contract has no server build_identity; this tool never claims one.

use anyhow::{bail, Result};
use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::errors::GrpcError;
use stock_analysis::grpc_client::pb::magic::market::v1::Operation;

#[derive(Parser)]
#[command(about = "Single-request LocalBridgeV1 flow failure diagnostic (loopback only)")]
struct Args {
    /// Explicit Mac local bridge endpoint, e.g. http://127.0.0.1:18082.
    #[arg(long)]
    addr: String,
    #[arg(long, value_enum)]
    operation: FlowOperation,
    /// One six-digit equity code, required only for money-flows.
    #[arg(long)]
    code: Option<String>,
    /// Board family, required only for board-flows.
    #[arg(long, value_enum)]
    kind: Option<BoardKind>,
    /// Board row limit, 1..=20, required only for board-flows.
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum FlowOperation {
    MoneyFlows,
    BoardFlows,
}

#[derive(Clone, Copy, ValueEnum)]
enum BoardKind {
    Industry,
    Concept,
    Region,
}

impl BoardKind {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Industry => "Industry",
            Self::Concept => "Concept",
            Self::Region => "Region",
        }
    }
}

fn loopback_addr(value: &str) -> Result<String> {
    let addr: SocketAddr = value
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("invalid loopback --addr"))?
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid loopback --addr"))?;
    if !addr.ip().is_loopback() || addr.port() == 0 {
        bail!("--addr must be an explicit HTTP loopback IP and port");
    }
    Ok(format!("http://{addr}"))
}

fn query_spec(args: &Args) -> Result<(Operation, serde_json::Value)> {
    match args.operation {
        FlowOperation::MoneyFlows => {
            if args.kind.is_some() || args.limit.is_some() {
                bail!("money-flows accepts only --code");
            }
            let code = args.code.as_deref().unwrap_or_default();
            if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
                bail!("money-flows requires one six-digit --code");
            }
            Ok((Operation::MoneyFlows, serde_json::json!({"codes":[code]})))
        }
        FlowOperation::BoardFlows => {
            if args.code.is_some() {
                bail!("board-flows accepts only --kind and --limit");
            }
            let kind = args
                .kind
                .ok_or_else(|| anyhow::anyhow!("board-flows requires --kind"))?;
            let limit = args
                .limit
                .filter(|value| (1..=20).contains(value))
                .ok_or_else(|| anyhow::anyhow!("board-flows requires --limit in 1..=20"))?;
            Ok((
                Operation::BoardFlows,
                serde_json::json!({"kind":kind.as_wire(),"limit":limit}),
            ))
        }
    }
}

fn error_variant(error: &GrpcError) -> &'static str {
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

fn safe_atom(value: Option<&str>) -> String {
    match value {
        None | Some("") => "absent".to_owned(),
        Some(value)
            if value.len() <= 64
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
                }) =>
        {
            value.to_owned()
        }
        Some(_) => "redacted".to_owned(),
    }
}

fn print_error(stage: &str, operation: Operation, correlation: &str, error: &GrpcError) {
    let variant = error_variant(error);
    let detail = error.details();
    let status = safe_atom(Some(&detail.code));
    let detail_request_id_match = match detail.request_id.as_deref() {
        Some(value) if value == correlation => "true",
        Some(_) => "false",
        None => "absent",
    };
    let retryable = match detail.retryable {
        Some(true) => "true",
        Some(false) => "false",
        None => "absent",
    };
    println!(
        "probe stage={stage} operation={} result=failed grpc_variant={variant} grpc_status={status} provider={} reason_code={} retryable={retryable} request_id_correlation={correlation} detail_request_id_match={detail_request_id_match}",
        operation.as_str_name(),
        safe_atom(detail.provider.as_deref()),
        safe_atom(detail.reason_code.as_deref()),
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let addr = loopback_addr(&args.addr)?;
    let (operation, payload) = query_spec(&args)?;
    let mut client = GrpcMarketClient::connect(&addr)
        .await
        .map_err(|_| anyhow::anyhow!("local bridge connection failed"))?;
    let health = client.get_health().await.map_err(|error| {
        print_error("health", operation, "absent", &error);
        anyhow::anyhow!("local bridge health failed")
    })?;
    let client_contract_sha256 = format!(
        "{:x}",
        Sha256::digest(stock_analysis::grpc_client::pb::FILE_DESCRIPTOR_SET)
    );
    println!(
        "probe stage=health operation={} live={} ready={} build_identity_digest=unavailable build_identity_qualification=unavailable_local_contract local_client_contract_sha256={client_contract_sha256}",
        operation.as_str_name(), health.live, health.ready,
    );
    if !health.live || !health.ready {
        bail!("local bridge is not live and ready");
    }
    let observed = client
        .query_local_flow_once_observed(operation, payload)
        .await
        .map_err(|_| anyhow::anyhow!("local flow request could not be constructed"))?;
    let correlation = observed.request_id_correlation().to_owned();
    match observed.into_result() {
        Ok(_) => println!(
            "probe stage=query operation={} result=success request_id_correlation={correlation} response_payload=omitted",
            operation.as_str_name(),
        ),
        Err(error) => print_error("query", operation, &correlation, &error),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_and_parameters_fail_closed_before_connection() {
        for value in [
            "https://127.0.0.1:50051",
            "http://example.com:50051",
            "http://localhost:50051",
            "http://127.0.0.1:50051/path",
            "http://user:pass@127.0.0.1:50051",
            "http://127.0.0.1:50051?next=remote",
        ] {
            assert!(loopback_addr(value).is_err());
        }
        assert!(loopback_addr("http://127.0.0.1:50051").is_ok());
        assert!(loopback_addr("http://[::1]:50051").is_ok());
        let args = Args {
            addr: "http://127.0.0.1:50051".into(),
            operation: FlowOperation::BoardFlows,
            code: None,
            kind: Some(BoardKind::Industry),
            limit: Some(21),
        };
        assert!(query_spec(&args).is_err());
    }

    #[test]
    fn diagnostic_fields_are_bounded_and_do_not_render_raw_request_id() {
        assert_eq!(safe_atom(Some("Eastmoney")), "Eastmoney");
        assert_eq!(safe_atom(Some("secret/value")), "redacted");
        assert_eq!(safe_atom(Some(&"x".repeat(65))), "redacted");
        let error = GrpcError::Internal {
            details: Box::new(stock_analysis::grpc_client::errors::ErrorDetail {
                request_id: Some("sha256:deadbeef".into()),
                provider: Some("Eastmoney".into()),
                reason_code: Some("unsupported_contract".into()),
                retryable: Some(false),
                ..Default::default()
            }),
        };
        assert_eq!(error_variant(&error), "Internal");
    }
}
