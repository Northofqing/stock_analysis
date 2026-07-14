//! v16.3 Commit 1 — Paper trade pre-trade gate.
//!
//! 4 项硬检查 (v15.1.1 默认出声 + v16.3 R1/R2 业务核心):
//!   1. AccountMode 行动授权 — ReduceOnly 禁开仓, Frozen 全禁
//!   2. 单票仓位硬线 — 不超 MAX_POSITION_PCT% (默认 10%)
//!   3. 现金底 — 不低于 CASH_FLOOR_PCT% (默认 15%)
//!   4. DataMode — Degraded 禁开仓, Unsafe 全禁
//!
//! 任何失败 → log::warn + 返回 Err (不入 paper_trades, 也不调 simulate)
//! 默认值出声 (v15.1.1 硬规则 1): 启动时 banner 打印当前 mode
//!
//! Commit 1 仅接轻量签名: `cash + total_value + current_position_pct`
//! (不强求 PaperPosition struct, v16.4 完整 position 接入再扩)

use crate::risk::action_gate::{authorize, AccountMode, ActionKind, GateResult};
use crate::risk::cash_guard::{check_cash, CashGuard};
use crate::trading::paper_trade::{Direction, PaperSignal};
use std::sync::OnceLock;

/// Fix 6: 读 env 覆盖 (v15.1.1 硬规则 1: 默认出声 + env 显式覆盖)
/// 默认值常量化 (编译期 fallback), 运行时读 env 覆盖.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// 最大滑点 (%), 超过则 evaluate 返回 Invalidated
/// 默认 2.0, env 覆盖: `PAPER_MAX_SLIPPAGE`
pub const MAX_SLIPPAGE_PCT_DEFAULT: f64 = 2.0;
pub static MAX_SLIPPAGE_PCT: std::sync::LazyLock<f64> =
    std::sync::LazyLock::new(|| env_or("PAPER_MAX_SLIPPAGE", MAX_SLIPPAGE_PCT_DEFAULT));

/// 最大单票仓位 (%)
pub const MAX_POSITION_PCT_DEFAULT: f64 = 10.0;
pub static MAX_POSITION_PCT: std::sync::LazyLock<f64> =
    std::sync::LazyLock::new(|| env_or("PAPER_MAX_POSITION_PCT", MAX_POSITION_PCT_DEFAULT));

/// 现金底 (%)
pub const CASH_FLOOR_PCT_DEFAULT: f64 = 15.0;
pub static CASH_FLOOR_PCT: std::sync::LazyLock<f64> =
    std::sync::LazyLock::new(|| env_or("PAPER_CASH_FLOOR_PCT", CASH_FLOOR_PCT_DEFAULT));

/// 启动 banner 是否已打印 (OnceLock, 避免重复)
static BANNER_PRINTED: OnceLock<bool> = OnceLock::new();

/// 启动时打印 v16.3 默认值 (v15.1.1 硬规则 1: 默认出声)
pub fn print_startup_banner() {
    if BANNER_PRINTED.set(true).is_ok() {
        log::info!(
            "[v16.3 paper_trade] 默认值: max_slippage={}%, max_position={}%, cash_floor={}% (env: PAPER_MAX_SLIPPAGE/PAPER_MAX_POSITION_PCT/PAPER_CASH_FLOOR_PCT 覆盖)",
            *MAX_SLIPPAGE_PCT, *MAX_POSITION_PCT, *CASH_FLOOR_PCT
        );
    }
}

/// Pre-trade gate — 4 项硬检查
///
/// # Arguments
/// - `signal`: paper_trade::PaperSignal (含 direction / data_mode)
/// - `quote_price`: 实际成交价 (本 fn 不用 — 滑点在 evaluate 里)
/// - `current_cash`: 当前现金
/// - `total_value`: 当前总资产 (现金 + 持仓市值)
/// - `current_position_pct`: 当前单票已占仓位 (%)
pub fn pre_trade_check(
    signal: &PaperSignal,
    _quote_price: f64,
    current_cash: f64,
    total_value: f64,
    current_position_pct: f64,
) -> Result<(), String> {
    // 1. AccountMode 行动授权
    let mode = parse_account_mode(&signal.account_mode);
    let action = match signal.direction {
        Direction::Buy => ActionKind::OpenNew,
        Direction::Sell => ActionKind::Reduce,
    };
    match authorize(action, mode) {
        GateResult::Allow => {}
        GateResult::Deny(reason) => {
            log::warn!(
                "[risk_adapter] 拒 {}({}): account_mode={} action={} 原因={}",
                signal.name, signal.code, signal.account_mode, action.label(), reason
            );
            return Err(format!(
                "account_mode {} 拒 {}: {}",
                signal.account_mode,
                action.label(),
                reason
            ));
        }
    }

    // 2. 单票仓位硬线 (仅 Buy 触发)
    if signal.direction == Direction::Buy && current_position_pct > *MAX_POSITION_PCT {
        log::warn!(
            "[risk_adapter] 拒 {}({}): 单票仓位 {:.1}% > 限 {}%",
            signal.name, signal.code, current_position_pct, *MAX_POSITION_PCT
        );
        return Err(format!(
            "单票仓位 {:.1}% 超限 {}%",
            current_position_pct, *MAX_POSITION_PCT
        ));
    }

    // 3. 现金底
    let guard = CashGuard { floor_pct: *CASH_FLOOR_PCT };
    if let Some(alert) = check_cash(current_cash, total_value, &guard) {
        if alert.below_floor {
            log::warn!(
                "[risk_adapter] 拒 {}({}): 现金占比 {:.1}% < 底 {}%",
                signal.name, signal.code, alert.cash_pct, *CASH_FLOOR_PCT
            );
            return Err(format!(
                "现金占比 {:.1}% 不足底限 {}%",
                alert.cash_pct, *CASH_FLOOR_PCT
            ));
        }
    }

    // 4. DataMode
    match signal.data_mode.as_str() {
        "Full" => {}
        "Degraded" if signal.direction == Direction::Buy => {
            log::warn!(
                "[risk_adapter] 拒 {}({}): data_mode=Degraded 禁开仓",
                signal.name, signal.code
            );
            return Err("data_mode=Degraded 禁开仓".to_string());
        }
        "Degraded" => {} // 允许减仓
        "Unsafe" => {
            log::warn!(
                "[risk_adapter] 拒 {}({}): data_mode=Unsafe 拒所有交易",
                signal.name, signal.code
            );
            return Err("data_mode=Unsafe 拒所有交易".to_string());
        }
        other => {
            log::warn!(
                "[risk_adapter] 拒 {}({}): 未知 data_mode '{}'",
                signal.name, signal.code, other
            );
            return Err(format!("未知 data_mode '{}'", other));
        }
    }

    Ok(())
}

