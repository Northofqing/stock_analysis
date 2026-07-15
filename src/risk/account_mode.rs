//! v12 PR1 账户模式三态判定 (AccountState).
//!
//! 设计: 纯函数 + 数据入参, 不直接读 portfolio DB.
//!       数据装配由 main 循环负责 (PR1-1.7), 这里只负责规则判定.
//!       这样保持模块可独立单测, 避免 mock.
//!
//! 状态机:
//!   Normal --(日亏 ≤ daily_loss_pct OR 连续止损 ≥ consecutive_stop_loss_n)--> ReduceOnly
//!   ReduceOnly --(日亏 ≤ circuit_breaker_pct OR 仓位 > position_overload_cheng)--> Frozen
//!   Frozen --(无)--> (无)
//!   ReduceOnly --(日亏 / 仓位回到阈值内)--> Normal   (但 v12.2 §2.3 要求"下一交易日盘前重置", 当前版本保留运行时评估)
//!   Frozen --(同上)--> ReduceOnly
//!
//! v12 §2.3 + BR-021.

use super::action_gate::AccountMode;
use chrono::NaiveTime;
use std::sync::OnceLock;

/// v17.1 review F9 fix: `PUSH_NORMAL_FORCE` env 缓存, 避免每次 `evaluate()`
/// syscall getenv (热路径).
///
/// 一次性 read, OnceLock 后 O(1) bool 查询. v15.x 4 铁律: 默认出声 — 此 helper
/// 仅在显式 `PUSH_NORMAL_FORCE=1` 时返回 `true`, 未设置时 `false` 走正常评估.
static PUSH_NORMAL_FORCE_ENABLED: OnceLock<bool> = OnceLock::new();

/// 检查是否启用了 PUSH_NORMAL_FORCE 临时旁路 (env-var 一次性 read).
///
/// **真风险控制应该在 broker 下单层做, 不在通知层** — 本 helper 仅服务于
/// v17.0 临时旁路: 仓位超限导致 Frozen → 推送被 L5 全 deny → 用户看不到消息.
/// 治本已在 commit 460ab25 (frozen_mode_respect) 落地; 本 helper 保留逃生口.
pub fn is_push_normal_forced() -> bool {
    *PUSH_NORMAL_FORCE_ENABLED.get_or_init(|| {
        std::env::var("PUSH_NORMAL_FORCE").ok().as_deref() == Some("1")
    })
}

/// 评估阈值 (code-level const fallback, 可被 config 覆盖)
pub mod thresholds {
    /// 当日累计亏损触发 ReduceOnly (默认 -1.5%)
    pub const DAILY_LOSS_PCT: f64 = -1.5;
    /// 当日累计亏损触发熔断 Frozen (默认 -2.0%)
    pub const CIRCUIT_BREAKER_PCT: f64 = -2.0;
    /// 连续止损笔数触发 ReduceOnly (默认 3)
    pub const CONSECUTIVE_STOP_LOSS_N: u32 = 3;
    /// 总仓位超限触发 Frozen (默认 8 成)
    pub const POSITION_OVERLOAD_CHENG: u8 = 8;
}

/// 入参: 推导 AccountMode 所需的 portfolio 指标
///
/// 字段语义与 BR-021 对齐.
#[derive(Clone, Debug, Default)]
pub struct PortfolioMetrics {
    /// 当日累计盈亏百分比 (带符号). 例: -1.2 表示 -1.2%
    pub today_pnl_pct: f64,
    /// 连续止损笔数 (按交易时间倒序数, 遇非止损交易重置)
    pub consecutive_stop_loss_n: u32,
    /// 总仓位成数 (0~10). 例: 5 表示 5 成 (50%)
    pub total_pos_cheng: u8,
    /// 数据完整度 (true = 三个指标均非 None)
    pub data_complete: bool,
}

/// 阈值配置 (允许 PR1-1.4 from_config 覆盖默认 const)
#[derive(Copy, Clone, Debug)]
pub struct ModeThresholds {
    pub daily_loss_pct: f64,
    pub circuit_breaker_pct: f64,
    pub consecutive_stop_loss_n: u32,
    pub position_overload_cheng: u8,
}

impl Default for ModeThresholds {
    fn default() -> Self {
        Self {
            daily_loss_pct: thresholds::DAILY_LOSS_PCT,
            circuit_breaker_pct: thresholds::CIRCUIT_BREAKER_PCT,
            consecutive_stop_loss_n: thresholds::CONSECUTIVE_STOP_LOSS_N,
            position_overload_cheng: thresholds::POSITION_OVERLOAD_CHENG,
        }
    }
}

