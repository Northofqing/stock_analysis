//! BR-148: capability diagnostics are orthogonal to governance DataMode.
//! v12 PR2-2.1: 数据模式三态判定 (DataHealth ∈ {Full, Degraded, Unsafe}).
//!
//! 设计: 与 `risk::account_mode` 对齐 — 纯函数 + 数据入参, 不直接读行情 DB.
//!       Capability 各自维护真实成功时间；从未成功的能力保持 Missing.
//!
//! 状态机:
//!   Full --(任一关键 Capability staleness > 120s)--> Degraded
//!   Full/Degraded --(Quote staleness > 120s)--> Unsafe
//!   Degraded --(全部 Capability 恢复)--> Full
//!
//! BR-216: MoneyFlow 仍未接入逐证券真实 provider，保持辅助能力；BR-253 的严格 T0
//! 全批准入已把 OrderBook 接入真实 provider。因此 OrderBook Missing 或超过其 600s
//! 预算会使 DataMode 降为 Degraded；只有 Quote 断流会使其进入 Unsafe。

#[cfg(test)]
use chrono::Utc;
use chrono::{DateTime, FixedOffset};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

/// v12 §2.4 数据能力枚举
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// 实时行情
    Quote,
    /// K线 (日/分钟)
    Kline,
    /// 资金流 (主力/北向)
    MoneyFlow,
    /// 新闻快讯
    News,
    /// 盘口深度 (十档买卖)
    OrderBook,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CapabilityState {
    Warming,
    Healthy,
    Stale,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityObservation {
    pub capability: Capability,
    pub state: CapabilityState,
    pub expected_now: bool,
    pub provider: Option<String>,
    pub provider_observed_at: Option<DateTime<FixedOffset>>,
    pub locally_observed_at: Option<DateTime<FixedOffset>>,
    pub last_success_at: Option<DateTime<FixedOffset>>,
    pub age_secs: Option<u64>,
    pub last_error_code: Option<String>,
    pub retryable: Option<bool>,
    pub next_retry_at: Option<DateTime<FixedOffset>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityNow {
    pub wall: DateTime<FixedOffset>,
    pub monotonic: Instant,
    pub expected_now: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilitySuccess {
    pub capability: Capability,
    pub provider: String,
    pub provider_observed_at: Option<DateTime<FixedOffset>>,
    pub locally_observed_at: DateTime<FixedOffset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityFailure {
    pub capability: Capability,
    pub provider: String,
    pub locally_observed_at: DateTime<FixedOffset>,
    pub reason_code: String,
    pub retryable: bool,
    pub next_retry_at: Option<DateTime<FixedOffset>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityDiagnosticSnapshot {
    pub observations: Vec<CapabilityObservation>,
    pub fingerprint: String,
    pub first_probe_complete: bool,
}

#[derive(Clone, Debug)]
struct CapabilityRecord {
    supported: bool,
    attempted: bool,
    provider: Option<String>,
    provider_observed_at: Option<DateTime<FixedOffset>>,
    locally_observed_at: Option<DateTime<FixedOffset>>,
    last_success_at: Option<DateTime<FixedOffset>>,
    last_success_mono: Option<Instant>,
    last_error_code: Option<String>,
    retryable: Option<bool>,
    next_retry_at: Option<DateTime<FixedOffset>>,
}

pub struct CapabilityTracker {
    records: RwLock<HashMap<Capability, CapabilityRecord>>,
    stale_after: Duration,
}

impl CapabilityTracker {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
            stale_after: Duration::from_secs(120),
        }
    }
    pub fn register_supported(&self, capability: Capability) -> Result<(), String> {
        self.register(capability, true)
    }
    pub fn register_unsupported(&self, capability: Capability) -> Result<(), String> {
        self.register(capability, false)
    }
    fn register(&self, capability: Capability, supported: bool) -> Result<(), String> {
        let mut g = self
            .records
            .write()
            .map_err(|_| "capability tracker write lock poisoned".to_string())?;
        g.entry(capability).or_insert(CapabilityRecord {
            supported,
            attempted: false,
            provider: None,
            provider_observed_at: None,
            locally_observed_at: None,
            last_success_at: None,
            last_success_mono: None,
            last_error_code: None,
            retryable: None,
            next_retry_at: None,
        });
        Ok(())
    }
    pub fn record_attempt_started(
        &self,
        capability: Capability,
        provider: &str,
        at: DateTime<FixedOffset>,
    ) -> Result<(), String> {
        if provider.trim().is_empty() {
            return Err("provider must not be blank".into());
        }
        let mut g = self
            .records
            .write()
            .map_err(|_| "capability tracker write lock poisoned".to_string())?;
        let r = g.entry(capability).or_insert_with(|| CapabilityRecord {
            supported: true,
            attempted: false,
            provider: None,
            provider_observed_at: None,
            locally_observed_at: None,
            last_success_at: None,
            last_success_mono: None,
            last_error_code: None,
            retryable: None,
            next_retry_at: None,
        });
        if !r.supported {
            return Err("unsupported capability cannot be attempted".into());
        }
        r.attempted = true;
        r.provider = Some(provider.to_string());
        r.locally_observed_at = Some(at);
        Ok(())
    }
    pub fn record_success(&self, s: CapabilitySuccess, now: Instant) -> Result<(), String> {
        let mut g = self
            .records
            .write()
            .map_err(|_| "capability tracker write lock poisoned".to_string())?;
        let r = g
            .get_mut(&s.capability)
            .ok_or("capability not registered")?;
        if !r.supported || !r.attempted {
            return Err("success requires a supported started attempt".into());
        }
        if s.provider.trim().is_empty() {
            return Err("provider must not be blank".into());
        }
        r.provider = Some(s.provider);
        r.provider_observed_at = s.provider_observed_at;
        r.locally_observed_at = Some(s.locally_observed_at);
        r.last_success_at = Some(s.locally_observed_at);
        r.last_success_mono = Some(now);
        r.last_error_code = None;
        r.retryable = None;
        r.next_retry_at = None;
        Ok(())
    }
    pub fn record_failure(&self, f: CapabilityFailure) -> Result<(), String> {
        if f.provider.trim().is_empty() || f.reason_code.trim().is_empty() {
            return Err("provider and reason_code must not be blank".into());
        }
        let mut g = self
            .records
            .write()
            .map_err(|_| "capability tracker write lock poisoned".to_string())?;
        let r = g
            .get_mut(&f.capability)
            .ok_or("capability not registered")?;
        if !r.supported || !r.attempted {
            return Err("failure requires a supported started attempt".into());
        }
        r.provider = Some(f.provider);
        r.locally_observed_at = Some(f.locally_observed_at);
        r.last_error_code = Some(f.reason_code);
        r.retryable = Some(f.retryable);
        r.next_retry_at = f.next_retry_at;
        Ok(())
    }
    pub fn snapshot_at(&self, now: CapabilityNow) -> Result<CapabilityDiagnosticSnapshot, String> {
        let g = self
            .records
            .read()
            .map_err(|_| "capability tracker read lock poisoned".to_string())?;
        let mut observations = Vec::new();
        for &cap in &Capability::ALL {
            let Some(r) = g.get(&cap) else { continue };
            let (state, age) = if !r.supported {
                (CapabilityState::Unsupported, None)
            } else if !r.attempted {
                (CapabilityState::Warming, None)
            } else if r.last_error_code.is_some() {
                (
                    CapabilityState::Failed,
                    r.last_success_mono
                        .map(|t| now.monotonic.saturating_duration_since(t).as_secs()),
                )
            } else {
                let a = r
                    .last_success_mono
                    .map(|t| now.monotonic.saturating_duration_since(t).as_secs());
                (
                    if a.is_none_or(|x| now.expected_now && x > self.stale_after.as_secs()) {
                        CapabilityState::Stale
                    } else {
                        CapabilityState::Healthy
                    },
                    a,
                )
            };
            observations.push(CapabilityObservation {
                capability: cap,
                state,
                expected_now: now.expected_now,
                provider: r.provider.clone(),
                provider_observed_at: r.provider_observed_at,
                locally_observed_at: r.locally_observed_at,
                last_success_at: r.last_success_at,
                age_secs: age,
                last_error_code: r.last_error_code.clone(),
                retryable: r.retryable,
                next_retry_at: r.next_retry_at,
            });
        }
        let first_probe_complete = observations
            .iter()
            .filter(|o| o.state != CapabilityState::Unsupported)
            .all(|o| o.state != CapabilityState::Warming);
        let mut h = DefaultHasher::new();
        for o in &observations {
            (
                o.capability,
                o.state,
                o.provider.as_deref(),
                o.last_error_code.as_deref(),
                o.retryable,
                o.expected_now,
            )
                .hash(&mut h);
        }
        let fingerprint = format!("cap-v1-{:016x}", h.finish());
        Ok(CapabilityDiagnosticSnapshot {
            observations,
            fingerprint,
            first_probe_complete,
        })
    }
}

impl Default for CapabilityTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Capability {
    pub const ALL: [Capability; 5] = [
        Capability::Quote,
        Capability::Kline,
        Capability::MoneyFlow,
        Capability::News,
        Capability::OrderBook,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Capability::Quote => "Quote",
            Capability::Kline => "Kline",
            Capability::MoneyFlow => "MoneyFlow",
            Capability::News => "News",
            Capability::OrderBook => "OrderBook",
        }
    }

    /// 该 capability 缺失是否影响"价格型建议" (PR2-2.1 关键)
    ///
    /// true = 关键能力, 缺失降级 DataMode
    /// false = 辅助能力, 缺失只挂横幅, 不降级
    ///
    /// BR-216: 判定依据是"是否已接入真实 provider", 不是"是否重要"。
    /// MoneyFlow 仍无逐证券真实 provider（BR-190
    /// `provider_capability_not_live_admitted`），若将其算作关键能力会让
    /// `critical_stale` 恒非空、Full 分支永不可达。OrderBook 已由 BR-253 的严格 T0
    /// 全批准入标记成功，必须作为关键能力参与 DataMode。
    pub fn is_critical(self) -> bool {
        !matches!(self, Capability::MoneyFlow)
    }
}

/// BR-216: Kline 属 AGENTS §2.4 的"日线/历史"档, 新鲜度按 1 个交易日计。
/// 沿用 120s 关键窗口会让日频数据结构上不可能新鲜。
pub const KLINE_MAX_AGE_SECS: u64 = 24 * 60 * 60;

/// BR-216: News 采集节律由 `NEWS_POLL_INTERVAL` 决定 (默认 120s)。
/// 预算必须显著大于轮询周期, 否则每轮之间必然抖出 stale。
pub const NEWS_MAX_AGE_SECS: u64 = 300;

/// 三态数据模式
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DataMode {
    Full,
    Degraded,
    Unsafe,
}

impl DataMode {
    pub fn label(self) -> &'static str {
        match self {
            DataMode::Full => "Full",
            DataMode::Degraded => "Degraded",
            DataMode::Unsafe => "Unsafe",
        }
    }
}

/// BR-135: an unchanged, confirmed Unsafe event is audited every 30 minutes.
pub const PERSISTENT_UNSAFE_REMINDER_INTERVAL: Duration = Duration::from_secs(30 * 60);

pub const UNSAFE_EVENT_FINGERPRINT_VERSION: &str = "data_mode_unsafe_v1";
pub const UNSAFE_HEARTBEAT_AUDIT_KIND: &str = "data_mode_unsafe_heartbeat_v1";
pub const UNSAFE_HEARTBEAT_AUDIT_OUTCOME: &str = "Deduped";
pub const UNSAFE_HEARTBEAT_AUDIT_CHANNEL: &str = "internal_audit";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersistentUnsafeAction {
    ExternalStatus { fingerprint: String },
    InternalHeartbeat { fingerprint: String },
    Silent,
}

#[derive(Clone, Debug)]
struct ConfirmedUnsafeEvent {
    fingerprint: String,
    confirmed_at: Instant,
}

/// Returns a stable identity for the source-backed Unsafe facts.
///
/// Dynamic age, wall time and ETA are intentionally excluded. The fixed
/// `Capability::ALL` order also makes caller ordering and duplicates inert.
pub fn unsafe_event_fingerprint(missing: &[Capability]) -> String {
    let ordered_missing = Capability::ALL
        .iter()
        .filter(|capability| missing.contains(capability))
        .map(|capability| capability.label())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{UNSAFE_EVENT_FINGERPRINT_VERSION}|{}|{ordered_missing}",
        DataMode::Unsafe.label()
    )
}

