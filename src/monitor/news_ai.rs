//! BR-172 evidence-preserving NewsAI core.
//!
//! This module is deliberately pure. It validates evidence, builds a
//! deterministic prompt, validates a strict model response and binds that
//! response to an actual model-call receipt. It does not acquire data, call a
//! model, persist predictions, reserve delivery identity, push, or commit
//! deduplication state.

use crate::calendar::latest_completed_trading_day_at;
use crate::data_gateway::historical_bars::AdmittedDailyBars;
use crate::data_gateway::market_data::AdmittedRealtimeQuote;
use crate::data_gateway::{BatchEvidence, GlobalNewsRecord, SinaInstrumentNewsRecord};
use crate::llm::{
    LlmError, LlmProvider, ModelCallReceipt as ProviderModelCallReceipt, ReceiptBearingJson,
};
use crate::magic_compat::ProviderId;
use crate::magic_compat::SourceEvidence;
use crate::news::aggregator::AdmittedGlobalNewsBatch;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use thiserror::Error;

const REALTIME_MAX_AGE_SECONDS: i64 = 5;
/// 数据证据时间戳 vs 分析时刻的容差(秒)。TDX servertime 与本地时钟存在
/// 结构性偏差(6-63s, 见 tdx_server_probe.rs), 且 server 端批次的
/// observed_at 是请求时刻而 source_at 是 provider 响应时刻 —— 证据时间戳
/// 晚于 as_of 是正常现象, 不能按 0 容差判定 "in the future"。
const DATA_EVIDENCE_FUTURE_TOLERANCE_SECONDS: i64 = 90;
const REQUIRED_DAILY_HISTORY: usize = 20;
const MODEL_CALL_TIMEOUT_SECONDS: u64 = 45;
pub const NEWS_AI_ANALYSIS_VERSION: &str = "news_ai_v1";
pub const NEWS_AI_SYSTEM_PROMPT_V1: &str = concat!(
    "You are an evidence-bound A-share news analyst. ",
    "Use only the supplied normalized evidence. ",
    "Do not infer missing facts or provide trading instructions. ",
    "Return exactly one JSON object with no markdown and exactly these fields: ",
    "{\"impact\":\"major_negative|negative|neutral|positive|major_positive\",",
    "\"confidence\":0,\"uncertainty\":\"non-empty text\",",
    "\"core_logic\":\"non-empty text\"}. ",
    "confidence must be an integer from 0 through 100."
);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NewsAiError {
    #[error("news evidence mismatch: {0}")]
    NewsEvidenceMismatch(String),
    #[error("instrument is not source-bound: {0}")]
    InstrumentNotSourceBound(String),
    #[error("news content is insufficient: {0}")]
    InsufficientContent(String),
    #[error("market evidence mismatch: {0}")]
    MarketEvidenceMismatch(String),
    #[error("realtime quote is stale: {0}")]
    StaleQuote(String),
    #[error("daily bars are stale: {0}")]
    StaleDailyBars(String),
    #[error("daily history is insufficient: {0}")]
    InsufficientDailyHistory(String),
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("model receipt missing")]
    ModelReceiptMissing,
    #[error("invalid model schema: {0}")]
    InvalidModelSchema(String),
    #[error("analysis audit failed: {0}")]
    AnalysisAuditFailed(String),
    #[error("prediction commit failed: {0}")]
    PredictionCommitFailed(String),
    #[error("delivery denied: {0}")]
    DeliveryDenied(String),
    #[error("delivery sink failed: {0}")]
    DeliverySinkFailed(String),
}

impl NewsAiError {
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::NewsEvidenceMismatch(_) => "news_evidence_mismatch",
            Self::InstrumentNotSourceBound(_) => "instrument_not_source_bound",
            Self::InsufficientContent(_) => "insufficient_content",
            Self::MarketEvidenceMismatch(_) => "market_evidence_mismatch",
            Self::StaleQuote(_) => "stale_quote",
            Self::StaleDailyBars(_) => "stale_daily_bars",
            Self::InsufficientDailyHistory(_) => "insufficient_daily_history",
            Self::ModelUnavailable(_) => "model_unavailable",
            Self::ModelReceiptMissing => "model_receipt_missing",
            Self::InvalidModelSchema(_) => "invalid_model_schema",
            Self::AnalysisAuditFailed(_) => "analysis_audit_failed",
            Self::PredictionCommitFailed(_) => "prediction_commit_failed",
            Self::DeliveryDenied(_) => "delivery_denied",
            Self::DeliverySinkFailed(_) => "delivery_sink_failed",
        }
    }
}

#[derive(Debug, Clone)]
enum AdmittedNewsSourceRecord {
    Global(GlobalNewsRecord),
    Sina(SinaInstrumentNewsRecord),
}

/// A source record that remains inseparable from its original batch evidence
/// and exact target instrument.
#[derive(Debug, Clone)]
pub struct AdmittedNewsFact {
    record: AdmittedNewsSourceRecord,
    batch: BatchEvidence,
    target_code: String,
}

impl AdmittedNewsFact {
    pub fn from_admitted_global(
        admitted: &AdmittedGlobalNewsBatch,
        record_index: usize,
        target_code: &str,
    ) -> Result<Self, NewsAiError> {
        let record = admitted.records().get(record_index).ok_or_else(|| {
            NewsAiError::NewsEvidenceMismatch(format!(
                "global-news record index {record_index} is outside admitted batch"
            ))
        })?;
        Self::from_global_parts(record, admitted.evidence(), target_code)
    }

    #[cfg(test)]
    pub(crate) fn from_global(
        record: &GlobalNewsRecord,
        batch: &BatchEvidence,
        target_code: &str,
    ) -> Result<Self, NewsAiError> {
        Self::from_global_parts(record, batch, target_code)
    }

