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

use crate::push_l1::{SignalEvent, SignalPayload, SignalSource};
use crate::push_l2::{DataMode, TemplateCategory, TemplateMetadata};

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
        if profile.frozen_mode_respect && ctx.is_frozen {
            return GovernanceDecision::Deny("frozen".to_string());
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

        // Step 5: 类别豁免 (post_session / quiet_hour 不限 cooldown, 但本函数只做 governance, cooldown 在 dispatcher 单独判定)
        let _ = event_kind_exempt_from_cooldown(profile.category);

        GovernanceDecision::Approve
    }
}

impl Default for GovernanceEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 类别豁免 (用于 dispatcher.cooldown_table 的 bypass 逻辑)
pub fn event_kind_exempt_from_cooldown(category: TemplateCategory) -> bool {
    matches!(category, TemplateCategory::PostSession | TemplateCategory::QuietHour)
}

/// 静默期判定 (02:00-06:00)
pub fn is_quiet_hour(ts: DateTime<Local>) -> bool {
    let hour = ts.hour();
    hour >= 2 && hour < 6
}

/// DataMode 严重度 (与 push_l2::DataMode PartialOrd 一致, 这里数值化便于比较)
///
/// 注: Push 的 DataMode 已有 PartialOrd, 这里是为了让 §3.5 的 "ctx.data_mode < profile.data_mode_min" 表述更明确
/// (即: 当前数据质量比模板要求的最低质量还要差)
pub fn data_mode_severity(mode: DataMode) -> u8 {
    match mode {
        DataMode::Full => 0,
        DataMode::Degraded => 1,
        DataMode::Unsafe => 2,
        DataMode::Down => 3,
    }
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
    use chrono::TimeZone;

    fn make_event(source: SignalSource) -> SignalEvent {
        let payload = match source {
            SignalSource::DataSourceDown => SignalPayload::DataSourceDown(DataSourceDownPayload::default()),
            _ => SignalPayload::LimitUp(LimitUpPayload::default()),
        };
        SignalEvent::new(
            source,
            "test",
            Some("600519".to_string()),
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
        assert_eq!(engine.check(&profile, &event, &ctx), GovernanceDecision::Approve);
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
        assert_eq!(engine.check(&profile, &event, &ctx), GovernanceDecision::Deny("quiet_hour".to_string()));
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
        let engine = GovernanceEngine::new();
        let profile = make_profile(true, true, DataMode::Full, false);
        let event = make_event(SignalSource::LimitUp);
        let ctx = GovernanceContext {
            is_frozen: true,
            ..Default::default()
        };
        assert_eq!(engine.check(&profile, &event, &ctx), GovernanceDecision::Deny("frozen".to_string()));
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
        assert_eq!(engine.check(&profile, &event, &ctx), GovernanceDecision::Deny("data_quality".to_string()));
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
        assert!(engine.check(&profile, &event, &ctx).is_approve(),
            "QuietHour 事件在静默期应放行 (自身治理观察)");
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
        assert!(engine.check(&profile, &event, &ctx).is_approve(),
            "b-008 §4.1: DataSourceDown 在 data_mode=Down 时应放行");
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
        assert_eq!(engine.check(&profile, &event, &ctx), GovernanceDecision::Deny("data_quality".to_string()));
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
        assert_eq!(engine.check(&profile, &event, &ctx), GovernanceDecision::Deny("daily_limit".to_string()));
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
        assert!(is_quiet_hour(Local.with_ymd_and_hms(2026, 7, 11, 2, 0, 0).unwrap()));
        assert!(is_quiet_hour(Local.with_ymd_and_hms(2026, 7, 11, 5, 59, 0).unwrap()));
        assert!(!is_quiet_hour(Local.with_ymd_and_hms(2026, 7, 11, 6, 0, 0).unwrap()));
        assert!(!is_quiet_hour(Local.with_ymd_and_hms(2026, 7, 11, 1, 59, 0).unwrap()));
        assert!(!is_quiet_hour(Local.with_ymd_and_hms(2026, 7, 11, 23, 0, 0).unwrap()));
    }

    #[test]
    fn data_mode_severity_ordering() {
        assert!(data_mode_severity(DataMode::Down) > data_mode_severity(DataMode::Unsafe));
        assert!(data_mode_severity(DataMode::Unsafe) > data_mode_severity(DataMode::Degraded));
        assert!(data_mode_severity(DataMode::Degraded) > data_mode_severity(DataMode::Full));
    }

    #[test]
    fn event_severity_ordering() {
        assert!(event_severity(&make_event_with_severity(Severity::Emergency))
            > event_severity(&make_event_with_severity(Severity::High)));
        assert!(event_severity(&make_event_with_severity(Severity::High))
            > event_severity(&make_event_with_severity(Severity::Normal)));
    }

    fn make_event_with_severity(sev: Severity) -> SignalEvent {
        SignalEvent::new(
            SignalSource::LimitUp,
            "test",
            Some("000001".to_string()),
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
    fn post_session_exempt_from_cooldown() {
        assert!(event_kind_exempt_from_cooldown(TemplateCategory::PostSession));
        assert!(event_kind_exempt_from_cooldown(TemplateCategory::QuietHour));
        assert!(!event_kind_exempt_from_cooldown(TemplateCategory::LimitUp));
        assert!(!event_kind_exempt_from_cooldown(TemplateCategory::Holding));
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
}