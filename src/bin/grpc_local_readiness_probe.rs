//! 本地 gRPC 数据通道就绪探针：只输出脱敏的批次接纳摘要。

use chrono::Utc;
use clap::Parser;
use stock_analysis::data_gateway::grpc_source::convert;
use stock_analysis::data_gateway::GatewayError;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::errors::GrpcError;
use stock_analysis::grpc_client::pb::magic::market::v1::{AdmissionState, Capability, Operation};

const REQUIRED_OPERATIONS: [Operation; 4] = [
    Operation::RealtimeQuotes,
    Operation::OrderBooks,
    Operation::T0Evidence,
    Operation::HistoricalBars,
];

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

fn capability_ready(capabilities: &[Capability], operation: Operation) -> bool {
    capabilities.iter().any(|capability| {
        capability.operation == operation as i32
            && capability.repository_admission == AdmissionState::Admitted as i32
            && capability.runtime_available
    })
}

fn health_failure(live: bool, ready: bool) -> Option<anyhow::Error> {
    (!live || !ready).then(|| probe_failure("Health", "not_ready", true))
}

fn capabilities_failure(capabilities: &[Capability]) -> Option<anyhow::Error> {
    REQUIRED_OPERATIONS
        .iter()
        .copied()
        .any(|operation| !capability_ready(capabilities, operation))
        .then(|| probe_failure("Capabilities", "required_capability_unavailable", false))
}

fn record_identity_failure(
    operation: &'static str,
    records: usize,
    identity_matches: bool,
) -> Option<anyhow::Error> {
    (records != 1 || !identity_matches)
        .then(|| probe_failure(operation, "record_identity_mismatch", false))
}

fn t0_failure(records: usize, identity_matches: bool, rejections: usize) -> Option<anyhow::Error> {
    (records != 1 || !identity_matches || rejections != 0)
        .then(|| probe_failure("T0Evidence", "record_identity_or_rejection_mismatch", false))
}

fn daily_failure(records: usize) -> Option<anyhow::Error> {
    (records == 0).then(|| probe_failure("HistoricalBars", "records_unavailable", false))
}

