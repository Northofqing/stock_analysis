use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;

use stock_analysis::data_gateway::market_capabilities::MarketSecurityIdentity;
use stock_analysis::data_gateway::{
    BatchEvidence, GatewayBatch, MarketCapabilitiesGateway, SinaInstrumentNewsGateway,
    SinaInstrumentNewsRecord,
};
use stock_analysis::magic_compat::ProviderId;
use stock_analysis::pipeline::chain_analysis::p01_projection::{
    acquire_and_persist_p01_chain, P01CompletedDayEvidence,
};

const P01_SOURCE_BINDING_SCHEMA: &str = "P01_SOURCE_BINDING_V1";
const P01_TEMPLATE_ID: &str = "preopen_news_hot_v1";
const P01_LIMIT_POOL_REQUEST_LIMIT: u32 = 200;
const P01_HEAD_LIMIT: usize = 3;
const P01_REQUEST_HASH_DOMAIN: &[u8] = b"P01_LIMIT_POOLS_REQUEST_V1\0";
const P01_IDENTITY_RECORD_HASH_DOMAIN: &[u8] = b"P01_SECURITY_IDENTITY_RECORD_V1\0";
const P01_NEWS_RECORD_HASH_DOMAIN: &[u8] = b"P01_INSTRUMENT_NEWS_RECORD_V1\0";
const P01_DELIVERY_SUBJECT_DOMAIN: &[u8] = b"P01_DELIVERY_SUBJECT_V1\0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct P01Failure {
    reason_code: &'static str,
    retryable: bool,
    stage: &'static str,
    business_date: Option<NaiveDate>,
    evidence_date: Option<NaiveDate>,
    source_evidence_sha256: Option<String>,
}

impl P01Failure {
    pub const fn terminal(reason_code: &'static str, stage: &'static str) -> Self {
        Self {
            reason_code,
            retryable: false,
            stage,
            business_date: None,
            evidence_date: None,
            source_evidence_sha256: None,
        }
    }

    const fn with_business_date(mut self, business_date: NaiveDate) -> Self {
        self.business_date = Some(business_date);
        self
    }

    pub(crate) const fn for_context(
        reason_code: &'static str,
        retryable: bool,
        stage: &'static str,
        context: P01BusinessContext,
    ) -> Self {
        Self {
            reason_code,
            retryable,
            stage,
            business_date: Some(context.business_date),
            evidence_date: Some(context.evidence_date),
            source_evidence_sha256: None,
        }
    }

    fn with_source_evidence_sha256(mut self, source_evidence_sha256: String) -> Self {
        debug_assert!(is_sha256_hex(&source_evidence_sha256));
        self.source_evidence_sha256 = Some(source_evidence_sha256);
        self
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn stage(&self) -> &'static str {
        self.stage
    }

    pub const fn business_date(&self) -> Option<NaiveDate> {
        self.business_date
    }

    pub const fn evidence_date(&self) -> Option<NaiveDate> {
        self.evidence_date
    }

    pub fn source_evidence_sha256(&self) -> Option<&str> {
        self.source_evidence_sha256.as_deref()
    }
}

impl fmt::Display for P01Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "P-01 failed reason_code={} retryable={} stage={} business_date={:?} evidence_date={:?}",
            self.reason_code,
            self.retryable,
            self.stage,
            self.business_date,
            self.evidence_date
        )
    }
}

