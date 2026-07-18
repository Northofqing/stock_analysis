//! v12 PR4-4.2: live_plan 两级渲染 (Advice / ExecutablePlan).
//!
//! 设计: 三重校验 (PR4 硬性要求):
//!   1. ActionGate 通过 (action 不是 Deny)
//!   2. DataMode ≠ Unsafe (缺 Quote 不能出价格型建议)
//!   3. available_shares(code) > 0 (无股可卖 → 不出 Reduce/Clear 建议)
//!
//! 通过三重 → ExecutablePlan (可直接执行)
//! 任一失败 → Advice (仅提示, 需人工确认)
//!
//! v12 §2.3 + BR-022 衍生.

use crate::risk::action_gate::{authorize, AccountMode, ActionKind, GateResult};

/// 数据模式 (复用 push_templates::DataMode, 简化为本地枚举避免跨 crate 依赖)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DataMode {
    Full,
    Degraded,
    Unsafe,
}

impl DataMode {
    pub fn is_unsafe(self) -> bool {
        matches!(self, DataMode::Unsafe)
    }
}

/// 输入: live_plan 评估所需的指标
#[derive(Clone, Debug)]
pub struct LivePlanInput {
    pub code: String,
    pub name: String,
    pub action: ActionKind,
    pub account_mode: AccountMode,
    pub data_mode: DataMode,
    pub available_shares: u32,
    pub price: f64,
    pub cost: f64,
}

/// 输出 (一级): Advice (默认, 无三重校验)
#[derive(Clone, Debug)]
pub struct Advice {
    pub code: String,
    pub name: String,
    pub action: ActionKind,
    pub rationale: String,
    /// 降级原因 (任一校验失败时填)
    pub downgrade_reason: Option<String>,
}

/// 输出 (二级): ExecutablePlan (三重校验全过)
#[derive(Clone, Debug)]
pub struct ExecutablePlan {
    pub code: String,
    pub name: String,
    pub action: ActionKind,
    pub price: f64,
    pub cost: f64,
    pub pnl_pct: f64,
    pub rationale: String,
}

/// PR4-4.2 主评估: 返回两级结果
pub fn evaluate(input: &LivePlanInput) -> LivePlanResult {
    // 校验 1: ActionGate
    let gate = authorize(input.action, input.account_mode);
    let gate_ok = gate.is_allow();

    // 校验 2: DataMode ≠ Unsafe
    let dm_ok = !input.data_mode.is_unsafe();

    // 校验 3: available_shares > 0 (仅对 Reduce/Clear 类动作强制)
    let needs_shares = matches!(
        input.action,
        ActionKind::Reduce | ActionKind::Clear | ActionKind::T0Positive | ActionKind::T0Reverse
    );
    let shares_ok = !needs_shares || input.available_shares > 0;

    let all_ok = gate_ok && dm_ok && shares_ok;

    let rationale = build_rationale(input, gate_ok, dm_ok, shares_ok);

    if all_ok {
        let pnl_pct = if input.cost > 0.0 {
            (input.price / input.cost - 1.0) * 100.0
        } else {
            0.0
        };
        LivePlanResult::Executable(ExecutablePlan {
            code: input.code.clone(),
            name: input.name.clone(),
            action: input.action,
            price: input.price,
            cost: input.cost,
            pnl_pct,
            rationale,
        })
    } else {
        let reason =
            build_downgrade_reason(gate, input.data_mode, input.available_shares, needs_shares);
        LivePlanResult::Advice(Advice {
            code: input.code.clone(),
            name: input.name.clone(),
            action: input.action,
            rationale,
            downgrade_reason: Some(reason),
        })
    }
}

fn build_rationale(input: &LivePlanInput, gate_ok: bool, dm_ok: bool, shares_ok: bool) -> String {
    let mut parts = Vec::new();
    parts.push(format!("动作: {}", input.action.label()));
    if !gate_ok {
        parts.push("ActionGate 否决".to_string());
    }
    if !dm_ok {
        parts.push("数据 Unsafe".to_string());
    }
    if !shares_ok {
        parts.push(format!("可用 {} 股 = 0", input.available_shares));
    }
    if parts.is_empty() {
        format!("三重校验通过: {}", input.action.label())
    } else {
        parts.join(" / ")
    }
}

fn build_downgrade_reason(
    gate: GateResult,
    data_mode: DataMode,
    available_shares: u32,
    needs_shares: bool,
) -> String {
    let mut reasons = Vec::new();
    if let GateResult::Deny(r) = gate {
        reasons.push(format!("Gate: {}", r));
    }
    if data_mode.is_unsafe() {
        reasons.push("DataMode: Unsafe".to_string());
    }
    if needs_shares && available_shares == 0 {
        reasons.push(format!("无股可卖 ({} 股)", available_shares));
    }
    reasons.join("; ")
}

