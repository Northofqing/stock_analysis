//! BR-175 no-network adapter from the typed research Gateway to legacy
//! research-context consumers.

use async_trait::async_trait;

use crate::data_gateway::{
    GeneralWebResearchBatch, GeneralWebResearchGateway, GeneralWebResearchProvider,
};
use crate::search_service::types::{
    NewsType, SearchEvidence, SearchProvider, SearchResponse, SearchResult, Sentiment,
};

pub struct GeneralWebSearchProvider {
    gateway: GeneralWebResearchGateway,
}

impl GeneralWebSearchProvider {
    pub fn new(provider: GeneralWebResearchProvider, api_keys: Vec<String>) -> Self {
        Self {
            gateway: GeneralWebResearchGateway::new(provider, api_keys),
        }
    }

    pub fn from_environment(provider: GeneralWebResearchProvider) -> Self {
        Self {
            gateway: GeneralWebResearchGateway::from_environment(provider),
        }
    }
}

#[async_trait]
impl SearchProvider for GeneralWebSearchProvider {
    fn name(&self) -> &str {
        self.gateway.provider().label()
    }

    fn is_available(&self) -> bool {
        self.gateway.is_available()
    }

    fn supports_general_web_search(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let started_at = std::time::Instant::now();
        match self.gateway.search(query, max_results).await {
            Ok(GeneralWebResearchBatch::Available { records, evidence }) => {
                let results = records
                    .into_iter()
                    .map(|record| {
                        let mut result = SearchResult {
                            title: record.title,
                            snippet: record.snippet,
                            url: record.url,
                            source: record.publisher,
                            published_date: record.published_at_raw,
                            news_type: NewsType::Other,
                            sentiment: Sentiment::Unknown,
                            importance: 5,
                            relevance: match evidence.provider {
                                GeneralWebResearchProvider::Bocha => 0.8,
                                GeneralWebResearchProvider::Tavily => 0.6,
                                GeneralWebResearchProvider::SerpApi => 0.7,
                            },
                            keywords: Vec::new(),
                            evidence: SearchEvidence::ResearchOnly {
                                provider: record.evidence.provider,
                                source: evidence.source.clone(),
                                observed_at: record.evidence.observed_at.to_rfc3339(),
                                batch_id: record.evidence.batch_id,
                                item_id: record.evidence.item_id,
                                publication_quality: record.evidence.publication_quality,
                            },
                        };
                        result.analyze_type();
                        result.analyze_sentiment();
                        result.calculate_importance();
                        result
                    })
                    .collect();
                let mut response =
                    SearchResponse::success(query.to_string(), self.name().to_string(), results);
                response.search_time = started_at.elapsed().as_secs_f64();
                response
            }
            Ok(GeneralWebResearchBatch::VerifiedEmpty(_)) => {
                let mut response =
                    SearchResponse::success(query.to_string(), self.name().to_string(), Vec::new());
                response.search_time = started_at.elapsed().as_secs_f64();
                response
            }
            Err(error) => {
                let mut response = SearchResponse::typed_error(
                    query.to_string(),
                    self.name().to_string(),
                    error.to_string(),
                    error.reason_code(),
                    error.retryable(),
                    error.stage().as_str(),
                );
                response.search_time = started_at.elapsed().as_secs_f64();
                response
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_credentials_remain_typed_and_fail_closed() {
        let provider = GeneralWebSearchProvider::new(GeneralWebResearchProvider::Bocha, Vec::new());
        let response = provider.search("TEST_CODE query", 1).await;
        assert!(!response.success);
        let failure = response.failure.expect("typed failure");
        assert_eq!(failure.reason_code, "missing_credentials");
        assert!(!failure.retryable);
        assert_eq!(failure.stage, "request");
    }
}