/// 解析 account_mode 字符串 → AccountMode enum
/// 不依赖 push_templates::AccountMode (跨 bin/lib 边界), 与 action_gate 同源
fn parse_account_mode(s: &str) -> AccountMode {
    match s {
        "ReduceOnly" => AccountMode::ReduceOnly,
        "Frozen" => AccountMode::Frozen,
        _ => AccountMode::Normal,
    }
}

// ============ Unit tests (≥ 12, 含边界 case) ============

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(account_mode: &str, data_mode: &str, direction: Direction) -> PaperSignal {
        PaperSignal {
            plan_id: "plan-test-001".to_string(),
            code: "688001".to_string(),
            name: "测试".to_string(),
            direction,
            price: 50.0,
            quantity: 100,
            virtual_reason: "NewsCatalyst".to_string(),
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            account_mode: account_mode.to_string(),
            data_mode: data_mode.to_string(),
        }
    }

    // ---- 1. AccountMode 拦截 ----

    #[test]
    fn rejects_buy_when_reduceonly() {
        let s = signal("ReduceOnly", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("ReduceOnly"));
    }

    #[test]
    fn allows_sell_when_reduceonly() {
        let s = signal("ReduceOnly", "Full", Direction::Sell);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_ok());
    }

    #[test]
    fn rejects_buy_when_frozen() {
        let s = signal("Frozen", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_sell_when_frozen() {
        let s = signal("Frozen", "Full", Direction::Sell);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Frozen"));
    }

    // ---- 2. 单票仓位硬线 ----

    #[test]
    fn rejects_buy_when_position_exceeds_10pct() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 12.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("仓位"));
    }

    #[test]
    fn allows_buy_at_position_boundary_10pct() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 10.0);
        assert!(r.is_ok());
    }

    #[test]
    fn allows_buy_at_position_exactly_10pct() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 10.0);
        assert!(r.is_ok());
    }

    #[test]
    fn rejects_buy_at_position_just_over_10pct() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 10.001);
        assert!(r.is_err());
    }

    // ---- 3. 现金底 ----

    #[test]
    fn rejects_when_cash_below_15pct() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 10000.0, 100000.0, 5.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("现金"));
    }

    #[test]
    fn allows_when_cash_above_15pct() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 30000.0, 100000.0, 5.0);
        assert!(r.is_ok());
    }

    // ---- 4. DataMode ----

    #[test]
    fn rejects_buy_when_degraded() {
        let s = signal("Normal", "Degraded", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Degraded"));
    }

    #[test]
    fn allows_sell_when_degraded() {
        let s = signal("Normal", "Degraded", Direction::Sell);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_ok(), "Degraded + Sell 应通过 (允许减仓)");
    }

    #[test]
    fn rejects_all_when_unsafe() {
        let s_buy = signal("Normal", "Unsafe", Direction::Buy);
        assert!(pre_trade_check(&s_buy, 50.0, 50000.0, 100000.0, 5.0).is_err());

        let s_sell = signal("Normal", "Unsafe", Direction::Sell);
        assert!(pre_trade_check(&s_sell, 50.0, 50000.0, 100000.0, 5.0).is_err());
    }

    #[test]
    fn rejects_unknown_data_mode() {
        let s = signal("Normal", "Weird", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("未知 data_mode"));
    }

    // ---- 5. 优先级: account_mode 优先于其他 ----

    #[test]
    fn account_mode_check_runs_first() {
        let s = signal("Frozen", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 50000.0, 100000.0, 5.0);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Frozen"));
    }

    // ---- 6. 边界: total_value=0 不触发 cash 检查 (避免除零) ----

    #[test]
    fn handles_zero_total_value() {
        let s = signal("Normal", "Full", Direction::Buy);
        let r = pre_trade_check(&s, 50.0, 0.0, 0.0, 0.0);
        assert!(r.is_ok());
    }
}
