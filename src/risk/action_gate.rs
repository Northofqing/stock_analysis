//! v12 PR1 动作门 (ActionGate) — 6 动作 × 3 模式权限矩阵.
//!
//! 设计: 纯函数表驱动, 不动 `veto_chain`. ActionGate 是新建的窄接缝,
//!       与现有 VetoRule/VetoChain 并存 (v12.2 §2.4 决策).
//!
//! 调用约定:
//!   1. 决策侧 (holding_plan / t0_advisor / live_plan) 在产出建议前调 `authorize`
//!   2. 返回 `GateResult::Allow` → 继续走推送路径
//!   3. 返回 `GateResult::Deny(reason)` → 建议降级 (不推送, 不写库) 或转 T-09 禁止操作推送
//!
//! 边界:
//!   - 6 动作 × 3 模式 = 18 格, 单测覆盖每格 (BR-022)
//!   - 冻结态反T被 Deny (v12.2 §2.3 显式要求, 单测专项覆盖)

/// v12 §2.3 交易动作枚举
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ActionKind {
    /// 建新仓
    OpenNew,
    /// 加仓已有持仓
    Add,
    /// 减仓
    Reduce,
    /// 清仓
    Clear,
    /// 持有观望 (PR4-4.2 live_plan 加)
    Hold,
    /// 正T (先卖后买, 等于加仓)
    T0Positive,
    /// 反T (先买后卖, 等于减仓)
    T0Reverse,
}

impl ActionKind {
    /// 中文标签 (供日志/推送渲染)
    pub fn label(self) -> &'static str {
        match self {
            ActionKind::OpenNew => "开新仓",
            ActionKind::Add => "加仓",
            ActionKind::Reduce => "减仓",
            ActionKind::Clear => "清仓",
            ActionKind::Hold => "持有观望",
            ActionKind::T0Positive => "正T",
            ActionKind::T0Reverse => "反T",
        }
    }

    /// 全部 7 变体 (单测遍历用)
    pub const ALL: [ActionKind; 7] = [
        ActionKind::OpenNew,
        ActionKind::Add,
        ActionKind::Reduce,
        ActionKind::Clear,
        ActionKind::Hold,
        ActionKind::T0Positive,
        ActionKind::T0Reverse,
    ];
}

/// 账户模式 — 与 push_templates::AccountMode 等价.
///
/// 这里重新定义 enum (而不是直接 import push_templates), 因为:
/// 1. action_gate 是 risk 库内的纯函数模块, 不应反向依赖 bin/monitor
/// 2. push_templates 本身依赖 super::notify, 跨 bin/lib 边界
/// 3. PR1 接入时由 PR1-1.6 在 boundary 处做 `From` 转换
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AccountMode {
    Normal,
    ReduceOnly,
    Frozen,
}

impl AccountMode {
    pub const ALL: [AccountMode; 3] = [
        AccountMode::Normal,
        AccountMode::ReduceOnly,
        AccountMode::Frozen,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AccountMode::Normal => "Normal",
            AccountMode::ReduceOnly => "ReduceOnly",
            AccountMode::Frozen => "Frozen",
        }
    }
}

/// 授权结果
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GateResult {
    /// 允许执行
    Allow,
    /// 拒绝执行 + 原因
    Deny(&'static str),
}

impl GateResult {
    pub fn is_allow(&self) -> bool {
        matches!(self, GateResult::Allow)
    }

    pub fn blocked_reason(&self) -> Option<&'static str> {
        match self {
            GateResult::Allow => None,
            GateResult::Deny(r) => Some(r),
        }
    }
}

/// 2026-09-22 用户决策: **完全解除**账户模式对交易动作的 gate。
///
/// 背景: 模拟盘实验被实盘语义的熔断器自锁 (连续止损 4 笔 → ReduceOnly →
/// 禁买入 → 无新卖出 → 计数永不清零 → 永续 ReduceOnly)。账户模式保留为
/// 纯状态展示 (banner / T-01 卡 / L5 Frozen 推送拦截不受影响)。
///
/// 原 BR-022 权限矩阵 (保留供未来接真实券商时恢复):
///
/// | Action       | Normal | ReduceOnly                | Frozen |
/// |--------------|--------|---------------------------|--------|
/// | OpenNew      | Allow  | Deny(禁建仓)              | Deny   |
/// | Add          | Allow  | Deny(禁加仓)              | Deny   |
/// | Reduce       | Allow  | Allow                     | Deny   |
/// | Clear        | Allow  | Allow                     | Deny   |
/// | T0Positive   | Allow  | Deny(只允许减仓)          | Deny   |
/// | T0Reverse    | Allow  | Allow                     | Deny   |
pub fn authorize(_action: ActionKind, _mode: AccountMode) -> GateResult {
    GateResult::Allow
}

