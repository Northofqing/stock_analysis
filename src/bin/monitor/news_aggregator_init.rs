//! Registered business rules: BR-078, BR-082, BR-137, BR-174, BR-244.
//! Unified global-news acquisition and receipt-gated notification projection.
//!
//! The old scheduler projected `MarketEvent` and advanced simhash before the
//! source facts had a durable ingress receipt. BR-174 splits that ownership:
//!
//! 1. [`fetch_raw_global_news_batch`] returns typed per-provider terminals and
//!    exact `GlobalNewsRecord + BatchEvidence` without notification effects.
//! 2. The selection ingress owner persists that batch and verifies a receipt.
//! 3. [`project_notifications_after_ingress`] accepts only the sealed
//!    `ReceiptedRawNewsBatch`, then projects BR-082/BR-137 `MarketEvent`s and
//!    advances notification simhash.
//! 4. BR-244 separately permits the public SourceOnly NewsFlash producer to
//!    project same-tick opaque `Available` terminals without minting or
//!    weakening the BR-174 selection-ingress receipt.
//!
//! ## 红线约束
//!
//! - AGENTS.md §§2.1/2.2: provider failure remains `Unavailable`; it is never
//!   converted to a successful empty notification batch.
//! - AGENTS.md §§2.4/2.7: publication, observation, provider and batch identity
//!   remain attached to raw source facts.

#![allow(
    dead_code,
    reason = "BR-174 receipt-gated projection is retained for the selection-v2 release; BR-183 forbids constructing a production receipt while that capability is disabled"
)]

use diesel::RunQueryDsl;
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use stock_analysis::news::aggregator::projection_v2::{
    NotificationProjectionError, NotificationProjectionState, ReceiptedRawNewsBatch,
};
use stock_analysis::news::aggregator::raw_v2::{
    self, registered_global_news_feeds, RawNewsAcquisitionError, RawNewsAggregationBatch,
};
use stock_analysis::signal::market_event::MarketEvent;

static NOTIFICATION_PROJECTION: Lazy<NotificationProjectionState> =
    Lazy::new(NotificationProjectionState::new);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalNewsPipelineRegistration {
    pub feed_count: usize,
    pub registered_feed_set_sha256: String,
}

pub fn uninitialized_global_news_pipeline_registration() -> GlobalNewsPipelineRegistration {
    GlobalNewsPipelineRegistration {
        feed_count: 0,
        registered_feed_set_sha256: sha256_domain(
            "stock_analysis.br196.registered_feed_set.v1",
            b"state=uninitialized\n",
        ),
    }
}

/// Initialize the notification-only state and report the immutable real
/// provider registry size.
pub fn init_global_news_pipeline() -> GlobalNewsPipelineRegistration {
    Lazy::force(&NOTIFICATION_PROJECTION);
    let registrations = registered_global_news_feeds();
    let canonical = registrations
        .iter()
        .map(|feed| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}",
                feed.feed_name,
                feed.gateway_provider,
                feed.provider_id,
                feed.source_contract,
                feed.capability_name,
                feed.max_limit,
                feed.upstream_revision
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let registration = GlobalNewsPipelineRegistration {
        feed_count: registrations.len(),
        registered_feed_set_sha256: sha256_domain(
            "stock_analysis.br196.registered_feed_set.v1",
            canonical.as_bytes(),
        ),
    };
    log::info!(
        "[NewsAggregator][BR-174] raw acquisition + receipted notification projection ready: {} unified Gateway feeds registered",
        registration.feed_count
    );
    log::warn!("[NewsAggregator][BR-244] {NEWS_FLASH_CRITICAL_DISABLED_BANNER}");
    registration
}

fn sha256_domain(domain: &str, payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

/// Fetch the complete registered real-provider set without constructing
/// notification events or mutating simhash.
pub async fn fetch_raw_global_news_batch(
    per_feed_limit: u32,
) -> Result<RawNewsAggregationBatch, RawNewsAcquisitionError> {
    let batch = raw_v2::fetch_raw_global_news_batch(per_feed_limit).await?;
    log::info!(
        "[NewsAggregator][BR-174] raw batch acquired attempts={} records={} sources_complete={} per_feed_limit={}",
        batch.attempts().len(),
        batch.source_record_count(),
        batch.sources_complete(),
        per_feed_limit
    );
    Ok(batch)
}

/// Project BR-082/BR-137 notification events only after the selection ingress
/// owner has returned a verified receipt capability.
pub fn project_notifications_after_ingress(
    batch: ReceiptedRawNewsBatch,
) -> Result<Vec<MarketEvent>, NotificationProjectionError> {
    let source_batch_content_hash = batch.source_batch_content_hash().to_owned();
    let ingress_receipt_hash = batch.ingress_receipt_hash().to_owned();
    let events = NOTIFICATION_PROJECTION.project_after_ingress(batch)?;
    log::info!(
        "[NewsAggregator][BR-174] receipted notification projection events={} source_batch_content_hash={} ingress_receipt_hash={}",
        events.len(),
        source_batch_content_hash,
        ingress_receipt_hash
    );
    Ok(events)
}

// ============================================================================
// v17.4 §5.1 能力1: NewsFlashGate — critical 即时推 + 4 时段聚合 Top3
// 业务规则登记: BR-082 (docs/business_rules.md, 红线 2.10)
// ============================================================================

/// 4 个聚合窗口 (开盘/午盘收/午盘开/收盘)
const AGG_WINDOWS: [(u32, u32); 4] = [(9, 30), (11, 30), (13, 0), (15, 0)];

/// 窗口触发容差: 窗口时刻起 5 分钟内首个 tick 触发 (news_monitor_loop 轮询默认
/// 120s, spec ±1min 会漏; 加宽到 5min + 当日一次门控, 偏差已在 spec 回填注明)
const AGG_WINDOW_TOLERANCE_SECS: i64 = 300;

pub const NEWS_FLASH_CRITICAL_DISABLED_BANNER: &str =
    "NewsFlashCritical disabled=no_authoritative_strength_provider";

/// The immutable presentation selected by BR-082. Source evidence stays on
/// the enclosing reservation and is not reconstructed from this text.
#[derive(Debug, PartialEq)]
pub enum FlashDecision {
    /// 即时推 (critical): 保留逐事件来源证据；event_id 仅作治理身份，
    /// 不得冒充证券代码。
    Critical {
        event_id: String,
        headline: String,
        source: String,
        observed_at: chrono::DateTime<chrono::Local>,
        source_published_on: chrono::NaiveDate,
        stale: bool,
        strength: u8,
        certainty: u8,
        text: String,
    },
    /// 时段聚合推: (窗口标签, 渲染文本)
    Aggregated { window: String, text: String },
}

/// A non-cloneable gate capability. Dropping it never claims delivery; the
/// owner must explicitly settle it so the gate can release pending state.
#[derive(Debug)]
pub struct FlashReservation {
    token_id: u64,
    push_kind: String,
    business_date: chrono::NaiveDate,
    decision_key: String,
    event_id: Option<String>,
    window: Option<String>,
    attempt_ordinal: u32,
    rendered_len: usize,
    reservation_identity_sha256: String,
    evidence_sha256: String,
    render_sha256: String,
    sources: Vec<stock_analysis::news::aggregator::raw_v2::NewsFlashSourceIdentity>,
    decision: Option<FlashDecision>,
}

impl FlashReservation {
    pub const fn token_id(&self) -> u64 {
        self.token_id
    }

    pub fn push_kind(&self) -> &str {
        &self.push_kind
    }

    pub const fn business_date(&self) -> chrono::NaiveDate {
        self.business_date
    }

    pub fn decision_key(&self) -> &str {
        &self.decision_key
    }

    pub fn event_id(&self) -> Option<&str> {
        self.event_id.as_deref()
    }

    pub fn window(&self) -> Option<&str> {
        self.window.as_deref()
    }

    pub const fn attempt_ordinal(&self) -> u32 {
        self.attempt_ordinal
    }

    pub const fn rendered_len(&self) -> usize {
        self.rendered_len
    }

    pub fn reservation_identity_sha256(&self) -> &str {
        &self.reservation_identity_sha256
    }

    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }

    pub fn render_sha256(&self) -> &str {
        &self.render_sha256
    }

    pub fn sources(&self) -> &[stock_analysis::news::aggregator::raw_v2::NewsFlashSourceIdentity] {
        &self.sources
    }

    pub fn audit_sources(&self) -> Vec<stock_analysis::event::NewsFlashAuditSource> {
        self.sources
            .iter()
            .map(|source| stock_analysis::event::NewsFlashAuditSource {
                event_id: source.event_id().to_owned(),
                provider: source.provider().to_owned(),
                source: source.source().to_owned(),
                published_at: source.published_at().fixed_offset(),
                observed_at: source.observed_at().fixed_offset(),
                batch_id: source.batch_id().to_owned(),
            })
            .collect()
    }

    fn matches_attempt(&self, attempt: &stock_analysis::event::NewsFlashAttemptReceipt) -> bool {
        let input = attempt.input();
        let expected_attempt_identity =
            stock_analysis::event::envelope::news_flash_sink_attempt_identity(
                &input.reservation_sha256,
                input.attempt_ordinal,
                &input.channel,
                &input.observed_at,
            );
        let expected_attempt_sha256 =
            stock_analysis::event::envelope::news_flash_sink_attempt_sha256(
                &input.push_kind,
                &input.decision_key,
                input.business_date,
                &input.reservation_sha256,
                &input.evidence_sha256,
                &input.render_sha256,
                &input.sources,
                input.attempt_ordinal,
                &input.channel,
                &input.observed_at,
                &expected_attempt_identity,
            );
        input.push_kind == self.push_kind
            && input.business_date == self.business_date
            && input.decision_key == self.decision_key
            && input.rendered_len == self.rendered_len
            && input.reservation_sha256 == self.reservation_identity_sha256
            && input.sources == self.audit_sources()
            && input.evidence_sha256 == self.evidence_sha256
            && input.render_sha256 == self.render_sha256
            && input.attempt_ordinal == self.attempt_ordinal
            && !attempt.envelope_id().trim().is_empty()
            && attempt.sink_attempt_identity() == expected_attempt_identity
            && attempt.sink_attempt_sha256() == expected_attempt_sha256
    }

    fn matches_accepted_receipt(
        &self,
        receipt: &stock_analysis::event::NewsFlashAcceptedReceipt,
    ) -> bool {
        let remote = receipt.remote_receipt();
        let expected_remote_identity =
            stock_analysis::event::envelope::news_flash_remote_receipt_identity(remote);
        let expected_remote_sha256 =
            stock_analysis::event::envelope::news_flash_remote_receipt_sha256(remote);
        self.matches_attempt(receipt.attempt())
            && remote.channel == receipt.attempt().input().channel
            && receipt.remote_receipt_identity() == expected_remote_identity.as_str()
            && receipt.remote_receipt_sha256() == expected_remote_sha256.as_str()
            && !receipt.terminal_envelope_id().trim().is_empty()
    }

    pub fn decision(&self) -> &FlashDecision {
        self.decision
            .as_ref()
            .expect("unsettled reservation owns its decision")
    }
}

