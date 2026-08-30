//! BR-175: generic web discovery is a ResearchOnly Gateway.
//!
//! Bocha, Tavily, and SerpAPI are not financial/news fact providers. This
//! module owns their wire protocols so business modules cannot turn direct web
//! results into an implicit fallback for pinned Magic providers.

use std::collections::HashSet;
use std::fmt;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_RESULTS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralWebResearchProvider {
    Bocha,
    Tavily,
    SerpApi,
}

impl GeneralWebResearchProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bocha => "Bocha",
            Self::Tavily => "Tavily",
            Self::SerpApi => "SerpAPI",
        }
    }

    pub const fn source(self) -> &'static str {
        match self {
            Self::Bocha => "bocha-general-web",
            Self::Tavily => "tavily-general-web",
            Self::SerpApi => "serpapi-general-web",
        }
    }

    /// Stable LocalBridge request spelling. This is deliberately separate
    /// from the display label (`SerpAPI`) and serde spelling (`serp_api`).
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Bocha => "Bocha",
            Self::Tavily => "Tavily",
            Self::SerpApi => "SerpApi",
        }
    }

    pub fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "Bocha" => Some(Self::Bocha),
            "Tavily" => Some(Self::Tavily),
            "SerpApi" => Some(Self::SerpApi),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchUseScope {
    ResearchOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationTimeQuality {
    ExactProviderTime,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralWebResearchBatchEvidence {
    pub provider: GeneralWebResearchProvider,
    pub source: String,
    pub query: String,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub use_scope: ResearchUseScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralWebResearchRecordEvidence {
    pub provider: GeneralWebResearchProvider,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub item_id: String,
    pub publication_quality: PublicationTimeQuality,
    pub use_scope: ResearchUseScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralWebResearchRecord {
    pub title: String,
    pub snippet: String,
    pub url: String,
    pub publisher: String,
    pub published_at_raw: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub evidence: GeneralWebResearchRecordEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeneralWebResearchBatch {
    Available {
        records: Vec<GeneralWebResearchRecord>,
        evidence: GeneralWebResearchBatchEvidence,
    },
    VerifiedEmpty(GeneralWebResearchBatchEvidence),
}

impl GeneralWebResearchBatch {
    pub fn evidence(&self) -> &GeneralWebResearchBatchEvidence {
        match self {
            Self::Available { evidence, .. } | Self::VerifiedEmpty(evidence) => evidence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralWebResearchStage {
    Request,
    Transport,
    Protocol,
    Evidence,
}

impl GeneralWebResearchStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Transport => "transport",
            Self::Protocol => "protocol",
            Self::Evidence => "evidence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralWebResearchError {
    provider: GeneralWebResearchProvider,
    reason_code: &'static str,
    retryable: bool,
    stage: GeneralWebResearchStage,
    message: String,
}

impl GeneralWebResearchError {
    fn new(
        provider: GeneralWebResearchProvider,
        reason_code: &'static str,
        retryable: bool,
        stage: GeneralWebResearchStage,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            reason_code,
            retryable,
            stage,
            message: message.into(),
        }
    }

    pub const fn provider(&self) -> GeneralWebResearchProvider {
        self.provider
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn stage(&self) -> GeneralWebResearchStage {
        self.stage
    }
}

impl fmt::Display for GeneralWebResearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} failed at {}: {}",
            self.provider.label(),
            self.reason_code,
            self.stage.as_str(),
            self.message
        )
    }
}

impl std::error::Error for GeneralWebResearchError {}

#[derive(Debug, Clone)]
struct RawResearchRecord {
    title: String,
    snippet: String,
    url: String,
    publisher: Option<String>,
    published_at_raw: Option<String>,
}

#[derive(Clone)]
pub struct GeneralWebResearchGateway {
    provider: GeneralWebResearchProvider,
}

impl GeneralWebResearchGateway {
    pub const fn new(provider: GeneralWebResearchProvider) -> Self {
        Self { provider }
    }

    /// Compatibility factory for callers that construct all gateways from
    /// process configuration. Provider credentials live in the remote host.
    pub fn from_environment(provider: GeneralWebResearchProvider) -> Self {
        Self::new(provider)
    }

    pub const fn provider(&self) -> GeneralWebResearchProvider {
        self.provider
    }

    pub fn is_available(&self) -> bool {
        // The remote host owns credentials. Bridge initialization is the only
        // local availability signal; call failures remain explicit.
        super::grpc_source::bridge_for("SemanticSearch").is_ok()
    }

    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<GeneralWebResearchBatch, GeneralWebResearchError> {
        let query = query.trim();
        if query.is_empty() || !(1..=MAX_RESULTS).contains(&limit) {
            return Err(GeneralWebResearchError::new(
                self.provider,
                "invalid_request",
                false,
                GeneralWebResearchStage::Request,
                format!("query must be non-empty and limit must be within 1..={MAX_RESULTS}"),
            ));
        }
        match super::grpc_source::bridge_for("SemanticSearch") {
            Ok(bridge) => {
                return match bridge
                    .semantic_search_async(self.provider, query, limit)
                    .await
                {
                    Ok(batch) => Ok(batch),
                    Err(e) => Err(GeneralWebResearchError::new(
                        self.provider,
                        e.reason_code(),
                        e.retryable(),
                        GeneralWebResearchStage::Transport,
                        e.to_string(),
                    )),
                };
            }
            Err(error) => {
                return Err(GeneralWebResearchError::new(
                    self.provider,
                    "grpc_bridge",
                    true,
                    GeneralWebResearchStage::Transport,
                    format!("SemanticSearch 桥初始化失败: {error}"),
                ));
            }
        }
    }
}

fn admit_records(
    provider: GeneralWebResearchProvider,
    query: &str,
    limit: usize,
    observed_at: DateTime<Utc>,
    records: Vec<RawResearchRecord>,
) -> Result<GeneralWebResearchBatch, GeneralWebResearchError> {
    let mut admitted = Vec::with_capacity(records.len());
    let mut seen_urls = HashSet::new();
    for raw in records {
        let title = raw.title.trim().to_string();
        if title.is_empty() {
            return Err(invalid_evidence(provider, "record title is empty"));
        }
        let parsed_url = url::Url::parse(raw.url.trim())
            .map_err(|error| invalid_evidence(provider, format!("invalid URL: {error}")))?;
        if parsed_url.scheme() != "https" || parsed_url.host_str().is_none() {
            return Err(invalid_evidence(
                provider,
                "record URL must be an absolute HTTPS URL",
            ));
        }
        let url = parsed_url.to_string();
        if !seen_urls.insert(url.clone()) {
            return Err(invalid_evidence(
                provider,
                format!("duplicate record URL: {url}"),
            ));
        }
        let published_at_raw = raw
            .published_at_raw
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let (published_at, publication_quality) = match published_at_raw.as_deref() {
            Some(value) => {
                let parsed = parse_provider_time(value).ok_or_else(|| {
                    invalid_evidence(
                        provider,
                        format!("provider publication time is invalid: {value}"),
                    )
                })?;
                if parsed > observed_at {
                    return Err(invalid_evidence(
                        provider,
                        format!("provider publication time is in the future: {value}"),
                    ));
                }
                (Some(parsed), PublicationTimeQuality::ExactProviderTime)
            }
            None => (None, PublicationTimeQuality::Missing),
        };
        let publisher = raw
            .publisher
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| parsed_url.host_str().expect("validated host").to_string());
        let item_id = sha256_hex(url.as_bytes());
        admitted.push((
            title,
            raw.snippet.chars().take(500).collect::<String>(),
            url,
            publisher,
            published_at_raw,
            published_at,
            publication_quality,
            item_id,
        ));
    }
    admitted.truncate(limit);

    let mut identity = format!(
        "{}\n{}\n{}\n",
        provider.source(),
        query,
        observed_at.to_rfc3339()
    );
    for record in &admitted {
        identity.push_str(&record.7);
        identity.push('\n');
    }
    let batch_id = sha256_hex(identity.as_bytes());
    let evidence = GeneralWebResearchBatchEvidence {
        provider,
        source: provider.source().to_string(),
        query: query.to_string(),
        observed_at,
        batch_id: batch_id.clone(),
        use_scope: ResearchUseScope::ResearchOnly,
    };
    if admitted.is_empty() {
        return Ok(GeneralWebResearchBatch::VerifiedEmpty(evidence));
    }

    let records = admitted
        .into_iter()
        .map(
            |(
                title,
                snippet,
                url,
                publisher,
                published_at_raw,
                published_at,
                publication_quality,
                item_id,
            )| GeneralWebResearchRecord {
                title,
                snippet,
                url,
                publisher,
                published_at_raw,
                published_at,
                evidence: GeneralWebResearchRecordEvidence {
                    provider,
                    observed_at,
                    batch_id: batch_id.clone(),
                    item_id,
                    publication_quality,
                    use_scope: ResearchUseScope::ResearchOnly,
                },
            },
        )
        .collect();
    Ok(GeneralWebResearchBatch::Available { records, evidence })
}

fn invalid_evidence(
    provider: GeneralWebResearchProvider,
    message: impl Into<String>,
) -> GeneralWebResearchError {
    GeneralWebResearchError::new(
        provider,
        "invalid_evidence",
        false,
        GeneralWebResearchStage::Evidence,
        message,
    )
}

fn parse_provider_time(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(value) = DateTime::parse_from_rfc2822(raw) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") {
        return Some(Utc.from_utc_datetime(&value));
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|value| Utc.from_utc_datetime(&value))
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(url: &str, published_at_raw: Option<&str>) -> RawResearchRecord {
        RawResearchRecord {
            title: "TEST_CODE research result".to_string(),
            snippet: "research context".to_string(),
            url: url.to_string(),
            publisher: None,
            published_at_raw: published_at_raw.map(str::to_string),
        }
    }

    #[test]
    fn general_web_research_admission_is_explicitly_research_only() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 28, 8, 0, 0).unwrap();
        let batch = admit_records(
            GeneralWebResearchProvider::Bocha,
            "TEST_CODE query",
            5,
            observed_at,
            vec![raw(
                "https://example.invalid/research",
                Some("2026-07-28T07:30:00Z"),
            )],
        )
        .unwrap();
        let GeneralWebResearchBatch::Available { records, evidence } = batch else {
            panic!("expected available research batch");
        };
        assert_eq!(evidence.use_scope, ResearchUseScope::ResearchOnly);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].evidence.use_scope,
            ResearchUseScope::ResearchOnly
        );
        assert_eq!(records[0].evidence.batch_id, evidence.batch_id);
        assert_eq!(
            records[0].evidence.publication_quality,
            PublicationTimeQuality::ExactProviderTime
        );
    }

    #[test]
    fn missing_publication_time_remains_missing() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 28, 8, 0, 0).unwrap();
        let batch = admit_records(
            GeneralWebResearchProvider::Tavily,
            "TEST_CODE query",
            5,
            observed_at,
            vec![raw("https://example.invalid/undated", None)],
        )
        .unwrap();
        let GeneralWebResearchBatch::Available { records, .. } = batch else {
            panic!("expected available research batch");
        };
        assert_eq!(records[0].published_at, None);
        assert_eq!(records[0].published_at_raw, None);
        assert_eq!(
            records[0].evidence.publication_quality,
            PublicationTimeQuality::Missing
        );
    }

    #[test]
    fn invalid_or_future_dates_and_non_https_or_duplicate_urls_fail_closed() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 28, 8, 0, 0).unwrap();
        for records in [
            vec![raw(
                "https://example.invalid/invalid-date",
                Some("yesterday"),
            )],
            vec![raw(
                "https://example.invalid/future",
                Some("2026-07-28T08:00:01Z"),
            )],
            vec![raw("http://example.invalid/plaintext", None)],
            vec![
                raw("https://example.invalid/duplicate", None),
                raw("https://example.invalid/duplicate", None),
            ],
        ] {
            let error = admit_records(
                GeneralWebResearchProvider::SerpApi,
                "TEST_CODE query",
                5,
                observed_at,
                records,
            )
            .unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(!error.retryable());
            assert_eq!(error.stage(), GeneralWebResearchStage::Evidence);
        }
    }

    #[test]
    fn verified_empty_preserves_batch_evidence() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 28, 8, 0, 0).unwrap();
        let batch = admit_records(
            GeneralWebResearchProvider::Bocha,
            "TEST_CODE query",
            5,
            observed_at,
            Vec::new(),
        )
        .unwrap();
        let GeneralWebResearchBatch::VerifiedEmpty(evidence) = batch else {
            panic!("expected verified-empty batch");
        };
        assert_eq!(evidence.provider, GeneralWebResearchProvider::Bocha);
        assert_eq!(evidence.use_scope, ResearchUseScope::ResearchOnly);
        assert!(!evidence.batch_id.is_empty());
    }

    #[tokio::test]
    async fn invalid_request_is_typed_before_transport() {
        let gateway = GeneralWebResearchGateway::new(GeneralWebResearchProvider::Bocha);
        let invalid = gateway.search("", 0).await.unwrap_err();
        assert_eq!(invalid.reason_code(), "invalid_request");
        assert!(!invalid.retryable());
        assert_eq!(invalid.stage(), GeneralWebResearchStage::Request);
    }
}
