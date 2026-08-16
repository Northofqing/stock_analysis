//! Secret-safe opening-readiness probe for the authenticated market client bundle (BR-231).

use clap::Parser;
use std::collections::BTreeSet;
use std::path::PathBuf;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::envelope::QueryResult;
use stock_analysis::grpc_client::pb::magic::market::v1::{AdmissionState, Capability, Operation};

const DIRECT_EXTERNAL_OPERATIONS: &[Operation] =
    &[Operation::SecurityMetadata, Operation::InstrumentNews];

const OPENING_CAPABILITY_OPERATIONS: &[Operation] = &[
    Operation::RealtimeQuotes,
    Operation::OrderBooks,
    Operation::SecurityMetadata,
    Operation::GlobalNews,
    Operation::Announcements,
    Operation::MarketAnnouncements,
    Operation::BoardConstituents,
    Operation::BoardMemberships,
    Operation::LimitPools,
    Operation::InstrumentNews,
    Operation::UpperLimitPoolReview,
];

#[derive(Parser)]
#[command(about = "Secret-safe authenticated market bundle readiness probe")]
struct Args {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    opening: bool,
    #[arg(long, default_value = "600396")]
    code: String,
}

fn capability_ready(capabilities: &[Capability], operation: Operation) -> bool {
    capabilities.iter().any(|capability| {
        capability.operation == operation as i32
            && capability.repository_admission == AdmissionState::Admitted as i32
            && capability.runtime_available
    })
}

fn external_contract_ready(operation: Operation) -> bool {
    DIRECT_EXTERNAL_OPERATIONS.contains(&operation)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if !args.opening {
        anyhow::bail!("only the explicit --opening readiness profile is supported");
    }

    let mut client = GrpcMarketClient::connect_client_bundle(&args.bundle)
        .await
        .map_err(|error| anyhow::anyhow!("bundle transport not ready: {error}"))?;
    let health = client
        .get_health()
        .await
        .map_err(|error| anyhow::anyhow!("bundle health unavailable: {error}"))?;
    println!("health live={} ready={}", health.live, health.ready);
    if !health.live || !health.ready {
        anyhow::bail!("bundle health is not opening-ready");
    }

    let capabilities = client
        .get_capabilities()
        .await
        .map_err(|error| anyhow::anyhow!("bundle capabilities unavailable: {error}"))?;
    for &operation in OPENING_CAPABILITY_OPERATIONS {
        let ready = capability_ready(&capabilities, operation);
        let contract = external_contract_ready(operation);
        println!(
            "capability operation={operation:?} admitted_runtime={ready} direct_contract={contract}"
        );
        if !ready {
            anyhow::bail!("opening capability {operation:?} has no admitted runtime provider");
        }
    }

    let security = client
        .query(
            Operation::SecurityMetadata,
            serde_json::json!({"codes": [args.code.clone()]}),
        )
        .await
        .map_err(|error| anyhow::anyhow!("SecurityMetadata canary failed: {error}"))?;
    let security_summary = validate_canary(&security, "magic.market.security_metadata", true)?;
    println!(
        "canary operation=SecurityMetadata admission=ADMITTED complete={} records={} schemas={} fields={} evidence=ok",
        security.complete,
        security.records.len(),
        security_summary.schemas,
        security_summary.fields,
    );
    let identities = stock_analysis::data_gateway::grpc_source::convert::security_identities(
        std::slice::from_ref(&args.code),
        &security,
        chrono::Utc::now(),
    )
    .map_err(|error| anyhow::anyhow!("SecurityMetadata identity projection failed: {error}"))?;
    println!(
        "projection operation=SecurityIdentity records={} evidence=ok",
        identities.records().len()
    );

    let news = client
        .query(
            Operation::InstrumentNews,
            serde_json::json!({"codes": [args.code], "limit": 1}),
        )
        .await
        .map_err(|error| anyhow::anyhow!("InstrumentNews canary failed: {error}"))?;
    let news_summary = validate_canary(&news, "magic.market.news_item", false)?;
    println!(
        "canary operation=InstrumentNews admission=ADMITTED complete={} records={} schemas={} fields={} evidence=ok",
        news.complete,
        news.records.len(),
        news_summary.schemas,
        news_summary.fields,
    );
    println!("opening_bundle_ready=true");
    Ok(())
}

