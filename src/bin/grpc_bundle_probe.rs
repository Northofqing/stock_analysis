//! Secret-safe opening-readiness and selected native-query probe for the authenticated bundle.

use clap::{Parser, ValueEnum};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use stock_analysis::data_gateway::instrument_identity::resolve_production_equity;
use stock_analysis::data_gateway::GlobalNewsProvider;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::envelope::{QueryAdmission, QueryResult};
use stock_analysis::grpc_client::errors::GrpcError;
use stock_analysis::grpc_client::external_pb::magic::market::v1::{
    AdmissionState as ExternalAdmissionState, Capability as ExternalCapability,
    Operation as ExternalOperation,
};
use stock_analysis::grpc_client::pb::magic::market::v1::{AdmissionState, Operation};
use stock_analysis::market_domain::{EvidenceTimestamp, ProviderId, SourceEvidence};

const DIRECT_EXTERNAL_OPERATIONS: &[ExternalOperation] = &[
    ExternalOperation::SecurityMetadata,
    ExternalOperation::MarketAnnouncements,
    ExternalOperation::GlobalNews,
    ExternalOperation::InstrumentNews,
    ExternalOperation::FuturesDelivery,
    ExternalOperation::CurrentAuctionObservations,
    ExternalOperation::EconomicReleaseObservations,
    ExternalOperation::EconomicReleaseSchedule,
];

const DIRECT_GLOBAL_NEWS_PROVIDERS: [&str; 4] = ["Eastmoney", "Cailianpress", "Jin10", "ThePaper"];

const STATIC_OPENING_CAPABILITY_FAMILIES: &[(&str, &[ExternalOperation])] = &[
    ("SecurityMetadata", &[ExternalOperation::SecurityMetadata]),
    ("GlobalNews", &[ExternalOperation::GlobalNews]),
    (
        "Announcements",
        &[
            ExternalOperation::MarketAnnouncements,
            ExternalOperation::Announcements,
        ],
    ),
    (
        "BoardMemberships",
        &[
            ExternalOperation::BoardMemberships,
            ExternalOperation::BoardConstituents,
        ],
    ),
    (
        "LimitPools",
        &[
            ExternalOperation::LimitPools,
            ExternalOperation::UpperLimitPoolReview,
        ],
    ),
    ("InstrumentNews", &[ExternalOperation::InstrumentNews]),
];