impl std::error::Error for P01Failure {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct P01BusinessContext {
    pub business_date: NaiveDate,
    pub evidence_date: NaiveDate,
}

impl P01BusinessContext {
    pub fn new(business_date: NaiveDate) -> Result<Self, P01Failure> {
        match stock_analysis::calendar::verified_a_share_trading_day(business_date) {
            Ok(true) => {}
            Ok(false) => {
                return Err(
                    P01Failure::terminal("p01_business_date_not_trading", "calendar")
                        .with_business_date(business_date),
                );
            }
            Err(_) => {
                return Err(
                    P01Failure::terminal("p01_trading_calendar_unavailable", "calendar")
                        .with_business_date(business_date),
                );
            }
        }
        let evidence_date = stock_analysis::calendar::verified_prev_a_share_trading_day(
            business_date,
        )
        .map_err(|_| {
            P01Failure::terminal("p01_trading_calendar_unavailable", "calendar")
                .with_business_date(business_date)
        })?;

        Ok(Self {
            business_date,
            evidence_date,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum P01ExecutionMode {
    Scheduled,
    Compensation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum P01Due {
    Due(P01BusinessContext),
    NotDue(P01NotDueReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum P01NotDueReason {
    NonTradingDay,
    CalendarUnavailable,
    BeforeWindow,
    ScheduledWindowClosed,
    CompensationBeforeWindowClosed,
    BusinessDateMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum P01RunOutcome {
    Delivered {
        decision_identity: String,
        receipt_sha256: String,
    },
    AlreadyDelivered {
        decision_identity: String,
    },
    AwaitingReconciliation {
        attempt_identity: String,
    },
    RetryableFailure(P01Failure),
    TerminalFailure(P01Failure),
}

impl P01RunOutcome {
    fn with_source_evidence_sha256(mut self, source_evidence_sha256: String) -> Self {
        match &mut self {
            Self::RetryableFailure(failure) | Self::TerminalFailure(failure) => {
                failure.source_evidence_sha256 = Some(source_evidence_sha256);
            }
            _ => {}
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum P01RenderMode {
    Scheduled,
    Compensation,
}

#[derive(Clone, Debug)]
pub struct P01BoundNews {
    pub title: String,
    pub source_name: String,
    pub published_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct P01BoundHead {
    pub concept: String,
    pub code: String,
    pub name: String,
    pub news: Vec<P01BoundNews>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct P01LimitPoolsBinding {
    request_kind: &'static str,
    request_trading_date: String,
    request_limit: u32,
    request_hash: String,
    provider: ProviderId,
    source: String,
    source_at: String,
    observed_at: String,
    batch_id: String,
    ordered_record_hashes: Vec<String>,
    record_count: usize,
    verified_empty: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct P01ChainDailyBinding {
    source_at: String,
    limit_pool_batch_id: String,
    ordered_limit_pool_record_hashes: Vec<String>,
    ordered_row_hashes: Vec<String>,
    record_count: usize,
    persistence_receipt: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct P01SecurityIdentityBinding {
    requested_codes: Vec<String>,
    resolved_codes: Vec<String>,
    provider: ProviderId,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    ordered_record_hashes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct P01InstrumentNewsBinding {
    code: String,
    range_start: String,
    range_end: String,
    provider: ProviderId,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    ordered_record_hashes: Vec<String>,
    record_count: usize,
    verified_empty: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct P01ExcludedLimitPoolRecord {
    record_hash: String,
    reason_code: &'static str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct P01CanonicalSourceBinding<'a> {
    schema_version: &'static str,
    business_date: String,
    evidence_date: String,
    template_id: &'static str,
    schedule_occurrence_identity: String,
    render_mode: P01RenderMode,
    captured_observed_at: String,
    limit_pools: &'a P01LimitPoolsBinding,
    chain_daily: &'a P01ChainDailyBinding,
    ordered_head_codes: &'a [String],
    security_identity: &'a P01SecurityIdentityBinding,
    instrument_news: &'a [P01InstrumentNewsBinding],
    excluded_limit_pool_records: &'a [P01ExcludedLimitPoolRecord],
    rendered_content_sha256: String,
}

#[derive(Clone, Debug)]
pub struct P01InputBinding {
    pub context: P01BusinessContext,
    pub captured_observed_at: DateTime<Local>,
    pub heads: Vec<P01BoundHead>,
    limit_pools: P01LimitPoolsBinding,
    chain_daily: P01ChainDailyBinding,
    ordered_head_codes: Vec<String>,
    security_identity: P01SecurityIdentityBinding,
    instrument_news: Vec<P01InstrumentNewsBinding>,
    excluded_limit_pool_records: Vec<P01ExcludedLimitPoolRecord>,
}

impl P01InputBinding {
    pub fn schedule_occurrence_identity(&self) -> String {
        format!("p01:{}", self.context.business_date)
    }

    pub fn canonical_source_bytes(
        &self,
        mode: P01RenderMode,
        rendered_text: &str,
    ) -> Result<Vec<u8>, P01Failure> {
        if rendered_text.trim().is_empty() {
            return Err(P01Failure::for_context(
                "p01_rendered_content_empty",
                false,
                "binding",
                self.context,
            ));
        }
        serde_json::to_vec(&P01CanonicalSourceBinding {
            schema_version: P01_SOURCE_BINDING_SCHEMA,
            business_date: self.context.business_date.format("%Y-%m-%d").to_string(),
            evidence_date: self.context.evidence_date.format("%Y-%m-%d").to_string(),
            template_id: P01_TEMPLATE_ID,
            schedule_occurrence_identity: self.schedule_occurrence_identity(),
            render_mode: mode,
            captured_observed_at: self
                .captured_observed_at
                .to_rfc3339_opts(SecondsFormat::Secs, false),
            limit_pools: &self.limit_pools,
            chain_daily: &self.chain_daily,
            ordered_head_codes: &self.ordered_head_codes,
            security_identity: &self.security_identity,
            instrument_news: &self.instrument_news,
            excluded_limit_pool_records: &self.excluded_limit_pool_records,
            rendered_content_sha256: sha256_hex(rendered_text.as_bytes()),
        })
        .map_err(|_| {
            P01Failure::for_context(
                "p01_source_binding_serialization_failed",
                false,
                "binding",
                self.context,
            )
        })
    }

    pub fn delivery_subject_hash(
        &self,
        mode: P01RenderMode,
        rendered_text: &str,
    ) -> Result<String, P01Failure> {
        self.canonical_source_bytes(mode, rendered_text)
            .map(|bytes| domain_sha256_hex(P01_DELIVERY_SUBJECT_DOMAIN, &bytes))
    }

    fn source_evidence_fingerprint(
        &self,
        mode: P01RenderMode,
        rendered_text: &str,
    ) -> Result<String, P01Failure> {
        self.canonical_source_bytes(mode, rendered_text)
            .map(|bytes| sha256_hex(&bytes))
    }

    #[cfg(test)]
    pub(crate) fn complete_test_input() -> Self {
        let context = P01BusinessContext::new(
            NaiveDate::from_ymd_opt(2026, 8, 18).expect("valid TEST_CODE business date"),
        )
        .expect("valid TEST_CODE P-01 context");
        let captured_observed_at = DateTime::parse_from_rfc3339("2026-08-18T15:30:00+08:00")
            .expect("valid TEST_CODE captured time")
            .with_timezone(&Local);
        let code = "TEST_CODE_000001".to_string();
        let limit_pool_batch = "TEST_CODE_P01_LIMIT_POOL_BATCH".to_string();
        let identity_batch = "TEST_CODE_P01_IDENTITY_BATCH".to_string();
        let news_batch = "TEST_CODE_P01_NEWS_BATCH".to_string();
        let record_hash = "a".repeat(64);
        Self {
            context,
            captured_observed_at,
            heads: vec![P01BoundHead {
                concept: "TEST_CODE_AI_CHAIN".to_string(),
                code: code.clone(),
                name: "TEST_CODE_IDENTITY_NAME".to_string(),
                news: vec![P01BoundNews {
                    title: "TEST_CODE_PROVIDER_HEADLINE".to_string(),
                    source_name: "TEST_CODE_PROVIDER".to_string(),
                    published_at: DateTime::parse_from_rfc3339("2026-08-18T01:00:00Z")
                        .expect("valid TEST_CODE published time")
                        .with_timezone(&Utc),
                }],
            }],
            limit_pools: P01LimitPoolsBinding {
                request_kind: "Upper",
                request_trading_date: "2026-08-17".to_string(),
                request_limit: P01_LIMIT_POOL_REQUEST_LIMIT,
                request_hash: {
                    #[derive(Serialize)]
                    struct ExactRequest<'a> {
                        kind: &'static str,
                        trading_date: &'a str,
                        limit: u32,
                    }
                    let request = serde_json::to_vec(&ExactRequest {
                        kind: "Upper",
                        trading_date: "2026-08-17",
                        limit: P01_LIMIT_POOL_REQUEST_LIMIT,
                    })
                    .expect("serialize TEST_CODE P-01 request");
                    domain_sha256_hex(P01_REQUEST_HASH_DOMAIN, &request)
                },
                provider: ProviderId::Tdx,
                source: "TEST_CODE_LIMIT_POOL_SOURCE".to_string(),
                source_at: "2026-08-17".to_string(),
                observed_at: "1787000000.000000000".to_string(),
                batch_id: limit_pool_batch.clone(),
                ordered_record_hashes: vec![record_hash.clone()],
                record_count: 1,
                verified_empty: false,
            },
            chain_daily: P01ChainDailyBinding {
                source_at: "2026-08-17".to_string(),
                limit_pool_batch_id: limit_pool_batch,
                ordered_limit_pool_record_hashes: vec![record_hash.clone()],
                ordered_row_hashes: vec!["c".repeat(64)],
                record_count: 1,
                persistence_receipt: "d".repeat(64),
            },
            ordered_head_codes: vec![code.clone()],
            security_identity: P01SecurityIdentityBinding {
                requested_codes: vec![code.clone()],
                resolved_codes: vec![code.clone()],
                provider: ProviderId::Tdx,
                source: "TEST_CODE_IDENTITY_SOURCE".to_string(),
                source_at: Some("2026-08-18T01:00:00Z".to_string()),
                observed_at: "1787000000.000000000".to_string(),
                batch_id: identity_batch,
                ordered_record_hashes: vec!["e".repeat(64)],
            },
            instrument_news: vec![P01InstrumentNewsBinding {
                code,
                range_start: "2026-08-17".to_string(),
                range_end: "2026-08-18".to_string(),
                provider: ProviderId::Sina,
                source: "TEST_CODE_NEWS_SOURCE".to_string(),
                source_at: Some("2026-08-18T01:00:00Z".to_string()),
                observed_at: "1787000000.000000000".to_string(),
                batch_id: news_batch,
                ordered_record_hashes: vec!["f".repeat(64)],
                record_count: 1,
                verified_empty: false,
            }],
            excluded_limit_pool_records: vec![],
        }
    }
}

/// Acquire and bind all source-backed P-01 inputs for one captured execution time.
///
/// The gRPC InstrumentNews adapter authorizes an inclusive local-date range. This
/// owner additionally rejects the whole result if any row is later than the one
/// captured `observed_at`; a server-now response is never silently truncated.
pub async fn load_p01_input_binding(
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
) -> Result<P01InputBinding, P01Failure> {
    validate_captured_observed_at(context, observed_at)?;
    let completed =
        tokio::task::spawn_blocking(move || acquire_and_persist_p01_chain(context.evidence_date))
            .await
            .map_err(|_| {
                P01Failure::for_context(
                    "p01_limit_pool_projection_join_failed",
                    true,
                    "limit_pools",
                    context,
                )
            })?
            .map_err(|error| {
                P01Failure::for_context(
                    error.reason_code(),
                    error.retryable(),
                    "limit_pools",
                    context,
                )
            })?;

    let head_codes = derive_head_codes(context, &completed)?;
    let identities = MarketCapabilitiesGateway::new()
        .security_identities(&head_codes)
        .await
        .map_err(|error| gateway_failure(context, "security_identity", error))?;

    let range_start = local_start_of_day(context, context.evidence_date)?;
    let gateway = SinaInstrumentNewsGateway::new();
    let tasks = head_codes.iter().cloned().map(|code| async move {
        gateway
            .instrument_news_in_range(
                &code,
                range_start.with_timezone(&Utc),
                observed_at.with_timezone(&Utc),
            )
            .await
            .map(|batch| (code, batch))
    });
    let mut news_batches = Vec::with_capacity(head_codes.len());
    for result in futures::future::join_all(tasks).await {
        news_batches
            .push(result.map_err(|error| gateway_failure(context, "instrument_news", error))?);
    }

    bind_p01_sources(context, observed_at, completed, identities, news_batches)
}

pub fn bind_p01_sources(
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
    completed: P01CompletedDayEvidence,
    identities: GatewayBatch<MarketSecurityIdentity>,
    news_batches: Vec<(String, GatewayBatch<SinaInstrumentNewsRecord>)>,
) -> Result<P01InputBinding, P01Failure> {
    validate_captured_observed_at(context, observed_at)?;
    let head_codes = derive_head_codes(context, &completed)?;
    let limit_pools = bind_limit_pools(context, &completed)?;
    let (security_identity, identity_names) =
        bind_security_identities(context, &head_codes, identities)?;
    let (instrument_news, news_by_code, total_news_records) =
        bind_instrument_news(context, observed_at, &head_codes, news_batches)?;
    if total_news_records == 0 {
        return Err(P01Failure::for_context(
            "p01_instrument_news_no_real_records",
            false,
            "instrument_news",
            context,
        ));
    }

    let mut heads = Vec::with_capacity(head_codes.len());
    for (row, code) in completed.chain_rows.iter().zip(&head_codes) {
        let name = identity_names.get(code).cloned().ok_or_else(|| {
            P01Failure::for_context(
                "p01_security_identity_exact_set_mismatch",
                false,
                "security_identity",
                context,
            )
        })?;
        let news = news_by_code.get(code).cloned().ok_or_else(|| {
            P01Failure::for_context(
                "p01_instrument_news_exact_set_mismatch",
                false,
                "instrument_news",
                context,
            )
        })?;
        heads.push(P01BoundHead {
            concept: row.concept.clone(),
            code: code.clone(),
            name,
            news,
        });
    }

    let projection = &completed.projection;
    let chain_daily = P01ChainDailyBinding {
        source_at: context.evidence_date.format("%Y-%m-%d").to_string(),
        limit_pool_batch_id: projection.limit_pool_batch_id.clone(),
        ordered_limit_pool_record_hashes: projection.ordered_limit_pool_record_hashes.clone(),
        ordered_row_hashes: projection.ordered_chain_row_hashes.clone(),
        record_count: completed.chain_rows.len(),
        persistence_receipt: projection.persistence_receipt_sha256.clone(),
    };
    let excluded_limit_pool_records = projection
        .excluded_record_hashes
        .iter()
        .map(|(record_hash, reason_code)| P01ExcludedLimitPoolRecord {
            record_hash: record_hash.clone(),
            reason_code,
        })
        .collect();

    Ok(P01InputBinding {
        context,
        captured_observed_at: observed_at,
        heads,
        limit_pools,
        chain_daily,
        ordered_head_codes: head_codes,
        security_identity,
        instrument_news,
        excluded_limit_pool_records,
    })
}

fn validate_captured_observed_at(
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
) -> Result<(), P01Failure> {
    if observed_at.date_naive() != context.business_date {
        return Err(P01Failure::for_context(
            "p01_observed_at_business_date_mismatch",
            false,
            "schedule",
            context,
        ));
    }
    Ok(())
}

fn local_start_of_day(
    context: P01BusinessContext,
    date: NaiveDate,
) -> Result<DateTime<Local>, P01Failure> {
    let local = date
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Local.from_local_datetime(&value).single());
    local.ok_or_else(|| {
        P01Failure::for_context(
            "p01_local_date_range_unrepresentable",
            false,
            "instrument_news",
            context,
        )
    })
}

fn derive_head_codes(
    context: P01BusinessContext,
    completed: &P01CompletedDayEvidence,
) -> Result<Vec<String>, P01Failure> {
    let projection = &completed.projection;
    if projection.evidence_date != context.evidence_date
        || projection.ordered_chain_row_hashes.len() != completed.chain_rows.len()
        || completed.chain_rows.is_empty()
        || projection
            .ordered_chain_row_hashes
            .iter()
            .any(|hash| !is_sha256_hex(hash))
        || !is_sha256_hex(&projection.persistence_receipt_sha256)
    {
        return Err(P01Failure::for_context(
            "p01_chain_projection_binding_mismatch",
            false,
            "chain_daily",
            context,
        ));
    }
    let mut codes = Vec::with_capacity(completed.chain_rows.len().min(P01_HEAD_LIMIT));
    let mut seen = HashSet::with_capacity(P01_HEAD_LIMIT);
    for row in completed.chain_rows.iter().take(P01_HEAD_LIMIT) {
        if row.date != context.evidence_date.format("%Y-%m-%d").to_string()
            || row.concept.trim().is_empty()
        {
            return Err(P01Failure::for_context(
                "p01_chain_projection_binding_mismatch",
                false,
                "chain_daily",
                context,
            ));
        }
        let stocks = serde_json::from_str::<Vec<String>>(&row.stocks).map_err(|_| {
            P01Failure::for_context("p01_chain_stocks_invalid", false, "chain_daily", context)
        })?;
        let code = stocks
            .first()
            .map(String::as_str)
            .filter(|code| valid_p01_code(code))
            .ok_or_else(|| {
                P01Failure::for_context("p01_chain_head_missing", false, "chain_daily", context)
            })?;
        if !seen.insert(code.to_owned()) {
            return Err(P01Failure::for_context(
                "p01_chain_head_duplicate",
                false,
                "chain_daily",
                context,
            ));
        }
        codes.push(code.to_owned());
    }
    Ok(codes)
}

fn bind_limit_pools(
    context: P01BusinessContext,
    completed: &P01CompletedDayEvidence,
) -> Result<P01LimitPoolsBinding, P01Failure> {
    let evidence = completed.limit_pool.evidence();
    let projection = &completed.projection;
    let evidence_date = context.evidence_date.format("%Y-%m-%d").to_string();
    if completed.limit_pool.is_verified_empty()
        || completed.limit_pool.records().is_empty()
        || evidence.source.trim().is_empty()
        || evidence.observed_at.trim().is_empty()
        || evidence.batch_id.trim().is_empty()
        || evidence.source_at.as_deref() != Some(evidence_date.as_str())
        || projection.limit_pool_batch_id != evidence.batch_id
        || projection.ordered_limit_pool_record_hashes.len() != completed.limit_pool.records().len()
        || projection
            .ordered_limit_pool_record_hashes
            .iter()
            .any(|hash| !is_sha256_hex(hash))
        || projection
            .excluded_record_hashes
            .iter()
            .any(|(hash, reason)| !is_sha256_hex(hash) || reason.trim().is_empty())
    {
        return Err(P01Failure::for_context(
            "p01_limit_pool_binding_mismatch",
            false,
            "limit_pools",
            context,
        ));
    }
    #[derive(Serialize)]
    struct ExactRequest<'a> {
        kind: &'static str,
        trading_date: &'a str,
        limit: u32,
    }
    let request_bytes = serde_json::to_vec(&ExactRequest {
        kind: "Upper",
        trading_date: &evidence_date,
        limit: P01_LIMIT_POOL_REQUEST_LIMIT,
    })
    .map_err(|_| {
        P01Failure::for_context(
            "p01_limit_pool_request_hash_failed",
            false,
            "limit_pools",
            context,
        )
    })?;
    Ok(P01LimitPoolsBinding {
        request_kind: "Upper",
        request_trading_date: evidence_date.clone(),
        request_limit: P01_LIMIT_POOL_REQUEST_LIMIT,
        request_hash: domain_sha256_hex(P01_REQUEST_HASH_DOMAIN, &request_bytes),
        provider: evidence.provider,
        source: evidence.source.clone(),
        source_at: evidence_date,
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        ordered_record_hashes: projection.ordered_limit_pool_record_hashes.clone(),
        record_count: completed.limit_pool.records().len(),
        verified_empty: false,
    })
}

fn bind_security_identities(
    context: P01BusinessContext,
    requested_codes: &[String],
    identities: GatewayBatch<MarketSecurityIdentity>,
) -> Result<
    (
        P01SecurityIdentityBinding,
        std::collections::HashMap<String, String>,
    ),
    P01Failure,
> {
    let (records, evidence) = match identities {
        GatewayBatch::Available { records, evidence } if !records.is_empty() => (records, evidence),
        GatewayBatch::Available { .. } | GatewayBatch::VerifiedEmpty(_) => {
            return Err(P01Failure::for_context(
                "p01_security_identity_exact_set_mismatch",
                false,
                "security_identity",
                context,
            ));
        }
    };
    if records.len() != requested_codes.len() || !valid_batch_evidence(&evidence) {
        return Err(P01Failure::for_context(
            "p01_security_identity_exact_set_mismatch",
            false,
            "security_identity",
            context,
        ));
    }

    #[derive(Serialize)]
    struct IdentityRecord<'a> {
        code: &'a str,
        name: &'a str,
        is_st: bool,
        source_at: String,
        observed_at: String,
        provider: ProviderId,
        batch_id: &'a str,
    }
    let mut resolved_codes = Vec::with_capacity(records.len());
    let mut record_hashes = Vec::with_capacity(records.len());
    let mut names = std::collections::HashMap::with_capacity(records.len());
    for (requested, record) in requested_codes.iter().zip(&records) {
        if record.code != *requested
            || record.name.trim().is_empty()
            || record.batch_id != evidence.batch_id
            || record.provider != evidence.provider
            || names
                .insert(record.code.clone(), record.name.clone())
                .is_some()
        {
            return Err(P01Failure::for_context(
                "p01_security_identity_exact_set_mismatch",
                false,
                "security_identity",
                context,
            ));
        }
        resolved_codes.push(record.code.clone());
        let canonical = IdentityRecord {
            code: &record.code,
            name: &record.name,
            is_st: record.is_st,
            source_at: record.source_at.to_rfc3339_opts(SecondsFormat::Nanos, true),
            observed_at: record
                .observed_at
                .to_rfc3339_opts(SecondsFormat::Nanos, true),
            provider: record.provider,
            batch_id: &record.batch_id,
        };
        record_hashes.push(hash_serializable(
            P01_IDENTITY_RECORD_HASH_DOMAIN,
            &canonical,
            context,
            "security_identity",
        )?);
    }

    Ok((
        P01SecurityIdentityBinding {
            requested_codes: requested_codes.to_vec(),
            resolved_codes,
            provider: evidence.provider,
            source: evidence.source,
            source_at: evidence.source_at,
            observed_at: evidence.observed_at,
            batch_id: evidence.batch_id,
            ordered_record_hashes: record_hashes,
        },
        names,
    ))
}

type P01NewsByCode = std::collections::HashMap<String, Vec<P01BoundNews>>;

fn bind_instrument_news(
    context: P01BusinessContext,
    captured_observed_at: DateTime<Local>,
    requested_codes: &[String],
    news_batches: Vec<(String, GatewayBatch<SinaInstrumentNewsRecord>)>,
) -> Result<(Vec<P01InstrumentNewsBinding>, P01NewsByCode, usize), P01Failure> {
    if news_batches.len() != requested_codes.len() {
        return Err(P01Failure::for_context(
            "p01_instrument_news_exact_set_mismatch",
            false,
            "instrument_news",
            context,
        ));
    }
    let range_start = context.evidence_date.format("%Y-%m-%d").to_string();
    let range_end = context.business_date.format("%Y-%m-%d").to_string();
    let captured_utc = captured_observed_at.with_timezone(&Utc);
    let shanghai =
        chrono::FixedOffset::east_opt(8 * 60 * 60).expect("Shanghai fixed offset is always valid");
    let mut bindings = Vec::with_capacity(news_batches.len());
    let mut by_code = std::collections::HashMap::with_capacity(news_batches.len());
    let mut total_records = 0usize;

    #[derive(Serialize)]
    struct NewsRecord<'a> {
        source: &'a str,
        external_id: &'a str,
        category: &'a str,
        code: &'a str,
        title: &'a str,
        summary: &'a str,
        url: &'a str,
        source_name: &'a str,
        published_at: String,
        fetched_at: String,
        content_hash: &'a str,
        provider: ProviderId,
        evidence_source_at: Option<&'a str>,
        evidence_observed_at: &'a str,
        batch_id: &'a str,
    }

    for (requested, (code, batch)) in requested_codes.iter().zip(news_batches) {
        if code != *requested || by_code.contains_key(&code) {
            return Err(P01Failure::for_context(
                "p01_instrument_news_exact_set_mismatch",
                false,
                "instrument_news",
                context,
            ));
        }
        let evidence = batch.evidence().clone();
        if !valid_batch_evidence(&evidence) {
            return Err(P01Failure::for_context(
                "p01_instrument_news_batch_evidence_invalid",
                false,
                "instrument_news",
                context,
            ));
        }
        let verified_empty = batch.is_verified_empty();
        if !verified_empty && batch.records().is_empty() {
            return Err(P01Failure::for_context(
                "p01_instrument_news_batch_status_invalid",
                false,
                "instrument_news",
                context,
            ));
        }
        let mut rows = Vec::with_capacity(batch.records().len());
        for record in batch.records() {
            let item = record.persistence_item();
            let record_evidence = record.evidence();
            let published_date = item.published_at.with_timezone(&shanghai).date_naive();
            if item.code.as_deref() != Some(code.as_str())
                || published_date < context.evidence_date
                || published_date > context.business_date
                || item.published_at > captured_utc
                || item.title.trim().is_empty()
                || record_evidence.provider() != evidence.provider
                || record_evidence.batch_id() != evidence.batch_id
            {
                return Err(P01Failure::for_context(
                    "p01_instrument_news_range_mismatch",
                    false,
                    "instrument_news",
                    context,
                ));
            }
            let canonical = NewsRecord {
                source: &item.source,
                external_id: &item.external_id,
                category: &item.category,
                code: &code,
                title: &item.title,
                summary: &item.summary,
                url: &item.url,
                source_name: &item.source_name,
                published_at: item
                    .published_at
                    .to_rfc3339_opts(SecondsFormat::Nanos, true),
                fetched_at: item.fetched_at.to_rfc3339_opts(SecondsFormat::Nanos, true),
                content_hash: &item.content_hash,
                provider: record_evidence.provider(),
                evidence_source_at: record_evidence.source_at(),
                evidence_observed_at: record_evidence.observed_at(),
                batch_id: record_evidence.batch_id(),
            };
            let hash = hash_serializable(
                P01_NEWS_RECORD_HASH_DOMAIN,
                &canonical,
                context,
                "instrument_news",
            )?;
            rows.push((
                item.published_at,
                item.external_id.clone(),
                hash,
                P01BoundNews {
                    title: item.title.clone(),
                    source_name: item.source_name.clone(),
                    published_at: item.published_at,
                },
            ));
        }
        rows.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        let ordered_record_hashes = rows.iter().map(|row| row.2.clone()).collect::<Vec<_>>();
        let rendered_rows = rows.into_iter().map(|row| row.3).collect::<Vec<_>>();
        total_records = total_records.saturating_add(rendered_rows.len());
        by_code.insert(code.clone(), rendered_rows);
        bindings.push(P01InstrumentNewsBinding {
            code,
            range_start: range_start.clone(),
            range_end: range_end.clone(),
            provider: evidence.provider,
            source: evidence.source,
            source_at: evidence.source_at,
            observed_at: evidence.observed_at,
            batch_id: evidence.batch_id,
            ordered_record_hashes,
            record_count: batch.records().len(),
            verified_empty,
        });
    }
    Ok((bindings, by_code, total_records))
}

fn valid_batch_evidence(evidence: &BatchEvidence) -> bool {
    !evidence.source.trim().is_empty()
        && !evidence.observed_at.trim().is_empty()
        && !evidence.batch_id.trim().is_empty()
}

fn valid_p01_code(code: &str) -> bool {
    #[cfg(test)]
    if let Some(code) = code.strip_prefix("TEST_CODE_") {
        return code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit());
    }
    code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit())
}

fn gateway_failure(
    context: P01BusinessContext,
    stage: &'static str,
    error: stock_analysis::data_gateway::GatewayError,
) -> P01Failure {
    P01Failure::for_context(error.reason_code(), error.retryable(), stage, context)
}

fn hash_serializable(
    domain: &[u8],
    value: &impl Serialize,
    context: P01BusinessContext,
    stage: &'static str,
) -> Result<String, P01Failure> {
    serde_json::to_vec(value)
        .map(|bytes| domain_sha256_hex(domain, &bytes))
        .map_err(|_| P01Failure::for_context("p01_record_hash_failed", false, stage, context))
}

fn domain_sha256_hex(domain: &[u8], bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    domain_sha256_hex(&[], bytes)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[async_trait::async_trait]
trait P01Ports: Sync {
    async fn inspect(
        &self,
        context: P01BusinessContext,
    ) -> Result<Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>, P01Failure>;

    async fn resume(
        &self,
        context: P01BusinessContext,
    ) -> Result<Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>, P01Failure>;

    async fn load(
        &self,
        context: P01BusinessContext,
        observed_at: DateTime<Local>,
    ) -> Result<P01InputBinding, P01Failure>;

    fn render(&self, mode: P01RenderMode, input: &P01InputBinding) -> Result<String, P01Failure>;

    async fn push(
        &self,
        mode: P01RenderMode,
        input: P01InputBinding,
        text: String,
    ) -> Result<crate::notify::PushOutcome, P01Failure>;
}

struct ProductionP01Ports;

#[async_trait::async_trait]
impl P01Ports for ProductionP01Ports {
    async fn inspect(
        &self,
        context: P01BusinessContext,
    ) -> Result<Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>, P01Failure>
    {
        crate::durable_delivery_runtime::inspect_business_date_once_claim(
            context.business_date,
            stock_analysis::durable_delivery::PushKind::PreopenNewsHot,
            stock_analysis::durable_delivery::DeliverySubKind::None,
            "GLOBAL",
            &format!("p01:{}", context.business_date),
        )
        .await
        .map_err(|_| {
            P01Failure::for_context(
                "p01_durable_preflight_failed",
                true,
                "durable_preflight",
                context,
            )
        })
    }

    async fn resume(
        &self,
        context: P01BusinessContext,
    ) -> Result<Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>, P01Failure>
    {
        crate::durable_delivery_runtime::resume_business_date_once_claim(
            context.business_date,
            stock_analysis::durable_delivery::PushKind::PreopenNewsHot,
            stock_analysis::durable_delivery::DeliverySubKind::None,
            "GLOBAL",
            &format!("p01:{}", context.business_date),
        )
        .await
        .map_err(|_| {
            P01Failure::for_context(
                "p01_durable_resume_failed",
                true,
                "durable_preflight",
                context,
            )
        })
    }

    async fn load(
        &self,
        context: P01BusinessContext,
        observed_at: DateTime<Local>,
    ) -> Result<P01InputBinding, P01Failure> {
        load_p01_input_binding(context, observed_at).await
    }

    fn render(&self, mode: P01RenderMode, input: &P01InputBinding) -> Result<String, P01Failure> {
        crate::push_templates::render_bound_preopen_news_hot(mode, input)
    }

    async fn push(
        &self,
        mode: P01RenderMode,
        input: P01InputBinding,
        text: String,
    ) -> Result<crate::notify::PushOutcome, P01Failure> {
        let context = input.context;
        let source_binding = input.canonical_source_bytes(mode, &text)?;
        let binding = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            context.business_date,
            input.schedule_occurrence_identity(),
            source_binding,
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            input.delivery_subject_hash(mode, &text)?,
            crate::durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            false,
        )
        .map_err(|_| {
            P01Failure::for_context(
                "p01_counted_binding_invalid",
                false,
                "durable_binding",
                context,
            )
        })?;
        let token = crate::presentation_registry::acquire_token(
            "P-01-preopen-news-hot",
            crate::notify::PushKind::PreopenNewsHot,
            "preopen_news_dispatcher",
            "render_preopen_news_hot",
        )
        .map_err(|_| {
            P01Failure::for_context(
                "p01_presentation_token_rejected",
                false,
                "presentation",
                context,
            )
        })?;
        Ok(crate::notify::push_counted_with_binding(token, &text, None, binding).await)
    }
}

fn render_mode(mode: P01ExecutionMode) -> P01RenderMode {
    match mode {
        P01ExecutionMode::Scheduled => P01RenderMode::Scheduled,
        P01ExecutionMode::Compensation => P01RenderMode::Compensation,
    }
}

fn execution_mode_label(mode: P01ExecutionMode) -> &'static str {
    match mode {
        P01ExecutionMode::Scheduled => "Scheduled",
        P01ExecutionMode::Compensation => "Compensation",
    }
}

fn failure_outcome(failure: P01Failure) -> P01RunOutcome {
    if failure.retryable() {
        P01RunOutcome::RetryableFailure(failure)
    } else {
        P01RunOutcome::TerminalFailure(failure)
    }
}

fn claimed_outcome(
    context: P01BusinessContext,
    evidence: crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence,
    delivered_by_this_run: bool,
) -> P01RunOutcome {
    use stock_analysis::durable_delivery::DecisionState;

    match evidence.state {
        DecisionState::Delivered => match evidence.authoritative_receipt_sha256 {
            Some(receipt_sha256) if is_sha256_hex(&receipt_sha256) => {
                if delivered_by_this_run {
                    P01RunOutcome::Delivered {
                        decision_identity: evidence.decision_identity,
                        receipt_sha256,
                    }
                } else {
                    P01RunOutcome::AlreadyDelivered {
                        decision_identity: evidence.decision_identity,
                    }
                }
            }
            _ => P01RunOutcome::TerminalFailure(P01Failure::for_context(
                "p01_authoritative_receipt_missing",
                false,
                "durable_authority",
                context,
            )),
        },
        DecisionState::RejectedDurable | DecisionState::ManualResolvedRejected => {
            P01RunOutcome::TerminalFailure(P01Failure::for_context(
                "p01_durable_delivery_rejected",
                false,
                "durable_authority",
                context,
            ))
        }
        DecisionState::Reserved => P01RunOutcome::RetryableFailure(P01Failure::for_context(
            "p01_durable_claim_reserved",
            true,
            "durable_authority",
            context,
        )),
        _ => match evidence.current_attempt_identity {
            Some(attempt_identity) if !attempt_identity.trim().is_empty() => {
                P01RunOutcome::AwaitingReconciliation { attempt_identity }
            }
            _ => P01RunOutcome::TerminalFailure(P01Failure::for_context(
                "p01_durable_attempt_identity_missing",
                false,
                "durable_authority",
                context,
            )),
        },
    }
}

async fn run_p01_once_with_ports<P: P01Ports>(
    mode: P01ExecutionMode,
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
    ports: &P,
) -> P01RunOutcome {
    match ports.inspect(context).await {
        Ok(Some(evidence)) => {
            if matches!(
                evidence.state,
                stock_analysis::durable_delivery::DecisionState::Delivered
                    | stock_analysis::durable_delivery::DecisionState::RejectedDurable
                    | stock_analysis::durable_delivery::DecisionState::ManualResolvedRejected
                    | stock_analysis::durable_delivery::DecisionState::UncertainManualReview
            ) {
                return claimed_outcome(context, evidence, false);
            }
            if mode == P01ExecutionMode::Compensation
                && evidence.state == stock_analysis::durable_delivery::DecisionState::Reserved
            {
                match evidence.source_binding_mode.as_deref() {
                    Some("Compensation") => {}
                    Some("Scheduled") => {
                        return failure_outcome(P01Failure::for_context(
                            "p01_scheduled_claim_late_resume_forbidden",
                            false,
                            "durable_authority",
                            context,
                        ));
                    }
                    _ => {
                        return failure_outcome(P01Failure::for_context(
                            "p01_stored_render_mode_invalid",
                            false,
                            "durable_authority",
                            context,
                        ));
                    }
                }
            }
            return match ports.resume(context).await {
                Ok(Some(evidence)) => {
                    let delivered_by_this_run = evidence.sink_calls > 0;
                    claimed_outcome(context, evidence, delivered_by_this_run)
                }
                Ok(None) => failure_outcome(P01Failure::for_context(
                    "p01_durable_claim_disappeared",
                    false,
                    "durable_authority",
                    context,
                )),
                Err(failure) => failure_outcome(failure),
            };
        }
        Ok(None) => {}
        Err(failure) => return failure_outcome(failure),
    }

    let input = match ports.load(context, observed_at).await {
        Ok(input) => input,
        Err(failure) => return failure_outcome(failure),
    };
    let mode = render_mode(mode);
    let text = match ports.render(mode, &input) {
        Ok(text) => text,
        Err(failure) => return failure_outcome(failure),
    };
    let source_evidence_sha256 = match input.source_evidence_fingerprint(mode, &text) {
        Ok(source_evidence_sha256) => source_evidence_sha256,
        Err(failure) => return failure_outcome(failure),
    };
    let push_outcome = match ports.push(mode, input, text).await {
        Ok(outcome) => outcome,
        Err(failure) => {
            return failure_outcome(failure.with_source_evidence_sha256(source_evidence_sha256));
        }
    };
    let outcome = match ports.inspect(context).await {
        Ok(Some(evidence)) => claimed_outcome(
            context,
            evidence,
            matches!(push_outcome, crate::notify::PushOutcome::Pushed),
        ),
        Ok(None) => match push_outcome {
            crate::notify::PushOutcome::SinkError(_) => failure_outcome(P01Failure::for_context(
                "p01_counted_sink_error",
                true,
                "durable_sink",
                context,
            )),
            crate::notify::PushOutcome::Denied(_) => failure_outcome(P01Failure::for_context(
                "p01_counted_push_denied",
                false,
                "durable_admission",
                context,
            )),
            crate::notify::PushOutcome::Pushed | crate::notify::PushOutcome::Deduped => {
                failure_outcome(P01Failure::for_context(
                    "p01_durable_claim_missing_after_push",
                    false,
                    "durable_authority",
                    context,
                ))
            }
        },
        Err(failure) => failure_outcome(failure),
    };
    outcome.with_source_evidence_sha256(source_evidence_sha256)
}

async fn audit_p01_outcome(
    mode: P01ExecutionMode,
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
    outcome: P01RunOutcome,
) -> P01RunOutcome {
    let failure = match &outcome {
        P01RunOutcome::RetryableFailure(failure) | P01RunOutcome::TerminalFailure(failure) => {
            Some(failure)
        }
        _ => None,
    };
    if let Some(failure) = failure {
        if let Err(error) = crate::durable_delivery_runtime::append_p01_failure_audit(
            execution_mode_label(mode),
            observed_at,
            failure,
            failure.source_evidence_sha256(),
        )
        .await
        {
            log::error!(
                "[P-01][BR-241] immutable failure audit append failed original_reason={} error={}",
                failure.reason_code(),
                error
            );
            return P01RunOutcome::RetryableFailure(P01Failure::for_context(
                "p01_failure_audit_append_failed",
                true,
                "failure_audit",
                context,
            ));
        }
    }
    outcome
}

async fn run_p01_once_authorized(
    mode: P01ExecutionMode,
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
) -> P01RunOutcome {
    let outcome = run_p01_once_with_ports(mode, context, observed_at, &ProductionP01Ports).await;
    audit_p01_outcome(mode, context, observed_at, outcome).await
}

pub async fn run_p01_once(
    mode: P01ExecutionMode,
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
) -> P01RunOutcome {
    if mode == P01ExecutionMode::Compensation {
        return audit_p01_outcome(
            mode,
            context,
            observed_at,
            failure_outcome(P01Failure::for_context(
                "p01_compensation_capability_required",
                false,
                "compensation_authority",
                context,
            )),
        )
        .await;
    }
    run_p01_once_authorized(mode, context, observed_at).await
}

pub async fn run_p01_compensation_once(
    capability: &crate::durable_delivery_runtime::P01CompensationCapability,
    context: P01BusinessContext,
    observed_at: DateTime<Local>,
) -> P01RunOutcome {
    if capability.business_date() != context.business_date {
        return audit_p01_outcome(
            P01ExecutionMode::Compensation,
            context,
            observed_at,
            failure_outcome(P01Failure::for_context(
                "p01_compensation_capability_date_mismatch",
                false,
                "compensation_authority",
                context,
            )),
        )
        .await;
    }
    crate::durable_delivery_runtime::with_p01_compensation_scope(
        capability,
        run_p01_once_authorized(P01ExecutionMode::Compensation, context, observed_at),
    )
    .await
}

pub fn classify_scheduled_due(captured_at: NaiveDateTime) -> P01Due {
    let business_date = captured_at.date();
    let context = match P01BusinessContext::new(business_date) {
        Ok(context) => context,
        Err(failure) if failure.reason_code() == "p01_business_date_not_trading" => {
            return P01Due::NotDue(P01NotDueReason::NonTradingDay);
        }
        Err(_) => return P01Due::NotDue(P01NotDueReason::CalendarUnavailable),
    };
    let scheduled_start = NaiveTime::from_hms_opt(9, 0, 0).expect("valid P-01 start time");
    let scheduled_end = NaiveTime::from_hms_opt(9, 15, 0).expect("valid P-01 end time");

    if captured_at.time() < scheduled_start {
        P01Due::NotDue(P01NotDueReason::BeforeWindow)
    } else if captured_at.time() >= scheduled_end {
        P01Due::NotDue(P01NotDueReason::ScheduledWindowClosed)
    } else {
        P01Due::Due(context)
    }
}

pub fn classify_compensation_due(
    requested_business_date: NaiveDate,
    captured_at: DateTime<Local>,
) -> P01Due {
    let context = match P01BusinessContext::new(requested_business_date) {
        Ok(context) => context,
        Err(failure) if failure.reason_code() == "p01_business_date_not_trading" => {
            return P01Due::NotDue(P01NotDueReason::NonTradingDay);
        }
        Err(_) => return P01Due::NotDue(P01NotDueReason::CalendarUnavailable),
    };
    if requested_business_date != captured_at.date_naive() {
        return P01Due::NotDue(P01NotDueReason::BusinessDateMismatch);
    }
    let scheduled_end = NaiveTime::from_hms_opt(9, 15, 0).expect("valid P-01 end time");
    if captured_at.time() < scheduled_end {
        P01Due::NotDue(P01NotDueReason::CompensationBeforeWindowClosed)
    } else {
        P01Due::Due(context)
    }
}

pub async fn p01_scheduler_loop() {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut terminal_business_date = None;
    loop {
        ticker.tick().await;
        let captured_at = Local::now();
        let context = match classify_scheduled_due(captured_at.naive_local()) {
            P01Due::Due(context) => context,
            P01Due::NotDue(P01NotDueReason::CalendarUnavailable) => {
                let failure = P01Failure::terminal("p01_trading_calendar_unavailable", "calendar")
                    .with_business_date(captured_at.date_naive());
                if let Err(error) = crate::durable_delivery_runtime::append_p01_failure_audit(
                    execution_mode_label(P01ExecutionMode::Scheduled),
                    captured_at,
                    &failure,
                    None,
                )
                .await
                {
                    log::error!(
                        "[P-01][BR-241] calendar authority unavailable and failure audit append failed: {}",
                        error
                    );
                }
                continue;
            }
            P01Due::NotDue(_) => continue,
        };
        if terminal_business_date == Some(context.business_date) {
            continue;
        }
        match run_p01_once(P01ExecutionMode::Scheduled, context, captured_at).await {
            P01RunOutcome::Delivered {
                decision_identity,
                receipt_sha256,
            } => log::info!(
                "[P-01][BR-241] delivered business_date={} decision={} receipt_sha256={}",
                context.business_date,
                decision_identity,
                receipt_sha256
            ),
            P01RunOutcome::AlreadyDelivered { decision_identity } => log::info!(
                "[P-01][BR-241] already_delivered business_date={} decision={}",
                context.business_date,
                decision_identity
            ),
            P01RunOutcome::AwaitingReconciliation { attempt_identity } => log::warn!(
                "[P-01][BR-241] awaiting_reconciliation business_date={} attempt={}",
                context.business_date,
                attempt_identity
            ),
            P01RunOutcome::RetryableFailure(failure) => log::warn!(
                "[P-01][BR-241] retryable_failure business_date={} evidence_date={:?} reason_code={} stage={}",
                context.business_date,
                failure.evidence_date(),
                failure.reason_code(),
                failure.stage()
            ),
            P01RunOutcome::TerminalFailure(failure) => {
                terminal_business_date = Some(context.business_date);
                log::error!(
                    "[P-01][BR-241] terminal_failure business_date={:?} evidence_date={:?} reason_code={} stage={}",
                    failure.business_date(),
                    failure.evidence_date(),
                    failure.reason_code(),
                    failure.stage()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Local, NaiveDate};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use stock_analysis::data_gateway::market_capabilities::MarketSecurityIdentity;
    use stock_analysis::data_gateway::{BatchEvidence, GatewayBatch, SinaInstrumentNewsRecord};
    use stock_analysis::data_provider::news_item::{content_hash, NewsItem};
    use stock_analysis::database::concepts::ChainDailyRow;
    use stock_analysis::magic_compat::{
        AssetClass, Exchange, InstrumentId, IsoDate, LimitPoolEntry, LimitPoolKind, NonEmptyText,
        Price, ProviderId, Ratio, RatioUnit, SourceEvidence,
    };
    use stock_analysis::pipeline::chain_analysis::p01_projection::{
        P01ChainProjectionReceipt, P01CompletedDayEvidence,
    };

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid test date")
    }

    fn evidence(batch_id: &str, source_at: &str) -> BatchEvidence {
        evidence_for(ProviderId::Tdx, batch_id, Some(source_at))
    }

    fn evidence_for(
        provider: ProviderId,
        batch_id: &str,
        source_at: Option<&str>,
    ) -> BatchEvidence {
        BatchEvidence {
            provider,
            source: "TEST_CODE_P01_SOURCE".to_string(),
            source_at: source_at.map(str::to_string),
            observed_at: "1787000000.000000000".to_string(),
            batch_id: batch_id.to_string(),
        }
    }

    fn completed_day() -> P01CompletedDayEvidence {
        let batch_id = "TEST_CODE_P01_LIMIT_POOL_BATCH";
        let source_at = "2026-08-17";
        let record_evidence =
            SourceEvidence::new(ProviderId::Tdx, "1787000000.000000000", batch_id)
                .and_then(|value| value.with_source_at(source_at))
                .expect("valid TEST_CODE source evidence");
        let record = LimitPoolEntry {
            kind: LimitPoolKind::Upper,
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_000001",
                AssetClass::Equity,
            )
            .expect("valid TEST_CODE instrument"),
            trading_date: IsoDate::new(source_at).expect("valid evidence date"),
            price: Price::new(10.0).expect("valid price"),
            change: Ratio::new(0.1, RatioUnit::Decimal).expect("valid ratio"),
            volume: None,
            turnover: None,
            sealed_amount: None,
            first_seal_at: None,
            last_seal_at: None,
            break_count: None,
            streak: None,
            industry: Some(NonEmptyText::new("TEST_CODE_AI_CHAIN").expect("valid chain")),
            board_name: None,
            seal_state: None,
            reseal_count: None,
            reason: None,
            evidence: record_evidence,
        };
        P01CompletedDayEvidence {
            limit_pool: GatewayBatch::Available {
                records: vec![record],
                evidence: evidence(batch_id, source_at),
            },
            chain_rows: vec![ChainDailyRow {
                date: source_at.to_string(),
                concept: "TEST_CODE_AI_CHAIN".to_string(),
                stocks: serde_json::json!(["TEST_CODE_000001"]).to_string(),
                continuation_count: 1,
            }],
            projection: P01ChainProjectionReceipt {
                evidence_date: date(2026, 8, 17),
                limit_pool_batch_id: batch_id.to_string(),
                ordered_limit_pool_record_hashes: vec!["a".repeat(64)],
                excluded_record_hashes: vec![],
                ordered_chain_row_hashes: vec!["b".repeat(64)],
                persistence_receipt_sha256: "c".repeat(64),
            },
        }
    }

    fn observed_at() -> DateTime<Local> {
        DateTime::parse_from_rfc3339("2026-08-18T09:05:00+08:00")
            .expect("valid observed time")
            .with_timezone(&Local)
    }

    struct RecordingP01Ports {
        inspections: Mutex<
            VecDeque<Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>>,
        >,
        provider_calls: AtomicUsize,
        p01_sink_calls: AtomicUsize,
        other_push_kind_calls: AtomicUsize,
        push_outcome: crate::notify::PushOutcome,
    }

    impl RecordingP01Ports {
        fn delivered() -> crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence {
            crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence {
                decision_identity: "TEST_CODE_P01_DECISION".to_owned(),
                state: stock_analysis::durable_delivery::DecisionState::Delivered,
                sink_calls: 0,
                current_attempt_identity: Some("TEST_CODE_P01_ATTEMPT".to_owned()),
                authoritative_receipt_sha256: Some("a".repeat(64)),
                source_binding_mode: Some("Scheduled".to_owned()),
                schedule_hydration: None,
            }
        }

        fn already_delivered() -> Self {
            Self {
                inspections: Mutex::new(VecDeque::from([Some(Self::delivered())])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::Pushed,
            }
        }

        fn accepted() -> Self {
            Self {
                inspections: Mutex::new(VecDeque::from([None, Some(Self::delivered())])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::Pushed,
            }
        }

        fn resumed_reserved_with_mode(mode: &str) -> Self {
            let mut delivered = Self::delivered();
            delivered.sink_calls = 1;
            delivered.source_binding_mode = Some(mode.to_owned());
            Self {
                inspections: Mutex::new(VecDeque::from([
                    Some(
                        crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence {
                            decision_identity: "TEST_CODE_P01_DECISION".to_owned(),
                            state: stock_analysis::durable_delivery::DecisionState::Reserved,
                            sink_calls: 0,
                            current_attempt_identity: None,
                            authoritative_receipt_sha256: None,
                            source_binding_mode: Some(mode.to_owned()),
                            schedule_hydration: None,
                        },
                    ),
                    Some(delivered),
                ])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::Pushed,
            }
        }

        fn resumed_reserved() -> Self {
            Self::resumed_reserved_with_mode("Scheduled")
        }

        fn pushed_without_durable_claim() -> Self {
            Self {
                inspections: Mutex::new(VecDeque::from([None, None])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::Pushed,
            }
        }

        fn sink_error_without_durable_claim() -> Self {
            Self {
                inspections: Mutex::new(VecDeque::from([None, None])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::SinkError(
                    "TEST_CODE_TRANSIENT_SINK_ERROR".to_owned(),
                ),
            }
        }

        fn deduped_with_delivered_claim() -> Self {
            Self {
                inspections: Mutex::new(VecDeque::from([None, Some(Self::delivered())])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::Deduped,
            }
        }

        fn uncertain_after_sink_error() -> Self {
            Self {
                inspections: Mutex::new(VecDeque::from([
                    None,
                    Some(crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence {
                        decision_identity: "TEST_CODE_P01_DECISION".to_owned(),
                        state:
                            stock_analysis::durable_delivery::DecisionState::UncertainManualReview,
                        sink_calls: 1,
                        current_attempt_identity: Some("TEST_CODE_P01_ATTEMPT".to_owned()),
                        authoritative_receipt_sha256: None,
                        source_binding_mode: Some("Compensation".to_owned()),
                        schedule_hydration: None,
                    }),
                ])),
                provider_calls: AtomicUsize::new(0),
                p01_sink_calls: AtomicUsize::new(0),
                other_push_kind_calls: AtomicUsize::new(0),
                push_outcome: crate::notify::PushOutcome::SinkError(
                    "TEST_CODE_UNCERTAIN_SINK".to_owned(),
                ),
            }
        }

        fn provider_calls(&self) -> usize {
            self.provider_calls.load(Ordering::SeqCst)
        }

        fn p01_sink_calls(&self) -> usize {
            self.p01_sink_calls.load(Ordering::SeqCst)
        }

        fn other_push_kind_calls(&self) -> usize {
            self.other_push_kind_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl P01Ports for RecordingP01Ports {
        async fn inspect(
            &self,
            context: P01BusinessContext,
        ) -> Result<
            Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>,
            P01Failure,
        > {
            self.inspections
                .lock()
                .expect("TEST_CODE inspection mutex")
                .pop_front()
                .ok_or_else(|| {
                    P01Failure::for_context(
                        "TEST_CODE_P01_INSPECTION_EXHAUSTED",
                        false,
                        "TEST_CODE_PORT",
                        context,
                    )
                })
        }

        async fn resume(
            &self,
            context: P01BusinessContext,
        ) -> Result<
            Option<crate::durable_delivery_runtime::BusinessDateOnceDispatchEvidence>,
            P01Failure,
        > {
            self.inspect(context).await
        }

        async fn load(
            &self,
            _context: P01BusinessContext,
            _observed_at: DateTime<Local>,
        ) -> Result<P01InputBinding, P01Failure> {
            self.provider_calls.fetch_add(1, Ordering::SeqCst);
            Ok(P01InputBinding::complete_test_input())
        }

        fn render(
            &self,
            mode: P01RenderMode,
            input: &P01InputBinding,
        ) -> Result<String, P01Failure> {
            crate::push_templates::render_bound_preopen_news_hot(mode, input)
        }

        async fn push(
            &self,
            _mode: P01RenderMode,
            _input: P01InputBinding,
            _text: String,
        ) -> Result<crate::notify::PushOutcome, P01Failure> {
            self.p01_sink_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.push_outcome.clone())
        }
    }

    fn complete_identity_batch() -> GatewayBatch<MarketSecurityIdentity> {
        let batch_id = "TEST_CODE_P01_IDENTITY_BATCH";
        GatewayBatch::Available {
            records: vec![MarketSecurityIdentity {
                code: "TEST_CODE_000001".to_string(),
                name: "TEST_CODE_IDENTITY_NAME".to_string(),
                is_st: false,
                source_at: DateTime::parse_from_rfc3339("2026-08-18T01:04:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                observed_at: DateTime::parse_from_rfc3339("2026-08-18T01:04:01Z")
                    .unwrap()
                    .with_timezone(&Utc),
                provider: ProviderId::Tdx,
                batch_id: batch_id.to_string(),
            }],
            evidence: evidence_for(ProviderId::Tdx, batch_id, Some("2026-08-18T01:04:00Z")),
        }
    }

    fn news_batch(published_at: &str) -> GatewayBatch<SinaInstrumentNewsRecord> {
        let batch_id = "TEST_CODE_P01_NEWS_BATCH";
        let published_at = DateTime::parse_from_rfc3339(published_at)
            .expect("valid TEST_CODE published time")
            .with_timezone(&Utc);
        let title = "TEST_CODE_PROVIDER_HEADLINE".to_string();
        let summary = "TEST_CODE_PROVIDER_SUMMARY".to_string();
        let item = NewsItem {
            source: "TEST_CODE_SINA".to_string(),
            external_id: "TEST_CODE_NEWS_ID".to_string(),
            category: "个股新闻".to_string(),
            code: Some("TEST_CODE_000001".to_string()),
            title: title.clone(),
            summary: summary.clone(),
            url: "https://example.test/TEST_CODE_NEWS_ID".to_string(),
            source_name: "TEST_CODE_PROVIDER".to_string(),
            published_at,
            fetched_at: DateTime::parse_from_rfc3339("2026-08-18T01:04:01Z")
                .unwrap()
                .with_timezone(&Utc),
            content_hash: content_hash(&title, &summary),
        };
        let record_evidence =
            SourceEvidence::new(ProviderId::Sina, "1787000000.000000000", batch_id)
                .and_then(|value| value.with_source_at(published_at.to_rfc3339()))
                .expect("valid TEST_CODE news evidence");
        GatewayBatch::Available {
            records: vec![SinaInstrumentNewsRecord::new(item, record_evidence)],
            evidence: evidence_for(ProviderId::Sina, batch_id, Some("2026-08-18T01:04:00Z")),
        }
    }

    #[test]
    fn p01_tuesday_uses_completed_monday() {
        let context = P01BusinessContext::new(date(2026, 8, 18)).unwrap();
        assert_eq!(context.evidence_date, date(2026, 8, 17));
    }

    #[test]
    fn p01_monday_uses_previous_friday() {
        let context = P01BusinessContext::new(date(2026, 8, 24)).unwrap();
        assert_eq!(context.evidence_date, date(2026, 8, 21));
    }

    #[test]
    fn p01_calendar_authority_unavailable_fails_closed() {
        let unsupported = date(2027, 1, 4);
        let failure = P01BusinessContext::new(unsupported).unwrap_err();
        assert_eq!(failure.reason_code(), "p01_trading_calendar_unavailable");
        assert!(matches!(
            classify_scheduled_due(unsupported.and_hms_opt(9, 5, 0).unwrap()),
            P01Due::NotDue(P01NotDueReason::CalendarUnavailable)
        ));
    }

    #[test]
    fn p01_window_is_start_inclusive_end_exclusive() {
        let due = date(2026, 8, 18);
        assert!(matches!(
            classify_scheduled_due(due.and_hms_opt(9, 0, 0).unwrap()),
            P01Due::Due(_)
        ));
        assert!(matches!(
            classify_scheduled_due(due.and_hms_opt(9, 15, 0).unwrap()),
            P01Due::NotDue(P01NotDueReason::ScheduledWindowClosed)
        ));
    }

    #[test]
    fn p01_compensation_requires_today_trading_day_and_closed_window() {
        let business_date = date(2026, 8, 18);
        let before = Local
            .with_ymd_and_hms(2026, 8, 18, 9, 14, 59)
            .single()
            .unwrap();
        let start = Local
            .with_ymd_and_hms(2026, 8, 18, 9, 15, 0)
            .single()
            .unwrap();

        assert!(matches!(
            classify_compensation_due(business_date, before),
            P01Due::NotDue(P01NotDueReason::CompensationBeforeWindowClosed)
        ));
        assert!(matches!(
            classify_compensation_due(business_date, start),
            P01Due::Due(_)
        ));
        assert!(matches!(
            classify_compensation_due(date(2026, 8, 17), start),
            P01Due::NotDue(P01NotDueReason::BusinessDateMismatch)
        ));
        assert!(matches!(
            classify_compensation_due(date(2026, 8, 16), start),
            P01Due::NotDue(P01NotDueReason::NonTradingDay)
        ));
    }

    #[test]
    fn p01_binding_requires_exact_head_identities() {
        let context = P01BusinessContext::new(date(2026, 8, 18)).unwrap();
        let identity_batch: GatewayBatch<MarketSecurityIdentity> = GatewayBatch::Available {
            records: vec![],
            evidence: evidence("TEST_CODE_P01_IDENTITY_BATCH", "2026-08-18T01:05:00Z"),
        };

        let error = bind_p01_sources(
            context,
            observed_at(),
            completed_day(),
            identity_batch,
            vec![],
        )
        .unwrap_err();

        assert_eq!(
            error.reason_code(),
            "p01_security_identity_exact_set_mismatch"
        );
    }

    #[test]
    fn p01_binding_rejects_news_outside_range() {
        let context = P01BusinessContext::new(date(2026, 8, 18)).unwrap();
        let error = bind_p01_sources(
            context,
            observed_at(),
            completed_day(),
            complete_identity_batch(),
            vec![(
                "TEST_CODE_000001".to_string(),
                news_batch("2026-08-16T12:00:00Z"),
            )],
        )
        .unwrap_err();

        assert_eq!(error.reason_code(), "p01_instrument_news_range_mismatch");
    }

    #[test]
    fn p01_binding_rejects_news_after_captured_upper_bound() {
        let context = P01BusinessContext::new(date(2026, 8, 18)).unwrap();
        let error = bind_p01_sources(
            context,
            observed_at(),
            completed_day(),
            complete_identity_batch(),
            vec![(
                "TEST_CODE_000001".to_string(),
                news_batch("2026-08-18T02:00:00Z"),
            )],
        )
        .unwrap_err();

        assert_eq!(error.reason_code(), "p01_instrument_news_range_mismatch");
    }

    #[test]
    fn p01_canonical_binding_is_exact_and_binds_rendered_bytes() {
        let input = P01InputBinding::complete_test_input();
        let canonical = input
            .canonical_source_bytes(P01RenderMode::Scheduled, "TEST_CODE_RENDERED_TEXT")
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        let object = value.as_object().unwrap();
        let expected_keys = [
            "schema_version",
            "business_date",
            "evidence_date",
            "template_id",
            "schedule_occurrence_identity",
            "render_mode",
            "captured_observed_at",
            "limit_pools",
            "chain_daily",
            "ordered_head_codes",
            "security_identity",
            "instrument_news",
            "excluded_limit_pool_records",
            "rendered_content_sha256",
        ];

        assert_eq!(object.len(), expected_keys.len());
        assert!(expected_keys.iter().all(|key| object.contains_key(*key)));
        assert_eq!(value["schema_version"], "P01_SOURCE_BINDING_V1");
        assert_eq!(value["template_id"], "preopen_news_hot_v1");
        assert_eq!(value["schedule_occurrence_identity"], "p01:2026-08-18");
        assert_eq!(value["render_mode"], "Scheduled");
        assert_eq!(value["limit_pools"]["request_kind"], "Upper");
        assert_eq!(value["limit_pools"]["request_limit"], 200);
        assert_eq!(
            value["rendered_content_sha256"],
            "66c452d8c5085cce3945dcd1cf4dc6f0e0317b3501d9201c6e8abdefc52d7a17"
        );
        assert_eq!(
            canonical,
            input
                .canonical_source_bytes(P01RenderMode::Scheduled, "TEST_CODE_RENDERED_TEXT")
                .unwrap()
        );
    }

    #[test]
    fn p01_render_or_source_byte_change_changes_delivery_subject_hash() {
        let input = P01InputBinding::complete_test_input();
        let scheduled = input
            .delivery_subject_hash(P01RenderMode::Scheduled, "TEST_CODE_RENDERED_TEXT")
            .unwrap();
        let changed_text = input
            .delivery_subject_hash(P01RenderMode::Scheduled, "TEST_CODE_RENDERED_TEXT_CHANGED")
            .unwrap();
        let compensation = input
            .delivery_subject_hash(P01RenderMode::Compensation, "TEST_CODE_RENDERED_TEXT")
            .unwrap();

        assert_ne!(scheduled, changed_text);
        assert_ne!(scheduled, compensation);
    }

    #[test]
    fn p01_canonical_binding_is_accepted_by_counted_delivery_validator() {
        let input = P01InputBinding::complete_test_input();
        let mode = P01RenderMode::Compensation;
        let text = crate::push_templates::render_bound_preopen_news_hot(mode, &input).unwrap();
        let source_binding = input.canonical_source_bytes(mode, &text).unwrap();
        let binding = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            input.context.business_date,
            input.schedule_occurrence_identity(),
            source_binding,
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            input.delivery_subject_hash(mode, &text).unwrap(),
            crate::durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            false,
        )
        .unwrap();

        assert_eq!(binding.validate_p01_text(&text), Ok(()));
    }

    #[tokio::test]
    async fn p01_preflight_precedes_every_provider_and_sink() {
        let ports = RecordingP01Ports::already_delivered();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Scheduled,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(outcome, P01RunOutcome::AlreadyDelivered { .. }));
        assert_eq!(ports.provider_calls(), 0);
        assert_eq!(ports.p01_sink_calls(), 0);
        assert_eq!(ports.other_push_kind_calls(), 0);
    }

    #[tokio::test]
    async fn p01_preflight_failure_does_not_fabricate_source_evidence() {
        let ports = RecordingP01Ports {
            inspections: Mutex::new(VecDeque::new()),
            provider_calls: AtomicUsize::new(0),
            p01_sink_calls: AtomicUsize::new(0),
            other_push_kind_calls: AtomicUsize::new(0),
            push_outcome: crate::notify::PushOutcome::Pushed,
        };
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Scheduled,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        match outcome {
            P01RunOutcome::TerminalFailure(failure) => {
                assert_eq!(failure.reason_code(), "TEST_CODE_P01_INSPECTION_EXHAUSTED");
                assert_eq!(failure.source_evidence_sha256(), None);
            }
            other => panic!("expected TEST_CODE preflight failure, got {other:?}"),
        }
        assert_eq!(ports.provider_calls(), 0);
        assert_eq!(ports.p01_sink_calls(), 0);
    }

    #[tokio::test]
    async fn p01_acceptance_uses_only_the_counted_p01_sink() {
        let ports = RecordingP01Ports::accepted();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Scheduled,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(outcome, P01RunOutcome::Delivered { .. }));
        assert_eq!(ports.provider_calls(), 1);
        assert_eq!(ports.p01_sink_calls(), 1);
        assert_eq!(ports.other_push_kind_calls(), 0);
    }

    #[tokio::test]
    async fn p01_reserved_stored_envelope_resume_reports_this_runs_delivery() {
        let ports = RecordingP01Ports::resumed_reserved();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Scheduled,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(outcome, P01RunOutcome::Delivered { .. }));
        assert_eq!(ports.provider_calls(), 0);
        assert_eq!(ports.p01_sink_calls(), 0);
    }

    #[tokio::test]
    async fn p01_compensation_never_resumes_a_stored_scheduled_envelope() {
        let ports = RecordingP01Ports::resumed_reserved();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Compensation,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(
            outcome,
            P01RunOutcome::TerminalFailure(ref failure)
                if failure.reason_code() == "p01_scheduled_claim_late_resume_forbidden"
        ));
        assert_eq!(ports.provider_calls(), 0);
        assert_eq!(ports.p01_sink_calls(), 0);
        assert_eq!(ports.other_push_kind_calls(), 0);
    }

    #[tokio::test]
    async fn p01_compensation_resumes_only_a_stored_compensation_envelope() {
        let ports = RecordingP01Ports::resumed_reserved_with_mode("Compensation");
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Compensation,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(outcome, P01RunOutcome::Delivered { .. }));
        assert_eq!(ports.provider_calls(), 0);
        assert_eq!(ports.p01_sink_calls(), 0);
        assert_eq!(ports.other_push_kind_calls(), 0);
    }

    #[tokio::test]
    async fn p01_push_outcome_without_durable_claim_is_not_delivery_evidence() {
        let ports = RecordingP01Ports::pushed_without_durable_claim();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Compensation,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(
            outcome,
            P01RunOutcome::TerminalFailure(ref failure)
                if failure.reason_code() == "p01_durable_claim_missing_after_push"
        ));
        assert_eq!(ports.p01_sink_calls(), 1);
    }

    #[tokio::test]
    async fn p01_sink_error_without_durable_claim_remains_retryable() {
        let ports = RecordingP01Ports::sink_error_without_durable_claim();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Scheduled,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(
            outcome,
            P01RunOutcome::RetryableFailure(ref failure)
                if failure.reason_code() == "p01_counted_sink_error"
        ));
        assert_eq!(ports.p01_sink_calls(), 1);
    }

    #[tokio::test]
    async fn p01_sink_failure_retains_the_exact_canonical_source_fingerprint() {
        let ports = RecordingP01Ports::sink_error_without_durable_claim();
        let context = P01BusinessContext::new(date(2026, 8, 18)).unwrap();
        let mode = P01RenderMode::Scheduled;
        let input = P01InputBinding::complete_test_input();
        let rendered = crate::push_templates::render_bound_preopen_news_hot(mode, &input).unwrap();
        let canonical = input.canonical_source_bytes(mode, &rendered).unwrap();
        let expected_source_evidence_sha256 = sha256_hex(&canonical);

        let outcome =
            run_p01_once_with_ports(P01ExecutionMode::Scheduled, context, observed_at(), &ports)
                .await;

        match outcome {
            P01RunOutcome::RetryableFailure(failure) => {
                assert_eq!(failure.reason_code(), "p01_counted_sink_error");
                assert_eq!(
                    failure.source_evidence_sha256(),
                    Some(expected_source_evidence_sha256.as_str())
                );
            }
            other => panic!("expected TEST_CODE retryable sink failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn p01_deduped_requires_delivered_postinspect_and_reports_existing_delivery() {
        let ports = RecordingP01Ports::deduped_with_delivered_claim();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Compensation,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(outcome, P01RunOutcome::AlreadyDelivered { .. }));
        assert_eq!(ports.p01_sink_calls(), 1);
    }

    #[tokio::test]
    async fn p01_uncertain_postinspect_never_becomes_retryable_resend() {
        let ports = RecordingP01Ports::uncertain_after_sink_error();
        let outcome = run_p01_once_with_ports(
            P01ExecutionMode::Compensation,
            P01BusinessContext::new(date(2026, 8, 18)).unwrap(),
            observed_at(),
            &ports,
        )
        .await;

        assert!(matches!(
            outcome,
            P01RunOutcome::AwaitingReconciliation { ref attempt_identity }
                if attempt_identity == "TEST_CODE_P01_ATTEMPT"
        ));
        assert_eq!(ports.p01_sink_calls(), 1);
    }
}