struct CanarySummary {
    schemas: String,
    fields: String,
}

fn validate_canary(
    result: &QueryResult,
    expected_schema: &str,
    allow_partial: bool,
) -> anyhow::Result<CanarySummary> {
    if result.admission != AdmissionState::Admitted {
        anyhow::bail!("canary response is not admitted");
    }
    if !result.diagnostic_blocker.is_empty() {
        anyhow::bail!("canary response is diagnostic, not production data");
    }
    if !allow_partial && !result.complete {
        anyhow::bail!("canary response is incomplete");
    }
    if result.selected_provider.trim().is_empty()
        || result.batch_id.trim().is_empty()
        || result.source.trim().is_empty()
        || result.source_at.trim().is_empty()
        || result.observed_at.trim().is_empty()
    {
        anyhow::bail!("canary evidence identity is incomplete");
    }

    if result.records.is_empty() {
        return Ok(CanarySummary {
            schemas: "verified-empty".to_string(),
            fields: "none".to_string(),
        });
    }

    let mut schemas = BTreeSet::new();
    let mut fields = BTreeSet::new();
    for record in &result.records {
        if record.schema != expected_schema
            || record.schema_version != 1
            || record.content_type != "application/json; charset=utf-8"
        {
            anyhow::bail!("canary record contract is unknown");
        }
        let object: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&record.data)
                .map_err(|_| anyhow::anyhow!("canary record is not a JSON object"))?;
        schemas.insert(format!("{}@{}", record.schema, record.schema_version));
        fields.extend(object.into_iter().map(|(key, _)| key));
    }
    Ok(CanarySummary {
        schemas: schemas.into_iter().collect::<Vec<_>>().join(","),
        fields: fields.into_iter().collect::<Vec<_>>().join(","),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capability(
        operation: Operation,
        admission: AdmissionState,
        runtime_available: bool,
    ) -> Capability {
        Capability {
            operation: operation as i32,
            repository_admission: admission as i32,
            runtime_available,
            provider: "TEST_CODE_provider".to_string(),
            exact_scope: "TEST_CODE_scope".to_string(),
            blocker: String::new(),
            diagnostic_available: false,
        }
    }

    #[test]
    fn opening_capability_requires_admitted_runtime_provider() {
        let rows = vec![
            capability(
                Operation::SecurityMetadata,
                AdmissionState::Unadmitted,
                true,
            ),
            capability(Operation::SecurityMetadata, AdmissionState::Admitted, false),
            capability(Operation::SecurityMetadata, AdmissionState::Admitted, true),
        ];
        assert!(capability_ready(&rows, Operation::SecurityMetadata));
        assert!(!capability_ready(&rows, Operation::InstrumentNews));
    }

    #[test]
    fn diagnostic_capability_cannot_satisfy_production_readiness() {
        let mut diagnostic = capability(Operation::MoneyFlows, AdmissionState::Unadmitted, true);
        diagnostic.diagnostic_available = true;
        assert!(!capability_ready(&[diagnostic], Operation::MoneyFlows));
    }

    #[test]
    fn direct_external_contract_allow_list_is_closed() {
        assert!(external_contract_ready(Operation::SecurityMetadata));
        assert!(external_contract_ready(Operation::InstrumentNews));
        assert!(!external_contract_ready(Operation::RealtimeQuotes));
        assert!(!external_contract_ready(Operation::BoardConstituents));
        assert!(!external_contract_ready(Operation::UpperLimitPoolReview));
    }
}