#[derive(Debug)]
pub enum FlashSettlement {
    Terminal(Box<stock_analysis::event::NewsFlashTerminalReceipt>),
    RolledBack { reason: String },
    Uncertain { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashSettlementError {
    UnknownReservation,
    BindingMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsFlashRecoveryError {
    PendingReservations,
    UnknownAcceptedWindow(String),
    BusinessDateMismatch {
        snapshot: chrono::NaiveDate,
        now: chrono::NaiveDate,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingFlash {
    Critical {
        event_id: String,
        reservation_identity_sha256: String,
    },
    Aggregate {
        index: usize,
        reservation_identity_sha256: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum WindowState {
    #[default]
    Eligible,
    Pending,
    Committed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlashSettlementAction {
    Committed,
    RolledBack,
    Uncertain,
}

/// v17.4 §5.1 门控状态机 (纯逻辑, 不做 IO — 推送由 caller 处理)
pub struct NewsFlashGate {
    day: chrono::NaiveDate,
    buffered_ids: std::collections::HashSet<String>,
    critical_committed: std::collections::HashSet<String>,
    critical_pending: std::collections::HashSet<String>,
    unresolved_reservations: std::collections::HashSet<String>,
    window_state: [WindowState; 4],
    buffer: Vec<stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent>,
    pending: std::collections::HashMap<u64, PendingFlash>,
    recovered_attempt_ordinals: std::collections::HashMap<String, u32>,
    next_token_id: u64,
}

impl NewsFlashGate {
    pub fn new(today: chrono::NaiveDate) -> Self {
        Self {
            day: today,
            buffered_ids: std::collections::HashSet::new(),
            critical_committed: std::collections::HashSet::new(),
            critical_pending: std::collections::HashSet::new(),
            unresolved_reservations: std::collections::HashSet::new(),
            window_state: [WindowState::Eligible; 4],
            buffer: Vec::new(),
            pending: std::collections::HashMap::new(),
            recovered_attempt_ordinals: std::collections::HashMap::new(),
            next_token_id: 1,
        }
    }

    /// 跨天重置 (BR-082: 日桶清零, 防内存增长)
    fn rollover(&mut self, today: chrono::NaiveDate) {
        if self.day != today {
            self.day = today;
            self.buffered_ids.clear();
            self.critical_committed.clear();
            self.critical_pending.clear();
            self.unresolved_reservations.clear();
            self.window_state = [WindowState::Eligible; 4];
            self.buffer.clear();
            self.pending.clear();
            self.recovered_attempt_ordinals.clear();
            self.next_token_id = 1;
            log::info!("[NewsFlashGate] day rollover → {} (buckets reset)", today);
        }
    }

    /// Replace restart-sensitive gate authority from the complete immutable
    /// BR-091 snapshot for one business date. A process-local reservation must
    /// settle before recovery so no in-flight capability is silently erased.
    pub fn recover(
        &mut self,
        snapshot: &stock_analysis::event::NewsFlashAuthoritySnapshot,
    ) -> Result<(), NewsFlashRecoveryError> {
        if !self.pending.is_empty() {
            return Err(NewsFlashRecoveryError::PendingReservations);
        }
        let mut recovered_window_state = [WindowState::Eligible; 4];
        for window in snapshot.accepted_windows() {
            let Some(index) = AGG_WINDOWS
                .iter()
                .position(|(hour, minute)| window == &format!("{hour:02}:{minute:02}"))
            else {
                return Err(NewsFlashRecoveryError::UnknownAcceptedWindow(
                    window.clone(),
                ));
            };
            recovered_window_state[index] = WindowState::Committed;
        }

        self.rollover(snapshot.business_date());

        self.critical_committed = snapshot.accepted_event_ids().iter().cloned().collect();
        self.critical_pending.clear();
        self.window_state = recovered_window_state;
        self.unresolved_reservations = snapshot.unresolved_reservations().iter().cloned().collect();
        self.recovered_attempt_ordinals.clear();
        for reservation in snapshot
            .unresolved_reservations()
            .iter()
            .chain(snapshot.definitively_rejected_reservations().iter())
        {
            self.recovered_attempt_ordinals.insert(
                reservation.clone(),
                snapshot.next_attempt_ordinal(reservation),
            );
        }
        Ok(())
    }

    pub fn reserve_from_authority(
        &mut self,
        snapshot: &stock_analysis::event::NewsFlashAuthoritySnapshot,
        events: &[stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent],
        now: chrono::DateTime<chrono::Local>,
        critical_threshold: u8,
        max_critical_per_day: u32,
    ) -> Result<Vec<FlashReservation>, NewsFlashRecoveryError> {
        if snapshot.business_date() != now.date_naive() {
            return Err(NewsFlashRecoveryError::BusinessDateMismatch {
                snapshot: snapshot.business_date(),
                now: now.date_naive(),
            });
        }
        self.recover(snapshot)?;
        Ok(self.reserve(events, now, critical_threshold, max_critical_per_day))
    }

    /// Admit real projected evidence and reserve eligible decisions. No dedup,
    /// quota or window state is committed until [`Self::settle`] accepts an
    /// exact authoritative receipt.
    ///
    /// critical 判定: strength ≥ threshold 且 certainty ≥ 60 (官方性门槛);
    /// 每日上限 max_per_day, 超限 warn 出声 (v15.x 静默路径可见)。
    pub fn reserve(
        &mut self,
        events: &[stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent],
        now: chrono::DateTime<chrono::Local>,
        _critical_threshold: u8,
        max_critical_per_day: u32,
    ) -> Vec<FlashReservation> {
        self.rollover(now.date_naive());
        let mut out = Vec::new();

        for projected in events {
            let e = projected.event();
            let provenance = e.provenance.first();
            let validation_error = if e.event_id.trim().is_empty() {
                Some("missing_event_id")
            } else if e.full_title.trim().is_empty() {
                Some("missing_headline")
            } else if provenance.is_none_or(|item| item.provider.trim().is_empty()) {
                Some("missing_provenance")
            } else if e.strength > 100 {
                Some("strength_out_of_range")
            } else if e.certainty > 100 {
                Some("certainty_out_of_range")
            } else if e.stale {
                Some("stale")
            } else if e.occurred_at > now {
                Some("future_publication")
            } else if e.occurred_at.date_naive() != now.date_naive() {
                Some("publication_date_not_current")
            } else if provenance
                .is_some_and(|item| item.fetched_at > now || item.fetched_at < e.occurred_at)
            {
                Some("invalid_observation_time")
            } else {
                None
            };
            if let Some(reason) = validation_error {
                log::warn!(
                    "[NewsFlashGate][BR-137] source event rejected before critical and aggregate governance: {reason}"
                );
                continue;
            }
            if self.buffer.len() < 200 && self.buffered_ids.insert(e.event_id.clone()) {
                self.buffer.push(projected.clone());
            }
        }

        let _critical_concurrency_capacity =
            self.critical_concurrency_capacity(max_critical_per_day);

        // Half-open BR-082 window: [target, target + 300 seconds).
        for (i, (h, m)) in AGG_WINDOWS.iter().enumerate() {
            if self.window_state[i] != WindowState::Eligible {
                continue;
            }
            let target = now
                .date_naive()
                .and_hms_opt(*h, *m, 0)
                .expect("valid window time")
                .and_local_timezone(chrono::Local)
                .single();
            let Some(target) = target else { continue };
            let delta = (now - target).num_seconds();
            if (0..AGG_WINDOW_TOLERANCE_SECS).contains(&delta) {
                let label = format!("{:02}:{:02}", h, m);
                if self.buffer.is_empty() {
                    // 红线 2.2: 无数据显式说明, 不臆造
                    log::info!("[NewsFlashGate] {} 窗口无事件, 跳过聚合推送", label);
                    continue;
                }
                let mut sorted = self.buffer.to_vec();
                sorted.sort_by_key(|item| std::cmp::Reverse(item.event().strength));
                let top3 = sorted.into_iter().take(3).collect::<Vec<_>>();
                let lines = top3
                    .iter()
                    .map(|projected| {
                        let event = projected.event();
                        format!(
                            "[{}] {} (强度{} 确定性{})",
                            event.event_type.label(),
                            event.full_title,
                            event.strength,
                            event.certainty
                        )
                    })
                    .collect::<Vec<_>>();
                let text = assemble_news_flash_aggregated(&label, &lines)
                    .expect("nonempty NewsFlash buffer produces a card");
                let decision = FlashDecision::Aggregated {
                    window: label.clone(),
                    text,
                };
                let mut reservation = make_reservation(
                    self.next_token_id,
                    self.day,
                    &format!("window:{label}"),
                    None,
                    Some(label.clone()),
                    top3,
                    decision,
                );
                reservation.attempt_ordinal = self
                    .recovered_attempt_ordinals
                    .get(reservation.reservation_identity_sha256())
                    .copied()
                    .unwrap_or(1);
                if self
                    .unresolved_reservations
                    .contains(reservation.reservation_identity_sha256())
                {
                    log::warn!(
                        "[NewsFlashGate][BR-244] exact unresolved reservation blocks automatic retry: {}",
                        reservation.reservation_identity_sha256()
                    );
                    continue;
                }
                self.next_token_id += 1;
                self.window_state[i] = WindowState::Pending;
                self.pending.insert(
                    reservation.token_id,
                    PendingFlash::Aggregate {
                        index: i,
                        reservation_identity_sha256: reservation
                            .reservation_identity_sha256()
                            .to_owned(),
                    },
                );
                out.push(reservation);
            }
        }

        out
    }

    fn critical_commit_quota_remaining(&self, max_critical_per_day: u32) -> usize {
        (max_critical_per_day as usize).saturating_sub(self.critical_committed.len())
    }

    fn critical_concurrency_capacity(&self, max_critical_per_day: u32) -> usize {
        self.critical_commit_quota_remaining(max_critical_per_day)
            .saturating_sub(self.critical_pending.len())
    }

    pub fn settle(
        &mut self,
        reservation: FlashReservation,
        settlement: FlashSettlement,
    ) -> Result<(), FlashSettlementError> {
        let pending = self
            .pending
            .remove(&reservation.token_id)
            .ok_or(FlashSettlementError::UnknownReservation)?;
        let action = match settlement {
            FlashSettlement::Terminal(terminal) => {
                let (attempt, action) = match terminal.as_ref() {
                    stock_analysis::event::NewsFlashTerminalReceipt::Accepted(receipt) => {
                        if !reservation.matches_accepted_receipt(receipt) {
                            self.restore_pending(&pending);
                            return Err(FlashSettlementError::BindingMismatch);
                        }
                        (receipt.attempt(), FlashSettlementAction::Committed)
                    }
                    stock_analysis::event::NewsFlashTerminalReceipt::DefinitivelyRejected(
                        receipt,
                    ) => (receipt.attempt(), FlashSettlementAction::RolledBack),
                    stock_analysis::event::NewsFlashTerminalReceipt::Uncertain(receipt) => {
                        (receipt.attempt(), FlashSettlementAction::Uncertain)
                    }
                };
                if !reservation.matches_attempt(attempt) {
                    self.restore_pending(&pending);
                    return Err(FlashSettlementError::BindingMismatch);
                }
                action
            }
            FlashSettlement::RolledBack { reason } => {
                log::warn!("[NewsFlashGate][BR-244] reservation rolled back: {reason}");
                FlashSettlementAction::RolledBack
            }
            FlashSettlement::Uncertain { reason } => {
                log::error!("[NewsFlashGate][BR-244] reservation uncertain: {reason}");
                FlashSettlementAction::Uncertain
            }
        };
        match pending {
            PendingFlash::Critical {
                event_id,
                reservation_identity_sha256,
            } => {
                self.critical_pending.remove(&reservation_identity_sha256);
                match action {
                    FlashSettlementAction::Committed => {
                        self.critical_committed.insert(event_id);
                    }
                    FlashSettlementAction::Uncertain => {
                        self.unresolved_reservations
                            .insert(reservation_identity_sha256);
                    }
                    FlashSettlementAction::RolledBack => {}
                }
            }
            PendingFlash::Aggregate {
                index,
                reservation_identity_sha256,
            } => match action {
                FlashSettlementAction::Committed => {
                    self.window_state[index] = WindowState::Committed;
                }
                FlashSettlementAction::RolledBack => {
                    self.window_state[index] = WindowState::Eligible;
                }
                FlashSettlementAction::Uncertain => {
                    self.window_state[index] = WindowState::Eligible;
                    self.unresolved_reservations
                        .insert(reservation_identity_sha256);
                }
            },
        }
        Ok(())
    }

    fn restore_pending(&mut self, pending: &PendingFlash) {
        match pending {
            PendingFlash::Critical {
                reservation_identity_sha256,
                ..
            } => {
                self.critical_pending.remove(reservation_identity_sha256);
                self.unresolved_reservations
                    .insert(reservation_identity_sha256.clone());
            }
            PendingFlash::Aggregate {
                index,
                reservation_identity_sha256,
            } => {
                self.window_state[*index] = WindowState::Eligible;
                self.unresolved_reservations
                    .insert(reservation_identity_sha256.clone());
            }
        }
    }

    #[cfg(test)]
    fn process(
        &mut self,
        events: &[MarketEvent],
        now: chrono::DateTime<chrono::Local>,
        critical_threshold: u8,
        max_critical_per_day: u32,
    ) -> Vec<FlashDecision> {
        let capability =
            stock_analysis::news::aggregator::raw_v2::NewsFlashProjectionTestCapability::bind()
                .expect("monitor unit test owns NewsFlash projection capability");
        let projected = events
            .iter()
            .cloned()
            .map(|event| {
                let published_at = event.occurred_at.with_timezone(&chrono::Utc);
                let observed_at = event
                    .provenance
                    .first()
                    .map(|source| source.fetched_at.with_timezone(&chrono::Utc))
                    .unwrap_or(published_at);
                let batch_id = format!("TEST_CODE_BATCH_{}", event.event_id);
                stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent::test_fixture(
                    &capability,
                    event,
                    "TEST_CODE_PROVIDER",
                    "TEST_CODE_SOURCE",
                    published_at,
                    observed_at,
                    &batch_id,
                )
            })
            .collect::<Vec<_>>();
        self.reserve(&projected, now, critical_threshold, max_critical_per_day)
            .into_iter()
            .map(|reservation| {
                reservation
                    .decision
                    .expect("test reservation owns its decision")
            })
            .collect()
    }
}

fn make_reservation(
    token_id: u64,
    day: chrono::NaiveDate,
    decision_key: &str,
    event_id: Option<String>,
    window: Option<String>,
    projected: Vec<stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent>,
    decision: FlashDecision,
) -> FlashReservation {
    let evidence_sha256 =
        stock_analysis::news::aggregator::raw_v2::ordered_news_flash_evidence_sha256(&projected);
    let text = match &decision {
        FlashDecision::Critical { text, .. } | FlashDecision::Aggregated { text, .. } => text,
    };
    let push_kind = match &decision {
        FlashDecision::Critical { .. } => {
            crate::notify::PushKind::NewsFlashCritical.stable_template_id()
        }
        FlashDecision::Aggregated { .. } => {
            crate::notify::PushKind::NewsFlashAggregated.stable_template_id()
        }
    };
    let render_sha256 = sha256_domain("stock_analysis.news_flash_render.v1", text.as_bytes());
    let business_date = day.to_string();
    let mut reservation_hasher = Sha256::new();
    reservation_hasher.update(b"stock_analysis.news_flash_reservation.v2");
    for value in [
        push_kind.as_str(),
        business_date.as_str(),
        decision_key,
        event_id.as_deref().unwrap_or("<absent>"),
        window.as_deref().unwrap_or("<absent>"),
        evidence_sha256.as_str(),
        render_sha256.as_str(),
    ] {
        reservation_hasher.update((value.len() as u64).to_be_bytes());
        reservation_hasher.update(value.as_bytes());
    }
    let reservation_identity_sha256 = format!("{:x}", reservation_hasher.finalize());
    let sources = projected
        .into_iter()
        .map(|event| event.source().clone())
        .collect();
    FlashReservation {
        token_id,
        push_kind,
        business_date: day,
        decision_key: decision_key.to_owned(),
        event_id,
        window,
        attempt_ordinal: 1,
        rendered_len: text.len(),
        reservation_identity_sha256,
        evidence_sha256,
        render_sha256,
        sources,
        decision: Some(decision),
    }
}

pub(super) fn assemble_news_flash_critical(
    hhmm: &str,
    event_label: &str,
    headline: &str,
    strength: u8,
    certainty: u8,
    ordinal: u32,
    daily_limit: u32,
) -> String {
    format!(
        "🚨 高分新闻快讯 ({hhmm})\n[{event_label}] {headline}\n强度 {strength} | 确定性 {certainty} | 今日第 {ordinal}/{daily_limit} 条"
    )
}

pub(super) fn assemble_news_flash_aggregated(
    window: &str,
    lines: &[String],
) -> Result<String, String> {
    if window.trim().is_empty()
        || lines.is_empty()
        || lines.iter().any(|line| line.trim().is_empty())
    {
        return Err("BR-196 NewsFlash aggregate requires window and nonempty lines".to_string());
    }
    let mut text = format!("📰 新闻时段聚合 ({window}) Top3:\n");
    for (rank, line) in lines.iter().take(3).enumerate() {
        text.push_str(&format!("{}. {line}\n", rank + 1));
    }
    Ok(text)
}

/// BR-244 immutable source-failure append. Mutable dispatcher counters are
/// updated only after the authoritative hash-chain returns a typed receipt.
pub fn audit_news_flash_source_failure(
    failure: &stock_analysis::news::aggregator::raw_v2::NewsFlashSourceFailure,
) -> Result<
    stock_analysis::event::NewsFlashFailureAuditReceipt,
    stock_analysis::event::NewsFlashFailureAuditError,
> {
    let source_record_count = u32::try_from(failure.source_record_count()).map_err(|_| {
        stock_analysis::event::NewsFlashFailureAuditError::InvalidInput(
            "source_record_count exceeds u32".to_owned(),
        )
    })?;
    let receipt = stock_analysis::event::publish_news_flash_failure(
        stock_analysis::event::NewsFlashFailureAuditInput {
            provider: Some(failure.provider().wire_name().to_owned()),
            available_provider: failure.available_provider_wire().map(str::to_owned),
            stage: failure.failed_stage().to_owned(),
            reason_code: failure.reason_code().to_owned(),
            diagnostic_code: failure.diagnostic_code().to_owned(),
            diagnostic: failure.diagnostic().to_owned(),
            retryable: failure.retryable(),
            observed_at: failure.observed_at().fixed_offset(),
            source_record_count,
            batch_id: failure.batch_id().map(str::to_owned),
            record_id: failure.record_id().map(str::to_owned),
        },
    )?;
    let detail = failure.audit_message();
    crate::push_templates::log_dispatcher_attempt(
        "N-01",
        false,
        failure.source_record_count(),
        &detail,
    );
    crate::push_templates::log_dispatcher_attempt(
        "N-02",
        false,
        failure.source_record_count(),
        &detail,
    );
    Ok(receipt)
}

pub fn audit_news_flash_batch_failure(
    reason: &str,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> Result<
    stock_analysis::event::NewsFlashFailureAuditReceipt,
    stock_analysis::event::NewsFlashFailureAuditError,
> {
    let diagnostic = bounded_news_flash_failure_diagnostic(reason);
    let receipt = stock_analysis::event::publish_news_flash_failure(
        stock_analysis::event::NewsFlashFailureAuditInput {
            provider: None,
            available_provider: None,
            stage: "raw_batch_acquisition".to_owned(),
            reason_code: "raw_batch_acquisition_failed".to_owned(),
            diagnostic_code: "raw_batch_acquisition_failed".to_owned(),
            diagnostic,
            retryable: false,
            observed_at: observed_at.fixed_offset(),
            source_record_count: 0,
            batch_id: None,
            record_id: None,
        },
    )?;
    crate::push_templates::log_dispatcher_attempt("N-01", false, 0, reason);
    crate::push_templates::log_dispatcher_attempt("N-02", false, 0, reason);
    Ok(receipt)
}

fn bounded_news_flash_failure_diagnostic(value: &str) -> String {
    const MAX_BYTES: usize = 512;
    const TRUNCATED_SUFFIX: &str = " [truncated]";
    if value.len() <= MAX_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_BYTES.saturating_sub(TRUNCATED_SUFFIX.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}{}", &value[..end], TRUNCATED_SUFFIX)
}

/// 推送包装: 把 FlashDecision 走现有 push_governor_v3 (L4 dedup: critical 按
/// event_id, 聚合按窗口标签 — 见 BR-082)。返回 (critical 推送数, 聚合推送数)。
#[cfg(test)]
pub async fn push_flash_decisions(decisions: Vec<FlashDecision>) -> (usize, usize) {
    let mut n_critical = 0usize;
    let mut n_agg = 0usize;
    for d in decisions {
        match d {
            FlashDecision::Critical {
                event_id,
                headline,
                source,
                observed_at,
                source_published_on,
                stale,
                strength,
                certainty,
                text,
            } => {
                let presentation_token = match crate::presentation_registry::acquire_token(
                    "N-01-news-flash-critical",
                    crate::notify::PushKind::NewsFlashCritical,
                    "news_flash_critical_dispatcher",
                    "assemble_news_flash_critical",
                ) {
                    Ok(token) => token,
                    Err(error) => {
                        log::error!("[NewsFlashGate][BR-196] critical token rejected: {error}");
                        crate::push_templates::log_dispatcher_attempt(
                            "N-01",
                            false,
                            1,
                            &format!("presentation_token_rejected:{error}"),
                        );
                        continue;
                    }
                };
                let outcome = match crate::v14_adapter::SourceFactEvidence::new(
                    crate::notify::PushKind::NewsFlashCritical,
                    event_id,
                    None,
                    headline,
                    source,
                    observed_at,
                    Some(source_published_on),
                    strength,
                    certainty,
                    stale,
                ) {
                    Ok(evidence) => {
                        crate::notify::push_presented_source_fact_v3(
                            presentation_token,
                            &text,
                            &evidence,
                        )
                        .await
                    }
                    Err(error) => {
                        log::error!(
                            "[NewsFlashGate][BR-137] critical source fact rejected: {error}"
                        );
                        crate::notify::PushOutcome::Denied(format!("source_fact_invalid:{error}"))
                    }
                };
                if outcome.is_pushed() {
                    n_critical += 1;
                    crate::push_templates::log_dispatcher_attempt("N-01", true, 1, "");
                } else {
                    log::info!("[NewsFlashGate] critical 未推 (治理): {:?}", outcome);
                    crate::push_templates::log_dispatcher_attempt(
                        "N-01",
                        false,
                        1,
                        &format!("governed_delivery_not_accepted:{outcome:?}"),
                    );
                }
            }
            FlashDecision::Aggregated { window, text } => {
                let presentation_token = match crate::presentation_registry::acquire_token(
                    "N-02-news-flash-aggregated",
                    crate::notify::PushKind::NewsFlashAggregated,
                    "news_flash_aggregate_dispatcher",
                    "assemble_news_flash_aggregated",
                ) {
                    Ok(token) => token,
                    Err(error) => {
                        log::error!("[NewsFlashGate][BR-196] aggregate token rejected: {error}");
                        crate::push_templates::log_dispatcher_attempt(
                            "N-02",
                            false,
                            1,
                            &format!("presentation_token_rejected:{error}"),
                        );
                        continue;
                    }
                };
                let outcome =
                    crate::notify::push_presented_v3(presentation_token, &text, Some(&window))
                        .await;
                if outcome.is_pushed() {
                    n_agg += 1;
                    crate::push_templates::log_dispatcher_attempt("N-02", true, 1, "");
                } else {
                    log::info!("[NewsFlashGate] {} 聚合未推 (治理): {:?}", window, outcome);
                    crate::push_templates::log_dispatcher_attempt(
                        "N-02",
                        false,
                        1,
                        &format!("governed_delivery_not_accepted:{outcome:?}"),
                    );
                }
            }
        }
    }
    (n_critical, n_agg)
}

/// BR-244 reservation owner. Every reservation is settled exactly once from
/// the dedicated physical/audit transaction result.
pub async fn push_flash_reservations(
    gate: &mut NewsFlashGate,
    reservations: Vec<FlashReservation>,
) -> (usize, usize) {
    let mut accepted = (0usize, 0usize);
    for reservation in reservations {
        let (is_critical, descriptor, kind) = match reservation.decision() {
            FlashDecision::Critical { .. } => (
                true,
                "N-01-news-flash-critical",
                crate::notify::PushKind::NewsFlashCritical,
            ),
            FlashDecision::Aggregated { .. } => (
                false,
                "N-02-news-flash-aggregated",
                crate::notify::PushKind::NewsFlashAggregated,
            ),
        };
        let outcome = match crate::presentation_registry::acquire_token(
            descriptor,
            kind,
            if is_critical {
                "news_flash_critical_dispatcher"
            } else {
                "news_flash_aggregate_dispatcher"
            },
            if is_critical {
                "assemble_news_flash_critical"
            } else {
                "assemble_news_flash_aggregated"
            },
        ) {
            Ok(token) => crate::notify::push_news_flash_v3(token, &reservation).await,
            Err(error) => crate::notify::NewsFlashNotifyOutcome::RejectedBeforeSink(format!(
                "presentation_token_rejected:{error}"
            )),
        };
        let (settlement, was_accepted, reason) = match outcome {
            crate::notify::NewsFlashNotifyOutcome::Terminal(terminal) => {
                let (was_accepted, reason) = match terminal.as_ref() {
                    stock_analysis::event::NewsFlashTerminalReceipt::Accepted(_) => {
                        (true, String::new())
                    }
                    stock_analysis::event::NewsFlashTerminalReceipt::DefinitivelyRejected(
                        receipt,
                    ) => (false, receipt.reason_code().to_owned()),
                    stock_analysis::event::NewsFlashTerminalReceipt::Uncertain(receipt) => {
                        (false, receipt.reason_code().to_owned())
                    }
                };
                (FlashSettlement::Terminal(terminal), was_accepted, reason)
            }
            crate::notify::NewsFlashNotifyOutcome::RejectedBeforeSink(reason) => (
                FlashSettlement::RolledBack {
                    reason: reason.clone(),
                },
                false,
                reason,
            ),
            crate::notify::NewsFlashNotifyOutcome::TerminalAuditFailed { reason } => (
                FlashSettlement::Uncertain {
                    reason: reason.clone(),
                },
                false,
                reason,
            ),
        };
        if let Err(error) = gate.settle(reservation, settlement) {
            log::error!("[NewsFlashGate][BR-244] settlement failed: {error:?}");
            crate::push_templates::log_dispatcher_attempt(
                if is_critical { "N-01" } else { "N-02" },
                false,
                1,
                &format!("settlement_failed:{error:?}"),
            );
            continue;
        }
        if !was_accepted {
            crate::push_templates::log_dispatcher_attempt(
                if is_critical { "N-01" } else { "N-02" },
                false,
                1,
                &reason,
            );
            continue;
        }
        if is_critical {
            accepted.0 += 1;
        } else {
            accepted.1 += 1;
        }
        crate::push_templates::log_dispatcher_attempt(
            if is_critical { "N-01" } else { "N-02" },
            true,
            1,
            "",
        );
    }
    accepted
}

/// BR-183 Track A 候选入池: 新闻标题 → LLM 提取受益个股 → pushed_stocks
/// 候选池 (复用 D-01/NewsCatalyst 已注册评分路径, intraday_monitor 直接消费)。
///
/// 当日一票一入: 入池前查 pushed_stocks 当日该 code 是否已有 (DB 级去重,
/// 吸取 T-03 内存去重重启丢失教训)。同票跨 tick 重复新闻不重复入池。
///
/// 红线 2.2: 无真实实时报价 (执行报价新鲜度 ≤5s) 的票不入池, 不造价格。
/// LLM 不可用 / 提取失败 → 出声 warn, 本轮不入池 (v15.x 静默路径可见)。
/// 返回 (入池数, 因无报价/已入池跳过数)。
pub async fn candidate_ingest_from_news(titles: &[String]) -> (usize, usize) {
    if titles.is_empty() {
        return (0, 0);
    }
    let registry = stock_analysis::llm::LlmRegistry::from_env();
    let Some(provider) = registry.select("ticker") else {
        log::warn!("[候选入池][BR-183] LLM role=ticker 无可用 provider (env 未配置), 本轮不入池");
        return (0, 0);
    };
    // 2026-08-08 实测: 20 条标题 → 输出超 8192 tokens 仍可能截断; 10 条
    // 覆盖单轮 tick 的头部新闻, 输出稳定收敛。
    let batch: Vec<String> = titles.iter().take(10).cloned().collect();
    let hits = match stock_analysis::llm::extract_tickers(provider, batch).await {
        Ok(hits) => hits,
        Err(error) => {
            log::warn!("[候选入池][BR-183] LLM 提取失败, 本轮不入池: {error}");
            return (0, 0);
        }
    };
    let mut recorded = 0usize;
    let mut skipped = 0usize;
    for hit in hits {
        if already_pooled_today(&hit.code) {
            skipped += 1;
            log::debug!(
                "[候选入池][BR-183] {}({}) 当日已入池, 跳过 (DB 级去重)",
                hit.name,
                hit.code
            );
            continue;
        }
        match stock_analysis::broker::execution_quote(&hit.code) {
            Ok(quote) => {
                let theme = json_escape(&hit.chain);
                let headline = json_escape(&hit.reason);
                let metric_json = format!(
                    "{{\"push_subkind\":\"NewsCatalyst\",\"theme\":\"{theme}\",\
                     \"headline\":\"{headline}\",\"llm_importance\":{}}}",
                    hit.importance
                );
                let outcome = stock_analysis::signal::push_recorder::record(
                    &stock_analysis::signal::push_recorder::PushRecordMeta {
                        code: hit.code.clone(),
                        name: hit.name.clone(),
                        push_kind: "D-01".to_string(),
                        push_price: quote.price,
                        metric_json,
                        source: "news_flash".to_string(),
                    },
                );
                match outcome {
                    Ok(_) => {
                        recorded += 1;
                        log::info!(
                            "[候选入池][BR-183] {}({}) 入池 importance={} chain={}",
                            hit.name,
                            hit.code,
                            hit.importance,
                            hit.chain
                        );
                    }
                    Err(error) => {
                        log::error!(
                            "[候选入池][BR-183] {}({}) pushed_stocks 写入失败: {error}",
                            hit.name,
                            hit.code
                        );
                    }
                }
            }
            Err(error) => {
                skipped += 1;
                log::warn!(
                    "[候选入池][BR-183] {}({}) 无真实实时报价, 跳过入池 (红线 2.2): {error}",
                    hit.name,
                    hit.code
                );
            }
        }
    }
    log::info!(
        "[候选入池][BR-183] recorded={} skipped={} titles={}",
        recorded,
        skipped,
        titles.len()
    );
    (recorded, skipped)
}

/// pushed_stocks 当日该 code 是否已有 (DB 级当日去重)。
fn already_pooled_today(code: &str) -> bool {
    let Ok(mut conn) = stock_analysis::database::DatabaseManager::get().get_conn() else {
        return false; // 连接失败 → 不拦截, 由写入路径暴露错误
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    diesel::sql_query(
        "SELECT COUNT(*) AS count FROM pushed_stocks \
         WHERE code = ? AND substr(push_time, 1, 10) = ?",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(&today)
    .get_result::<PooledCountRow>(&mut conn)
    .map(|row| row.count > 0)
    .unwrap_or_else(|error| {
        log::error!("[候选入池][BR-183] 当日去重查询失败: {error}");
        false
    })
}

#[derive(diesel::QueryableByName)]
struct PooledCountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// JSON 字符串转义 (候选 metric_json 内联字段用)
fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .chars()
        .take(120)
        .collect()
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::signal::market_event::{Direction, EventType};

    fn ev(id_seed: &str, strength: u8, certainty: u8) -> MarketEvent {
        let mut e = MarketEvent::new(
            EventType::Policy,
            format!("测试事件-{}", id_seed),
            None,
            Direction::Bull,
            strength,
            certainty,
        );
        e.event_id = format!("TEST_CODE_eid-{}", id_seed); // 固定 id 便于断言
        e.occurred_at = at(0, 0);
        e.provenance
            .push(stock_analysis::signal::market_event::SourceRef {
                provider: "TEST_CODE_NEWS_PROVIDER".to_string(),
                url: None,
                fetched_at: at(0, 0),
            });
        e
    }

    fn at(h: u32, m: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
            .date_naive()
            .and_hms_opt(h, m, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .unwrap()
    }

    fn at_second(h: u32, m: u32, s: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
            .date_naive()
            .and_hms_opt(h, m, s)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .unwrap()
    }

    fn projected(
        event: MarketEvent,
    ) -> stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent {
        let capability =
            stock_analysis::news::aggregator::raw_v2::NewsFlashProjectionTestCapability::bind()
                .expect("monitor unit test owns NewsFlash projection capability");
        let published_at = event.occurred_at.with_timezone(&chrono::Utc);
        let observed_at = event
            .provenance
            .first()
            .map(|source| source.fetched_at.with_timezone(&chrono::Utc))
            .unwrap_or(published_at);
        let batch_id = format!("TEST_CODE_BATCH_{}", event.event_id);
        stock_analysis::news::aggregator::raw_v2::NewsFlashProjectedEvent::test_fixture(
            &capability,
            event,
            "TEST_CODE_PROVIDER",
            "TEST_CODE_SOURCE",
            published_at,
            observed_at,
            &batch_id,
        )
    }

    fn authority_snapshot(
        business_date: chrono::NaiveDate,
        accepted_event_ids: &[&str],
        accepted_windows: &[&str],
        unresolved_reservations: &[&str],
        definitively_rejected_reservations: &[&str],
        next_attempt_ordinals: &[(&str, u32)],
    ) -> stock_analysis::event::NewsFlashAuthoritySnapshot {
        let capability = stock_analysis::event::NewsFlashAuthoritySnapshotTestCapability::bind()
            .expect("monitor unit test owns NewsFlash authority snapshot capability");
        stock_analysis::event::NewsFlashAuthoritySnapshot::test_fixture(
            &capability,
            business_date,
            accepted_event_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            accepted_windows
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            unresolved_reservations
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            definitively_rejected_reservations
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            next_attempt_ordinals
                .iter()
                .map(|(reservation, ordinal)| ((*reservation).to_owned(), *ordinal))
                .collect(),
        )
        .expect("TEST_CODE authority snapshot")
    }

    #[test]
    fn br244_source_only_gate_never_reserves_n01_without_authoritative_strength() {
        let now = at(10, 0);
        let mut gate = NewsFlashGate::new(now.date_naive());
        let forged_strength = projected(ev("critical-disabled", 100, 100));
        assert!(gate.reserve(&[forged_strength], now, 1, 20).is_empty());
        assert!(gate.critical_committed.is_empty());
        assert_eq!(
            NEWS_FLASH_CRITICAL_DISABLED_BANNER,
            "NewsFlashCritical disabled=no_authoritative_strength_provider"
        );
    }

    #[test]
    fn br244_quota_counts_only_committed_and_pending_only_reduces_concurrency_capacity() {
        let now = at(10, 0);
        let mut gate = NewsFlashGate::new(now.date_naive());
        gate.critical_committed
            .insert("TEST_CODE_event_accepted".to_owned());
        gate.critical_pending
            .insert("TEST_CODE_reservation_pending_1".to_owned());
        gate.critical_pending
            .insert("TEST_CODE_reservation_pending_2".to_owned());
        gate.unresolved_reservations
            .insert("TEST_CODE_reservation_uncertain".to_owned());

        assert_eq!(gate.critical_commit_quota_remaining(3), 2);
        assert_eq!(gate.critical_concurrency_capacity(3), 0);
        gate.critical_pending.clear();
        assert_eq!(gate.critical_concurrency_capacity(3), 2);
    }

    #[test]
    fn br244_gate_aggregate_window_is_half_open_at_90_91_and_300_seconds() {
        for second in [90_i64, 91] {
            let mut gate = NewsFlashGate::new(at(9, 0).date_naive());
            let source = projected(ev(&format!("window-{second}"), 0, 100));
            assert!(gate.reserve(&[source], at(9, 0), 80, 20).is_empty());
            let now = at(9, 30) + chrono::Duration::seconds(second);
            assert_eq!(gate.reserve(&[], now, 80, 20).len(), 1);
        }

        let mut gate = NewsFlashGate::new(at(9, 0).date_naive());
        let source = projected(ev("window-300", 0, 100));
        assert!(gate.reserve(&[source], at(9, 0), 80, 20).is_empty());
        assert!(gate.reserve(&[], at_second(9, 35, 0), 80, 20).is_empty());
    }

    #[test]
    fn br244_reservation_v2_exposes_canonical_authority_fields_and_binds_push_kind() {
        let now = at(9, 0);
        let source = projected(ev("reservation-v2", 0, 100));
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate
            .reserve(std::slice::from_ref(&source), now, 80, 20)
            .is_empty());
        let reservation = gate.reserve(&[], at(9, 31), 80, 20).pop().unwrap();
        assert_eq!(
            reservation.push_kind(),
            crate::notify::PushKind::NewsFlashAggregated.stable_template_id()
        );
        assert_eq!(reservation.business_date(), now.date_naive());
        assert_eq!(reservation.decision_key(), "window:09:30");
        assert_eq!(reservation.event_id(), None);
        assert_eq!(reservation.window(), Some("09:30"));
        assert_eq!(reservation.attempt_ordinal(), 1);
        assert_eq!(reservation.sources().len(), 1);
        assert_eq!(reservation.reservation_identity_sha256().len(), 64);
        assert_eq!(
            reservation.rendered_len(),
            match reservation.decision() {
                FlashDecision::Aggregated { text, .. } => text.len(),
                FlashDecision::Critical { .. } => panic!("SourceOnly gate must not reserve N-01"),
            }
        );

        let aggregate = make_reservation(
            1,
            now.date_naive(),
            "TEST_CODE_same_decision",
            None,
            None,
            vec![source.clone()],
            FlashDecision::Aggregated {
                window: "TEST_CODE_window".to_owned(),
                text: "TEST_CODE same render".to_owned(),
            },
        );
        let critical = make_reservation(
            2,
            now.date_naive(),
            "TEST_CODE_same_decision",
            None,
            None,
            vec![source],
            FlashDecision::Critical {
                event_id: "TEST_CODE_event".to_owned(),
                headline: "TEST_CODE headline".to_owned(),
                source: "TEST_CODE source".to_owned(),
                observed_at: now,
                source_published_on: now.date_naive(),
                stale: false,
                strength: 0,
                certainty: 100,
                text: "TEST_CODE same render".to_owned(),
            },
        );
        assert_ne!(
            aggregate.reservation_identity_sha256(),
            critical.reservation_identity_sha256(),
            "stable push kind is an explicit reservation-v2 identity field"
        );
    }

    #[test]
    fn br244_recovery_restores_accepted_event_and_window_authority() {
        let now = at(9, 0);
        let snapshot = authority_snapshot(
            now.date_naive(),
            &["TEST_CODE_event_accepted"],
            &["09:30"],
            &[],
            &[],
            &[],
        );
        let mut gate = NewsFlashGate::new(now.date_naive());
        gate.recover(&snapshot).unwrap();
        assert!(gate.critical_committed.contains("TEST_CODE_event_accepted"));
        assert_eq!(gate.critical_commit_quota_remaining(20), 19);
        assert_eq!(gate.window_state[0], WindowState::Committed);

        let source = projected(ev("accepted-window", 0, 100));
        assert!(gate.reserve(&[source], at(9, 31), 80, 20).is_empty());
    }

    #[test]
    fn br244_recovery_blocks_exact_unresolved_but_definitive_rejection_retries_next_ordinal() {
        let now = at(9, 0);
        let source = projected(ev("recovery-identity", 0, 100));
        let mut probe = NewsFlashGate::new(now.date_naive());
        assert!(probe
            .reserve(std::slice::from_ref(&source), now, 80, 20)
            .is_empty());
        let identity = probe
            .reserve(&[], at(9, 31), 80, 20)
            .pop()
            .unwrap()
            .reservation_identity_sha256()
            .to_owned();

        let unresolved = authority_snapshot(
            now.date_naive(),
            &[],
            &[],
            &[identity.as_str()],
            &[],
            &[(identity.as_str(), 2)],
        );
        let mut unresolved_gate = NewsFlashGate::new(now.date_naive());
        assert!(unresolved_gate
            .reserve_from_authority(
                &unresolved,
                std::slice::from_ref(&source),
                at(9, 31),
                80,
                20
            )
            .unwrap()
            .is_empty());
        let changed = projected(ev("recovery-changed-identity", 0, 100));
        let changed_reservations = unresolved_gate.reserve(&[changed], at(9, 32), 80, 20);
        assert_eq!(changed_reservations.len(), 1);
        assert_ne!(
            changed_reservations[0].reservation_identity_sha256(),
            identity
        );
        assert_eq!(changed_reservations[0].attempt_ordinal(), 1);

        let definitively_rejected = authority_snapshot(
            now.date_naive(),
            &[],
            &[],
            &[],
            &[identity.as_str()],
            &[(identity.as_str(), 2)],
        );
        let mut rejected_gate = NewsFlashGate::new(now.date_naive());
        let retry = rejected_gate
            .reserve_from_authority(&definitively_rejected, &[source], at(9, 31), 80, 20)
            .unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].reservation_identity_sha256(), identity);
        assert_eq!(retry[0].attempt_ordinal(), 2);
    }

    #[test]
    fn br244_gate_aggregate_rollback_retries_identical_binding_then_uncertain_stops() {
        let mut gate = NewsFlashGate::new(at(9, 0).date_naive());
        let sources = [70_u8, 60, 50]
            .into_iter()
            .enumerate()
            .map(|(index, strength)| projected(ev(&format!("agg-{index}"), strength, 50)))
            .collect::<Vec<_>>();
        assert!(gate.reserve(&sources, at(9, 0), 80, 20).is_empty());
        let first = gate.reserve(&[], at(9, 31), 80, 20).pop().unwrap();
        let first_binding = first.evidence_sha256().to_owned();
        gate.settle(
            first,
            FlashSettlement::RolledBack {
                reason: "TEST_CODE_SINK_REJECTED".to_owned(),
            },
        )
        .unwrap();
        let second = gate.reserve(&[], at(9, 32), 80, 20).pop().unwrap();
        assert_eq!(second.evidence_sha256(), first_binding);
        gate.settle(
            second,
            FlashSettlement::Uncertain {
                reason: "TEST_CODE_POST_SINK_UNKNOWN".to_owned(),
            },
        )
        .unwrap();
        assert!(gate.reserve(&[], at(9, 33), 80, 20).is_empty());

        let changed = projected(ev("agg-new-identity", 100, 50));
        let third = gate.reserve(&[changed], at(9, 34), 80, 20);
        assert_eq!(
            third.len(),
            1,
            "uncertain must block only the exact prior reservation identity"
        );
        assert_ne!(third[0].evidence_sha256(), first_binding);
    }

    /// AC34 + AC46: 阈值默认 80/certainty 60 门; 低分不推
    #[test]
    fn gate_critical_threshold_cannot_enable_source_only_n01() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let d = g.process(
            &[ev("a", 85, 70), ev("b", 85, 30), ev("c", 60, 90)],
            at(10, 0),
            80,
            20,
        );
        assert!(d.is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br244_source_only_n01_performs_zero_pushes() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;
        let mut gate = NewsFlashGate::new(at(10, 0).date_naive());
        let decisions = gate.process(&[ev("source-fact", 90, 90)], at(10, 0), 80, 20);

        assert_eq!(push_flash_decisions(decisions).await, (0, 0));
    }

    #[test]
    fn br137_stale_flash_is_excluded_from_critical_and_aggregate_buffer() {
        let now = at(10, 0);
        let mut stale = ev("stale-source-fact", 90, 90);
        stale.stale = true;
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate.process(&[stale], now, 80, 20).is_empty());
        assert!(gate.buffer.is_empty());
        assert!(gate.buffered_ids.is_empty());
    }

    #[test]
    fn br137_old_flash_is_rejected_even_when_upstream_stale_flag_is_false() {
        let now = at(10, 0);
        let old_time = now - chrono::Duration::days(1);
        let mut old = ev("old-source-fact", 70, 70);
        old.stale = false;
        old.occurred_at = old_time;
        old.provenance[0].fetched_at = old_time;
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate.process(&[old], now, 80, 20).is_empty());
        assert!(gate.buffer.is_empty());
        assert!(gate.buffered_ids.is_empty());
    }

    #[test]
    fn br137_malformed_flash_is_excluded_from_critical_and_aggregate_buffer() {
        let now = at(10, 0);
        let mut malformed = ev("malformed-source-fact", 101, 90);
        malformed.event_id.clear();
        malformed.full_title.clear();
        malformed.subject.clear();
        malformed.provenance.clear();
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate.process(&[malformed], now, 80, 20).is_empty());
        assert!(gate.buffer.is_empty());
        assert!(gate.buffered_ids.is_empty());
    }

    /// BR-082: event_id 当日去重
    #[test]
    fn gate_dedup_same_event_id() {
        let mut g = NewsFlashGate::new(at(9, 0).date_naive());
        let e = ev("dup", 90, 90);
        assert!(g
            .process(std::slice::from_ref(&e), at(9, 0), 80, 20)
            .is_empty());
        assert!(g.process(&[e], at(9, 1), 80, 20).is_empty());
        assert_eq!(g.buffer.len(), 1, "same event_id is buffered once");
    }

    #[test]
    fn br244_replayed_source_event_does_not_expand_buffer_or_repeat_window() {
        let mut gate = NewsFlashGate::new(at(9, 20).date_naive());
        let mut source_event = ev("br244-replay", 0, 100);
        source_event.direction = Direction::Neutral;
        source_event.occurred_at = at(9, 20);
        source_event.provenance[0].fetched_at = at(9, 20);

        assert!(gate
            .process(std::slice::from_ref(&source_event), at(9, 20), 80, 20)
            .is_empty());
        assert!(gate.process(&[source_event], at(9, 22), 80, 20).is_empty());
        assert_eq!(gate.buffer.len(), 1, "same event_id enters the buffer once");

        let first_window = gate.process(&[], at(9, 30), 80, 20);
        assert_eq!(first_window.len(), 1);
        assert!(matches!(
            &first_window[0],
            FlashDecision::Aggregated { window, .. } if window == "09:30"
        ));
        assert!(gate.process(&[], at(9, 31), 80, 20).is_empty());
    }

    #[test]
    fn br244_source_failure_audit_records_both_public_push_kinds_as_failures() {
        let source = include_str!("news_aggregator_init.rs");
        let helper = source
            .split("pub fn audit_news_flash_source_failure")
            .nth(1)
            .expect("BR-244 source failure audit helper")
            .split("pub async fn push_flash_decisions")
            .next()
            .expect("audit helper precedes dispatcher");
        assert!(helper.contains("stock_analysis::event::NewsFlashFailureAuditReceipt"));
        assert!(helper.contains("stock_analysis::event::NewsFlashFailureAuditInput"));
        assert!(helper.contains("publish_news_flash_failure("));
        assert!(helper.contains("provider: Some(failure.provider().wire_name().to_owned())"));
        assert!(helper.contains(".available_provider_wire()"));
        assert!(helper.contains("diagnostic_code: failure.diagnostic_code().to_owned()"));
        assert!(helper.contains("diagnostic: failure.diagnostic().to_owned()"));
        assert!(helper.contains("source_record_count"));
        assert!(helper.contains("batch_id: failure.batch_id().map(str::to_owned)"));
        assert!(helper.contains("record_id: failure.record_id().map(str::to_owned)"));
        assert!(!helper.contains("news_flash_failure_identity_hash("));
        assert!(helper.contains("\"N-01\""));
        assert!(helper.contains("\"N-02\""));
    }

    #[test]
    fn br244_batch_failure_diagnostic_is_utf8_safe_and_bounded() {
        let diagnostic = bounded_news_flash_failure_diagnostic(&"测".repeat(300));
        assert!(diagnostic.len() <= 512);
        assert!(diagnostic.ends_with(" [truncated]"));
        assert!(std::str::from_utf8(diagnostic.as_bytes()).is_ok());
    }

    /// BR-082: 每日上限
    #[test]
    fn gate_daily_cap() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        g.critical_committed.extend(
            ["TEST_CODE_a", "TEST_CODE_b"]
                .into_iter()
                .map(str::to_owned),
        );
        g.unresolved_reservations
            .insert("TEST_CODE_uncertain".to_owned());
        assert_eq!(g.critical_commit_quota_remaining(3), 1);
    }

    /// AC35: 窗口触发一次/日 + Top3 按 strength 降序
    #[test]
    fn gate_window_fires_once_with_top3() {
        let mut g = NewsFlashGate::new(at(9, 0).date_naive());
        // 9:00 喂 4 条低分事件 (进 buffer, 不 critical)
        let events: Vec<MarketEvent> = [40u8, 70, 55, 60]
            .iter()
            .enumerate()
            .map(|(i, &s)| ev(&format!("w{}", i), s, 50))
            .collect();
        assert!(g.process(&events, at(9, 0), 80, 20).is_empty());
        // 9:31 → 触发 09:30 窗口
        let d1 = g.process(&[], at(9, 31), 80, 20);
        assert_eq!(d1.len(), 1);
        match &d1[0] {
            FlashDecision::Aggregated { window: w, text } => {
                assert_eq!(w, "09:30");
                assert!(text.contains("强度70"), "Top1 应是 strength=70: {}", text);
                assert_eq!(text.matches("测试事件").count(), 3, "只取 Top3");
            }
            other => panic!("应为 Aggregated, got {:?}", other),
        }
        // 9:33 再 tick → 同窗口不重复触发
        assert!(g.process(&[], at(9, 33), 80, 20).is_empty(), "窗口当日一次");
    }

    /// Track A: 空标题列表 → 不入池 (0, 0), 不触发 LLM
    #[tokio::test]
    async fn candidate_ingest_empty_titles_is_noop() {
        let (recorded, skipped) = candidate_ingest_from_news(&[]).await;
        assert_eq!((recorded, skipped), (0, 0));
    }

    /// Track A: JSON 转义不产生畸形 metric_json
    #[test]
    fn json_escape_handles_quotes_and_backslashes() {
        let escaped = json_escape("PCB \"涨价\" \\ 事件");
        assert_eq!(escaped, "PCB \\\"涨价\\\" \\\\ 事件");
    }

    /// 红线 2.2: 窗口无事件不臆造推送
    #[test]
    fn gate_window_empty_buffer_no_push() {
        let mut g = NewsFlashGate::new(at(11, 0).date_naive());
        assert!(g.process(&[], at(11, 30), 80, 20).is_empty());
    }

    /// AC46: config 默认值
    #[test]
    fn news_config_defaults() {
        let cfg = stock_analysis::config::MonitorConfig::default();
        assert_eq!(cfg.news_critical_score_threshold, 80);
        assert_eq!(cfg.news_max_critical_per_day, 20);
    }

    #[test]
    fn global_news_pipeline_reports_the_fixed_real_provider_registry() {
        let first = init_global_news_pipeline();
        let second = init_global_news_pipeline();
        assert_eq!(first.feed_count, 4);
        assert_eq!(first, second);
        assert_eq!(first.registered_feed_set_sha256.len(), 64);
    }

    #[tokio::test]
    async fn raw_fetch_rejects_zero_limit_before_provider_work() {
        let error = fetch_raw_global_news_batch(0)
            .await
            .expect_err("TEST_CODE zero limit must fail before provider work");
        assert!(matches!(error, RawNewsAcquisitionError::InvalidLimit(0)));
    }
}
