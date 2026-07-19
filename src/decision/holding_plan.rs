//! v12 PR4-4.1: 持仓明日三预案 (高开 / 平开 / 低开).
//!
//! 设计: 盘后为每持仓生成三预案, 消费支撑压力/chip_distribution/最近资金流.
//!       数据缺失时预案降级并标注 (BR-005 §13 准确性).
//!
//! 输出: HoldingThreePlans (高开/平开/低开三预案 + 数据完整度 + 降级标注)

use chrono::Local;

/// 预案动作
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlanAction {
    /// 减仓 1/3
    ReduceOneThird,
    /// 减仓 1/2
    ReduceHalf,
    /// 清仓
    Clear,
    /// 持有观望
    Hold,
    /// 加仓
    Add,
}

impl PlanAction {
    pub fn label(self) -> &'static str {
        match self {
            PlanAction::ReduceOneThird => "减仓1/3",
            PlanAction::ReduceHalf => "减仓1/2",
            PlanAction::Clear => "清仓",
            PlanAction::Hold => "持有观望",
            PlanAction::Add => "加仓",
        }
    }
}

/// 单预案
#[derive(Clone, Debug)]
pub struct ScenarioPlan {
    pub name: &'static str,     // "高开"/"平开"/"低开"
    pub gap_threshold_pct: f64, // 高开阈值 (例如 +2.0 表示 >2% 高开)
    pub action: PlanAction,
    pub rationale: String,   // 预案理由 (人类可读)
    pub data_degraded: bool, // 数据缺失降级标记
}

impl ScenarioPlan {
    pub fn high_open(plan: HoldingThreePlansInput) -> Self {
        // 高开 (>X%): 减仓兑现利润, 不贪
        let data_degraded = !plan.has_support_pressure;
        Self {
            name: "高开",
            gap_threshold_pct: plan.high_gap_x,
            action: PlanAction::ReduceOneThird,
            rationale: if plan.has_support_pressure {
                "高开兑现部分利润, 剩余持仓观察压力位突破".to_string()
            } else {
                "数据缺失, 保守减仓1/3 (降级)".to_string()
            },
            data_degraded,
        }
    }

    pub fn flat_open(plan: HoldingThreePlansInput) -> Self {
        let data_degraded = !plan.has_chip_distribution;
        Self {
            name: "平开",
            gap_threshold_pct: 0.0,
            action: PlanAction::Hold,
            rationale: if plan.has_chip_distribution {
                "平开持有, 观察筹码分布与主力净流".to_string()
            } else {
                "数据缺失, 持有观望 (降级)".to_string()
            },
            data_degraded,
        }
    }

    pub fn low_open(plan: HoldingThreePlansInput) -> Self {
        let data_degraded = !plan.has_recent_flow;
        Self {
            name: "低开",
            gap_threshold_pct: -plan.high_gap_x,
            action: PlanAction::Hold, // 默认 Hold, 若止损命中走止损路径
            rationale: if plan.has_recent_flow {
                "低开不杀跌, 观察主力净流是否恶化".to_string()
            } else {
                "数据缺失, 持有观望 + 盯紧止损 (降级)".to_string()
            },
            data_degraded,
        }
    }
}

/// 输入: 生成三预案所需的最小数据集
#[derive(Clone, Debug)]
pub struct HoldingThreePlansInput {
    pub code: String,
    pub name: String,
    /// 高开阈值 (例: 2.0 表示 >2%)
    pub high_gap_x: f64,
    /// 是否有支撑压力位数据
    pub has_support_pressure: bool,
    /// 是否有筹码分布数据
    pub has_chip_distribution: bool,
    /// 是否有近期资金流数据
    pub has_recent_flow: bool,
    /// 硬止损价 (低开跌破则执行止损)
    pub hard_stop: f64,
    /// 浮盈百分比 (用于决策调整)
    pub pnl_pct: f64,
}

