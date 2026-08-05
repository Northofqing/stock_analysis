//! Registered business rules: BR-005, BR-048, BR-137, BR-160, BR-192.
//! v14_adapter.rs — v14.2 七层架构与 v13 推送链路的桥接层 (b011 修复版)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.4 + b-009 R-4 落地.
//!
//! b011 P0-1/P0-2/P1-2 修复后的职责 (两段式, 取代旧 v14_dispatch 单函数):
//!   1. `v14_gate`   — 投递前闸门: L4 dedup (真实 (kind,code)+冷却窗口) + L5 governance.
//!      Deny 也写 L7 留痕 (sink="none", pushed=false).
//!   2. `v14_record_delivery` — 投递后记录: 把**真实**投递结果 (成功与否 + 实际通道)
//!      写入 L7 push_analytics. sink_name 不再是入口硬编码字面量.
//!
//! 旧版问题 (b011 实证, 已修):
//!   - sink_name 入口硬编码 "wechat", 实际走飞书 → analytics 全表假数据
//!   - pushed 由 governance 批准推导, sink 失败也记 1
//!   - 附带一次 ConsoleSink 假路由 + 同步 block_on 开销 (纯影子, 已删)
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2: 不静默填补, gate/record 结果必须显式 log
//!   - b-009 R-4: v13 入口强制走 v14.2 dispatcher (notify::push_governor_inner 调本模块)
//!
//! 调用链:
//!   notify::push_governor{,_v3}(text, kind[, code])
//!     -> v14_gate(kind, code)                 [L4 dedup + L5 governance]
//!     -> [Approved] notify::push_wechat(text) [真实投递: 飞书/微信/dry-run]
//!     -> v14_record_delivery(...)             [L7 记真实 sink + 真实结果]

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{Local, NaiveDate, TimeZone, Timelike};
use stock_analysis::push_l1::{
    NewsCatalystPayload, PostSessionReviewPayload, Severity, SignalEvent, SignalPayload,
    SignalSource,
};
use stock_analysis::push_l2::{DataMode, RenderedText, TemplateMetadata};
use stock_analysis::push_l4::{Dispatcher, ReserveOutcome};
use stock_analysis::push_l5::{GovernanceContext, GovernanceDecision, GovernanceEngine};
use stock_analysis::push_l7::{build_analytics, AnalyticsStore, SqliteStore};

use super::notify::{CooldownScope, PushKind};

/// v14.2 全局 stack (OnceLock 单例)
pub struct V14Stack {
    /// b013 P2-13: 改 RwLock (Dispatcher 现在 &self, 读路径无锁竞争)
    pub dispatcher: std::sync::RwLock<Dispatcher>,
    pub governance: GovernanceEngine,
    pub store: Mutex<SqliteStore>,
}

static V14_STACK: OnceLock<Result<V14Stack, String>> = OnceLock::new();

/// 构造（或获取）全局 v14.2 stack 单例
pub fn v14_stack() -> Result<&'static V14Stack, String> {
    let initialized = V14_STACK.get_or_init(|| {
        let test_mode = stock_analysis::risk::env_guard::current_env()
            == stock_analysis::risk::env_guard::TradingEnv::Test;
        let store_path = if test_mode {
            std::path::PathBuf::from("data/test/push_analytics.db")
        } else {
            std::path::PathBuf::from("data/push_analytics.db")
        };
        let parent = store_path
            .parent()
            .ok_or_else(|| format!("L7 store path has no parent: {}", store_path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create L7 directory {}: {error}", parent.display()))?;
        let store = SqliteStore::open(&store_path).map_err(|error| {
            format!("open persistent L7 store {}: {error}", store_path.display())
        })?;
        log::info!("[v14.2][BR-113] SqliteStore={}", store_path.display());
        Ok(V14Stack {
            dispatcher: std::sync::RwLock::new(Dispatcher::new()),
            governance: GovernanceEngine::new(),
            store: Mutex::new(store),
        })
    });
    initialized.as_ref().map_err(Clone::clone)
}

/// 闸门结果 — Approved 携带 event 供投递后 v14_record_delivery 关联
#[derive(Debug, Clone)]
pub enum V14Gate {
    Approved(Box<SignalEvent>),
    Deduped,
    Denied(String),
}

/// BR-137: source-self-contained news evidence allowed to use the narrow
/// DataMode::Down profile. Callers cannot construct fields directly and must
/// pass all provenance checks before reaching governance.
#[derive(Debug, Clone)]
pub struct SourceFactEvidence {
    kind: PushKind,
    governance_identity: String,
    security_code: Option<String>,
    headline: String,
    source: String,
    observed_at: chrono::DateTime<Local>,
    source_published_on: chrono::NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFactError {
    KindNotAllowed(PushKind),
    MissingGovernanceIdentity,
    MissingSecurityCode(PushKind),
    UnexpectedSecurityCode(PushKind),
    InvalidSecurityCode,
    MissingHeadline,
    MissingSource,
    MissingPublishedDate,
    StrengthOutOfRange(u8),
    CertaintyOutOfRange(u8),
    Stale,
    FutureObservedAt,
    FuturePublishedDate,
}

impl std::fmt::Display for SourceFactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KindNotAllowed(kind) => {
                write!(formatter, "PushKind::{kind:?} is not a source fact")
            }
            Self::MissingGovernanceIdentity => {
                formatter.write_str("source fact governance identity is missing")
            }
            Self::MissingSecurityCode(kind) => {
                write!(formatter, "PushKind::{kind:?} requires a security code")
            }
            Self::UnexpectedSecurityCode(kind) => {
                write!(
                    formatter,
                    "PushKind::{kind:?} must not carry a security code"
                )
            }
            Self::InvalidSecurityCode => formatter.write_str("source fact security code rejected"),
            Self::MissingHeadline => formatter.write_str("source fact headline is missing"),
            Self::MissingSource => formatter.write_str("source fact source is missing"),
            Self::MissingPublishedDate => {
                formatter.write_str("source fact provider publication date is missing")
            }
            Self::StrengthOutOfRange(value) => {
                write!(formatter, "source fact strength out of range: {value}")
            }
            Self::CertaintyOutOfRange(value) => {
                write!(formatter, "source fact certainty out of range: {value}")
            }
            Self::Stale => formatter.write_str("source fact is stale"),
            Self::FutureObservedAt => {
                formatter.write_str("source fact observed_at is in the future")
            }
            Self::FuturePublishedDate => {
                formatter.write_str("source fact provider publication date is in the future")
            }
        }
    }
}

impl std::error::Error for SourceFactError {}

impl SourceFactEvidence {
    #[allow(
        clippy::too_many_arguments,
        reason = "BR-137 evidence constructor makes every source fact field explicit"
    )]
    pub fn new(
        kind: PushKind,
        governance_identity: String,
        security_code: Option<String>,
        headline: String,
        source: String,
        observed_at: chrono::DateTime<Local>,
        source_published_on: Option<chrono::NaiveDate>,
        strength: u8,
        certainty: u8,
        stale: bool,
    ) -> Result<Self, SourceFactError> {
        if !matches!(
            kind,
            PushKind::Announcement
                | PushKind::PolicyHit
                | PushKind::EarningsBeat
                | PushKind::EarningsMiss
                | PushKind::AnalystUpgrade
                | PushKind::NewsFlashCritical
        ) {
            return Err(SourceFactError::KindNotAllowed(kind));
        }
        if governance_identity.trim().is_empty() {
            return Err(SourceFactError::MissingGovernanceIdentity);
        }
        if headline.trim().is_empty() {
            return Err(SourceFactError::MissingHeadline);
        }
        if source.trim().is_empty() {
            return Err(SourceFactError::MissingSource);
        }
        match kind {
            PushKind::Announcement
            | PushKind::EarningsBeat
            | PushKind::EarningsMiss
            | PushKind::AnalystUpgrade => {
                let code = security_code
                    .as_deref()
                    .filter(|code| !code.trim().is_empty())
                    .ok_or(SourceFactError::MissingSecurityCode(kind))?;
                stock_analysis::risk::env_guard::validate_symbol_for_current_env(code)
                    .map_err(|_| SourceFactError::InvalidSecurityCode)?;
            }
            PushKind::PolicyHit => {
                if let Some(code) = security_code.as_deref() {
                    if code.trim().is_empty() {
                        return Err(SourceFactError::MissingSecurityCode(kind));
                    }
                    stock_analysis::risk::env_guard::validate_symbol_for_current_env(code)
                        .map_err(|_| SourceFactError::InvalidSecurityCode)?;
                }
            }
            PushKind::NewsFlashCritical => {
                if security_code.is_some() {
                    return Err(SourceFactError::UnexpectedSecurityCode(kind));
                }
            }
            _ => unreachable!("source-fact whitelist checked above"),
        }
        if strength > 100 {
            return Err(SourceFactError::StrengthOutOfRange(strength));
        }
        if certainty > 100 {
            return Err(SourceFactError::CertaintyOutOfRange(certainty));
        }
        if stale {
            return Err(SourceFactError::Stale);
        }
        if observed_at > Local::now() {
            return Err(SourceFactError::FutureObservedAt);
        }
        let source_published_on =
            source_published_on.ok_or(SourceFactError::MissingPublishedDate)?;
        let today = Local::now().date_naive();
        if source_published_on > today {
            return Err(SourceFactError::FuturePublishedDate);
        }
        if source_published_on < today {
            return Err(SourceFactError::Stale);
        }
        Ok(Self {
            kind,
            governance_identity,
            security_code,
            headline,
            source,
            observed_at,
            source_published_on,
        })
    }

    pub fn kind(&self) -> PushKind {
        self.kind
    }

    pub fn security_code(&self) -> Option<&str> {
        self.security_code.as_deref()
    }
}