#[derive(Parser)]
#[command(about = "Secret-safe authenticated market bundle readiness and native-query probe")]
struct Args {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    opening: bool,
    #[arg(long, value_enum)]
    native_operation: Option<NativeOperation>,
    /// Read-only whole-market R-08 probe through the ordinary client adapter.
    #[arg(long)]
    market_announcements_date: Option<String>,
    #[arg(long, default_value = "600396")]
    code: String,
    #[arg(long, default_value = "live")]
    stage: String,
    #[arg(long, default_value_t = 20)]
    limit: u32,
    #[arg(long)]
    country: Option<String>,
    #[arg(long)]
    start: Option<String>,
    #[arg(long)]
    end: Option<String>,
    #[arg(long)]
    year: Option<u32>,
    #[arg(long)]
    month: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum NativeOperation {
    FuturesDelivery,
    CurrentAuctionObservations,
    EconomicReleaseObservations,
    EconomicReleaseSchedule,
}

struct NativeQuerySpec {
    operation: ExternalOperation,
    params: serde_json::Value,
    provider: &'static str,
    record_provider: &'static str,
    record_schema: &'static str,
    record_version: u32,
    record_source_at: bool,
    delivery_scope: Option<(u32, u32)>,
}

const CFFEX_HOLIDAY_URL: &str =
    "https://www.gov.cn/gongbao/2025/issue_12406/material/gwygb202532.pdf";
const CFFEX_2026_PLANNED_DAYS: [u32; 12] = [16, 24, 20, 17, 15, 22, 17, 21, 18, 16, 20, 18];

fn cffex_rule_url(product: &str) -> Option<(&'static str, &'static str)> {
    match product {
        "If" => Some(("IF", "https://www.cffex.com.cn/cn/hs300.html")),
        "Ih" => Some(("IH", "https://www.cffex.com.cn/cn/sz50gzqh.html")),
        "Ic" => Some(("IC", "https://www.cffex.com.cn/cn/zz500.html")),
        "Im" => Some(("IM", "https://www.cffex.com.cn/zz1000/")),
        _ => None,
    }
}

fn futures_delivery_scope(args: &Args) -> anyhow::Result<(u32, u32)> {
    let year = args
        .year
        .ok_or_else(|| anyhow::anyhow!("--year is required"))?;
    let month = args
        .month
        .ok_or_else(|| anyhow::anyhow!("--month is required"))?;
    if year != 2026 || !(1..=12).contains(&month) {
        anyhow::bail!("FuturesDelivery probe only supports a 2026 contract month");
    }
    Ok((year, month))
}

fn native_query_spec(args: &Args, selection: NativeOperation) -> anyhow::Result<NativeQuerySpec> {
    let spec = match selection {
        NativeOperation::FuturesDelivery => {
            let (year, month) = futures_delivery_scope(args)?;
            NativeQuerySpec {
                operation: ExternalOperation::FuturesDelivery,
                params: serde_json::json!({"year": year, "month": month}),
                provider: "Cffex",
                record_provider: "Cffex",
                record_schema: "magic.market.futures_delivery_event",
                record_version: 2,
                record_source_at: false,
                delivery_scope: Some((year, month)),
            }
        }
        NativeOperation::CurrentAuctionObservations => {
            if !matches!(args.stage.as_str(), "live" | "final") {
                anyhow::bail!("auction stage must be live or final");
            }
            let instrument = resolve_production_equity(&args.code, None)
                .map_err(|error| anyhow::anyhow!("auction instrument is invalid: {error}"))?
                .instrument()
                .clone();
            NativeQuerySpec {
                operation: ExternalOperation::CurrentAuctionObservations,
                params: serde_json::json!({"instruments":[instrument],"stage":args.stage.as_str()}),
                provider: "HithinkFinance",
                record_provider: "Tonghuashun",
                record_schema: "magic.market.current_auction_observation",
                record_version: 1,
                record_source_at: false,
                delivery_scope: None,
            }
        }
        NativeOperation::EconomicReleaseObservations => {
            let request = stock_analysis::data_gateway::EconomicReleaseObservationsRequest::new(
                args.limit,
                args.country.clone(),
            )
            .map_err(|error| anyhow::anyhow!("release request is invalid: {error}"))?;
            let mut params = serde_json::json!({"limit":request.limit()});
            if let Some(country) = request.country() {
                params["country"] = serde_json::Value::String(country.to_owned());
            }
            NativeQuerySpec {
                operation: ExternalOperation::EconomicReleaseObservations,
                params,
                provider: "Jin10",
                record_provider: "Jin10",
                record_schema: "magic.market.economic_release_observation",
                record_version: 1,
                record_source_at: true,
                delivery_scope: None,
            }
        }
        NativeOperation::EconomicReleaseSchedule => {
            let start = args
                .start
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--start is required"))?;
            let end = args
                .end
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--end is required"))?;
            let start = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
                .map_err(|error| anyhow::anyhow!("--start is invalid: {error}"))?;
            let end = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
                .map_err(|error| anyhow::anyhow!("--end is invalid: {error}"))?;
            let request = stock_analysis::data_gateway::EconomicReleaseScheduleRequest::new(
                start, end, args.limit,
            )
            .map_err(|error| anyhow::anyhow!("schedule request is invalid: {error}"))?;
            NativeQuerySpec {
                operation: ExternalOperation::EconomicReleaseSchedule,
                params: serde_json::json!({
                    "start":request.start(),"end":request.end(),"limit":request.limit()
                }),
                provider: "Fred",
                record_provider: "Fred",
                record_schema: "magic.market.economic_release_schedule_entry",
                record_version: 1,
                record_source_at: false,
                delivery_scope: None,
            }
        }
    };
    Ok(spec)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FuturesDeliveryProbeRecord {
    product: String,
    contract_code: String,
    last_trading_date: Option<chrono::NaiveDate>,
    delivery_date: chrono::NaiveDate,
    method: String,
    schedule_status: String,
    date_basis: String,
    rule_url: String,
    holiday_calendar_url: String,
    evidence: SourceEvidence,
}

// This is a planned-calendar probe, not confirmation that delivery occurred.
fn validate_futures_delivery(result: &QueryResult, year: u32, month: u32) -> anyhow::Result<()> {
    if result.records.len() != 4
        || result.batch_id != format!("cffex-equity-index-planned-delivery-2026-v2:{month:02}")
    {
        anyhow::bail!("CFFEX monthly delivery batch must cover four products");
    }
    let batch_at = EvidenceTimestamp::parse_instant(&result.observed_at)
        .map_err(|_| anyhow::anyhow!("CFFEX batch observation time is invalid"))?;
    let suffix = format!("{:02}{month:02}", year % 100);
    let expected_date =
        chrono::NaiveDate::from_ymd_opt(2026, month, CFFEX_2026_PLANNED_DAYS[month as usize - 1])
            .ok_or_else(|| anyhow::anyhow!("CFFEX planned date invalid"))?;
    let mut products = BTreeSet::new();
    for payload in &result.records {
        let record: FuturesDeliveryProbeRecord = serde_json::from_slice(&payload.data)
            .map_err(|_| anyhow::anyhow!("CFFEX delivery record contract is invalid"))?;
        let (product_code, rule_url) = cffex_rule_url(&record.product)
            .ok_or_else(|| anyhow::anyhow!("CFFEX delivery product is outside IF/IH/IC/IM"))?;
        let record_at = EvidenceTimestamp::parse_instant(record.evidence.observed_at())
            .map_err(|_| anyhow::anyhow!("CFFEX record observation time is invalid"))?;
        if !products.insert(product_code)
            || record.contract_code != format!("{product_code}{suffix}")
            || record.delivery_date != expected_date
            || record.last_trading_date != Some(record.delivery_date)
            || record.method != "Cash"
            || record.schedule_status != "Planned"
            || record.date_basis != "CffexRuleAndPublishedHolidays"
            || record.rule_url != rule_url
            || record.holiday_calendar_url != CFFEX_HOLIDAY_URL
            || record.evidence.provider() != ProviderId::Cffex
            || record.evidence.source_at().is_some()
            || record.evidence.batch_id() != result.batch_id
            || record_at != batch_at
        {
            anyhow::bail!("CFFEX delivery scope or evidence conflicts with request");
        }
    }
    Ok(())
}

async fn run_native_query(
    client: &mut GrpcMarketClient,
    capabilities: &[ExternalCapability],
    spec: NativeQuerySpec,
) -> anyhow::Result<()> {
    if !capability_ready(capabilities, spec.operation) {
        anyhow::bail!("selected native operation has no admitted runtime capability");
    }
    let result = client
        .query_external_native(spec.operation, spec.params)
        .await
        .map_err(|error| structured_probe_error("native query failed", error))?;
    if result.admission != QueryAdmission::Admitted
        || !result.complete
        || !result.diagnostic_blocker.is_empty()
        || result.selected_provider != spec.provider
        || result.batch_id.is_empty()
        || result.observed_at.is_empty()
        || !result.source().starts_with("grpc-mtls:")
    {
        anyhow::bail!("native response envelope is not qualified");
    }
    if (!spec.record_source_at && !result.source_at.is_empty())
        || (spec.record_source_at && (result.records.is_empty() != result.source_at.is_empty()))
    {
        anyhow::bail!("native batch source-time policy conflicts with contract");
    }
    for record in &result.records {
        if record.schema != spec.record_schema
            || record.schema_version != spec.record_version
            || record.content_type != "application/json; charset=utf-8"
        {
            anyhow::bail!("native record schema/version/content type conflicts with contract");
        }
        let value: serde_json::Value = serde_json::from_slice(&record.data)
            .map_err(|_| anyhow::anyhow!("native record JSON is invalid"))?;
        let evidence = value
            .get("evidence")
            .ok_or_else(|| anyhow::anyhow!("native record evidence is absent"))?;
        let record_source_at = evidence
            .get("source_at")
            .ok_or_else(|| anyhow::anyhow!("native record source_at is absent"))?;
        let record_source_at_matches = if spec.record_source_at {
            record_source_at
                .as_str()
                .is_some_and(|source_at| !source_at.is_empty())
        } else {
            record_source_at.is_null()
        };
        if evidence.get("provider").and_then(serde_json::Value::as_str)
            != Some(spec.record_provider)
            || evidence.get("batch_id").and_then(serde_json::Value::as_str)
                != Some(result.batch_id.as_str())
            || evidence
                .get("observed_at")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty)
            || !record_source_at_matches
        {
            anyhow::bail!("native record evidence conflicts with contract");
        }
    }
    if let Some((year, month)) = spec.delivery_scope {
        validate_futures_delivery(&result, year, month)?;
    }
    println!(
        "native operation={:?} provider={} admission=ADMITTED complete=true records={} schema={} batch_id_present=true observed_at_present=true source_at_present={} evidence_shape=ok",
        spec.operation,
        spec.provider,
        result.records.len(),
        spec.record_schema,
        !result.source_at.is_empty(),
    );
    if spec.delivery_scope.is_some() {
        println!("r08_schedule_status=planned confirmed_delivery=false");
    }
    Ok(())
}

async fn run_market_announcements(
    client: &mut GrpcMarketClient,
    capabilities: &[ExternalCapability],
    date: &str,
    limit: u32,
) -> anyhow::Result<()> {
    let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("market announcement date must be YYYY-MM-DD"))?;
    if !(1..=300).contains(&limit) {
        anyhow::bail!("market announcement limit must be 1..=300");
    }
    if !capability_ready(capabilities, ExternalOperation::MarketAnnouncements) {
        anyhow::bail!("MarketAnnouncements has no admitted runtime capability");
    }
    let result = client
        .query(
            Operation::MarketAnnouncements,
            serde_json::json!({"start":date.to_string(),"end":date.to_string(),"limit":limit}),
        )
        .await
        .map_err(|error| structured_probe_error("MarketAnnouncements failed", error))?;
    if result.admission != QueryAdmission::Admitted
        || !result.complete
        || !result.diagnostic_blocker.is_empty()
        || result.selected_provider != "Cninfo"
        || result.batch_id.is_empty()
        || !result.source().starts_with("grpc-mtls:")
    {
        anyhow::bail!("MarketAnnouncements response envelope is not qualified");
    }
    let mut records = 0usize;
    for record in &result.records {
        if record.schema != "magic.market.announcement"
            || record.schema_version != 1
            || record.content_type != "application/json; charset=utf-8"
        {
            anyhow::bail!(
                "MarketAnnouncements record contract mismatch: schema={} version={} content_type={}",
                record.schema,
                record.schema_version,
                record.content_type
            );
        }
        let json: serde_json::Value = serde_json::from_slice(&record.data)
            .map_err(|_| anyhow::anyhow!("MarketAnnouncements record JSON invalid"))?;
        records += match json {
            serde_json::Value::Object(_) => 1,
            serde_json::Value::Array(rows) if rows.iter().all(serde_json::Value::is_object) => {
                rows.len()
            }
            _ => anyhow::bail!("MarketAnnouncements record shape invalid"),
        };
    }
    if records > limit as usize {
        anyhow::bail!("MarketAnnouncements exceeded requested limit");
    }
    println!(
        "canary operation=MarketAnnouncements admission=ADMITTED complete=true provider=Cninfo records={records} batch_id={}",
        result.batch_id
    );
    Ok(())
}