/// 输出: 三预案
#[derive(Clone, Debug)]
pub struct HoldingThreePlans {
    pub code: String,
    pub name: String,
    pub date: String,
    pub scenarios: Vec<ScenarioPlan>,
    /// 整体降级 (任一关键数据缺失)
    pub overall_degraded: bool,
}

impl HoldingThreePlans {
    /// 主评估: 生成三预案
    ///
    /// 规则 (v12 §13 准确性):
    ///   1. 高开 (>high_gap_x%): 减仓1/3 (数据缺失 → 仍减仓, 标降级)
    ///   2. 平开 (±high_gap_x%): 持有观望 (数据缺失 → 仍持有, 标降级)
    ///   3. 低开 (<-high_gap_x%): 持有观望, 但跌破 hard_stop 则走止损路径 (由调用方决策)
    pub fn evaluate(input: HoldingThreePlansInput) -> Self {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let scenarios = vec![
            ScenarioPlan::high_open(input.clone()),
            ScenarioPlan::flat_open(input.clone()),
            ScenarioPlan::low_open(input.clone()),
        ];
        let overall_degraded = scenarios.iter().any(|s| s.data_degraded);
        Self {
            code: input.code.clone(),
            name: input.name.clone(),
            date: today,
            scenarios,
            overall_degraded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_full() -> HoldingThreePlansInput {
        HoldingThreePlansInput {
            code: "TEST_CODE_000001".to_string(),
            name: "测试".to_string(),
            high_gap_x: 2.0,
            has_support_pressure: true,
            has_chip_distribution: true,
            has_recent_flow: true,
            hard_stop: 9.50,
            pnl_pct: 5.0,
        }
    }

    #[test]
    fn all_three_scenarios_generated() {
        let plans = HoldingThreePlans::evaluate(input_full());
        assert_eq!(plans.scenarios.len(), 3);
        assert!(!plans.overall_degraded);
    }

    #[test]
    fn high_open_reduces_one_third() {
        let s = ScenarioPlan::high_open(input_full());
        assert_eq!(s.name, "高开");
        assert_eq!(s.action, PlanAction::ReduceOneThird);
        assert!(!s.data_degraded);
        assert!(s.rationale.contains("高开兑现"));
    }

    #[test]
    fn flat_open_holds() {
        let s = ScenarioPlan::flat_open(input_full());
        assert_eq!(s.action, PlanAction::Hold);
        assert!(!s.data_degraded);
    }

    #[test]
    fn low_open_holds_with_stop_caveat() {
        let s = ScenarioPlan::low_open(input_full());
        assert_eq!(s.action, PlanAction::Hold);
        assert!(s.rationale.contains("主力净流"));
    }

    #[test]
    fn missing_data_marks_degraded() {
        let mut inp = input_full();
        inp.has_support_pressure = false;
        let s = ScenarioPlan::high_open(inp);
        assert!(s.data_degraded, "数据缺失应标降级");
        assert!(s.rationale.contains("降级"));
    }

    #[test]
    fn overall_degraded_if_any_scenario_degraded() {
        let mut inp = input_full();
        inp.has_chip_distribution = false; // flat_open 会降级
        let plans = HoldingThreePlans::evaluate(inp);
        assert!(plans.overall_degraded);
    }

    #[test]
    fn action_labels() {
        assert_eq!(PlanAction::ReduceOneThird.label(), "减仓1/3");
        assert_eq!(PlanAction::ReduceHalf.label(), "减仓1/2");
        assert_eq!(PlanAction::Clear.label(), "清仓");
        assert_eq!(PlanAction::Hold.label(), "持有观望");
        assert_eq!(PlanAction::Add.label(), "加仓");
    }

    #[test]
    fn date_in_output() {
        let plans = HoldingThreePlans::evaluate(input_full());
        assert!(!plans.date.is_empty());
        // 形如 2026-07-05
        assert_eq!(plans.date.len(), 10);
    }
}
