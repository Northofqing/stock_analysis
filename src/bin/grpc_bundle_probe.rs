//! Secret-safe opening-readiness probe for the authenticated market client bundle (BR-238).

use clap::Parser;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use stock_analysis::data_gateway::instrument_identity::resolve_production_equity;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::envelope::QueryResult;
use stock_analysis::grpc_client::pb::magic::market::v1::{AdmissionState, Capability, Operation};

const DIRECT_EXTERNAL_OPERATIONS: &[Operation] =
    &[Operation::SecurityMetadata, Operation::InstrumentNews];

const STATIC_OPENING_CAPABILITY_FAMILIES: &[(&str, &[Operation])] = &[
    ("SecurityMetadata", &[Operation::SecurityMetadata]),
    ("GlobalNews", &[Operation::GlobalNews]),
    (
        "Announcements",
        &[Operation::MarketAnnouncements, Operation::Announcements],
    ),
    (
        "BoardMemberships",
        &[Operation::BoardMemberships, Operation::BoardConstituents],
    ),
    (
        "LimitPools",
        &[Operation::LimitPools, Operation::UpperLimitPoolReview],
    ),
    ("InstrumentNews", &[Operation::InstrumentNews]),
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

fn capability_family_ready(capabilities: &[Capability], operations: &[Operation]) -> bool {
    operations
        .iter()
        .copied()
        .any(|operation| capability_ready(capabilities, operation))
}

fn external_contract_ready(operation: Operation) -> bool {
    DIRECT_EXTERNAL_OPERATIONS.contains(&operation)
}

fn canonical_bundle_path(path: &Path) -> anyhow::Result<PathBuf> {
    std::fs::canonicalize(path)
        .map_err(|_| anyhow::anyhow!("client-bundle directory is unavailable"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    if !args.opening {
        anyhow::bail!("only the explicit --opening readiness profile is supported");
    }
    let bundle = canonical_bundle_path(&args.bundle)?;

    let mut client = GrpcMarketClient::connect_client_bundle(&bundle)
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
    for &(family, operations) in STATIC_OPENING_CAPABILITY_FAMILIES {
        let ready = capability_family_ready(&capabilities, operations);
        let alternatives = operations
            .iter()
            .map(|operation| format!("{operation:?}"))
            .collect::<Vec<_>>()
            .join("|");
        println!(
            "capability_family family={family} alternatives={alternatives} admitted_runtime={ready}"
        );
        if !ready {
            anyhow::bail!("opening capability family {family} has no admitted runtime provider");
        }
        for &operation in operations {
            println!(
                "capability operation={operation:?} admitted_runtime={} direct_contract={}",
                capability_ready(&capabilities, operation),
                external_contract_ready(operation)
            );
        }
    }

    let instrument = resolve_production_equity(&args.code, None)
        .map_err(|error| anyhow::anyhow!("canary instrument is invalid: {error}"))?
        .instrument()
        .clone();

    let security = client
        .query(
            Operation::SecurityMetadata,
            serde_json::json!({"instruments": [instrument.clone()]}),
        )
        .await
        .map_err(|error| anyhow::anyhow!("SecurityMetadata canary failed: {error}"))?;
    let security_summary =
        validate_canary(&security, "magic.market.security_metadata", true, true)?;
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

    let news_end = chrono::Local::now().date_naive();
    let news_start = stock_analysis::calendar::prev_trading_day(news_end);
    let news = client
        .query(
            Operation::InstrumentNews,
            serde_json::json!({
                "instrument": instrument.clone(),
                "start": news_start.format("%Y-%m-%d").to_string(),
                "end": news_end.format("%Y-%m-%d").to_string(),
                "limit": 1
            }),
        )
        .await
        .map_err(|error| anyhow::anyhow!("InstrumentNews canary failed: {error}"))?;
    let news_summary = validate_canary(&news, "magic.market.news_item", false, false)?;
    println!(
        "canary operation=InstrumentNews admission=ADMITTED complete={} records={} schemas={} fields={} evidence=ok",
        news.complete,
        news.records.len(),
        news_summary.schemas,
        news_summary.fields,
    );
    let news_projection =
        stock_analysis::data_gateway::grpc_source::convert::external_instrument_news_in_range_at(
            &args.code,
            &instrument,
            news_start,
            news_end,
            1,
            &news,
            chrono::Utc::now(),
        )
        .map_err(|error| anyhow::anyhow!("InstrumentNews production projection failed: {error}"))?;
    println!(
        "projection operation=InstrumentNews records={} evidence=ok",
        news_projection.records().len()
    );

    // Exercise the same nine route canaries used by the production monitor.
    // The bundle path stays process-local and is never printed.
    std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", &bundle);
    std::env::set_var("DATA_GATEWAY_GRPC", "1");
    stock_analysis::data_gateway::grpc_source::reset_bridge();
    let report = stock_analysis::data_gateway::grpc_source::external_static_opening_readiness()
        .await
        .map_err(|error| anyhow::anyhow!("production static readiness failed: {error}"))?;
    for route in &report.routes {
        println!(
            "static_route route={} profile={} provider={:?} source_present={} source_at_present={} observed_at_present={} batch_id_present={} records={}",
            route.route,
            route.profile,
            route.provider,
            !route.source.trim().is_empty(),
            route.source_at.is_some(),
            !route.observed_at.trim().is_empty(),
            !route.batch_id.trim().is_empty(),
            route.records,
        );
    }
    println!(
        "opening_static_ready=true routes={}/9 global_news={}/4 route_names={} degraded_routes={}",
        report.routes.len(),
        report.global_news_routes(),
        report.route_names(),
        report.degraded_route_names()
    );
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
    require_source_at: bool,
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
        || result.observed_at.trim().is_empty()
    {
        anyhow::bail!("canary evidence identity is incomplete");
    }
    if require_source_at && result.source_at.trim().is_empty() {
        anyhow::bail!("canary provider source time is required for this operation");
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
    fn opening_capability_family_accepts_one_admitted_runtime_alias() {
        for (selected, family) in [
            (
                Operation::Announcements,
                &[Operation::MarketAnnouncements, Operation::Announcements][..],
            ),
            (
                Operation::BoardMemberships,
                &[Operation::BoardMemberships, Operation::BoardConstituents][..],
            ),
            (
                Operation::LimitPools,
                &[Operation::LimitPools, Operation::UpperLimitPoolReview][..],
            ),
        ] {
            let rows = vec![capability(selected, AdmissionState::Admitted, true)];
            assert!(capability_family_ready(&rows, family));
        }
    }

    #[test]
    fn static_probe_does_not_require_live_session_capabilities() {
        let operations = STATIC_OPENING_CAPABILITY_FAMILIES
            .iter()
            .flat_map(|(_, operations)| operations.iter())
            .copied()
            .collect::<Vec<_>>();
        assert!(!operations.contains(&Operation::RealtimeQuotes));
        assert!(!operations.contains(&Operation::OrderBooks));
        assert!(!operations.contains(&Operation::T0Evidence));
    }

    #[test]
    fn direct_external_contract_allow_list_is_closed() {
        assert!(external_contract_ready(Operation::SecurityMetadata));
        assert!(external_contract_ready(Operation::InstrumentNews));
        assert!(!external_contract_ready(Operation::RealtimeQuotes));
        assert!(!external_contract_ready(Operation::BoardConstituents));
        assert!(!external_contract_ready(Operation::UpperLimitPoolReview));
    }

    #[test]
    fn instrument_news_probe_preserves_missing_source_time() {
        let result = QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: "TEST_CODE_provider".to_string(),
            batch_id: "TEST_CODE_batch".to_string(),
            complete: true,
            observed_at: "2026-08-17T09:20:01+08:00".to_string(),
            source_at: String::new(),
            records: vec![],
            source: "TEST_CODE_mtls_authority".to_string(),
            diagnostic_blocker: String::new(),
        };
        let summary = validate_canary(&result, "magic.market.news_item", false, false)
            .expect("InstrumentNews may truthfully omit provider source_at");
        assert_eq!(summary.schemas, "verified-empty");
    }

    #[test]
    fn relative_bundle_directory_is_canonicalized_before_production_reuse() {
        let cwd = std::env::current_dir().expect("TEST_CODE current directory");
        let unique = format!(
            "TEST_CODE_grpc_bundle_probe_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("TEST_CODE system clock")
                .as_nanos()
        );
        let bundle = cwd.join(unique);
        std::fs::create_dir(&bundle).expect("TEST_CODE bundle directory");
        let relative = bundle
            .strip_prefix(&cwd)
            .expect("TEST_CODE relative bundle path");

        let canonical = canonical_bundle_path(relative).expect("relative path is normalized");

        assert!(canonical.is_absolute());
        assert_eq!(canonical, bundle.canonicalize().unwrap());
        std::fs::remove_dir(&bundle).expect("TEST_CODE cleanup bundle directory");
    }
}