/// Tracks only authoritative external delivery or immutable heartbeat confirmation.
///
/// The caller supplies the monotonic clock and records a result only after its
/// matching side effect and every mandatory audit have succeeded.
#[derive(Clone, Debug, Default)]
pub struct PersistentUnsafeReminder {
    confirmed: Option<ConfirmedUnsafeEvent>,
}

impl PersistentUnsafeReminder {
    pub fn action(
        &self,
        mode: DataMode,
        missing: &[Capability],
        now: Instant,
    ) -> Result<PersistentUnsafeAction, String> {
        if mode != DataMode::Unsafe {
            return Ok(PersistentUnsafeAction::Silent);
        }
        let fingerprint = unsafe_event_fingerprint(missing);
        let Some(confirmed) = &self.confirmed else {
            return Ok(PersistentUnsafeAction::ExternalStatus { fingerprint });
        };
        if confirmed.fingerprint != fingerprint {
            return Ok(PersistentUnsafeAction::ExternalStatus { fingerprint });
        }
        let elapsed = now
            .checked_duration_since(confirmed.confirmed_at)
            .ok_or_else(|| {
                "BR-135 monotonic heartbeat clock moved backwards; state unchanged".to_string()
            })?;
        if elapsed >= PERSISTENT_UNSAFE_REMINDER_INTERVAL {
            Ok(PersistentUnsafeAction::InternalHeartbeat { fingerprint })
        } else {
            Ok(PersistentUnsafeAction::Silent)
        }
    }

