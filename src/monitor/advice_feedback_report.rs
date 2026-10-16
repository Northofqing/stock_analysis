//! 建议质量报告 —— 自进化反馈回路第③段 (消费) (2026-10-16).
//!
//! 读 `advice_outcome_tracking` 已回填行 → 转
//! `market_analyzer::performance_feedback::ExecutionRow` → `evaluate()` → 渲染推送文本.
//!
//! 约束: `performance_feedback.rs:4` / AGENTS §2.10 —— **仅审计输出, 不自动改规则**.
//!       本模块只产出"给人看的结论", 不修改任何规则/权重/阈值.
//!
//! 口径: 全部是"假设执行"的建议质量 (以推送日收盘为基准, 看后续交易日表现),
//!       **不是用户真实收益** —— 渲染文本里必须写明, 否则会误导.
//! 样本: 少于 `MIN_SAMPLES_CONCLUSION` 时只报计数, 不给胜率结论.

use crate::database::advice_outcome_tracking::{load_filled_outcomes, FilledAdviceRow};
use crate::database::DatabaseManager;
use chrono::NaiveDate;
use stock_analysis::market_analyzer::performance_feedback::{
    evaluate, ExecutionRow, FeedbackReport,
};

/// 样本门槛: 低于此值只报计数、不出胜率结论 (小样本的百分比会误导).
pub const MIN_SAMPLES_CONCLUSION: usize = 10;

/// 单次最多处理多少行 (防止窗口过大拖慢盘后任务).
const REPORT_ROW_LIMIT: i64 = 5000;

/// 已回填行 → `ExecutionRow` (字段一一对应, 口径见模块头).
fn to_execution_rows(rows: &[FilledAdviceRow]) -> Vec<ExecutionRow> {
    rows.iter()
        .map(|row| ExecutionRow {
            plan_id: format!("{}-{}", row.business_date, row.code),
            code: row.code.clone(),
            expected_price: row.expected_price.unwrap_or(0.0),
            actual_change_t1: row.change_t1,
            actual_change_t3: row.change_t3,
            actual_change_t5: row.change_t5,
            mfe: row.mfe,
            mae: row.mae,
            t1_special_case: None,
        })
        .collect()
}

/// 渲染推送文本 (简洁版 —— 用户要求报告类内容整理简化后推送).
fn render_report(report: &FeedbackReport, since: &str, row_count: usize) -> String {
    let stats = &report.stats;
    let mut text = String::new();
    text.push_str(&format!("【建议质量报告】{since} ~ {}\n\n", report.date));
    text.push_str(&format!(
        "样本 {} 条（其中 {} 条已有 T+1 结果）\n",
        row_count, stats.executed
    ));

    if row_count < MIN_SAMPLES_CONCLUSION {
        text.push_str("\n样本不足 10 条，暂不给胜率结论 —— 继续积累。\n");
    } else {
        text.push_str(&format!(
            "\n方向胜率：T+1 {:.0}% / T+3 {:.0}% / T+5 {:.0}%\n",
            stats.t1_hit_rate * 100.,
            stats.t3_hit_rate * 100.,
            stats.t5_hit_rate * 100.
        ));
        text.push_str(&format!(
            "波动：最大有利 {:+.1}% / 最大不利 {:+.1}% / 捕获率 {:.2}\n",
            report.mfe_mae.avg_mfe, report.mfe_mae.avg_mae, report.mfe_mae.capture_ratio
        ));
    }

    if !report.suggestions.is_empty() {
        text.push_str("\n规则建议（仅审计，不自动改）：\n");
        for item in report.suggestions.iter().take(5) {
            text.push_str(&format!(
                "· [{}] {} → {}（置信 {:.0}%）\n",
                item.rule_id,
                item.issue,
                item.suggestion,
                item.confidence * 100.
            ));
        }
    }

    text.push_str(
        "\n口径：以推送日收盘为基准、假设执行的建议质量，停牌/非交易日不计入；不代表实际持仓盈亏。",
    );
    text
}

/// 生成窗口内的建议质量报告文本; 无数据 → 返回空串 (调用方据此跳过推送).
pub fn build_advice_feedback_report(
    db: &DatabaseManager,
    since_business_date: &str,
    as_of: NaiveDate,
) -> Result<String, String> {
    let rows = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        load_filled_outcomes(&mut *conn, since_business_date, REPORT_ROW_LIMIT)?
    };
    if rows.is_empty() {
        return Ok(String::new());
    }
    let exec_rows = to_execution_rows(&rows);
    let report = evaluate(&exec_rows, &as_of.format("%Y-%m-%d").to_string());
    Ok(render_report(&report, since_business_date, rows.len()))
}
