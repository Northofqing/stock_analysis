//! v12 MVP5-5.1: performance_feedback (反馈进化).
//!
//! 设计: 跟踪 execution_tracking (PR3 新增) 计算执行率 / MFE / MAE / 规则改动建议.
//!       **仅审计输出**, 不自动改规则 (AGENTS §2.10 业务规则文档化).
//!
//! 输入: execution_tracking 行 + 当前规则集.
//! 输出: FeedbackReport (执行率 + MFE/MAE + 规则建议).

use serde::{Deserialize, Serialize};

/// 单条 execution_tracking 行 (PR3 schema)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExecutionRow {
    pub plan_id: String,
    pub code: String,
    pub expected_price: f64,
    pub actual_change_t1: Option<f64>,
    pub actual_change_t3: Option<f64>,
    pub actual_change_t5: Option<f64>,
    pub mfe: Option<f64>,
    pub mae: Option<f64>,
    pub t1_special_case: Option<String>,
}

/// 执行率统计
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExecutionStats {
    pub total: u32,
    pub executed: u32,
    pub not_executed: u32,
    /// T+1 胜率 (actual_change_t1 > 0)
    pub t1_hit_rate: f64,
    /// T+3 胜率
    pub t3_hit_rate: f64,
    /// T+5 胜率
    pub t5_hit_rate: f64,
}

/// MFE / MAE 统计
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MfeMaeStats {
    pub avg_mfe: f64,        // 平均最大有利偏移 (%)
    pub avg_mae: f64,        // 平均最大不利偏移 (%)
    pub capture_ratio: f64,  // 捕获率: avg_mfe / (avg_mfe - avg_mae), 越高越好
}

/// 规则改动建议 (仅审计输出, 不自动改)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuleSuggestion {
    pub rule_id: String,        // e.g. "BR-006"
    pub issue: String,          // 问题描述
    pub suggestion: String,     // 建议
    pub confidence: f64,         // 0~1
}

/// v12 MVP5 反馈报告
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeedbackReport {
    pub date: String,
    pub stats: ExecutionStats,
    pub mfe_mae: MfeMaeStats,
    pub suggestions: Vec<RuleSuggestion>,
    /// 数据完整度 (有 actual_change 的行 / 总行)
    pub data_completeness: f64,
}