    pub fn record_external_confirmed(
        &mut self,
        mode: DataMode,
        missing: &[Capability],
        now: Instant,
    ) {
        if mode == DataMode::Unsafe {
            self.confirmed = Some(ConfirmedUnsafeEvent {
                fingerprint: unsafe_event_fingerprint(missing),
                confirmed_at: now,
            });
        } else {
            self.confirmed = None;
        }
    }

    pub fn record_heartbeat_confirmed(
        &mut self,
        fingerprint: &str,
        now: Instant,
    ) -> Result<(), String> {
        let confirmed = self
            .confirmed
            .as_mut()
            .ok_or_else(|| "BR-135 heartbeat has no confirmed Unsafe event".to_string())?;
        if confirmed.fingerprint != fingerprint {
            return Err("BR-135 heartbeat fingerprint does not match confirmed event".to_string());
        }
        now.checked_duration_since(confirmed.confirmed_at)
            .ok_or_else(|| {
                "BR-135 monotonic heartbeat clock moved backwards; state unchanged".to_string()
            })?;
        confirmed.confirmed_at = now;
        Ok(())
    }
}

/// 单个 capability 的新鲜度快照 (由主循环填入)
#[derive(Copy, Clone, Debug)]
pub struct CapabilityStatus {
    pub cap: Capability,
    /// 自上次成功刷新起的秒数. None 表示从未刷新过 (Missing).
    pub staleness_secs: Option<u64>,
}

impl CapabilityStatus {
    pub fn missing(cap: Capability) -> Self {
        Self {
            cap,
            staleness_secs: None,
        }
    }

    pub fn fresh(cap: Capability, secs: u64) -> Self {
        Self {
            cap,
            staleness_secs: Some(secs),
        }
    }

    /// 是否可用: 有数据且 staleness ≤ max_age_secs
    pub fn is_ok(&self, max_age_secs: u64) -> bool {
        match self.staleness_secs {
            Some(s) => s <= max_age_secs,
            None => false,
        }
    }
}

/// 入参: 5 个 capability 的状态 + 配置
#[derive(Clone, Debug)]
pub struct DataHealthInput {
    pub capabilities: Vec<CapabilityStatus>,
    /// 关键 capability 的 staleness 阈值 (默认 120s, 复用 data_quality.rs:296-309)
    pub critical_max_age_secs: u64,
    /// OrderBook 专用阈值 (默认 600s, 因为盘口刷新频率低)
    pub orderbook_max_age_secs: u64,
}

impl DataHealthInput {
    /// BR-216: 按采集节律给出该 capability 的新鲜度预算。
    ///
    /// 分级后仍保留调用方语义: 放大 `critical_max_age_secs` 可整体放宽,
    /// 但不得把日频/新闻档收窄到低于其自身节律。
    pub fn max_age_for(&self, cap: Capability) -> u64 {
        match cap {
            Capability::Quote => self.critical_max_age_secs,
            Capability::Kline => KLINE_MAX_AGE_SECS.max(self.critical_max_age_secs),
            Capability::News => NEWS_MAX_AGE_SECS.max(self.critical_max_age_secs),
            Capability::MoneyFlow | Capability::OrderBook => self.orderbook_max_age_secs,
        }
    }
}