/// BR-160 immutable source-batch binding for A-10. This is deliberately
/// narrower than the generic governor: only a committed ChainIntelligenceBatch
/// may use account-independent governance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBatchEvidence {
    kind: PushKind,
    business_date: NaiveDate,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
    batch_id: String,
    content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceBatchError {
    KindNotAllowed(PushKind),
    FutureBusinessDate(NaiveDate),
    ObservedBeforeBusinessDate,
    FutureObservedAt,
    InvalidBatchId,
    InvalidContentHash,
}

impl std::fmt::Display for SourceBatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KindNotAllowed(kind) => {
                write!(
                    formatter,
                    "PushKind::{kind:?} is not an admitted source batch"
                )
            }
            Self::FutureBusinessDate(date) => {
                write!(
                    formatter,
                    "source batch business date is in the future: {date}"
                )
            }
            Self::ObservedBeforeBusinessDate => {
                formatter.write_str("source batch observation predates its business date")
            }
            Self::FutureObservedAt => {
                formatter.write_str("source batch observation time is in the future")
            }
            Self::InvalidBatchId => formatter.write_str("source batch identity is invalid"),
            Self::InvalidContentHash => formatter.write_str("source batch content hash is invalid"),
        }
    }
}

impl std::error::Error for SourceBatchError {}

impl SourceBatchEvidence {
    pub fn new(
        kind: PushKind,
        business_date: NaiveDate,
        observed_at: chrono::DateTime<chrono::FixedOffset>,
        batch_id: String,
        content_hash: String,
    ) -> Result<Self, SourceBatchError> {
        if kind != PushKind::CatalystReview {
            return Err(SourceBatchError::KindNotAllowed(kind));
        }
        if business_date > Local::now().date_naive() {
            return Err(SourceBatchError::FutureBusinessDate(business_date));
        }
        if observed_at.with_timezone(&Local).date_naive() < business_date {
            return Err(SourceBatchError::ObservedBeforeBusinessDate);
        }
        if observed_at > Local::now().fixed_offset() {
            return Err(SourceBatchError::FutureObservedAt);
        }
        if batch_id.trim() != batch_id
            || !batch_id.starts_with("chain-batch:")
            || batch_id.len() <= "chain-batch:".len()
        {
            return Err(SourceBatchError::InvalidBatchId);
        }
        if content_hash.len() != 64
            || !content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(SourceBatchError::InvalidContentHash);
        }
        Ok(Self {
            kind,
            business_date,
            observed_at,
            batch_id,
            content_hash,
        })
    }

    pub fn kind(&self) -> PushKind {
        self.kind
    }

    pub fn business_date(&self) -> NaiveDate {
        self.business_date
    }

    pub fn observed_at(&self) -> chrono::DateTime<chrono::FixedOffset> {
        self.observed_at
    }

    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    fn governance_identity(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.business_date,
            self.observed_at.to_rfc3339(),
            self.batch_id,
            self.content_hash
        )
    }
}

/// 投递前闸门: L4 dedup + L5 governance (b011 P0-2)
///
/// `code`: 票级冷却键; None 时 PerTicket 类 kind 的冷却归模板层 memo, L4 不拦.
/// `sub_kind`: stable delivery subtype metadata used by prepared generic
/// internals. Counted DailyReport variants do not enter through this public
/// kind-only gate; BR-192 requires `v14_gate_counted_binding`.
pub fn v14_gate(kind: PushKind, code: Option<&str>) -> V14Gate {
    v14_gate_with_sub_kind(kind, code, None, None)
}

/// BR-196 exact-six governance gate.
///
/// The typed dispatch owns the only admitted tuple set and an
/// invocation-scoped Shanghai-noon clock. No process-global quiet-hour state
/// is changed, so ordinary test and production gates keep their real policy.
pub(super) fn v14_gate_br196_smoke(
    dispatch: &crate::br196_test_delivery::GovernanceSmokeDispatch<'_>,
) -> V14Gate {
    if let Err(reason) = dispatch.validate_for_use() {
        return V14Gate::Denied(reason);
    }
    let kind = dispatch.push_kind();
    let code = dispatch.code();
    let event = signal_event_for_kind(kind, code);
    let profile = default_profile_for_kind(kind);
    let context = match governance_ctx_for_banner_at(dispatch.governance_now(), false) {
        Ok(context) => context,
        Err(error) => return V14Gate::Denied(error),
    };
    v14_gate_prepared(V14PreparedGate {
        kind,
        has_governance_identity: code.is_some(),
        sub_kind: None,
        cooldown_override_secs: None,
        event,
        profile,
        context_source: GovernanceContextSource::CombinedAccount,
        context_override: Some(context),
    })
}