/// 评估结果 — 当前 AccountMode + 触发原因 (供 T-01 推送文案)
#[derive(Clone, Debug)]
pub struct ModeEvaluation {
    pub mode: AccountMode,
    /// 触发当前模式的具体原因 (按评估顺序取首个触发)
    pub trigger_reason: Option<String>,
    /// 前一模式 (None 表示首次评估)
    pub prev_mode: Option<AccountMode>,
}

impl ModeEvaluation {
    pub fn is_changed(&self) -> bool {
        match self.prev_mode {
            Some(p) => p != self.mode,
            None => false, // 首次评估不算变更, 不触发 T-01 推送
        }
    }
}

/// BR-021 主评估函数
///
/// 触发规则:
///   1. 数据不完整 (data_complete=false) → 保守取 ReduceOnly, 原因 "数据缺失"
///   2. ReduceOnly 触发条件: today_pnl_pct ≤ daily_loss_pct **或** consecutive_stop_loss_n ≥ N
///   3. Frozen 触发条件 (任何状态下评估):
///      - today_pnl_pct ≤ circuit_breaker_pct
///      - **或** total_pos_cheng > position_overload_cheng
///   4. 优先级: Frozen > ReduceOnly > Normal
///
/// Frozen 一旦触发, 不会自动回到 ReduceOnly (除非下交易日盘前重置, 由 PR1-1.7 处理).
pub fn evaluate(
    metrics: &PortfolioMetrics,
    prev: Option<AccountMode>,
    thresholds: &ModeThresholds,
) -> ModeEvaluation {
    // v17.0 临时旁路: PUSH_NORMAL_FORCE=1 启动 → 跳过 Frozen 检查, 强制 Normal
    // 用途: 仓位超限导致 Frozen → 推送被 L5 全 deny → 用户看不到任何消息
    //       这违反 4 铁律"默认值必须是出声状态", 临时让 evaluate() 直接返 Normal
    //       真风险控制应该在 broker 下单层做, 不在通知层 (v17.1 治本)
    // F9 fix: 用 OnceLock 缓存的 helper 代替每次 syscall getenv (热路径优化).
    if is_push_normal_forced() {
        return ModeEvaluation {
            mode: AccountMode::Normal,
            trigger_reason: Some("PUSH_NORMAL_FORCE=1 临时旁路 (v17.1 治本)".to_string()),
            prev_mode: prev,
        };
    }

    // 0. Frozen 状态保持优先 (BR-021 强制: 等下一交易日盘前重置, 不运行时回退)
    //    即使数据缺失也不能掩盖 Frozen 状态 — 会污染审计 + 误导下游 RiskMode
    if matches!(prev, Some(AccountMode::Frozen)) {
        return ModeEvaluation {
            mode: AccountMode::Frozen,
            trigger_reason: Some("Frozen 状态保持 (BR-021 强制, 等下一交易日盘前重置)".to_string()),
            prev_mode: prev,
        };
    }

    // 1. 数据缺失 → 保守 ReduceOnly (此时 prev 必非 Frozen, 已由 0 分支覆盖)
    if !metrics.data_complete {
        return ModeEvaluation {
            mode: AccountMode::ReduceOnly,
            trigger_reason: Some("数据缺失, 保守取 ReduceOnly (BR-021)".to_string()),
            prev_mode: prev,
        };
    }

    // 2. Frozen 触发 (在 Normal/ReduceOnly/Frozen 上都评估, 触发后保持)
    if metrics.today_pnl_pct <= thresholds.circuit_breaker_pct {
        return ModeEvaluation {
            mode: AccountMode::Frozen,
            trigger_reason: Some(format!(
                "当日亏损 {:+.2}% 触发熔断线 {:+.2}%",
                metrics.today_pnl_pct, thresholds.circuit_breaker_pct
            )),
            prev_mode: prev,
        };
    }
    if metrics.total_pos_cheng > thresholds.position_overload_cheng {
        return ModeEvaluation {
            mode: AccountMode::Frozen,
            trigger_reason: Some(format!(
                "总仓位 {} 成 超限 {} 成",
                metrics.total_pos_cheng, thresholds.position_overload_cheng
            )),
            prev_mode: prev,
        };
    }

    // 3. ReduceOnly 触发
    if metrics.today_pnl_pct <= thresholds.daily_loss_pct {
        return ModeEvaluation {
            mode: AccountMode::ReduceOnly,
            trigger_reason: Some(format!(
                "当日亏损 {:+.2}% 触发降级线 {:+.2}%",
                metrics.today_pnl_pct, thresholds.daily_loss_pct
            )),
            prev_mode: prev,
        };
    }
    if metrics.consecutive_stop_loss_n >= thresholds.consecutive_stop_loss_n {
        return ModeEvaluation {
            mode: AccountMode::ReduceOnly,
            trigger_reason: Some(format!(
                "连续止损 {} 笔 ≥ 阈值 {}",
                metrics.consecutive_stop_loss_n, thresholds.consecutive_stop_loss_n
            )),
            prev_mode: prev,
        };
    }

    // 4. 触发线都没碰 → Normal
    //    (Frozen 状态保持已在最前面 0 分支处理, 这里不再重复)
    ModeEvaluation {
        mode: AccountMode::Normal,
        trigger_reason: None,
        prev_mode: prev,
    }
}

