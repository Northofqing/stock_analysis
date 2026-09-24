//! BR-172 NewsAI governed producer（默认启用，2026-08-11 起取消 env 开关）。
//!
//! This adapter consumes only source-bound batches from the same aggregator
//! tick, acquires audited market evidence, performs one receipt-bearing model
//! call, appends the immutable assessment audit and enters the exact-identity
//! governed delivery state machine. It has no trading capability.

use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use stock_analysis::calendar::{self, MarketSession};
use stock_analysis::data_gateway::instrument_identity::resolve_production_equity;
use stock_analysis::data_gateway::{HistoricalBarsGateway, MarketDataGateway};
use stock_analysis::llm::LlmRegistry;
use stock_analysis::monitor::news_ai::{
    deliver_governed_news_ai, AdmittedNewsFact, GovernedNewsAiDelivery, NewsAIAnalyzer,
    NewsAiChainContext, NewsAiDeliveryAuditReceipt, NewsAiDeliveryReservation,
    NewsAiGovernedDeliveryOutcome, NewsAiGovernedDeliveryPort, NewsAiPhysicalPushOutcome,
    NewsAiPredictionLinkReceipt, NewsAiRequest, NewsAiReserveOutcome, NewsMarketContext,
    NewsMarketSnapshot, NEWS_AI_ANALYSIS_VERSION,
};
use stock_analysis::news::aggregator::AdmittedGlobalNewsBatch;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_ASSESSMENTS_PER_TICK: usize = 5;
const MAX_CANDIDATE_INSPECTIONS_PER_TICK: usize = 40;
const DAILY_HISTORY_DAYS: usize = 60;

static NEWS_AI_BATCH_PERMIT: Lazy<Arc<Semaphore>> = Lazy::new(|| Arc::new(Semaphore::new(1)));
// NEWS_AI_BATCH_PERMIT keeps the worker single-flight, so its cursor can be
// advanced after each bounded scan without concurrent writers.
static NEXT_NEWS_AI_CANDIDATE: AtomicUsize = AtomicUsize::new(0);

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
    DisabledNoExecutableCapability,
}

impl std::fmt::Display for ProducerSchedulingCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enabled => formatter.write_str("enabled"),
            Self::DisabledTestProcessIsolation => {
                formatter.write_str("disabled:test_process_isolation")
            }
            Self::DisabledNoExecutableCapability => {
                formatter.write_str("disabled:no_executable_capability")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateExecution {
    DeliverAuditedAssessment,
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
            (NewAnalysisCapability::Enabled, _)
            | (_, GovernedDeliveryRecoveryCapability::Enabled) => {
                ProducerSchedulingCapability::Enabled
            }
            _ => ProducerSchedulingCapability::DisabledNoExecutableCapability,
        };
        Self {
            new_analysis,
            governed_delivery_recovery,
            scheduling,
        }
    }

    fn candidate_execution(&self, existing: bool) -> CandidateExecution {
        if existing {
            return match self.governed_delivery_recovery {
                GovernedDeliveryRecoveryCapability::Enabled => {
                    CandidateExecution::DeliverAuditedAssessment
                }
                _ => CandidateExecution::DeferAuditedAssessment,
            };
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
        }
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

    /// Schedule one bounded worker only when at least one executable capability
    /// exists. A model-less tick may still retry an already audited assessment;
    /// a delivery-less tick may still append a new receipt-bearing assessment.
    pub(super) fn schedule_from_same_tick(&self, batches: &[AdmittedGlobalNewsBatch]) {
        let status = self.runtime_status();
        match status.scheduling {
            ProducerSchedulingCapability::Enabled => {}
            disabled => {
                log::warn!(
                    "[NewsAI][BR-172] producer not scheduled producer_scheduling={disabled} new_analysis={} governed_delivery_recovery={}",
                    status.new_analysis,
                    status.governed_delivery_recovery
                );
                return;
            }
        }
        let permit = match NEWS_AI_BATCH_PERMIT.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                log::info!("[NewsAI][BR-172] skipped busy=true; no completion state written");
                return;
            }
        };
        let batches = batches.to_vec();
        let analyzer = self.analyzer.clone();
        tokio::spawn(async move {
            run_same_tick_batches(batches, analyzer, status, permit).await;
        });
    }
}

