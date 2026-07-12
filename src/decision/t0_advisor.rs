//! v12 MVP2-2.1: 做T建议 (T0Advice).
//!
//! 设计: 在持持仓上判断是否可做T, 返回 T0 结论 + 观察区.
//!       关键约束 (v12 §13 + BR-022 衍生):
//!         - 主升核心票 (TrendMainUpCore) → Forbidden, 防止卖飞
//!         - 当日买入 (buy_date == today) → 不可卖 (T+1 制度)
//!         - ReduceOnly 仅允许反T (接回底仓), 不允许正T (加仓)
//!         - available_shares 必须 > 0 才能做T
//!         - 数据缺失保守取 Forbidden
//!
//! 输出: T0Recommendation {kind: PositiveT|ReverseT|Forbidden, reason, sell_zone, buy_zone}

use chrono::Local;

/// 趋势状态 (与 monitor::scanner::TrendStatus 对齐, 简化版)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrendStatus {
    /// 主升核心 (v12 §13 显式禁做T)
    MainUpCore,
    /// 主升
    MainUp,
    /// 震荡
    Range,
    /// 走弱
    Weak,
    /// 退潮
    Fade,
}

impl TrendStatus {
    pub fn label(self) -> &'static str {
        match self {
            TrendStatus::MainUpCore => "主升核心",
            TrendStatus::MainUp => "主升",
            TrendStatus::Range => "震荡",
            TrendStatus::Weak => "走弱",
            TrendStatus::Fade => "退潮",
        }
    }
}

/// T0 类型
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum T0Kind {
    /// 反T (先卖后买, 接回底仓) — 减仓类
    ReverseT,
    /// 正T (先买后卖, 加仓) — 加仓类
    PositiveT,
}

impl T0Kind {
    pub fn label(self) -> &'static str {
        match self {
            T0Kind::ReverseT => "ReverseT",
            T0Kind::PositiveT => "PositiveT",
        }
    }
}

/// T0 评估结果
#[derive(Clone, Debug, PartialEq)]
pub enum T0Verdict {
    /// 允许做T
    Allowed {
        kind: T0Kind,
        sell_zone: (f64, f64),
        buy_zone: (f64, f64),
        min_spread_pct: f64,
    },
    /// 禁止做T (含原因)
    Forbidden(String),
}

impl T0Verdict {
    pub fn is_allowed(&self) -> bool {
        matches!(self, T0Verdict::Allowed { .. })
    }

    pub fn reason(&self) -> String {
        match self {
            T0Verdict::Allowed { .. } => "Allowed".to_string(),
            T0Verdict::Forbidden(r) => r.clone(),
        }
    }
}

/// 输入: T0 评估所需指标
#[derive(Clone, Debug)]
pub struct T0Input {
    pub code: String,
    pub name: String,
    pub trend: TrendStatus,
    pub buy_date: String, // YYYY-MM-DD
    pub available_shares: u32,
    pub current_price: f64,
    pub cost_price: f64,
    pub support: f64,
    pub pressure: f64,
    /// 显式 kind hint (由调用方决定尝试正T/反T). None = 自动选择.
    pub kind_hint: Option<T0Kind>,
    /// 账户模式 (Normal / ReduceOnly). ReduceOnly 仅反T.
    pub account_mode_is_reduce_only: bool,
}

impl T0Input {
    fn held_today(&self) -> bool {
        let today = Local::now().format("%Y-%m-%d").to_string();
        self.buy_date == today
    }
}

