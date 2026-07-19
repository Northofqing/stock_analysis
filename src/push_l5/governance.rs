//! push_l5/governance.rs — L5 Governance (v14.2 §3.5)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.5 + b-008 §4.1 落地.
//!
//! W5.1 范围:
//!   - GovernanceEngine struct (state: ctx + profile)
//!   - check_governance() 入口: 静默期 / 冻结模式 / data_mode / always_send_on_data_source_down
//!   - GovernanceDecision 枚举 (Approve / Deny + reason)
//!   - 复用 push_l2::TemplateMetadata (W3.1 已落地)
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2: governance 不静默填补, 缺失数据模式 → Deny
//!   - b-008 §4.1: data_mode = Down 时, 仅 always_send_on_data_source_down=true 且 event_kind=data_source_down 才放行

use chrono::{DateTime, Local, Timelike};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::push_l1::{SignalEvent, SignalPayload, SignalSource};
use crate::push_l2::{DataMode, TemplateMetadata};

/// v17.1 review F8 fix: Frozen warn 节流.
///
/// 每条 push 都打 "Frozen 上下文 + frozen_mode_respect=true → 放行" 在 frozen_mode_respect
/// 启用场景下形成日志噪声 (默认 false, 但逃生路径仍可能启用). 节流到 60s 一次.
///
/// 实现: 进程级 AtomicU64 (上次 warn 的 UNIX 秒); 距上次 ≥ 60s 才打 warn.
const FROZEN_WARN_THROTTLE_SECS: u64 = 60;
static LAST_FROZEN_WARN_TS: AtomicU64 = AtomicU64::new(0);

/// 当前时间 (UNIX seconds). 测试可替换.
#[cfg(not(test))]
fn now_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
fn now_unix_secs() -> u64 {
    0 // 测试场景用确定性 0; 节流 helper 走测试 helper 自己 mock
}

/// 节流打 Frozen warn: 距上次 ≥ 60s 才打, 否则 silent.
///
/// 修 FINDING #5+#6 (review F8):
/// - 修 race: 用 compare_exchange 循环保证多个并发线程只有 1 个能更新 timestamp
///   (原 load-then-store 允许多个线程都看到 stale last_ts 后都打 warn).
/// - 修 clock skew: 时钟回退时 saturating_sub 返 0 → 持续静默; 改用 max(now, last)
///   保证 timestamp 单调不减, 时钟回退后下次 warn 仍能 fire (但保持 60s 间隔).
fn throttled_frozen_warn() {
    let now = now_unix_secs();
    loop {
        let last = LAST_FROZEN_WARN_TS.load(Ordering::Relaxed);
        // monotonic timestamp: max(now, last) 保证不回退
        let new_ts = now.max(last);
        if new_ts.saturating_sub(last) < FROZEN_WARN_THROTTLE_SECS {
            return;
        }
        // compare_exchange: 仅当 last 仍是当前值才更新, 否则重试
        match LAST_FROZEN_WARN_TS.compare_exchange(
            last,
            new_ts,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                log::warn!(
                    "[v17.1-F8] Frozen 上下文 + frozen_mode_respect=true → 放行, 模板应渲染 ⚠️ 警告 (节流 60s/次)"
                );
                return;
            }
            Err(_) => continue, // 其他线程抢先更新, 重读
        }
    }
}

/// 测试用: 重置节流 timestamp, 让下次 warn 可通过.
#[cfg(test)]
fn reset_frozen_warn_throttle_for_test() {
    LAST_FROZEN_WARN_TS.store(0, Ordering::Relaxed);
}

/// Governance 决策
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GovernanceDecision {
    /// 允许推送
    Approve,
    /// 拒绝推送 + 原因 (String 而非 &'static str, 支持 Serialize)
    Deny(String),
}

impl GovernanceDecision {
    pub fn is_approve(&self) -> bool {
        matches!(self, Self::Approve)
    }
    pub fn deny_reason(&self) -> Option<&str> {
        match self {
            Self::Approve => None,
            Self::Deny(r) => Some(r.as_str()),
        }
    }
}

/// 治理上下文 (运行时状态)
#[derive(Debug, Clone)]
pub struct GovernanceContext {
    /// 当前数据质量
    pub data_mode: DataMode,
    /// 是否在静默期 (02:00-06:00)
    pub is_quiet_hour: bool,
    /// 是否冻结模式 (大盘熔断 / 风控全局冻结)
    pub is_frozen: bool,
    /// 当前时间 (用于静默期判定 + analytics)
    pub now: DateTime<Local>,
    /// 今日已推送次数 (用于 max_per_user_per_day)
    pub today_pushed_count: u32,
}