fn capability_ready(capabilities: &[ExternalCapability], operation: ExternalOperation) -> bool {
    capabilities.iter().any(|capability| {
        capability.operation == operation as i32
            && capability.repository_admission == ExternalAdmissionState::Admitted as i32
            && capability.runtime_available
    })
}

fn capability_family_ready(
    capabilities: &[ExternalCapability],
    operations: &[ExternalOperation],
) -> bool {
    operations
        .iter()
        .copied()
        .any(|operation| capability_ready(capabilities, operation))
}

fn external_contract_ready(operation: ExternalOperation) -> bool {
    DIRECT_EXTERNAL_OPERATIONS.contains(&operation)
}

fn canonical_bundle_path(path: &Path) -> anyhow::Result<PathBuf> {
    std::fs::canonicalize(path)
        .map_err(|_| anyhow::anyhow!("client-bundle directory is unavailable"))
}

fn structured_probe_error(context: &str, error: GrpcError) -> anyhow::Error {
    let detail = error.details();
    let provider = detail.provider.as_deref().unwrap_or("<absent>");
    let reason_code = detail.reason_code.as_deref().unwrap_or("<absent>");
    let retryable = detail
        .retryable
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<absent>".to_owned());
    let admission = detail
        .admission
        .map(|value| format!("{value:?}"))
        .unwrap_or_else(|| "<absent>".to_owned());
    let evidence_code = detail
        .evidence_code
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or("<absent>");
    let evidence_field = detail
        .evidence_field
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or("<absent>");
    let record_index = detail
        .record_index
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<absent>".to_owned());
    anyhow::anyhow!(
        "{context}: grpc_code={} provider={provider} reason_code={reason_code} retryable={retryable} admission={admission} evidence_code={evidence_code} evidence_field={evidence_field} record_index={record_index}",
        detail.code
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    let selected_modes = u8::from(args.opening)
        + u8::from(args.native_operation.is_some())
        + u8::from(args.market_announcements_date.is_some());
    if selected_modes != 1 {
        anyhow::bail!(
            "select exactly one of --opening, --native-operation or --market-announcements-date"
        );
    }
    let native_spec = args
        .native_operation
        .map(|selection| native_query_spec(&args, selection))
        .transpose()?;
    let bundle = canonical_bundle_path(&args.bundle)?;
    let public_inputs = stock_analysis::grpc_client::build_identity::compiled_public_inputs()
        .map_err(|error| anyhow::anyhow!("compiled public inputs unavailable: {error}"))?;
    println!(
        "compiled_public_inputs={}",
        serde_json::to_string(&public_inputs)?
    );

    let mut client = GrpcMarketClient::connect_client_bundle(&bundle)
        .await
        .map_err(|error| anyhow::anyhow!("bundle transport not ready: {error}"))?;
    let health = client
        .get_external_health()
        .await
        .map_err(|error| anyhow::anyhow!("bundle health unavailable: {error}"))?;
    println!("health live={} ready={}", health.live, health.ready);
    println!(
        "health_fields observability_present={} build_identity_present={}",
        health.observability.is_some(),
        health.build_identity.is_some()
    );
    stock_analysis::grpc_client::build_identity::qualify_public_health(&health)
        .map_err(|error| anyhow::anyhow!("bundle health is not opening-qualified: {error}"))?;
    println!("health_qualification deployment_build_identity=matched");

    let capabilities = client
        .get_external_capabilities()
        .await
        .map_err(|error| anyhow::anyhow!("bundle capabilities unavailable: {error}"))?;
    if let Some(spec) = native_spec {
        return run_native_query(&mut client, &capabilities, spec).await;
    }
    if let Some(date) = &args.market_announcements_date {
        return run_market_announcements(&mut client, &capabilities, date, args.limit).await;
    }
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

    let mut direct_failures = Vec::new();
    match client
        .query(
            Operation::SecurityMetadata,
            serde_json::json!({"instruments": [instrument.clone()]}),
        )
        .await
    {
        Ok(security) => {
            match validate_canary(&security, "magic.market.security_metadata", 1, true, true) {
                Ok(security_summary) => {
                    println!(
                        "canary operation=SecurityMetadata admission=ADMITTED complete={} records={} schemas={} fields={} evidence=ok",
                        security.complete,
                        security.records.len(),
                        security_summary.schemas,
                        security_summary.fields,
                    );
                    match stock_analysis::data_gateway::grpc_source::convert::security_identities(
                        std::slice::from_ref(&args.code),
                        &security,
                        chrono::Utc::now(),
                    ) {
                        Ok(identities) => println!(
                            "projection operation=SecurityIdentity records={} evidence=ok",
                            identities.records().len()
                        ),
                        Err(_) => {
                            eprintln!(
                                "probe_failure stage=SecurityIdentity reason_code=projection_invalid"
                            );
                            direct_failures.push("SecurityIdentity");
                        }
                    }
                }
                Err(_) => {
                    eprintln!("probe_failure stage=SecurityMetadata reason_code=contract_invalid");
                    direct_failures.push("SecurityMetadataContract");
                }
            }
        }
        Err(error) => {
            eprintln!(
                "{}",
                structured_probe_error("SecurityMetadata canary failed", error)
            );
            direct_failures.push("SecurityMetadataRpc");
        }
    }

    let news_captured_at = chrono::Local::now().fixed_offset();
    let news_captured_through = news_captured_at.with_timezone(&chrono::Utc);
    let news_end = news_captured_at.date_naive();
    let news_start = stock_analysis::calendar::prev_trading_day(news_end);
    let news = client
        .query(
            Operation::InstrumentNews,
            serde_json::json!({
                "instrument": instrument.clone(),
                "start": news_start.format("%Y-%m-%d").to_string(),
                "end": news_end.format("%Y-%m-%d").to_string(),
                "limit": 1,
                "captured_through": news_captured_at.to_rfc3339()
            }),
        )
        .await;
    match news {
        Ok(news) => match validate_canary(&news, "magic.market.news_item", 2, false, false) {
            Ok(news_summary) => {
                println!(
                    "canary operation=InstrumentNews admission=ADMITTED complete={} records={} schemas={} fields={} evidence=ok",
                    news.complete,
                    news.records.len(),
                    news_summary.schemas,
                    news_summary.fields,
                );
                match stock_analysis::data_gateway::grpc_source::convert::external_instrument_news_in_range_at(
                    &args.code,
                    &instrument,
                    &news,
                    stock_analysis::data_gateway::grpc_source::convert::ExternalInstrumentNewsRequestContext::new(
                        news_start,
                        news_end,
                        1,
                        news_captured_through,
                        chrono::Utc::now(),
                    ),
                ) {
                    Ok(news_projection) => println!(
                        "projection operation=InstrumentNews records={} evidence=ok",
                        news_projection.records().len()
                    ),
                    Err(_) => {
                        eprintln!(
                            "probe_failure stage=InstrumentNewsProjection reason_code=projection_invalid"
                        );
                        direct_failures.push("InstrumentNewsProjection");
                    }
                }
            }
            Err(_) => {
                eprintln!("probe_failure stage=InstrumentNews reason_code=contract_invalid");
                direct_failures.push("InstrumentNewsContract");
            }
        },
        Err(error) => {
            eprintln!(
                "{}",
                structured_probe_error("InstrumentNews canary failed", error)
            );
            direct_failures.push("InstrumentNewsRpc");
        }
    }

    let mut direct_global_news_failures = Vec::new();
    for provider in DIRECT_GLOBAL_NEWS_PROVIDERS {
        let Some(provider_kind) = GlobalNewsProvider::from_wire_name(provider) else {
            anyhow::bail!("direct GlobalNews provider registry is invalid");
        };
        match client
            .query(
                Operation::GlobalNews,
                serde_json::json!({"provider": provider, "limit": 1}),
            )
            .await
        {
            Ok(news) => {
                match validate_canary(&news, "magic.market.news_item", 2, false, false) {
                    Ok(summary) => match stock_analysis::data_gateway::grpc_source::convert::external_global_news(provider_kind, &news) {
                        Ok(projection) => println!(
                            "canary operation=GlobalNews provider={} admission=ADMITTED complete={} records={} schemas={} fields={} projected_records={} evidence=ok",
                            provider,
                            news.complete,
                            news.records.len(),
                            summary.schemas,
                            summary.fields,
                            projection.records().len(),
                        ),
                        Err(_) => {
                            eprintln!(
                                "probe_failure stage=GlobalNews-{}Projection reason_code=projection_invalid",
                                provider
                            );
                            direct_global_news_failures.push(provider);
                        }
                    },
                    Err(_) => {
                        eprintln!(
                            "probe_failure stage=GlobalNews-{} reason_code=contract_invalid",
                            provider
                        );
                        direct_global_news_failures.push(provider);
                    }
                }
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    structured_probe_error(
                        &format!("GlobalNews-{provider} canary failed"),
                        error,
                    )
                );
                direct_global_news_failures.push(provider);
            }
        }
    }

    // Exercise the same nine route canaries used by the production monitor.
    // The bundle path stays process-local and is never printed.
    std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", &bundle);
    stock_analysis::data_gateway::grpc_source::reset_bridge();
    let report =
        stock_analysis::data_gateway::grpc_source::external_static_opening_diagnostics()
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "static diagnostics prerequisite failed: capability={} provider={:?} reason_code={} retryable={}",
                    error.capability(),
                    error.provider(),
                    error.reason_code(),
                    error.retryable()
                )
            })?;
    for route in report.ready_routes() {
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
    for failure in report.failures() {
        println!(
            "static_route_failure route={} capability={} provider={:?} reason_code={} retryable={}",
            failure.route,
            failure.capability,
            failure.provider,
            failure.reason_code,
            failure.retryable
        );
    }
    let global_news_routes = report
        .ready_routes()
        .iter()
        .filter(|route| route.route.starts_with("GlobalNews-"))
        .count();
    let opening_ready = direct_failures.is_empty() && report.production_ready();
    println!(
        "opening_static_ready={} attempts={}/9 ready_routes={} failed_routes={} global_news={}/4 direct_global_news_failures={} attempt_order={}",
        opening_ready,
        report.ready_routes().len() + report.failures().len(),
        report.ready_routes().len(),
        report.failed_route_names(),
        global_news_routes,
        if direct_global_news_failures.is_empty() {
            "none".to_owned()
        } else {
            direct_global_news_failures.join(",")
        },
        report.attempted_route_names(),
    );
    if !opening_ready {
        anyhow::bail!(
            "opening probe failed direct_stages={} static_failed_routes={}",
            if direct_failures.is_empty() {
                "none".to_owned()
            } else {
                direct_failures.join(",")
            },
            report.failed_route_names()
        );
    }
    Ok(())
}

