//! BR-175: generic web discovery is a ResearchOnly Gateway.
//!
//! Bocha, Tavily, and SerpAPI are not financial/news fact providers. This
//! module owns their wire protocols so business modules cannot turn direct web
//! results into an implicit fallback for pinned Magic providers.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_RESULTS: usize = 50;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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

    const fn credential_env_name(self) -> &'static str {
        match self {
            Self::Bocha => "BOCHA_API_KEYS",
            Self::Tavily => "TAVILY_API_KEYS",
            Self::SerpApi => "SERPAPI_KEYS",
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
    api_keys: Arc<Vec<String>>,
    key_cursor: Arc<AtomicUsize>,
    client: reqwest::Client,
}

impl GeneralWebResearchGateway {
    pub fn new(provider: GeneralWebResearchProvider, api_keys: Vec<String>) -> Self {
        let api_keys = api_keys
            .into_iter()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect();
        Self {
            provider,
            api_keys: Arc::new(api_keys),
            key_cursor: Arc::new(AtomicUsize::new(0)),
            client: reqwest::Client::new(),
        }
    }

    /// Build from process credentials without exposing the environment
    /// variable name outside the Gateway boundary.
    pub fn from_environment(provider: GeneralWebResearchProvider) -> Self {
        let api_keys = std::env::var(provider.credential_env_name())
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self::new(provider, api_keys)
    }

    pub const fn provider(&self) -> GeneralWebResearchProvider {
        self.provider
    }

