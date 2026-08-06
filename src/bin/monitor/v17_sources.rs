//! Registered business rules: BR-112, BR-137, BR-138, BR-210.
//! v17.7 Task 5: Monitor-only source-to-push adapter
//!
//! Consumes `NormalizedSourceEvent` from the news aggregator and dispatches
//! exactly one `push_governor_v3` call per event. No retry, no fallback PushKind.
//!
//! v17.7 Task 7: Adds bounded polling for earnings and analyst data on the watchlist.

use crate::notify::{self, PushKind, PushOutcome};
use chrono::{DateTime, Local};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use stock_analysis::company_financials;
use stock_analysis::data_provider::consensus;
use stock_analysis::monitor::event_bus::MonitorEvent;
use stock_analysis::monitor::news_monitor::{DedupClaim, NewsMonitor};
use stock_analysis::news::aggregator::analyst_state::{
    AnalystKey, AnalystObservation, AnalystStateStore,
};
use stock_analysis::news::aggregator::classifier::{
    classify_announcement_with_provenance, classify_earnings,
    validate_announcement_source_fact_with_provenance, EarningsClassification, EarningsConfig,
    EarningsKind,
};
use stock_analysis::news::aggregator::source_event::SourceBatchEvidence;
use stock_analysis::news::aggregator::{
    NormalizedSourceError, NormalizedSourceEvent, SourcePushKind,
};

#[derive(Debug, Clone)]
pub struct EvidenceBackedConsensus {
    pub data: consensus::ConsensusData,
    pub evidence: SourceBatchEvidence,
}

fn source_batch_from_gateway(
    evidence: stock_analysis::data_gateway::BatchEvidence,
    content_sha256: String,
) -> anyhow::Result<SourceBatchEvidence> {
    SourceBatchEvidence::new(
        evidence.provider,
        evidence.source,
        evidence.source_at,
        evidence.observed_at,
        evidence.batch_id,
        content_sha256,
    )
    .map_err(anyhow::Error::from)
}

fn financial_batch_evidence(
    financials: &company_financials::Financials,
) -> anyhow::Result<SourceBatchEvidence> {
    let evidence = financials.require_projection_evidence()?;
    SourceBatchEvidence::new(
        evidence.provider,
        evidence.source.clone(),
        evidence.source_at.clone(),
        evidence.observed_at.clone(),
        evidence.batch_id.clone(),
        evidence.content_sha256.clone(),
    )
    .map_err(anyhow::Error::from)
}