/// Internal sub-kind-aware gate shared by the generic `v14_gate` adapter.
/// Counted delivery must use `v14_gate_counted_binding` with immutable evidence.
pub fn v14_gate_with_sub_kind(
    kind: PushKind,
    code: Option<&str>,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> V14Gate {
    let event = signal_event_for_kind(kind, code);
    let profile = default_profile_for_kind(kind);
    v14_gate_prepared(V14PreparedGate {
        kind,
        has_governance_identity: code.is_some(),
        sub_kind,
        cooldown_override_secs,
        event,
        profile,
        context_source: GovernanceContextSource::CombinedAccount,
        context_override: None,
    })
}

/// BR-192 sole L5 gate for a counted delivery with explicit immutable source
/// binding. The governance event is stable for the caller-supplied schedule
/// occurrence and is never reused as durable evidence.
pub fn v14_gate_counted_binding(
    kind: PushKind,
    code: Option<&str>,
    sub_kind: Option<&str>,
    schedule_occurrence_identity: &str,
    business_date: NaiveDate,
) -> V14Gate {
    if !crate::durable_delivery_runtime::is_counted_kind(kind) {
        return V14Gate::Denied("counted_binding_kind_required".to_owned());
    }
    if schedule_occurrence_identity.trim().is_empty() {
        return V14Gate::Denied("counted_schedule_occurrence_required".to_owned());
    }
    let Some(naive_timestamp) = business_date.and_hms_opt(12, 0, 0) else {
        return V14Gate::Denied("counted_business_date_invalid".to_owned());
    };
    let Some(timestamp) = Local.from_local_datetime(&naive_timestamp).single() else {
        return V14Gate::Denied("counted_business_date_local_time_invalid".to_owned());
    };
    let (source, kind_str, severity) = map_push_kind(kind);
    let mut event = SignalEvent::new(
        source,
        kind_str,
        code.map(str::to_owned),
        timestamp,
        signal_payload_for_kind(kind),
        severity,
    );
    event.event_id =
        stock_analysis::push_l1::make_source_fact_event_id(kind_str, schedule_occurrence_identity);
    let profile = default_profile_for_kind(kind);
    v14_gate_prepared(V14PreparedGate {
        kind,
        has_governance_identity: code.is_some(),
        sub_kind,
        cooldown_override_secs: None,
        event,
        profile,
        context_source: GovernanceContextSource::CountedCombinedAccount,
        context_override: None,
    })
}

/// BR-194 narrow L5 gate for the canonical R-04 provider binding.
pub fn v14_gate_counted_source_only_binding(
    kind: PushKind,
    binding: &crate::durable_delivery_runtime::CountedDeliveryBinding,
) -> V14Gate {
    if kind != PushKind::ReviewLhb {
        return V14Gate::Denied("counted_source_only_kind_not_allowed".to_owned());
    }
    if let Err(reason) = binding.validate_r04_source_only() {
        return V14Gate::Denied(reason.to_owned());
    }
    let Some(naive_timestamp) = binding.business_date().and_hms_opt(12, 0, 0) else {
        return V14Gate::Denied("counted_source_only_binding_invalid".to_owned());
    };
    let Some(timestamp) = Local.from_local_datetime(&naive_timestamp).single() else {
        return V14Gate::Denied("counted_source_only_binding_invalid".to_owned());
    };
    let (source, kind_str, severity) = map_push_kind(kind);
    let mut event = SignalEvent::new(
        source,
        kind_str,
        binding.governance_code().map(str::to_owned),
        timestamp,
        signal_payload_for_kind(kind),
        severity,
    );
    event.event_id = stock_analysis::push_l1::make_source_fact_event_id(
        kind_str,
        binding.schedule_occurrence_identity(),
    );
    v14_gate_prepared(V14PreparedGate {
        kind,
        has_governance_identity: binding.governance_code().is_some(),
        sub_kind: None,
        cooldown_override_secs: None,
        event,
        profile: counted_source_only_profile(kind),
        context_source: GovernanceContextSource::CountedSourceOnly,
        context_override: None,
    })
}

/// BR-199 closed L5 gate for the canonical public-only R-08 binding.
pub fn v14_gate_r08_source_only_binding(
    kind: PushKind,
    binding: &crate::durable_delivery_runtime::CountedDeliveryBinding,
) -> V14Gate {
    if kind != PushKind::EventCalendar {
        return V14Gate::Denied("counted_r08_source_only_kind_not_allowed".to_owned());
    }
    if let Err(reason) = binding.validate_r08_public_source_only() {
        return V14Gate::Denied(reason.to_owned());
    }
    let Some(naive_timestamp) = binding.business_date().and_hms_opt(12, 0, 0) else {
        return V14Gate::Denied("counted_r08_source_only_binding_invalid".to_owned());
    };
    let Some(timestamp) = Local.from_local_datetime(&naive_timestamp).single() else {
        return V14Gate::Denied("counted_r08_source_only_binding_invalid".to_owned());
    };
    let (source, kind_str, severity) = map_push_kind(kind);
    let mut event = SignalEvent::new(
        source,
        kind_str,
        None,
        timestamp,
        signal_payload_for_kind(kind),
        severity,
    );
    event.event_id = stock_analysis::push_l1::make_source_fact_event_id(
        kind_str,
        binding.schedule_occurrence_identity(),
    );
    v14_gate_prepared(V14PreparedGate {
        kind,
        has_governance_identity: false,
        sub_kind: None,
        cooldown_override_secs: None,
        event,
        profile: counted_source_only_profile(kind),
        context_source: GovernanceContextSource::CountedSourceOnly,
        context_override: None,
    })
}

/// BR-137 narrow gate for validated source facts. Generic callers cannot
/// supply a relaxed profile or a prepared SignalEvent.
pub fn v14_gate_source_fact(evidence: &SourceFactEvidence) -> V14Gate {
    let event = signal_event_for_source_fact(evidence);
    v14_gate_prepared(V14PreparedGate {
        kind: evidence.kind,
        has_governance_identity: true,
        sub_kind: None,
        cooldown_override_secs: None,
        event,
        profile: source_fact_profile(evidence.kind),
        context_source: GovernanceContextSource::SourceFact,
        context_override: None,
    })
}

/// BR-160 sole uncounted source-batch gate. Account mode is not applicable to
/// the already committed A-10 batch; global data health remains observable but
/// does not replace the batch's own admission contract.
pub fn v14_gate_source_batch(evidence: &SourceBatchEvidence) -> V14Gate {
    let Some(naive_timestamp) = evidence.business_date.and_hms_opt(12, 0, 0) else {
        return V14Gate::Denied("source_batch_business_date_invalid".to_owned());
    };
    let Some(timestamp) = Local.from_local_datetime(&naive_timestamp).single() else {
        return V14Gate::Denied("source_batch_business_date_local_time_invalid".to_owned());
    };
    let (source, kind_str, severity) = map_push_kind(evidence.kind);
    let mut event = SignalEvent::new(
        source,
        kind_str,
        None,
        timestamp,
        SignalPayload::PostSessionReview(PostSessionReviewPayload {
            review_type: Some("catalyst_review".to_owned()),
            summary: Some(format!(
                "batch_id={};content_sha256={}",
                evidence.batch_id, evidence.content_hash
            )),
        }),
        severity,
    );
    event.event_id = stock_analysis::push_l1::make_source_fact_event_id(
        kind_str,
        &evidence.governance_identity(),
    );
    v14_gate_prepared(V14PreparedGate {
        kind: evidence.kind,
        has_governance_identity: true,
        sub_kind: None,
        cooldown_override_secs: None,
        event,
        profile: source_batch_profile(evidence.kind),
        context_source: GovernanceContextSource::SourceBatch,
        context_override: None,
    })
}

#[derive(Clone, Copy)]
enum GovernanceContextSource {
    CombinedAccount,
    CountedCombinedAccount,
    SourceFact,
    SourceBatch,
    CountedSourceOnly,
}

struct V14PreparedGate<'a> {
    kind: PushKind,
    has_governance_identity: bool,
    sub_kind: Option<&'a str>,
    cooldown_override_secs: Option<u32>,
    event: SignalEvent,
    profile: TemplateMetadata,
    context_source: GovernanceContextSource,
    context_override: Option<GovernanceContext>,
}

fn v14_gate_prepared(request: V14PreparedGate<'_>) -> V14Gate {
    let V14PreparedGate {
        kind,
        has_governance_identity,
        sub_kind,
        cooldown_override_secs,
        event,
        profile,
        context_source,
        context_override,
    } = request;
    let source_fact_context = matches!(
        context_source,
        GovernanceContextSource::SourceFact | GovernanceContextSource::SourceBatch
    );
    if crate::durable_delivery_runtime::is_counted_kind(kind)
        && !matches!(
            context_source,
            GovernanceContextSource::CountedCombinedAccount
                | GovernanceContextSource::CountedSourceOnly
        )
    {
        return V14Gate::Denied("counted_binding_required".to_owned());
    }
    let stack = match v14_stack() {
        Ok(stack) => stack,
        Err(error) => {
            log::error!("[v14.2][BR-113] {error}");
            return V14Gate::Denied("analytics_store_unavailable".to_string());
        }
    };
    let (_, kind_str, _) = map_push_kind(kind);

    // b013 review P0-3: dedup 仅在 governance Approved + 投递成功后才插 (避免 Denied/failed 误锁窗口).
    // 旧实现: dedup 先于 governance, 5min 静默期内的 Denied retry 全被挡到下次窗口.
    // b013 review P1-11: Deduped 也写 L7 (sink="deduped", pushed=false), 让归因分析看得到被治理掉的数量.

    // L5 governance 先判 (data_mode/frozen/quiet_hour/daily_limit)
    let mut ctx = match context_override
        .map(Ok)
        .unwrap_or_else(|| match context_source {
            GovernanceContextSource::CombinedAccount
            | GovernanceContextSource::CountedCombinedAccount => current_governance_ctx(),
            GovernanceContextSource::SourceFact | GovernanceContextSource::SourceBatch => {
                current_source_fact_governance_ctx()
            }
            GovernanceContextSource::CountedSourceOnly => {
                current_counted_source_only_governance_ctx()
            }
        }) {
        Ok(ctx) => ctx,
        Err(error) => {
            log::error!("[v14.2][BR-113] {error}");
            return V14Gate::Denied("governance_context_unavailable".to_string());
        }
    };
    if profile.max_per_user_per_day.is_some() {
        let count_result = {
            let store = match lock_store(stack) {
                Ok(store) => store,
                Err(error) => {
                    log::error!("[v14.2][BR-113] {error}");
                    return V14Gate::Denied("analytics_store_lock_unavailable".to_string());
                }
            };
            store.count_today_pushed_for_user_and_template(
                "default",
                kind_str,
                ctx.now.date_naive(),
            )
        };
        match count_result {
            Ok(count) => match u32::try_from(count) {
                Ok(count) => ctx.today_pushed_count = count,
                Err(_) => {
                    return deny_for_unavailable_daily_count(
                        stack,
                        &event,
                        kind_str,
                        &ctx,
                        format!("invalid daily count {count}"),
                    );
                }
            },
            Err(error) => {
                return deny_for_unavailable_daily_count(
                    stack,
                    &event,
                    kind_str,
                    &ctx,
                    error.to_string(),
                );
            }
        }
    }
    let decision = stack.governance.check(&profile, &event, &ctx);
    if !decision.is_approve() {
        let reason = decision.deny_reason().unwrap_or("unknown").to_string();
        log::info!("[v14.2] L5 deny: PushKind={:?} reason={}", kind, reason);
        // L7 留痕
        let analytics = build_analytics(
            &event,
            kind_str,
            1,
            ctx.data_mode,
            decision,
            None,
            false,
            "none",
            "default",
            vec![],
        );
        if let Err(error) = lock_store(stack)
            .and_then(|store| store.record(&analytics).map_err(|error| error.to_string()))
        {
            log::error!("[v14.2][BR-113] L5 deny audit failed: {error}");
            return V14Gate::Denied("analytics_audit_unavailable".to_string());
        }
        return V14Gate::Denied(reason);
    }

    // BR-192: counted delivery retains L5 governance but has exactly one
    // reservation/dedup/cooldown owner.  The durable coordinator performs the
    // authoritative admission transaction after this non-delivery gate.
    if crate::durable_delivery_runtime::is_counted_kind(kind) {
        return V14Gate::Approved(Box::new(event));
    }

    // L4 dedup (v15.1 A3: 用 reserve() 只检查不插入, 投递成功后由 push_governor_inner 调 commit())
    // v17.6 §5.1: sub_kind 参与 dedup key (per-sub_kind 独立窗口)
    let cooldown = dedup_cooldown(
        kind,
        has_governance_identity,
        source_fact_context,
        cooldown_override_secs,
    );
    let outcome = match lock_dispatcher(stack) {
        Ok(dispatcher) if source_fact_context => {
            dispatcher.reserve_with_identity(&event, Some(&event.event_id), cooldown, sub_kind)
        }
        Ok(dispatcher) => dispatcher.reserve(&event, cooldown, sub_kind),
        Err(error) => {
            log::error!("[v14.2][BR-113] {error}");
            return V14Gate::Denied("dedup_lock_unavailable".to_string());
        }
    };
    if let ReserveOutcome::Deduped = outcome {
        log::info!(
            "[v14.2] L4 dedup: PushKind={:?} sub_kind={:?} (reserved-only, no insert yet)",
            kind,
            sub_kind
        );
        // b013 P1-11: dedup 留痕
        let analytics = build_analytics(
            &event,
            kind_str,
            1,
            ctx.data_mode,
            GovernanceDecision::Approve,
            None,
            false,
            "deduped",
            "default",
            vec![],
        );
        if let Err(error) = lock_store(stack)
            .and_then(|store| store.record(&analytics).map_err(|error| error.to_string()))
        {
            log::error!("[v14.2][BR-113] L4 dedup audit failed: {error}");
            return V14Gate::Denied("analytics_audit_unavailable".to_string());
        }
        return V14Gate::Deduped;
    }

    V14Gate::Approved(Box::new(event))
}