impl Default for DataHealthInput {
    fn default() -> Self {
        Self {
            capabilities: Capability::ALL
                .iter()
                .map(|c| CapabilityStatus::missing(*c))
                .collect(),
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        }
    }
}

/// 评估结果
#[derive(Clone, Debug)]
pub struct DataHealth {
    pub mode: DataMode,
    pub missing: Vec<Capability>,
    /// prev 模式 (None 表示首次评估)
    pub prev_mode: Option<DataMode>,
    /// ETA 恢复预计 (供 T-02 推送文案), 简单写 "N/A" or "{capability} 刷新后"
    pub eta: Option<String>,
}

impl DataHealth {
    pub fn is_changed(&self) -> bool {
        match self.prev_mode {
            Some(p) => p != self.mode,
            None => false, // 首次评估不算变更, 不触发 T-02
        }
    }
}

/// PR2-2.1 主评估函数
///
/// 规则:
///   1. 任一**关键** capability 缺失或超过其注册的新鲜度预算 → Degraded
///   2. Quote staleness > critical_max_age_secs (即行情断流) → Unsafe
///   3. OrderBook 使用既有 600s 预算；缺失或超期进入 Degraded，绝不升级为 Unsafe
///   4. 全 Full 且全 fresh → Full
///
/// `prev` 由调用方从 history 表恢复, 首次评估传 None.
pub fn evaluate(input: &DataHealthInput, prev: Option<DataMode>) -> DataHealth {
    let mut missing: Vec<Capability> = Vec::new();
    let mut critical_stale: Vec<Capability> = Vec::new();
    let mut quote_stale = false;

    for cs in &input.capabilities {
        let max_age = input.max_age_for(cs.cap);

        if cs.is_ok(max_age) {
            continue;
        }

        missing.push(cs.cap);

        if cs.cap.is_critical() {
            critical_stale.push(cs.cap);
            if matches!(cs.cap, Capability::Quote) {
                quote_stale = true;
            }
        }
    }

    // 1. Quote 断流 → Unsafe
    if quote_stale {
        let only_quote = missing.len() <= 1;
        return DataHealth {
            mode: DataMode::Unsafe,
            missing,
            prev_mode: prev,
            eta: Some(if only_quote {
                "Quote 恢复并通过有效性校验后".to_string()
            } else {
                "关键能力恢复并通过 freshness 校验后".to_string()
            }),
        };
    }

    // 2. 关键能力降级 → Degraded
    if !critical_stale.is_empty() {
        let caps: Vec<String> = critical_stale
            .iter()
            .map(|c| c.label().to_string())
            .collect();
        let eta = format!("{} 刷新后", caps.join("/"));
        return DataHealth {
            mode: DataMode::Degraded,
            missing,
            prev_mode: prev,
            eta: Some(eta),
        };
    }

    // 3. 仅辅助能力（当前仅 MoneyFlow）缺失 → Full, 横幅提示
    DataHealth {
        mode: DataMode::Full,
        missing,
        prev_mode: prev,
        eta: None,
    }
}

/// 便利: 构造 DataHealthInput from `(cap, last_update_secs_ago)` pairs
pub fn input_from_pairs(
    critical_max_age_secs: u64,
    pairs: &[(Capability, Option<u64>)],
) -> DataHealthInput {
    DataHealthInput {
        capabilities: pairs
            .iter()
            .map(|(cap, s)| match s {
                Some(secs) => CapabilityStatus::fresh(*cap, *secs),
                None => CapabilityStatus::missing(*cap),
            })
            .collect(),
        critical_max_age_secs,
        orderbook_max_age_secs: 600,
    }
}

static LAST_CAPABILITY_SUCCESS: OnceLock<RwLock<HashMap<Capability, Instant>>> = OnceLock::new();

fn capability_successes() -> &'static RwLock<HashMap<Capability, Instant>> {
    LAST_CAPABILITY_SUCCESS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Record a capability only after its production source and quality checks succeed.
pub fn mark_capability_success(capability: Capability) -> Result<(), String> {
    capability_successes()
        .write()
        .map_err(|_| "capability success tracker write lock poisoned".to_string())?
        .insert(capability, Instant::now());
    Ok(())
}

fn input_from_successes_at(
    successes: &HashMap<Capability, Instant>,
    now: Instant,
    critical_max_age_secs: u64,
    orderbook_max_age_secs: u64,
) -> DataHealthInput {
    DataHealthInput {
        capabilities: Capability::ALL
            .iter()
            .map(|capability| CapabilityStatus {
                cap: *capability,
                staleness_secs: successes
                    .get(capability)
                    .map(|last_success| now.saturating_duration_since(*last_success).as_secs()),
            })
            .collect(),
        critical_max_age_secs,
        orderbook_max_age_secs,
    }
}