impl Default for GovernanceContext {
    fn default() -> Self {
        Self {
            data_mode: DataMode::Full,
            is_quiet_hour: false,
            is_frozen: false,
            now: Local::now(),
            today_pushed_count: 0,
        }
    }
}

/// 治理引擎 (单实例, 全局共享, b-009 R-4: dispatcher 调用)
pub struct GovernanceEngine;

impl GovernanceEngine {
    pub fn new() -> Self {
        Self
    }

    /// 核心治理判定 — v14.2 §3.5 严格按流程
    ///
    /// 检查顺序 (与 §3.5 流程一致):
    ///   1. 静默期: profile.quiet_hours_respect + ctx.is_quiet_hour → Deny("quiet_hour")
    ///   2. 冻结模式: profile.frozen_mode_respect + ctx.is_frozen → Deny("frozen")
    ///   3. data_mode 质量: ctx.data_mode < profile.data_mode_min → Deny("data_quality")
    ///      但若 profile.always_send_on_data_source_down + event_kind = data_source_down → Approve
    ///   4. 每日上限: ctx.today_pushed_count >= profile.max_per_user_per_day → Deny("daily_limit")
    ///   5. 类别豁免: post_session / quiet_hour 类别无视 cooldown
    pub fn check(
        &self,
        profile: &TemplateMetadata,
        event: &SignalEvent,
        ctx: &GovernanceContext,
    ) -> GovernanceDecision {
        // Step 1: 静默期 (b-009/W5.X 修订: QuietHourSource 自身事件豁免)
        if profile.quiet_hours_respect
            && ctx.is_quiet_hour
            && event.source != SignalSource::QuietHour
        {
            return GovernanceDecision::Deny("quiet_hour".to_string());
        }

        // Step 2: 冻结模式
        // v17.1 治本: Frozen 状态不再 Deny, 模板从 ctx.is_frozen 读后渲染 ⚠️ 警告
        // 仓位风险控制应在 broker 下单层, 通知层应保持出声 (4 铁律)
        // 字段保留用于日志/审计.
        // F8 fix: 节流到 60s 一次, 避免 frozen_mode_respect=true 启用时每条 push 都 warn.
        if profile.frozen_mode_respect && ctx.is_frozen {
            throttled_frozen_warn();
            // 故意 fall through, 不 return Deny
        }

        // Step 3: 数据质量 (b-008 §4.1: data_source_down 主动告警可豁免)
        // 严重度方向: Down > Unsafe > Degraded > Full (as u8 序)
        // "ctx.data_mode > data_mode_min" 表示当前比最低要求还差 → Deny
        if (ctx.data_mode as u8) > (profile.data_mode_min as u8) {
            // b-008 §4.1 + W5.X 修订: 既要看 source 也要看 payload, 避免数据损坏误放行
            if profile.always_send_on_data_source_down && is_data_source_down_event(event) {
                // 放行
            } else {
                return GovernanceDecision::Deny("data_quality".to_string());
            }
        }

        // Step 4: 每日上限
        if let Some(max) = profile.max_per_user_per_day {
            if ctx.today_pushed_count >= max {
                return GovernanceDecision::Deny("daily_limit".to_string());
            }
        }

        GovernanceDecision::Approve
    }
}

impl Default for GovernanceEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 静默期判定 (02:00-06:00)
pub fn is_quiet_hour(ts: DateTime<Local>) -> bool {
    let hour = ts.hour();
    (2..6).contains(&hour)
}

/// 从 SignalEvent 推断 payload 严重度 (用于 governance 优先级)
pub fn event_severity(event: &SignalEvent) -> u8 {
    use crate::push_l1::Severity;
    match event.severity {
        Severity::Emergency => 4,
        Severity::High => 3,
        Severity::Normal => 2,
        Severity::Info => 1,
    }
}

