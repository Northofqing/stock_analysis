//! BR-172 NewsAI governed producer（默认启用，2026-08-11 起取消 env 开关）。
//!
//! New analysis consumes only same-tick admitted batches. Independent durable
//! recovery consumes existing immutable assessments without acquisition or
//! model calls. One bounded worker owns both and has no trading capability.

use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use stock_analysis::calendar::{self, MarketSession};
use stock_analysis::data_gateway::instrument_identity::resolve_production_equity;
use stock_analysis::data_gateway::{HistoricalBarsGateway, MarketDataGateway};
use stock_analysis::database::news_ai::NewsAiPendingRecovery;
use stock_analysis::llm::LlmRegistry;
use stock_analysis::monitor::news_ai::{
    deliver_governed_news_ai, AdmittedNewsFact, GovernedNewsAiDelivery, NewsAIAnalyzer,
    NewsAiAnalysisProfile, NewsAiChainContext, NewsAiDeliveryAuditReceipt,
    NewsAiDeliveryReservation, NewsAiGovernedDeliveryOutcome, NewsAiGovernedDeliveryPort,
    NewsAiIdentityV3, NewsAiPhysicalPushOutcome, NewsAiPredictionLinkReceipt, NewsAiRequest,
    NewsAiReserveOutcome, NewsMarketContext, NewsMarketSnapshot,
};
use stock_analysis::news::aggregator::AdmittedGlobalNewsBatch;
use tokio::sync::Semaphore;

const MAX_ASSESSMENTS_PER_TICK: usize = 5;
const MAX_CANDIDATE_INSPECTIONS_PER_TICK: usize = 40;
const DAILY_HISTORY_DAYS: usize = 60;

static NEWS_AI_BATCH_PERMIT: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(1)));
// NEWS_AI_BATCH_PERMIT keeps the worker single-flight, so its cursor can be
// advanced after each bounded scan without concurrent writers.
static NEXT_NEWS_AI_CANDIDATE: AtomicUsize = AtomicUsize::new(0);