async fn run_same_tick_batches(
    batches: Vec<AdmittedGlobalNewsBatch>,
    analyzer: Option<NewsAIAnalyzer>,
    status: NewsAiRuntimeStatus,
    _permit: OwnedSemaphorePermit,
) {
    let candidates = exact_candidates(&batches);
    if candidates.is_empty() {
        log::debug!("[NewsAI][BR-172] no exact source-bound equity target");
        return;
    }

    let mut pushed = 0_usize;
    let mut link_recovered = 0_usize;
    let mut retained = 0_usize;
    let mut deferred = 0_usize;
    let mut deduped = 0_usize;
    // 2026-09-22 NewsAI counted 准入接线后: counted 准入拒绝 (PerTicket 1200s
    // 冷却窗内同票) 是**设计内的正常丢弃**, 不是故障。单开计数避免污染
    // failed= 与 WARN 级别 (真实故障 ReserveFailed/RollbackFailed/
    // PostSinkCommitFailed 才能与"按设计丢弃"区分)。
    let mut denied = 0_usize;
    let mut failed = 0_usize;
    let mut budget = CandidateVisitBudget::new(
        candidates.len(),
        NEXT_NEWS_AI_CANDIDATE.load(Ordering::Relaxed),
    );
    while let Some(index) = budget.next_index() {
        let candidate = &candidates[index];
        let did_work = match assess_candidate(analyzer.as_ref(), &status, candidate).await {
            Ok(CandidateOutcome::Governed { existing, delivery }) => {
                let did_work = !matches!(&delivery, NewsAiGovernedDeliveryOutcome::Deduped { .. });
                match delivery {
                    NewsAiGovernedDeliveryOutcome::Pushed { .. } => pushed += 1,
                    NewsAiGovernedDeliveryOutcome::PredictionLinkRecovered { .. } => {
                        link_recovered += 1;
                    }
                    NewsAiGovernedDeliveryOutcome::RetainedNoDelivery { .. } => retained += 1,
                    NewsAiGovernedDeliveryOutcome::Deduped { .. } => deduped += 1,
                    // counted 准入拒绝: 设计内丢弃, info 级, 不计入 failed。
                    NewsAiGovernedDeliveryOutcome::Denied { reason, .. } => {
                        denied += 1;
                        log::info!(
                        "[NewsAI][BR-172] counted admission denied key={} existing={} reason={reason}",
                        candidate.key,
                        existing
                    );
                    }
                    other => {
                        failed += 1;
                        log::warn!(
                        "[NewsAI][BR-172] governed delivery incomplete key={} existing={} outcome={other:?}",
                        candidate.key,
                        existing
                    );
                    }
                }
                did_work
            }
            Ok(CandidateOutcome::AwaitingDeliveryRecovery { existing }) => {
                deferred += 1;
                log::warn!(
                    "[NewsAI][BR-172] assessment retained for governed delivery recovery key={} existing={} governed_delivery_recovery={}",
                    candidate.key,
                    existing,
                    status.governed_delivery_recovery
                );
                !existing
            }
            Ok(CandidateOutcome::TerminalDenied) => {
                denied += 1;
                false
            }
            Err(error) => {
                failed += 1;
                log::warn!(
                    "[NewsAI][BR-172] candidate failed key={} error={error}",
                    candidate.key
                );
                true
            }
        };
        budget.record_work(did_work);
    }
    NEXT_NEWS_AI_CANDIDATE.store(budget.next_start(), Ordering::Relaxed);
    log::info!(
        "[NewsAI][BR-172] completed pushed={} link_recovered={} retained_neutral={} deferred_delivery={} deduped={} admission_denied={} failed={}",
        pushed,
        link_recovered,
        retained,
        deferred,
        deduped,
        denied,
        failed
    );
}