fn consensus_content_sha256(data: &consensus::ConsensusData) -> anyhow::Result<String> {
    let ratings = data
        .rating_distribution
        .iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let reports = data
        .recent_reports
        .iter()
        .map(|report| {
            serde_json::json!({
                "title": report.title,
                "organization": report.org_name,
                "published_on": report.publish_date,
                "rating": report.rating,
            })
        })
        .collect::<Vec<_>>();
    let canonical = serde_json::to_vec(&serde_json::json!({
        "report_count": data.report_count,
        "broker_count": data.broker_count,
        "eps_this_year_avg": data.eps_this_year_avg,
        "eps_next_year_avg": data.eps_next_year_avg,
        "eps_next2_year_avg": data.eps_next2_year_avg,
        "rating_distribution": ratings,
        "target_price_high_avg": data.target_price_high_avg,
        "target_price_low_avg": data.target_price_low_avg,
        "latest_report_date": data.latest_report_date,
        "recent_reports": reports,
    }))
    .map_err(|error| anyhow::anyhow!("BR-159 consensus content serialization failed: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.consensus_projection_content.v1\0");
    hasher.update(canonical);
    Ok(format!("{:x}", hasher.finalize()))
}

fn project_consensus_batch(
    batch: stock_analysis::data_gateway::GatewayBatch<consensus::ConsensusData>,
) -> anyhow::Result<EvidenceBackedConsensus> {
    match batch {
        stock_analysis::data_gateway::GatewayBatch::Available {
            mut records,
            evidence,
        } if records.len() == 1 => {
            let data = records.remove(0);
            let content_sha256 = consensus_content_sha256(&data)?;
            let evidence = source_batch_from_gateway(evidence, content_sha256)?;
            Ok(EvidenceBackedConsensus { data, evidence })
        }
        stock_analysis::data_gateway::GatewayBatch::Available { records, .. } => {
            anyhow::bail!(
                "BR-164 consensus gateway cardinality invalid: expected=1 actual={}",
                records.len()
            )
        }
        stock_analysis::data_gateway::GatewayBatch::VerifiedEmpty(_) => {
            anyhow::bail!("BR-164 consensus gateway returned verified empty")
        }
    }
}

fn latest_batch_observed_at(
    batches: &[SourceBatchEvidence],
) -> Result<DateTime<Local>, NormalizedSourceError> {
    batches
        .iter()
        .map(|evidence| {
            stock_analysis::data_gateway::parse_evidence_instant(
                "v17-source-batch-observed-at",
                evidence.provider,
                "observed_at",
                &evidence.observed_at,
            )
            .map(|value| value.with_timezone(&Local))
            .map_err(|_| NormalizedSourceError::InvalidBatchEvidence)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or(NormalizedSourceError::MissingBatchEvidence)
}

/// Bounded state map for deduping OrderUpdate events.
/// Tracks (action, shares) per code; only emits if the tuple changes.
#[derive(Debug, Default)]
pub struct MarketActionState {
    seen: HashMap<String, (String, u64)>, // code → (action, shares)
}

impl MarketActionState {
    /// Returns true if this is a new state (code/action/shares combination is different
    /// from the last time we saw this code), false if unchanged.
    pub fn accept(&mut self, event: &MonitorEvent) -> bool {
        if let MonitorEvent::OrderUpdate {
            code,
            action,
            shares,
        } = event
        {
            let prev = self.seen.get(code).cloned();
            let is_new = prev.as_ref() != Some(&(action.clone(), *shares));
            self.seen.insert(code.clone(), (action.clone(), *shares));
            is_new
        } else {
            false
        }
    }
}

/// Build a MarketActionAlert NormalizedSourceEvent from an OrderUpdate MonitorEvent.
pub fn normalize_market_action(event: &MonitorEvent) -> Option<NormalizedSourceEvent> {
    if let MonitorEvent::OrderUpdate {
        code,
        action,
        shares,
    } = event
    {
        NormalizedSourceEvent::new(
            SourcePushKind::MarketActionAlert,
            format!("order:{}:{}:{}", code, action, shares),
            Some(code.clone()),
            format!("OrderUpdate: {} {} shares", action, shares),
            format!("Order action {} for {}", action, code),
            stock_analysis::signal::market_event::Direction::Neutral,
            70,
            90,
            Local::now(),
            None,
            false,
            "monitor".into(),
            None,
        )
        .ok()
    } else {
        None
    }
}

/// Handle a MonitorEvent: dedup via MarketActionState, then push via push_normalized_event.
pub async fn handle_monitor_event(
    event: &MonitorEvent,
    state: &Mutex<MarketActionState>,
) -> Option<PushAttempt> {
    let is_new = {
        let mut s = state.lock().ok()?;
        s.accept(event)
    }; // MutexGuard dropped here, before any await
    if !is_new {
        return None; // unchanged, skip
    }
    let normalized = normalize_market_action(event)?;
    Some(push_normalized_event(normalized).await)
}

#[derive(Debug, Clone)]
pub struct PushAttempt {
    pub kind: PushKind,
    pub code: Option<String>,
    pub outcome: PushOutcome,
    pub rendered_len: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SourcePollReport {
    pub attempted: usize,
    pub classified: usize,
    pub pushed: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnouncementDisposition {
    Pushed,
    FilteredClassification,
    FilteredLifecycle,
    FilteredAudience,
    FilteredDuplicate,
    Failed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AnnouncementDispositionCounts {
    pub pushed: usize,
    pub filtered_classification: usize,
    pub filtered_lifecycle: usize,
    pub filtered_audience: usize,
    pub filtered_duplicate: usize,
    pub failed: usize,
}

#[derive(Debug, Default)]
pub struct AnnouncementSourceRouteReport {
    pub source: SourcePollReport,
    /// BR-137/BR-138: one disposition for every provider input, in the same
    /// order. Identity failures and classification failures remain explicit
    /// `Failed` entries; no input may fall through to legacy delivery.
    input_dispositions: Vec<AnnouncementDisposition>,
}

impl AnnouncementSourceRouteReport {
    pub fn disposition_for_input(&self, input_index: usize) -> Option<AnnouncementDisposition> {
        self.input_dispositions.get(input_index).copied()
    }

    pub fn disposition_counts(&self) -> AnnouncementDispositionCounts {
        let mut counts = AnnouncementDispositionCounts::default();
        for disposition in &self.input_dispositions {
            match disposition {
                AnnouncementDisposition::Pushed => counts.pushed += 1,
                AnnouncementDisposition::FilteredClassification => {
                    counts.filtered_classification += 1;
                }
                AnnouncementDisposition::FilteredLifecycle => counts.filtered_lifecycle += 1,
                AnnouncementDisposition::FilteredAudience => counts.filtered_audience += 1,
                AnnouncementDisposition::FilteredDuplicate => counts.filtered_duplicate += 1,
                AnnouncementDisposition::Failed => counts.failed += 1,
            }
        }
        counts
    }

    #[cfg(test)]
    pub fn with_dispositions_for_test(dispositions: Vec<AnnouncementDisposition>) -> Self {
        Self {
            source: SourcePollReport::default(),
            input_dispositions: dispositions,
        }
    }
}

fn record_source_attempt(report: &mut SourcePollReport, outcome: &PushOutcome) {
    match outcome {
        PushOutcome::Pushed => report.pushed += 1,
        _ => report.failed += 1,
    }
}

/// Route complete real-provider announcements one by one through the sole
/// normalized source-fact path. Successfully classified items are owned by
/// this path even when governance/sink fails, so legacy delivery cannot bypass
/// the explicit outcome. The next provider poll remains the retry boundary.
#[cfg(test)]
pub async fn route_announcements(
    announcements: &[stock_analysis::announcement::Announcement],
    eligible_codes: &HashSet<String>,
) -> AnnouncementSourceRouteReport {
    route_announcements_with_provenance(
        announcements,
        eligible_codes,
        Local::now(),
        "cninfo-market",
    )
    .await
}

pub async fn route_announcement_batch(
    batch: &stock_analysis::announcement::AnnouncementBatch,
    eligible_codes: &HashSet<String>,
) -> AnnouncementSourceRouteReport {
    let observed_at = match stock_analysis::data_gateway::parse_evidence_instant(
        "R-08-announcements",
        batch.evidence.provider,
        "observed_at",
        &batch.evidence.observed_at,
    ) {
        Ok(value) => value.with_timezone(&Local),
        Err(error) => {
            log::error!(
                "[v17.7][BR-159][BR-168][BR-210] announcement batch observed_at invalid: {error}"
            );
            return AnnouncementSourceRouteReport {
                source: SourcePollReport {
                    attempted: batch.announcements.len(),
                    // A malformed batch envelope remains machine-visible even
                    // when it contains no rows. Per-input dispositions still
                    // remain exactly aligned with the input cardinality.
                    failed: batch.announcements.len().max(1),
                    ..SourcePollReport::default()
                },
                input_dispositions: vec![
                    AnnouncementDisposition::Failed;
                    batch.announcements.len()
                ],
            };
        }
    };
    route_announcements_with_provenance(
        &batch.announcements,
        eligible_codes,
        observed_at,
        &batch.evidence.source,
    )
    .await
}

async fn route_announcements_with_provenance(
    announcements: &[stock_analysis::announcement::Announcement],
    eligible_codes: &HashSet<String>,
    observed_at: chrono::DateTime<Local>,
    source: &str,
) -> AnnouncementSourceRouteReport {
    let mut routed = AnnouncementSourceRouteReport::default();
    for announcement in announcements {
        routed.source.attempted += 1;
        let Some(external_id) = announcement
            .external_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            routed.source.failed += 1;
            routed
                .input_dispositions
                .push(AnnouncementDisposition::Failed);
            log::warn!(
                "[v17.7][BR-137] announcement source route skipped: missing provider external_id"
            );
            continue;
        };
        // BR-137/BR-138: no handled/filter outcome may hide malformed, stale,
        // or future provider evidence. This validation intentionally precedes
        // both the lifecycle and immutable-keyword dispositions.
        if let Err(error) =
            validate_announcement_source_fact_with_provenance(announcement, observed_at, source)
        {
            routed.source.failed += 1;
            routed
                .input_dispositions
                .push(AnnouncementDisposition::Failed);
            log::warn!(
                "[v17.7][BR-137] announcement source fact rejected before filtering: reason={error}"
            );
            continue;
        }
        if !stock_analysis::announcement::announcement_title_is_immediately_actionable(
            &announcement.title,
        ) {
            routed.source.classified += 1;
            routed.source.skipped += 1;
            routed
                .input_dispositions
                .push(AnnouncementDisposition::FilteredLifecycle);

            continue;
        }
        // BR-138: ordinary announcements that did not match the immutable
        // notification keyword snapshot remain owned and handled without
        // polluting lifecycle audit counts.
        if matches!(
            announcement.level,
            stock_analysis::announcement::AnnLevel::Skip
        ) {
            routed.source.classified += 1;
            routed.source.skipped += 1;
            routed
                .input_dispositions
                .push(AnnouncementDisposition::FilteredClassification);

            continue;
        }
        let event = match classify_announcement_with_provenance(announcement, observed_at, source) {
            Ok(event) => event,
            Err(error) => {
                routed.source.failed += 1;
                routed
                    .input_dispositions
                    .push(AnnouncementDisposition::Failed);
                log::warn!("[v17.7][BR-137] announcement classification rejected: reason={error}");
                continue;
            }
        };
        routed.source.classified += 1;
        if !eligible_codes.contains(&announcement.code) {
            routed.source.skipped += 1;
            routed
                .input_dispositions
                .push(AnnouncementDisposition::FilteredAudience);

            continue;
        }
        // BR-224: 同一公告一天内只投递一次。上游 `market_announcements` 每轮
        // (NEWS_POLL_INTERVAL 默认 120s) 都返回当日全量公告；L4 进程内 source-fact
        // 冷却是主去重权威，本持久认领提供跨重启的同日纵深防御。
        // §3 测试隔离：持久认领是生产运行时关注点，测试只覆盖 L4 内存层，
        // 否则残留键会让测试跨运行非幂等。
        let dedup_key = format!(
            "annroute:{}:{source}:{external_id}",
            observed_at.date_naive()
        );
        let production_dedup = stock_analysis::risk::env_guard::current_env()
            != stock_analysis::risk::env_guard::TradingEnv::Test;
        match if production_dedup {
            NewsMonitor::claim_dedup_key(&dedup_key)
        } else {
            DedupClaim::Claimed
        } {
            DedupClaim::Claimed => {}
            DedupClaim::AlreadyClaimed => {
                routed.source.skipped += 1;
                routed
                    .input_dispositions
                    .push(AnnouncementDisposition::FilteredDuplicate);
                continue;
            }
            DedupClaim::Unavailable => {
                // L4 进程内冷却仍是主去重权威（BR-137 source-fact 键）；持久层
                // 只提供跨重启的纵深防御。存储不可用时不得静默吞掉真实公告，
                // 但必须以 error 级显式可见。
                log::error!(
                    "[v17.7][BR-224] durable dedup store unavailable; falling back to L4 in-memory cooldown only"
                );
            }
        }
        let attempt = push_normalized_event(event).await;
        if production_dedup && attempt.outcome != PushOutcome::Pushed {
            // 保持「下一轮 provider 轮询即重试边界」的既有契约：投递未成功时
            // 释放认领，否则该公告会被永久吞掉。释放失败必须显式可见。
            if !NewsMonitor::release_dedup_key(&dedup_key) {
                log::error!(
                    "[v17.7][BR-224] dedup claim release failed after unsuccessful delivery; announcement will not retry today"
                );
            }
        }
        record_source_attempt(&mut routed.source, &attempt.outcome);
        routed
            .input_dispositions
            .push(if attempt.outcome == PushOutcome::Pushed {
                AnnouncementDisposition::Pushed
            } else {
                AnnouncementDisposition::Failed
            });
        log::info!(
            "[v17.7][BR-137] announcement normalized outcome={:?}",
            attempt.outcome
        );
    }
    debug_assert_eq!(
        routed.input_dispositions.len(),
        announcements.len(),
        "BR-137 every provider input must have one disposition"
    );
    // BR-226: 每轮聚合摘要, 替代逐条 INFO (噪音治理)
    {
        let mut lifecycle = 0usize;
        let mut classification = 0usize;
        let mut audience = 0usize;
        let mut duplicate = 0usize;
        let mut failed = 0usize;
        let mut pushed = 0usize;
        for disposition in &routed.input_dispositions {
            match disposition {
                AnnouncementDisposition::FilteredLifecycle => lifecycle += 1,
                AnnouncementDisposition::FilteredClassification => classification += 1,
                AnnouncementDisposition::FilteredAudience => audience += 1,
                AnnouncementDisposition::FilteredDuplicate => duplicate += 1,
                AnnouncementDisposition::Failed => failed += 1,
                AnnouncementDisposition::Pushed => pushed += 1,
            }
        }
        log::info!(
            "[v17.7][BR-226] 公告过滤摘要: 共 {} 条 | 生命周期 {} / 分类跳过 {} / 范围外 {} / 重复 {} / 失败 {} / 推送 {}",
            announcements.len(),
            lifecycle,
            classification,
            audience,
            duplicate,
            failed,
            pushed
        );
    }
    routed
}

/// Maps SourcePushKind 1:1 to the corresponding PushKind variant.
fn source_push_kind_to_push_kind(kind: SourcePushKind) -> PushKind {
    match kind {
        SourcePushKind::Announcement => PushKind::Announcement,
        SourcePushKind::PolicyHit => PushKind::PolicyHit,
        SourcePushKind::EarningsBeat => PushKind::EarningsBeat,
        SourcePushKind::EarningsMiss => PushKind::EarningsMiss,
        SourcePushKind::AnalystUpgrade => PushKind::AnalystUpgrade,
        SourcePushKind::MarketActionAlert => PushKind::MarketActionAlert,
    }
}

fn source_presentation_tuple(kind: SourcePushKind) -> (&'static str, &'static str, &'static str) {
    match kind {
        SourcePushKind::Announcement => (
            "S-01-announcement",
            "v17_source_dispatcher",
            "v17_sources_render_message_announcement",
        ),
        SourcePushKind::PolicyHit => (
            "S-02-policy-hit",
            "v17_source_dispatcher",
            "v17_sources_render_message_policy_hit",
        ),
        SourcePushKind::EarningsBeat => (
            "S-03-earnings-beat",
            "v17_source_dispatcher",
            "v17_sources_render_message_earnings_beat",
        ),
        SourcePushKind::EarningsMiss => (
            "S-04-earnings-miss",
            "v17_source_dispatcher",
            "v17_sources_render_message_earnings_miss",
        ),
        SourcePushKind::AnalystUpgrade => (
            "S-05-analyst-upgrade",
            "v17_source_dispatcher",
            "v17_sources_render_message_analyst_upgrade",
        ),
        SourcePushKind::MarketActionAlert => (
            "S-06-market-action-alert",
            "v17_source_dispatcher",
            "v17_sources_render_message_market_action_alert",
        ),
    }
}

/// Returns a static str label for human-readable rendering.
fn source_push_kind_label(kind: SourcePushKind) -> &'static str {
    match kind {
        SourcePushKind::Announcement => "Announcement",
        SourcePushKind::PolicyHit => "PolicyHit",
        SourcePushKind::EarningsBeat => "EarningsBeat",
        SourcePushKind::EarningsMiss => "EarningsMiss",
        SourcePushKind::AnalystUpgrade => "AnalystUpgrade",
        SourcePushKind::MarketActionAlert => "MarketActionAlert",
    }
}

/// Renders a NormalizedSourceEvent into a push message string.
fn render_message(event: &NormalizedSourceEvent) -> String {
    let mut s = format!(
        "[{}] {}\n{}\nsource={}",
        source_push_kind_label(event.push_kind),
        event.title,
        event.summary,
        event.source
    );
    if let Some(url) = &event.url {
        s.push_str(&format!(" url={}", url));
    }
    if !event.metadata.is_empty() {
        for (k, v) in &event.metadata {
            s.push_str(&format!(" {}={}", k, v));
        }
    }
    s
}

/// BR-196 typed TEST_CODE fixtures for all six normalized-source presentation
/// shapes.  This exercises the production `render_message` seam without
/// entering governance or a provider.
pub(super) fn build_br196_normalized_source_previews() -> Result<Vec<(&'static str, String)>, String>
{
    use stock_analysis::signal::market_event::Direction;

    let identity =
        crate::br196_test_delivery::TestSecurityIdentity::parse("TEST_CODE_SOURCE_ALPHA")?;
    let now = Local::now();
    let specs = [
        ("S-01-announcement", SourcePushKind::Announcement),
        ("S-02-policy-hit", SourcePushKind::PolicyHit),
        ("S-03-earnings-beat", SourcePushKind::EarningsBeat),
        ("S-04-earnings-miss", SourcePushKind::EarningsMiss),
        ("S-05-analyst-upgrade", SourcePushKind::AnalystUpgrade),
        (
            "S-06-market-action-alert",
            SourcePushKind::MarketActionAlert,
        ),
    ];
    specs
        .into_iter()
        .map(|(template_id, source_kind)| {
            let code = if source_kind == SourcePushKind::PolicyHit {
                None
            } else {
                Some(identity.as_str().to_string())
            };
            let source = "TEST_CODE_NORMALIZED_SOURCE".to_string();
            let published_on = if source_kind == SourcePushKind::MarketActionAlert {
                None
            } else {
                Some(now.date_naive())
            };
            let common = (
                source_kind,
                format!("{}_{}", identity.as_str(), template_id),
                code,
                format!("TEST_CODE {:?} 标题", source_kind),
                "TEST_CODE 标准化来源摘要".to_string(),
                Direction::Neutral,
                80,
                80,
                now,
                published_on,
                false,
                source.clone(),
                None,
            );
            let event = if matches!(
                source_kind,
                SourcePushKind::EarningsBeat
                    | SourcePushKind::EarningsMiss
                    | SourcePushKind::AnalystUpgrade
            ) {
                let evidence = SourceBatchEvidence::new(
                    magic_market_core::ProviderId::Eastmoney,
                    source,
                    Some(now.date_naive().to_string()),
                    now.to_rfc3339(),
                    format!("TEST_CODE_BATCH_{source_kind:?}"),
                    "a".repeat(64),
                )
                .map_err(|error| format!("BR-196 batch fixture rejected: {error}"))?;
                NormalizedSourceEvent::new_with_batch_evidence(
                    common.0,
                    common.1,
                    common.2,
                    common.3,
                    common.4,
                    common.5,
                    common.6,
                    common.7,
                    common.8,
                    common.9,
                    common.10,
                    common.11,
                    common.12,
                    vec![evidence],
                )
            } else {
                NormalizedSourceEvent::new(
                    common.0, common.1, common.2, common.3, common.4, common.5, common.6, common.7,
                    common.8, common.9, common.10, common.11, common.12,
                )
            }
            .map_err(|error| format!("BR-196 normalized fixture rejected: {error}"))?;
            Ok((template_id, render_message(&event)))
        })
        .collect()
}

/// Pushes a single NormalizedSourceEvent through the monitor push pipeline.
/// Calls `push_governor_v3` exactly once; no retry, no fallback PushKind.
pub async fn push_normalized_event(event: NormalizedSourceEvent) -> PushAttempt {
    let kind = source_push_kind_to_push_kind(event.push_kind);
    let (family_key, producer_seam_id, renderer_seam_id) =
        source_presentation_tuple(event.push_kind);
    let presentation_token = match crate::presentation_registry::acquire_token(
        family_key,
        kind,
        producer_seam_id,
        renderer_seam_id,
    ) {
        Ok(token) => token,
        Err(error) => {
            return PushAttempt {
                kind,
                code: event.code.clone(),
                outcome: PushOutcome::Denied(error),
                rendered_len: 0,
            };
        }
    };
    let code_str = event.code.clone();
    if let Err(error) = event.validate() {
        log::error!(
            "[v17.7][BR-137] normalized source event rejected: kind={kind:?} reason={error}"
        );
        return PushAttempt {
            kind,
            code: code_str,
            outcome: PushOutcome::Denied(format!("source_event_invalid:{error}")),
            rendered_len: 0,
        };
    }
    let rendered = render_message(&event);
    let outcome = if matches!(
        kind,
        PushKind::Announcement
            | PushKind::PolicyHit
            | PushKind::EarningsBeat
            | PushKind::EarningsMiss
            | PushKind::AnalystUpgrade
    ) {
        match crate::v14_adapter::SourceFactEvidence::new(
            kind,
            event.event_id.clone(),
            event.code.clone(),
            event.title.clone(),
            event.source.clone(),
            event.observed_at,
            event.source_published_on,
            event.strength,
            event.certainty,
            event.stale,
        ) {
            Ok(evidence) => {
                notify::push_presented_source_fact_v3(presentation_token, &rendered, &evidence)
                    .await
            }
            Err(error) => {
                log::error!("[v17.7][BR-137] source fact rejected: kind={kind:?} reason={error}");
                PushOutcome::Denied(format!("source_fact_invalid:{error}"))
            }
        }
    } else {
        notify::push_presented_v3(presentation_token, &rendered, code_str.as_deref()).await
    };
    let rendered_len = rendered.len();
    PushAttempt {
        kind,
        code: code_str,
        outcome,
        rendered_len,
    }
}

/// Processes a batch of NormalizedSourceEvents, skipping those with empty
/// title or event_id before attempting any push.
pub async fn push_normalized_events(events: Vec<NormalizedSourceEvent>) -> SourcePollReport {
    let mut report = SourcePollReport::default();
    for event in events {
        report.attempted += 1;
        if let Err(error) = event.validate() {
            log::warn!(
                "[v17.7][BR-137] source batch item skipped: kind={:?} reason={error}",
                event.push_kind
            );
            report.skipped += 1;
            continue;
        }
        report.classified += 1;
        let attempt = push_normalized_event(event).await;
        record_source_attempt(&mut report, &attempt.outcome);
    }
    report
}

/// Build a NormalizedSourceEvent for an earnings classification.
fn earnings_classification_to_event(
    code: &str,
    classification: &EarningsClassification,
    source_published_on: chrono::NaiveDate,
    source_batches: Vec<SourceBatchEvidence>,
) -> Result<NormalizedSourceEvent, stock_analysis::news::aggregator::NormalizedSourceError> {
    let push_kind = match classification.kind {
        EarningsKind::Beat => SourcePushKind::EarningsBeat,
        EarningsKind::Miss => SourcePushKind::EarningsMiss,
        EarningsKind::Unclassified => SourcePushKind::EarningsBeat, // Should not happen
    };
    let (direction, title_prefix) = match classification.kind {
        EarningsKind::Beat => (
            stock_analysis::signal::market_event::Direction::Bull,
            "超预期",
        ),
        EarningsKind::Miss => (
            stock_analysis::signal::market_event::Direction::Bear,
            "低于预期",
        ),
        EarningsKind::Unclassified => (
            stock_analysis::signal::market_event::Direction::Neutral,
            "未分类",
        ),
    };
    let title = format!(
        "{} 业绩{} (实际EPS {} vs 预期 {})",
        code, title_prefix, classification.actual, classification.reference
    );
    let summary = format!("delta {}%", classification.delta_pct);
    let event_id = format!(
        "earnings:{}:{}",
        code,
        classification.report_date.format("%Y%m%d")
    );
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("actual".to_string(), classification.actual.to_string());
    metadata.insert(
        "reference".to_string(),
        classification.reference.to_string(),
    );
    metadata.insert(
        "delta_pct".to_string(),
        classification.delta_pct.to_string(),
    );

    let source = source_batches
        .first()
        .ok_or(NormalizedSourceError::MissingBatchEvidence)?
        .source
        .clone();
    let observed_at = latest_batch_observed_at(&source_batches)?;
    Ok(NormalizedSourceEvent::new_with_batch_evidence(
        push_kind,
        event_id,
        Some(code.to_string()),
        title,
        summary,
        direction,
        80,
        90,
        observed_at,
        Some(source_published_on),
        false,
        source,
        None,
        source_batches,
    )?
    .with_metadata("actual", metadata["actual"].clone())
    .with_metadata("reference", metadata["reference"].clone())
    .with_metadata("delta_pct", metadata["delta_pct"].clone()))
}

/// Build a NormalizedSourceEvent for an analyst upgrade.
fn analyst_upgrade_event(
    code: &str,
    broker: &str,
    from: &str,
    to: &str,
    report_id: &str,
    publish_date: chrono::NaiveDate,
    source_evidence: SourceBatchEvidence,
) -> Result<NormalizedSourceEvent, stock_analysis::news::aggregator::NormalizedSourceError> {
    let event_id = format!("analyst:{}:{}:{}", code, broker, report_id);
    let title = format!("{} 券商上调评级", broker);
    let summary = format!("从 {} 上调至 {}", from, to);
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("broker".to_string(), broker.to_string());
    metadata.insert("from_rating".to_string(), from.to_string());
    metadata.insert("to_rating".to_string(), to.to_string());

    let observed_at = latest_batch_observed_at(std::slice::from_ref(&source_evidence))?;
    let source = source_evidence.source.clone();
    Ok(NormalizedSourceEvent::new_with_batch_evidence(
        SourcePushKind::AnalystUpgrade,
        event_id,
        Some(code.to_string()),
        title,
        summary,
        stock_analysis::signal::market_event::Direction::Bull,
        70,
        80,
        observed_at,
        Some(publish_date),
        false,
        source,
        None,
        vec![source_evidence],
    )?
    .with_metadata("broker", metadata["broker"].clone())
    .with_metadata("from_rating", metadata["from_rating"].clone())
    .with_metadata("to_rating", metadata["to_rating"].clone()))
}

/// Trait for fetching earnings and consensus data.
/// Allows deterministic tests without opening provider transports.
pub trait EarningsFetcher: Send + Sync {
    async fn fetch_financials(&self, code: &str) -> anyhow::Result<company_financials::Financials>;
    async fn fetch_consensus(&self, code: &str) -> anyhow::Result<EvidenceBackedConsensus>;
}

/// Real fetcher using the existing data providers.
pub struct RealEarningsFetcher;

impl EarningsFetcher for RealEarningsFetcher {
    async fn fetch_financials(&self, code: &str) -> anyhow::Result<company_financials::Financials> {
        let batch = stock_analysis::data_gateway::CompanyDataGateway::new()
            .income_statements(&[code.to_string()])
            .await?;
        company_financials::project_income_statements(batch)
    }
    async fn fetch_consensus(&self, code: &str) -> anyhow::Result<EvidenceBackedConsensus> {
        let batch = stock_analysis::data_gateway::ConsensusDataGateway::new()
            .fetch(code)
            .await?;
        log::info!("[v17_sources][BR-164] admitted consensus batch: {batch}");
        project_consensus_batch(batch)
    }
}

/// Poll earnings and analyst data for the watchlist.
///
/// For each code in `our_codes`:
/// - If `elapsed < poll_secs_earnings` since last earnings poll, skip.
/// - Otherwise fetch financials + consensus, classify earnings, build event.
/// - If `elapsed < poll_secs_analyst` since last analyst poll, skip analyst check.
/// - Otherwise for each recent report, call analyst_store.observe(), build upgrade events.
///
/// Returns a SourcePollReport summarizing what was attempted/pushed/skipped/failed.
pub async fn poll_earnings_and_analyst(
    our_codes: &std::collections::HashSet<String>,
    earnings_cfg: &EarningsConfig,
    analyst_store: &AnalystStateStore,
    last_poll_earnings: Arc<Mutex<HashMap<String, Instant>>>,
    last_poll_analyst: Arc<Mutex<HashMap<String, Instant>>>,
    poll_secs_earnings: u64,
    poll_secs_analyst: u64,
) -> SourcePollReport {
    if our_codes.is_empty() {
        return SourcePollReport::default();
    }

    let fetcher = RealEarningsFetcher;
    let mut events: Vec<NormalizedSourceEvent> = Vec::new();
    let mut source_failures = 0usize;
    let now = Instant::now();

    // Track poll times for this iteration
    let poll_secs_earnings_duration = std::time::Duration::from_secs(poll_secs_earnings);
    let poll_secs_analyst_duration = std::time::Duration::from_secs(poll_secs_analyst);

    for code in our_codes {
        let code_str = code.as_str();

        // --- Earnings polling ---
        {
            let should_poll = {
                let last_polls = last_poll_earnings.lock().unwrap();
                last_polls
                    .get(code_str)
                    .map(|last| last.elapsed() >= poll_secs_earnings_duration)
                    .unwrap_or(true) // Never polled = poll now
            };

            if should_poll {
                let (financials_result, consensus_result) = tokio::join!(
                    fetcher.fetch_financials(code_str),
                    fetcher.fetch_consensus(code_str)
                );
                match (financials_result, consensus_result) {
                    (Ok(financials), Ok(consensus)) => {
                        let source_evidence = financial_batch_evidence(&financials)
                            .map_err(|error| error.to_string())
                            .and_then(|financial_evidence| {
                                let raw = financials
                                    .published_date
                                    .as_deref()
                                    .ok_or_else(|| "financial NOTICE_DATE missing".to_string())?;
                                let published_on =
                                    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(
                                        |error| format!("financial NOTICE_DATE invalid: {error}"),
                                    )?;
                                Ok((
                                    vec![financial_evidence, consensus.evidence.clone()],
                                    published_on,
                                ))
                            });
                        match (financials.history.first(), source_evidence) {
                            (Some(latest_period), Ok((source_batches, published_on))) => {
                                if let Some(classification) =
                                    classify_earnings(latest_period, &consensus.data, earnings_cfg)
                                {
                                    match earnings_classification_to_event(
                                        code_str,
                                        &classification,
                                        published_on,
                                        source_batches,
                                    ) {
                                        Ok(event) => events.push(event),
                                        Err(error) => {
                                            source_failures += 1;
                                            log::warn!(
                                                "[v17_sources][BR-137] earnings source fact rejected: {error}"
                                            );
                                        }
                                    }
                                }
                            }
                            (None, _) => {
                                source_failures += 1;
                                log::warn!(
                                    "[v17_sources][BR-115] {code_str} financial history missing"
                                );
                            }
                            (_, Err(error)) => {
                                source_failures += 1;
                                log::warn!(
                                    "[v17_sources][BR-137] {code_str} earnings publication evidence rejected: {error}"
                                );
                            }
                        }
                        let mut last_polls = last_poll_earnings.lock().unwrap();
                        last_polls.insert(code_str.to_string(), now);
                    }
                    (financials_result, consensus_result) => {
                        source_failures += 1;
                        log::warn!(
                            "[v17_sources][BR-115] {} earnings batch rejected: financials={}; consensus={}",
                            code_str,
                            financials_result
                                .err()
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "ok".to_string()),
                            consensus_result
                                .err()
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "ok".to_string())
                        );
                    }
                }
            }
        }

        // --- Analyst polling ---
        {
            let should_poll = {
                let last_polls = last_poll_analyst.lock().unwrap();
                last_polls
                    .get(code_str)
                    .map(|last| last.elapsed() >= poll_secs_analyst_duration)
                    .unwrap_or(true)
            };

            if should_poll {
                match fetcher.fetch_consensus(code_str).await {
                    Ok(consensus) => {
                        for report in &consensus.data.recent_reports {
                            let key = AnalystKey {
                                code: code_str.to_string(),
                                broker: report.org_name.clone(),
                            };
                            let publish_date = match chrono::NaiveDate::parse_from_str(
                                &report.publish_date,
                                "%Y-%m-%d",
                            ) {
                                Ok(date) => date,
                                Err(error) => {
                                    source_failures += 1;
                                    log::warn!(
                                        "[v17_sources][BR-115] {} analyst report date invalid ({}): {}",
                                        code_str,
                                        report.publish_date,
                                        error
                                    );
                                    continue;
                                }
                            };
                            let obs = AnalystObservation {
                                rating: report.rating.clone(),
                                publish_date,
                                report_id: report.title.clone(), // Use title as report_id proxy
                            };

                            if let stock_analysis::news::aggregator::analyst_state::ObservationDecision::Upgrade { from, to } = analyst_store.observe(key.clone(), obs) {
                                match analyst_upgrade_event(
                                    code_str,
                                    &report.org_name,
                                    &from,
                                    &to,
                                    &report.title,
                                    publish_date,
                                    consensus.evidence.clone(),
                                ) {
                                    Ok(event) => events.push(event),
                                    Err(error) => {
                                        source_failures += 1;
                                        log::warn!(
                                            "[v17_sources][BR-137] analyst source fact rejected: {error}"
                                        );
                                    }
                                }
                            }
                        }

                        // Update last poll time
                        {
                            let mut last_polls = last_poll_analyst.lock().unwrap();
                            last_polls.insert(code_str.to_string(), now);
                        }
                    }
                    Err(e) => {
                        source_failures += 1;
                        log::warn!(
                            "[v17_sources] {} analyst consensus fetch failed: {}",
                            code_str,
                            e
                        );
                    }
                }
            }
        }
    }

    // Push all collected events
    let mut report = if events.is_empty() {
        SourcePollReport::default()
    } else {
        push_normalized_events(events).await
    };
    report.failed += source_failures;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use stock_analysis::signal::market_event::Direction;

    /// Returns a valid announcement event for testing.
    fn test_announcement_event() -> NormalizedSourceEvent {
        NormalizedSourceEvent {
            push_kind: SourcePushKind::Announcement,
            event_id: "ann-1".into(),
            code: Some("TEST_CODE_ANNOUNCEMENT".into()),
            title: "关于回购股份方案的公告".into(),
            summary: "回购".into(),
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            observed_at: Local::now(),
            source_published_on: Some(Local::now().date_naive()),
            stale: false,
            source: "cninfo-market".into(),
            source_batches: Vec::new(),
            url: Some("https://example.invalid/ann-1".into()),
            metadata: Default::default(),
        }
    }

    /// Returns an event with empty title — bypasses NormalizedSourceEvent::new()
    /// validation to test the adapter's own empty-title filter.
    fn test_event_with_empty_title() -> NormalizedSourceEvent {
        NormalizedSourceEvent {
            push_kind: SourcePushKind::Announcement,
            event_id: "ann-empty".into(),
            code: Some("TEST_CODE_EMPTY_TITLE".into()),
            title: "".into(),
            summary: "".into(),
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            observed_at: Local::now(),
            source_published_on: Some(Local::now().date_naive()),
            stale: false,
            source: "test".into(),
            source_batches: Vec::new(),
            url: None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn announcement_adapter_calls_only_announcement_kind() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let report = push_normalized_events(vec![test_announcement_event()]).await;
        assert_eq!(report.attempted, 1);
        assert_eq!(report.pushed, 1);
        assert_eq!(report.failed, 0);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_real_announcement_batch_has_a_production_source_fact_owner() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let announcement = stock_analysis::announcement::Announcement {
            code: "TEST_CODE_ANN_PRODUCER".to_string(),
            name: "测试公司".to_string(),
            title: "关于回购股份方案的公告".to_string(),
            date: Local::now().date_naive().format("%Y-%m-%d").to_string(),
            summary: "回购".to_string(),
            content: "真实公告正文协议夹具".to_string(),
            level: stock_analysis::announcement::AnnLevel::Important,
            reason: "标题含回购".to_string(),
            external_id: Some("TEST_CODE_ANN_EXTERNAL".to_string()),
            url: Some("https://example.invalid/TEST_CODE_ANN_EXTERNAL".to_string()),
        };

        let eligible = HashSet::from([announcement.code.clone()]);
        let routed = route_announcements(&[announcement], &eligible).await;
        assert_eq!(routed.source.attempted, 1);
        assert_eq!(routed.source.classified, 1);
        assert_eq!(routed.source.pushed, 1);
        assert_eq!(
            routed.disposition_for_input(0),
            Some(AnnouncementDisposition::Pushed)
        );
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_repeated_real_announcement_batch_is_not_pushed_twice() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let announcement = stock_analysis::announcement::Announcement {
            code: "TEST_CODE_ANN_REPEAT".to_string(),
            name: "测试公司".to_string(),
            title: "关于同一真实公告重复轮询的公告".to_string(),
            date: Local::now().date_naive().format("%Y-%m-%d").to_string(),
            summary: "重复轮询".to_string(),
            content: "真实公告正文协议夹具".to_string(),
            level: stock_analysis::announcement::AnnLevel::Important,
            reason: "重复轮询测试".to_string(),
            external_id: Some("TEST_CODE_ANN_REPEAT_EXTERNAL".to_string()),
            url: Some("https://example.invalid/TEST_CODE_ANN_REPEAT_EXTERNAL".to_string()),
        };

        let eligible = HashSet::from([announcement.code.clone()]);
        let first = route_announcements(std::slice::from_ref(&announcement), &eligible).await;
        let second = route_announcements(&[announcement], &eligible).await;
        assert_eq!(first.source.pushed, 1);
        assert_eq!(second.source.pushed, 0);
        assert_eq!(second.source.failed, 1);
    }

    fn br138_important_announcement(
        external_id: &str,
        code: &str,
    ) -> stock_analysis::announcement::Announcement {
        stock_analysis::announcement::Announcement {
            code: code.to_string(),
            name: "测试公司".to_string(),
            title: "重大监管问询公告".to_string(),
            date: Local::now().date_naive().format("%Y-%m-%d").to_string(),
            summary: "监管问询".to_string(),
            content: "真实公告正文协议夹具".to_string(),
            level: stock_analysis::announcement::AnnLevel::Important,
            reason: "标题含监管问询".to_string(),
            external_id: Some(external_id.to_string()),
            url: Some(format!("https://example.invalid/{external_id}")),
        }
    }

    fn br210_announcement_batch(
        observed_at: &str,
        lifecycle_only: bool,
    ) -> stock_analysis::announcement::AnnouncementBatch {
        let mut announcement = br138_important_announcement(
            "TEST_CODE_BR210_EXTERNAL",
            "TEST_CODE_BR210_ANNOUNCEMENT",
        );
        if lifecycle_only {
            announcement.title = "关于注销部分回购股份并减少注册资本通知债权人的公告".to_string();
            announcement.level = stock_analysis::announcement::AnnLevel::Skip;
        }
        stock_analysis::announcement::AnnouncementBatch {
            announcements: vec![announcement],
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Cninfo,
                source: "TEST_CODE_cninfo-market".to_string(),
                source_at: Some("2026-08-04T00:00:00+08:00".to_string()),
                observed_at: observed_at.to_string(),
                batch_id: "TEST_CODE_BR210_BATCH".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn br210_announcement_batch_accepts_magic_fractional_epoch_observation() {
        // §2.4: observed_at must stay inside the freshness window. A hardcoded
        // wall-clock epoch made this test pass only on the day it was written
        // and fail every day after, so derive it from the current instant like
        // the sibling fractional-epoch test does.
        let now = Local::now();
        let observed_at = format!("{}.{:09}", now.timestamp(), now.timestamp_subsec_nanos());
        let report = route_announcement_batch(
            &br210_announcement_batch(&observed_at, true),
            &HashSet::new(),
        )
        .await;

        assert_eq!(report.source.attempted, 1);
        assert_eq!(report.source.failed, 0);
        assert_eq!(report.source.skipped, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::FilteredLifecycle)
        );
    }

    #[tokio::test]
    async fn br210_announcement_batch_rejects_malformed_observation() {
        let report = route_announcement_batch(
            &br210_announcement_batch("1785799979.8510450000", true),
            &HashSet::new(),
        )
        .await;

        assert_eq!(report.source.attempted, 1);
        assert_eq!(report.source.failed, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::Failed)
        );
    }

    #[tokio::test]
    async fn br210_actionable_announcement_reaches_audience_filter_with_fractional_epoch() {
        let now = Local::now();
        let observed_at = format!("{}.{:09}", now.timestamp(), now.timestamp_subsec_nanos());
        let report = route_announcement_batch(
            &br210_announcement_batch(&observed_at, false),
            &HashSet::new(),
        )
        .await;

        assert_eq!(report.source.attempted, 1);
        assert_eq!(report.source.classified, 1);
        assert_eq!(report.source.failed, 0);
        assert_eq!(report.source.skipped, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::FilteredAudience)
        );
    }

    #[tokio::test]
    async fn br210_malformed_empty_batch_is_machine_visible_without_fake_input() {
        let mut batch = br210_announcement_batch("1785799979.8510450000", true);
        batch.announcements.clear();

        let report = route_announcement_batch(&batch, &HashSet::new()).await;

        assert_eq!(report.source.attempted, 0);
        assert_eq!(report.source.failed, 1);
        assert!(report.input_dispositions.is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br138_off_universe_announcement_is_handled_without_push() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let eligible = HashSet::from(["TEST_CODE_ALLOWED".to_string()]);
        let report = route_announcements(
            &[br138_important_announcement(
                "TEST_CODE_EXTERNAL",
                "TEST_CODE_OTHER",
            )],
            &eligible,
        )
        .await;
        assert_eq!(report.source.pushed, 0);
        assert_eq!(report.source.skipped, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::FilteredAudience)
        );
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br138_lifecycle_only_announcement_is_handled_without_legacy_fallback() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let code = "TEST_CODE_LIFECYCLE";
        let eligible = HashSet::from([code.to_string()]);
        let mut announcement = br138_important_announcement("TEST_CODE_LIFECYCLE_EXTERNAL", code);
        announcement.title = "关于注销部分回购股份并减少注册资本通知债权人的公告".to_string();
        announcement.level = stock_analysis::announcement::AnnLevel::Skip;
        announcement.content.clear();

        let report = route_announcements(&[announcement], &eligible).await;

        assert_eq!(report.source.classified, 1);
        assert_eq!(report.source.pushed, 0);
        assert_eq!(report.source.skipped, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::FilteredLifecycle)
        );
    }

    #[tokio::test]
    async fn br138_keyword_unmatched_announcement_is_handled_without_false_empty_title_failure() {
        let code = "TEST_CODE_KEYWORD_SKIP";
        let eligible = HashSet::from([code.to_string()]);
        let mut announcement =
            br138_important_announcement("TEST_CODE_KEYWORD_SKIP_EXTERNAL", code);
        announcement.title = "关于召开年度股东大会的通知".to_string();
        announcement.level = stock_analysis::announcement::AnnLevel::Skip;
        announcement.reason.clear();

        let report = route_announcements(&[announcement], &eligible).await;

        assert_eq!(report.source.attempted, 1);
        assert_eq!(report.source.classified, 1);
        assert_eq!(report.source.pushed, 0);
        assert_eq!(report.source.skipped, 1);
        assert_eq!(report.source.failed, 0);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::FilteredClassification)
        );
    }

    #[tokio::test]
    async fn br138_skip_rows_must_pass_source_fact_validation_before_filtering() {
        let observed_at = Local::now();
        let eligible = HashSet::new();
        let base = br138_important_announcement(
            "TEST_CODE_SKIP_VALIDATION_EXTERNAL",
            "TEST_CODE_SKIP_VALIDATION",
        );

        let mut empty_title = base.clone();
        empty_title.title = "   ".to_string();
        empty_title.level = stock_analysis::announcement::AnnLevel::Skip;

        let mut empty_code = base.clone();
        empty_code.code = "   ".to_string();
        empty_code.level = stock_analysis::announcement::AnnLevel::Skip;

        let mut invalid_date = base.clone();
        invalid_date.date = "not-a-provider-date".to_string();
        invalid_date.level = stock_analysis::announcement::AnnLevel::Skip;

        let mut stale_date = base.clone();
        stale_date.date = (observed_at.date_naive() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        stale_date.level = stock_analysis::announcement::AnnLevel::Skip;

        let mut future_date = base.clone();
        future_date.date = (observed_at.date_naive() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        future_date.level = stock_analysis::announcement::AnnLevel::Skip;

        let mut empty_source = base;
        empty_source.level = stock_analysis::announcement::AnnLevel::Skip;

        for (announcement, source) in [
            (empty_title, "TEST_CODE_cninfo-market"),
            (empty_code, "TEST_CODE_cninfo-market"),
            (invalid_date, "TEST_CODE_cninfo-market"),
            (stale_date, "TEST_CODE_cninfo-market"),
            (future_date, "TEST_CODE_cninfo-market"),
            (empty_source, "   "),
        ] {
            let report = route_announcements_with_provenance(
                &[announcement],
                &eligible,
                observed_at,
                source,
            )
            .await;
            assert_eq!(report.source.attempted, 1);
            assert_eq!(report.source.classified, 0);
            assert_eq!(report.source.skipped, 0);
            assert_eq!(report.source.failed, 1);
            assert_eq!(
                report.disposition_for_input(0),
                Some(AnnouncementDisposition::Failed)
            );
        }

        let old_observed_at = observed_at - chrono::Duration::days(1);
        let mut old_observation_and_publication = br138_important_announcement(
            "TEST_CODE_SKIP_OLD_OBSERVATION_EXTERNAL",
            "TEST_CODE_SKIP_OLD_OBSERVATION",
        );
        old_observation_and_publication.date =
            old_observed_at.date_naive().format("%Y-%m-%d").to_string();
        old_observation_and_publication.level = stock_analysis::announcement::AnnLevel::Skip;
        let report = route_announcements_with_provenance(
            &[old_observation_and_publication],
            &eligible,
            old_observed_at,
            "TEST_CODE_cninfo-market",
        )
        .await;
        assert_eq!(report.source.classified, 0);
        assert_eq!(report.source.skipped, 0);
        assert_eq!(report.source.failed, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::Failed)
        );
    }

    #[test]
    fn br138_disposition_counts_are_explicit_for_canary_evidence() {
        let report = AnnouncementSourceRouteReport::with_dispositions_for_test(vec![
            AnnouncementDisposition::Pushed,
            AnnouncementDisposition::FilteredClassification,
            AnnouncementDisposition::FilteredLifecycle,
            AnnouncementDisposition::FilteredAudience,
            AnnouncementDisposition::Failed,
        ]);

        let counts = report.disposition_counts();
        assert_eq!(counts.pushed, 1);
        assert_eq!(counts.filtered_classification, 1);
        assert_eq!(counts.filtered_lifecycle, 1);
        assert_eq!(counts.filtered_audience, 1);
        assert_eq!(counts.failed, 1);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br138_eligible_actionable_announcement_still_reaches_governance() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let eligible = HashSet::from(["TEST_CODE_ALLOWED".to_string()]);
        let report = route_announcements(
            &[br138_important_announcement(
                "TEST_CODE_ALLOWED_EXTERNAL",
                "TEST_CODE_ALLOWED",
            )],
            &eligible,
        )
        .await;
        assert_eq!(report.source.classified, 1);
        assert_eq!(report.source.pushed, 1);
        assert_eq!(
            report.disposition_for_input(0),
            Some(AnnouncementDisposition::Pushed)
        );
    }

    #[tokio::test]
    async fn br137_every_provider_input_has_a_fail_closed_disposition() {
        let eligible = HashSet::from(["TEST_CODE_ALLOWED".to_string()]);
        let mut missing_id = br138_important_announcement("unused", "TEST_CODE_ALLOWED");
        missing_id.external_id = None;
        let mut stale = br138_important_announcement("TEST_CODE_STALE", "TEST_CODE_ALLOWED");
        stale.date = "2026-01-01".to_string();
        let mut lifecycle =
            br138_important_announcement("TEST_CODE_LIFECYCLE_2", "TEST_CODE_ALLOWED");
        lifecycle.title = "关于注销部分回购股份并减少注册资本通知债权人的公告".to_string();
        lifecycle.level = stock_analysis::announcement::AnnLevel::Skip;
        let outside = br138_important_announcement("TEST_CODE_OUTSIDE", "TEST_CODE_OTHER");

        let report = route_announcements(&[missing_id, stale, lifecycle, outside], &eligible).await;

        assert_eq!(report.source.attempted, 4);
        assert_eq!(report.source.failed, 2);
        assert_eq!(report.source.skipped, 2);
        assert_eq!(
            (0..4)
                .map(|index| report.disposition_for_input(index).unwrap())
                .collect::<Vec<_>>(),
            vec![
                AnnouncementDisposition::Failed,
                AnnouncementDisposition::Failed,
                AnnouncementDisposition::FilteredLifecycle,
                AnnouncementDisposition::FilteredAudience,
            ]
        );
        assert_eq!(report.disposition_for_input(4), None);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_complete_announcement_pushes_when_global_data_mode_is_down() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;

        let attempt = push_normalized_event(test_announcement_event()).await;
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_market_action_approved_at_data_mode_unsafe_after_c_decision() {
        // 2026-08-06 C 方案 (commit a9f006a): data_mode_min 全局放宽为 Down,
        // Unsafe 不再拦截 (仅数据全挂才拦, 且 Down 为最大枚举值 → 实际永不拦,
        // 状态出声由 DataMode banner 承担)。原 br137 market_action 严格断言
        // 反转 (与 br137_complete_announcement 同步)。
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;

        let event =
            normalize_market_action(&order_update("TEST_CODE_MARKET_ACTION_DOWN", "sell", 100))
                .expect("normalized market action");
        assert_eq!(
            push_normalized_event(event).await.outcome,
            PushOutcome::Pushed
        );
    }

    #[tokio::test]
    async fn br137_stale_source_event_is_skipped_explicitly() {
        let mut event = test_announcement_event();
        event.stale = true;
        let report = push_normalized_events(vec![event]).await;
        assert_eq!(report.attempted, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.pushed, 0);
    }

    #[tokio::test]
    async fn missing_source_identity_is_skipped_not_pushed() {
        let report = push_normalized_events(vec![test_event_with_empty_title()]).await;
        assert_eq!(report.skipped, 1);
        assert_eq!(report.pushed, 0);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn analyst_upgrade_maps_to_analyst_upgrade_kind() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let observed_at = Local::now();
        let source_evidence = SourceBatchEvidence::new(
            magic_market_core::ProviderId::Eastmoney,
            " Wind".into(),
            None,
            observed_at.to_rfc3339(),
            "TEST_CODE_ANALYST_BATCH".into(),
            "a".repeat(64),
        )
        .expect("analyst evidence");
        let event = NormalizedSourceEvent::new_with_batch_evidence(
            SourcePushKind::AnalystUpgrade,
            "analyst-1".into(),
            Some("TEST_CODE_ANALYST".into()),
            "券商上调评级".into(),
            "上调至买入".into(),
            Direction::Bull,
            70,
            80,
            observed_at,
            Some(Local::now().date_naive()),
            false,
            " Wind".into(),
            None,
            vec![source_evidence],
        )
        .expect("evidence-backed analyst event");
        let attempt = push_normalized_event(event).await;
        assert_eq!(attempt.kind, PushKind::AnalystUpgrade);
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn policy_hit_with_no_code_is_pushed_as_policy() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let event = NormalizedSourceEvent {
            push_kind: SourcePushKind::PolicyHit,
            event_id: "pol-1".into(),
            code: None,
            title: "关于促进数字经济高质量发展的通知".into(),
            summary: "政策".into(),
            direction: Direction::Bull,
            strength: 80,
            certainty: 90,
            observed_at: Local::now(),
            source_published_on: Some(Local::now().date_naive()),
            stale: false,
            source: "ndrc".into(),
            source_batches: Vec::new(),
            url: Some("https://example.invalid/pol-1".into()),
            metadata: Default::default(),
        };
        let attempt = push_normalized_event(event).await;
        assert_eq!(attempt.kind, PushKind::PolicyHit);
        assert!(attempt.code.is_none());
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
    }

    /// Helper: build a FinancialPeriod for testing.
    fn test_financial_period(
        _code: &str,
        eps: f64,
        report_date: &str,
    ) -> company_financials::FinancialPeriod {
        company_financials::FinancialPeriod {
            report_date: Some(report_date.to_string()),
            eps: Some(eps),
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            op_cash_flow_ps: None,
            total_asset_turnover: None,
            debt_to_assets: None,
        }
    }

    /// Helper: build a ConsensusData for testing with a single recent report.
    fn test_consensus_data(
        code: &str,
        broker: &str,
        rating: &str,
        eps_avg: f64,
    ) -> stock_analysis::data_provider::consensus::ConsensusData {
        use std::collections::HashMap;
        let mut rating_dist = HashMap::new();
        rating_dist.insert(rating.to_string(), 1);
        stock_analysis::data_provider::consensus::ConsensusData {
            report_count: 1,
            broker_count: 1,
            eps_this_year_avg: Some(eps_avg),
            eps_next_year_avg: None,
            eps_next2_year_avg: None,
            rating_distribution: rating_dist,
            target_price_high_avg: None,
            target_price_low_avg: None,
            latest_report_date: Some("2026-07-15".to_string()),
            recent_reports: vec![stock_analysis::data_provider::consensus::RecentReport {
                title: format!("{}研报-{}", broker, code),
                org_name: broker.to_string(),
                publish_date: "2026-07-15".to_string(),
                rating: rating.to_string(),
            }],
        }
    }

    fn test_source_batch(
        provider: magic_market_core::ProviderId,
        source: &str,
        batch_id: &str,
        content_byte: char,
    ) -> SourceBatchEvidence {
        SourceBatchEvidence::new(
            provider,
            source.to_string(),
            Some(Local::now().date_naive().to_string()),
            Local::now().to_rfc3339(),
            batch_id.to_string(),
            content_byte.to_string().repeat(64),
        )
        .expect("test source batch")
    }

    #[test]
    fn br210_latest_batch_observation_accepts_mixed_magic_encodings() {
        let financial = SourceBatchEvidence::new(
            magic_market_core::ProviderId::Sina,
            "TEST_CODE_sina-financial".to_string(),
            None,
            "1785799979.851045000".to_string(),
            "TEST_CODE_BR210_FINANCIAL".to_string(),
            "a".repeat(64),
        )
        .expect("Sina fractional epoch evidence");
        let consensus = SourceBatchEvidence::new(
            magic_market_core::ProviderId::Eastmoney,
            "TEST_CODE_eastmoney-consensus".to_string(),
            None,
            "unix-ms:1785799980851".to_string(),
            "TEST_CODE_BR210_CONSENSUS".to_string(),
            "b".repeat(64),
        )
        .expect("Eastmoney millisecond epoch evidence");

        let latest = latest_batch_observed_at(&[financial, consensus])
            .expect("mixed Magic evidence encodings must remain comparable");

        assert_eq!(latest.timestamp_millis(), 1_785_799_980_851);
    }

    #[test]
    fn consensus_projection_keeps_gateway_evidence_and_content_identity() {
        let data = test_consensus_data("TEST_CODE_CONSENSUS", "TEST_CODE券商", "买入", 1.25);
        let observed_at = Local::now().to_rfc3339();
        let batch = stock_analysis::data_gateway::GatewayBatch::Available {
            records: vec![data],
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Eastmoney,
                source: "TEST_CODE_eastmoney-research".into(),
                source_at: None,
                observed_at: observed_at.clone(),
                batch_id: "TEST_CODE_CONSENSUS_BATCH".into(),
            },
        };

        let projected = project_consensus_batch(batch).expect("evidence-backed projection");
        assert_eq!(
            projected.evidence.provider,
            magic_market_core::ProviderId::Eastmoney
        );
        assert_eq!(projected.evidence.source, "TEST_CODE_eastmoney-research");
        assert_eq!(projected.evidence.source_at, None);
        assert_eq!(projected.evidence.observed_at, observed_at);
        assert_eq!(projected.evidence.batch_id, "TEST_CODE_CONSENSUS_BATCH");
        assert_eq!(projected.evidence.content_sha256.len(), 64);
    }

    #[test]
    fn consensus_projection_rejects_cardinality_without_dropping_evidence() {
        let data = test_consensus_data("TEST_CODE_CONSENSUS", "TEST_CODE券商", "买入", 1.25);
        let batch = stock_analysis::data_gateway::GatewayBatch::Available {
            records: vec![data.clone(), data],
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Eastmoney,
                source: "TEST_CODE_eastmoney-research".into(),
                source_at: None,
                observed_at: Local::now().to_rfc3339(),
                batch_id: "TEST_CODE_CONSENSUS_BATCH".into(),
            },
        };

        let error = project_consensus_batch(batch).expect_err("cardinality must fail closed");
        assert!(error.to_string().contains("expected=1 actual=2"));
    }

    #[tokio::test]
    async fn earnings_beat_and_miss_map_to_distinct_push_kinds() {
        let earnings_cfg = EarningsConfig {
            metric: "eps".to_string(),
            beat_threshold_pct: 10.0,
            miss_threshold_pct: -10.0,
            poll_interval_secs: 900,
        };

        // Beat case: actual EPS 1.10, consensus 1.00 → delta +10% → Beat
        let beat_actual = test_financial_period("TEST_CODE_EARNINGS_BEAT", 1.10, "2026-03-31");
        let beat_consensus = test_consensus_data("TEST_CODE_EARNINGS_BEAT", "券商A", "买入", 1.00);
        let beat_classification = classify_earnings(&beat_actual, &beat_consensus, &earnings_cfg);
        assert!(
            beat_classification.is_some(),
            "Beat classification should not be None"
        );
        assert_eq!(
            beat_classification.as_ref().unwrap().kind,
            EarningsKind::Beat
        );

        let beat_event = earnings_classification_to_event(
            "TEST_CODE_EARNINGS_BEAT",
            beat_classification.as_ref().unwrap(),
            Local::now().date_naive(),
            vec![
                test_source_batch(
                    magic_market_core::ProviderId::Sina,
                    "TEST_CODE_FINANCIAL_PROVIDER",
                    "TEST_CODE_FINANCIAL_BATCH",
                    'a',
                ),
                test_source_batch(
                    magic_market_core::ProviderId::Eastmoney,
                    "TEST_CODE_CONSENSUS_PROVIDER",
                    "TEST_CODE_CONSENSUS_BATCH",
                    'b',
                ),
            ],
        )
        .expect("same-day earnings source fact");
        assert_eq!(beat_event.push_kind, SourcePushKind::EarningsBeat);
        assert_eq!(beat_event.source, "TEST_CODE_FINANCIAL_PROVIDER");
        assert_eq!(
            beat_event.source_published_on,
            Some(Local::now().date_naive()),
            "provider NOTICE_DATE, not accounting period, controls freshness"
        );

        // Miss case: actual EPS 0.89, consensus 1.00 → delta -11% → Miss
        let miss_actual = test_financial_period("TEST_CODE_EARNINGS_MISS", 0.89, "2026-03-31");
        let miss_consensus = test_consensus_data("TEST_CODE_EARNINGS_MISS", "券商B", "中性", 1.00);
        let miss_classification = classify_earnings(&miss_actual, &miss_consensus, &earnings_cfg);
        assert!(
            miss_classification.is_some(),
            "Miss classification should not be None"
        );
        assert_eq!(
            miss_classification.as_ref().unwrap().kind,
            EarningsKind::Miss
        );

        let miss_event = earnings_classification_to_event(
            "TEST_CODE_EARNINGS_MISS",
            miss_classification.as_ref().unwrap(),
            Local::now().date_naive(),
            vec![
                test_source_batch(
                    magic_market_core::ProviderId::Sina,
                    "TEST_CODE_FINANCIAL_PROVIDER",
                    "TEST_CODE_FINANCIAL_BATCH",
                    'c',
                ),
                test_source_batch(
                    magic_market_core::ProviderId::Eastmoney,
                    "TEST_CODE_CONSENSUS_PROVIDER",
                    "TEST_CODE_CONSENSUS_BATCH",
                    'd',
                ),
            ],
        )
        .expect("same-day earnings source fact");
        assert_eq!(miss_event.push_kind, SourcePushKind::EarningsMiss);

        // Verify beat and miss map to different PushKinds
        assert_ne!(
            source_push_kind_to_push_kind(beat_event.push_kind),
            source_push_kind_to_push_kind(miss_event.push_kind)
        );
        assert_eq!(
            source_push_kind_to_push_kind(beat_event.push_kind),
            PushKind::EarningsBeat
        );
        assert_eq!(
            source_push_kind_to_push_kind(miss_event.push_kind),
            PushKind::EarningsMiss
        );
    }

    #[tokio::test]
    async fn repeated_analyst_report_is_not_pushed_twice() {
        let analyst_store = AnalystStateStore::new(10_000);

        let key = AnalystKey {
            code: "TEST_CODE_ANALYST_STATE".to_string(),
            broker: "券商A".to_string(),
        };

        let obs = AnalystObservation {
            rating: "中性".to_string(),
            publish_date: chrono::NaiveDate::parse_from_str("2026-07-15", "%Y-%m-%d").unwrap(),
            report_id: "研报-TEST_CODE_ANALYST_STATE-2026-07-15".to_string(),
        };

        // First observation: should be Observed (new entry)
        let first_decision = analyst_store.observe(key.clone(), obs.clone());
        assert_eq!(
            first_decision,
            stock_analysis::news::aggregator::analyst_state::ObservationDecision::Observed
        );

        // Same report again (same report_id AND same publish_date): should be Duplicate
        let second_decision = analyst_store.observe(key.clone(), obs.clone());
        assert_eq!(
            second_decision,
            stock_analysis::news::aggregator::analyst_state::ObservationDecision::Duplicate
        );

        // No push should be generated for Duplicate, so attempted=0 for the second call
        // This is the key assertion: repeated report is not pushed twice
        match second_decision {
            stock_analysis::news::aggregator::analyst_state::ObservationDecision::Duplicate => {}
            _ => panic!("Expected Duplicate, got {:?}", second_decision),
        }
    }

    // -------------------------------------------------------------------------
    // Task 8: MarketActionAlert transition tests
    // -------------------------------------------------------------------------

    /// Helper: build an OrderUpdate MonitorEvent.
    fn order_update(code: &str, action: &str, shares: u64) -> MonitorEvent {
        MonitorEvent::OrderUpdate {
            code: code.into(),
            action: action.into(),
            shares,
        }
    }

    #[test]
    fn order_update_maps_to_emergency_market_action() {
        let event = order_update("TEST_CODE_MARKET_ACTION", "sell", 100);
        let normalized = normalize_market_action(&event).unwrap();
        assert_eq!(normalized.push_kind, SourcePushKind::MarketActionAlert);
        assert_eq!(normalized.code.as_deref(), Some("TEST_CODE_MARKET_ACTION"));
        assert!(normalized.title.contains("sell"));
    }

    #[test]
    fn unchanged_order_state_is_not_re_emitted() {
        let mut state = MarketActionState::default();
        let event = order_update("TEST_CODE_MARKET_ACTION", "sell", 100);
        assert!(state.accept(&event), "first emission should be accepted");
        assert!(!state.accept(&event), "identical state should be rejected");
    }

    #[test]
    fn market_action_state_dedup_within_capacity() {
        let mut state = MarketActionState::default();
        // Different codes are independent
        let e1 = order_update("TEST_CODE_MARKET_ACTION_1", "buy", 100);
        let e2 = order_update("TEST_CODE_MARKET_ACTION_2", "sell", 200);
        assert!(state.accept(&e1));
        assert!(state.accept(&e2));
        // Same code/action/shares again is rejected
        assert!(!state.accept(&e1));
        assert!(!state.accept(&e2));
        // Different action for same code is accepted
        let e3 = order_update("TEST_CODE_MARKET_ACTION_1", "sell", 100); // same code but different action
        assert!(state.accept(&e3), "different action should be new state");
        let e4 = order_update("TEST_CODE_MARKET_ACTION_1", "buy", 200); // different shares
        assert!(state.accept(&e4), "different shares should be new");
    }

    #[test]
    fn handle_monitor_event_non_order_returns_none() {
        use stock_analysis::monitor::event_bus::MonitorEvent;
        let state = Mutex::new(MarketActionState::default());
        // Alert event should return None
        let alert = MonitorEvent::Alert {
            title: "test".into(),
            success: true,
        };
        // This is a compile-time check that non-OrderUpdate variants type-check
        // Actual runtime behavior: the function returns None for non-OrderUpdate
        let result = futures::executor::block_on(handle_monitor_event(&alert, &state));
        assert!(result.is_none());
    }
}