    fn from_global_parts(
        record: &GlobalNewsRecord,
        batch: &BatchEvidence,
        target_code: &str,
    ) -> Result<Self, NewsAiError> {
        validate_code(target_code)?;
        validate_batch_evidence(batch)?;
        if batch.source
            != expected_global_source(batch.provider).ok_or_else(|| {
                NewsAiError::NewsEvidenceMismatch(format!(
                    "provider {} is not an admitted global-news provider",
                    provider_tag(batch.provider)
                ))
            })?
        {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "global-news provider/source contract differs from BR-166".to_owned(),
            ));
        }
        validate_source_evidence(&record.evidence, batch)?;
        let observed_at = parse_observed_at(&batch.observed_at)?;
        if record.observed_at != observed_at {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "global record observation differs from batch".to_owned(),
            ));
        }
        let record_source_at = record.evidence.source_at().ok_or_else(|| {
            NewsAiError::NewsEvidenceMismatch(
                "global record provider publication time is missing".to_owned(),
            )
        })?;
        if parse_source_at(batch.provider, record_source_at)? != record.published_at {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "global record publication time differs from record evidence".to_owned(),
            ));
        }
        if record.published_at > record.observed_at {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "global record was published after observation".to_owned(),
            ));
        }
        if !record
            .instruments
            .iter()
            .any(|code| codes_equal(code, target_code))
        {
            return Err(NewsAiError::InstrumentNotSourceBound(format!(
                "global item {} does not explicitly name {target_code}",
                record.item_id
            )));
        }
        validate_news_fields(
            &record.item_id,
            &record.title,
            record.summary.as_deref(),
            record.content.as_deref(),
            &record.canonical_url,
        )?;

        Ok(Self {
            record: AdmittedNewsSourceRecord::Global(record.clone()),
            batch: batch.clone(),
            target_code: target_code.to_owned(),
        })
    }

    pub fn from_sina(
        record: &SinaInstrumentNewsRecord,
        batch: &BatchEvidence,
        target_code: &str,
    ) -> Result<Self, NewsAiError> {
        validate_code(target_code)?;
        validate_batch_evidence(batch)?;
        validate_source_evidence(record.evidence(), batch)?;
        if batch.provider != ProviderId::Sina || batch.source != "sina-company-news" {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "Sina instrument news has unexpected batch provider/source".to_owned(),
            ));
        }
        let item = record.persistence_item();
        if item.source != "sina_stock" || item.external_id != item.url {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "Sina persistence identity/source differs from admitted record".to_owned(),
            ));
        }
        let item_code = item.code.as_deref().ok_or_else(|| {
            NewsAiError::InstrumentNotSourceBound(
                "Sina instrument-news projection has no code".to_owned(),
            )
        })?;
        if !codes_equal(item_code, target_code) {
            return Err(NewsAiError::InstrumentNotSourceBound(format!(
                "Sina item code {item_code} differs from {target_code}"
            )));
        }
        let observed_at = parse_observed_at(&batch.observed_at)?;
        if item.fetched_at != observed_at {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "Sina item observation differs from batch".to_owned(),
            ));
        }
        let record_source_at = record.evidence().source_at().ok_or_else(|| {
            NewsAiError::NewsEvidenceMismatch(
                "Sina record provider publication time is missing".to_owned(),
            )
        })?;
        if parse_source_at(ProviderId::Sina, record_source_at)? != item.published_at {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "Sina item publication time differs from record evidence".to_owned(),
            ));
        }
        if item.published_at > item.fetched_at {
            return Err(NewsAiError::NewsEvidenceMismatch(
                "Sina item was published after observation".to_owned(),
            ));
        }
        validate_news_fields(
            &item.external_id,
            &item.title,
            nonempty_optional(&item.summary),
            None,
            &item.url,
        )?;

        Ok(Self {
            record: AdmittedNewsSourceRecord::Sina(record.clone()),
            batch: batch.clone(),
            target_code: target_code.to_owned(),
        })
    }

    pub fn provider(&self) -> ProviderId {
        self.batch.provider
    }

    pub fn source(&self) -> &str {
        &self.batch.source
    }

    pub fn source_batch_id(&self) -> &str {
        &self.batch.batch_id
    }

    pub fn target_code(&self) -> &str {
        &self.target_code
    }

    pub fn item_id(&self) -> &str {
        match &self.record {
            AdmittedNewsSourceRecord::Global(record) => &record.item_id,
            AdmittedNewsSourceRecord::Sina(record) => &record.persistence_item().external_id,
        }
    }

    pub fn title(&self) -> &str {
        match &self.record {
            AdmittedNewsSourceRecord::Global(record) => &record.title,
            AdmittedNewsSourceRecord::Sina(record) => &record.persistence_item().title,
        }
    }

    pub fn summary(&self) -> Option<&str> {
        match &self.record {
            AdmittedNewsSourceRecord::Global(record) => record.summary.as_deref(),
            AdmittedNewsSourceRecord::Sina(record) => {
                nonempty_optional(&record.persistence_item().summary)
            }
        }
    }

    pub fn content(&self) -> Option<&str> {
        match &self.record {
            AdmittedNewsSourceRecord::Global(record) => record.content.as_deref(),
            AdmittedNewsSourceRecord::Sina(_) => None,
        }
    }

    pub fn published_at(&self) -> DateTime<Utc> {
        match &self.record {
            AdmittedNewsSourceRecord::Global(record) => record.published_at,
            AdmittedNewsSourceRecord::Sina(record) => record.persistence_item().published_at,
        }
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        match &self.record {
            AdmittedNewsSourceRecord::Global(record) => record.observed_at,
            AdmittedNewsSourceRecord::Sina(record) => record.persistence_item().fetched_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsMarketContext {
    Intraday,
    PostClose,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SettledDailyBarInput {
    pub date: NaiveDate,
    pub close: f64,
    pub volume: f64,
    pub settled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RealtimeQuoteInput {
    pub code: String,
    pub price: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub evidence: BatchEvidence,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct NewsMarketEvidenceInput {
    pub target_code: String,
    pub context: NewsMarketContext,
    pub as_of: DateTime<Utc>,
    /// Supplied by the repository trading-calendar owner. This core never
    /// guesses weekends or holidays.
    pub latest_completed_trading_day: NaiveDate,
    /// Newest first, all settled.
    pub daily_bars: Vec<SettledDailyBarInput>,
    pub daily_evidence: BatchEvidence,
    pub quote: Option<RealtimeQuoteInput>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewsMarketMetrics {
    pub latest_close: f64,
    pub ma5: f64,
    pub ma10: f64,
    pub ma20: f64,
    pub return_5d_percent: f64,
    pub bias_5_percent: f64,
    pub latest_volume: f64,
    pub average_volume_5d: f64,
    pub average_volume_20d: f64,
}

#[derive(Debug, Clone)]
pub struct NewsMarketSnapshot {
    target_code: String,
    context: NewsMarketContext,
    as_of: DateTime<Utc>,
    daily_bars: Vec<SettledDailyBarInput>,
    daily_evidence: BatchEvidence,
    quote: Option<RealtimeQuoteInput>,
    metrics: NewsMarketMetrics,
}

impl NewsMarketSnapshot {
    /// Build decision-ready market evidence only from capabilities produced by
    /// the audited production Gateways. The caller supplies the analysis
    /// target and time/context, but cannot supply the market identity,
    /// provider evidence, expected completed session or normalized values.
    pub fn try_from_admitted(
        target_code: &str,
        context: NewsMarketContext,
        as_of: DateTime<Utc>,
        daily: AdmittedDailyBars,
        quote: Option<AdmittedRealtimeQuote>,
    ) -> Result<Self, NewsAiError> {
        validate_code(target_code)?;
        let (daily_target_code, records, daily_evidence) = daily.into_bound_parts();
        if !codes_equal(&daily_target_code, target_code) {
            return Err(market_error(format!(
                "daily-bar code {} differs from {target_code}",
                daily_target_code
            )));
        }

        let quote = quote
            .map(|admitted| {
                if !codes_equal(admitted.code(), target_code) {
                    return Err(market_error(format!(
                        "quote code {} differs from {target_code}",
                        admitted.code()
                    )));
                }
                Ok(RealtimeQuoteInput {
                    code: admitted.code().to_owned(),
                    price: admitted.price(),
                    source_at: admitted.source_at(),
                    observed_at: admitted.observed_at(),
                    evidence: admitted.evidence().clone(),
                })
            })
            .transpose()?;
        let daily_bars = records
            .into_iter()
            .map(|bar| SettledDailyBarInput {
                date: bar.date,
                close: bar.close,
                volume: bar.volume,
                settled: bar.settled,
            })
            .collect();
        let latest_completed_trading_day =
            latest_completed_trading_day_at(china_naive_datetime(as_of)?);
        Self::try_from_parts(
            target_code.to_owned(),
            context,
            as_of,
            latest_completed_trading_day,
            daily_bars,
            daily_evidence,
            quote,
        )
    }

    #[cfg(test)]
    pub(crate) fn try_from_input(input: NewsMarketEvidenceInput) -> Result<Self, NewsAiError> {
        Self::try_from_parts(
            input.target_code,
            input.context,
            input.as_of,
            input.latest_completed_trading_day,
            input.daily_bars,
            input.daily_evidence,
            input.quote,
        )
    }

    fn try_from_parts(
        target_code: String,
        context: NewsMarketContext,
        as_of: DateTime<Utc>,
        latest_completed_trading_day: NaiveDate,
        daily_bars: Vec<SettledDailyBarInput>,
        daily_evidence: BatchEvidence,
        quote: Option<RealtimeQuoteInput>,
    ) -> Result<Self, NewsAiError> {
        validate_code(&target_code)?;
        validate_batch_evidence(&daily_evidence)
            .map_err(|error| market_error(error.to_string()))?;
        let daily_observed_at = parse_observed_at(&daily_evidence.observed_at)
            .map_err(|error| market_error(error.to_string()))?;
        if daily_observed_at
            > as_of + chrono::Duration::seconds(DATA_EVIDENCE_FUTURE_TOLERANCE_SECONDS)
        {
            return Err(market_error("daily batch observation is in the future"));
        }
        let metrics = validate_daily_bars(&daily_bars, latest_completed_trading_day)?;

        match context {
            NewsMarketContext::Intraday => {
                let quote = quote
                    .as_ref()
                    .ok_or_else(|| market_error("intraday context requires one realtime quote"))?;
                validate_quote(&target_code, quote, as_of)?;
            }
            NewsMarketContext::PostClose if quote.is_some() => {
                return Err(market_error(
                    "post-close context must not contain an intraday quote",
                ));
            }
            NewsMarketContext::PostClose => {}
        }

        Ok(Self {
            target_code,
            context,
            as_of,
            daily_bars,
            daily_evidence,
            quote,
            metrics,
        })
    }

    pub fn context(&self) -> NewsMarketContext {
        self.context
    }

    pub fn target_code(&self) -> &str {
        &self.target_code
    }

    pub fn metrics(&self) -> &NewsMarketMetrics {
        &self.metrics
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricEvidence {
    provider: ProviderId,
    batch_id: String,
    observed_at: DateTime<Utc>,
}

impl MetricEvidence {
    pub fn try_new(
        provider: ProviderId,
        batch_id: impl Into<String>,
        observed_at: DateTime<Utc>,
    ) -> Result<Self, NewsAiError> {
        let batch_id = batch_id.into();
        if batch_id.trim().is_empty() {
            return Err(market_error("optional metric batch ID is empty"));
        }
        Ok(Self {
            provider,
            batch_id,
            observed_at,
        })
    }
}

/// There is intentionally no `Default`: every optional field must be either
/// evidence-bearing or explicitly unavailable.
#[derive(Debug, Clone, PartialEq)]
pub enum EvidenceStatus<T> {
    Available { value: T, evidence: MetricEvidence },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionalNewsMetric {
    pub name: String,
    pub status: EvidenceStatus<f64>,
}

#[derive(Debug, Clone)]
pub struct NewsAiRequest {
    fact: AdmittedNewsFact,
    market: NewsMarketSnapshot,
    optional_metrics: Vec<OptionalNewsMetric>,
    analysis_version: String,
    evidence_hash: String,
    normalized_prompt: String,
}

impl NewsAiRequest {
    pub fn try_new(
        fact: AdmittedNewsFact,
        market: NewsMarketSnapshot,
        mut optional_metrics: Vec<OptionalNewsMetric>,
        analysis_version: &str,
    ) -> Result<Self, NewsAiError> {
        if analysis_version.trim().is_empty() {
            return Err(market_error("analysis version is empty"));
        }
        if !codes_equal(fact.target_code(), market.target_code()) {
            return Err(market_error(format!(
                "news code {} differs from market code {}",
                fact.target_code(),
                market.target_code()
            )));
        }
        validate_optional_metrics(&mut optional_metrics, market.as_of)?;
        let evidence_hash =
            hash_request_evidence(&fact, &market, &optional_metrics, analysis_version);
        let normalized_prompt =
            build_normalized_prompt(&fact, &market, &optional_metrics, analysis_version)?;
        Ok(Self {
            fact,
            market,
            optional_metrics,
            analysis_version: analysis_version.to_owned(),
            evidence_hash,
            normalized_prompt,
        })
    }

    pub fn fact(&self) -> &AdmittedNewsFact {
        &self.fact
    }

    pub fn market(&self) -> &NewsMarketSnapshot {
        &self.market
    }

    pub fn optional_metrics(&self) -> &[OptionalNewsMetric] {
        &self.optional_metrics
    }

    pub fn analysis_version(&self) -> &str {
        &self.analysis_version
    }

    pub fn evidence_hash(&self) -> &str {
        &self.evidence_hash
    }

    pub fn normalized_prompt(&self) -> &str {
        &self.normalized_prompt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewsImpact {
    MajorNegative,
    Negative,
    Neutral,
    Positive,
    MajorPositive,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictModelOutput {
    pub impact: NewsImpact,
    pub confidence: u8,
    pub uncertainty: String,
    pub core_logic: String,
}

pub fn parse_strict_model_output(response: &str) -> Result<StrictModelOutput, NewsAiError> {
    let output: StrictModelOutput = serde_json::from_str(response)
        .map_err(|error| NewsAiError::InvalidModelSchema(error.to_string()))?;
    if output.confidence > 100 {
        return Err(NewsAiError::InvalidModelSchema(
            "confidence must be within 0..=100".to_owned(),
        ));
    }
    if output.uncertainty.trim().is_empty() || output.core_logic.trim().is_empty() {
        return Err(NewsAiError::InvalidModelSchema(
            "uncertainty and core_logic must be non-empty".to_owned(),
        ));
    }
    Ok(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCallReceipt {
    provider: String,
    model: String,
    upstream_request_id: Option<String>,
    upstream_response_id: String,
    system_sha256: String,
    user_sha256: String,
    response_sha256: String,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
}

impl ModelCallReceipt {
    fn try_from_provider(
        receipt: ProviderModelCallReceipt,
        system: &str,
        user: &str,
        response: &str,
    ) -> Result<Self, NewsAiError> {
        if receipt.provider().trim().is_empty() || receipt.model().trim().is_empty() {
            return Err(NewsAiError::ModelUnavailable(
                "actual provider/model is empty".to_owned(),
            ));
        }
        let upstream_request_id =
            validate_optional_model_id("upstream request", receipt.upstream_request_id())?;
        let upstream_response_id =
            validate_required_model_id("upstream response", receipt.upstream_response_id())?;
        if receipt.completed_at() < receipt.started_at() {
            return Err(NewsAiError::ModelUnavailable(
                "model completion precedes start".to_owned(),
            ));
        }
        let expected_system_sha256 = sha256_hex(system.as_bytes());
        let expected_user_sha256 = sha256_hex(user.as_bytes());
        let expected_response_sha256 = sha256_hex(response.as_bytes());
        if receipt.system_sha256() != expected_system_sha256 {
            return Err(NewsAiError::ModelUnavailable(
                "model receipt system hash differs from versioned NewsAI instruction".to_owned(),
            ));
        }
        if receipt.user_sha256() != expected_user_sha256 {
            return Err(NewsAiError::ModelUnavailable(
                "model receipt user hash differs from normalized NewsAI prompt".to_owned(),
            ));
        }
        if receipt.response_sha256() != expected_response_sha256 {
            return Err(NewsAiError::ModelUnavailable(
                "model receipt response hash differs from raw response".to_owned(),
            ));
        }
        Ok(Self {
            provider: receipt.provider().to_owned(),
            model: receipt.model().to_owned(),
            upstream_request_id,
            upstream_response_id,
            system_sha256: expected_system_sha256,
            user_sha256: expected_user_sha256,
            response_sha256: expected_response_sha256,
            started_at: *receipt.started_at(),
            completed_at: *receipt.completed_at(),
        })
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        provider: &str,
        model: &str,
        request_id: Option<&str>,
        prompt: &str,
        response: &str,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
    ) -> Result<Self, NewsAiError> {
        let (_, _, receipt) = ReceiptBearingJson::test_fixture(
            provider,
            model,
            request_id,
            "TEST_CODE_MODEL_RESPONSE",
            NEWS_AI_SYSTEM_PROMPT_V1,
            prompt,
            response,
            started_at,
            completed_at,
        )
        .into_parts();
        Self::try_from_provider(receipt, NEWS_AI_SYSTEM_PROMPT_V1, prompt, response)
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn upstream_request_id(&self) -> Option<&str> {
        self.upstream_request_id.as_deref()
    }

    pub fn upstream_response_id(&self) -> &str {
        &self.upstream_response_id
    }

    pub fn system_sha256(&self) -> &str {
        &self.system_sha256
    }

    pub fn user_sha256(&self) -> &str {
        &self.user_sha256
    }

    pub fn response_sha256(&self) -> &str {
        &self.response_sha256
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    pub fn completed_at(&self) -> DateTime<Utc> {
        self.completed_at
    }

    fn try_from_persisted(input: PersistedModelCallReceipt) -> Result<Self, NewsAiError> {
        validate_nonempty_id("persisted model provider", &input.provider)?;
        validate_nonempty_id("persisted model", &input.model)?;
        let upstream_request_id = validate_optional_model_id(
            "persisted upstream request",
            input.upstream_request_id.as_deref(),
        )?;
        let upstream_response_id = validate_required_model_id(
            "persisted upstream response",
            Some(&input.upstream_response_id),
        )?;
        for (label, value) in [
            ("persisted model system", input.system_sha256.as_str()),
            ("persisted model user", input.user_sha256.as_str()),
            ("persisted model response", input.response_sha256.as_str()),
        ] {
            validate_sha256(label, value)?;
        }
        if input.completed_at < input.started_at {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted model completion precedes start".to_owned(),
            ));
        }
        Ok(Self {
            provider: input.provider,
            model: input.model,
            upstream_request_id,
            upstream_response_id,
            system_sha256: input.system_sha256,
            user_sha256: input.user_sha256,
            response_sha256: input.response_sha256,
            started_at: input.started_at,
            completed_at: input.completed_at,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PersistedModelCallReceipt {
    pub provider: String,
    pub model: String,
    pub upstream_request_id: Option<String>,
    pub upstream_response_id: String,
    pub system_sha256: String,
    pub user_sha256: String,
    pub response_sha256: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiAssessment {
    assessment_id: String,
    impact: NewsImpact,
    confidence: u8,
    uncertainty: String,
    core_logic: String,
    input_evidence_sha256: String,
    normalized_prompt_sha256: String,
    receipt: ModelCallReceipt,
}

impl NewsAiAssessment {
    pub fn from_model_response(
        request: &NewsAiRequest,
        response: &str,
        receipt: Option<ModelCallReceipt>,
    ) -> Result<Self, NewsAiError> {
        let receipt = receipt.ok_or(NewsAiError::ModelReceiptMissing)?;
        let prompt_hash = sha256_hex(request.normalized_prompt().as_bytes());
        if receipt.system_sha256 != sha256_hex(NEWS_AI_SYSTEM_PROMPT_V1.as_bytes()) {
            return Err(NewsAiError::ModelUnavailable(
                "model receipt system hash differs from NewsAI v1 instruction".to_owned(),
            ));
        }
        if receipt.user_sha256 != prompt_hash {
            return Err(NewsAiError::ModelUnavailable(
                "model receipt user hash differs from normalized prompt".to_owned(),
            ));
        }
        if receipt.response_sha256 != sha256_hex(response.as_bytes()) {
            return Err(NewsAiError::ModelUnavailable(
                "model receipt response hash differs from response".to_owned(),
            ));
        }
        let output = parse_strict_model_output(response)?;
        let assessment_id = assessment_identity(request);
        Ok(Self {
            assessment_id,
            impact: output.impact,
            confidence: output.confidence,
            uncertainty: output.uncertainty,
            core_logic: output.core_logic,
            input_evidence_sha256: request.evidence_hash.clone(),
            normalized_prompt_sha256: prompt_hash,
            receipt,
        })
    }

    pub fn assessment_id(&self) -> &str {
        &self.assessment_id
    }

    pub fn impact(&self) -> NewsImpact {
        self.impact
    }

    pub fn confidence(&self) -> u8 {
        self.confidence
    }

    pub fn uncertainty(&self) -> &str {
        &self.uncertainty
    }

    pub fn core_logic(&self) -> &str {
        &self.core_logic
    }

    pub fn input_evidence_sha256(&self) -> &str {
        &self.input_evidence_sha256
    }

    pub fn normalized_prompt_sha256(&self) -> &str {
        &self.normalized_prompt_sha256
    }

    pub fn receipt(&self) -> &ModelCallReceipt {
        &self.receipt
    }

    pub(crate) fn try_from_persisted(
        input: PersistedNewsAiAssessment,
    ) -> Result<Self, NewsAiError> {
        validate_sha256("persisted assessment identity", &input.assessment_id)?;
        validate_sha256(
            "persisted assessment input evidence",
            &input.input_evidence_sha256,
        )?;
        validate_sha256(
            "persisted assessment prompt",
            &input.normalized_prompt_sha256,
        )?;
        if input.confidence > 100 {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted assessment confidence exceeds 100".to_owned(),
            ));
        }
        if input.uncertainty.trim().is_empty() || input.core_logic.trim().is_empty() {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted assessment reasoning is empty".to_owned(),
            ));
        }
        let receipt = ModelCallReceipt::try_from_persisted(input.receipt)?;
        if receipt.system_sha256() != sha256_hex(NEWS_AI_SYSTEM_PROMPT_V1.as_bytes())
            || receipt.user_sha256() != input.normalized_prompt_sha256
        {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted model receipt differs from NewsAI prompt evidence".to_owned(),
            ));
        }
        Ok(Self {
            assessment_id: input.assessment_id,
            impact: input.impact,
            confidence: input.confidence,
            uncertainty: input.uncertainty,
            core_logic: input.core_logic,
            input_evidence_sha256: input.input_evidence_sha256,
            normalized_prompt_sha256: input.normalized_prompt_sha256,
            receipt,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PersistedNewsAiAssessment {
    pub assessment_id: String,
    pub impact: NewsImpact,
    pub confidence: u8,
    pub uncertainty: String,
    pub core_logic: String,
    pub input_evidence_sha256: String,
    pub normalized_prompt_sha256: String,
    pub receipt: PersistedModelCallReceipt,
}

/// Exact BR-172 delivery identity. Its hash is stable across retries and is
/// bound only to admitted source identity plus the analysis version; display
/// text, process time and model output never alter the identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiDeliveryIdentity {
    provider: ProviderId,
    source_batch_id: String,
    source_item_id: String,
    target_code: String,
    analysis_version: String,
    sha256: String,
}

impl NewsAiDeliveryIdentity {
    fn from_fact(fact: &AdmittedNewsFact, analysis_version: &str) -> Self {
        let mut hasher = Sha256::new();
        for value in [
            provider_tag(fact.provider()),
            fact.source_batch_id(),
            fact.item_id(),
            fact.target_code(),
            analysis_version,
        ] {
            hash_field(&mut hasher, value);
        }
        Self {
            provider: fact.provider(),
            source_batch_id: fact.source_batch_id().to_owned(),
            source_item_id: fact.item_id().to_owned(),
            target_code: fact.target_code().to_owned(),
            analysis_version: analysis_version.to_owned(),
            sha256: format!("{:x}", hasher.finalize()),
        }
    }

    pub fn provider(&self) -> ProviderId {
        self.provider
    }

    pub fn source_batch_id(&self) -> &str {
        &self.source_batch_id
    }

    pub fn source_item_id(&self) -> &str {
        &self.source_item_id
    }

    pub fn target_code(&self) -> &str {
        &self.target_code
    }

    pub fn analysis_version(&self) -> &str {
        &self.analysis_version
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Delivery-ready projection whose constructor is crate-private. The database
/// audit owner must mint it from an append/verified-existing receipt; callers
/// cannot promote a bare model response into production delivery.
#[derive(Debug, Clone)]
pub struct AuditedNewsAiAssessment {
    delivery: GovernedNewsAiDelivery,
}

impl AuditedNewsAiAssessment {
    pub(crate) fn try_from_assessment_audit(
        request: NewsAiRequest,
        assessment: NewsAiAssessment,
        assessment_audit_record_sha256: &str,
    ) -> Result<Self, NewsAiError> {
        validate_sha256("assessment audit record", assessment_audit_record_sha256)?;
        let expected_assessment_id = assessment_identity(&request);
        if assessment.assessment_id() != expected_assessment_id {
            return Err(NewsAiError::AnalysisAuditFailed(
                "assessment identity differs from exact source identity".to_owned(),
            ));
        }
        let fact = request.fact().clone();
        let analysis_version = request.analysis_version().to_owned();
        let identity = NewsAiDeliveryIdentity::from_fact(&fact, &analysis_version);
        Ok(Self {
            delivery: GovernedNewsAiDelivery {
                fact,
                analysis_version,
                assessment,
                assessment_audit_record_sha256: assessment_audit_record_sha256.to_owned(),
                identity,
            },
        })
    }

    pub(crate) fn try_from_persisted_assessment_audit(
        fact: AdmittedNewsFact,
        analysis_version: &str,
        assessment: PersistedNewsAiAssessment,
        assessment_audit_record_sha256: &str,
    ) -> Result<Self, NewsAiError> {
        if analysis_version.trim().is_empty() {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted assessment analysis version is empty".to_owned(),
            ));
        }
        validate_sha256("assessment audit record", assessment_audit_record_sha256)?;
        let assessment = NewsAiAssessment::try_from_persisted(assessment)?;
        let expected_assessment_id = assessment_identity_for_fact(&fact, analysis_version);
        if assessment.assessment_id() != expected_assessment_id {
            return Err(NewsAiError::AnalysisAuditFailed(
                "persisted assessment identity differs from admitted source fact".to_owned(),
            ));
        }
        let identity = NewsAiDeliveryIdentity::from_fact(&fact, analysis_version);
        Ok(Self {
            delivery: GovernedNewsAiDelivery {
                fact,
                analysis_version: analysis_version.to_owned(),
                assessment,
                assessment_audit_record_sha256: assessment_audit_record_sha256.to_owned(),
                identity,
            },
        })
    }

    pub fn delivery(&self) -> &GovernedNewsAiDelivery {
        &self.delivery
    }
}

/// Complete immutable input presented at the governed-delivery seam.
#[derive(Debug, Clone)]
pub struct GovernedNewsAiDelivery {
    fact: AdmittedNewsFact,
    analysis_version: String,
    assessment: NewsAiAssessment,
    assessment_audit_record_sha256: String,
    identity: NewsAiDeliveryIdentity,
}

impl GovernedNewsAiDelivery {
    pub fn fact(&self) -> &AdmittedNewsFact {
        &self.fact
    }

    pub fn analysis_version(&self) -> &str {
        &self.analysis_version
    }

    pub fn assessment(&self) -> &NewsAiAssessment {
        &self.assessment
    }

    pub fn assessment_audit_record_sha256(&self) -> &str {
        &self.assessment_audit_record_sha256
    }

    pub fn identity(&self) -> &NewsAiDeliveryIdentity {
        &self.identity
    }

    /// Render only immutable source/model/audit evidence. This card does not
    /// infer holdings, prices or trading actions and has no default values.
    pub fn render_card(&self) -> String {
        let impact = match self.assessment.impact() {
            NewsImpact::MajorNegative => "重大负面",
            NewsImpact::Negative => "负面",
            NewsImpact::Neutral => "中性",
            NewsImpact::Positive => "正面",
            NewsImpact::MajorPositive => "重大正面",
        };
        format!(
            "🧠 AI 新闻证据分析\n\
             标的：{}\n\
             标题：{}\n\
             来源：{} / {:?}\n\
             发布时间：{}\n\
             影响：{}（置信度 {}%）\n\
             核心逻辑：{}\n\
             不确定性：{}\n\
             模型：{} / {}\n\
             模型响应：{}\n\
             证据哈希：{}\n\
             评估审计：{}\n\
             投递身份：{}\n\
             ⚠️ 仅为来源绑定的模型分析，不构成交易建议。",
            self.fact.target_code(),
            self.fact.title(),
            self.fact.source(),
            self.fact.provider(),
            self.fact.published_at().to_rfc3339(),
            impact,
            self.assessment.confidence(),
            self.assessment.core_logic(),
            self.assessment.uncertainty(),
            self.assessment.receipt().provider(),
            self.assessment.receipt().model(),
            self.assessment.receipt().upstream_response_id(),
            self.assessment.input_evidence_sha256(),
            self.assessment_audit_record_sha256,
            self.identity.sha256(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiDeliveryReservation {
    delivery_identity_sha256: String,
    reservation_id: String,
}

impl NewsAiDeliveryReservation {
    pub(crate) fn try_new(
        delivery_identity_sha256: &str,
        reservation_id: &str,
    ) -> Result<Self, NewsAiError> {
        validate_sha256("delivery reservation identity", delivery_identity_sha256)?;
        validate_nonempty_id("delivery reservation", reservation_id)?;
        Ok(Self {
            delivery_identity_sha256: delivery_identity_sha256.to_owned(),
            reservation_id: reservation_id.to_owned(),
        })
    }

    pub fn delivery_identity_sha256(&self) -> &str {
        &self.delivery_identity_sha256
    }

    pub fn reservation_id(&self) -> &str {
        &self.reservation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsAiReserveOutcome {
    Reserved(NewsAiDeliveryReservation),
    LinkPending(NewsAiDeliveryLinkRecovery),
    Deduped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiDeliveryAuditReceipt {
    delivery_identity_sha256: String,
    audit_event_id: String,
}

impl NewsAiDeliveryAuditReceipt {
    pub(crate) fn try_new(
        delivery_identity_sha256: &str,
        audit_event_id: &str,
    ) -> Result<Self, NewsAiError> {
        validate_sha256("delivery audit identity", delivery_identity_sha256)?;
        validate_nonempty_id("delivery audit event", audit_event_id)?;
        Ok(Self {
            delivery_identity_sha256: delivery_identity_sha256.to_owned(),
            audit_event_id: audit_event_id.to_owned(),
        })
    }

    pub fn delivery_identity_sha256(&self) -> &str {
        &self.delivery_identity_sha256
    }

    pub fn audit_event_id(&self) -> &str {
        &self.audit_event_id
    }
}

/// Capability reconstructed only from a durable `delivered` ledger state.
/// It authorizes prediction linkage but carries no permission to call a sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiDeliveryLinkRecovery {
    reservation: NewsAiDeliveryReservation,
    delivery_audit: NewsAiDeliveryAuditReceipt,
}

impl NewsAiDeliveryLinkRecovery {
    pub(crate) fn try_new(
        reservation: NewsAiDeliveryReservation,
        delivery_audit: NewsAiDeliveryAuditReceipt,
    ) -> Result<Self, NewsAiError> {
        if reservation.delivery_identity_sha256() != delivery_audit.delivery_identity_sha256() {
            return Err(NewsAiError::AnalysisAuditFailed(
                "link recovery reservation differs from delivery audit".to_owned(),
            ));
        }
        Ok(Self {
            reservation,
            delivery_audit,
        })
    }

    pub fn reservation(&self) -> &NewsAiDeliveryReservation {
        &self.reservation
    }

    pub fn delivery_audit(&self) -> &NewsAiDeliveryAuditReceipt {
        &self.delivery_audit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsAiPhysicalPushOutcome {
    Pushed(NewsAiDeliveryAuditReceipt),
    Deduped,
    Denied(String),
    /// Definitive failure before the physical sink was attempted.
    SinkError(String),
    /// Sink was attempted or accepted, so retry is forbidden even when the
    /// post-sink audit could not be completed.
    PostSinkFailure {
        delivery_audit_event_id: Option<String>,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiPredictionLinkReceipt {
    delivery_identity_sha256: String,
    assessment_id: String,
    delivery_audit_event_id: String,
    prediction_link_id: String,
}

impl NewsAiPredictionLinkReceipt {
    pub fn try_new(
        delivery_identity_sha256: &str,
        assessment_id: &str,
        delivery_audit_event_id: &str,
        prediction_link_id: &str,
    ) -> Result<Self, NewsAiError> {
        validate_sha256(
            "prediction link delivery identity",
            delivery_identity_sha256,
        )?;
        validate_sha256("prediction link assessment identity", assessment_id)?;
        validate_nonempty_id(
            "prediction link delivery audit event",
            delivery_audit_event_id,
        )?;
        validate_sha256("prediction link", prediction_link_id)?;
        Ok(Self {
            delivery_identity_sha256: delivery_identity_sha256.to_owned(),
            assessment_id: assessment_id.to_owned(),
            delivery_audit_event_id: delivery_audit_event_id.to_owned(),
            prediction_link_id: prediction_link_id.to_owned(),
        })
    }

    pub fn prediction_link_id(&self) -> &str {
        &self.prediction_link_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsAiGovernedDeliveryOutcome {
    RetainedNoDelivery {
        assessment_id: String,
    },
    Pushed {
        delivery_identity_sha256: String,
        delivery_audit_event_id: String,
        prediction_link_id: String,
    },
    PredictionLinkRecovered {
        delivery_identity_sha256: String,
        delivery_audit_event_id: String,
        prediction_link_id: String,
    },
    Deduped {
        delivery_identity_sha256: String,
    },
    Denied {
        delivery_identity_sha256: String,
        reason: String,
    },
    SinkError {
        delivery_identity_sha256: String,
        reason: String,
    },
    ReserveFailed {
        delivery_identity_sha256: String,
        reason: String,
    },
    RollbackFailed {
        delivery_identity_sha256: String,
        original_outcome: String,
        reason: String,
    },
    PostSinkCommitFailed {
        delivery_identity_sha256: String,
        delivery_audit_event_id: String,
        reason: String,
    },
    PostSinkRecovery {
        delivery_identity_sha256: String,
        delivery_audit_event_id: Option<String>,
        reason: String,
    },
}

/// The adapter seam is intentionally narrow: production supplies the durable
/// reservation/prediction store and the normal governed sink; tests supply an
/// in-memory adapter. `Pushed` is impossible without a durable delivery-audit
/// receipt, so a transport-only success cannot advance settlement.
#[async_trait]
pub trait NewsAiGovernedDeliveryPort: Send + Sync {
    async fn reserve(
        &self,
        delivery: &GovernedNewsAiDelivery,
    ) -> Result<NewsAiReserveOutcome, String>;

    async fn push(
        &self,
        delivery: &GovernedNewsAiDelivery,
        reservation: &NewsAiDeliveryReservation,
    ) -> NewsAiPhysicalPushOutcome;

    async fn commit(
        &self,
        delivery: &GovernedNewsAiDelivery,
        reservation: &NewsAiDeliveryReservation,
        delivery_audit: &NewsAiDeliveryAuditReceipt,
    ) -> Result<NewsAiPredictionLinkReceipt, String>;

    async fn rollback(
        &self,
        delivery: &GovernedNewsAiDelivery,
        reservation: &NewsAiDeliveryReservation,
    ) -> Result<(), String>;
}

/// Deep BR-172 state machine. It owns ordering and settlement semantics; the
/// adapter cannot cause commit before a receipt-bearing physical delivery.
pub async fn deliver_governed_news_ai<P>(
    audited: &AuditedNewsAiAssessment,
    port: &P,
) -> NewsAiGovernedDeliveryOutcome
where
    P: NewsAiGovernedDeliveryPort + ?Sized,
{
    let delivery = audited.delivery();
    let identity = delivery.identity().sha256().to_owned();
    if delivery.assessment().impact() == NewsImpact::Neutral {
        return NewsAiGovernedDeliveryOutcome::RetainedNoDelivery {
            assessment_id: delivery.assessment().assessment_id().to_owned(),
        };
    }

    let reservation = match port.reserve(delivery).await {
        Ok(NewsAiReserveOutcome::Deduped) => {
            return NewsAiGovernedDeliveryOutcome::Deduped {
                delivery_identity_sha256: identity,
            };
        }
        Ok(NewsAiReserveOutcome::LinkPending(recovery)) => {
            if recovery.reservation().delivery_identity_sha256() != identity
                || recovery.delivery_audit().delivery_identity_sha256() != identity
            {
                return NewsAiGovernedDeliveryOutcome::ReserveFailed {
                    delivery_identity_sha256: identity,
                    reason: "link recovery identity differs from delivery".to_owned(),
                };
            }
            let reservation = recovery.reservation();
            let delivery_audit = recovery.delivery_audit();
            return match port.commit(delivery, reservation, delivery_audit).await {
                Ok(link)
                    if link.delivery_identity_sha256 == identity
                        && link.assessment_id == delivery.assessment().assessment_id()
                        && link.delivery_audit_event_id == delivery_audit.audit_event_id() =>
                {
                    NewsAiGovernedDeliveryOutcome::PredictionLinkRecovered {
                        delivery_identity_sha256: identity,
                        delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                        prediction_link_id: link.prediction_link_id().to_owned(),
                    }
                }
                Ok(_) => NewsAiGovernedDeliveryOutcome::PostSinkCommitFailed {
                    delivery_identity_sha256: identity,
                    delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                    reason: "recovered prediction link receipt differs from delivery".to_owned(),
                },
                Err(reason) => NewsAiGovernedDeliveryOutcome::PostSinkCommitFailed {
                    delivery_identity_sha256: identity,
                    delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                    reason,
                },
            };
        }
        Ok(NewsAiReserveOutcome::Reserved(reservation))
            if reservation.delivery_identity_sha256() == identity =>
        {
            reservation
        }
        Ok(NewsAiReserveOutcome::Reserved(_)) => {
            return NewsAiGovernedDeliveryOutcome::ReserveFailed {
                delivery_identity_sha256: identity,
                reason: "reservation identity differs from delivery".to_owned(),
            };
        }
        Err(reason) => {
            return NewsAiGovernedDeliveryOutcome::ReserveFailed {
                delivery_identity_sha256: identity,
                reason,
            };
        }
    };

    match port.push(delivery, &reservation).await {
        NewsAiPhysicalPushOutcome::Pushed(delivery_audit) => {
            if delivery_audit.delivery_identity_sha256() != identity {
                return NewsAiGovernedDeliveryOutcome::PostSinkCommitFailed {
                    delivery_identity_sha256: identity,
                    delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                    reason: "delivery audit identity differs from reservation".to_owned(),
                };
            }
            match port.commit(delivery, &reservation, &delivery_audit).await {
                Ok(link)
                    if link.delivery_identity_sha256 == identity
                        && link.assessment_id == delivery.assessment().assessment_id()
                        && link.delivery_audit_event_id == delivery_audit.audit_event_id() =>
                {
                    NewsAiGovernedDeliveryOutcome::Pushed {
                        delivery_identity_sha256: identity,
                        delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                        prediction_link_id: link.prediction_link_id().to_owned(),
                    }
                }
                Ok(_) => NewsAiGovernedDeliveryOutcome::PostSinkCommitFailed {
                    delivery_identity_sha256: identity,
                    delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                    reason: "prediction link receipt differs from delivery".to_owned(),
                },
                Err(reason) => NewsAiGovernedDeliveryOutcome::PostSinkCommitFailed {
                    delivery_identity_sha256: identity,
                    delivery_audit_event_id: delivery_audit.audit_event_id().to_owned(),
                    reason,
                },
            }
        }
        NewsAiPhysicalPushOutcome::Deduped => {
            rollback_or_failure(port, delivery, &reservation, identity, "deduped").await
        }
        NewsAiPhysicalPushOutcome::Denied(reason) => {
            let original = format!("denied:{reason}");
            match port.rollback(delivery, &reservation).await {
                Ok(()) => NewsAiGovernedDeliveryOutcome::Denied {
                    delivery_identity_sha256: identity,
                    reason,
                },
                Err(rollback_reason) => NewsAiGovernedDeliveryOutcome::RollbackFailed {
                    delivery_identity_sha256: identity,
                    original_outcome: original,
                    reason: rollback_reason,
                },
            }
        }
        NewsAiPhysicalPushOutcome::SinkError(reason) => {
            let original = format!("sink_error:{reason}");
            match port.rollback(delivery, &reservation).await {
                Ok(()) => NewsAiGovernedDeliveryOutcome::SinkError {
                    delivery_identity_sha256: identity,
                    reason,
                },
                Err(rollback_reason) => NewsAiGovernedDeliveryOutcome::RollbackFailed {
                    delivery_identity_sha256: identity,
                    original_outcome: original,
                    reason: rollback_reason,
                },
            }
        }
        NewsAiPhysicalPushOutcome::PostSinkFailure {
            delivery_audit_event_id,
            reason,
        } => NewsAiGovernedDeliveryOutcome::PostSinkRecovery {
            delivery_identity_sha256: identity,
            delivery_audit_event_id,
            reason,
        },
    }
}

async fn rollback_or_failure<P>(
    port: &P,
    delivery: &GovernedNewsAiDelivery,
    reservation: &NewsAiDeliveryReservation,
    identity: String,
    original_outcome: &str,
) -> NewsAiGovernedDeliveryOutcome
where
    P: NewsAiGovernedDeliveryPort + ?Sized,
{
    match port.rollback(delivery, reservation).await {
        Ok(()) => NewsAiGovernedDeliveryOutcome::Deduped {
            delivery_identity_sha256: identity,
        },
        Err(reason) => NewsAiGovernedDeliveryOutcome::RollbackFailed {
            delivery_identity_sha256: identity,
            original_outcome: original_outcome.to_owned(),
            reason,
        },
    }
}

fn validate_sha256(label: &str, value: &str) -> Result<(), NewsAiError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(NewsAiError::AnalysisAuditFailed(format!(
            "{label} must be a SHA-256 hex digest"
        )));
    }
    Ok(())
}

fn validate_nonempty_id(label: &str, value: &str) -> Result<(), NewsAiError> {
    if value.trim().is_empty() {
        return Err(NewsAiError::AnalysisAuditFailed(format!(
            "{label} ID is empty"
        )));
    }
    Ok(())
}

/// Side-effect-free BR-172 model adapter. Provider selection occurs once
/// outside this type; a failed call is never retried against another provider.
#[derive(Clone)]
pub struct NewsAIAnalyzer {
    provider: Arc<dyn LlmProvider>,
}

impl NewsAIAnalyzer {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    pub async fn assess(&self, request: &NewsAiRequest) -> Result<NewsAiAssessment, NewsAiError> {
        let completed = tokio::time::timeout(
            std::time::Duration::from_secs(MODEL_CALL_TIMEOUT_SECONDS),
            self.provider
                .chat_json_with_receipt(NEWS_AI_SYSTEM_PROMPT_V1, request.normalized_prompt()),
        )
        .await
        .map_err(|_| {
            NewsAiError::ModelUnavailable(format!(
                "model call exceeded {MODEL_CALL_TIMEOUT_SECONDS}s"
            ))
        })?
        .map_err(model_call_error)?;
        assessment_from_receipt_bearing_json(request, completed)
    }

    #[deprecated(note = "BR-172 rejects the legacy scalar/keyword decision interface")]
    pub async fn quick_decision(
        &self,
        _title: &str,
        _code: &str,
        _name: &str,
    ) -> Result<String, NewsAiError> {
        Err(NewsAiError::ModelUnavailable(
            "legacy quick_decision is disabled; no keyword fallback is permitted".to_owned(),
        ))
    }
}

fn assessment_from_receipt_bearing_json(
    request: &NewsAiRequest,
    completed: ReceiptBearingJson,
) -> Result<NewsAiAssessment, NewsAiError> {
    let (_parsed, raw_response, provider_receipt) = completed.into_parts();
    let receipt = ModelCallReceipt::try_from_provider(
        provider_receipt,
        NEWS_AI_SYSTEM_PROMPT_V1,
        request.normalized_prompt(),
        &raw_response,
    )?;
    NewsAiAssessment::from_model_response(request, &raw_response, Some(receipt))
}

fn model_call_error(error: LlmError) -> NewsAiError {
    NewsAiError::ModelUnavailable(error.to_string())
}

fn validate_optional_model_id(
    label: &str,
    value: Option<&str>,
) -> Result<Option<String>, NewsAiError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                Err(NewsAiError::ModelUnavailable(format!(
                    "present {label} ID is empty"
                )))
            } else {
                Ok(value.to_owned())
            }
        })
        .transpose()
}

fn validate_required_model_id(label: &str, value: Option<&str>) -> Result<String, NewsAiError> {
    validate_optional_model_id(label, value)?
        .ok_or_else(|| NewsAiError::ModelUnavailable(format!("{label} ID is missing")))
}

fn validate_batch_evidence(batch: &BatchEvidence) -> Result<(), NewsAiError> {
    if batch.source.trim().is_empty() || batch.batch_id.trim().is_empty() {
        return Err(NewsAiError::NewsEvidenceMismatch(
            "batch source or batch ID is empty".to_owned(),
        ));
    }
    parse_observed_at(&batch.observed_at)?;
    Ok(())
}

fn validate_source_evidence(
    record: &SourceEvidence,
    batch: &BatchEvidence,
) -> Result<(), NewsAiError> {
    if record.provider() != batch.provider
        || record.batch_id() != batch.batch_id
        || record.observed_at() != batch.observed_at
    {
        return Err(NewsAiError::NewsEvidenceMismatch(
            "record provider/batch/observation differs from batch".to_owned(),
        ));
    }
    Ok(())
}

fn validate_news_fields(
    item_id: &str,
    title: &str,
    summary: Option<&str>,
    content: Option<&str>,
    canonical_url: &str,
) -> Result<(), NewsAiError> {
    if item_id.trim().is_empty() {
        return Err(NewsAiError::NewsEvidenceMismatch(
            "item ID is empty".to_owned(),
        ));
    }
    if !canonical_url.starts_with("https://") {
        return Err(NewsAiError::NewsEvidenceMismatch(
            "canonical URL is not HTTPS".to_owned(),
        ));
    }
    if title.trim().is_empty()
        && summary.is_none_or(|value| value.trim().is_empty())
        && content.is_none_or(|value| value.trim().is_empty())
    {
        return Err(NewsAiError::InsufficientContent(
            "title, summary and content are all empty".to_owned(),
        ));
    }
    Ok(())
}

fn validate_code(code: &str) -> Result<(), NewsAiError> {
    let normalized = normalized_code(code);
    if normalized.len() != 6 || !normalized.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NewsAiError::InstrumentNotSourceBound(format!(
            "invalid A-share code {code:?}"
        )));
    }
    Ok(())
}

fn normalized_code(code: &str) -> &str {
    #[cfg(test)]
    {
        code.strip_prefix("TEST_CODE_").unwrap_or(code)
    }
    #[cfg(not(test))]
    {
        code
    }
}

fn codes_equal(left: &str, right: &str) -> bool {
    normalized_code(left) == normalized_code(right)
}

fn china_naive_datetime(value: DateTime<Utc>) -> Result<NaiveDateTime, NewsAiError> {
    let china = FixedOffset::east_opt(8 * 60 * 60)
        .ok_or_else(|| market_error("A-share UTC+08:00 offset is unavailable"))?;
    Ok(value.with_timezone(&china).naive_local())
}

fn parse_observed_at(value: &str) -> Result<DateTime<Utc>, NewsAiError> {
    if let Some((seconds, nanos)) = value.split_once('.') {
        if nanos.len() == 9 && nanos.bytes().all(|byte| byte.is_ascii_digit()) {
            let seconds = seconds.parse::<i64>().map_err(|error| {
                NewsAiError::NewsEvidenceMismatch(format!(
                    "invalid observation seconds {value:?}: {error}"
                ))
            })?;
            let nanos = nanos.parse::<u32>().map_err(|error| {
                NewsAiError::NewsEvidenceMismatch(format!(
                    "invalid observation nanoseconds {value:?}: {error}"
                ))
            })?;
            return DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
                NewsAiError::NewsEvidenceMismatch(format!(
                    "observation timestamp is out of range {value:?}"
                ))
            });
        }
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc));
    }
    if let Some(parsed) = parse_unix_epoch_seconds_or_millis(value) {
        return Ok(parsed);
    }
    Err(NewsAiError::NewsEvidenceMismatch(format!(
        "invalid observation timestamp {value:?}: neither RFC3339 nor unix epoch"
    )))
}

/// 解析 unix epoch 秒(10 位)或毫秒(13 位)格式的时间戳。
fn parse_unix_epoch_seconds_or_millis(value: &str) -> Option<DateTime<Utc>> {
    let digits = value.parse::<i64>().ok()?;
    if value.len() == 13 {
        // 毫秒
        DateTime::from_timestamp(digits / 1000, (digits % 1000) as u32 * 1_000_000)
    } else {
        // 秒
        DateTime::from_timestamp(digits, 0)
    }
}

fn parse_source_at(provider: ProviderId, value: &str) -> Result<DateTime<Utc>, NewsAiError> {
    if provider == ProviderId::Eastmoney {
        let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").map_err(|error| {
            NewsAiError::NewsEvidenceMismatch(format!(
                "invalid Eastmoney publication time {value:?}: {error}"
            ))
        })?;
        let china = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
            NewsAiError::NewsEvidenceMismatch("UTC+08:00 is unavailable".to_owned())
        })?;
        return china
            .from_local_datetime(&naive)
            .single()
            .map(|value| value.with_timezone(&Utc))
            .ok_or_else(|| {
                NewsAiError::NewsEvidenceMismatch(format!(
                    "ambiguous Eastmoney publication time {value:?}"
                ))
            });
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc));
    }
    if let Some(parsed) = parse_unix_epoch_seconds_or_millis(value) {
        return Ok(parsed);
    }
    Err(NewsAiError::NewsEvidenceMismatch(format!(
        "invalid publication time {value:?}: neither RFC3339 nor unix epoch"
    )))
}

fn validate_daily_bars(
    bars: &[SettledDailyBarInput],
    latest_completed_trading_day: NaiveDate,
) -> Result<NewsMarketMetrics, NewsAiError> {
    if bars.len() < REQUIRED_DAILY_HISTORY {
        return Err(NewsAiError::InsufficientDailyHistory(format!(
            "requires at least {REQUIRED_DAILY_HISTORY} settled bars, got {}",
            bars.len()
        )));
    }
    if bars[0].date != latest_completed_trading_day {
        return Err(NewsAiError::StaleDailyBars(format!(
            "latest daily bar {} differs from expected completed trading day {}",
            bars[0].date, latest_completed_trading_day
        )));
    }
    let mut seen = HashSet::with_capacity(bars.len());
    for (index, bar) in bars.iter().enumerate() {
        if !seen.insert(bar.date) {
            return Err(market_error(format!(
                "duplicate daily bar date {}",
                bar.date
            )));
        }
        if index > 0 && bars[index - 1].date <= bar.date {
            return Err(market_error("daily bars must be strictly newest first"));
        }
        if !bar.settled {
            return Err(market_error(format!(
                "daily bar {} is not settled",
                bar.date
            )));
        }
        if !bar.close.is_finite() || bar.close <= 0.0 {
            return Err(market_error(format!(
                "daily close on {} must be finite and positive",
                bar.date
            )));
        }
        if !bar.volume.is_finite() || bar.volume < 0.0 {
            return Err(market_error(format!(
                "daily volume on {} must be finite and non-negative",
                bar.date
            )));
        }
    }

    let average_close = |count: usize| -> f64 {
        bars.iter().take(count).map(|bar| bar.close).sum::<f64>() / count as f64
    };
    let average_volume = |count: usize| -> f64 {
        bars.iter().take(count).map(|bar| bar.volume).sum::<f64>() / count as f64
    };
    let latest_close = bars[0].close;
    let ma5 = average_close(5);
    Ok(NewsMarketMetrics {
        latest_close,
        ma5,
        ma10: average_close(10),
        ma20: average_close(20),
        return_5d_percent: (latest_close / bars[4].close - 1.0) * 100.0,
        bias_5_percent: (latest_close / ma5 - 1.0) * 100.0,
        latest_volume: bars[0].volume,
        average_volume_5d: average_volume(5),
        average_volume_20d: average_volume(20),
    })
}

fn validate_quote(
    target_code: &str,
    quote: &RealtimeQuoteInput,
    as_of: DateTime<Utc>,
) -> Result<(), NewsAiError> {
    validate_batch_evidence(&quote.evidence).map_err(|error| market_error(error.to_string()))?;
    if !codes_equal(&quote.code, target_code) {
        return Err(market_error(format!(
            "quote code {} differs from {target_code}",
            quote.code
        )));
    }
    if !quote.price.is_finite() || quote.price <= 0.0 {
        return Err(market_error("quote price must be finite and positive"));
    }
    let batch_observed_at = parse_observed_at(&quote.evidence.observed_at)
        .map_err(|error| market_error(error.to_string()))?;
    if quote.observed_at != batch_observed_at {
        return Err(market_error(
            "quote observation differs from batch observation",
        ));
    }
    let batch_source_at = quote
        .evidence
        .source_at
        .as_deref()
        .ok_or_else(|| market_error("quote batch source time is missing"))?;
    if DateTime::parse_from_rfc3339(batch_source_at)
        .map(|value| value.with_timezone(&Utc) != quote.source_at)
        .unwrap_or(true)
    {
        return Err(market_error(
            "quote source time differs from batch source time",
        ));
    }
    if quote.source_at
        > quote.observed_at + chrono::Duration::seconds(DATA_EVIDENCE_FUTURE_TOLERANCE_SECONDS)
        || quote.observed_at
            > as_of + chrono::Duration::seconds(DATA_EVIDENCE_FUTURE_TOLERANCE_SECONDS)
    {
        return Err(market_error("quote timestamps are out of order"));
    }
    if as_of.signed_duration_since(quote.source_at)
        > chrono::Duration::seconds(REALTIME_MAX_AGE_SECONDS)
    {
        return Err(NewsAiError::StaleQuote(format!(
            "quote age exceeds {REALTIME_MAX_AGE_SECONDS} seconds"
        )));
    }
    Ok(())
}

fn validate_optional_metrics(
    metrics: &mut [OptionalNewsMetric],
    as_of: DateTime<Utc>,
) -> Result<(), NewsAiError> {
    let mut names = HashSet::with_capacity(metrics.len());
    for metric in metrics.iter() {
        let name = metric.name.trim();
        if name.is_empty() || name != metric.name || !names.insert(name.to_owned()) {
            return Err(market_error(
                "optional metric names must be non-empty and unique",
            ));
        }
        match &metric.status {
            EvidenceStatus::Available { value, evidence } => {
                if !value.is_finite() {
                    return Err(market_error(format!(
                        "optional metric {name} is not finite"
                    )));
                }
                if evidence.batch_id.trim().is_empty()
                    || evidence.observed_at
                        > as_of + chrono::Duration::seconds(DATA_EVIDENCE_FUTURE_TOLERANCE_SECONDS)
                {
                    return Err(market_error(format!(
                        "optional metric {name} has invalid evidence"
                    )));
                }
            }
            EvidenceStatus::Unavailable { reason } if reason.trim().is_empty() => {
                return Err(market_error(format!(
                    "optional metric {name} has an empty unavailable reason"
                )));
            }
            EvidenceStatus::Unavailable { .. } => {}
        }
    }
    metrics.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(())
}

fn build_normalized_prompt(
    fact: &AdmittedNewsFact,
    market: &NewsMarketSnapshot,
    metrics: &[OptionalNewsMetric],
    analysis_version: &str,
) -> Result<String, NewsAiError> {
    let mut available = BTreeMap::new();
    let mut unavailable = BTreeMap::new();
    for metric in metrics {
        match &metric.status {
            EvidenceStatus::Available { value, .. } => {
                available.insert(metric.name.as_str(), *value);
            }
            EvidenceStatus::Unavailable { reason } => {
                unavailable.insert(metric.name.as_str(), reason.as_str());
            }
        }
    }
    let quote = market.quote.as_ref().map(|quote| {
        serde_json::json!({
            "price": quote.price,
            "provider": provider_tag(quote.evidence.provider),
            "batch_id": quote.evidence.batch_id,
            "source_at": quote.source_at.to_rfc3339(),
        })
    });
    serde_json::to_string(&serde_json::json!({
        "analysis_version": analysis_version,
        "target_code": fact.target_code(),
        "news": {
            "provider": provider_tag(fact.provider()),
            "source": fact.source(),
            "batch_id": fact.source_batch_id(),
            "item_id": fact.item_id(),
            "published_at": fact.published_at().to_rfc3339(),
            "title": fact.title(),
            "summary": fact.summary(),
            "content": fact.content(),
        },
        "market": {
            "context": match market.context {
                NewsMarketContext::Intraday => "intraday",
                NewsMarketContext::PostClose => "post_close",
            },
            "daily_provider": provider_tag(market.daily_evidence.provider),
            "daily_batch_id": market.daily_evidence.batch_id,
            "latest_close": market.metrics.latest_close,
            "ma5": market.metrics.ma5,
            "ma10": market.metrics.ma10,
            "ma20": market.metrics.ma20,
            "return_5d_percent": market.metrics.return_5d_percent,
            "bias_5_percent": market.metrics.bias_5_percent,
            "latest_volume": market.metrics.latest_volume,
            "average_volume_5d": market.metrics.average_volume_5d,
            "average_volume_20d": market.metrics.average_volume_20d,
            "quote": quote,
        },
        "available_optional_metrics": available,
        "unavailable_optional_metrics": unavailable,
        "required_output_schema": {
            "impact": [
                "major_negative",
                "negative",
                "neutral",
                "positive",
                "major_positive"
            ],
            "confidence": "integer 0..=100",
            "uncertainty": "non-empty string",
            "core_logic": "non-empty string"
        }
    }))
    .map_err(|error| market_error(format!("prompt serialization failed: {error}")))
}

fn hash_request_evidence(
    fact: &AdmittedNewsFact,
    market: &NewsMarketSnapshot,
    metrics: &[OptionalNewsMetric],
    analysis_version: &str,
) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, fact.target_code());
    hash_field(&mut hasher, analysis_version);
    hash_batch_evidence(&mut hasher, &fact.batch);
    match &fact.record {
        AdmittedNewsSourceRecord::Global(record) => {
            hash_field(&mut hasher, "global");
            for value in [
                record.item_id.as_str(),
                record.title.as_str(),
                record.summary.as_deref().unwrap_or(""),
                record.content.as_deref().unwrap_or(""),
                record.publisher.as_str(),
                record.canonical_url.as_str(),
                record.language.as_str(),
            ] {
                hash_field(&mut hasher, value);
            }
            hash_field(&mut hasher, &record.published_at.to_rfc3339());
            hash_field(&mut hasher, &record.observed_at.to_rfc3339());
            for instrument in &record.instruments {
                hash_field(&mut hasher, instrument);
            }
            for topic in &record.topics {
                hash_field(&mut hasher, topic);
            }
            hash_source_evidence(&mut hasher, &record.evidence);
        }
        AdmittedNewsSourceRecord::Sina(record) => {
            hash_field(&mut hasher, "sina_instrument");
            let item = record.persistence_item();
            for value in [
                item.source.as_str(),
                item.external_id.as_str(),
                item.category.as_str(),
                item.code.as_deref().unwrap_or(""),
                item.title.as_str(),
                item.summary.as_str(),
                item.url.as_str(),
                item.source_name.as_str(),
                item.content_hash.as_str(),
            ] {
                hash_field(&mut hasher, value);
            }
            hash_field(&mut hasher, &item.published_at.to_rfc3339());
            hash_field(&mut hasher, &item.fetched_at.to_rfc3339());
            hash_source_evidence(&mut hasher, record.evidence());
        }
    }
    hash_field(
        &mut hasher,
        match market.context {
            NewsMarketContext::Intraday => "intraday",
            NewsMarketContext::PostClose => "post_close",
        },
    );
    hash_field(&mut hasher, market.target_code());
    hash_field(&mut hasher, &market.as_of.to_rfc3339());
    hash_batch_evidence(&mut hasher, &market.daily_evidence);
    for bar in &market.daily_bars {
        hash_field(&mut hasher, &bar.date.to_string());
        hash_field(&mut hasher, &bar.close.to_bits().to_string());
        hash_field(&mut hasher, &bar.volume.to_bits().to_string());
        hash_field(&mut hasher, if bar.settled { "settled" } else { "open" });
    }
    for value in [
        market.metrics.latest_close,
        market.metrics.ma5,
        market.metrics.ma10,
        market.metrics.ma20,
        market.metrics.return_5d_percent,
        market.metrics.bias_5_percent,
        market.metrics.latest_volume,
        market.metrics.average_volume_5d,
        market.metrics.average_volume_20d,
    ] {
        hash_field(&mut hasher, &value.to_bits().to_string());
    }
    if let Some(quote) = &market.quote {
        hash_field(&mut hasher, &quote.code);
        hash_field(&mut hasher, &quote.source_at.to_rfc3339());
        hash_field(&mut hasher, &quote.observed_at.to_rfc3339());
        hash_field(&mut hasher, &quote.price.to_bits().to_string());
        hash_batch_evidence(&mut hasher, &quote.evidence);
    } else {
        hash_field(&mut hasher, "quote_absent");
    }
    for metric in metrics {
        hash_field(&mut hasher, &metric.name);
        match &metric.status {
            EvidenceStatus::Available { value, evidence } => {
                hash_field(&mut hasher, "available");
                hash_field(&mut hasher, &value.to_bits().to_string());
                hash_field(&mut hasher, provider_tag(evidence.provider));
                hash_field(&mut hasher, &evidence.batch_id);
                hash_field(&mut hasher, &evidence.observed_at.to_rfc3339());
            }
            EvidenceStatus::Unavailable { reason } => {
                hash_field(&mut hasher, "unavailable");
                hash_field(&mut hasher, reason);
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

fn assessment_identity(request: &NewsAiRequest) -> String {
    assessment_identity_for_fact(request.fact(), request.analysis_version())
}

fn assessment_identity_for_fact(fact: &AdmittedNewsFact, analysis_version: &str) -> String {
    let mut hasher = Sha256::new();
    for value in [
        provider_tag(fact.provider()),
        fact.source_batch_id(),
        fact.item_id(),
        fact.target_code(),
        analysis_version,
    ] {
        hash_field(&mut hasher, value);
    }
    format!("{:x}", hasher.finalize())
}

fn hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn hash_batch_evidence(hasher: &mut Sha256, evidence: &BatchEvidence) {
    hash_field(hasher, provider_tag(evidence.provider));
    hash_field(hasher, &evidence.source);
    hash_field(hasher, evidence.source_at.as_deref().unwrap_or(""));
    hash_field(hasher, &evidence.observed_at);
    hash_field(hasher, &evidence.batch_id);
}

fn hash_source_evidence(hasher: &mut Sha256, evidence: &SourceEvidence) {
    hash_field(hasher, provider_tag(evidence.provider()));
    hash_field(hasher, evidence.source_at().unwrap_or(""));
    hash_field(hasher, evidence.observed_at());
    hash_field(hasher, evidence.batch_id());
}

fn sha256_hex(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

const fn provider_tag(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Tdx => "tdx",
        ProviderId::Tencent => "tencent",
        ProviderId::Eastmoney => "eastmoney",
        ProviderId::Sina => "sina",
        ProviderId::Baostock => "baostock",
        ProviderId::Baidu => "baidu",
        ProviderId::Tonghuashun => "tonghuashun",
        ProviderId::Iwencai => "iwencai",
        ProviderId::Cninfo => "cninfo",
        ProviderId::Cailianpress => "cailianpress",
        ProviderId::Jin10 => "jin10",
        ProviderId::ThePaper => "thepaper",
        ProviderId::Yonhap => "yonhap",
        ProviderId::WallstreetCn => "wallstreet_cn",
        ProviderId::Sse => "sse",
        ProviderId::Szse => "szse",
        ProviderId::Hkex => "hkex",
        ProviderId::Cffex => "cffex",
        ProviderId::StateCouncil => "state_council",
        ProviderId::Nbs => "nbs",
        ProviderId::Pbc => "pbc",
        ProviderId::Cfets => "cfets",
        ProviderId::Fred => "fred",
        ProviderId::Imf => "imf",
        ProviderId::WorldBank => "world_bank",
        ProviderId::SecEdgar => "sec_edgar",
        ProviderId::XinhuaFinance => "xinhua_finance",
        ProviderId::Yicai => "yicai",
        ProviderId::SecuritiesTimes => "securities_times",
        ProviderId::LocalAnalysis => "local_analysis",
        ProviderId::LocalTerminal => "local_terminal",
        ProviderId::Custom => "custom",
    }
}

const fn expected_global_source(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Eastmoney => Some("eastmoney-web"),
        ProviderId::Cailianpress => Some("cls-v1"),
        ProviderId::Jin10 => Some("jin10-flash-v1"),
        ProviderId::ThePaper => Some("thepaper-finance-v1"),
        _ => None,
    }
}

fn market_error(message: impl Into<String>) -> NewsAiError {
    NewsAiError::MarketEvidenceMismatch(message.into())
}

fn nonempty_optional(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
mod tests {
    use super::*;
    use crate::data_gateway::historical_bars::AdmittedDailyBars;
    use crate::data_gateway::market_data::RealtimeMarketQuote;
    use crate::data_provider::{AdjustType, KlineData};
    use crate::magic_compat::SourceEvidence;
    use async_trait::async_trait;
    use serde_json::Value;

    #[test]
    fn provider_tags_cover_all_newly_pinned_source_identities() {
        let cases = [
            (ProviderId::Nbs, "nbs"),
            (ProviderId::Pbc, "pbc"),
            (ProviderId::Cfets, "cfets"),
            (ProviderId::Fred, "fred"),
            (ProviderId::Imf, "imf"),
            (ProviderId::WorldBank, "world_bank"),
            (ProviderId::SecEdgar, "sec_edgar"),
            (ProviderId::XinhuaFinance, "xinhua_finance"),
            (ProviderId::Yicai, "yicai"),
            (ProviderId::SecuritiesTimes, "securities_times"),
        ];
        for (provider, expected) in cases {
            assert_eq!(provider_tag(provider), expected);
        }
    }

    #[derive(Clone)]
    struct ReceiptProvider {
        raw_response: String,
    }

    #[async_trait]
    impl LlmProvider for ReceiptProvider {
        fn name(&self) -> &'static str {
            "TEST_CODE_model_provider"
        }

        fn model(&self) -> &str {
            "TEST_CODE_configured_model"
        }

        async fn chat_json(&self, _system: &str, _user: &str) -> Result<Value, LlmError> {
            Err(LlmError::ReceiptUnavailable {
                provider: self.name().to_owned(),
                model: self.model().to_owned(),
            })
        }

        async fn chat_json_with_receipt(
            &self,
            system: &str,
            user: &str,
        ) -> Result<ReceiptBearingJson, LlmError> {
            Ok(ReceiptBearingJson::test_fixture(
                self.name(),
                "TEST_CODE_upstream_model",
                Some("TEST_CODE_upstream_request"),
                "TEST_CODE_upstream_response",
                system,
                user,
                &self.raw_response,
                instant("2026-07-27T01:00:04Z"),
                instant("2026-07-27T01:00:05Z"),
            ))
        }
    }

    fn instant(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("TEST_CODE valid instant")
            .with_timezone(&Utc)
    }

    fn observed(value: DateTime<Utc>) -> String {
        format!(
            "{}.{:09}",
            value.timestamp(),
            value.timestamp_subsec_nanos()
        )
    }

    fn news_batch(observed_at: DateTime<Utc>) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Cailianpress,
            source: "cls-v1".to_owned(),
            source_at: Some("2026-07-27T01:00:00Z".to_owned()),
            observed_at: observed(observed_at),
            batch_id: "TEST_CODE_NEWS_BATCH".to_owned(),
        }
    }

    fn global_record(observed_at: DateTime<Utc>) -> GlobalNewsRecord {
        let published_at = instant("2026-07-27T01:00:00Z");
        GlobalNewsRecord {
            item_id: "TEST_CODE_NEWS_ITEM".to_owned(),
            title: "TEST_CODE 公司取得重大合同".to_owned(),
            summary: Some("TEST_CODE 合同金额及履约期已由来源披露".to_owned()),
            content: None,
            publisher: "财联社".to_owned(),
            canonical_url: "https://example.invalid/TEST_CODE_NEWS_ITEM".to_owned(),
            published_at,
            observed_at,
            instruments: vec!["600519".to_owned()],
            topics: vec!["合同".to_owned()],
            language: "zh-CN".to_owned(),
            evidence: SourceEvidence::new(
                ProviderId::Cailianpress,
                observed(observed_at),
                "TEST_CODE_NEWS_BATCH",
            )
            .expect("TEST_CODE evidence")
            .with_source_at(published_at.to_rfc3339())
            .expect("TEST_CODE source time"),
        }
    }

    fn daily_bars() -> Vec<SettledDailyBarInput> {
        let latest = NaiveDate::from_ymd_opt(2026, 7, 24).expect("TEST_CODE date");
        (0..20)
            .map(|index| SettledDailyBarInput {
                date: latest - chrono::Duration::days(index),
                close: 100.0 - index as f64,
                volume: 1_000_000.0 + index as f64,
                settled: true,
            })
            .collect()
    }

    fn daily_batch(observed_at: DateTime<Utc>) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_tdx-bars".to_owned(),
            source_at: Some("2026-07-24T07:00:00Z".to_owned()),
            observed_at: observed(observed_at),
            batch_id: "TEST_CODE_DAILY_BATCH".to_owned(),
        }
    }

    fn admitted_daily_bars(
        target_code: &str,
        as_of: DateTime<Utc>,
        count: usize,
        settled: bool,
    ) -> AdmittedDailyBars {
        let latest =
            crate::calendar::latest_completed_trading_day_at(china_naive_datetime(as_of).unwrap());
        admitted_daily_bars_with_latest(target_code, as_of, latest, count, settled)
    }

    fn admitted_daily_bars_with_latest(
        target_code: &str,
        as_of: DateTime<Utc>,
        latest: NaiveDate,
        count: usize,
        settled: bool,
    ) -> AdmittedDailyBars {
        let records = crate::calendar::recent_trading_days(latest, count)
            .into_iter()
            .enumerate()
            .map(|(index, date)| KlineData {
                date,
                open: 100.0 - index as f64,
                high: 101.0 - index as f64,
                low: 99.0 - index as f64,
                close: 100.0 - index as f64,
                volume: 1_000_000.0 + index as f64,
                amount: 100_000_000.0 + index as f64,
                pct_chg: 1.0,
                intraday_price: None,
                settled,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: AdjustType::None,
            })
            .collect();
        AdmittedDailyBars::from_test_fixture(target_code, records, daily_batch(as_of))
            .expect("TEST_CODE admitted daily bars")
    }

    fn admitted_realtime_quote(
        target_code: &str,
        source_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
    ) -> AdmittedRealtimeQuote {
        let batch_id = "TEST_CODE_NEWS_AI_QUOTE_BATCH";
        let evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_tencent-quote".to_owned(),
            source_at: Some(source_at.to_rfc3339()),
            observed_at: observed_at.to_rfc3339(),
            batch_id: batch_id.to_owned(),
        };
        AdmittedRealtimeQuote::from_test_fixture(
            RealtimeMarketQuote {
                code: target_code.to_owned(),
                name: "TEST_CODE quote".to_owned(),
                price: 101.0,
                previous_close: 100.0,
                change_percent: 1.0,
                source_at,
                observed_at,
                provider: ProviderId::Tencent,
                batch_id: batch_id.to_owned(),
            },
            evidence,
        )
        .expect("TEST_CODE admitted realtime quote")
    }

    #[test]
    fn br172_production_market_snapshot_derives_latest_completed_trading_day() {
        let as_of = instant("2026-07-27T01:00:03Z");
        let snapshot = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::PostClose,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, true),
            None,
        )
        .expect("TEST_CODE sealed daily evidence must create a snapshot");
        assert_eq!(snapshot.metrics().latest_close, 100.0);
        assert_eq!(snapshot.metrics().ma20, 90.5);
    }

    #[test]
    fn br172_production_market_snapshot_rejects_daily_instrument_mismatch() {
        let as_of = instant("2026-07-27T01:00:03Z");
        let error = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::PostClose,
            as_of,
            admitted_daily_bars("TEST_CODE_000001", as_of, 20, true),
            None,
        )
        .expect_err("TEST_CODE another instrument's daily bars must fail");
        assert_eq!(error.reason_code(), "market_evidence_mismatch");
    }

    #[test]
    fn br172_production_intraday_snapshot_requires_sealed_quote() {
        let as_of = instant("2026-07-27T01:30:03Z");
        let error = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::Intraday,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, true),
            None,
        )
        .expect_err("TEST_CODE intraday evidence without a sealed quote must fail");
        assert_eq!(error.reason_code(), "market_evidence_mismatch");
    }

    #[test]
    fn br172_production_post_close_snapshot_rejects_realtime_quote() {
        let as_of = instant("2026-07-27T08:00:03Z");
        let error = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::PostClose,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, true),
            Some(admitted_realtime_quote(
                "TEST_CODE_600519",
                as_of - chrono::Duration::seconds(1),
                as_of,
            )),
        )
        .expect_err("TEST_CODE post-close evidence must not contain a realtime quote");
        assert_eq!(error.reason_code(), "market_evidence_mismatch");
    }

    #[test]
    fn br172_production_intraday_snapshot_rejects_quote_instrument_mismatch() {
        let as_of = instant("2026-07-27T01:30:03Z");
        let error = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::Intraday,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, true),
            Some(admitted_realtime_quote(
                "TEST_CODE_000001",
                as_of - chrono::Duration::seconds(1),
                as_of,
            )),
        )
        .expect_err("TEST_CODE another instrument's quote must fail");
        assert_eq!(error.reason_code(), "market_evidence_mismatch");
    }

    #[test]
    fn br172_production_intraday_snapshot_enforces_five_second_quote_freshness() {
        let as_of = instant("2026-07-27T01:30:10Z");
        let error = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::Intraday,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, true),
            Some(admitted_realtime_quote(
                "TEST_CODE_600519",
                as_of - chrono::Duration::seconds(6),
                as_of - chrono::Duration::seconds(5),
            )),
        )
        .expect_err("TEST_CODE a six-second-old quote must fail");
        assert_eq!(error.reason_code(), "stale_quote");
    }

    #[test]
    fn br172_production_intraday_snapshot_accepts_fresh_sealed_evidence() {
        let as_of = instant("2026-07-27T01:30:03Z");
        let snapshot = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::Intraday,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, true),
            Some(admitted_realtime_quote(
                "TEST_CODE_600519",
                as_of - chrono::Duration::seconds(1),
                as_of,
            )),
        )
        .expect("TEST_CODE fresh sealed market evidence must pass");
        assert_eq!(snapshot.context(), NewsMarketContext::Intraday);
        assert_eq!(snapshot.metrics().latest_close, 100.0);
    }

    #[test]
    fn br172_production_market_snapshot_rejects_short_or_unsettled_daily_history() {
        let as_of = instant("2026-07-27T01:00:03Z");
        let short = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::PostClose,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 19, true),
            None,
        )
        .expect_err("TEST_CODE fewer than twenty bars must fail");
        assert_eq!(short.reason_code(), "insufficient_daily_history");

        let unsettled = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::PostClose,
            as_of,
            admitted_daily_bars("TEST_CODE_600519", as_of, 20, false),
            None,
        )
        .expect_err("TEST_CODE unsettled daily bars must fail");
        assert_eq!(unsettled.reason_code(), "market_evidence_mismatch");
    }

    #[test]
    fn br172_production_market_snapshot_rejects_stale_daily_history() {
        let as_of = instant("2026-07-27T01:00:03Z");
        let expected =
            crate::calendar::latest_completed_trading_day_at(china_naive_datetime(as_of).unwrap());
        let stale_latest = crate::calendar::prev_trading_day(expected);
        let error = NewsMarketSnapshot::try_from_admitted(
            "TEST_CODE_600519",
            NewsMarketContext::PostClose,
            as_of,
            admitted_daily_bars_with_latest("TEST_CODE_600519", as_of, stale_latest, 20, true),
            None,
        )
        .expect_err("TEST_CODE lagging daily bars must fail");
        assert_eq!(error.reason_code(), "stale_daily_bars");
    }

    fn post_close_snapshot(as_of: DateTime<Utc>) -> NewsMarketSnapshot {
        NewsMarketSnapshot::try_from_input(NewsMarketEvidenceInput {
            target_code: "TEST_CODE_600519".to_owned(),
            context: NewsMarketContext::PostClose,
            as_of,
            latest_completed_trading_day: NaiveDate::from_ymd_opt(2026, 7, 24)
                .expect("TEST_CODE date"),
            daily_bars: daily_bars(),
            daily_evidence: daily_batch(as_of),
            quote: None,
        })
        .expect("TEST_CODE post-close snapshot")
    }

    fn request() -> NewsAiRequest {
        let as_of = instant("2026-07-27T01:00:03Z");
        let fact = AdmittedNewsFact::from_global(
            &global_record(as_of),
            &news_batch(as_of),
            "TEST_CODE_600519",
        )
        .expect("TEST_CODE admitted fact");
        NewsAiRequest::try_new(
            fact,
            post_close_snapshot(as_of),
            vec![OptionalNewsMetric {
                name: "pe_ratio".to_owned(),
                status: EvidenceStatus::Unavailable {
                    reason: "TEST_CODE equivalent typed field unavailable".to_owned(),
                },
            }],
            "TEST_CODE_news_ai_v1",
        )
        .expect("TEST_CODE request")
    }

    #[test]
    fn br172_global_fact_binds_record_batch_and_explicit_instrument() {
        let observed_at = instant("2026-07-27T01:00:03Z");
        let fact = AdmittedNewsFact::from_global(
            &global_record(observed_at),
            &news_batch(observed_at),
            "TEST_CODE_600519",
        )
        .expect("exact evidence must be admitted");
        assert_eq!(fact.item_id(), "TEST_CODE_NEWS_ITEM");
        assert_eq!(fact.source_batch_id(), "TEST_CODE_NEWS_BATCH");
        assert_eq!(fact.target_code(), "TEST_CODE_600519");
    }

    #[test]
    fn br172_global_fact_rejects_record_batch_mismatch() {
        let observed_at = instant("2026-07-27T01:00:03Z");
        let mut batch = news_batch(observed_at);
        batch.batch_id = "TEST_CODE_OTHER_BATCH".to_owned();
        let error =
            AdmittedNewsFact::from_global(&global_record(observed_at), &batch, "TEST_CODE_600519")
                .expect_err("mismatched batch must fail");
        assert_eq!(error.reason_code(), "news_evidence_mismatch");
    }

    #[test]
    fn br172_global_fact_rejects_inferred_instrument() {
        let observed_at = instant("2026-07-27T01:00:03Z");
        let error = AdmittedNewsFact::from_global(
            &global_record(observed_at),
            &news_batch(observed_at),
            "TEST_CODE_000001",
        )
        .expect_err("title inference is not source binding");
        assert_eq!(error.reason_code(), "instrument_not_source_bound");
    }

    #[test]
    fn br172_intraday_snapshot_rejects_quote_older_than_five_seconds() {
        let as_of = instant("2026-07-27T01:00:10Z");
        let quote_source_at = instant("2026-07-27T01:00:04Z");
        let quote_observed_at = instant("2026-07-27T01:00:05Z");
        let error = NewsMarketSnapshot::try_from_input(NewsMarketEvidenceInput {
            target_code: "TEST_CODE_600519".to_owned(),
            context: NewsMarketContext::Intraday,
            as_of,
            latest_completed_trading_day: NaiveDate::from_ymd_opt(2026, 7, 24)
                .expect("TEST_CODE date"),
            daily_bars: daily_bars(),
            daily_evidence: daily_batch(as_of),
            quote: Some(RealtimeQuoteInput {
                code: "TEST_CODE_600519".to_owned(),
                price: 101.0,
                source_at: quote_source_at,
                observed_at: quote_observed_at,
                evidence: BatchEvidence {
                    provider: ProviderId::Tencent,
                    source: "TEST_CODE_tencent-quote".to_owned(),
                    source_at: Some(quote_source_at.to_rfc3339()),
                    observed_at: observed(quote_observed_at),
                    batch_id: "TEST_CODE_QUOTE_BATCH".to_owned(),
                },
            }),
        })
        .expect_err("stale quote must fail");
        assert_eq!(error.reason_code(), "stale_quote");
    }

    #[test]
    fn br172_market_snapshot_requires_ma20_history() {
        let as_of = instant("2026-07-27T01:00:03Z");
        let mut bars = daily_bars();
        bars.truncate(19);
        let error = NewsMarketSnapshot::try_from_input(NewsMarketEvidenceInput {
            target_code: "TEST_CODE_600519".to_owned(),
            context: NewsMarketContext::PostClose,
            as_of,
            latest_completed_trading_day: NaiveDate::from_ymd_opt(2026, 7, 24)
                .expect("TEST_CODE date"),
            daily_bars: bars,
            daily_evidence: daily_batch(as_of),
            quote: None,
        })
        .expect_err("MA20 requires 20 settled bars");
        assert_eq!(error.reason_code(), "insufficient_daily_history");
    }

    #[test]
    fn br172_optional_unavailable_metric_is_not_filled_with_zero() {
        let request = request();
        assert!(!request.normalized_prompt().contains(r#""pe_ratio":0"#));
        assert!(request
            .normalized_prompt()
            .contains("equivalent typed field unavailable"));
    }

    #[test]
    fn br172_strict_schema_rejects_unknown_impact() {
        let response = r#"{
            "impact":"watch",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对订单落地",
            "core_logic":"TEST_CODE 公告可能影响未来收入"
        }"#;
        let error = parse_strict_model_output(response).expect_err("unknown impact must fail");
        assert_eq!(error.reason_code(), "invalid_model_schema");
    }

    #[test]
    fn br172_strict_schema_rejects_out_of_range_confidence() {
        let response = r#"{
            "impact":"positive",
            "confidence":101,
            "uncertainty":"TEST_CODE 尚需核对订单落地",
            "core_logic":"TEST_CODE 公告可能影响未来收入"
        }"#;
        let error =
            parse_strict_model_output(response).expect_err("confidence above 100 must fail");
        assert_eq!(error.reason_code(), "invalid_model_schema");
    }

    #[test]
    fn br172_strict_schema_rejects_blank_required_reasoning() {
        let response = r#"{
            "impact":"negative",
            "confidence":70,
            "uncertainty":" ",
            "core_logic":"TEST_CODE 公告可能影响未来收入"
        }"#;
        let error = parse_strict_model_output(response).expect_err("blank uncertainty must fail");
        assert_eq!(error.reason_code(), "invalid_model_schema");
    }

    #[test]
    fn br172_strict_schema_rejects_unknown_fields_and_trailing_content() {
        let unknown = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入",
            "action":"buy"
        }"#;
        let trailing = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        } trailing"#;
        assert!(parse_strict_model_output(unknown).is_err());
        assert!(parse_strict_model_output(trailing).is_err());
    }

    #[test]
    fn br172_strict_schema_rejects_duplicate_fields() {
        let duplicate = r#"{
            "impact":"positive",
            "impact":"negative",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        }"#;
        assert!(parse_strict_model_output(duplicate).is_err());
    }

    #[test]
    fn parse_source_at_accepts_unix_epoch_seconds_and_millis() {
        // 财联社(Cailianpress)publication_time 为 unix 秒时间戳(10 位)。
        let seconds = DateTime::from_timestamp(1787276172, 0).expect("seconds in range");
        assert_eq!(
            parse_source_at(ProviderId::Cailianpress, "1787276172").expect("unix seconds"),
            seconds
        );
        let millis = DateTime::from_timestamp(1787276172, 123_000_000).expect("millis in range");
        assert_eq!(
            parse_source_at(ProviderId::Cailianpress, "1787276172123").expect("unix millis"),
            millis
        );
    }

    #[test]
    fn parse_source_at_keeps_rfc3339_and_eastmoney() {
        assert_eq!(
            parse_source_at(ProviderId::Jin10, "2026-08-21T09:36:12+08:00").expect("rfc3339"),
            DateTime::parse_from_rfc3339("2026-08-21T09:36:12+08:00")
                .expect("rfc3339 valid")
                .with_timezone(&Utc)
        );
        assert_eq!(
            parse_source_at(ProviderId::Eastmoney, "2026-08-21 09:36").expect("eastmoney"),
            Utc.from_utc_datetime(
                &NaiveDateTime::parse_from_str("2026-08-21 09:36", "%Y-%m-%d %H:%M")
                    .expect("naive valid")
            ) - chrono::Duration::hours(8)
        );
    }

    #[test]
    fn parse_observed_at_accepts_unix_epoch_seconds() {
        let seconds = DateTime::from_timestamp(1787276172, 0).expect("seconds in range");
        assert_eq!(
            parse_observed_at("1787276172").expect("unix seconds"),
            seconds
        );
        assert!(parse_observed_at("not-a-timestamp").is_err());
    }

    #[test]
    fn br172_assessment_requires_actual_model_receipt() {
        let error = NewsAiAssessment::from_model_response(
            &request(),
            r#"{
                "impact":"positive",
                "confidence":70,
                "uncertainty":"TEST_CODE 尚需核对",
                "core_logic":"TEST_CODE 合同可能提升收入"
            }"#,
            None,
        )
        .expect_err("missing model receipt must fail");
        assert_eq!(error.reason_code(), "model_receipt_missing");
    }

    #[test]
    fn br172_assessment_binds_prompt_and_response_to_receipt() {
        let request = request();
        let response = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        }"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_model_provider",
            "TEST_CODE_model",
            Some("TEST_CODE_request_id"),
            request.normalized_prompt(),
            response,
            instant("2026-07-27T01:00:04Z"),
            instant("2026-07-27T01:00:05Z"),
        )
        .expect("TEST_CODE receipt");
        let assessment = NewsAiAssessment::from_model_response(&request, response, Some(receipt))
            .expect("receipt-bound response");
        assert_eq!(assessment.impact(), NewsImpact::Positive);
        assert_eq!(assessment.confidence(), 70);
        assert_eq!(assessment.receipt().provider(), "TEST_CODE_model_provider");
        assert_eq!(
            assessment.receipt().upstream_response_id(),
            "TEST_CODE_MODEL_RESPONSE"
        );
        assert_eq!(
            assessment.receipt().system_sha256(),
            sha256_hex(NEWS_AI_SYSTEM_PROMPT_V1.as_bytes())
        );
        assert_eq!(
            assessment.receipt().user_sha256(),
            sha256_hex(request.normalized_prompt().as_bytes())
        );
    }

    #[tokio::test]
    async fn br172_analyzer_uses_one_receipt_bearing_provider_call() {
        let request = request();
        let response = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        }"#;
        let analyzer = NewsAIAnalyzer::new(Arc::new(ReceiptProvider {
            raw_response: response.to_owned(),
        }));

        let assessment = analyzer
            .assess(&request)
            .await
            .expect("receipt-bearing provider response");

        assert_eq!(assessment.impact(), NewsImpact::Positive);
        assert_eq!(assessment.receipt().model(), "TEST_CODE_upstream_model");
        assert_eq!(
            assessment.receipt().upstream_request_id(),
            Some("TEST_CODE_upstream_request")
        );
        assert_eq!(
            assessment.receipt().upstream_response_id(),
            "TEST_CODE_upstream_response"
        );
    }

    #[derive(Default)]
    struct RecordingGovernedDeliveryPort {
        actions: std::sync::Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl NewsAiGovernedDeliveryPort for RecordingGovernedDeliveryPort {
        async fn reserve(
            &self,
            delivery: &GovernedNewsAiDelivery,
        ) -> Result<NewsAiReserveOutcome, String> {
            self.actions.lock().unwrap().push("reserve");
            Ok(NewsAiReserveOutcome::Reserved(
                NewsAiDeliveryReservation::try_new(
                    delivery.identity().sha256(),
                    "TEST_CODE_RESERVATION_001",
                )
                .unwrap(),
            ))
        }

        async fn push(
            &self,
            delivery: &GovernedNewsAiDelivery,
            _reservation: &NewsAiDeliveryReservation,
        ) -> NewsAiPhysicalPushOutcome {
            self.actions.lock().unwrap().push("push");
            NewsAiPhysicalPushOutcome::Pushed(
                NewsAiDeliveryAuditReceipt::try_new(delivery.identity().sha256(), &"b".repeat(64))
                    .unwrap(),
            )
        }

        async fn commit(
            &self,
            delivery: &GovernedNewsAiDelivery,
            _reservation: &NewsAiDeliveryReservation,
            delivery_audit: &NewsAiDeliveryAuditReceipt,
        ) -> Result<NewsAiPredictionLinkReceipt, String> {
            self.actions.lock().unwrap().push("commit");
            NewsAiPredictionLinkReceipt::try_new(
                delivery.identity().sha256(),
                delivery.assessment().assessment_id(),
                delivery_audit.audit_event_id(),
                &"c".repeat(64),
            )
            .map_err(|error| error.to_string())
        }

        async fn rollback(
            &self,
            _delivery: &GovernedNewsAiDelivery,
            _reservation: &NewsAiDeliveryReservation,
        ) -> Result<(), String> {
            self.actions.lock().unwrap().push("rollback");
            Ok(())
        }
    }

    #[tokio::test]
    async fn br172_governed_delivery_commits_only_after_audited_push() {
        let request = request();
        let response = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        }"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_model_provider",
            "TEST_CODE_model",
            Some("TEST_CODE_request_id"),
            request.normalized_prompt(),
            response,
            instant("2026-07-27T01:00:04Z"),
            instant("2026-07-27T01:00:05Z"),
        )
        .unwrap();
        let assessment =
            NewsAiAssessment::from_model_response(&request, response, Some(receipt)).unwrap();
        let audited = AuditedNewsAiAssessment::try_from_assessment_audit(
            request,
            assessment,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let port = RecordingGovernedDeliveryPort::default();

        let outcome = deliver_governed_news_ai(&audited, &port).await;

        assert_eq!(
            outcome,
            NewsAiGovernedDeliveryOutcome::Pushed {
                delivery_identity_sha256: audited.delivery().identity().sha256().to_owned(),
                delivery_audit_event_id: "b".repeat(64),
                prediction_link_id: "c".repeat(64),
            }
        );
        assert_eq!(
            *port.actions.lock().unwrap(),
            vec!["reserve", "push", "commit"]
        );
    }

    #[derive(Default)]
    struct LinkRecoveryPort {
        reserve_calls: std::sync::atomic::AtomicUsize,
        push_calls: std::sync::atomic::AtomicUsize,
        commit_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl NewsAiGovernedDeliveryPort for LinkRecoveryPort {
        async fn reserve(
            &self,
            delivery: &GovernedNewsAiDelivery,
        ) -> Result<NewsAiReserveOutcome, String> {
            let call = self
                .reserve_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let reservation = NewsAiDeliveryReservation::try_new(
                delivery.identity().sha256(),
                "TEST_CODE_LINK_RECOVERY_RESERVATION",
            )
            .unwrap();
            if call == 0 {
                Ok(NewsAiReserveOutcome::Reserved(reservation))
            } else {
                let audit = NewsAiDeliveryAuditReceipt::try_new(
                    delivery.identity().sha256(),
                    "TEST_CODE_PERSISTED_ENVELOPE_ID",
                )
                .unwrap();
                Ok(NewsAiReserveOutcome::LinkPending(
                    NewsAiDeliveryLinkRecovery::try_new(reservation, audit).unwrap(),
                ))
            }
        }

        async fn push(
            &self,
            delivery: &GovernedNewsAiDelivery,
            _reservation: &NewsAiDeliveryReservation,
        ) -> NewsAiPhysicalPushOutcome {
            self.push_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            NewsAiPhysicalPushOutcome::Pushed(
                NewsAiDeliveryAuditReceipt::try_new(
                    delivery.identity().sha256(),
                    "TEST_CODE_PERSISTED_ENVELOPE_ID",
                )
                .unwrap(),
            )
        }

        async fn commit(
            &self,
            delivery: &GovernedNewsAiDelivery,
            _reservation: &NewsAiDeliveryReservation,
            delivery_audit: &NewsAiDeliveryAuditReceipt,
        ) -> Result<NewsAiPredictionLinkReceipt, String> {
            let call = self
                .commit_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                return Err("TEST_CODE_LINK_STORE_UNAVAILABLE".to_owned());
            }
            NewsAiPredictionLinkReceipt::try_new(
                delivery.identity().sha256(),
                delivery.assessment().assessment_id(),
                delivery_audit.audit_event_id(),
                &"c".repeat(64),
            )
            .map_err(|error| error.to_string())
        }

        async fn rollback(
            &self,
            _delivery: &GovernedNewsAiDelivery,
            _reservation: &NewsAiDeliveryReservation,
        ) -> Result<(), String> {
            Err("TEST_CODE_LINK_RECOVERY_MUST_NOT_ROLLBACK".to_owned())
        }
    }

    #[tokio::test]
    async fn br172_delivered_retry_links_prediction_without_calling_sink_again() {
        let request = request();
        let response = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        }"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_model_provider",
            "TEST_CODE_model",
            Some("TEST_CODE_request_id"),
            request.normalized_prompt(),
            response,
            instant("2026-07-27T01:00:04Z"),
            instant("2026-07-27T01:00:05Z"),
        )
        .unwrap();
        let assessment =
            NewsAiAssessment::from_model_response(&request, response, Some(receipt)).unwrap();
        let audited = AuditedNewsAiAssessment::try_from_assessment_audit(
            request,
            assessment,
            &"a".repeat(64),
        )
        .unwrap();
        let port = LinkRecoveryPort::default();

        assert!(matches!(
            deliver_governed_news_ai(&audited, &port).await,
            NewsAiGovernedDeliveryOutcome::PostSinkCommitFailed { .. }
        ));
        assert!(matches!(
            deliver_governed_news_ai(&audited, &port).await,
            NewsAiGovernedDeliveryOutcome::PredictionLinkRecovered { .. }
        ));
        assert_eq!(port.push_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            port.commit_calls.load(std::sync::atomic::Ordering::SeqCst),
            2
        );
    }

    #[test]
    fn br172_delivery_card_contains_only_bound_model_and_audit_evidence() {
        let request = request();
        let response = r#"{
            "impact":"positive",
            "confidence":70,
            "uncertainty":"TEST_CODE 尚需核对",
            "core_logic":"TEST_CODE 合同可能提升收入"
        }"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_model_provider",
            "TEST_CODE_model",
            Some("TEST_CODE_request_id"),
            request.normalized_prompt(),
            response,
            instant("2026-07-27T01:00:04Z"),
            instant("2026-07-27T01:00:05Z"),
        )
        .unwrap();
        let assessment =
            NewsAiAssessment::from_model_response(&request, response, Some(receipt)).unwrap();
        let audited = AuditedNewsAiAssessment::try_from_assessment_audit(
            request,
            assessment,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();

        let card = audited.delivery().render_card();
        assert!(card.contains("TEST_CODE 合同可能提升收入"));
        assert!(card.contains("TEST_CODE_model_provider / TEST_CODE_model"));
        assert!(card.contains(audited.delivery().identity().sha256()));
        assert!(card.contains("不构成交易建议"));
        assert!(!card.contains("建议买入"));
    }

    #[test]
    #[allow(deprecated)]
    fn br172_legacy_quick_decision_never_falls_back_to_keywords() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("TEST_CODE runtime");
        let analyzer = NewsAIAnalyzer::new(Arc::new(ReceiptProvider {
            raw_response: "{}".to_owned(),
        }));
        let error = runtime
            .block_on(analyzer.quick_decision(
                "TEST_CODE 收到中标通知",
                "TEST_CODE_600519",
                "TEST_CODE 公司",
            ))
            .expect_err("legacy interface must fail explicitly");
        assert_eq!(error.reason_code(), "model_unavailable");
    }
}