/// Build a health snapshot from actual process-local source successes.
///
/// A capability absent from the tracker has never succeeded in this process and
/// is therefore reported as Missing. BR-253 marks OrderBook only after a strict,
/// exact, fully successful T0 batch from its real provider.
pub fn current_data_health_input(
    critical_max_age_secs: u64,
    orderbook_max_age_secs: u64,
) -> Result<DataHealthInput, String> {
    let successes = capability_successes()
        .read()
        .map_err(|_| "capability success tracker read lock poisoned".to_string())?;
    Ok(input_from_successes_at(
        &successes,
        Instant::now(),
        critical_max_age_secs,
        orderbook_max_age_secs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn br148_new_capabilities_are_warming_or_unsupported() {
        let t = CapabilityTracker::new();
        t.register_supported(Capability::Quote).unwrap();
        t.register_unsupported(Capability::MoneyFlow).unwrap();
        let wall = FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 22, 9, 0, 0)
            .unwrap();
        let s = t
            .snapshot_at(CapabilityNow {
                wall,
                monotonic: Instant::now(),
                expected_now: true,
            })
            .unwrap();
        assert_eq!(
            s.observations
                .iter()
                .find(|o| o.capability == Capability::Quote)
                .unwrap()
                .state,
            CapabilityState::Warming
        );
        let money_flow = s
            .observations
            .iter()
            .find(|o| o.capability == Capability::MoneyFlow)
            .unwrap();
        assert_eq!(money_flow.state, CapabilityState::Unsupported);
        assert!(money_flow.next_retry_at.is_none());
    }

    #[test]
    fn br148_success_and_failure_preserve_evidence_and_fingerprint_ignores_age() {
        let t = CapabilityTracker::new();
        t.register_supported(Capability::Quote).unwrap();
        let wall = Utc::now().fixed_offset();
        t.record_attempt_started(Capability::Quote, "TEST_CODE_provider", wall)
            .unwrap();
        let start = Instant::now();
        t.record_success(
            CapabilitySuccess {
                capability: Capability::Quote,
                provider: "TEST_CODE_provider".into(),
                provider_observed_at: Some(wall),
                locally_observed_at: wall,
            },
            start,
        )
        .unwrap();
        let a = t
            .snapshot_at(CapabilityNow {
                wall,
                monotonic: start,
                expected_now: true,
            })
            .unwrap();
        assert_eq!(a.observations[0].state, CapabilityState::Healthy);
        let b = t
            .snapshot_at(CapabilityNow {
                wall,
                monotonic: start + Duration::from_secs(1),
                expected_now: true,
            })
            .unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        t.record_attempt_started(Capability::Quote, "TEST_CODE_provider", wall)
            .unwrap();
        t.record_failure(CapabilityFailure {
            capability: Capability::Quote,
            provider: "TEST_CODE_provider".into(),
            locally_observed_at: wall,
            reason_code: "timeout".into(),
            retryable: true,
            next_retry_at: None,
        })
        .unwrap();
        let failed = t
            .snapshot_at(CapabilityNow {
                wall,
                monotonic: start + Duration::from_secs(2),
                expected_now: true,
            })
            .unwrap();
        assert_eq!(failed.observations[0].state, CapabilityState::Failed);
        assert!(failed.observations[0].last_success_at.is_some());
    }

    #[test]
    fn tracker_fail_closed_transition_matrix() {
        let tracker = CapabilityTracker::default();
        let wall = FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 8, 28, 9, 30, 0)
            .unwrap();

        assert_eq!(
            tracker.record_attempt_started(Capability::Quote, " ", wall),
            Err("provider must not be blank".to_owned())
        );
        tracker.register_unsupported(Capability::MoneyFlow).unwrap();
        assert_eq!(
            tracker.record_attempt_started(Capability::MoneyFlow, "TEST_CODE_provider", wall),
            Err("unsupported capability cannot be attempted".to_owned())
        );
        assert_eq!(
            tracker.record_success(
                CapabilitySuccess {
                    capability: Capability::News,
                    provider: "TEST_CODE_provider".to_owned(),
                    provider_observed_at: Some(wall),
                    locally_observed_at: wall,
                },
                Instant::now(),
            ),
            Err("capability not registered".to_owned())
        );

        tracker.register_supported(Capability::Kline).unwrap();
        assert_eq!(
            tracker.record_success(
                CapabilitySuccess {
                    capability: Capability::Kline,
                    provider: "TEST_CODE_provider".to_owned(),
                    provider_observed_at: Some(wall),
                    locally_observed_at: wall,
                },
                Instant::now(),
            ),
            Err("success requires a supported started attempt".to_owned())
        );
        assert_eq!(
            tracker.record_failure(CapabilityFailure {
                capability: Capability::Kline,
                provider: "TEST_CODE_provider".to_owned(),
                locally_observed_at: wall,
                reason_code: "TEST_CODE_not_started".to_owned(),
                retryable: false,
                next_retry_at: None,
            }),
            Err("failure requires a supported started attempt".to_owned())
        );

        tracker.register_supported(Capability::News).unwrap();
        tracker
            .record_attempt_started(Capability::News, "TEST_CODE_provider", wall)
            .unwrap();
        assert_eq!(
            tracker.record_success(
                CapabilitySuccess {
                    capability: Capability::News,
                    provider: " ".to_owned(),
                    provider_observed_at: Some(wall),
                    locally_observed_at: wall,
                },
                Instant::now(),
            ),
            Err("provider must not be blank".to_owned())
        );

        tracker.register_supported(Capability::OrderBook).unwrap();
        tracker
            .record_attempt_started(Capability::OrderBook, "TEST_CODE_provider", wall)
            .unwrap();
        for (provider, reason_code) in [(" ", "TEST_CODE_failure"), ("TEST_CODE_provider", " ")] {
            assert_eq!(
                tracker.record_failure(CapabilityFailure {
                    capability: Capability::OrderBook,
                    provider: provider.to_owned(),
                    locally_observed_at: wall,
                    reason_code: reason_code.to_owned(),
                    retryable: true,
                    next_retry_at: None,
                }),
                Err("provider and reason_code must not be blank".to_owned())
            );
        }

        tracker
            .record_attempt_started(Capability::Kline, "TEST_CODE_provider", wall)
            .unwrap();
        tracker
            .record_attempt_started(Capability::Quote, "TEST_CODE_provider", wall)
            .unwrap();
        tracker
            .record_failure(CapabilityFailure {
                capability: Capability::Quote,
                provider: "TEST_CODE_provider".to_owned(),
                locally_observed_at: wall,
                reason_code: "TEST_CODE_provider_unavailable".to_owned(),
                retryable: true,
                next_retry_at: Some(wall + chrono::Duration::seconds(30)),
            })
            .unwrap();

        let snapshot = tracker
            .snapshot_at(CapabilityNow {
                wall,
                monotonic: Instant::now(),
                expected_now: true,
            })
            .unwrap();
        let quote = snapshot
            .observations
            .iter()
            .find(|observation| observation.capability == Capability::Quote)
            .expect("TEST_CODE Quote observation");
        assert_eq!(quote.state, CapabilityState::Failed);
        assert_eq!(quote.provider.as_deref(), Some("TEST_CODE_provider"));
        assert_eq!(
            quote.last_error_code.as_deref(),
            Some("TEST_CODE_provider_unavailable")
        );
        assert_eq!(quote.retryable, Some(true));
        assert_eq!(
            quote.next_retry_at,
            Some(wall + chrono::Duration::seconds(30))
        );
        assert!(snapshot.first_probe_complete);
    }

    fn input_all_fresh() -> DataHealthInput {
        DataHealthInput {
            capabilities: Capability::ALL
                .iter()
                .map(|c| CapabilityStatus::fresh(*c, 30))
                .collect(),
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        }
    }

    // ---- 全 Full 场景 ----

    #[test]
    fn full_when_all_fresh() {
        let h = evaluate(&input_all_fresh(), None);
        assert_eq!(h.mode, DataMode::Full);
        assert!(h.missing.is_empty());
        assert!(!h.is_changed());
    }

    #[test]
    fn degraded_when_only_orderbook_missing_after_provider_admission() {
        let mut input = input_all_fresh();
        input.capabilities[4] = CapabilityStatus::missing(Capability::OrderBook);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Degraded);
        assert!(h.missing.contains(&Capability::OrderBook));
    }

    #[test]
    fn full_when_orderbook_stale() {
        let mut input = input_all_fresh();
        // OrderBook 5 分钟前, 阈值 600s → 仍 ok
        input.capabilities[4] = CapabilityStatus::fresh(Capability::OrderBook, 300);
        let h = evaluate(&input, None);
        assert_eq!(h.mode, DataMode::Full);
    }

    #[test]
    fn degraded_when_orderbook_exceeds_its_existing_budget() {
        let mut input = input_all_fresh();
        input.capabilities[4] =
            CapabilityStatus::fresh(Capability::OrderBook, input.orderbook_max_age_secs + 1);
        assert_eq!(
            evaluate(&input, Some(DataMode::Full)).mode,
            DataMode::Degraded
        );
    }

    // ---- Degraded 场景 ----

    #[test]
    fn degraded_when_kline_stale() {
        let mut input = input_all_fresh();
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, KLINE_MAX_AGE_SECS + 1);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Degraded);
        assert!(h.is_changed(), "Full → Degraded 应算变更");
        assert!(h.missing.contains(&Capability::Kline));
        assert!(h.eta.as_ref().unwrap().contains("Kline"));
    }

    /// BR-216: 日频数据不得套用 120s 关键窗口, 否则结构上不可能新鲜。
    #[test]
    fn br216_kline_within_one_trading_day_stays_fresh() {
        let mut input = input_all_fresh();
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, 4 * 60 * 60);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Full);
        assert!(!h.missing.contains(&Capability::Kline));
    }

    /// BR-216: News 预算必须显著大于 `NEWS_POLL_INTERVAL` 默认 120s,
    /// 否则每两轮轮询之间必然抖出一次 stale。
    #[test]
    fn br216_news_survives_one_poll_interval() {
        let mut input = input_all_fresh();
        input.capabilities[3] = CapabilityStatus::fresh(Capability::News, 121);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Full);

        input.capabilities[3] = CapabilityStatus::fresh(Capability::News, NEWS_MAX_AGE_SECS + 1);
        assert_eq!(
            evaluate(&input, Some(DataMode::Full)).mode,
            DataMode::Degraded
        );
    }

    /// BR-216: 调用方放大 `critical_max_age_secs` 仍可整体放宽, 但不得收窄分级档。
    #[test]
    fn br216_widened_critical_window_never_narrows_graded_budgets() {
        let mut input = input_all_fresh();
        input.critical_max_age_secs = 2 * KLINE_MAX_AGE_SECS;
        assert_eq!(input.max_age_for(Capability::Kline), 2 * KLINE_MAX_AGE_SECS);
        assert_eq!(input.max_age_for(Capability::News), 2 * KLINE_MAX_AGE_SECS);
        input.critical_max_age_secs = 1;
        assert_eq!(input.max_age_for(Capability::Kline), KLINE_MAX_AGE_SECS);
        assert_eq!(input.max_age_for(Capability::News), NEWS_MAX_AGE_SECS);
        assert_eq!(input.max_age_for(Capability::Quote), 1);
    }

    #[test]
    fn degraded_when_news_missing() {
        let mut input = input_all_fresh();
        input.capabilities[3] = CapabilityStatus::missing(Capability::News);
        let h = evaluate(&input, None);
        assert_eq!(h.mode, DataMode::Degraded);
        assert!(h.missing.contains(&Capability::News));
    }

    /// BR-216: MoneyFlow 在 monitor 进程内无准入 provider
    /// (BR-190 `provider_capability_not_live_admitted`), 归为辅助能力:
    /// 只上横幅提示"未接入", 不得把 DataMode 永久压在 Degraded。
    #[test]
    fn br216_moneyflow_stale_surfaces_in_missing_without_degrading() {
        let mut input = input_all_fresh();
        input.capabilities[2] = CapabilityStatus::missing(Capability::MoneyFlow);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Full);
        assert!(
            h.missing.contains(&Capability::MoneyFlow),
            "未接入必须仍然可见, 不得静默 (§2.2)"
        );
    }

    #[test]
    fn br216_provider_backed_orderbook_is_critical() {
        assert!(Capability::OrderBook.is_critical());
        assert!(!Capability::MoneyFlow.is_critical());
    }

    // ---- Unsafe 场景 ----

    #[test]
    fn unsafe_when_quote_stale() {
        let mut input = input_all_fresh();
        input.capabilities[0] = CapabilityStatus::fresh(Capability::Quote, 150);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Unsafe);
        assert_eq!(h.missing[0], Capability::Quote);
    }

    #[test]
    fn unsafe_when_quote_missing() {
        let mut input = input_all_fresh();
        input.capabilities[0] = CapabilityStatus::missing(Capability::Quote);
        let h = evaluate(&input, None);
        assert_eq!(h.mode, DataMode::Unsafe);
    }

    #[test]
    fn unsafe_takes_priority_over_degraded() {
        let mut input = input_all_fresh();
        // Kline stale (Degraded) + Quote stale (Unsafe) → Unsafe
        input.capabilities[0] = CapabilityStatus::fresh(Capability::Quote, 200);
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, KLINE_MAX_AGE_SECS + 1);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Unsafe);
    }

    // ---- 状态变更 ----

    #[test]
    fn first_eval_not_changed() {
        let h = evaluate(&input_all_fresh(), None);
        assert!(!h.is_changed(), "首次评估不算变更");
    }

    #[test]
    fn same_mode_not_changed() {
        let h = evaluate(&input_all_fresh(), Some(DataMode::Full));
        assert!(!h.is_changed());
    }

    #[test]
    fn full_to_degraded_changed() {
        let mut input = input_all_fresh();
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, KLINE_MAX_AGE_SECS + 1);
        let h = evaluate(&input, Some(DataMode::Full));
        assert!(h.is_changed());
    }

    // ---- Capability 分类 ----

    #[test]
    fn capability_critical_classification() {
        assert!(Capability::Quote.is_critical());
        assert!(Capability::Kline.is_critical());
        assert!(Capability::News.is_critical());
        // BR-216: 无准入 provider 的能力归辅助, 否则安全门恒红即失效。
        assert!(
            !Capability::MoneyFlow.is_critical(),
            "MoneyFlow 未接入真实 provider, 不得永久压低 DataMode"
        );
        assert!(
            Capability::OrderBook.is_critical(),
            "BR-253 已准入真实 OrderBook，缺失必须降级"
        );
    }

    #[test]
    fn capability_labels() {
        assert_eq!(Capability::Quote.label(), "Quote");
        assert_eq!(Capability::Kline.label(), "Kline");
        assert_eq!(Capability::MoneyFlow.label(), "MoneyFlow");
        assert_eq!(Capability::News.label(), "News");
        assert_eq!(Capability::OrderBook.label(), "OrderBook");
    }

    #[test]
    fn data_mode_labels() {
        assert_eq!(DataMode::Full.label(), "Full");
        assert_eq!(DataMode::Degraded.label(), "Degraded");
        assert_eq!(DataMode::Unsafe.label(), "Unsafe");
    }

    #[test]
    fn br135_unchanged_unsafe_switches_from_external_status_to_internal_heartbeat() {
        let start = Instant::now();
        let mut state = PersistentUnsafeReminder::default();
        let missing = [Capability::Quote, Capability::News];
        assert_eq!(UNSAFE_HEARTBEAT_AUDIT_KIND, "data_mode_unsafe_heartbeat_v1");
        assert_eq!(UNSAFE_HEARTBEAT_AUDIT_OUTCOME, "Deduped");
        assert_eq!(UNSAFE_HEARTBEAT_AUDIT_CHANNEL, "internal_audit");

        let first = state.action(DataMode::Unsafe, &missing, start).unwrap();
        assert!(matches!(
            first,
            PersistentUnsafeAction::ExternalStatus { .. }
        ));
        state.record_external_confirmed(DataMode::Unsafe, &missing, start);

        assert_eq!(
            state
                .action(
                    DataMode::Unsafe,
                    &missing,
                    start + Duration::from_secs(1_799),
                )
                .unwrap(),
            PersistentUnsafeAction::Silent
        );
        let due = state
            .action(
                DataMode::Unsafe,
                &missing,
                start + Duration::from_secs(1_800),
            )
            .unwrap();
        let fingerprint = match due {
            PersistentUnsafeAction::InternalHeartbeat { fingerprint } => fingerprint,
            other => panic!("expected internal heartbeat, got {other:?}"),
        };

        state
            .record_heartbeat_confirmed(&fingerprint, start + Duration::from_secs(1_800))
            .unwrap();
        assert_eq!(
            state
                .action(
                    DataMode::Unsafe,
                    &missing,
                    start + Duration::from_secs(1_801),
                )
                .unwrap(),
            PersistentUnsafeAction::Silent
        );
    }

    #[test]
    fn br135_fingerprint_is_canonical_and_missing_change_is_external_fact() {
        let start = Instant::now();
        let mut state = PersistentUnsafeReminder::default();
        let unordered = [Capability::News, Capability::Quote, Capability::News];
        let canonical = [Capability::Quote, Capability::News];

        let first = state.action(DataMode::Unsafe, &unordered, start).unwrap();
        let first_fingerprint = match first {
            PersistentUnsafeAction::ExternalStatus { fingerprint } => fingerprint,
            other => panic!("expected external status, got {other:?}"),
        };
        assert_eq!(
            first_fingerprint,
            unsafe_event_fingerprint(&canonical),
            "input order and duplicates must not affect event identity"
        );
        state.record_external_confirmed(DataMode::Unsafe, &canonical, start);

        assert_eq!(
            state
                .action(DataMode::Unsafe, &unordered, start + Duration::from_secs(1),)
                .unwrap(),
            PersistentUnsafeAction::Silent
        );
        assert!(matches!(
            state
                .action(
                    DataMode::Unsafe,
                    &[Capability::Quote, Capability::Kline, Capability::News],
                    start + Duration::from_secs(2),
                )
                .unwrap(),
            PersistentUnsafeAction::ExternalStatus { .. }
        ));
    }

    #[test]
    fn br135_failures_and_unconfirmed_recovery_do_not_advance_state() {
        let start = Instant::now();
        let mut state = PersistentUnsafeReminder::default();
        let missing = [Capability::Quote];
        state.record_external_confirmed(DataMode::Unsafe, &missing, start);

        let due_at = start + Duration::from_secs(1_800);
        let due = state.action(DataMode::Unsafe, &missing, due_at).unwrap();
        assert!(matches!(
            due,
            PersistentUnsafeAction::InternalHeartbeat { .. }
        ));
        assert_eq!(
            state.action(DataMode::Unsafe, &missing, due_at).unwrap(),
            due,
            "an unconfirmed audit must remain due"
        );

        assert_eq!(
            state.action(DataMode::Full, &[], due_at).unwrap(),
            PersistentUnsafeAction::Silent
        );
        assert!(matches!(
            state.action(DataMode::Unsafe, &missing, due_at).unwrap(),
            PersistentUnsafeAction::InternalHeartbeat { .. }
        ));
        state.record_external_confirmed(DataMode::Full, &[], due_at);
        assert!(matches!(
            state
                .action(DataMode::Unsafe, &missing, due_at + Duration::from_secs(1),)
                .unwrap(),
            PersistentUnsafeAction::ExternalStatus { .. }
        ));
    }

    #[test]
    fn br135_clock_rollback_and_wrong_fingerprint_are_explicit_errors() {
        let start = Instant::now();
        let mut state = PersistentUnsafeReminder::default();
        let missing = [Capability::Quote];
        state.record_external_confirmed(
            DataMode::Unsafe,
            &missing,
            start + Duration::from_secs(10),
        );

        assert!(state
            .action(DataMode::Unsafe, &missing, start)
            .unwrap_err()
            .contains("moved backwards"));
        assert!(state
            .record_heartbeat_confirmed(
                "data_mode_unsafe_v1|Unsafe|News",
                start + Duration::from_secs(1_800),
            )
            .unwrap_err()
            .contains("fingerprint"));
        assert_eq!(
            state
                .action(
                    DataMode::Unsafe,
                    &missing,
                    start + Duration::from_secs(1_809),
                )
                .unwrap(),
            PersistentUnsafeAction::Silent
        );
        assert!(matches!(
            state
                .action(
                    DataMode::Unsafe,
                    &missing,
                    start + Duration::from_secs(1_810),
                )
                .unwrap(),
            PersistentUnsafeAction::InternalHeartbeat { .. }
        ));
    }

    // ---- 输入辅助 ----

    #[test]
    fn input_from_pairs_basic() {
        let input = input_from_pairs(
            120,
            &[(Capability::Quote, Some(10)), (Capability::OrderBook, None)],
        );
        assert_eq!(input.capabilities.len(), 2);
        assert!(input.capabilities[0].is_ok(120));
        assert!(!input.capabilities[1].is_ok(600));
    }

    #[test]
    fn tracker_input_keeps_never_successful_capabilities_missing() {
        let now = Instant::now();
        let mut successes = HashMap::new();
        successes.insert(Capability::Quote, now);

        let input = input_from_successes_at(&successes, now, 120, 600);

        assert_eq!(input.capabilities.len(), Capability::ALL.len());
        assert_eq!(input.capabilities[0].staleness_secs, Some(0));
        assert!(input.capabilities[1..]
            .iter()
            .all(|status| status.staleness_secs.is_none()));
    }

    #[test]
    fn tracker_input_uses_elapsed_time_since_success() {
        let now = Instant::now();
        let mut successes = HashMap::new();
        successes.insert(
            Capability::Kline,
            now.checked_sub(std::time::Duration::from_secs(121))
                .expect("test instant must support a short subtraction"),
        );

        let input = input_from_successes_at(&successes, now, 120, 600);
        let kline = input
            .capabilities
            .iter()
            .find(|status| status.cap == Capability::Kline)
            .expect("Kline status must exist");

        assert_eq!(kline.staleness_secs, Some(121));
        assert!(!kline.is_ok(input.critical_max_age_secs));
    }

    // ---- 边界 ----

    #[test]
    fn staleness_at_threshold_is_ok() {
        // staleness = max_age → still ok
        let cs = CapabilityStatus::fresh(Capability::Quote, 120);
        assert!(cs.is_ok(120));
    }

    #[test]
    fn staleness_above_threshold_is_stale() {
        let cs = CapabilityStatus::fresh(Capability::Quote, 121);
        assert!(!cs.is_ok(120));
    }

    /// 异常: missing 字段空, prev=Some 任意状态 → is_changed 取决于新模式
    #[test]
    fn all_missing_falls_to_unsafe() {
        let input = DataHealthInput::default(); // 5 个全 missing
        let h = evaluate(&input, None);
        assert_eq!(
            h.mode,
            DataMode::Unsafe,
            "全 missing → Quote missing → Unsafe"
        );
        assert_eq!(h.missing.len(), 5);
    }
}
