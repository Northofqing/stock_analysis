//! BR-066/BR-133/BR-137/BR-166/BR-172/BR-238 evidence-preserving global financial-news acquisition.

use super::review::{acquisition_request_hash, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};

use crate::market_domain::{ProviderId, SourceEvidence};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};

/// Maximum per-request record count supported by every registered provider.
pub const MAX_GLOBAL_NEWS_LIMIT: u32 = 20;

/// One released global-news provider and its immutable source contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalNewsProvider {
    Eastmoney,
    Cailianpress,
    Jin10,
    ThePaper,
}

impl GlobalNewsProvider {
    /// Stable LocalBridgeV1 request value. This is intentionally independent
    /// from `Debug` so transport identity cannot change with diagnostics.
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Eastmoney => "Eastmoney",
            Self::Cailianpress => "Cailianpress",
            Self::Jin10 => "Jin10",
            Self::ThePaper => "ThePaper",
        }
    }

    pub fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "Eastmoney" => Some(Self::Eastmoney),
            "Cailianpress" => Some(Self::Cailianpress),
            "Jin10" => Some(Self::Jin10),
            "ThePaper" => Some(Self::ThePaper),
            _ => None,
        }
    }

    pub const fn provider_id(self) -> ProviderId {
        match self {
            Self::Eastmoney => ProviderId::Eastmoney,
            Self::Cailianpress => ProviderId::Cailianpress,
            Self::Jin10 => ProviderId::Jin10,
            Self::ThePaper => ProviderId::ThePaper,
        }
    }

    pub const fn source(self) -> &'static str {
        match self {
            Self::Eastmoney => "eastmoney-web",
            Self::Cailianpress => "cls-v1",
            Self::Jin10 => "jin10-flash-v1",
            Self::ThePaper => "thepaper-finance-v1",
        }
    }

    pub const fn feed_name(self) -> &'static str {
        match self {
            Self::Eastmoney => "eastmoney_global_news",
            Self::Cailianpress => "cls_global_news",
            Self::Jin10 => "jin10_global_news",
            Self::ThePaper => "thepaper_global_news",
        }
    }

    const fn capability(self) -> &'static str {
        match self {
            Self::Eastmoney => "GlobalNews-Eastmoney",
            Self::Cailianpress => "GlobalNews-CLS",
            Self::Jin10 => "GlobalNews-Jin10",
            Self::ThePaper => "GlobalNews-ThePaper",
        }
    }
}

/// One admitted global-news fact retaining its upstream record evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalNewsRecord {
    pub item_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub publisher: String,
    pub canonical_url: String,
    pub published_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub instruments: Vec<String>,
    pub topics: Vec<String>,
    pub language: String,
    pub evidence: SourceEvidence,
}

pub(crate) fn parse_global_news_provider_time(
    provider: GlobalNewsProvider,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    parse_provider_time(provider, value)
}

pub(crate) fn parse_global_news_observed_at(
    provider: GlobalNewsProvider,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    parse_observed_at(provider, value)
}

pub(crate) fn validate_global_news_batch_evidence(
    provider: GlobalNewsProvider,
    evidence: &BatchEvidence,
) -> Result<(DateTime<Utc>, DateTime<Utc>), GatewayError> {
    let capability = provider.capability();
    let provider_id = provider.provider_id();
    if evidence.provider != provider_id || evidence.source != provider.source() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            "global-news batch provider/source contract mismatch",
        ));
    }
    if evidence.batch_id.trim().is_empty() || evidence.observed_at.trim().is_empty() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            "global-news batch identity is incomplete",
        ));
    }
    let source_at = evidence
        .source_at
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                capability,
                Some(provider_id),
                "global-news batch source time is missing",
            )
        })?;
    let parsed_source_at = parse_global_news_provider_time(provider, source_at)?;
    let parsed_observed_at =
        parse_global_news_observed_at(provider, evidence.observed_at.as_str())?;
    if parsed_source_at > parsed_observed_at {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            "global-news batch source time is after observation time",
        ));
    }
    Ok((parsed_source_at, parsed_observed_at))
}

/// Production seam for all released typed global financial-news clients.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalNewsGateway;

impl GlobalNewsGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn global_news(
        &self,
        provider: GlobalNewsProvider,
        limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        let capability = provider.capability();
        let provider_id = provider.provider_id();
        let request_hash =
            acquisition_request_hash(capability, format!("{}:{limit}", provider.source()));
        // P4 M3 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("GlobalNews") {
            Ok(bridge) => {
                let result = bridge.global_news_async(provider, limit).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(provider_id);
                return audit_gateway_result(capability, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(capability, provider_id, &request_hash, Err(error));
            }
        }
        // no-feature (monitor 零 magic 构建): library transport 编译期不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}

fn parse_provider_time(
    provider: GlobalNewsProvider,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    let parse_shanghai = |format: &str| {
        let naive = NaiveDateTime::parse_from_str(value, format).map_err(|error| {
            GatewayError::invalid_evidence(
                provider.capability(),
                Some(provider.provider_id()),
                format!("invalid provider time {value:?}: {error}"),
            )
        })?;
        let china = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
            GatewayError::invalid_evidence(
                provider.capability(),
                Some(provider.provider_id()),
                "UTC+08:00 offset is unavailable",
            )
        })?;
        china.from_local_datetime(&naive).single().ok_or_else(|| {
            GatewayError::invalid_evidence(
                provider.capability(),
                Some(provider.provider_id()),
                format!("ambiguous provider time {value:?}"),
            )
        })
    };
    let parse_rfc3339 = || {
        DateTime::parse_from_rfc3339(value)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(|error| {
                GatewayError::invalid_evidence(
                    provider.capability(),
                    Some(provider.provider_id()),
                    format!("invalid provider time {value:?}: {error}"),
                )
            })
    };
    match provider {
        GlobalNewsProvider::Eastmoney => {
            parse_shanghai("%Y-%m-%d %H:%M").map(|timestamp| timestamp.with_timezone(&Utc))
        }
        GlobalNewsProvider::Jin10 => parse_shanghai("%Y-%m-%d %H:%M:%S")
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .or_else(|_| parse_rfc3339()),
        GlobalNewsProvider::Cailianpress => super::evidence_time::parse_evidence_instant(
            provider.capability(),
            provider.provider_id(),
            "source_at",
            value,
        ),
        GlobalNewsProvider::ThePaper if value.starts_with("unix-ms:") => {
            let milliseconds = value
                .strip_prefix("unix-ms:")
                .and_then(|raw| raw.parse::<i64>().ok())
                .ok_or_else(|| {
                    GatewayError::invalid_evidence(
                        provider.capability(),
                        Some(provider.provider_id()),
                        format!("invalid ThePaper provider time {value:?}"),
                    )
                })?;
            DateTime::from_timestamp_millis(milliseconds).ok_or_else(|| {
                GatewayError::invalid_evidence(
                    provider.capability(),
                    Some(provider.provider_id()),
                    format!("out-of-range ThePaper provider time {value:?}"),
                )
            })
        }
        GlobalNewsProvider::ThePaper => parse_rfc3339(),
    }
}

fn parse_observed_at(
    provider: GlobalNewsProvider,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    super::evidence_time::parse_evidence_instant(
        provider.capability(),
        provider.provider_id(),
        "observed_at",
        value,
    )
}