async fn run(args: Args) -> anyhow::Result<()> {
    let code = args.code.clone();
    let mut client = GrpcMarketClient::connect(&args.addr)
        .await
        .map_err(|error| safe_grpc("Connect", error))?;

    let health = client
        .get_health()
        .await
        .map_err(|error| safe_grpc("Health", error))?;
    if let Some(error) = health_failure(health.live, health.ready) {
        return Err(error);
    }
    print_result("Health", "not_applicable", 1, "not_applicable", "ready");

    let capabilities = client
        .get_capabilities()
        .await
        .map_err(|error| safe_grpc("Capabilities", error))?;
    if let Some(error) = capabilities_failure(&capabilities) {
        return Err(error);
    }
    print_result(
        "Capabilities",
        "not_applicable",
        REQUIRED_OPERATIONS.len(),
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
    let quote_identity_matches = quotes
        .records()
        .first()
        .is_some_and(|record| record.code == code);
    if let Some(error) = record_identity_failure(
        "RealtimeQuotes",
        quotes.records().len(),
        quote_identity_matches,
    ) {
        return Err(error);
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
    let book_identity_matches = books
        .records()
        .first()
        .is_some_and(|record| record.code == code);
    if let Some(error) =
        record_identity_failure("OrderBooks", books.records().len(), book_identity_matches)
    {
        return Err(error);
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
    let t0_identity_matches = t0.records.first().is_some_and(|record| record.code == code);
    if let Some(error) = t0_failure(t0.records.len(), t0_identity_matches, t0.rejections.len()) {
        return Err(error);
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
    if let Some(error) = daily_failure(daily.records().len()) {
        return Err(error);
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::grpc_client::errors::ErrorDetail;

    fn admitted_capability(operation: Operation) -> Capability {
        Capability {
            operation: operation as i32,
            repository_admission: AdmissionState::Admitted as i32,
            runtime_available: true,
            ..Default::default()
        }
    }

    fn required_capabilities() -> Vec<Capability> {
        [
            Operation::RealtimeQuotes,
            Operation::OrderBooks,
            Operation::T0Evidence,
            Operation::HistoricalBars,
        ]
        .into_iter()
        .map(admitted_capability)
        .collect()
    }

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

    #[test]
    fn health_requires_both_live_and_ready() {
        assert!(health_failure(true, true).is_none());
        for (live, ready) in [(false, true), (true, false), (false, false)] {
            assert_eq!(
                health_failure(live, ready).unwrap().to_string(),
                "operation=Health reason_code=not_ready retryable=true"
            );
        }
    }

    #[test]
    fn every_required_capability_must_be_admitted_and_runtime_available() {
        let admitted = required_capabilities();
        assert!(capabilities_failure(&admitted).is_none());

        for index in 0..admitted.len() {
            let mut not_admitted = admitted.clone();
            not_admitted[index].repository_admission = AdmissionState::Unadmitted as i32;
            assert_eq!(
                capabilities_failure(&not_admitted).unwrap().to_string(),
                "operation=Capabilities reason_code=required_capability_unavailable retryable=false"
            );

            let mut unavailable = admitted.clone();
            unavailable[index].runtime_available = false;
            assert_eq!(
                capabilities_failure(&unavailable).unwrap().to_string(),
                "operation=Capabilities reason_code=required_capability_unavailable retryable=false"
            );
        }
    }

    #[test]
    fn record_identity_requires_exactly_one_matching_record() {
        assert!(record_identity_failure("RealtimeQuotes", 1, true).is_none());
        for (records, identity_matches) in [(0, true), (2, true), (1, false)] {
            assert_eq!(
                record_identity_failure("RealtimeQuotes", records, identity_matches)
                    .unwrap()
                    .to_string(),
                "operation=RealtimeQuotes reason_code=record_identity_mismatch retryable=false"
            );
        }
    }

    #[test]
    fn t0_requires_one_matching_record_and_no_rejections() {
        assert!(t0_failure(1, true, 0).is_none());
        for (records, identity_matches, rejections) in
            [(0, true, 0), (2, true, 0), (1, false, 0), (1, true, 1)]
        {
            assert_eq!(
                t0_failure(records, identity_matches, rejections)
                    .unwrap()
                    .to_string(),
                "operation=T0Evidence reason_code=record_identity_or_rejection_mismatch retryable=false"
            );
        }
    }

    #[test]
    fn historical_bars_require_non_empty_records() {
        assert!(daily_failure(1).is_none());
        assert_eq!(
            daily_failure(0).unwrap().to_string(),
            "operation=HistoricalBars reason_code=records_unavailable retryable=false"
        );
    }

    #[test]
    fn typed_errors_do_not_expose_untrusted_details() {
        let sentinel = "TEST_CODE_LEAK_SENTINEL price token /tmp/private";
        let grpc = safe_grpc(
            "RealtimeQuotes",
            GrpcError::from(tonic::Status::unavailable(sentinel)),
        )
        .to_string();
        let gateway = safe_gateway(
            "HistoricalBars",
            GatewayError::unavailable("HistoricalDailyBars", None, true, sentinel),
        )
        .to_string();
        let typed_grpc = safe_grpc(
            "OrderBooks",
            GrpcError::Unavailable {
                details: Box::new(ErrorDetail {
                    reason_code: Some("provider_unavailable".to_string()),
                    retryable: Some(true),
                    request_id: Some(sentinel.to_string()),
                    ..Default::default()
                }),
            },
        )
        .to_string();

        assert_eq!(
            grpc,
            "operation=RealtimeQuotes reason_code=absent retryable=absent"
        );
        assert_eq!(
            gateway,
            "operation=HistoricalBars reason_code=no_verified_batch retryable=true"
        );
        assert_eq!(
            typed_grpc,
            "operation=OrderBooks reason_code=provider_unavailable retryable=true"
        );
        for safe in [&grpc, &gateway, &typed_grpc] {
            for forbidden in ["TEST_CODE_LEAK_SENTINEL", "price", "token", "/"] {
                assert!(!safe.contains(forbidden));
            }
        }
    }

    #[tokio::test]
    async fn loopback_connect_failure_is_nonzero_and_does_not_expose_code() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let error = run(Args {
            addr,
            code: "TEST_CODE_LEAK_SENTINEL_9F3A".to_string(),
        })
        .await
        .expect_err("no loopback server must fail the probe");
        let safe = error.to_string();

        assert!(safe.contains("operation=Connect"));
        assert!(!safe.contains("TEST_CODE_LEAK_SENTINEL_9F3A"));
    }
}