fn lock_dispatcher(
    stack: &'static V14Stack,
) -> Result<std::sync::RwLockReadGuard<'static, Dispatcher>, String> {
    stack
        .dispatcher
        .read()
        .map_err(|_| "dispatcher RwLock poisoned; governance rejected".to_string())
}

fn dedup_cooldown(
    kind: PushKind,
    has_code: bool,
    source_fact: bool,
    cooldown_override_secs: Option<u32>,
) -> Option<Duration> {
    match kind.cooldown_scope() {
        // BR-137: legacy Announcement remains externally deduplicated, while
        // its normalized source-fact path uses provider event_id as the L4 key.
        CooldownScope::External if !source_fact => None,
        CooldownScope::PerTicket if !has_code => None,
        _ => cooldown_override_secs
            .or_else(|| kind.cooldown_secs())
            .map(|secs| Duration::from_secs(u64::from(secs))),
    }
}

/// v15.1 A3: push 成功后 commit dedup entry (由 push_governor_inner 调用)
pub fn commit_dedup_for_event(
    event: &SignalEvent,
    kind: PushKind,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> Result<(), String> {
    if crate::durable_delivery_runtime::is_counted_kind(kind) {
        return Err(format!(
            "BR-192 counted PushKind::{kind:?} cannot use legacy dedup commit"
        ));
    }
    let stack = v14_stack()?;
    let source_fact = is_source_fact_signal(kind, event);
    let cooldown = dedup_cooldown(
        kind,
        source_fact || event.code.is_some(),
        source_fact,
        cooldown_override_secs,
    );
    let dispatcher = lock_dispatcher(stack)?;
    if source_fact {
        dispatcher.commit_with_identity(event, Some(&event.event_id), cooldown, sub_kind);
    } else {
        dispatcher.commit(event, cooldown, sub_kind);
    }
    Ok(())
}

/// v15.1 A3: push 失败后 rollback (no-op, reserve 不留痕, 保留 API 对称)
pub fn rollback_dedup_for_event(
    event: &SignalEvent,
    kind: PushKind,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> Result<(), String> {
    if crate::durable_delivery_runtime::is_counted_kind(kind) {
        return Err(format!(
            "BR-192 counted PushKind::{kind:?} cannot use legacy dedup rollback"
        ));
    }
    let stack = v14_stack()?;
    let source_fact = is_source_fact_signal(kind, event);
    let cooldown = dedup_cooldown(
        kind,
        source_fact || event.code.is_some(),
        source_fact,
        cooldown_override_secs,
    );
    lock_dispatcher(stack)?.rollback(event, cooldown, sub_kind);
    Ok(())
}

fn lock_store(
    stack: &'static V14Stack,
) -> Result<std::sync::MutexGuard<'static, SqliteStore>, String> {
    stack
        .store
        .lock()
        .map_err(|_| "L7 store Mutex poisoned; analytics rejected".to_string())
}

fn deny_for_unavailable_daily_count(
    stack: &'static V14Stack,
    event: &SignalEvent,
    kind_str: &str,
    ctx: &GovernanceContext,
    error: String,
) -> V14Gate {
    log::error!(
        "[v14.2][BR-005] daily push count unavailable for {}: {}",
        kind_str,
        error
    );
    let reason = "daily_limit_count_unavailable".to_string();
    let analytics = build_analytics(
        event,
        kind_str,
        1,
        ctx.data_mode,
        GovernanceDecision::Deny(reason.clone()),
        None,
        false,
        "none",
        "default",
        vec![error],
    );
    match lock_store(stack)
        .and_then(|store| store.record(&analytics).map_err(|error| error.to_string()))
    {
        Ok(()) => V14Gate::Denied(reason),
        Err(record_error) => {
            log::error!("[v14.2][BR-113] daily-count failure audit failed: {record_error}");
            V14Gate::Denied("analytics_audit_unavailable".to_string())
        }
    }
}

/// 投递后记录: 真实投递结果 → L7 push_analytics (b011 P0-1)
///
/// `sink_name` 必须是实际投递通道 ("feishu"/"wechat"/"dry_run"), 由 notify 层回传.
pub fn v14_record_delivery(
    event: &SignalEvent,
    kind: PushKind,
    text: &str,
    delivered: bool,
    sink_name: &str,
) -> Result<(), String> {
    let stack = v14_stack()?;
    let (_, kind_str, _) = map_push_kind(kind);
    let ctx = if is_source_fact_signal(kind, event) || kind == PushKind::CatalystReview {
        current_source_fact_governance_ctx()?
    } else {
        current_governance_ctx()?
    };
    let analytics = build_analytics(
        event,
        kind_str,
        1,
        ctx.data_mode,
        GovernanceDecision::Approve,
        Some(&RenderedText::new(text)),
        delivered,
        sink_name,
        "default",
        vec![],
    );
    lock_store(stack)?
        .record(&analytics)
        .map_err(|error| error.to_string())?;
    log::info!(
        "[v14.2] L7 record: PushKind={:?} pushed={} sink={} event={}",
        kind,
        delivered,
        sink_name,
        event.event_id
    );
    Ok(())
}

/// 测试辅助: 清空 L4 dedup 表 (V14_STACK 是 OnceLock 单例, 跨测试共享)
#[cfg(test)]
pub fn _reset_dedup_for_test() {
    let mut banner = crate::LATEST_BANNER
        .lock()
        .expect("test banner lock must be available");
    *banner = Some(crate::push_templates::BannerCtx::test_default());
    drop(banner);
    let stack = v14_stack().expect("test L7 store must initialize");
    match stack.dispatcher.write() {
        Ok(g) => g.clear_dedup(),
        Err(poisoned) => poisoned.into_inner().clear_dedup(),
    }
}

/// 测试辅助: 模拟 push 成功后调 dispatcher.commit() 插入 dedup entry.
/// v15.1 A3 后 reserve() 不占位, 必须 commit() 才会让后续 reserve 看到 Deduped.
/// v17.6 §5.1: sub_kind 参与 dedup key, 测试场景默认 None.
#[cfg(test)]
pub fn _commit_dedup_for_test(kind: PushKind, code: Option<&str>) {
    let stack = v14_stack().expect("test L7 store must initialize");
    let (source, kind_str, severity) = map_push_kind(kind);
    let event = SignalEvent::new(
        source,
        kind_str,
        code.map(str::to_string),
        Local::now(),
        SignalPayload::HoldingHealth(Default::default()),
        severity,
    );
    let cooldown = match kind.cooldown_scope() {
        CooldownScope::External => None,
        CooldownScope::PerTicket if code.is_none() => None,
        _ => kind
            .cooldown_secs()
            .map(|s| Duration::from_secs(u64::from(s))),
    };
    match stack.dispatcher.write() {
        Ok(g) => g.commit(&event, cooldown, None),
        Err(poisoned) => poisoned.into_inner().commit(&event, cooldown, None),
    }
}

/// PushKind → SignalSource/kind_str/severity 映射
fn map_push_kind(kind: PushKind) -> (SignalSource, &'static str, Severity) {
    use SignalSource::*;
    match kind {
        PushKind::HoldingEvent => (HoldingHealth, "holding_event", Severity::High),
        PushKind::DailyReport => (HoldingHealth, "daily_report", Severity::Normal),
        PushKind::Announcement => (NewsCatalyst, "announcement", Severity::High),
        PushKind::AuctionVolume => (SectorRotation, "auction_volume", Severity::Normal),
        PushKind::VirtualWatch => (HoldingHealth, "virtual_watch", Severity::Normal),
        PushKind::LimitBoards => (HoldingHealth, "limit_boards", Severity::High),
        PushKind::SectorTop => (SectorRotation, "sector_top", Severity::Normal),
        PushKind::FundInflow => (SectorRotation, "fund_inflow", Severity::Normal),
        PushKind::AuctionRepush => (SectorRotation, "auction_repush", Severity::Normal),
        PushKind::FactorIC => (HoldingHealth, "factor_ic", Severity::Normal),
        PushKind::SectorTier => (SectorRotation, "sector_tier", Severity::Normal),
        PushKind::CapitalVerify => (HoldingHealth, "capital_verify", Severity::High),
        PushKind::WeeklySOP => (HoldingHealth, "weekly_sop", Severity::Normal),
        PushKind::StockPick => (HoldingHealth, "stock_pick", Severity::Normal),
        PushKind::IndustryChain => (SectorRotation, "industry_chain", Severity::Normal),
        PushKind::TurnoverTop => (SectorRotation, "turnover_top", Severity::Normal),
        PushKind::CandidateBoard => (HoldingHealth, "candidate_board", Severity::Normal),
        PushKind::NewsRanked => (NewsCatalyst, "news_ranked", Severity::Normal),
        PushKind::AccountMode => (HoldingHealth, "account_mode", Severity::High),
        PushKind::DataMode => (DataSourceDown, "data_mode", Severity::High),
        PushKind::HoldingPlan => (HoldingHealth, "holding_plan", Severity::Normal),
        PushKind::T0Advice => (HoldingHealth, "t0_advice", Severity::Normal),
        PushKind::CandidateTriggered => (HoldingHealth, "candidate_triggered", Severity::Normal),
        PushKind::ForbiddenOps => (HoldingHealth, "forbidden_ops", Severity::Emergency),
        PushKind::PaperTrade => (HoldingHealth, "paper_trade", Severity::Normal),
        PushKind::CloseCall => (HoldingHealth, "close_call", Severity::High),
        PushKind::ReviewMarket => (HoldingHealth, "review_market", Severity::Normal),
        PushKind::ReviewLhb => (HoldingHealth, "review_lhb", Severity::Normal),
        PushKind::ReviewProviderTopN => (HoldingHealth, "review_provider_top_n", Severity::Normal),
        PushKind::PositionReview => (HoldingHealth, "position_review", Severity::Normal),
        PushKind::ReviewSignal => (HoldingHealth, "review_signal", Severity::Normal),
        PushKind::ReviewFailure => (HoldingHealth, "review_failure", Severity::High),
        PushKind::TomorrowWatch => (HoldingHealth, "tomorrow_watch", Severity::Normal),
        PushKind::EventCalendar => (HoldingHealth, "event_calendar", Severity::Normal),
        PushKind::PreopenNewsHot => (NewsCatalyst, "preopen_news_hot", Severity::High),
        PushKind::IntradayMarket => (SectorRotation, "intraday_market", Severity::Normal),
        PushKind::NewsCatalyst => (NewsCatalyst, "news_catalyst", Severity::High),
        PushKind::SectorAnomaly => (SectorRotation, "sector_anomaly", Severity::Normal),
        PushKind::NewsToIdea => (NewsCatalyst, "news_to_idea", Severity::Normal),
        PushKind::CatalystReview => (HoldingHealth, "catalyst_review", Severity::Normal),
        PushKind::IndustryChainIntraday => {
            (SectorRotation, "industry_chain_intraday", Severity::Normal)
        }
        PushKind::PostFixedPriceOrder => (HoldingHealth, "post_fixed_price_order", Severity::High),
        PushKind::PostFixedPriceFill => (HoldingHealth, "post_fixed_price_fill", Severity::High),
        PushKind::StPriceLimitChanged => (HoldingHealth, "st_price_limit_changed", Severity::High),
        PushKind::EtfClosingCallAuction => {
            (SectorRotation, "etf_closing_call_auction", Severity::Normal)
        }
        PushKind::BlockTradeIntradayConfirm => (
            HoldingHealth,
            "block_trade_intraday_confirm",
            Severity::High,
        ),
        PushKind::BlockTradePriceRange => {
            (HoldingHealth, "block_trade_price_range", Severity::Normal)
        }
        PushKind::PaperReview => (HoldingHealth, "paper_review", Severity::Normal),
        // v17.4 能力1 (BR-082)
        PushKind::NewsFlashCritical => (NewsCatalyst, "news_flash_critical", Severity::High),
        PushKind::NewsFlashAggregated => (NewsCatalyst, "news_flash_aggregated", Severity::Normal),
        PushKind::CandidateInvalidated => {
            (HoldingHealth, "candidate_invalidated", Severity::Normal)
        }
        // v15.1 C1.3: IPO PushKind 映射到 SignalSource::Ipo
        PushKind::IpoListingApproval | PushKind::IpoProspectus => {
            (SignalSource::Ipo, "ipo_official", Severity::High)
        }
        PushKind::IpoCatalyst => (SignalSource::Ipo, "ipo_catalyst", Severity::Normal),
        // v15.3 D5.2: 4 路源 SignalSource 映射
        PushKind::PolicyHit => (SignalSource::Policy, "policy_hit", Severity::High),
        PushKind::EarningsBeat => (SignalSource::Earnings, "earnings_beat", Severity::High),
        PushKind::EarningsMiss => (SignalSource::Earnings, "earnings_miss", Severity::High),
        PushKind::AnalystUpgrade => (
            SignalSource::AnalystView,
            "analyst_upgrade",
            Severity::Normal,
        ),
        PushKind::MarketActionAlert => (
            SignalSource::MarketAction,
            "market_action_alert",
            Severity::High,
        ),
    }
}

fn signal_payload_for_kind(kind: PushKind) -> SignalPayload {
    match kind {
        PushKind::DataMode => SignalPayload::DataSourceDown(Default::default()),
        _ => SignalPayload::HoldingHealth(Default::default()),
    }
}

fn signal_event_for_kind(kind: PushKind, code: Option<&str>) -> SignalEvent {
    let (source, kind_str, severity) = map_push_kind(kind);
    SignalEvent::new(
        source,
        kind_str,
        code.map(str::to_string),
        Local::now(),
        signal_payload_for_kind(kind),
        severity,
    )
}

fn signal_event_for_source_fact(evidence: &SourceFactEvidence) -> SignalEvent {
    let (source, kind_str, severity) = map_push_kind(evidence.kind);
    let mut event = SignalEvent::new(
        source,
        kind_str,
        evidence.security_code.clone(),
        evidence.observed_at,
        SignalPayload::NewsCatalyst(NewsCatalystPayload {
            code: evidence.security_code.clone(),
            headline: Some(evidence.headline.clone()),
            source: Some(evidence.source.clone()),
            published_on: Some(evidence.source_published_on),
        }),
        severity,
    );
    event.event_id =
        stock_analysis::push_l1::make_source_fact_event_id(kind_str, &evidence.governance_identity);
    event
}

fn source_fact_profile(kind: PushKind) -> TemplateMetadata {
    let mut profile = default_profile_for_kind(kind);
    profile.category = stock_analysis::push_l2::TemplateCategory::News;
    profile.data_mode_min = DataMode::Down;
    profile.always_send_on_data_source_down = false;
    profile
}

fn source_batch_profile(kind: PushKind) -> TemplateMetadata {
    let mut profile = default_profile_for_kind(kind);
    profile.data_mode_min = DataMode::Down;
    profile.always_send_on_data_source_down = false;
    profile
}

/// BR-197 R-04 is independent of account and intraday capability state. Its
/// exact provider/date/batch/canonical/hash/text binding is the component data
/// quality gate. The real process-local DataMode remains in analytics, but a
/// correctly stale after-close Quote must not veto this report.
fn counted_source_only_profile(kind: PushKind) -> TemplateMetadata {
    let mut profile = default_profile_for_kind(kind);
    profile.data_mode_min = DataMode::Down;
    profile.always_send_on_data_source_down = false;
    profile
}

fn is_source_fact_signal(kind: PushKind, event: &SignalEvent) -> bool {
    matches!(
        kind,
        PushKind::Announcement
            | PushKind::PolicyHit
            | PushKind::EarningsBeat
            | PushKind::EarningsMiss
            | PushKind::AnalystUpgrade
            | PushKind::NewsFlashCritical
    ) && matches!(event.payload, SignalPayload::NewsCatalyst(_))
}

/// 默认 profile (按 PushKind 给推荐 category + §14.3 冷却)
fn default_profile_for_kind(kind: PushKind) -> TemplateMetadata {
    use stock_analysis::push_l2::TemplateCategory::*;
    let category = match kind {
        PushKind::DataMode => DataSource,
        PushKind::ForbiddenOps
        | PushKind::AccountMode
        | PushKind::CapitalVerify
        | PushKind::StPriceLimitChanged => Risk,
        _ => Holding,
    };

    TemplateMetadata {
        category,
        // 🚨紧急类 (风控/持仓事件) 不受静默期抑制, 其余尊重 02:00-06:00 静默
        quiet_hours_respect: !kind.level().is_emergency(),
        // v17.1 治本: frozen_mode_respect 改为 false (L5 governance 不再 Deny Frozen)
        // Frozen 状态保留在 ctx.is_frozen, 模板自行渲染 ⚠️ 警告
        // 4 铁律: 通知层保持出声, 仓位风险控制在 broker 下单层
        frozen_mode_respect: false,
        // A-01/R-03/R-04/R-08/A-10 are independently sourced after-close
        // reports. Their gates are the admitted event/component batches
        // (BR-158/BR-159/BR-110/BR-161/BR-160), not intraday
        // quote/order-book capabilities represented by the global mode.
        // Missing inputs remain explicit at their own acquisition gates.
        data_mode_min: if matches!(
            kind,
            PushKind::AccountMode
                | PushKind::PaperReview
                | PushKind::IndustryChain
                | PushKind::ReviewLhb
                | PushKind::ReviewProviderTopN
                | PushKind::EventCalendar
                | PushKind::CatalystReview
                // BR-225: R-07/R-11 同为独立来源盘后报告 —
                // 龙虎榜/候选/持仓快照/收盘估值批次是自身数据门,
                // 盘后陈旧 Quote 不得否决 (BR-197 同款理由)
                | PushKind::TomorrowWatch
                | PushKind::PositionReview
        ) {
            DataMode::Down
        } else {
            DataMode::Degraded
        },
        // b011: 不再硬编码 60, 与 §14.3 治理表一致 (0 = 无冷却)
        cooldown_secs: kind.cooldown_secs().map(u64::from).unwrap_or(0),
        max_per_user_per_day: matches!(kind, PushKind::CandidateBoard).then_some(5),
        always_send_on_data_source_down: matches!(kind, PushKind::DataMode),
    }
}

/// 当前治理上下文
///
/// b013 review P0-7: 接 `crate::main::current_banner()` (由
/// `evaluate_data_mode_hook` + `evaluate_account_mode_hook` 周期刷, v41 LATEST_BANNER).
/// 真实 data_mode (Degraded/Full) + account_mode (Frozen/ReduceOnly/Normal) 真正进 L5.
/// `is_quiet_hour` 仍接本地时钟 (§3.5 02:00-06:00).
fn current_governance_ctx() -> Result<GovernanceContext, String> {
    let now = Local::now();
    let is_quiet_hour = current_quiet_hour(now);
    governance_ctx_for_banner_at(now, is_quiet_hour)
}

fn governance_ctx_for_banner_at(
    now: chrono::DateTime<Local>,
    is_quiet_hour: bool,
) -> Result<GovernanceContext, String> {
    // b013 P0-7: 直接读 LATEST_BANNER (main.rs 顶层 pub static, 由
    // evaluate_account_mode_hook + evaluate_data_mode_hook 周期刷)
    let banner = crate::LATEST_BANNER
        .lock()
        .map_err(|_| "governance banner lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "governance banner unavailable".to_string())?;
    Ok(GovernanceContext {
        data_mode: match banner.data_mode {
            crate::push_templates::DataMode::Full => DataMode::Full,
            crate::push_templates::DataMode::Degraded => DataMode::Degraded,
            _ => DataMode::Down,
        },
        is_quiet_hour,
        // b013 P0-7: Frozen 模式经 banner 进来, 治理能真正拦
        is_frozen: matches!(
            banner.account_mode,
            crate::push_templates::AccountMode::Frozen
        ),
        now,
        // BR-005: 有日上限的模板由 v14_gate 在 L5 检查前从 L7 注入真实成功数。
        today_pushed_count: 0,
    })
}

/// BR-137 source facts do not consume account state. Their governance context
/// therefore reads the real process-local capability tracker directly instead
/// of the combined account/data banner. `is_frozen=false` means this profile
/// does not apply the account freeze gate; it is not an inferred account mode.
fn current_source_fact_governance_ctx() -> Result<GovernanceContext, String> {
    use stock_analysis::monitor::data_mode::{
        current_data_health_input, evaluate, DataMode as HealthDataMode,
    };

    let now = Local::now();
    let health = evaluate(&current_data_health_input(120, 600)?, None);
    Ok(GovernanceContext {
        data_mode: match health.mode {
            HealthDataMode::Full => DataMode::Full,
            HealthDataMode::Degraded => DataMode::Degraded,
            HealthDataMode::Unsafe => DataMode::Down,
        },
        is_quiet_hour: current_quiet_hour(now),
        is_frozen: false,
        now,
        today_pushed_count: 0,
    })
}

/// BR-194/BR-197 R-04 provider-only governance context. Account freeze is not
/// a dependency of this report. Real process-local DataMode is retained for
/// analytics, while the canonical component binding owns data-quality
/// admission; quiet hours and the analytics-owned daily count remain active.
fn current_counted_source_only_governance_ctx() -> Result<GovernanceContext, String> {
    use stock_analysis::monitor::data_mode::{
        current_data_health_input, evaluate, DataMode as HealthDataMode,
    };

    let now = Local::now();
    let health = evaluate(&current_data_health_input(120, 600)?, None);
    Ok(GovernanceContext {
        data_mode: match health.mode {
            HealthDataMode::Full => DataMode::Full,
            HealthDataMode::Degraded => DataMode::Degraded,
            HealthDataMode::Unsafe => DataMode::Down,
        },
        is_quiet_hour: current_quiet_hour(now),
        // Account mode is not applicable to the canonical R-04 source-only
        // binding. This does not infer or persist a Normal account state.
        is_frozen: false,
        now,
        today_pushed_count: 0,
    })
}

fn current_quiet_hour(now: chrono::DateTime<Local>) -> bool {
    quiet_hour_active_at(now.time())
}

/// Shared BR-209/L5 quiet-hour policy. Review preflight and final delivery
/// governance must evaluate the same predicate so provider-free deferral cannot
/// drift from the sink-side race defence.
pub(crate) fn quiet_hour_active_at(now: chrono::NaiveTime) -> bool {
    match std::env::var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE")
        .ok()
        .as_deref()
    {
        // Explicit test/operations override. Without one, retain 02:00-06:00.
        Some("0") | Some("false") => false,
        Some("1") | Some("true") => true,
        _ => (2..6).contains(&now.hour()),
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct TestBannerGuard(Option<crate::push_templates::BannerCtx>);

    impl TestBannerGuard {
        fn full() -> Self {
            let previous = crate::LATEST_BANNER
                .lock()
                .expect("test banner lock")
                .replace(crate::push_templates::BannerCtx::test_default());
            Self(previous)
        }
    }

    impl Drop for TestBannerGuard {
        fn drop(&mut self) {
            *crate::LATEST_BANNER.lock().expect("restore banner lock") = self.0.take();
        }
    }

    fn source_fact(kind: PushKind) -> SourceFactEvidence {
        source_fact_with_identity(kind, "TEST_CODE_EVENT_ID")
    }

    fn source_fact_with_identity(kind: PushKind, identity: &str) -> SourceFactEvidence {
        let security_code = (!matches!(kind, PushKind::PolicyHit | PushKind::NewsFlashCritical))
            .then(|| "TEST_CODE_SOURCE_FACT".to_string());
        let now = Local::now();
        SourceFactEvidence::new(
            kind,
            identity.to_string(),
            security_code,
            "已验证来源事实".to_string(),
            "TEST_CODE_PROVIDER".to_string(),
            now,
            Some(now.date_naive()),
            80,
            90,
            false,
        )
        .expect("complete source fact")
    }

    fn isolated_stack() -> &'static V14Stack {
        Box::leak(Box::new(V14Stack {
            dispatcher: std::sync::RwLock::new(Dispatcher::new()),
            governance: GovernanceEngine::new(),
            store: Mutex::new(SqliteStore::open_in_memory().expect("test L7 store")),
        }))
    }

    #[test]
    fn br113_poisoned_dispatcher_lock_is_rejected() {
        let stack = isolated_stack();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = stack.dispatcher.write().expect("fresh dispatcher lock");
            panic!("TEST_CODE poison dispatcher");
        }));
        assert!(lock_dispatcher(stack).is_err());
    }

    #[test]
    fn br113_poisoned_store_lock_is_rejected() {
        let stack = isolated_stack();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = stack.store.lock().expect("fresh store lock");
            panic!("TEST_CODE poison store");
        }));
        assert!(lock_store(stack).is_err());
    }

    #[test]
    fn v14_stack_init() {
        // v14_stack 是 OnceLock 单例, 跨测试共享, 仅验证可访问.
        let _stack = v14_stack().unwrap();
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br196_scoped_smoke_clock_does_not_relax_ordinary_quiet_hour() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        std::env::set_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "1");
        let _banner_guard = TestBannerGuard::full();
        _reset_dedup_for_test();

        assert!(matches!(
            v14_gate(PushKind::PreopenNewsHot, None),
            V14Gate::Denied(reason) if reason == "quiet_hour"
        ));

        let context = crate::br196_test_delivery::GovernanceSmokeContext::for_review_date(
            Local::now().date_naive(),
        )
        .expect("construct scoped BR-196 governance context");
        let dispatch = context
            .dispatch("P-01-preopen-news-hot", PushKind::PreopenNewsHot, None)
            .expect("mint exact-six dispatch");
        assert!(matches!(
            v14_gate_br196_smoke(&dispatch),
            V14Gate::Approved(_)
        ));
        assert!(matches!(
            v14_gate(PushKind::PreopenNewsHot, None),
            V14Gate::Denied(reason) if reason == "quiet_hour"
        ));
    }

    #[test]
    fn br005_candidate_board_profile_has_daily_limit_five() {
        assert_eq!(
            default_profile_for_kind(PushKind::CandidateBoard).max_per_user_per_day,
            Some(5)
        );
        assert_eq!(
            default_profile_for_kind(PushKind::HoldingEvent).max_per_user_per_day,
            None
        );
    }

    #[test]
    fn br161_event_calendar_uses_its_component_evidence_not_intraday_global_mode() {
        assert_eq!(
            default_profile_for_kind(PushKind::EventCalendar).data_mode_min,
            DataMode::Down
        );
    }

    #[test]
    fn br158_paper_review_uses_its_admitted_daily_batch_not_intraday_global_mode() {
        assert_eq!(
            default_profile_for_kind(PushKind::PaperReview).data_mode_min,
            DataMode::Down
        );
    }

    #[test]
    fn br159_br160_chain_reports_use_admitted_batches_not_intraday_global_mode() {
        assert_eq!(
            default_profile_for_kind(PushKind::IndustryChain).data_mode_min,
            DataMode::Down
        );
        assert_eq!(
            default_profile_for_kind(PushKind::CatalystReview).data_mode_min,
            DataMode::Down
        );
    }

    #[test]
    fn br160_source_batch_binding_is_closed_and_content_bound() {
        let date = Local::now().date_naive();
        let observed_at = Local::now().fixed_offset();
        let first = SourceBatchEvidence::new(
            PushKind::CatalystReview,
            date,
            observed_at,
            "chain-batch:TEST_CODE_A10".to_owned(),
            "a".repeat(64),
        )
        .expect("valid A-10 source binding");
        let same = SourceBatchEvidence::new(
            PushKind::CatalystReview,
            date,
            observed_at,
            "chain-batch:TEST_CODE_A10".to_owned(),
            "a".repeat(64),
        )
        .expect("same A-10 source binding");
        assert_eq!(first.governance_identity(), same.governance_identity());

        assert!(matches!(
            SourceBatchEvidence::new(
                PushKind::IndustryChain,
                date,
                observed_at,
                "chain-batch:TEST_CODE_A10".to_owned(),
                "a".repeat(64),
            ),
            Err(SourceBatchError::KindNotAllowed(PushKind::IndustryChain))
        ));
        assert!(matches!(
            SourceBatchEvidence::new(
                PushKind::CatalystReview,
                date,
                observed_at,
                "TEST_CODE_not_a_chain_batch".to_owned(),
                "a".repeat(64),
            ),
            Err(SourceBatchError::InvalidBatchId)
        ));
        assert!(matches!(
            SourceBatchEvidence::new(
                PushKind::CatalystReview,
                date,
                observed_at,
                "chain-batch:TEST_CODE_A10".to_owned(),
                "not-a-hash".to_owned(),
            ),
            Err(SourceBatchError::InvalidContentHash)
        ));
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br160_source_batch_gate_and_l7_do_not_require_account_banner() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        _reset_dedup_for_test();
        let previous_banner = crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .take();
        let evidence = SourceBatchEvidence::new(
            PushKind::CatalystReview,
            Local::now().date_naive(),
            Local::now().fixed_offset(),
            "chain-batch:TEST_CODE_A10_NO_BANNER".to_owned(),
            "b".repeat(64),
        )
        .expect("valid A-10 source binding");

        let gate = v14_gate_source_batch(&evidence);
        let record_result = match &gate {
            V14Gate::Approved(event) => {
                match &event.payload {
                    SignalPayload::PostSessionReview(payload) => {
                        assert_eq!(payload.review_type.as_deref(), Some("catalyst_review"));
                        let summary = payload.summary.as_deref().expect("source batch summary");
                        assert!(summary.contains("chain-batch:TEST_CODE_A10_NO_BANNER"));
                        assert!(summary.contains(&"b".repeat(64)));
                    }
                    other => panic!("A-10 must retain source-batch provenance, got {other:?}"),
                }
                v14_record_delivery(
                    event,
                    PushKind::CatalystReview,
                    "TEST_CODE_A10",
                    true,
                    "dry_run",
                )
            }
            other => Err(format!(
                "source batch gate rejected without banner: {other:?}"
            )),
        };

        *crate::LATEST_BANNER.lock().expect("restore banner lock") = previous_banner;
        assert!(matches!(gate, V14Gate::Approved(_)), "gate={gate:?}");
        assert!(record_result.is_ok(), "record={record_result:?}");
    }

    #[test]
    fn br197_source_only_profile_uses_component_quality_without_changing_default_profile() {
        let profile = counted_source_only_profile(PushKind::ReviewLhb);
        assert_eq!(profile.data_mode_min, DataMode::Down);
        assert!(!profile.always_send_on_data_source_down);
        assert_eq!(
            default_profile_for_kind(PushKind::ReviewLhb).data_mode_min,
            DataMode::Down,
            "legacy/default ReviewLhb profile remains unchanged"
        );

        let event = signal_event_for_kind(PushKind::ReviewLhb, None);
        let down_context = GovernanceContext {
            data_mode: DataMode::Down,
            ..Default::default()
        };
        assert_eq!(
            GovernanceEngine::new().check(&profile, &event, &down_context),
            GovernanceDecision::Approve
        );

        let degraded_context = GovernanceContext {
            data_mode: DataMode::Degraded,
            ..Default::default()
        };
        assert_eq!(
            GovernanceEngine::new().check(&profile, &event, &degraded_context),
            GovernanceDecision::Approve
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_generic_counted_gate_fails_closed() {
        _reset_dedup_for_test();
        assert!(matches!(
            v14_gate(PushKind::HoldingEvent, Some("TEST_CODE_600519")),
            V14Gate::Denied(reason) if reason == "counted_binding_required"
        ));
        assert!(matches!(
            v14_gate(PushKind::FactorIC, None),
            V14Gate::Denied(reason) if reason == "counted_binding_required"
        ));
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_explicit_counted_gate_builds_stable_governance_event() {
        let _guard = crate::TestEnvGuard::dry_run_non_quiet();
        let _banner_guard = TestBannerGuard::full();
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
        let first = v14_gate_counted_binding(
            PushKind::HoldingPlan,
            Some("TEST_CODE_600519"),
            None,
            "TEST_CODE_OCCURRENCE_20260730",
            business_date,
        );
        let second = v14_gate_counted_binding(
            PushKind::HoldingPlan,
            Some("TEST_CODE_600519"),
            None,
            "TEST_CODE_OCCURRENCE_20260730",
            business_date,
        );
        let (V14Gate::Approved(first), V14Gate::Approved(second)) = (first, second) else {
            panic!("explicit counted bindings must reach L5 approval in non-quiet test mode");
        };
        assert_eq!(first.event_id, second.event_id);
        assert_eq!(first.ts, second.ts);
        assert_eq!(first.code.as_deref(), Some("TEST_CODE_600519"));
        assert_eq!(first.ts.date_naive(), business_date);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_counted_commit_cannot_reenter_legacy_dispatcher() {
        _reset_dedup_for_test();
        let event = SignalEvent::new(
            SignalSource::HoldingHealth,
            "daily_report",
            None,
            Local::now(),
            SignalPayload::HoldingHealth(Default::default()),
            Severity::Normal,
        );
        let override_secs = Some(1_800);
        let error = commit_dedup_for_event(
            &event,
            PushKind::DailyReport,
            Some("SectorTier"),
            override_secs,
        )
        .unwrap_err();
        assert!(error.contains("cannot use legacy dedup commit"));
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_explicit_counted_gate_rejects_missing_occurrence() {
        let _guard = crate::TestEnvGuard::dry_run_non_quiet();
        assert!(matches!(
            v14_gate_counted_binding(
                PushKind::HoldingPlan,
                Some("TEST_CODE_000001"),
                None,
                " ",
                NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            ),
            V14Gate::Denied(reason) if reason == "counted_schedule_occurrence_required"
        ));
    }

    #[test]
    fn map_push_kind_covers_all_variants() {
        let (src, kind, sev) = map_push_kind(PushKind::DailyReport);
        assert_eq!(src, SignalSource::HoldingHealth);
        assert_eq!(kind, "daily_report");
        assert_eq!(sev, Severity::Normal);

        let (src, kind, sev) = map_push_kind(PushKind::ForbiddenOps);
        assert_eq!(src, SignalSource::HoldingHealth);
        assert_eq!(kind, "forbidden_ops");
        assert_eq!(sev, Severity::Emergency);
    }

    #[test]
    fn data_mode_alert_is_a_data_source_down_event_with_down_exemption() {
        use stock_analysis::push_l1::{SignalPayload, SignalSource};
        use stock_analysis::push_l2::TemplateCategory;

        let event = signal_event_for_kind(PushKind::DataMode, None);
        assert_eq!(event.source, SignalSource::DataSourceDown);
        assert!(matches!(event.payload, SignalPayload::DataSourceDown(_)));
        let profile = default_profile_for_kind(PushKind::DataMode);
        assert_eq!(profile.category, TemplateCategory::DataSource);
        assert!(profile.always_send_on_data_source_down);
    }

    #[test]
    fn account_mode_alert_is_eligible_when_market_data_is_down() {
        assert_eq!(
            default_profile_for_kind(PushKind::AccountMode).data_mode_min,
            DataMode::Down
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br137_source_fact_is_approved_at_data_mode_down_with_news_payload() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        _reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;

        let gate = v14_gate_source_fact(&source_fact(PushKind::EarningsMiss));
        let V14Gate::Approved(event) = gate else {
            panic!("complete source fact must pass at Down: {gate:?}");
        };
        assert_eq!(event.code.as_deref(), Some("TEST_CODE_SOURCE_FACT"));
        assert_ne!(event.event_id, "TEST_CODE_EVENT_ID");
        match &event.payload {
            SignalPayload::NewsCatalyst(payload) => {
                assert_eq!(payload.code.as_deref(), Some("TEST_CODE_SOURCE_FACT"));
                assert_eq!(payload.headline.as_deref(), Some("已验证来源事实"));
                assert_eq!(payload.source.as_deref(), Some("TEST_CODE_PROVIDER"));
                assert_eq!(payload.published_on, Some(Local::now().date_naive()));
            }
            other => panic!("source fact must retain news provenance, got {other:?}"),
        }

        use stock_analysis::event::DomainEvent;
        let delivery = stock_analysis::event::PushDeliveryEvent::new(
            "earnings_miss".to_string(),
            event.code.clone(),
            "Pushed".to_string(),
            "dry_run".to_string(),
            1,
            1,
        );
        assert_eq!(
            delivery.entity_key(),
            None,
            "delivery audit identity must be redacted"
        );
        let payload = delivery.payload();
        assert!(payload.get("code").is_none());
        assert_eq!(payload["identity_hash"].as_str().unwrap().len(), 64);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br137_l4_dedup_uses_provider_identity_not_security_code() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        _reset_dedup_for_test();
        let first_fact = source_fact_with_identity(PushKind::EarningsMiss, "provider-event-1");
        let second_fact = source_fact_with_identity(PushKind::EarningsMiss, "provider-event-2");

        let V14Gate::Approved(first_event) = v14_gate_source_fact(&first_fact) else {
            panic!("first source fact must be approved");
        };
        commit_dedup_for_event(&first_event, PushKind::EarningsMiss, None, None).unwrap();
        assert!(matches!(
            v14_gate_source_fact(&second_fact),
            V14Gate::Approved(_)
        ));
        assert!(matches!(
            v14_gate_source_fact(&first_fact),
            V14Gate::Deduped
        ));
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br137_source_fact_gate_and_l7_do_not_require_account_banner() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        _reset_dedup_for_test();
        let previous_banner = crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .take();

        let gate = v14_gate_source_fact(&source_fact(PushKind::PolicyHit));
        let record_result = match &gate {
            V14Gate::Approved(event) => v14_record_delivery(
                event,
                PushKind::PolicyHit,
                "TEST_CODE_FACT",
                true,
                "dry_run",
            ),
            other => Err(format!(
                "source fact gate rejected without banner: {other:?}"
            )),
        };

        *crate::LATEST_BANNER.lock().expect("restore banner lock") = previous_banner;
        assert!(matches!(gate, V14Gate::Approved(_)), "gate={gate:?}");
        assert!(record_result.is_ok(), "record={record_result:?}");
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br137_generic_mixed_news_remains_denied_at_data_mode_down() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        _reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;

        assert!(matches!(
            v14_gate(PushKind::NewsCatalyst, Some("TEST_CODE_MIXED_NEWS")),
            V14Gate::Denied(reason) if reason == "data_quality"
        ));
    }

    #[test]
    fn br137_source_fact_rejects_non_whitelisted_or_invalid_evidence() {
        let now = Local::now();
        let valid = |kind,
                     identity: &str,
                     code: Option<&str>,
                     headline: &str,
                     source: &str,
                     observed_at: chrono::DateTime<Local>,
                     strength,
                     certainty,
                     stale| {
            let source_published_on = observed_at.date_naive();
            SourceFactEvidence::new(
                kind,
                identity.to_string(),
                code.map(str::to_string),
                headline.to_string(),
                source.to_string(),
                observed_at,
                Some(source_published_on),
                strength,
                certainty,
                stale,
            )
        };

        assert!(valid(
            PushKind::MarketActionAlert,
            "event",
            Some("TEST_CODE_MARKET_ACTION"),
            "headline",
            "provider",
            now,
            80,
            90,
            false
        )
        .is_err());
        assert_eq!(
            SourceFactEvidence::new(
                PushKind::PolicyHit,
                "event".to_string(),
                None,
                "headline".to_string(),
                "provider".to_string(),
                now,
                None,
                80,
                90,
                false,
            )
            .unwrap_err(),
            SourceFactError::MissingPublishedDate
        );
        assert!(valid(
            PushKind::Announcement,
            "event",
            None,
            "headline",
            "provider",
            now,
            80,
            90,
            false
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            " ",
            None,
            "headline",
            "provider",
            now,
            80,
            90,
            false
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            "event",
            None,
            " ",
            "provider",
            now,
            80,
            90,
            false
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            "event",
            None,
            "headline",
            " ",
            now,
            80,
            90,
            false
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            "event",
            None,
            "headline",
            "provider",
            now,
            101,
            90,
            false
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            "event",
            None,
            "headline",
            "provider",
            now,
            80,
            101,
            false
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            "event",
            None,
            "headline",
            "provider",
            now,
            80,
            90,
            true
        )
        .is_err());
        assert!(valid(
            PushKind::PolicyHit,
            "event",
            None,
            "headline",
            "provider",
            now + chrono::Duration::minutes(1),
            80,
            90,
            false
        )
        .is_err());
    }

    #[test]
    fn br137_critical_flash_identity_is_not_a_security_code() {
        let fact = source_fact(PushKind::NewsFlashCritical);
        assert_eq!(fact.governance_identity, "TEST_CODE_EVENT_ID");
        assert_eq!(fact.security_code.as_deref(), None);
    }

    #[test]
    fn profile_cooldown_follows_push_kind_table() {
        assert_eq!(
            default_profile_for_kind(PushKind::DailyReport).cooldown_secs,
            86_400
        );
        assert_eq!(
            default_profile_for_kind(PushKind::HoldingEvent).cooldown_secs,
            0
        );
    }
}