/// One scheduling seam owns both kinds of work and their shared single-flight
/// permit. Futures are supplied at the database/provider boundary.
fn schedule_news_ai_tick<R, RF, A, AF>(
    permits: &Arc<Semaphore>,
    selection_enabled: bool,
    session: MarketSession,
    batches: Option<Vec<AdmittedGlobalNewsBatch>>,
    recover: R,
    analyze: A,
) -> Option<tokio::task::JoinHandle<()>>
where
    R: FnOnce(usize) -> RF + Send + 'static,
    RF: Future<Output = usize> + Send,
    A: FnOnce(Vec<AdmittedGlobalNewsBatch>, usize) -> AF + Send + 'static,
    AF: Future<Output = ()> + Send,
{
    let batches = batches.filter(|batches| {
        selection_enabled && (session.is_trading() || session.is_auction()) && !batches.is_empty()
    });
    let permit = permits.clone().try_acquire_owned().ok()?;
    Some(tokio::spawn(async move {
        let _permit = permit;
        // One worker gives both owners a bounded share. A full recovery queue
        // cannot consume the live allowance, and live ingress cannot skip recovery.
        let recovery_limit = if batches.is_some() {
            2
        } else {
            MAX_ASSESSMENTS_PER_TICK
        };
        let used = recover(recovery_limit).await;
        if used > recovery_limit {
            log::error!(
                "[NewsAI][BR-172] recovery exceeded its hard work limit; live work stopped"
            );
            return;
        }
        if let Some(batches) = batches {
            analyze(batches, used).await;
        }
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NewAnalysisCapability {
    Enabled,
    DisabledTestProcessIsolation,
    DisabledModelProviderUnavailable,
}

impl std::fmt::Display for NewAnalysisCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enabled => formatter.write_str("enabled"),
            Self::DisabledTestProcessIsolation => {
                formatter.write_str("disabled:test_process_isolation")
            }
            Self::DisabledModelProviderUnavailable => {
                formatter.write_str("disabled:model_provider_unavailable")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GovernedDeliveryRecoveryCapability {
    Enabled,
    DisabledTestProcessIsolation,
    DisabledLaunchStage,
    DisabledAuditHealth { reason_code: String },
    DisabledPhysicalSink { reason_code: String },
}

impl std::fmt::Display for GovernedDeliveryRecoveryCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enabled => formatter.write_str("enabled"),
            Self::DisabledTestProcessIsolation => {
                formatter.write_str("disabled:test_process_isolation")
            }
            Self::DisabledLaunchStage => formatter.write_str("disabled:launch_stage_denied"),
            Self::DisabledAuditHealth { reason_code } => {
                write!(
                    formatter,
                    "disabled:delivery_audit_unavailable:{reason_code}"
                )
            }
            Self::DisabledPhysicalSink { reason_code } => {
                write!(
                    formatter,
                    "disabled:physical_sink_unavailable:{reason_code}"
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProducerSchedulingCapability {
    Enabled,
    DisabledTestProcessIsolation,
}

impl std::fmt::Display for ProducerSchedulingCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enabled => formatter.write_str("enabled"),
            Self::DisabledTestProcessIsolation => {
                formatter.write_str("disabled:test_process_isolation")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateExecution {
    DeferAuditedAssessment,
    CreateAssessmentAndDeliver,
    CreateAssessmentOnly,
    RejectNewAnalysisUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewsAiRuntimeStatus {
    new_analysis: NewAnalysisCapability,
    governed_delivery_recovery: GovernedDeliveryRecoveryCapability,
    scheduling: ProducerSchedulingCapability,
}

impl NewsAiRuntimeStatus {
    fn from_capabilities(
        new_analysis: NewAnalysisCapability,
        governed_delivery_recovery: GovernedDeliveryRecoveryCapability,
    ) -> Self {
        let scheduling = match (&new_analysis, &governed_delivery_recovery) {
            (
                NewAnalysisCapability::DisabledTestProcessIsolation,
                GovernedDeliveryRecoveryCapability::DisabledTestProcessIsolation,
            ) => ProducerSchedulingCapability::DisabledTestProcessIsolation,
            // Manual recovery audit work remains executable even without a
            // model or a physical delivery capability.
            _ => ProducerSchedulingCapability::Enabled,
        };
        Self {
            new_analysis,
            governed_delivery_recovery,
            scheduling,
        }
    }

    fn candidate_execution(&self, existing: bool) -> CandidateExecution {
        if existing {
            // The independent durable queue is the sole owner of existing
            // assessments, including ones also present in this live batch.
            return CandidateExecution::DeferAuditedAssessment;
        }

        match (&self.new_analysis, &self.governed_delivery_recovery) {
            (NewAnalysisCapability::Enabled, GovernedDeliveryRecoveryCapability::Enabled) => {
                CandidateExecution::CreateAssessmentAndDeliver
            }
            (NewAnalysisCapability::Enabled, _) => CandidateExecution::CreateAssessmentOnly,
            _ => CandidateExecution::RejectNewAnalysisUnavailable,
        }
    }
}

/// Small typed seam shared by startup reporting, scheduling and the runner.
/// Delivery health is refreshed per tick; exact per-delivery governance is
/// still revalidated immediately before the physical sink.
pub(super) struct NewsAiProducer {
    analyzer: Option<NewsAIAnalyzer>,
    test_process_isolation: bool,
    critical_sender: Option<stock_analysis::monitor::news_ai::CriticalCompletionSender>,
}

impl NewsAiProducer {
    pub(super) fn from_runtime() -> Self {
        let test_process_isolation = stock_analysis::risk::env_guard::runtime_is_test_process();
        let analyzer = if test_process_isolation {
            None
        } else {
            LlmRegistry::from_env()
                .select("news_ai")
                .map(NewsAIAnalyzer::new)
        };
        Self {
            analyzer,
            test_process_isolation,
            critical_sender: None,
        }
    }

    pub(super) fn with_critical_completion(mut self,
        sender: stock_analysis::monitor::news_ai::CriticalCompletionSender) -> Self {
        self.critical_sender = Some(sender);
        self
    }

    fn runtime_status(&self) -> NewsAiRuntimeStatus {
        if self.test_process_isolation {
            return NewsAiRuntimeStatus::from_capabilities(
                NewAnalysisCapability::DisabledTestProcessIsolation,
                GovernedDeliveryRecoveryCapability::DisabledTestProcessIsolation,
            );
        }

        let new_analysis = if self.analyzer.is_some() {
            NewAnalysisCapability::Enabled
        } else {
            NewAnalysisCapability::DisabledModelProviderUnavailable
        };
        let governed_delivery_recovery = match super::notify::news_ai_common_gate_status() {
            super::notify::NewsAiCommonGateStatus::Ready => {
                GovernedDeliveryRecoveryCapability::Enabled
            }
            super::notify::NewsAiCommonGateStatus::LaunchStageDenied => {
                GovernedDeliveryRecoveryCapability::DisabledLaunchStage
            }
            super::notify::NewsAiCommonGateStatus::AuditUnavailable { reason_code } => {
                GovernedDeliveryRecoveryCapability::DisabledAuditHealth { reason_code }
            }
            super::notify::NewsAiCommonGateStatus::PhysicalSinkUnavailable { reason_code } => {
                GovernedDeliveryRecoveryCapability::DisabledPhysicalSink { reason_code }
            }
        };
        NewsAiRuntimeStatus::from_capabilities(new_analysis, governed_delivery_recovery)
    }

    pub(super) fn log_startup_banner(&self) {
        let status = self.runtime_status();
        let banner = format!(
            "[NewsAI][BR-112][BR-172] producer_scheduling={} new_analysis={} governed_delivery_recovery={}; exact per-delivery governance remains required",
            status.scheduling, status.new_analysis, status.governed_delivery_recovery
        );
        if status.new_analysis == NewAnalysisCapability::Enabled
            && status.governed_delivery_recovery == GovernedDeliveryRecoveryCapability::Enabled
        {
            log::info!("{banner}");
        } else {
            log::warn!("{banner}");
        }
    }

    /// Recovery is independent of live ingress, selection activation, session,
    /// and model capability. Only the optional *new analysis* branch uses them.
    pub(super) fn schedule_tick(
        &self,
        selection_enabled: bool,
        session: MarketSession,
        batches: Option<&[AdmittedGlobalNewsBatch]>,
    ) {
        if self.test_process_isolation {
            return;
        }
        let status = self.runtime_status();
        let batches = batches
            .filter(|_| status.new_analysis == NewAnalysisCapability::Enabled)
            .map(<[AdmittedGlobalNewsBatch]>::to_vec);
        let analyzer = self.analyzer.clone();
        let recovery_status = status.clone();
        let critical_sender = self.critical_sender.clone();
        if schedule_news_ai_tick(
            &NEWS_AI_BATCH_PERMIT,
            selection_enabled,
            session,
            batches,
            move |limit| run_durable_recovery(recovery_status, limit),
            move |batches, used| run_same_tick_batches(batches, analyzer, status, used, critical_sender),
        )
        .is_none()
        {
            log::info!("[NewsAI][BR-172] skipped busy=true; no completion state written");
        }
    }
}

/// No analyzer, acquisition, or assessment-creation capability is passed here.
async fn run_durable_recovery(status: NewsAiRuntimeStatus, limit: usize) -> usize {
    let mut stats = NewsAiRunStats::default();
    let mut recovered_work = 0_usize;
    let pending = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db().load_pending_news_ai_recoveries(limit)
    })
    .await;
    match pending {
        Ok(Ok(pending)) => {
            recovered_work = pending.len();
            for recovery in pending {
                match recovery {
                    NewsAiPendingRecovery::Ready(audited) => {
                        if status.governed_delivery_recovery
                            != GovernedDeliveryRecoveryCapability::Enabled
                        {
                            stats.deferred += 1;
                            continue;
                        }
                        let key = audited.delivery().assessment().assessment_id().to_owned();
                        let outcome =
                            deliver_governed_news_ai(&audited, &ProductionNewsAiDeliveryPort).await;
                        stats.record_governed(&key, true, outcome);
                    }
                    NewsAiPendingRecovery::ManualReview {
                        assessment_id,
                        reason,
                        claim_id,
                    } => {
                        stats.deferred += 1;
                        let result = tokio::task::spawn_blocking(move || {
                            publish_manual_review(
                                &assessment_id, claim_id, reason.len(),
                                |kind, identity, outcome, channel, rendered_len, latency_ms| {
                                    stock_analysis::event::publish_delivery(
                                        kind, identity, outcome, channel, rendered_len, latency_ms,
                                    )?;
                                    log::warn!("[NewsAI][BR-172] pending assessment requires manual review assessment_id={assessment_id} claim={claim_id} reason={reason}");
                                    Ok(())
                                },
                                || stock_analysis::database::get_db()
                                    .confirm_news_ai_recovery_review(claim_id)
                                    .map_err(|error| error.to_string()),
                            )
                        }).await;
                        if !matches!(result, Ok(Ok(()))) {
                            stats.failed += 1;
                            log::error!("[NewsAI][BR-172] manual-review notice unconfirmed; retry retained: {result:?}");
                        }
                    }
                }
            }
        }
        Ok(Err(error)) => {
            stats.failed += 1;
            log::warn!("[NewsAI][BR-172] durable pending scan failed: {error}");
        }
        Err(error) => {
            stats.failed += 1;
            log::warn!("[NewsAI][BR-172] durable pending scan task failed: {error}");
        }
    }
    log::info!(
        "[NewsAI][BR-172] recovery visited={} pushed={} deferred={} failed={}",
        recovered_work,
        stats.pushed,
        stats.deferred,
        stats.failed
    );
    recovered_work
}

fn publish_manual_review<P, C>(
    assessment_id: &str,
    claim_id: i64,
    reason_len: usize,
    publish: P,
    confirm: C,
) -> Result<(), String>
where
    P: FnOnce(&str, Option<&str>, &str, &str, usize, u64) -> Result<(), String>,
    C: FnOnce() -> Result<(), String>,
{
    let identity = format!("news-ai-recovery:{assessment_id}:{claim_id}");
    // This acknowledges an internal notice of blocked delivery, never a sink
    // receipt or resolution of the assessment. Use the closed audit vocabulary.
    publish(
        "NewsAiRecoveryManualReview",
        Some(&identity),
        "Denied",
        "internal_audit",
        reason_len,
        0,
    )?;
    confirm()
}

async fn run_same_tick_batches(
    batches: Vec<AdmittedGlobalNewsBatch>,
    analyzer: Option<NewsAIAnalyzer>,
    status: NewsAiRuntimeStatus,
    recovered_work: usize,
    critical_sender: Option<stock_analysis::monitor::news_ai::CriticalCompletionSender>,
) {
    let mut stats = NewsAiRunStats::default();
    let Some(profile) = analyzer
        .as_ref()
        .and_then(|analyzer| analyzer.identity_profile().ok())
    else {
        log::warn!("[NewsAI] no qualified pre-call model profile; live analysis skipped");
        return;
    };
    let candidates = mixed_candidates(&batches, &profile,analyzer.as_ref().expect("qualified analyzer profile"));
    if candidates.is_empty() {
        log::debug!("[NewsAI][BR-172] no exact source-bound equity or empty-instrument global candidate");
    }
    let mut budget = CandidateVisitBudget::with_worked(
        candidates.len(),
        NEXT_NEWS_AI_CANDIDATE.load(Ordering::Relaxed),
        recovered_work,
    );
    while let Some(index) = budget.next_index() {
        let candidate = &candidates[index];
        let key=candidate.key();
        let outcome=match candidate {
            MixedCandidate::Equity(candidate)=> {
                let designated = AdmittedNewsFact::from_admitted_global(&candidate.batch, candidate.record_index, &candidate.target_code)
                    .ok().and_then(|fact| stock_analysis::monitor::news_ai::canonical_critical_target(&fact))
                    .as_deref() == Some(candidate.target_code.as_str());
                if designated && critical_sender.is_some() {
                    assess_critical_candidate(analyzer.as_ref(), &status, candidate, critical_sender.as_ref().expect("checked sender")).await
                } else { assess_candidate(analyzer.as_ref(), &status, candidate).await }
            }
            MixedCandidate::Global { fact,.. }=>match critical_sender.as_ref() {
                Some(sender)=>assess_global_critical_candidate(analyzer.as_ref(),&status,fact.clone(),sender).await,
                None=>Err("global completion owner unavailable".into()),
            },
        };
        let did_work = match outcome {
            Ok(CandidateOutcome::Governed { existing, delivery }) => {
                stats.record_governed(key, existing, delivery)
            }
            Ok(CandidateOutcome::AwaitingDeliveryRecovery { existing }) => {
                stats.deferred += 1;
                log::warn!(
                    "[NewsAI][BR-172] assessment retained for governed delivery recovery key={} existing={} governed_delivery_recovery={}",
                    key,
                    existing,
                    status.governed_delivery_recovery
                );
                !existing
            }
            Ok(CandidateOutcome::TerminalDenied) => {
                stats.denied += 1;
                false
            }
            Err(error) => {
                stats.failed += 1;
                log::warn!(
                    "[NewsAI][BR-172] candidate failed key={} error={error}",
                    key
                );
                true
            }
        };
        budget.record_work(did_work);
    }
    NEXT_NEWS_AI_CANDIDATE.store(budget.next_start(), Ordering::Relaxed);
    log::info!(
        "[NewsAI][BR-172] completed pushed={} link_recovered={} retained_neutral={} deferred_delivery={} deduped={} admission_denied={} failed={}",
        stats.pushed,
        stats.link_recovered,
        stats.retained,
        stats.deferred,
        stats.deduped,
        stats.denied,
        stats.failed
    );
}

#[derive(Default)]
struct NewsAiRunStats {
    pushed: usize,
    link_recovered: usize,
    retained: usize,
    deferred: usize,
    deduped: usize,
    denied: usize,
    failed: usize,
}

impl NewsAiRunStats {
    fn record_governed(
        &mut self,
        key: &str,
        existing: bool,
        delivery: NewsAiGovernedDeliveryOutcome,
    ) -> bool {
        let did_work = !matches!(&delivery, NewsAiGovernedDeliveryOutcome::Deduped { .. });
        match delivery {
            NewsAiGovernedDeliveryOutcome::Pushed { .. } => self.pushed += 1,
            NewsAiGovernedDeliveryOutcome::PredictionLinkRecovered { .. } => {
                self.link_recovered += 1;
            }
            NewsAiGovernedDeliveryOutcome::RetainedNoDelivery { .. } => self.retained += 1,
            NewsAiGovernedDeliveryOutcome::Deduped { .. } => self.deduped += 1,
            // counted 准入拒绝是设计内丢弃，不污染 failed 指标。
            NewsAiGovernedDeliveryOutcome::Denied { reason, .. } => {
                self.denied += 1;
                log::info!(
                    "[NewsAI][BR-172] counted admission denied key={} existing={} reason={reason}",
                    key,
                    existing
                );
            }
            other => {
                self.failed += 1;
                log::warn!(
                    "[NewsAI][BR-172] governed delivery incomplete key={} existing={} outcome={other:?}",
                    key,
                    existing
                );
            }
        }
        did_work
    }
}

#[derive(Debug, Clone)]
struct NewsAiCandidate {
    key: String,
    batch: AdmittedGlobalNewsBatch,
    record_index: usize,
    target_code: String,
    identity: NewsAiIdentityV3,
}

/// Inspect a bounded portion of the admitted batch and spend the per-tick
/// quota only on actual new/recovery work. Completed identities do not occupy
/// the first five slots forever; repeated failures still move the cursor so a
/// later candidate gets a turn on the next tick.
struct CandidateVisitBudget {
    total: usize,
    start: usize,
    inspected: usize,
    worked: usize,
}

impl CandidateVisitBudget {
    fn new(total: usize, start: usize) -> Self {
        Self::with_worked(total, start, 0)
    }

    fn with_worked(total: usize, start: usize, worked: usize) -> Self {
        Self {
            total,
            start: if total == 0 { 0 } else { start % total },
            inspected: 0,
            worked,
        }
    }

    fn next_index(&mut self) -> Option<usize> {
        if self.total == 0
            || self.inspected >= self.total.min(MAX_CANDIDATE_INSPECTIONS_PER_TICK)
            || self.worked >= MAX_ASSESSMENTS_PER_TICK
        {
            return None;
        }
        let index = (self.start + self.inspected) % self.total;
        self.inspected += 1;
        Some(index)
    }

    fn record_work(&mut self, did_work: bool) {
        self.worked += usize::from(did_work);
    }

    fn next_start(&self) -> usize {
        if self.total == 0 {
            0
        } else {
            (self.start + self.inspected) % self.total
        }
    }
}

fn exact_candidates(
    batches: &[AdmittedGlobalNewsBatch],
    profile: &NewsAiAnalysisProfile,
) -> Vec<NewsAiCandidate> {
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
                            "[NewsAI][BR-172][BR-173] source target rejected code={source_code:?}: {error}"
                        );
                        continue;
                    }
                };
                let target_code = identity.storage_code().to_owned();
                let identity =
                    match AdmittedNewsFact::from_admitted_global(batch, record_index, &target_code)
                        .and_then(|fact| NewsAiIdentityV3::from_fact(&fact, profile))
                    {
                        Ok(identity) => identity,
                        Err(error) => {
                            log::warn!("[NewsAI] candidate identity rejected: {error}");
                            continue;
                        }
                    };
                let key = identity.digest();
                unique
                    .entry(key.clone())
                    .or_insert_with(|| NewsAiCandidate {
                        key,
                        batch: batch.clone(),
                        record_index,
                        target_code,
                        identity,
                    });
            }
        }
    }
    unique.into_values().collect()
}

enum CandidateOutcome {
    Governed {
        existing: bool,
        delivery: NewsAiGovernedDeliveryOutcome,
    },
    AwaitingDeliveryRecovery {
        existing: bool,
    },
    TerminalDenied,
}

/// BR-249: 读取目标股产业链上下文。
///
/// 1. chain_daily 最新涨停主线簇（断点 A 落库）中找目标股所属主线、簇规模、
///    近 10 自然日上榜天数（与 pipeline::extra_context::chain_mainline_note
///    同一数据链路）；2. BR-170 持仓产业链板块归属。任一步失败返回空上下文——
///    链数据是辅助证据，不阻塞逐条评估，空字段进 prompt/hash 均被容忍。
fn load_chain_context(code: &str) -> NewsAiChainContext {
    let db = stock_analysis::database::get_db();
    let mut ctx = NewsAiChainContext::default();
    match db.get_latest_chain_clusters_strict() {
        Ok(rows) => {
            for row in &rows {
                let codes: Vec<String> = match serde_json::from_str(&row.stocks) {
                    Ok(codes) => codes,
                    Err(_) => continue,
                };
                if codes.iter().any(|candidate| candidate == code) {
                    ctx.mainline_concept = Some(row.concept.clone());
                    ctx.mainline_cluster_size = Some(codes.len());
                    ctx.mainline_date = Some(row.date.clone());
                    if let Ok(as_of) = chrono::NaiveDate::parse_from_str(&row.date, "%Y-%m-%d") {
                        if let Ok(streak) =
                            db.get_chain_appearance_days_as_of_strict(&row.concept, 10, as_of)
                        {
                            ctx.mainline_streak_days = Some(streak);
                        }
                    }
                    break;
                }
            }
        }
        Err(_) => {}
    }
    if let Ok(Some(link)) = db.linked_position_chain(code) {
        ctx.board_name = Some(link.board_name);
    }
    ctx
}

/// BR-250: 经证券身份统一 Gateway 解析标的名称 (display-only, 仅卡片渲染)。
///
/// 与 BR-225 resolve_preopen_head_names 同一数据来源。名称不进 identity/证据
/// 哈希, 失败降级为 None 不阻塞逐条评估——与产业链上下文同一容错哲学。
async fn resolve_target_name(code: &str) -> Option<String> {
    use stock_analysis::data_gateway::{GatewayBatch, MarketCapabilitiesGateway};
    let codes = vec![code.to_owned()];
    let batch = MarketCapabilitiesGateway::new()
        .security_identities(&codes)
        .await
        .ok()?;
    let records = match batch {
        GatewayBatch::Available { records, .. } => records,
        GatewayBatch::VerifiedEmpty(_) => return None,
    };
    records
        .into_iter()
        .find(|record| record.code == code)
        .map(|record| record.name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

async fn assess_candidate(
    analyzer: Option<&NewsAIAnalyzer>,
    status: &NewsAiRuntimeStatus,
    candidate: &NewsAiCandidate,
) -> Result<CandidateOutcome, String> {
    // Durable recovery is owned by the independent scanner. A disabled live
    // capability must not acquire even a database/market/model attempt.
    let execution = status.candidate_execution(false);
    if execution == CandidateExecution::RejectNewAnalysisUnavailable {
        return Err(
            "receipt-bearing news_ai model provider unavailable; assessment not written".to_owned(),
        );
    }
    let analyzer = analyzer.ok_or_else(|| {
        "receipt-bearing news_ai model provider unavailable; assessment not written".to_owned()
    })?;
    let fact = AdmittedNewsFact::from_admitted_global(
        &candidate.batch,
        candidate.record_index,
        &candidate.target_code,
    )
    .map_err(|error| error.to_string())?;
    // This indexed read is a scheduling hint for a counted decision that can
    // never be reopened under the frozen identity. Every nonterminal state
    // still takes the fully validated BR-172 audit path below.
    let terminal_identity = candidate.identity.clone();
    let terminal_denial = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db()
            .is_news_ai_terminal_denial_for_identity(&terminal_identity)
    })
    .await
    .map_err(|error| format!("terminal NewsAI decision lookup task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    if terminal_denial {
        return Ok(CandidateOutcome::TerminalDenied);
    }
    let result = analyzer
        .assess_if_absent(
            candidate.identity.clone(),
            |identity| async move {
                tokio::task::spawn_blocking(move || {
                    stock_analysis::database::get_db()
                        .load_audited_news_ai_assessment_for_identity(&identity)
                })
                .await
                .map_err(|e| format!("assessment identity lookup task failed: {e}"))?
                .map(|existing| existing.is_some())
                .map_err(|e| e.to_string())
            },
            |identity| async move {
                prepare_candidate_request(fact, &candidate.target_code, identity).await
            },
        )
        .await?;
    let Some((request, assessment)) = result else {
        return Ok(CandidateOutcome::AwaitingDeliveryRecovery { existing: true });
    };
    let audited = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db().append_audited_news_ai_assessment(request, assessment)
    })
    .await
    .map_err(|error| format!("assessment audit task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    match execution {
        CandidateExecution::CreateAssessmentAndDeliver => {
            let delivery = deliver_governed_news_ai(&audited, &ProductionNewsAiDeliveryPort).await;
            Ok(CandidateOutcome::Governed {
                existing: false,
                delivery,
            })
        }
        CandidateExecution::CreateAssessmentOnly => {
            Ok(CandidateOutcome::AwaitingDeliveryRecovery { existing: false })
        }
        _ => unreachable!("typed execution rejected before model acquisition"),
    }
}

async fn prepare_candidate_request(
    mut fact: AdmittedNewsFact,
    target_code: &str,
    identity: NewsAiIdentityV3,
) -> Result<NewsAiRequest, String> {
    if let Some(name) = resolve_target_name(target_code).await {
        fact = fact.with_target_name(name);
    }
    let context = news_market_context(calendar::current_session());
    let daily = HistoricalBarsGateway::new()
        .required_daily_bars_async(target_code, DAILY_HISTORY_DAYS)
        .await
        .map_err(|error| error.to_string())?;
    let quote = if context == NewsMarketContext::Intraday {
        let code = target_code.to_owned();
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
    // as_of 必须在市场证据就绪之后取:日K/quote 批次的 observed_at 是获取
    // 完成时刻,若 as_of 早于它们会被 NewsMarketSnapshot 误判 "in the future"。
    let as_of = chrono::Utc::now();
    let market = NewsMarketSnapshot::try_from_admitted(target_code, context, as_of, daily, quote)
        .map_err(|error| error.to_string())?;
    // BR-249: 产业链上下文（chain_daily 最新涨停主线簇 + BR-170 板块归属）。
    // 读取失败降级为空上下文，不阻塞逐条评估。
    let chain_code = target_code.to_owned();
    let chain = tokio::task::spawn_blocking(move || load_chain_context(&chain_code))
        .await
        .map_err(|error| format!("chain context task failed: {error}"))?;
    NewsAiRequest::try_new_v3(fact, market, Vec::new(), identity, chain)
        .map_err(|error| error.to_string())
}

struct ProductionNewsAiDeliveryPort;

struct DurableNewsAiSinkAttempt {
    delivery: GovernedNewsAiDelivery,
    reservation: NewsAiDeliveryReservation,
}

#[async_trait]
impl super::notify::PhysicalSinkAttemptMarker for DurableNewsAiSinkAttempt {
    async fn mark_sink_started(&mut self) -> Result<(), String> {
        let delivery = self.delivery.clone();
        let reservation = self.reservation.clone();
        tokio::task::spawn_blocking(move || {
            stock_analysis::database::get_db().begin_news_ai_sink_attempt(&delivery, &reservation)
        })
        .await
        .map_err(|error| format!("sink-attempt audit task failed before sink: {error}"))?
        .map_err(|error| format!("sink-attempt audit failed before sink: {error}"))
    }
}

#[async_trait]
impl NewsAiGovernedDeliveryPort for ProductionNewsAiDeliveryPort {
    async fn reserve(
        &self,
        delivery: &GovernedNewsAiDelivery,
    ) -> Result<NewsAiReserveOutcome, String> {
        let delivery = delivery.clone();
        tokio::task::spawn_blocking(move || {
            stock_analysis::database::get_db().reserve_news_ai_delivery(&delivery)
        })
        .await
        .map_err(|error| format!("delivery reservation task failed: {error}"))?
        .map_err(|error| error.to_string())
    }

    async fn push(
        &self,
        delivery: &GovernedNewsAiDelivery,
        reservation: &NewsAiDeliveryReservation,
    ) -> NewsAiPhysicalPushOutcome {
        let text = delivery.render_card();
        let prepared = match super::notify::preflight_news_ai_analysis_v3(text, delivery) {
            Ok(prepared) => prepared,
            Err(super::notify::NewsAiPreflightRejection::Denied(reason)) => {
                return NewsAiPhysicalPushOutcome::Denied(reason);
            }
            Err(super::notify::NewsAiPreflightRejection::Error(reason)) => {
                return NewsAiPhysicalPushOutcome::SinkError(reason);
            }
        };

        let mut sink_attempt = DurableNewsAiSinkAttempt {
            delivery: delivery.clone(),
            reservation: reservation.clone(),
        };
        match super::notify::send_preflighted_news_ai_analysis_v3(prepared, &mut sink_attempt).await
        {
            // 2026-09-22: counted 准入拒绝 (预算满 / 冷却头 / launch gate) —
            // 卡片从未交给 sink, 按 `Denied` 收口 (不是 sink 故障)。
            // `deliver_governed_news_ai` 会走 `rollback` 把 BR-172 从
            // `Reserved` 收口到 `RolledBack`, 不留悬挂状态。
            super::notify::NewsAiNotifyOutcome::AdmissionDenied(reason) => {
                NewsAiPhysicalPushOutcome::Denied(reason)
            }
            // Preparation failed before the counted sink attempt. The core
            // rolls Reserved back for SinkError; reporting Denied here would
            // incorrectly count a capability failure as a policy rejection.
            super::notify::NewsAiNotifyOutcome::PreSinkError(reason) => {
                NewsAiPhysicalPushOutcome::SinkError(reason)
            }
            super::notify::NewsAiNotifyOutcome::Pushed { audit } => {
                let delivered = delivery.clone();
                let delivered_reservation = reservation.clone();
                let persisted_audit = audit.clone();
                match tokio::task::spawn_blocking(move || {
                    stock_analysis::database::get_db().record_news_ai_delivered(
                        &delivered,
                        &delivered_reservation,
                        &persisted_audit,
                    )
                })
                .await
                {
                    Ok(Ok(receipt)) => NewsAiPhysicalPushOutcome::Pushed(receipt),
                    Ok(Err(error)) => {
                        post_sink_failure(delivery, reservation, Some(audit), error.to_string())
                            .await
                    }
                    Err(error) => {
                        post_sink_failure(delivery, reservation, Some(audit), error.to_string())
                            .await
                    }
                }
            }
            super::notify::NewsAiNotifyOutcome::SinkError(reason) => {
                // counted 未给出 Delivered；BR-172 marker 尚未写入。由核心
                // state machine 从 Reserved rollback，后续是否能重试只由
                // counted 决策的权威状态决定。
                NewsAiPhysicalPushOutcome::SinkError(reason)
            }
            super::notify::NewsAiNotifyOutcome::CountedDeliveredMarkerFailed(reason) => {
                // counted 已确认 Delivered，但 BR-172 marker 尚未落库，当前
                // Reserved 无法合法写 post_sink_recovery。保留 Reserved，下一轮
                // 读取同一 counted 决策并只补 marker 与专属审计。
                NewsAiPhysicalPushOutcome::PostSinkFailure {
                    delivery_audit_event_id: None,
                    reason,
                }
            }
            super::notify::NewsAiNotifyOutcome::PostSinkAuditFailed { audit, reason } => {
                post_sink_failure(delivery, reservation, audit, reason).await
            }
        }
    }

    async fn commit(
        &self,
        delivery: &GovernedNewsAiDelivery,
        reservation: &NewsAiDeliveryReservation,
        delivery_audit: &NewsAiDeliveryAuditReceipt,
    ) -> Result<NewsAiPredictionLinkReceipt, String> {
        let delivery = delivery.clone();
        let reservation = reservation.clone();
        let delivery_audit = delivery_audit.clone();
        tokio::task::spawn_blocking(move || {
            stock_analysis::database::get_db().link_news_ai_prediction(
                &delivery,
                &reservation,
                &delivery_audit,
            )
        })
        .await
        .map_err(|error| format!("prediction link task failed: {error}"))?
        .map_err(|error| error.to_string())
    }

    async fn rollback(
        &self,
        delivery: &GovernedNewsAiDelivery,
        reservation: &NewsAiDeliveryReservation,
        reason: &str,
    ) -> Result<(), String> {
        // 2026-09-23 对抗性复审 F5a: 真实拒绝原因此前在 port 边界被丢弃, 账本
        // 只留下无语义常量。保留 `BR172_PRE_SINK_NOT_DELIVERED:` 前缀以维持可
        // 检索性, 后缀是深状态机传来的真实原因。
        // `rolled_back` 行的 reason 列有 CHECK 非空约束 (`database/news_ai.rs`),
        // 且 `validate_exact_text` 要求首尾无空白 —— 故先 trim, 为空则回退到
        // 无后缀常量: 任何分支都不会写出空 reason, 也不会写出带首尾空白的
        // reason 而让 rollback 失败。
        let trimmed = reason.trim();
        let rollback_reason = if trimmed.is_empty() {
            "BR172_PRE_SINK_NOT_DELIVERED".to_owned()
        } else {
            format!("BR172_PRE_SINK_NOT_DELIVERED:{trimmed}")
        };
        let delivery = delivery.clone();
        let reservation = reservation.clone();
        tokio::task::spawn_blocking(move || {
            stock_analysis::database::get_db().rollback_news_ai_delivery(
                &delivery,
                &reservation,
                &rollback_reason,
            )
        })
        .await
        .map_err(|error| format!("delivery rollback task failed: {error}"))?
        .map_err(|error| error.to_string())
    }
}

async fn post_sink_failure(
    delivery: &GovernedNewsAiDelivery,
    reservation: &NewsAiDeliveryReservation,
    audit: Option<stock_analysis::event::PersistedDeliveryAuditReceipt>,
    reason: String,
) -> NewsAiPhysicalPushOutcome {
    let recovery_delivery = delivery.clone();
    let recovery_reservation = reservation.clone();
    let recovery_audit = audit.clone();
    let recovery_reason = reason.clone();
    let recovery = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db().record_news_ai_post_sink_recovery(
            &recovery_delivery,
            &recovery_reservation,
            recovery_audit.as_ref(),
            &recovery_reason,
        )
    })
    .await;
    let reason = match recovery {
        Ok(Ok(())) => reason,
        Ok(Err(error)) => format!("{reason}; post-sink recovery audit failed: {error}"),
        Err(error) => format!("{reason}; post-sink recovery task failed: {error}"),
    };
    NewsAiPhysicalPushOutcome::PostSinkFailure {
        delivery_audit_event_id: audit.map(|receipt| receipt.envelope_id().to_owned()),
        reason,
    }
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
    use stock_analysis::data_gateway::{BatchEvidence, GlobalNewsRecord};
    use stock_analysis::market_domain::{ProviderId, SourceEvidence};

    fn scheduling_batch() -> AdmittedGlobalNewsBatch {
        AdmittedGlobalNewsBatch::from_parts(
            Vec::new(),
            BatchEvidence {
                provider: ProviderId::Cailianpress,
                source: "TEST_CODE_SCHEDULER".to_owned(),
                source_at: None,
                observed_at: "TEST_CODE_OBSERVATION".to_owned(),
                batch_id: "TEST_CODE_SCHEDULER_BATCH".to_owned(),
            },
        )
    }

    #[test]
    fn br172_manual_notice_is_confirmed_only_after_publication() {
        let events = std::cell::RefCell::new(Vec::new());
        publish_manual_review(
            "TEST_CODE_ASSESSMENT",
            42,
            17,
            |kind, identity, outcome, channel, rendered_len, latency_ms| {
                let event = stock_analysis::event::PushDeliveryEvent::new(
                    kind.to_owned(),
                    identity.map(str::to_owned),
                    outcome.to_owned(),
                    channel.to_owned(),
                    rendered_len,
                    latency_ms,
                );
                let envelope = stock_analysis::event::EventEnvelope::from_event(
                    &event,
                    "TEST_CODE_NOTICE".to_owned(),
                    "TEST_CODE_TRACE".to_owned(),
                    chrono::Local::now(),
                )
                .map_err(|error| error.to_string())?;
                assert_eq!(identity, Some("news-ai-recovery:TEST_CODE_ASSESSMENT:42"));
                assert_eq!(envelope.payload["kind"], "NewsAiRecoveryManualReview");
                assert_eq!(envelope.payload["outcome"], "Denied");
                assert_eq!(envelope.payload["channel"], "internal_audit");
                assert_eq!(envelope.payload["retryable"], false);
                assert_eq!(envelope.payload["rendered_len"], 17);
                events.borrow_mut().push("publish");
                Ok(())
            },
            || {
                events.borrow_mut().push("confirm");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*events.borrow(), vec!["publish", "confirm"]);

        assert!(publish_manual_review(
            "TEST_CODE_ASSESSMENT",
            42,
            17,
            |_, _, _, _, _, _| Err("TEST_CODE_AUDIT_UNAVAILABLE".to_owned()),
            || panic!("failed publication must not be acknowledged"),
        )
        .is_err());
        let publications = std::cell::Cell::new(0);
        assert!(publish_manual_review(
            "TEST_CODE_ASSESSMENT",
            42,
            17,
            |_, _, _, _, _, _| {
                publications.set(publications.get() + 1);
                Ok(())
            },
            || Err("TEST_CODE_ACK_WRITE_FAILED".to_owned()),
        )
        .is_err());
        publish_manual_review(
            "TEST_CODE_ASSESSMENT",
            42,
            17,
            |_, _, _, _, _, _| {
                publications.set(publications.get() + 1);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            publications.get(),
            2,
            "effect/ack crash window is explicitly at-least-once"
        );
    }

    #[tokio::test]
    async fn br172_scheduler_recovers_without_live_ingress() {
        for (selection, session, batches) in [
            (true, MarketSession::Morning, None),
            (true, MarketSession::Morning, Some(Vec::new())),
            (
                false,
                MarketSession::Morning,
                Some(vec![scheduling_batch()]),
            ),
            (true, MarketSession::Closed, Some(vec![scheduling_batch()])),
        ] {
            let recovered = Arc::new(AtomicUsize::new(0));
            let analysis = Arc::new(AtomicUsize::new(0));
            let observed_recovered = recovered.clone();
            let observed_analysis = analysis.clone();
            let task = schedule_news_ai_tick(
                &Arc::new(Semaphore::new(1)),
                selection,
                session,
                batches,
                move |limit| async move {
                    assert_eq!(limit, 5);
                    observed_recovered.fetch_add(1, Ordering::SeqCst);
                    1
                },
                move |_, _| async move {
                    observed_analysis.fetch_add(1, Ordering::SeqCst);
                },
            )
            .expect("persisted recovery must be scheduled without live input");
            task.await.unwrap();
            assert_eq!(recovered.load(Ordering::SeqCst), 1);
            assert_eq!(analysis.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn br172_scheduler_shares_one_worker_without_starving_live_or_recovery() {
        let permits = Arc::new(Semaphore::new(1));
        let release = Arc::new(tokio::sync::Notify::new());
        let began = Arc::new(tokio::sync::Notify::new());
        let recoveries = Arc::new(AtomicUsize::new(0));
        let analyses = Arc::new(AtomicUsize::new(0));
        let (wait, started, recovery_count, analysis_count) = (
            release.clone(),
            began.clone(),
            recoveries.clone(),
            analyses.clone(),
        );
        let first = schedule_news_ai_tick(
            &permits,
            true,
            MarketSession::Morning,
            Some(vec![scheduling_batch()]),
            move |limit| async move {
                assert_eq!(limit, 2, "live work retains a bounded share");
                recovery_count.fetch_add(1, Ordering::SeqCst);
                started.notify_one();
                wait.notified().await;
                2
            },
            move |batches, used| async move {
                assert_eq!(batches.len(), 1);
                let mut budget = CandidateVisitBudget::with_worked(20, 0, used);
                let mut worked = 0;
                while budget.next_index().is_some() {
                    worked += 1;
                    budget.record_work(true);
                }
                assert_eq!(worked, 3, "total recovery plus live work is capped at five");
                analysis_count.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
        began.notified().await;
        assert!(schedule_news_ai_tick(
            &permits,
            false,
            MarketSession::Closed,
            None,
            |_| async { panic!("concurrent recovery must not execute") },
            |_, _| async { panic!("concurrent analysis must not execute") },
        )
        .is_none());
        release.notify_one();
        first.await.unwrap();
        assert_eq!(recoveries.load(Ordering::SeqCst), 1);
        assert_eq!(analyses.load(Ordering::SeqCst), 1);
        let next = schedule_news_ai_tick(
            &permits,
            false,
            MarketSession::Closed,
            None,
            |_| async { 0 },
            |_, _| async { panic!("no live admission") },
        )
        .expect("the worker releases its permit after both branches");
        next.await.unwrap();
    }

    #[test]
    fn br172_sixth_admitted_news_item_remains_eligible_after_five_seen_items() {
        let observed = chrono::Utc::now();
        let records = (0..6)
            .map(|index| GlobalNewsRecord {
                item_id: format!("item-{index}"),
                title: format!("news {index}"),
                summary: None,
                content: None,
                publisher: "test source".to_owned(),
                canonical_url: format!("https://example.invalid/news/{index}"),
                published_at: observed,
                observed_at: observed,
                instruments: vec!["600519".to_owned()],
                topics: vec![],
                language: "zh".to_owned(),
                evidence: SourceEvidence::new(
                    ProviderId::Cailianpress,
                    observed.to_rfc3339(),
                    "batch-six",
                )
                .expect("source evidence")
                .with_source_at(observed.to_rfc3339())
                .unwrap(),
            })
            .collect();
        let batch = AdmittedGlobalNewsBatch::from_parts(
            records,
            BatchEvidence {
                provider: ProviderId::Cailianpress,
                source: "cls-v1".to_owned(),
                source_at: Some(observed.to_rfc3339()),
                observed_at: observed.to_rfc3339(),
                batch_id: "batch-six".to_owned(),
            },
        );

        let profile =
            NewsAiAnalysisProfile::for_configured_model("TEST_CODE_provider", "TEST_CODE_model")
                .unwrap();
        let candidates = exact_candidates(&[batch], &profile);
        assert_eq!(candidates.len(), 6);
        assert!(candidates
            .iter()
            .any(
                |candidate| candidate.batch.records()[candidate.record_index].item_id == "item-5"
            ));
    }

    fn v3_revision_batch() -> AdmittedGlobalNewsBatch {
        let observed = chrono::Utc::now();
        let records = ["TEST_CODE original content", "TEST_CODE revised content"]
            .into_iter()
            .map(|content| GlobalNewsRecord {
                item_id: "TEST_CODE_same_item".to_owned(),
                title: "TEST_CODE news".to_owned(),
                summary: None,
                content: Some(content.to_owned()),
                publisher: "TEST_CODE source".to_owned(),
                canonical_url: "https://example.invalid/item".to_owned(),
                published_at: observed,
                observed_at: observed,
                instruments: vec!["600519".to_owned()],
                topics: vec![],
                language: "zh".to_owned(),
                evidence: SourceEvidence::new(
                    ProviderId::Cailianpress,
                    observed.to_rfc3339(),
                    "TEST_CODE_batch",
                )
                .unwrap()
                .with_source_at(observed.to_rfc3339())
                .unwrap(),
            })
            .collect();
        AdmittedGlobalNewsBatch::from_parts(
            records,
            BatchEvidence {
                provider: ProviderId::Cailianpress,
                source: "cls-v1".to_owned(),
                source_at: Some(observed.to_rfc3339()),
                observed_at: observed.to_rfc3339(),
                batch_id: "TEST_CODE_batch".to_owned(),
            },
        )
    }

    #[test]
    fn v3_same_tick_keeps_distinct_text_revisions() {
        let batch = v3_revision_batch();
        let observed = batch.records()[0].observed_at;
        let profile =
            NewsAiAnalysisProfile::for_configured_model("TEST_CODE_provider", "TEST_CODE_model")
                .unwrap();
        assert_eq!(exact_candidates(&[batch.clone()], &profile).len(), 2);
        let mut repeated = batch.records()[0].clone();
        let next_observed = observed + chrono::Duration::seconds(1);
        repeated.observed_at = next_observed;
        repeated.evidence = SourceEvidence::new(
            ProviderId::Cailianpress,
            next_observed.to_rfc3339(),
            "TEST_CODE_batch_next",
        )
        .unwrap()
        .with_source_at(observed.to_rfc3339())
        .unwrap();
        let next_batch = AdmittedGlobalNewsBatch::from_parts(
            vec![repeated],
            BatchEvidence {
                provider: ProviderId::Cailianpress,
                source: "cls-v1".to_owned(),
                source_at: Some(observed.to_rfc3339()),
                observed_at: next_observed.to_rfc3339(),
                batch_id: "TEST_CODE_batch_next".to_owned(),
            },
        );
        assert_eq!(
            exact_candidates(&[batch, next_batch], &profile).len(),
            2,
            "same revision in a new batch is not a third candidate"
        );
    }

    #[test]
    fn br172_completed_prefix_does_not_starve_sixth_item_or_remove_work_limit() {
        let mut first_tick = CandidateVisitBudget::new(6, 0);
        let mut visited = Vec::new();
        while let Some(index) = first_tick.next_index() {
            visited.push(index);
            first_tick.record_work(index == 5);
        }
        assert_eq!(visited, vec![0, 1, 2, 3, 4, 5]);

        let mut all_new = CandidateVisitBudget::new(6, 0);
        let mut first_five = Vec::new();
        while let Some(index) = all_new.next_index() {
            first_five.push(index);
            all_new.record_work(true);
        }
        assert_eq!(first_five, vec![0, 1, 2, 3, 4]);
        let mut second_tick = CandidateVisitBudget::new(6, all_new.next_start());
        assert_eq!(second_tick.next_index(), Some(5));
    }

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
    fn governed_adapter_uses_exact_delivery_without_order_capability() {
        let source = include_str!("news_ai_shadow.rs");
        let candidate = source
            .split("async fn assess_candidate")
            .nth(1)
            .expect("candidate implementation")
            .split("struct ProductionNewsAiDeliveryPort")
            .next()
            .expect("candidate implementation boundary");
        assert!(candidate.contains("deliver_governed_news_ai"));
        let production = source
            .split("impl NewsAiGovernedDeliveryPort for ProductionNewsAiDeliveryPort")
            .nth(1)
            .expect("production port implementation")
            .split("const fn news_market_context")
            .next()
            .expect("production port boundary");
        assert!(production.contains("reserve_news_ai_delivery"));
        assert!(production.contains("link_news_ai_prediction"));
        for (prefix, suffix) in [("place_", "order"), ("Trading", "Bus")] {
            assert!(!production.contains(&format!("{prefix}{suffix}")));
        }
    }

    #[test]
    fn br172_governance_preflight_precedes_durable_sink_started_and_physical_send() {
        let source = include_str!("news_ai_shadow.rs");
        let push = source
            .find("async fn push(")
            .expect("production NewsAI delivery port push method");
        let production = source[push..]
            .split("async fn commit(")
            .next()
            .expect("production push method boundary");
        let preflight = production
            .find("preflight_news_ai_analysis_v3")
            .expect("typed NewsAI governance preflight");
        let physical_send = production
            .find("send_preflighted_news_ai_analysis_v3")
            .expect("preflight-authorized physical send");

        assert!(preflight < physical_send);
        assert!(production.contains("NewsAiPreflightRejection::Denied"));
        assert!(production.contains("NewsAiNotifyOutcome::PreSinkError"));
        // 准入拒绝从 Reserved rollback；此断言只覆盖该方法的映射，
        // counted 与 BR-172 的状态行为由隔离测试另行验证。
        assert!(production.contains("NewsAiNotifyOutcome::AdmissionDenied"));
        assert!(production.contains("NewsAiPhysicalPushOutcome::Denied"));

        let marker_impl = source
            .split("impl super::notify::PhysicalSinkAttemptMarker")
            .nth(1)
            .expect("durable NewsAI sink-attempt marker")
            .split("impl NewsAiGovernedDeliveryPort")
            .next()
            .expect("marker implementation boundary");
        assert!(marker_impl.contains("begin_news_ai_sink_attempt"));

        let notify = include_str!("notify.rs");
        let preflight_body = notify
            .split("pub(super) fn preflight_news_ai_analysis_v3")
            .nth(1)
            .expect("NewsAI typed preflight implementation")
            .split("pub(super) async fn send_preflighted_news_ai_analysis_v3")
            .next()
            .expect("NewsAI preflight boundary");
        for required_gate in [
            "news_ai_common_gate_status",
            "v14_gate_news_ai",
            "news_ai_governance_binding_mismatch",
        ] {
            assert!(
                preflight_body.contains(required_gate),
                "preflight must complete {required_gate} before SinkStarted"
            );
        }

        let transport = notify
            .split("async fn push_wechat_with_attempt_marker")
            .nth(1)
            .expect("attempt-aware physical transport")
            .split("pub(super) fn deliver_authoritative_blocking")
            .next()
            .expect("attempt-aware transport boundary");
        let daemon_send = transport
            .find("send_via_magiclaw_daemon(")
            .expect("daemon physical request");
        assert!(transport[..daemon_send]
            .rfind("mark_physical_sink_attempt")
            .is_some());

        for (helper, next_helper, physical_call) in [
            (
                "async fn push_feishu_http_with_client_and_attempt_marker",
                "async fn push_via_magiclaw_cli_with_attempt_marker",
                "client.post(url).json(&payload).send().await",
            ),
            (
                "async fn push_via_magiclaw_cli_with_attempt_marker",
                "struct CliDeliveryReceipt",
                "cmd.output().await",
            ),
        ] {
            let helper_body = notify
                .split(helper)
                .nth(1)
                .unwrap_or_else(|| panic!("attempt-aware helper {helper}"))
                .split(next_helper)
                .next()
                .expect("physical helper boundary");
            let call = helper_body
                .find(physical_call)
                .unwrap_or_else(|| panic!("physical transport call {physical_call}"));
            assert!(helper_body[..call]
                .rfind("mark_physical_sink_attempt")
                .is_some());
        }
    }

    #[test]
    fn br172_startup_banner_reports_the_actual_governed_producer_status() {
        let main = include_str!("main.rs");
        assert!(!main.contains("governed delivery remains disabled"));
        assert!(!main.contains("immutable assessment shadow enabled"));
        assert!(main.contains("news_ai_producer.log_startup_banner()"));
        // Scheduling behavior is exercised by br172_scheduler_* through the
        // production scheduling seam, not by matching the spelling of a call.
        assert!(!main.contains("news_ai_shadow::spawn_from_same_tick(&admitted)"));
    }

    #[test]
    fn br172_model_unavailable_still_schedules_audited_delivery_recovery() {
        let status = NewsAiRuntimeStatus::from_capabilities(
            NewAnalysisCapability::DisabledModelProviderUnavailable,
            GovernedDeliveryRecoveryCapability::Enabled,
        );

        assert_eq!(status.scheduling, ProducerSchedulingCapability::Enabled);
        assert_eq!(
            status.candidate_execution(true),
            CandidateExecution::DeferAuditedAssessment
        );
        assert_eq!(
            status.candidate_execution(false),
            CandidateExecution::RejectNewAnalysisUnavailable
        );
    }

    #[tokio::test]
    async fn br172_model_is_never_called_when_new_analysis_capability_is_disabled() {
        let status = NewsAiRuntimeStatus::from_capabilities(
            NewAnalysisCapability::DisabledModelProviderUnavailable,
            GovernedDeliveryRecoveryCapability::Enabled,
        );
        use stock_analysis::llm::{LlmError, LlmProvider, ReceiptBearingJson};
        struct CountingProvider(Arc<AtomicUsize>);
        #[async_trait]
        impl LlmProvider for CountingProvider {
            fn name(&self) -> &'static str {
                "TEST_CODE_disabled_provider"
            }
            fn model(&self) -> &str {
                "TEST_CODE_disabled_model"
            }
            async fn chat_json(&self, _: &str, _: &str) -> Result<serde_json::Value, LlmError> {
                panic!("disabled model")
            }
            async fn chat_json_with_receipt(
                &self,
                _: &str,
                _: &str,
            ) -> Result<ReceiptBearingJson, LlmError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                panic!("disabled receipt model")
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let analyzer = NewsAIAnalyzer::new(Arc::new(CountingProvider(calls.clone())));
        let profile = analyzer.identity_profile().unwrap();
        let candidates = exact_candidates(&[v3_revision_batch()], &profile);
        assert_eq!(candidates.len(), 2);
        let outcome = assess_candidate(Some(&analyzer), &status, &candidates[0]).await;
        assert!(outcome
            .err()
            .unwrap()
            .contains("model provider unavailable"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn br172_analysis_can_run_while_governed_delivery_recovery_is_unavailable() {
        let status = NewsAiRuntimeStatus::from_capabilities(
            NewAnalysisCapability::Enabled,
            GovernedDeliveryRecoveryCapability::DisabledLaunchStage,
        );

        assert_eq!(status.scheduling, ProducerSchedulingCapability::Enabled);
        assert_eq!(
            status.candidate_execution(false),
            CandidateExecution::CreateAssessmentOnly
        );
        assert_eq!(
            status.candidate_execution(true),
            CandidateExecution::DeferAuditedAssessment
        );
    }

    #[test]
    fn br172_manual_recovery_still_schedules_without_model_or_delivery_capability() {
        let status = NewsAiRuntimeStatus::from_capabilities(
            NewAnalysisCapability::DisabledModelProviderUnavailable,
            GovernedDeliveryRecoveryCapability::DisabledAuditHealth {
                reason_code: "audit_health_unverified".to_owned(),
            },
        );

        assert_eq!(status.scheduling, ProducerSchedulingCapability::Enabled);
        assert_eq!(
            status.candidate_execution(true),
            CandidateExecution::DeferAuditedAssessment
        );
        assert_eq!(
            status.candidate_execution(false),
            CandidateExecution::RejectNewAnalysisUnavailable
        );
    }
    #[test]
    fn news_n01_canonical_target_does_not_follow_result_or_budget_order() {
        let batch = v3_revision_batch();
        let profile = NewsAiAnalysisProfile::for_configured_model("TEST_CODE_provider","TEST_CODE_model").unwrap();
        let mut records=batch.records().to_vec();
        for record in &mut records { record.instruments=vec!["600600".into(),"600519".into(),"600600".into()]; }
        let ordered=AdmittedGlobalNewsBatch::from_parts(records.clone(),batch.evidence().clone());
        let first=exact_candidates(&[ordered],&profile);
        assert_eq!(first.len(),4); // two real revisions, two distinct source-bound targets each
        for candidate in &first {
            let fact=AdmittedNewsFact::from_admitted_global(&candidate.batch,candidate.record_index,&candidate.target_code).unwrap();
            assert_eq!(stock_analysis::monitor::news_ai::canonical_critical_target(&fact).as_deref(),Some("600519"));
        }
        for record in &mut records { record.instruments.reverse(); }
        let reversed=AdmittedGlobalNewsBatch::from_parts(records,batch.evidence().clone());
        let second=exact_candidates(&[reversed],&profile);
        assert_eq!(first.iter().map(|c|&c.key).collect::<Vec<_>>(),second.iter().map(|c|&c.key).collect::<Vec<_>>());
        let mut no_capacity=CandidateVisitBudget::with_worked(first.len(),0,MAX_ASSESSMENTS_PER_TICK);
        assert!(no_capacity.next_index().is_none());
        // Quota never creates a fallback designation for an unvisited target.
        assert!(first.iter().all(|c| c.target_code=="600519" || c.target_code=="600600"));
    }

    #[tokio::test]
    async fn news_global_n01_mixed_candidates_call_budget_and_slot_refusal() {
        use stock_analysis::llm::{LlmError,LlmProvider,ReceiptBearingJson};
        use std::task::Poll;
        struct Provider(Arc<AtomicUsize>);
        #[async_trait]
        impl LlmProvider for Provider {
            fn name(&self)->&'static str { "TEST_CODE_GLOBAL_PROVIDER" }
            fn model(&self)->&str { "TEST_CODE_GLOBAL_MODEL" }
            async fn chat_json(&self,_:&str,_:&str)->Result<serde_json::Value,LlmError> { panic!("receipt seam only") }
            async fn chat_json_with_receipt(&self,_:&str,_:&str)->Result<ReceiptBearingJson,LlmError> {
                self.0.fetch_add(1,Ordering::SeqCst);Err(LlmError::ReceiptUnavailable{provider:self.name().into(),model:self.model().into()})
            }
        }
        let calls=Arc::new(AtomicUsize::new(0));let analyzer=NewsAIAnalyzer::new(Arc::new(Provider(calls.clone())));let profile=analyzer.identity_profile().unwrap();
        let old=v3_revision_batch();let template=old.records()[0].clone();let mut records=old.records().to_vec();
        for i in 0..45 { let mut r=template.clone();r.item_id=format!("TEST_CODE_GLOBAL_{i}");r.title=format!("TEST_CODE macro {i}");r.instruments.clear();records.push(r); }
        let mut invalid=template.clone();invalid.item_id="TEST_CODE_INVALID_NONEMPTY".into();invalid.instruments=vec!["not-a-stock".into()];records.push(invalid);
        let batch=AdmittedGlobalNewsBatch::from_parts(records,old.evidence().clone());let candidates=mixed_candidates(&[batch.clone(),batch.clone()],&profile,&analyzer);
        assert_eq!(candidates.iter().filter(|c|matches!(c,MixedCandidate::Equity(_))).count(),2);
        assert_eq!(candidates.iter().filter(|c|matches!(c,MixedCandidate::Global{..})).count(),45);
        assert_eq!(candidates.iter().map(MixedCandidate::key).collect::<std::collections::BTreeSet<_>>().len(),47);
        let mut reversed=batch.records().to_vec();reversed.reverse();let backwards=AdmittedGlobalNewsBatch::from_parts(reversed,batch.evidence().clone());
        assert_eq!(candidates.iter().map(MixedCandidate::key).collect::<Vec<_>>(),mixed_candidates(&[backwards],&profile,&analyzer).iter().map(MixedCandidate::key).collect::<Vec<_>>());
        for recovered in [0,2,5] {
            let (tx,_rx)=stock_analysis::monitor::news_ai::critical_news_completion_channel();let before=calls.load(Ordering::SeqCst);let mut budget=CandidateVisitBudget::with_worked(candidates.len(),0,recovered);
            while let Some(index)=budget.next_index() {
                // Same cursor and shared work counter: ordinary and global visits consume one allowance.
                match &candidates[index] {
                    MixedCandidate::Global{fact,..}=> {
                        let mut slot=None;
                        assert!(analyzer.assess_global_critical_if_absent(fact.clone(),|_|async{Ok(false)},|r|async {
                            slot=Some(tx.reserve().await.map_err(|_|"closed".to_owned())?);Ok(r)
                        }).await.is_err());drop(slot);
                    }
                    MixedCandidate::Equity(_)=> { assert!(Provider(calls.clone()).chat_json_with_receipt("TEST_CODE ordinary system","TEST_CODE ordinary input").await.is_err()); }
                }
                budget.record_work(true); // real backend attempt failed; production counts failed attempts
            }
            assert_eq!(calls.load(Ordering::SeqCst)-before,5-recovered);assert_eq!(budget.worked,5);assert!(budget.inspected<=40);
        }
        let mut history_only=CandidateVisitBudget::new(candidates.len(),0);while history_only.next_index().is_some() { history_only.record_work(false); }assert_eq!(history_only.inspected,40);
        let (tx,mut rx)=stock_analysis::monitor::news_ai::critical_news_completion_channel();let mut held=Vec::new();for _ in 0..5 { held.push(tx.reserve().await.unwrap()); }
        let fact=candidates.iter().find_map(|c|if let MixedCandidate::Global{fact,..}=c {Some(fact.clone())}else{None}).unwrap();let before=calls.load(Ordering::SeqCst);
        let mut pending=Box::pin(analyzer.assess_global_critical_if_absent(fact.clone(),|_|async{Ok(false)},|r|async { let _slot=tx.reserve().await.map_err(|_|"closed".to_owned())?;Ok(r) }));
        std::future::poll_fn(|cx| { assert!(pending.as_mut().poll(cx).is_pending());Poll::Ready(()) }).await;drop(pending);assert_eq!(calls.load(Ordering::SeqCst),before);
        rx.close();drop(held);assert!(analyzer.assess_global_critical_if_absent(fact,|_|async{Ok(false)},|r|async { let _slot=tx.reserve().await.map_err(|_|"closed".to_owned())?;Ok(r) }).await.is_err());assert_eq!(calls.load(Ordering::SeqCst),before);
        // Library SQLite/receipt/event wholes prove successful token handoff; this bin whole does not simulate live delivery.
    }

}

async fn assess_critical_candidate(
    analyzer: Option<&NewsAIAnalyzer>, status: &NewsAiRuntimeStatus, candidate: &NewsAiCandidate,
    sender: &stock_analysis::monitor::news_ai::CriticalCompletionSender,
) -> Result<CandidateOutcome,String> {
    let execution = status.candidate_execution(false);
    if execution == CandidateExecution::RejectNewAnalysisUnavailable {
        return Err("receipt-bearing news_ai unavailable".into());
    }
    let analyzer = analyzer.ok_or_else(||"receipt-bearing news_ai unavailable".to_owned())?;
    let fact = AdmittedNewsFact::from_admitted_global(&candidate.batch,candidate.record_index,&candidate.target_code)
        .map_err(|e|e.to_string())?;
    if stock_analysis::monitor::news_ai::canonical_critical_target(&fact).as_deref() != Some(candidate.target_code.as_str()) {
        return Err("critical designation changed".into());
    }
    let profile = analyzer.critical_identity_profile().map_err(|e|e.to_string())?;
    let identity = NewsAiIdentityV3::from_fact(&fact,&profile).map_err(|e|e.to_string())?;
    let barrier_fact = fact.clone();
    let mut completion_slot = None;
    let slot_owner = &mut completion_slot;
    let result = analyzer.assess_critical_if_absent(identity,
        move |_| async move {
            tokio::task::spawn_blocking(move || stock_analysis::database::get_db().has_audited_news_base(&barrier_fact))
                .await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())
        },
        |identity| async move {
            let request = prepare_candidate_request(fact,&candidate.target_code,identity).await?;
            // Only a genuinely absent, prepared candidate reserves capacity;
            // a historical hit never waits for a slot or calls the model.
            *slot_owner = Some(sender.reserve().await.map_err(|_|"critical completion receiver closed before call".to_owned())?);
            Ok(request)
        }
    ).await?;
    let Some(result) = result else { return Ok(CandidateOutcome::AwaitingDeliveryRecovery{existing:true}); };
    let completion_slot = completion_slot.take().ok_or_else(||"critical completion slot missing".to_owned())?;
    // The real audit/mint and handoff share the same blocking owner. Dropping
    // an awaiting join handle cannot strand a freshly committed score outside it.
    let (audited, completion_outcome) = tokio::task::spawn_blocking(move || {
        let (audited, critical) = stock_analysis::database::get_db().append_audited_critical_news(result)
            .map_err(|e|e.to_string())?;
        Ok::<_,String>((audited,completion_slot.submit(critical)))
    }).await.map_err(|e|e.to_string())??;
    if completion_outcome == stock_analysis::monitor::news_ai::CriticalCompletionSubmitted::RetainedReceiverClosed {
        log::warn!("[NewsAI][BR244] audited score retained by completion owner after receiver closed; no historical remint");
    }
    match execution {
        CandidateExecution::CreateAssessmentAndDeliver => Ok(CandidateOutcome::Governed{existing:false,
            delivery:deliver_governed_news_ai(&audited,&ProductionNewsAiDeliveryPort).await}),
        CandidateExecution::CreateAssessmentOnly => Ok(CandidateOutcome::AwaitingDeliveryRecovery{existing:false}),
        _ => unreachable!("typed availability checked before acquisition"),
    }
}

enum MixedCandidate {
    Equity(NewsAiCandidate),
    Global { key:String,fact:stock_analysis::monitor::news_ai::GlobalCriticalFact },
}
impl MixedCandidate {
    fn key(&self)->&str { match self { Self::Equity(c)=>&c.key,Self::Global{key,..}=>key } }
}
fn mixed_candidates(batches:&[AdmittedGlobalNewsBatch],profile:&NewsAiAnalysisProfile,analyzer:&NewsAIAnalyzer)->Vec<MixedCandidate> {
    let mut all=BTreeMap::new();
    for c in exact_candidates(batches,profile) { all.insert(c.key.clone(),MixedCandidate::Equity(c)); }
    for batch in batches {
        for (index,record) in batch.records().iter().enumerate() {
            if !record.instruments.is_empty() { continue; } // Invalid/non-A-share is never Global.
            let fact=match stock_analysis::monitor::news_ai::GlobalCriticalFact::from_admitted(batch,index) {
                Ok(value)=>value,Err(error)=>{log::warn!("[NewsAI] global source refused: {error}");continue;}
            };
            let key=match analyzer.global_critical_identity(&fact) { Ok(id)=>id.digest(),Err(error)=>{log::warn!("[NewsAI] global profile refused: {error}");continue;} };
            all.entry(key.clone()).or_insert(MixedCandidate::Global{key,fact});
        }
    }all.into_values().collect()
}
async fn assess_global_critical_candidate(analyzer:Option<&NewsAIAnalyzer>,status:&NewsAiRuntimeStatus,
    fact:stock_analysis::monitor::news_ai::GlobalCriticalFact,sender:&stock_analysis::monitor::news_ai::CriticalCompletionSender)->Result<CandidateOutcome,String> {
    let execution=status.candidate_execution(false);
    if execution==CandidateExecution::RejectNewAnalysisUnavailable { return Err("receipt-bearing news_ai unavailable".into()); }
    let analyzer=analyzer.ok_or_else(||"receipt-bearing news_ai unavailable".to_owned())?;
    let mut slot=None;let owner=&mut slot;
    let result=analyzer.assess_global_critical_if_absent(fact,
        |fact|async move { tokio::task::spawn_blocking(move||stock_analysis::database::get_db().has_audited_global_news_base(&fact)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string()) },
        |request|async move {
            *owner=Some(sender.reserve().await.map_err(|_|"global completion receiver closed before model call".to_owned())?);
            Ok(request)
        }).await?;
    let Some(result)=result else { return Ok(CandidateOutcome::AwaitingDeliveryRecovery{existing:true}); };
    let slot=slot.take().ok_or_else(||"global completion slot missing".to_owned())?;
    let completion=tokio::task::spawn_blocking(move|| {
        let score=stock_analysis::database::get_db().append_audited_global_critical_news(result).map_err(|e|e.to_string())?;
        Ok::<_,String>(slot.submit(score))
    }).await.map_err(|e|e.to_string())??;
    if completion==stock_analysis::monitor::news_ai::CriticalCompletionSubmitted::RetainedReceiverClosed { log::warn!("[NewsAI] global audited score retained after receiver closure; no historical remint"); }
    // This purpose has no stock prediction/delivery record. Its sole sink is the existing NewsFlashGate.
    Ok(CandidateOutcome::AwaitingDeliveryRecovery{existing:false})
}