/// MVP5-5.1 主评估
pub fn evaluate(rows: &[ExecutionRow], date: &str) -> FeedbackReport {
    let total = rows.len() as u32;
    let executed = rows.iter().filter(|r| r.actual_change_t1.is_some()).count() as u32;
    let not_executed = total - executed;

    let t1_hits = rows.iter().filter(|r| r.actual_change_t1.map_or(false, |v| v > 0.0)).count();
    let t3_hits = rows.iter().filter(|r| r.actual_change_t3.map_or(false, |v| v > 0.0)).count();
    let t5_hits = rows.iter().filter(|r| r.actual_change_t5.map_or(false, |v| v > 0.0)).count();
    let t1_total = rows.iter().filter(|r| r.actual_change_t1.is_some()).count();
    let t3_total = rows.iter().filter(|r| r.actual_change_t3.is_some()).count();
    let t5_total = rows.iter().filter(|r| r.actual_change_t5.is_some()).count();

    let t1_hit_rate = if t1_total > 0 { t1_hits as f64 / t1_total as f64 } else { 0.0 };
    let t3_hit_rate = if t3_total > 0 { t3_hits as f64 / t3_total as f64 } else { 0.0 };
    let t5_hit_rate = if t5_total > 0 { t5_hits as f64 / t5_total as f64 } else { 0.0 };

    // MFE / MAE
    let mfe_vals: Vec<f64> = rows.iter().filter_map(|r| r.mfe).collect();
    let mae_vals: Vec<f64> = rows.iter().filter_map(|r| r.mae).collect();
    let avg_mfe = if !mfe_vals.is_empty() { mfe_vals.iter().sum::<f64>() / mfe_vals.len() as f64 } else { 0.0 };
    let avg_mae = if !mae_vals.is_empty() { mae_vals.iter().sum::<f64>() / mae_vals.len() as f64 } else { 0.0 };
    let capture_ratio = if avg_mfe > 0.0 { avg_mfe / (avg_mfe - avg_mae) } else { 0.0 };

    // 规则改动建议
    let mut suggestions = Vec::new();
    if t1_hit_rate < 0.3 && t1_total >= 20 {
        suggestions.push(RuleSuggestion {
            rule_id: "BR-006".to_string(),
            issue: format!("T+1 胜率 {:.1}% 低于 30%", t1_hit_rate * 100.0),
            suggestion: "考虑关停胜率持续 < 30% 的主题, 跑 winrate_simulator 验证".to_string(),
            confidence: 0.8,
        });
    }
    if capture_ratio < 0.5 && mfe_vals.len() >= 10 {
        suggestions.push(RuleSuggestion {
            rule_id: "BR-020".to_string(),
            issue: format!("捕获率 {:.2} < 0.5 (MFE={:.2}, MAE={:.2})", capture_ratio, avg_mfe, avg_mae),
            suggestion: "考虑调高止盈阈值或调低止损阈值, 改善捕获率".to_string(),
            confidence: 0.6,
        });
    }

    let data_completeness = if total > 0 { executed as f64 / total as f64 } else { 0.0 };

    FeedbackReport {
        date: date.to_string(),
        stats: ExecutionStats {
            total,
            executed,
            not_executed,
            t1_hit_rate,
            t3_hit_rate,
            t5_hit_rate,
        },
        mfe_mae: MfeMaeStats { avg_mfe, avg_mae, capture_ratio },
        suggestions,
        data_completeness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_with_t1(code: &str, change: f64, mfe: f64, mae: f64) -> ExecutionRow {
        ExecutionRow {
            plan_id: format!("plan-{}", code),
            code: code.to_string(),
            expected_price: 10.0,
            actual_change_t1: Some(change),
            actual_change_t3: None,
            actual_change_t5: None,
            mfe: Some(mfe),
            mae: Some(mae),
            t1_special_case: None,
        }
    }

    #[test]
    fn empty_input_zero_stats() {
        let r = evaluate(&[], "2026-07-05");
        assert_eq!(r.stats.total, 0);
        assert_eq!(r.data_completeness, 0.0);
        assert!(r.suggestions.is_empty());
    }

    #[test]
    fn high_win_rate_no_suggestion() {
        let rows: Vec<ExecutionRow> = (0..30).map(|i| row_with_t1(&format!("{:06}", i), 1.0, 5.0, -2.0)).collect();
        let r = evaluate(&rows, "2026-07-05");
        assert_eq!(r.stats.t1_hit_rate, 1.0);
        assert!(r.suggestions.is_empty(), "高胜率不应有规则建议");
    }

    #[test]
    fn low_win_rate_triggers_suggestion() {
        let rows: Vec<ExecutionRow> = (0..30).map(|i| row_with_t1(&format!("{:06}", i), -1.0, 0.5, -3.0)).collect();
        let r = evaluate(&rows, "2026-07-05");
        assert!(r.stats.t1_hit_rate < 0.3);
        assert!(!r.suggestions.is_empty(), "低胜率应触发规则建议");
        assert_eq!(r.suggestions[0].rule_id, "BR-006");
    }

    #[test]
    fn capture_ratio_calc() {
        // MFE=5, MAE=-2 → capture_ratio = 5 / (5 - (-2)) = 5/7 ≈ 0.71
        let rows = vec![row_with_t1("A", 1.0, 5.0, -2.0)];
        let r = evaluate(&rows, "2026-07-05");
        assert!((r.mfe_mae.capture_ratio - 5.0 / 7.0).abs() < 0.01);
    }

    #[test]
    fn low_capture_ratio_triggers_suggestion() {
        // MFE=2, MAE=-10 → capture_ratio = 2/12 ≈ 0.17 < 0.5
        let rows: Vec<ExecutionRow> = (0..15).map(|i| row_with_t1(&format!("{:06}", i), -1.0, 2.0, -10.0)).collect();
        let r = evaluate(&rows, "2026-07-05");
        assert!(r.mfe_mae.capture_ratio < 0.5);
        // 应有 ≥1 个建议 (胜率低 + 捕获率低)
        assert!(r.suggestions.len() >= 1);
    }

    #[test]
    fn suggestions_never_auto_apply() {
        // 验证 suggestions 仅审计, 不返回"applied" 字段
        let rows: Vec<ExecutionRow> = (0..30).map(|i| row_with_t1(&format!("{:06}", i), -1.0, 0.5, -3.0)).collect();
        let r = evaluate(&rows, "2026-07-05");
        for s in &r.suggestions {
            assert!(s.confidence >= 0.0 && s.confidence <= 1.0);
            assert!(!s.suggestion.contains("applied"), "suggestion 不应含 'applied'");
        }
    }
}