#[derive(Debug, Clone)]
struct NewsAiCandidate {
    key: String,
    batch: AdmittedGlobalNewsBatch,
    record_index: usize,
    target_code: String,
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
        Self {
            total,
            start: if total == 0 { 0 } else { start % total },
            inspected: 0,
            worked: 0,
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

fn exact_candidates(batches: &[AdmittedGlobalNewsBatch]) -> Vec<NewsAiCandidate> {
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
                    .or_insert_with(|| NewsAiCandidate {
                        key,
                        batch: batch.clone(),
                        record_index,
                        target_code,
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
    let mut fact = AdmittedNewsFact::from_admitted_global(
        &candidate.batch,
        candidate.record_index,
        &candidate.target_code,
    )
    .map_err(|error| error.to_string())?;
    // This indexed read is a scheduling hint for a counted decision that can
    // never be reopened under the frozen identity. Every nonterminal state
    // still takes the fully validated BR-172 audit path below.
    let terminal_fact = fact.clone();
    let terminal_denial = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db()
            .is_news_ai_terminal_denial_for_fact(&terminal_fact, NEWS_AI_ANALYSIS_VERSION)
    })
    .await
    .map_err(|error| format!("terminal NewsAI decision lookup task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    if terminal_denial {
        return Ok(CandidateOutcome::TerminalDenied);
    }
    // BR-250: 注入证券名称供卡片渲染; 解析失败保持 None, 不阻塞评估。
    let name_code = candidate.target_code.clone();
    let name = resolve_target_name(&name_code).await;
    if let Some(name) = name {
        fact = fact.with_target_name(name);
    }

    let identity_fact = fact.clone();
    let existing = tokio::task::spawn_blocking(move || {
        stock_analysis::database::get_db()
            .load_audited_news_ai_assessment_for_fact(&identity_fact, NEWS_AI_ANALYSIS_VERSION)
    })
    .await
    .map_err(|error| format!("assessment identity lookup task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    let execution = status.candidate_execution(existing.is_some());
    match execution {
        CandidateExecution::DeliverAuditedAssessment => {
            let audited = existing.expect("typed execution requires an audited assessment");
            let delivery = deliver_governed_news_ai(&audited, &ProductionNewsAiDeliveryPort).await;
            return Ok(CandidateOutcome::Governed {
                existing: true,
                delivery,
            });
        }
        CandidateExecution::DeferAuditedAssessment => {
            return Ok(CandidateOutcome::AwaitingDeliveryRecovery { existing: true });
        }
        CandidateExecution::RejectNewAnalysisUnavailable => {
            return Err(
                "receipt-bearing news_ai model provider unavailable; assessment not written"
                    .to_owned(),
            );
        }
        CandidateExecution::CreateAssessmentAndDeliver
        | CandidateExecution::CreateAssessmentOnly => {}
    }

    let analyzer = analyzer.ok_or_else(|| {
        "receipt-bearing news_ai model provider unavailable; assessment not written".to_owned()
    })?;

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
    // as_of 必须在市场证据就绪之后取:日K/quote 批次的 observed_at 是获取
    // 完成时刻,若 as_of 早于它们会被 NewsMarketSnapshot 误判 "in the future"。
    let as_of = chrono::Utc::now();
    let market =
        NewsMarketSnapshot::try_from_admitted(&candidate.target_code, context, as_of, daily, quote)
            .map_err(|error| error.to_string())?;
    // BR-249: 产业链上下文（chain_daily 最新涨停主线簇 + BR-170 板块归属）。
    // 读取失败降级为空上下文，不阻塞逐条评估。
    let chain_code = candidate.target_code.clone();
    let chain = tokio::task::spawn_blocking(move || load_chain_context(&chain_code))
        .await
        .map_err(|error| format!("chain context task failed: {error}"))?;
    let request = NewsAiRequest::try_new(fact, market, Vec::new(), NEWS_AI_ANALYSIS_VERSION, chain)
        .map_err(|error| error.to_string())?;
    let assessment = analyzer
        .assess(&request)
        .await
        .map_err(|error| error.to_string())?;
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
        CandidateExecution::DeliverAuditedAssessment
        | CandidateExecution::DeferAuditedAssessment
        | CandidateExecution::RejectNewAnalysisUnavailable => {
            unreachable!("typed execution already returned before model acquisition")
        }
    }
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
                    ProviderId::Eastmoney,
                    observed.to_rfc3339(),
                    "batch-six",
                )
                .expect("source evidence"),
            })
            .collect();
        let batch = AdmittedGlobalNewsBatch::from_parts(
            records,
            BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "test source".to_owned(),
                source_at: None,
                observed_at: observed.to_rfc3339(),
                batch_id: "batch-six".to_owned(),
            },
        );

        let candidates = exact_candidates(&[batch]);
        assert_eq!(candidates.len(), 6);
        assert!(candidates
            .iter()
            .any(|candidate| candidate.key.contains("item-5")));
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
        assert!(main.contains("news_ai_producer.schedule_from_same_tick(&admitted)"));
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
            CandidateExecution::DeliverAuditedAssessment
        );
        assert_eq!(
            status.candidate_execution(false),
            CandidateExecution::RejectNewAnalysisUnavailable
        );
    }

    #[test]
    fn br172_model_is_never_called_when_new_analysis_capability_is_disabled() {
        let status = NewsAiRuntimeStatus::from_capabilities(
            NewAnalysisCapability::DisabledModelProviderUnavailable,
            GovernedDeliveryRecoveryCapability::Enabled,
        );
        let source = include_str!("news_ai_shadow.rs");
        let runner = source
            .split("async fn assess_candidate(")
            .nth(1)
            .expect("NewsAI candidate runner")
            .split("struct ProductionNewsAiDeliveryPort")
            .next()
            .expect("candidate runner boundary");
        let execution = runner
            .find("candidate_execution(existing.is_some())")
            .expect("typed candidate execution decision");
        let model_call = runner
            .find(".assess(&request)")
            .expect("receipt-bearing model call");

        assert_eq!(
            status.candidate_execution(false),
            CandidateExecution::RejectNewAnalysisUnavailable
        );
        assert!(execution < model_call);
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
    fn br172_producer_does_not_schedule_when_no_executable_capability_exists() {
        let status = NewsAiRuntimeStatus::from_capabilities(
            NewAnalysisCapability::DisabledModelProviderUnavailable,
            GovernedDeliveryRecoveryCapability::DisabledAuditHealth {
                reason_code: "audit_health_unverified".to_owned(),
            },
        );

        assert_eq!(
            status.scheduling,
            ProducerSchedulingCapability::DisabledNoExecutableCapability
        );
        assert_eq!(
            status.candidate_execution(true),
            CandidateExecution::DeferAuditedAssessment
        );
        assert_eq!(
            status.candidate_execution(false),
            CandidateExecution::RejectNewAnalysisUnavailable
        );
    }
}
