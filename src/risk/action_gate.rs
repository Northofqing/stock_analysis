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

/// BR-022 权限矩阵: 6 动作 × 3 模式 = 18 格
///
/// | Action       | Normal | ReduceOnly                | Frozen |
/// |--------------|--------|---------------------------|--------|
/// | OpenNew      | Allow  | Deny(禁建仓)              | Deny   |
/// | Add          | Allow  | Deny(禁加仓)              | Deny   |
/// | Reduce       | Allow  | Allow                     | Deny   |
/// | Clear        | Allow  | Allow                     | Deny   |
/// | T0Positive   | Allow  | Deny(只允许减仓)          | Deny   |
/// | T0Reverse    | Allow  | Allow                     | Deny   |
pub fn authorize(action: ActionKind, mode: AccountMode) -> GateResult {
    use AccountMode::*;
    use ActionKind::*;

    match (action, mode) {
        // ============ Normal: 全 Allow ============
        (OpenNew, Normal)
        | (Add, Normal)
        | (Reduce, Normal)
        | (Clear, Normal)
        | (Hold, Normal)
        | (T0Positive, Normal)
        | (T0Reverse, Normal) => GateResult::Allow,

        // ============ ReduceOnly ============
        (OpenNew, ReduceOnly) => GateResult::Deny("账户降级 ReduceOnly, 禁止开新仓"),
        (Add, ReduceOnly) => GateResult::Deny("账户降级 ReduceOnly, 禁止加仓"),
        (T0Positive, ReduceOnly) => {
            GateResult::Deny("账户降级 ReduceOnly, 只允许减仓, 不允许做T加仓")
        }
        (Reduce, ReduceOnly) => GateResult::Allow,
        (Clear, ReduceOnly) => GateResult::Allow,
        (Hold, ReduceOnly) => GateResult::Allow, // 持有观望永远允许
        (T0Reverse, ReduceOnly) => GateResult::Allow, // 反T接回底仓允许 (v12.2 §2.3 显式)

        // ============ Frozen: 全 Deny (含反T) ============
        (_, Frozen) => GateResult::Deny("账户熔断 Frozen, 禁止任何新动作"),
    }
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

    /// 18 格权限矩阵表驱动单测 — 必须与 v12 §2.3 矩阵逐格一致
    #[test]
    fn matrix_all_18_cells() {
        // 期望矩阵: Normal 全 Allow, ReduceOnly 大部分 Deny (除减仓类), Frozen 全 Deny
        struct Expectation {
            action: ActionKind,
            mode: AccountMode,
            allow: bool,
            reason_contains: Option<&'static str>,
        }
        let matrix = [
            // Normal (6 格全 Allow)
            Expectation {
                action: ActionKind::OpenNew,
                mode: AccountMode::Normal,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::Add,
                mode: AccountMode::Normal,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::Reduce,
                mode: AccountMode::Normal,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::Clear,
                mode: AccountMode::Normal,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::T0Positive,
                mode: AccountMode::Normal,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::T0Reverse,
                mode: AccountMode::Normal,
                allow: true,
                reason_contains: None,
            },
            // ReduceOnly (3 Deny + 3 Allow)
            Expectation {
                action: ActionKind::OpenNew,
                mode: AccountMode::ReduceOnly,
                allow: false,
                reason_contains: Some("开新仓"),
            },
            Expectation {
                action: ActionKind::Add,
                mode: AccountMode::ReduceOnly,
                allow: false,
                reason_contains: Some("加仓"),
            },
            Expectation {
                action: ActionKind::Reduce,
                mode: AccountMode::ReduceOnly,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::Clear,
                mode: AccountMode::ReduceOnly,
                allow: true,
                reason_contains: None,
            },
            Expectation {
                action: ActionKind::T0Positive,
                mode: AccountMode::ReduceOnly,
                allow: false,
                reason_contains: Some("只允许减仓"),
            },
            Expectation {
                action: ActionKind::T0Reverse,
                mode: AccountMode::ReduceOnly,
                allow: true,
                reason_contains: None,
            },
            // Frozen (6 格全 Deny)
            Expectation {
                action: ActionKind::OpenNew,
                mode: AccountMode::Frozen,
                allow: false,
                reason_contains: Some("熔断"),
            },
            Expectation {
                action: ActionKind::Add,
                mode: AccountMode::Frozen,
                allow: false,
                reason_contains: Some("熔断"),
            },
            Expectation {
                action: ActionKind::Reduce,
                mode: AccountMode::Frozen,
                allow: false,
                reason_contains: Some("熔断"),
            },
            Expectation {
                action: ActionKind::Clear,
                mode: AccountMode::Frozen,
                allow: false,
                reason_contains: Some("熔断"),
            },
            Expectation {
                action: ActionKind::T0Positive,
                mode: AccountMode::Frozen,
                allow: false,
                reason_contains: Some("熔断"),
            },
            Expectation {
                action: ActionKind::T0Reverse,
                mode: AccountMode::Frozen,
                allow: false,
                reason_contains: Some("熔断"),
            },
        ];
        assert_eq!(matrix.len(), 18, "18 格矩阵");
        for e in &matrix {
            let r = authorize(e.action, e.mode);
            assert_eq!(
                r.is_allow(),
                e.allow,
                "矩阵不一致: {:?} × {:?} 期望 {} 实得 {}",
                e.action,
                e.mode,
                if e.allow { "Allow" } else { "Deny" },
                if r.is_allow() { "Allow" } else { "Deny" }
            );
            if let Some(expected) = e.reason_contains {
                let reason = r.blocked_reason().expect("Deny 必有 reason");
                assert!(
                    reason.contains(expected),
                    "{:?} × {:?} reason 不含 '{}', 实得 '{}'",
                    e.action,
                    e.mode,
                    expected,
                    reason
                );
            }
        }
    }

    /// 专项: Frozen 下反T被 Deny (v12.2 §2.3 显式要求)
    #[test]
    fn frozen_blocks_reverse_t() {
        let r = authorize(ActionKind::T0Reverse, AccountMode::Frozen);
        assert!(
            !r.is_allow(),
            "Frozen 下反T必须 Deny (与 Normal/ReduceOnly 反T 行为区分)"
        );
        assert!(r.blocked_reason().unwrap().contains("熔断"));
    }

    /// 专项: ReduceOnly 反T放行 (v12.2 §2.3 显式要求)
    #[test]
    fn reduce_only_allows_reverse_t() {
        let r = authorize(ActionKind::T0Reverse, AccountMode::ReduceOnly);
        assert!(r.is_allow(), "ReduceOnly 下反T必须 Allow (接回底仓)");
    }

    /// 专项: ReduceOnly 正T被 Deny (避免做T加仓扩大敞口)
    #[test]
    fn reduce_only_blocks_positive_t() {
        let r = authorize(ActionKind::T0Positive, AccountMode::ReduceOnly);
        assert!(
            !r.is_allow(),
            "ReduceOnly 下正T必须 Deny (做T加仓违反降级精神)"
        );
        assert!(r.blocked_reason().unwrap().contains("只允许减仓"));
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

        let result_frozen = authorize_all(AccountMode::Frozen);
        for (a, r) in result_frozen.iter() {
            assert!(!r.is_allow(), "Frozen 下 {:?} 应 Deny", a);
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