/// 便捷: 批量检查一个 mode 下所有 7 个 action (PR4-4.2 加 Hold)
///
/// 供监控/审计/单测一次性确认 7 格结果.
pub fn authorize_all(mode: AccountMode) -> [(ActionKind, GateResult); 7] {
    let mut out = [(ActionKind::OpenNew, GateResult::Allow); 7];
    for (i, a) in ActionKind::ALL.iter().enumerate() {
        out[i] = (*a, authorize(*a, mode));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 18 格权限矩阵表驱动单测 — 2026-09-22 用户决策后全格 Allow (完全解除),
    /// 矩阵保留以锁定「模式不再 gate 动作」的语义 (恢复原矩阵见 authorize 注释).
    #[test]
    fn matrix_all_18_cells_allow_after_20260922_decision() {
        for action in ActionKind::ALL {
            for mode in AccountMode::ALL {
                let r = authorize(action, mode);
                assert!(
                    r.is_allow(),
                    "2026-09-22 决策: {:?} × {:?} 应 Allow (账户模式不再 gate 动作)",
                    action,
                    mode
                );
            }
        }
    }

    /// 专项: 2026-09-22 后 Frozen 不再 Deny 反T (原 v12.2 §2.3 要求已随
    /// 完全解除决策废止; 原语义见 authorize 注释矩阵).
    #[test]
    fn frozen_no_longer_blocks_reverse_t() {
        let r = authorize(ActionKind::T0Reverse, AccountMode::Frozen);
        assert!(
            r.is_allow(),
            "2026-09-22 决策: Frozen 下反T 也应 Allow (模式不再 gate 动作)"
        );
    }

    /// 专项: ReduceOnly 反T放行 (v12.2 §2.3 显式要求)
    #[test]
    fn reduce_only_allows_reverse_t() {
        let r = authorize(ActionKind::T0Reverse, AccountMode::ReduceOnly);
        assert!(r.is_allow(), "ReduceOnly 下反T必须 Allow (接回底仓)");
    }

    /// 专项: 2026-09-22 后 ReduceOnly 不再 Deny 正T (完全解除决策).
    #[test]
    fn reduce_only_no_longer_blocks_positive_t() {
        let r = authorize(ActionKind::T0Positive, AccountMode::ReduceOnly);
        assert!(
            r.is_allow(),
            "2026-09-22 决策: ReduceOnly 下正T 也应 Allow (模式不再 gate 动作)"
        );
    }

    /// GateResult API 行为
    #[test]
    fn gate_result_api() {
        let allow = GateResult::Allow;
        assert!(allow.is_allow());
        assert!(allow.blocked_reason().is_none());

        let deny = GateResult::Deny("test reason");
        assert!(!deny.is_allow());
        assert_eq!(deny.blocked_reason(), Some("test reason"));
    }

    /// authorize_all 7 格齐全 (PR4-4.2 加 Hold)
    #[test]
    fn authorize_all_returns_7() {
        let result = authorize_all(AccountMode::Normal);
        assert_eq!(result.len(), 7);
        for (a, r) in result.iter() {
            assert!(r.is_allow(), "Normal 下 {:?} 应 Allow", a);
        }

        // 2026-09-22 决策: 任意模式下全 Allow (完全解除).
        let result_frozen = authorize_all(AccountMode::Frozen);
        for (a, r) in result_frozen.iter() {
            assert!(
                r.is_allow(),
                "2026-09-22 决策: Frozen 下 {:?} 也应 Allow",
                a
            );
        }
    }

    /// 标签稳定 (供日志/推送渲染)
    #[test]
    fn action_kind_labels() {
        assert_eq!(ActionKind::OpenNew.label(), "开新仓");
        assert_eq!(ActionKind::Add.label(), "加仓");
        assert_eq!(ActionKind::Reduce.label(), "减仓");
        assert_eq!(ActionKind::Clear.label(), "清仓");
        assert_eq!(ActionKind::T0Positive.label(), "正T");
        assert_eq!(ActionKind::T0Reverse.label(), "反T");
    }

    #[test]
    fn account_mode_labels() {
        assert_eq!(AccountMode::Normal.label(), "Normal");
        assert_eq!(AccountMode::ReduceOnly.label(), "ReduceOnly");
        assert_eq!(AccountMode::Frozen.label(), "Frozen");
    }

    /// ActionKind::ALL 7 个且无重复 (PR4-4.2 加 Hold)
    #[test]
    fn action_kind_all_unique() {
        let mut seen = std::collections::HashSet::new();
        for a in ActionKind::ALL.iter() {
            assert!(seen.insert(a), "{:?} 重复", a);
        }
        assert_eq!(ActionKind::ALL.len(), 7);
    }

    /// AccountMode::ALL 3 个且无重复
    #[test]
    fn account_mode_all_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in AccountMode::ALL.iter() {
            assert!(seen.insert(m), "{:?} 重复", m);
        }
        assert_eq!(AccountMode::ALL.len(), 3);
    }
}
