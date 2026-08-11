//! BR-172 NewsAI shadow producer（默认启用，2026-08-11 起取消 env 开关）。
//!
//! This adapter consumes only source-bound batches from the same aggregator
//! tick, acquires audited market evidence, performs one receipt-bearing model
//! call and appends the immutable assessment audit. It has no delivery,
//! prediction, reservation or trading capability.

use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use std::sync::Arc;
use stock_analysis::calendar::{self, MarketSession};
use stock_analysis::data_gateway::instrument_identity::resolve_production_equity;
use stock_analysis::data_gateway::{HistoricalBarsGateway, MarketDataGateway};
use stock_analysis::database::news_ai::NewsAiAssessmentAuditInput;
use stock_analysis::llm::LlmRegistry;
use stock_analysis::monitor::news_ai::{
    AdmittedNewsFact, NewsAIAnalyzer, NewsAiRequest, NewsMarketContext, NewsMarketSnapshot,
    NEWS_AI_ANALYSIS_VERSION,
};
use stock_analysis::news::aggregator::AdmittedGlobalNewsBatch;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_ASSESSMENTS_PER_TICK: usize = 5;
const DAILY_HISTORY_DAYS: usize = 60;

static SHADOW_BATCH_PERMIT: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(1)));

/// Spawn one bounded worker without delaying existing flash/selection
/// governance. A concurrent batch is skipped without writing completion state.
pub fn spawn_from_same_tick(batches: &[AdmittedGlobalNewsBatch]) {
    // v15 隔离: --test 进程不真调 LLM, 也不写评估审计。
    if stock_analysis::risk::env_guard::runtime_is_test_process() {
        log::debug!("[NewsAI-shadow][BR-172] --test 进程隔离, 跳过 AI 评估");
        return;
    }
    let permit = match SHADOW_BATCH_PERMIT.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            log::info!("[NewsAI-shadow][BR-172] skipped busy=true; no completion state written");
            return;
        }
    };
    let batches = batches.to_vec();
    tokio::spawn(async move {
        run_same_tick_batches(batches, permit).await;
    });
}

async fn run_same_tick_batches(
    batches: Vec<AdmittedGlobalNewsBatch>,
    _permit: OwnedSemaphorePermit,
) {
    let candidates = exact_candidates(&batches);
    if candidates.is_empty() {
        log::debug!("[NewsAI-shadow][BR-172] no exact source-bound equity target");
        return;
    }

    let registry = LlmRegistry::from_env();
    let Some(provider) = registry.select("news_ai") else {
        log::warn!("[NewsAI-shadow][BR-172] model unavailable; no assessment state written");
        return;
    };
    let analyzer = NewsAIAnalyzer::new(provider);

    let mut retained = 0_usize;
    let mut skipped_existing = 0_usize;
    let mut failed = 0_usize;
    for candidate in candidates {
        match assess_candidate(&analyzer, &candidate).await {
            Ok(CandidateOutcome::Retained { inserted }) => {
                retained += usize::from(inserted);
                skipped_existing += usize::from(!inserted);
            }
            Ok(CandidateOutcome::AlreadyRetained) => skipped_existing += 1,
            Err(error) => {
                failed += 1;
                log::warn!(
                    "[NewsAI-shadow][BR-172] candidate failed key={} error={error}",
                    candidate.key
                );
            }
        }
    }
    log::info!(
        "[NewsAI-shadow][BR-172] completed retained={} existing={} failed={}",
        retained,
        skipped_existing,
        failed
    );
}

#[derive(Debug, Clone)]
struct ShadowCandidate {
    key: String,
    batch: AdmittedGlobalNewsBatch,
    record_index: usize,
    target_code: String,
}

