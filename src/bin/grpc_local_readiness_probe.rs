//! 本地 gRPC 数据通道就绪探针：只输出脱敏的批次接纳摘要。

use chrono::Utc;
use clap::Parser;
use stock_analysis::data_gateway::grpc_source::convert;
use stock_analysis::data_gateway::GatewayError;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::errors::GrpcError;
use stock_analysis::grpc_client::pb::magic::market::v1::{AdmissionState, Operation};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:18083")]
    addr: String,
    #[arg(long, default_value = "600396")]
    code: String,
}

fn format_result(
    operation: &str,
    provider: &str,
    records: usize,
    time_untrustworthy: &str,
    status: &str,
) -> String {
    format!(
        "operation={operation} provider={provider} records={records} \
         time_untrustworthy={time_untrustworthy} status={status}"
    )
}

fn print_result(
    operation: &str,
    provider: &str,
    records: usize,
    time_untrustworthy: &str,
    status: &str,
) {
    println!(
        "{}",
        format_result(operation, provider, records, time_untrustworthy, status)
    );
}

fn safe_grpc(context: &'static str, error: GrpcError) -> anyhow::Error {
    let details = error.details();
    let reason_code = details.reason_code.as_deref().unwrap_or("absent");
    let retryable = details
        .retryable
        .map(|value| value.to_string())
        .unwrap_or_else(|| "absent".to_string());
    anyhow::anyhow!("operation={context} reason_code={reason_code} retryable={retryable}")
}

fn safe_gateway(context: &'static str, error: GatewayError) -> anyhow::Error {
    anyhow::anyhow!(
        "operation={context} reason_code={} retryable={}",
        error.reason_code(),
        error.retryable()
    )
}

fn probe_failure(
    operation: &'static str,
    reason_code: &'static str,
    retryable: bool,
) -> anyhow::Error {
    anyhow::anyhow!("operation={operation} reason_code={reason_code} retryable={retryable}")
}

fn capability_ready(
    capabilities: &[stock_analysis::grpc_client::pb::magic::market::v1::Capability],
    operation: Operation,
) -> bool {
    capabilities.iter().any(|capability| {
        capability.operation == operation as i32
            && capability.repository_admission == AdmissionState::Admitted as i32
            && capability.runtime_available
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let code = args.code.clone();
    let mut client = GrpcMarketClient::connect(&args.addr)
        .await
        .map_err(|error| safe_grpc("Connect", error))?;

    let health = client
        .get_health()
        .await
        .map_err(|error| safe_grpc("Health", error))?;
    if !health.live || !health.ready {
        return Err(probe_failure("Health", "not_ready", true));
    }
    print_result("Health", "not_applicable", 1, "not_applicable", "ready");

    let capabilities = client
        .get_capabilities()
        .await
        .map_err(|error| safe_grpc("Capabilities", error))?;
    let required_operations = [
        Operation::RealtimeQuotes,
        Operation::OrderBooks,
        Operation::T0Evidence,
        Operation::HistoricalBars,
    ];
    if required_operations
        .iter()
        .copied()
        .any(|operation| !capability_ready(&capabilities, operation))
    {
        return Err(probe_failure(
            "Capabilities",
            "required_capability_unavailable",
            false,
        ));
    }
    print_result(
        "Capabilities",
        "not_applicable",
        required_operations.len(),
        "not_applicable",
        "available",
    );

    let quote_q = client
        .query(
            Operation::RealtimeQuotes,
            serde_json::json!({"codes": [&code]}),
        )
        .await
        .map_err(|error| safe_grpc("RealtimeQuotes", error))?;
    let quotes = convert::realtime_quotes_at(&quote_q, Utc::now())
        .map_err(|error| safe_gateway("RealtimeQuotes", error))?;
    if quotes.records().len() != 1 || quotes.records()[0].code != code {
        return Err(probe_failure(
            "RealtimeQuotes",
            "record_identity_mismatch",
            false,
        ));
    }
    let quote_provider = format!("{:?}", quotes.evidence().provider);
    print_result(
        "RealtimeQuotes",
        &quote_provider,
        quotes.records().len(),
        "not_applicable",
        "available",
    );

    let book_q = client
        .query(Operation::OrderBooks, serde_json::json!({"codes": [&code]}))
        .await
        .map_err(|error| safe_grpc("OrderBooks", error))?;
    let books = convert::order_books_at(&book_q, Utc::now())
        .map_err(|error| safe_gateway("OrderBooks", error))?;
    if books.records().len() != 1 || books.records()[0].code != code {
        return Err(probe_failure(
            "OrderBooks",
            "record_identity_mismatch",
            false,
        ));
    }
    let book_provider = format!("{:?}", books.evidence().provider);
    print_result(
        "OrderBooks",
        &book_provider,
        books.records().len(),
        "not_applicable",
        "available",
    );

    let t0_q = client
        .query(Operation::T0Evidence, serde_json::json!({"codes": [&code]}))
        .await
        .map_err(|error| safe_grpc("T0Evidence", error))?;
    let t0 = convert::t0_evidence_batch_at(&t0_q, Utc::now())
        .map_err(|error| safe_gateway("T0Evidence", error))?;
    if t0.records.len() != 1 || t0.records[0].code != code || !t0.rejections.is_empty() {
        return Err(probe_failure(
            "T0Evidence",
            "record_identity_or_rejection_mismatch",
            false,
        ));
    }
    let t0_provider = format!("{:?}", t0.provider);
    let time_untrustworthy = t0.time_untrustworthy.to_string();
    print_result(
        "T0Evidence",
        &t0_provider,
        t0.records.len(),
        &time_untrustworthy,
        "available",
    );

    let daily_q = client
        .query(
            Operation::HistoricalBars,
            serde_json::json!({"codes": [&code], "days": 5}),
        )
        .await
        .map_err(|error| safe_grpc("HistoricalBars", error))?;
    let daily = convert::historical_bars(&code, &daily_q)
        .map_err(|error| safe_gateway("HistoricalBars", error))?;
    if daily.records().is_empty() {
        return Err(probe_failure(
            "HistoricalBars",
            "records_unavailable",
            false,
        ));
    }
    let daily_provider = format!("{:?}", daily.evidence().provider);
    print_result(
        "HistoricalBars",
        &daily_provider,
        daily.records().len(),
        "not_applicable",
        "available",
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_line_has_only_the_five_safe_fields() {
        let result = format_result("T0Evidence", "Tdx", 1, "false", "available");

        assert_eq!(
            result,
            "operation=T0Evidence provider=Tdx records=1 time_untrustworthy=false status=available"
        );
        for forbidden in ["600396", "price", "token", "/"] {
            assert!(!result.contains(forbidden));
        }
    }
}