/// 输出联合
#[derive(Clone, Debug)]
pub enum LivePlanResult {
    Advice(Advice),
    Executable(ExecutablePlan),
}

impl LivePlanResult {
    pub fn is_executable(&self) -> bool {
        matches!(self, LivePlanResult::Executable(_))
    }

    pub fn code(&self) -> &str {
        match self {
            LivePlanResult::Advice(a) => &a.code,
            LivePlanResult::Executable(e) => &e.code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_normal() -> LivePlanInput {
        LivePlanInput {
            code: "TEST_CODE_000001".to_string(),
            name: "测试".to_string(),
            action: ActionKind::Hold,
            account_mode: AccountMode::Normal,
            data_mode: DataMode::Full,
            available_shares: 1000,
            price: 12.0,
            cost: 11.0,
        }
    }

    // ---- 三重校验通过 → ExecutablePlan ----

    #[test]
    fn all_checks_pass_returns_executable() {
        let r = evaluate(&input_normal());
        assert!(r.is_executable(), "三重校验全过应返回 ExecutablePlan");
    }

    #[test]
    fn executable_has_pnl() {
        let r = evaluate(&input_normal());
        if let LivePlanResult::Executable(e) = r {
            // pnl = (12/11 - 1) * 100 ≈ 9.09
            assert!(
                (e.pnl_pct - 9.09).abs() < 0.5,
                "pnl_pct 计算错误: {}",
                e.pnl_pct
            );
        } else {
            panic!("应返回 ExecutablePlan");
        }
    }

    // ---- 校验 1: ActionGate 失败 → Advice ----

    #[test]
    fn frozen_mode_blocks_executable() {
        let mut inp = input_normal();
        inp.account_mode = AccountMode::Frozen;
        inp.action = ActionKind::OpenNew;
        let r = evaluate(&inp);
        assert!(!r.is_executable());
        if let LivePlanResult::Advice(a) = r {
            assert!(a.downgrade_reason.unwrap().contains("Gate"));
        }
    }

    #[test]
    fn reduce_only_blocks_open_new() {
        let mut inp = input_normal();
        inp.account_mode = AccountMode::ReduceOnly;
        inp.action = ActionKind::OpenNew;
        let r = evaluate(&inp);
        assert!(!r.is_executable());
    }

    #[test]
    fn reduce_only_allows_reduce() {
        let mut inp = input_normal();
        inp.account_mode = AccountMode::ReduceOnly;
        inp.action = ActionKind::Reduce;
        let r = evaluate(&inp);
        assert!(r.is_executable(), "ReduceOnly 应允许减仓");
    }

    // ---- 校验 2: DataMode Unsafe → Advice ----

    #[test]
    fn unsafe_data_blocks_executable() {
        let mut inp = input_normal();
        inp.data_mode = DataMode::Unsafe;
        let r = evaluate(&inp);
        assert!(!r.is_executable());
        if let LivePlanResult::Advice(a) = r {
            assert!(a.downgrade_reason.unwrap().contains("Unsafe"));
        }
    }

    #[test]
    fn degraded_data_allows_executable() {
        // Degraded 不阻断 (只有 Unsafe 阻断)
        let mut inp = input_normal();
        inp.data_mode = DataMode::Degraded;
        let r = evaluate(&inp);
        assert!(r.is_executable(), "Degraded 不应阻断");
    }

    // ---- 校验 3: available_shares = 0 阻断减仓类 ----

    #[test]
    fn zero_shares_blocks_reduce() {
        let mut inp = input_normal();
        inp.available_shares = 0;
        inp.action = ActionKind::Reduce;
        let r = evaluate(&inp);
        assert!(!r.is_executable(), "无股可卖应阻断 Reduce");
        if let LivePlanResult::Advice(a) = r {
            assert!(a.downgrade_reason.unwrap().contains("无股可卖"));
        }
    }

    #[test]
    fn zero_shares_does_not_block_add() {
        // Add 不需要 shares (本来就是建仓)
        let mut inp = input_normal();
        inp.available_shares = 0;
        inp.action = ActionKind::Add;
        let r = evaluate(&inp);
        assert!(r.is_executable(), "Add 不受 available_shares 约束");
    }

    // ---- 组合 ----

    #[test]
    fn multiple_failures_concatenate() {
        let mut inp = input_normal();
        inp.account_mode = AccountMode::Frozen;
        inp.data_mode = DataMode::Unsafe;
        inp.available_shares = 0;
        inp.action = ActionKind::Reduce;
        let r = evaluate(&inp);
        assert!(!r.is_executable());
        if let LivePlanResult::Advice(a) = r {
            let reason = a.downgrade_reason.unwrap();
            assert!(reason.contains("Gate"));
            assert!(reason.contains("Unsafe"));
            assert!(reason.contains("无股可卖"));
        }
    }

    #[test]
    fn code_passthrough() {
        let r = evaluate(&input_normal());
        assert_eq!(r.code(), "TEST_CODE_000001");
    }
}