fn exact_candidates(batches: &[AdmittedGlobalNewsBatch]) -> Vec<ShadowCandidate> {
    let mut unique = BTreeMap::new();
    for batch in batches {
        for (record_index, record) in batch.records().iter().enumerate() {
            for source_code in &record.instruments {
                let identity = match resolve_production_equity(source_code, None).and_then(
                    |identity| {
                        identity.require_a_share()?;
                        Ok(identity)
                    },
                ) {
                    Ok(identity) => identity,
                    Err(error) => {
                        log::warn!(
                            "[NewsAI-shadow][BR-172][BR-173] source target rejected code={source_code:?}: {error}"
                        );
                        continue;
                    }
                };
                let target_code = identity.storage_code().to_owned();
                let key = format!(
                    "{:?}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
                    batch.evidence().provider,
                    batch.evidence().batch_id,
                    record.item_id,
                    target_code,
                    NEWS_AI_ANALYSIS_VERSION
                );
                unique
                    .entry(key.clone())
                    .or_insert_with(|| ShadowCandidate {
                        key,
                        batch: batch.clone(),
                        record_index,
                        target_code,
                    });
            }
        }
    }
    unique
        .into_values()
        .take(MAX_ASSESSMENTS_PER_TICK)
        .collect()
}

enum CandidateOutcome {
    AlreadyRetained,
    Retained { inserted: bool },
}

async fn assess_candidate(
    analyzer: &NewsAIAnalyzer,
    candidate: &ShadowCandidate,
) -> Result<CandidateOutcome, String> {
    let fact = AdmittedNewsFact::from_admitted_global(
        &candidate.batch,
        candidate.record_index,
        &candidate.target_code,
    )
    .map_err(|error| error.to_string())?;

    let identity_fact = fact.clone();
    let already_retained = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db()
            .has_news_ai_assessment_for_fact(&identity_fact, NEWS_AI_ANALYSIS_VERSION)
    })
    .await
    .map_err(|error| format!("assessment identity lookup task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    if already_retained {
        return Ok(CandidateOutcome::AlreadyRetained);
    }

    let as_of = chrono::Utc::now();
    let context = news_market_context(calendar::current_session());
    let daily = HistoricalBarsGateway::new()
        .required_daily_bars_async(&candidate.target_code, DAILY_HISTORY_DAYS)
        .await
        .map_err(|error| error.to_string())?;
    let quote = if context == NewsMarketContext::Intraday {
        let code = candidate.target_code.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                MarketDataGateway::new().required_realtime_quote(&code)
            })
            .await
            .map_err(|error| format!("realtime quote task failed: {error}"))?
            .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let market =
        NewsMarketSnapshot::try_from_admitted(&candidate.target_code, context, as_of, daily, quote)
            .map_err(|error| error.to_string())?;
    let request = NewsAiRequest::try_new(fact, market, Vec::new(), NEWS_AI_ANALYSIS_VERSION)
        .map_err(|error| error.to_string())?;
    let assessment = analyzer
        .assess(&request)
        .await
        .map_err(|error| error.to_string())?;
    let audit_input = NewsAiAssessmentAuditInput::from_core(&request, &assessment)
        .map_err(|error| error.to_string())?;
    let receipt = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db().append_news_ai_assessment(&audit_input)
    })
    .await
    .map_err(|error| format!("assessment audit task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    Ok(CandidateOutcome::Retained {
        inserted: receipt.inserted,
    })
}

const fn news_market_context(session: MarketSession) -> NewsMarketContext {
    match session {
        MarketSession::Auction
        | MarketSession::Morning
        | MarketSession::LunchBreak
        | MarketSession::Afternoon => NewsMarketContext::Intraday,
        MarketSession::AfterHours | MarketSession::Closed => NewsMarketContext::PostClose,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_mapping_requires_realtime_only_during_market_windows() {
        assert_eq!(
            news_market_context(MarketSession::Morning),
            NewsMarketContext::Intraday
        );
        assert_eq!(
            news_market_context(MarketSession::AfterHours),
            NewsMarketContext::PostClose
        );
        assert_eq!(
            news_market_context(MarketSession::Closed),
            NewsMarketContext::PostClose
        );
    }

    #[test]
    fn shadow_adapter_has_no_delivery_prediction_or_order_capability() {
        let source = include_str!("news_ai_shadow.rs");
        for (prefix, suffix) in [
            ("push_", "governor"),
            ("save_", "prediction"),
            ("place_", "order"),
            ("Trading", "Bus"),
            ("Sink", "Router"),
        ] {
            assert!(!source.contains(&format!("{prefix}{suffix}")));
        }
    }
}
