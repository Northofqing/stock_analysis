//! 报告与通知渲染：单股报告、回测汇总 Markdown。
//!
//! 这里只包含纯粹的字符串拼装，不做 IO 与异步调用，便于测试。

use crate::strategy::core::BacktestSummary;

use super::AnalysisResult;

/// 生成单股通知文案（单股推送模式使用）。
pub(super) fn generate_single_report(result: &AnalysisResult) -> String {
    let limit_up_tag = if result.is_limit_up { " 🔥涨停" } else { "" };
    let contrarian_tag = if result.contrarian_signal { " 🔄反向信号" } else { "" };
    let contrarian_line = result
        .contrarian_reason
        .as_ref()
        .map(|r| format!("\n{}", r))
        .unwrap_or_default();

    format!(
        "{} {}({}){}{}\n\n操作建议: {}\n评分: {}{}\n\n{}",
        result.get_emoji(),
        result.name,
        result.code,
        limit_up_tag,
        contrarian_tag,
        result.operation_advice,
        result.sentiment_score,
        contrarian_line,
        result.analysis_summary
    )
}

/// 渲染多因子回测报告 Markdown。
pub(super) fn build_backtest_report(summary: &BacktestSummary) -> String {
    let mut s = String::new();
    s.push_str("# 📊 多因子策略回测报告\n\n");
    s.push_str(&format!(
        "**生成时间**: {}\n\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    s.push_str("---\n\n");

    s.push_str("## 回测结果汇总\n\n");
    s.push_str("| 指标 | 数值 | 说明 |\n");
    s.push_str("|------|------|------|\n");
    s.push_str(&format!(
        "| 初始资金 | ¥{:.2}万 | 回测初始资金 |\n",
        summary.initial_capital / 10000.0
    ));
    s.push_str(&format!(
        "| 期末资产 | ¥{:.2}万 | 当前总资产 |\n",
        summary.final_value / 10000.0
    ));
    s.push_str(&format!(
        "| 总收益率 | {:.2}% | {} |\n",
        summary.total_return * 100.0,
        if summary.total_return > 0.0 { "📈 盈利" } else { "📉 亏损" }
    ));
    s.push_str(&format!(
        "| 年化收益率 | {:.2}% | 折算成年化收益 |\n",
        summary.annual_return * 100.0
    ));
    s.push_str(&format!(
        "| 最大回撤 | {:.2}% | {} |\n",
        summary.max_drawdown * 100.0,
        if summary.max_drawdown < 0.1 {
            "🛡️ 风险较低"
        } else if summary.max_drawdown < 0.2 {
            "⚠️ 风险适中"
        } else {
            "🚨 风险较高"
        }
    ));
    s.push_str(&format!(
        "| 夏普比率 | {:.2} | {} |\n",
        summary.sharpe_ratio,
        if summary.sharpe_ratio > 1.0 {
            "⭐ 优秀"
        } else if summary.sharpe_ratio > 0.5 {
            "✅ 良好"
        } else {
            "⚠️ 一般"
        }
    ));
    s.push_str(&format!(
        "| 交易次数 | {} 次 | 总交易次数 |\n",
        summary.total_trades
    ));
    s.push_str(&format!(
        "| 胜率 | {:.1}% | 盈利交易占比 |\n",
        summary.win_rate * 100.0
    ));

    s.push_str("\n## 策略说明\n\n");
    s.push_str("**多因子选股策略**: 基于市值、市盈率、市净率、换手率等多因子综合评分，选出得分最高的3只股票进行等权重配置。\n\n");

    if let Some(chart_path) = &summary.chart_path {
        s.push_str(&format!("**回测图表**: {}\n\n", chart_path));
    }

    s
}