    pub fn is_available(&self) -> bool {
        // P4 M4b 批次 1B: grpc 模式桥存在即视为可用 (API key 服务端持有)。
        // 保持 SearchService 路由语义 — 桥故障在调用时显式报错, 不静默回退。
        matches!(
            super::grpc_source::bridge_for("SemanticSearch"),
            Ok(Some(_))
        ) || !self.api_keys.is_empty()
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
        // BR-242: grpc 模式保留调用方选择的 exact provider，API key 仍由服务端持有。
        match super::grpc_source::bridge_for("SemanticSearch") {
            Ok(Some(bridge)) => {
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
            Ok(None) => {}
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
        let api_key = self.next_key().ok_or_else(|| {
            GeneralWebResearchError::new(
                self.provider,
                "missing_credentials",
                false,
                GeneralWebResearchStage::Request,
                "no non-empty API key is configured",
            )
        })?;
        let observed_at = Utc::now();
        let raw = match self.provider {
            GeneralWebResearchProvider::Bocha => self.search_bocha(query, limit, api_key).await?,
            GeneralWebResearchProvider::Tavily => self.search_tavily(query, limit, api_key).await?,
            GeneralWebResearchProvider::SerpApi => {
                self.search_serpapi(query, limit, api_key).await?
            }
        };
        admit_records(self.provider, query, limit, observed_at, raw)
    }

    fn next_key(&self) -> Option<&str> {
        let len = self.api_keys.len();
        if len == 0 {
            return None;
        }
        let index = self.key_cursor.fetch_add(1, Ordering::Relaxed) % len;
        self.api_keys.get(index).map(String::as_str)
    }

    async fn search_bocha(
        &self,
        query: &str,
        limit: usize,
        api_key: &str,
    ) -> Result<Vec<RawResearchRecord>, GeneralWebResearchError> {
        #[derive(Serialize)]
        struct Request<'a> {
            query: &'a str,
            freshness: &'static str,
            summary: bool,
            count: usize,
        }
        #[derive(Deserialize)]
        struct Page {
            name: String,
            snippet: Option<String>,
            summary: Option<String>,
            url: String,
            #[serde(rename = "siteName")]
            site_name: Option<String>,
            #[serde(rename = "datePublished")]
            date_published: Option<String>,
        }
        #[derive(Deserialize)]
        struct Pages {
            value: Vec<Page>,
        }
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "webPages")]
            web_pages: Option<Pages>,
        }
        #[derive(Deserialize)]
        struct Response {
            code: u32,
            msg: Option<String>,
            data: Option<Data>,
        }

        let response = self
            .client
            .post("https://api.bocha.cn/v1/web-search")
            .bearer_auth(api_key)
            .json(&Request {
                query,
                freshness: "oneWeek",
                summary: true,
                count: limit,
            })
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| self.transport_error(error))?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.status_error(status));
        }
        let payload: Response = response
            .json()
            .await
            .map_err(|error| self.protocol_error(error))?;
        if payload.code != 200 {
            return Err(GeneralWebResearchError::new(
                self.provider,
                "provider_rejected",
                false,
                GeneralWebResearchStage::Protocol,
                payload
                    .msg
                    .unwrap_or_else(|| format!("provider code {}", payload.code)),
            ));
        }
        Ok(payload
            .data
            .and_then(|data| data.web_pages)
            .map(|pages| pages.value)
            .unwrap_or_default()
            .into_iter()
            .map(|page| RawResearchRecord {
                title: page.name,
                snippet: page.summary.or(page.snippet).unwrap_or_default(),
                url: page.url,
                publisher: page.site_name,
                published_at_raw: page.date_published,
            })
            .collect())
    }

    async fn search_tavily(
        &self,
        query: &str,
        limit: usize,
        api_key: &str,
    ) -> Result<Vec<RawResearchRecord>, GeneralWebResearchError> {
        #[derive(Serialize)]
        struct Request<'a> {
            query: &'a str,
            search_depth: &'static str,
            max_results: usize,
            include_answer: bool,
            include_raw_content: bool,
            days: u32,
        }
        #[derive(Deserialize)]
        struct Item {
            title: String,
            content: String,
            url: String,
            published_date: Option<String>,
        }
        #[derive(Deserialize)]
        struct Response {
            results: Vec<Item>,
        }

        let response = self
            .client
            .post("https://api.tavily.com/search")
            .bearer_auth(api_key)
            .json(&Request {
                query,
                search_depth: "advanced",
                max_results: limit,
                include_answer: false,
                include_raw_content: false,
                days: 7,
            })
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| self.transport_error(error))?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.status_error(status));
        }
        let payload: Response = response
            .json()
            .await
            .map_err(|error| self.protocol_error(error))?;
        Ok(payload
            .results
            .into_iter()
            .map(|item| RawResearchRecord {
                title: item.title,
                snippet: item.content,
                url: item.url,
                publisher: None,
                published_at_raw: item.published_date,
            })
            .collect())
    }

    async fn search_serpapi(
        &self,
        query: &str,
        limit: usize,
        api_key: &str,
    ) -> Result<Vec<RawResearchRecord>, GeneralWebResearchError> {
        #[derive(Deserialize)]
        struct Item {
            title: String,
            snippet: Option<String>,
            link: String,
            source: Option<String>,
            date: Option<String>,
        }
        #[derive(Deserialize)]
        struct Response {
            organic_results: Option<Vec<Item>>,
        }
        let limit_text = limit.to_string();

        let response = self
            .client
            .get("https://serpapi.com/search")
            .query(&[
                ("engine", "baidu"),
                ("q", query),
                ("api_key", api_key),
                ("num", limit_text.as_str()),
            ])
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| self.transport_error(error))?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.status_error(status));
        }
        let payload: Response = response
            .json()
            .await
            .map_err(|error| self.protocol_error(error))?;
        Ok(payload
            .organic_results
            .unwrap_or_default()
            .into_iter()
            .map(|item| RawResearchRecord {
                title: item.title,
                snippet: item.snippet.unwrap_or_default(),
                url: item.link,
                publisher: item.source,
                published_at_raw: item.date,
            })
            .collect())
    }

    fn transport_error(&self, error: reqwest::Error) -> GeneralWebResearchError {
        GeneralWebResearchError::new(
            self.provider,
            "transport",
            true,
            GeneralWebResearchStage::Transport,
            error.to_string(),
        )
    }

    fn protocol_error(&self, error: reqwest::Error) -> GeneralWebResearchError {
        GeneralWebResearchError::new(
            self.provider,
            "protocol",
            false,
            GeneralWebResearchStage::Protocol,
            error.to_string(),
        )
    }

    fn status_error(&self, status: StatusCode) -> GeneralWebResearchError {
        let (reason_code, retryable) = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ("authentication", false),
            StatusCode::TOO_MANY_REQUESTS => ("rate_limited", true),
            value if value.is_server_error() => ("provider_rejected", true),
            _ => ("provider_rejected", false),
        };
        GeneralWebResearchError::new(
            self.provider,
            reason_code,
            retryable,
            GeneralWebResearchStage::Transport,
            format!("HTTP {status}"),
        )
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
    async fn invalid_request_and_missing_credentials_are_typed() {
        let gateway = GeneralWebResearchGateway::new(GeneralWebResearchProvider::Bocha, Vec::new());
        let invalid = gateway.search("", 0).await.unwrap_err();
        assert_eq!(invalid.reason_code(), "invalid_request");
        assert!(!invalid.retryable());
        assert_eq!(invalid.stage(), GeneralWebResearchStage::Request);

        let missing = gateway.search("TEST_CODE query", 1).await.unwrap_err();
        assert_eq!(missing.reason_code(), "missing_credentials");
        assert!(!missing.retryable());
    }
}