struct CanarySummary {
    schemas: String,
    fields: String,
}

fn validate_canary(
    result: &QueryResult,
    expected_schema: &str,
    expected_version: u32,
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
        || result.source().trim().is_empty()
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
            || record.schema_version != expected_version
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
    use prost::Message;
    fn capability(
        operation: ExternalOperation,
        admission: ExternalAdmissionState,
        runtime_available: bool,
    ) -> ExternalCapability {
        ExternalCapability {
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
                ExternalOperation::SecurityMetadata,
                ExternalAdmissionState::Unadmitted,
                true,
            ),
            capability(
                ExternalOperation::SecurityMetadata,
                ExternalAdmissionState::Admitted,
                false,
            ),
            capability(
                ExternalOperation::SecurityMetadata,
                ExternalAdmissionState::Admitted,
                true,
            ),
        ];
        assert!(capability_ready(&rows, ExternalOperation::SecurityMetadata));
        assert!(!capability_ready(&rows, ExternalOperation::InstrumentNews));
    }

    #[test]
    fn diagnostic_capability_cannot_satisfy_production_readiness() {
        let mut diagnostic = capability(
            ExternalOperation::MoneyFlows,
            ExternalAdmissionState::Unadmitted,
            true,
        );
        diagnostic.diagnostic_available = true;
        assert!(!capability_ready(
            &[diagnostic],
            ExternalOperation::MoneyFlows
        ));
    }

    #[test]
    fn opening_capability_family_accepts_one_admitted_runtime_alias() {
        for (selected, family) in [
            (
                ExternalOperation::Announcements,
                &[
                    ExternalOperation::MarketAnnouncements,
                    ExternalOperation::Announcements,
                ][..],
            ),
            (
                ExternalOperation::BoardMemberships,
                &[
                    ExternalOperation::BoardMemberships,
                    ExternalOperation::BoardConstituents,
                ][..],
            ),
            (
                ExternalOperation::LimitPools,
                &[
                    ExternalOperation::LimitPools,
                    ExternalOperation::UpperLimitPoolReview,
                ][..],
            ),
        ] {
            let rows = vec![capability(selected, ExternalAdmissionState::Admitted, true)];
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
        assert!(!operations.contains(&ExternalOperation::RealtimeQuotes));
        assert!(!operations.contains(&ExternalOperation::OrderBooks));
        assert!(!operations.contains(&ExternalOperation::T0Evidence));
    }

    #[test]
    fn direct_external_contract_allow_list_is_closed() {
        assert!(external_contract_ready(ExternalOperation::SecurityMetadata));
        assert!(external_contract_ready(ExternalOperation::GlobalNews));
        assert!(external_contract_ready(ExternalOperation::InstrumentNews));
        assert!(external_contract_ready(ExternalOperation::FuturesDelivery));
        assert!(external_contract_ready(
            ExternalOperation::CurrentAuctionObservations
        ));
        assert!(external_contract_ready(
            ExternalOperation::EconomicReleaseObservations
        ));
        assert!(external_contract_ready(
            ExternalOperation::EconomicReleaseSchedule
        ));
        assert!(!external_contract_ready(ExternalOperation::RealtimeQuotes));
        assert!(!external_contract_ready(
            ExternalOperation::BoardConstituents
        ));
        assert!(!external_contract_ready(
            ExternalOperation::UpperLimitPoolReview
        ));
    }

    #[test]
    fn native_probe_cli_selects_published_requests() {
        let auction = Args::try_parse_from([
            "probe",
            "--bundle",
            "/tmp/TEST_CODE_bundle",
            "--native-operation",
            "current-auction-observations",
            "--stage",
            "final",
        ])
        .expect("auction probe args");
        let auction_spec = native_query_spec(&auction, auction.native_operation.unwrap())
            .expect("auction request");
        assert_eq!(
            auction_spec.operation,
            ExternalOperation::CurrentAuctionObservations
        );
        assert_eq!(auction_spec.params["stage"], "final");

        let observations = Args::try_parse_from([
            "probe",
            "--bundle",
            "/tmp/TEST_CODE_bundle",
            "--native-operation",
            "economic-release-observations",
            "--limit",
            "20",
            "--country",
            "中国",
        ])
        .expect("Jin10 probe args");
        let observations_spec =
            native_query_spec(&observations, observations.native_operation.unwrap())
                .expect("Jin10 request");
        assert_eq!(
            observations_spec.operation,
            ExternalOperation::EconomicReleaseObservations
        );
        assert_eq!(
            observations_spec.params,
            serde_json::json!({"limit":20,"country":"中国"})
        );

        let schedule = Args::try_parse_from([
            "probe",
            "--bundle",
            "/tmp/TEST_CODE_bundle",
            "--native-operation",
            "economic-release-schedule",
            "--start",
            "2026-09-13",
            "--end",
            "2026-10-13",
        ])
        .expect("FRED probe args");
        let schedule_spec =
            native_query_spec(&schedule, schedule.native_operation.unwrap()).expect("FRED request");
        assert_eq!(
            schedule_spec.operation,
            ExternalOperation::EconomicReleaseSchedule
        );
        assert_eq!(
            schedule_spec.params,
            serde_json::json!({
                "start":"2026-09-13","end":"2026-10-13","limit":20
            })
        );

        let delivery = Args::try_parse_from([
            "probe",
            "--bundle",
            "/tmp/TEST_CODE_bundle",
            "--native-operation",
            "futures-delivery",
            "--year",
            "2026",
            "--month",
            "9",
        ])
        .expect("CFFEX probe args");
        let delivery_spec =
            native_query_spec(&delivery, delivery.native_operation.unwrap()).unwrap();
        assert_eq!(delivery_spec.operation, ExternalOperation::FuturesDelivery);
        assert_eq!(
            delivery_spec.params,
            serde_json::json!({"year":2026,"month":9})
        );
        assert_eq!(delivery_spec.delivery_scope, Some((2026, 9)));
    }

    #[test]
    fn futures_delivery_probe_rejects_invalid_scope_before_transport() {
        for args in [
            vec!["--year", "2026"],
            vec!["--month", "9"],
            vec!["--year", "2027", "--month", "9"],
            vec!["--year", "2026", "--month", "0"],
            vec!["--year", "2026", "--month", "13"],
        ] {
            let mut cli = vec![
                "probe",
                "--bundle",
                "/tmp/TEST_CODE_bundle",
                "--native-operation",
                "futures-delivery",
            ];
            cli.extend(args);
            let parsed = Args::try_parse_from(cli).unwrap();
            assert!(native_query_spec(&parsed, NativeOperation::FuturesDelivery).is_err());
        }
    }

    fn cffex_probe_response() -> QueryResult {
        let records = ["If", "Ih", "Ic", "Im"]
            .into_iter()
            .map(|product| {
                let code = product.to_ascii_uppercase();
                let (_, rule_url) = cffex_rule_url(product).unwrap();
                stock_analysis::grpc_client::envelope::CanonicalRecord {
                    schema: "magic.market.futures_delivery_event".to_owned(),
                    schema_version: 2,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&serde_json::json!({
                        "product":product,
                        "contract_code":format!("{code}2609"),
                        "last_trading_date":"2026-09-18",
                        "delivery_date":"2026-09-18",
                        "method":"Cash",
                        "schedule_status":"Planned",
                        "date_basis":"CffexRuleAndPublishedHolidays",
                        "rule_url":rule_url,
                        "holiday_calendar_url":CFFEX_HOLIDAY_URL,
                        "evidence":{
                            "provider":"Cffex",
                            "source_at":null,
                            "observed_at":"2026-09-27T08:00:00+08:00",
                            "batch_id":"cffex-equity-index-planned-delivery-2026-v2:09"
                        }
                    }))
                    .unwrap(),
                }
            })
            .collect();
        QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Cffex".to_owned(),
            batch_id: "cffex-equity-index-planned-delivery-2026-v2:09".to_owned(),
            complete: true,
            observed_at: "2026-09-27T08:00:00+08:00".to_owned(),
            source_at: String::new(),
            records,
            provenance:
                stock_analysis::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
                    "grpc-mtls:TEST_CODE_cffex".to_owned(),
                ),
            diagnostic_blocker: String::new(),
        }
    }

    #[test]
    fn futures_delivery_probe_requires_exact_monthly_four_product_evidence() {
        let accepted = cffex_probe_response();
        validate_futures_delivery(&accepted, 2026, 9).unwrap();

        let mut empty = cffex_probe_response();
        empty.records.clear();
        assert!(validate_futures_delivery(&empty, 2026, 9).is_err());

        let mut duplicate = cffex_probe_response();
        duplicate.records[3].data = duplicate.records[0].data.clone();
        assert!(validate_futures_delivery(&duplicate, 2026, 9).is_err());

        let mut wrong_month = accepted;
        let mut record: serde_json::Value =
            serde_json::from_slice(&wrong_month.records[0].data).unwrap();
        record["delivery_date"] = serde_json::json!("2026-10-16");
        wrong_month.records[0].data = serde_json::to_vec(&record).unwrap();
        assert!(validate_futures_delivery(&wrong_month, 2026, 9).is_err());

        let mut older_observation = cffex_probe_response();
        let mut record: serde_json::Value =
            serde_json::from_slice(&older_observation.records[0].data).unwrap();
        record["evidence"]["observed_at"] = serde_json::json!("2026-09-27T07:59:59+08:00");
        older_observation.records[0].data = serde_json::to_vec(&record).unwrap();
        assert!(validate_futures_delivery(&older_observation, 2026, 9).is_err());
    }

    #[test]
    fn direct_global_news_canary_set_is_closed_and_complete() {
        assert_eq!(
            DIRECT_GLOBAL_NEWS_PROVIDERS,
            ["Eastmoney", "Cailianpress", "Jin10", "ThePaper"]
        );
    }

    #[test]
    fn br238_probe_error_exposes_only_safe_structured_authority() {
        let wire = stock_analysis::grpc_client::pb::magic::market::v1::ErrorDetail {
            request_id: "TEST_ONLY_PRIVATE_REQUEST".to_owned(),
            operation: Operation::InstrumentNews as i32,
            provider: "Sina".to_owned(),
            reason_code: "invalid_evidence".to_owned(),
            retryable: false,
            admission: AdmissionState::Unadmitted as i32,
            evidence_code: "record_time_conflict".to_owned(),
            evidence_field: "records[0].published_at".to_owned(),
            record_index: 0,
            has_record_index: true,
        };
        let error =
            stock_analysis::grpc_client::errors::GrpcError::from(tonic::Status::with_details(
                tonic::Code::FailedPrecondition,
                "TEST_ONLY_PRIVATE_STATUS",
                wire.encode_to_vec().into(),
            ));

        let rendered = structured_probe_error("InstrumentNews canary failed", error).to_string();
        assert!(rendered.contains("reason_code=invalid_evidence"));
        assert!(rendered.contains("provider=Sina"));
        assert!(rendered.contains("admission=Unadmitted"));
        assert!(rendered.contains("evidence_code=record_time_conflict"));
        assert!(rendered.contains("evidence_field=records[0].published_at"));
        assert!(rendered.contains("record_index=0"));
        assert!(!rendered.contains("TEST_ONLY_PRIVATE"));
    }

    #[test]
    fn instrument_news_probe_preserves_missing_source_time() {
        let result = QueryResult {
            admission: stock_analysis::grpc_client::envelope::QueryAdmission::Admitted,
            selected_provider: "TEST_CODE_provider".to_string(),
            batch_id: "TEST_CODE_batch".to_string(),
            complete: true,
            observed_at: "2026-08-17T09:20:01+08:00".to_string(),
            source_at: String::new(),
            records: vec![],
            provenance:
                stock_analysis::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
                    "TEST_CODE_mtls_authority".to_string(),
                ),
            diagnostic_blocker: String::new(),
        };
        let summary = validate_canary(&result, "magic.market.news_item", 2, false, false)
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