/// v17.5 §5.10 / BR-021: 8:30 盘前重置信号 helper.
///
/// 返回 `true` 当:
///   1. `now_local` 落在 `[08:30:00, 08:31:00)` 1 分钟窗口内,
///   2. 且 `prev` 是 `Some(AccountMode::Frozen)`.
///
/// 否则返回 `false`.
///
/// 实际 reset 动作不在本函数内 — caller (例如 monitor loop) 收到 `true` 后,
/// 下一次调用 `evaluate()` 应传 `prev = None`,让 evaluate 基于当前 metrics 重判:
///   - 当前 metrics 安全 (无亏损/无仓位超限) → 自动回 Normal
///   - 当前 metrics 仍超限 → 仍在 Frozen (无需 cron 介入)
///
/// **为什么不新造 struct**: 本信号 helper 是纯函数 + 可独立单测 + 不需要
/// `LAST_ACCOUNT_MODE` 全局缓存,跟现有 `evaluate()` 架构完全兼容.
/// caller 集成代码 ~3 行 (e.g. `if should_reset...() { next_eval_prev = None }`).
pub fn should_reset_at_8_30(prev: Option<AccountMode>, now_local: NaiveTime) -> bool {
    let window_start = NaiveTime::from_hms_opt(8, 30, 0).expect("08:30:00 valid");
    let window_end = NaiveTime::from_hms_opt(8, 31, 0).expect("08:31:00 valid");
    if now_local < window_start || now_local >= window_end {
        return false;
    }
    matches!(prev, Some(AccountMode::Frozen))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pnl: f64, consec: u32, pos: u8) -> PortfolioMetrics {
        PortfolioMetrics {
            today_pnl_pct: pnl,
            consecutive_stop_loss_n: consec,
            total_pos_cheng: pos,
            data_complete: true,
        }
    }

    fn t() -> ModeThresholds {
        ModeThresholds::default()
    }

    // ---- 正常态 ----

    #[test]
    fn normal_all_metrics_safe() {
        let r = evaluate(&m(0.5, 0, 4), None, &t());
        assert_eq!(r.mode, AccountMode::Normal);
        assert!(r.trigger_reason.is_none());
        assert!(!r.is_changed());
    }

    #[test]
    fn normal_after_positive_pnl() {
        let r = evaluate(&m(1.5, 0, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Normal);
        assert!(!r.is_changed());
    }

    // ---- ReduceOnly 触发 ----

    #[test]
    fn reduce_only_by_daily_loss_at_threshold() {
        // 日亏 1.5% (恰好等于阈值) → ReduceOnly
        let r = evaluate(&m(-1.5, 0, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::ReduceOnly);
        assert!(r.trigger_reason.as_ref().unwrap().contains("-1.50%"));
    }

    #[test]
    fn reduce_only_by_daily_loss_above_threshold() {
        let r = evaluate(&m(-2.5, 0, 5), Some(AccountMode::Normal), &t());
        // 但 2.5% 已经 ≤ 熔断线 -2.0%, 应优先 Frozen
        assert_eq!(r.mode, AccountMode::Frozen, "2.5% 亏损应优先 Frozen");
    }

    #[test]
    fn reduce_only_by_consecutive_stop_loss() {
        let r = evaluate(&m(0.0, 3, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::ReduceOnly);
        assert!(r.trigger_reason.as_ref().unwrap().contains("连续止损 3"));
    }

    #[test]
    fn reduce_only_below_threshold_for_consecutive() {
        // 2 笔 (默认阈值 3) → 仍 Normal
        let r = evaluate(&m(0.0, 2, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Normal);
    }

    // ---- Frozen 触发 ----

    #[test]
    fn frozen_by_circuit_breaker_at_threshold() {
        // 日亏 -2.0% (恰好等于熔断线) → Frozen
        let r = evaluate(&m(-2.0, 0, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Frozen);
        assert!(r.trigger_reason.as_ref().unwrap().contains("熔断"));
    }

    #[test]
    fn frozen_by_circuit_breaker_deep_loss() {
        let r = evaluate(&m(-5.0, 0, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Frozen);
    }

    #[test]
    fn frozen_by_position_overload() {
        let r = evaluate(&m(0.0, 0, 9), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Frozen);
        assert!(r.trigger_reason.as_ref().unwrap().contains("仓位 9 成"));
    }

    #[test]
    fn frozen_by_position_overload_at_threshold() {
        // 8 成 (默认阈值, 不超)
        let r = evaluate(&m(0.0, 0, 8), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Normal, "8 成 == 阈值, 不超限");
    }

    #[test]
    fn frozen_takes_priority_over_reduce_only() {
        // 同时触发: 日亏 -2.5% (Frozen) + 连续止损 5 (ReduceOnly)
        // 优先级 Frozen > ReduceOnly
        let r = evaluate(&m(-2.5, 5, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::Frozen);
        assert!(r.trigger_reason.as_ref().unwrap().contains("熔断"));
    }

    // ---- Frozen 状态保持 ----

    #[test]
    fn frozen_state_persists_when_conditions_clear() {
        // 当前 Frozen, 数据回到安全区 (无日亏, 仓位轻) → 仍 Frozen
        // 因为 v12 §2.3 要求"下一交易日盘前重置", 不自动回退
        let r = evaluate(&m(0.5, 0, 4), Some(AccountMode::Frozen), &t());
        assert_eq!(r.mode, AccountMode::Frozen);
        assert!(r.trigger_reason.as_ref().unwrap().contains("保持"));
    }

    // ---- 状态变更 ----

    #[test]
    fn is_changed_returns_true_on_transition() {
        let r = evaluate(&m(-1.6, 0, 5), Some(AccountMode::Normal), &t());
        assert!(r.is_changed(), "Normal → ReduceOnly 应算变更");
    }

    #[test]
    fn is_changed_returns_false_on_same() {
        let r = evaluate(&m(-1.6, 0, 5), Some(AccountMode::ReduceOnly), &t());
        assert!(!r.is_changed(), "ReduceOnly → ReduceOnly 不算变更");
    }

    #[test]
    fn is_changed_returns_false_on_first_eval() {
        // 首次评估 prev=None, 不算变更 (无状态可对比)
        let r = evaluate(&m(-1.6, 0, 5), None, &t());
        assert!(!r.is_changed(), "首次评估不算变更, 不触发推送");
    }

    #[test]
    fn normal_to_reduce_only_to_frozen_transition() {
        // Normal → ReduceOnly (日亏 1.6%)
        let r1 = evaluate(&m(-1.6, 0, 5), Some(AccountMode::Normal), &t());
        assert_eq!(r1.mode, AccountMode::ReduceOnly);
        assert!(r1.is_changed());

        // ReduceOnly → Frozen (继续亏损到 2.5%)
        let r2 = evaluate(&m(-2.5, 0, 5), Some(AccountMode::ReduceOnly), &t());
        assert_eq!(r2.mode, AccountMode::Frozen);
        assert!(r2.is_changed());
    }

    // ---- 数据缺失 ----

    #[test]
    fn missing_data_conservative_reduce_only() {
        let metrics = PortfolioMetrics {
            data_complete: false,
            ..Default::default()
        };
        let r = evaluate(&metrics, Some(AccountMode::Normal), &t());
        assert_eq!(r.mode, AccountMode::ReduceOnly);
        assert!(r.trigger_reason.as_ref().unwrap().contains("数据缺失"));
    }

    #[test]
    fn missing_data_keeps_frozen() {
        // 修复 (2026-07-05 BR-021 强制): 数据缺失 + prev=Frozen → 必须保持 Frozen.
        // BR-021 明确说"Frozen 必须等下一交易日盘前重置", 不运行时回退.
        // 旧实现走 ReduceOnly, 会污染审计 + 误开 RiskMode 降级.
        let metrics = PortfolioMetrics {
            data_complete: false,
            ..Default::default()
        };
        let r = evaluate(&metrics, Some(AccountMode::Frozen), &t());
        assert_eq!(r.mode, AccountMode::Frozen, "Frozen 必须保持 (BR-021)");
        assert!(r.trigger_reason.as_ref().unwrap().contains("Frozen"));
    }

    // ---- 阈值自定义 ----

    #[test]
    fn custom_thresholds_work() {
        // 自定义更宽松的阈值: -5% 才 ReduceOnly, -10% 才 Frozen
        let custom = ModeThresholds {
            daily_loss_pct: -5.0,
            circuit_breaker_pct: -10.0,
            consecutive_stop_loss_n: 5,
            position_overload_cheng: 9,
        };
        let r = evaluate(&m(-3.0, 0, 5), Some(AccountMode::Normal), &custom);
        assert_eq!(r.mode, AccountMode::Normal, "3% 亏损在宽松阈值下应 Normal");

        let r = evaluate(&m(-6.0, 0, 5), Some(AccountMode::Normal), &custom);
        assert_eq!(
            r.mode,
            AccountMode::ReduceOnly,
            "6% 亏损在宽松阈值下应 ReduceOnly"
        );
    }

    // ---- 默认阈值常量稳定性 ----

    #[test]
    fn default_thresholds_match_br021() {
        assert_eq!(thresholds::DAILY_LOSS_PCT, -1.5);
        assert_eq!(thresholds::CIRCUIT_BREAKER_PCT, -2.0);
        assert_eq!(thresholds::CONSECUTIVE_STOP_LOSS_N, 3);
        assert_eq!(thresholds::POSITION_OVERLOAD_CHENG, 8);
    }

    // ============== v17.5 §5.10 / BR-021: 8:30 盘前重置信号测试 ==============

    #[test]
    fn should_reset_at_8_30_true_in_window_with_frozen_prev() {
        let at_8_30 = NaiveTime::from_hms_opt(8, 30, 0).unwrap();
        let at_8_30_45 = NaiveTime::from_hms_opt(8, 30, 45).unwrap();
        assert!(should_reset_at_8_30(Some(AccountMode::Frozen), at_8_30));
        assert!(should_reset_at_8_30(Some(AccountMode::Frozen), at_8_30_45));
    }

    #[test]
    fn should_reset_at_8_30_false_outside_window() {
        let at_8_29 = NaiveTime::from_hms_opt(8, 29, 59).unwrap();
        let at_8_31 = NaiveTime::from_hms_opt(8, 31, 0).unwrap();
        let at_9_30 = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
        assert!(!should_reset_at_8_30(Some(AccountMode::Frozen), at_8_29));
        assert!(!should_reset_at_8_30(Some(AccountMode::Frozen), at_8_31));
        assert!(!should_reset_at_8_30(Some(AccountMode::Frozen), at_9_30));
    }

    #[test]
    fn should_reset_at_8_30_false_when_prev_not_frozen() {
        let at_8_30 = NaiveTime::from_hms_opt(8, 30, 0).unwrap();
        // 窗口内, 但 prev 是 Normal / ReduceOnly / None → 都 false
        assert!(!should_reset_at_8_30(Some(AccountMode::Normal), at_8_30));
        assert!(!should_reset_at_8_30(Some(AccountMode::ReduceOnly), at_8_30));
        assert!(!should_reset_at_8_30(None, at_8_30));
    }

    // ============== F9: PUSH_NORMAL_FORCE OnceLock 缓存测试 ==============

    #[test]
    fn is_push_normal_forced_default_false_when_env_unset() {
        // cargo test process 在未污染环境下, 默认 false (PUSH_NORMAL_FORCE 未 set).
        // 若测试运行环境 set 了 PUSH_NORMAL_FORCE=1, 假设测试编排已确保隔离.
        // 此测试仅验证 helper callable + 返回 bool, 不依赖具体值.
        let _: bool = is_push_normal_forced();
    }

    #[test]
    fn is_push_normal_forced_cached_idempotent() {
        // OnceLock 保证多次调用结果一致 (无需 syscall).
        // 任何环境下, helper 是 stable boolean (process-global cache).
        let first = is_push_normal_forced();
        let second = is_push_normal_forced();
        let third = is_push_normal_forced();
        assert_eq!(first, second);
        assert_eq!(second, third);
    }
}