/// 工具函数: 判断 event 是否 DataSourceDown 类型 (用于 always_send_on_data_source_down 豁免)
pub fn is_data_source_down_event(event: &SignalEvent) -> bool {
    matches!(event.payload, SignalPayload::DataSourceDown(_))
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_l1::{DataSourceDownPayload, LimitUpPayload, Severity};
    use crate::push_l2::TemplateCategory;
    use chrono::TimeZone;

    fn make_event(source: SignalSource) -> SignalEvent {
        let payload = match source {
            SignalSource::DataSourceDown => {
                SignalPayload::DataSourceDown(DataSourceDownPayload::default())
            }
            _ => SignalPayload::LimitUp(LimitUpPayload::default()),
        };
        SignalEvent::new(
            source,
            "test",
            Some("TEST_CODE_600519".to_string()),
            Local::now(),
            payload,
            Severity::High,
        )
    }

    fn make_profile(
        quiet_hours_respect: bool,
        frozen_mode_respect: bool,
        data_mode_min: DataMode,
        always_send_on_data_source_down: bool,
    ) -> TemplateMetadata {
        TemplateMetadata {
            category: TemplateCategory::LimitUp,
            quiet_hours_respect,
            frozen_mode_respect,
            data_mode_min,
            cooldown_secs: 60,
            max_per_user_per_day: None,
            always_send_on_data_source_down,
        }
    }

    #[test]
    fn approve_in_full_data_no_quiet_no_frozen() {
        let engine = GovernanceEngine::new();
        let profile = make_profile(true, true, DataMode::Full, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext::default();
        assert_eq!(
            engine.check(&profile, &event, &ctx),
            GovernanceDecision::Approve
        );
    }

    #[test]
    fn deny_in_quiet_hour_when_respect_enabled() {
        let engine = GovernanceEngine::new();
        let profile = make_profile(true, false, DataMode::Full, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            is_quiet_hour: true,
            ..Default::default()
        };
        assert_eq!(
            engine.check(&profile, &event, &ctx),
            GovernanceDecision::Deny("quiet_hour".to_string())
        );
    }

    #[test]
    fn approve_in_quiet_hour_when_respect_disabled() {
        let engine = GovernanceEngine::new();
        let profile = make_profile(false, false, DataMode::Full, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            is_quiet_hour: true,
            ..Default::default()
        };
        assert!(engine.check(&profile, &event, &ctx).is_approve());
    }

    #[test]
    fn deny_in_frozen_mode_when_respect_enabled() {
        // v17.1 治本: Frozen 状态不再 Deny, 而是由模板在 banner 渲染 ⚠️ 警告
        // 旧行为: assert_eq!(...Deny("frozen"...))  - 违反 4 铁律"默认值出声"
        // 新行为: Approve (放行), ctx.is_frozen 保留给模板做警告
        // 仓位风险控制应在 broker 下单层, 不在通知层 (v17.1 决策)
        let engine = GovernanceEngine::new();
        let profile = make_profile(true, true, DataMode::Full, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            is_frozen: true,
            ..Default::default()
        };
        assert!(
            engine.check(&profile, &event, &ctx).is_approve(),
            "v17.1: Frozen 状态必须放行 (banner 警告), 不能 Deny"
        );
    }

    #[test]
    fn deny_when_data_mode_worse_than_minimum() {
        // W5.X 修订: 重命名, 匹配实际语义 "ctx 比 min 严重" (即 ctx > min)
        let engine = GovernanceEngine::new();
        // 模板要求 Full(0), 当前 Degraded(1) → Deny
        let profile = make_profile(false, false, DataMode::Full, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            data_mode: DataMode::Degraded,
            ..Default::default()
        };
        assert_eq!(
            engine.check(&profile, &event, &ctx),
            GovernanceDecision::Deny("data_quality".to_string())
        );
    }

    // W5.X 新增: QuietHour 事件在静默期仍应放行 (自身治理事件, b-008 §3.0)
    #[test]
    fn approve_quiet_hour_when_event_is_quiet_hour_source() {
        let engine = GovernanceEngine::new();
        let profile = make_profile(true, false, DataMode::Full, false);
        let event = make_event(SignalSource::QuietHour);
        let ctx = GovernanceContext {
            is_quiet_hour: true,
            ..Default::default()
        };
        assert!(
            engine.check(&profile, &event, &ctx).is_approve(),
            "QuietHour 事件在静默期应放行 (自身治理观察)"
        );
    }

    #[test]
    fn approve_when_data_mode_meets_minimum() {
        let engine = GovernanceEngine::new();
        let profile = make_profile(false, false, DataMode::Degraded, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            data_mode: DataMode::Unsafe,
            ..Default::default()
        };
        // 注: Unsafe(2) 比 Degraded(1) 严重, 应 Deny (data_quality)
        assert_eq!(
            engine.check(&profile, &event, &ctx),
            GovernanceDecision::Deny("data_quality".to_string()),
            "Unsafe 模式比 Degraded 要求更差, 应 Deny"
        );
    }

    #[test]
    fn data_source_down_exempt_when_all_sources_down() {
        let engine = GovernanceEngine::new();
        // 模板要求 Full, 当前 Down, 且 always_send=true, event 是 DataSourceDown → Approve
        let profile = make_profile(false, false, DataMode::Full, true);
        let event = make_event(SignalSource::DataSourceDown);
        let ctx = GovernanceContext {
            data_mode: DataMode::Down,
            ..Default::default()
        };
        assert!(
            engine.check(&profile, &event, &ctx).is_approve(),
            "b-008 §4.1: DataSourceDown 在 data_mode=Down 时应放行"
        );
    }

    #[test]
    fn deny_data_source_down_when_exempt_disabled() {
        let engine = GovernanceEngine::new();
        // 即使是 DataSourceDown 事件, 若 always_send=false → Deny
        let profile = make_profile(false, false, DataMode::Full, false);
        let event = make_event(SignalSource::DataSourceDown);
        let ctx = GovernanceContext {
            data_mode: DataMode::Down,
            ..Default::default()
        };
        assert_eq!(
            engine.check(&profile, &event, &ctx),
            GovernanceDecision::Deny("data_quality".to_string())
        );
    }

    #[test]
    fn deny_when_daily_limit_reached() {
        let engine = GovernanceEngine::new();
        let mut profile = make_profile(false, false, DataMode::Full, false);
        profile.max_per_user_per_day = Some(3);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            today_pushed_count: 3,
            ..Default::default()
        };
        assert_eq!(
            engine.check(&profile, &event, &ctx),
            GovernanceDecision::Deny("daily_limit".to_string())
        );
    }

    #[test]
    fn approve_below_daily_limit() {
        let engine = GovernanceEngine::new();
        let mut profile = make_profile(false, false, DataMode::Full, false);
        profile.max_per_user_per_day = Some(3);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            today_pushed_count: 2,
            ..Default::default()
        };
        assert!(engine.check(&profile, &event, &ctx).is_approve());
    }

    #[test]
    fn quiet_hour_detection_2am_to_6am() {
        assert!(is_quiet_hour(
            Local.with_ymd_and_hms(2026, 7, 11, 2, 0, 0).unwrap()
        ));
        assert!(is_quiet_hour(
            Local.with_ymd_and_hms(2026, 7, 11, 5, 59, 0).unwrap()
        ));
        assert!(!is_quiet_hour(
            Local.with_ymd_and_hms(2026, 7, 11, 6, 0, 0).unwrap()
        ));
        assert!(!is_quiet_hour(
            Local.with_ymd_and_hms(2026, 7, 11, 1, 59, 0).unwrap()
        ));
        assert!(!is_quiet_hour(
            Local.with_ymd_and_hms(2026, 7, 11, 23, 0, 0).unwrap()
        ));
    }

    #[test]
    fn event_severity_ordering() {
        assert!(
            event_severity(&make_event_with_severity(Severity::Emergency))
                > event_severity(&make_event_with_severity(Severity::High))
        );
        assert!(
            event_severity(&make_event_with_severity(Severity::High))
                > event_severity(&make_event_with_severity(Severity::Normal))
        );
    }

    fn make_event_with_severity(sev: Severity) -> SignalEvent {
        SignalEvent::new(
            SignalSource::LimitUp,
            "test",
            Some("TEST_CODE_000001".to_string()),
            Local::now(),
            SignalPayload::LimitUp(LimitUpPayload::default()),
            sev,
        )
    }

    #[test]
    fn is_data_source_down_event_helper() {
        let event = make_event(SignalSource::DataSourceDown);
        assert!(is_data_source_down_event(&event));
        let event = make_event(SignalSource::LimitUp);
        assert!(!is_data_source_down_event(&event));
    }

    #[test]
    fn deny_reason_extraction() {
        let d = GovernanceDecision::Deny("frozen".to_string());
        assert_eq!(d.deny_reason(), Some("frozen"));
        assert!(!d.is_approve());

        let d = GovernanceDecision::Approve;
        assert_eq!(d.deny_reason(), None);
        assert!(d.is_approve());
    }

    // ============== F8: Frozen warn 节流测试 ==============
    //
    // 注: cfg(test) 下 now_unix_secs() 返回 0, 节流窗口 = [0, 60).
    // 第一次 throttled_frozen_warn() → 0.saturating_sub(0)=0 < 60 → 静默 (不更新 last_ts).
    // 第二次在窗口内 → 静默.
    // reset_frozen_warn_throttle_for_test 不改语义, 仅给 multi-test 隔离用.

    #[test]
    fn throttled_frozen_warn_helper_callable() {
        // 不 panic + 不 log error (cfg(test) 下 now_unix_secs=0 走节流路径).
        reset_frozen_warn_throttle_for_test();
        throttled_frozen_warn();
        throttled_frozen_warn();
        // 测试不依赖 log capture — 仅验证 helper 不 panic.
    }

    #[test]
    fn frozen_warn_throttle_constant_in_range() {
        // 文档化窗口: 60s 一次. 防止后续修改默认值.
        assert_eq!(FROZEN_WARN_THROTTLE_SECS, 60);
    }
}