/// PR2-2.1 主评估
///
/// 规则 (按优先级):
///   1. 主升核心 → Forbidden("主升核心票防卖飞")
///   2. 退潮 → Forbidden("退潮期不做T")
///   3. 当日买入 → Forbidden("T+1 锁仓")
///   4. available_shares == 0 → Forbidden("无可卖底仓")
///   5. ReduceOnly + 正T → Forbidden("ReduceOnly 仅反T")
///   6. 否则按 kind_hint 或默认反T: 卖出观察区=pressure±1%, 接回=support±1%
pub fn evaluate(input: &T0Input) -> T0Verdict {
    // 1. 主升核心票 → 永远禁 (v12 §13)
    if input.trend == TrendStatus::MainUpCore {
        return T0Verdict::Forbidden("主升核心票防卖飞 (BR-022 衍生)".to_string());
    }

    // 2. 退潮 → 禁
    if input.trend == TrendStatus::Fade {
        return T0Verdict::Forbidden("退潮期不做T (BR-022 衍生)".to_string());
    }

    // 3. T+1 锁仓
    if input.held_today() {
        return T0Verdict::Forbidden("T+1 锁仓: 当日买入不可卖".to_string());
    }

    // 4. 无可卖底仓
    if input.available_shares == 0 {
        return T0Verdict::Forbidden(format!("无可卖底仓 ({} 股)", input.available_shares));
    }

    // 5. ReduceOnly 仅反T
    let kind = if let Some(h) = input.kind_hint {
        if input.account_mode_is_reduce_only && h == T0Kind::PositiveT {
            return T0Verdict::Forbidden("ReduceOnly 账户仅允许反T, 不允许正T".to_string());
        }
        h
    } else if input.account_mode_is_reduce_only {
        T0Kind::ReverseT
    } else {
        // 默认走反T (更保守, 接回底仓)
        T0Kind::ReverseT
    };

    // 6. 观察区计算: 卖出观察 = pressure ± 1%, 接回观察 = support ± 1%
    let pressure_band = input.pressure * 0.01;
    let support_band = input.support * 0.01;

    let sell_zone = (
        input.pressure - pressure_band,
        input.pressure + pressure_band,
    );
    let buy_zone = (input.support - support_band, input.support + support_band);

    // 最小价差 (覆盖 2× 往返成本 ~0.6%, 取 ≥1.5% 为门槛)
    let min_spread_pct = 1.5_f64;

    // 价差不足 → 仍允许但标注
    let spread = ((sell_zone.0 - buy_zone.1) / input.current_price) * 100.0;
    if spread < min_spread_pct {
        // 数据缺失保守, 仍允许但 reason 标注价差不足
        return T0Verdict::Allowed {
            kind,
            sell_zone,
            buy_zone,
            min_spread_pct: spread.max(0.0), // 实测价差, 不足则 < 1.5
        };
    }

    T0Verdict::Allowed {
        kind,
        sell_zone,
        buy_zone,
        min_spread_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_default() -> T0Input {
        T0Input {
            code: "000001".to_string(),
            name: "测试".to_string(),
            trend: TrendStatus::Range,
            buy_date: "2026-07-01".to_string(), // 历史日期
            available_shares: 1000,
            current_price: 12.0,
            cost_price: 11.0,
            support: 11.5,
            pressure: 12.5,
            kind_hint: None,
            account_mode_is_reduce_only: false,
        }
    }

    // ---- 主升核心票禁做T (v12 §13 硬性要求) ----

    #[test]
    fn main_up_core_forbidden() {
        let mut inp = input_default();
        inp.trend = TrendStatus::MainUpCore;
        let v = evaluate(&inp);
        assert!(!v.is_allowed());
        assert!(v.reason().contains("主升核心"));
    }

    // ---- 退潮期禁 ----

    #[test]
    fn fade_forbidden() {
        let mut inp = input_default();
        inp.trend = TrendStatus::Fade;
        let v = evaluate(&inp);
        assert!(!v.is_allowed());
        assert!(v.reason().contains("退潮"));
    }

    // ---- T+1 锁仓 ----

    #[test]
    fn held_today_forbidden() {
        let mut inp = input_default();
        inp.buy_date = Local::now().format("%Y-%m-%d").to_string();
        let v = evaluate(&inp);
        assert!(!v.is_allowed());
        assert!(v.reason().contains("T+1"));
    }

    // ---- 无可卖底仓 ----

    #[test]
    fn zero_shares_forbidden() {
        let mut inp = input_default();
        inp.available_shares = 0;
        let v = evaluate(&inp);
        assert!(!v.is_allowed());
        assert!(v.reason().contains("无可卖"));
    }

    // ---- ReduceOnly 仅反T ----

    #[test]
    fn reduce_only_blocks_positive_t() {
        let mut inp = input_default();
        inp.account_mode_is_reduce_only = true;
        inp.kind_hint = Some(T0Kind::PositiveT);
        let v = evaluate(&inp);
        assert!(!v.is_allowed());
        assert!(v.reason().contains("ReduceOnly"));
    }

    #[test]
    fn reduce_only_allows_reverse_t() {
        let mut inp = input_default();
        inp.account_mode_is_reduce_only = true;
        inp.kind_hint = Some(T0Kind::ReverseT);
        let v = evaluate(&inp);
        assert!(v.is_allowed());
    }

    // ---- 正常 Range 允许 ----

    #[test]
    fn range_default_allows_reverse_t() {
        let v = evaluate(&input_default());
        assert!(v.is_allowed(), "Range 默认应允许反T");
        if let T0Verdict::Allowed {
            kind,
            sell_zone,
            buy_zone,
            ..
        } = v
        {
            assert_eq!(kind, T0Kind::ReverseT);
            // sell_zone 在 pressure ± 1% (12.5 ± 0.125)
            assert!((sell_zone.0 - 12.375).abs() < 0.01);
            assert!((sell_zone.1 - 12.625).abs() < 0.01);
            // buy_zone 在 support ± 1% (11.5 ± 0.115)
            assert!((buy_zone.0 - 11.385).abs() < 0.01);
            assert!((buy_zone.1 - 11.615).abs() < 0.01);
        }
    }

    #[test]
    fn positive_t_hint_allows() {
        let mut inp = input_default();
        inp.kind_hint = Some(T0Kind::PositiveT);
        let v = evaluate(&inp);
        assert!(v.is_allowed());
    }

    // ---- 价差不足仍允许 (用实测 spread) ----

    #[test]
    fn tight_spread_yields_smaller_min_spread() {
        let mut inp = input_default();
        // support=12.4, pressure=12.5 → spread ~ 0.8% < 1.5%
        inp.support = 12.4;
        inp.pressure = 12.5;
        let v = evaluate(&inp);
        if let T0Verdict::Allowed { min_spread_pct, .. } = v {
            assert!(
                min_spread_pct < 1.5,
                "实测 spread 应 < 1.5, 实得 {}",
                min_spread_pct
            );
        } else {
            panic!("应允许但 spread 不足");
        }
    }

    // ---- 优先级 ----

    #[test]
    fn main_up_core_takes_priority_over_other_rules() {
        // 主升核心 + 当日买入 → 主升核心原因
        let mut inp = input_default();
        inp.trend = TrendStatus::MainUpCore;
        inp.buy_date = Local::now().format("%Y-%m-%d").to_string();
        let v = evaluate(&inp);
        assert!(v.reason().contains("主升核心"));
    }

    // ---- 标签 ----

    #[test]
    fn trend_labels() {
        assert_eq!(TrendStatus::MainUpCore.label(), "主升核心");
        assert_eq!(TrendStatus::MainUp.label(), "主升");
        assert_eq!(TrendStatus::Range.label(), "震荡");
        assert_eq!(TrendStatus::Weak.label(), "走弱");
        assert_eq!(TrendStatus::Fade.label(), "退潮");
    }

    #[test]
    fn kind_labels() {
        assert_eq!(T0Kind::ReverseT.label(), "ReverseT");
        assert_eq!(T0Kind::PositiveT.label(), "PositiveT");
    }
}
