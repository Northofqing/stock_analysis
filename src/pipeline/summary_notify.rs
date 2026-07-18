//! 汇总通知：生成图表 / 日报 Markdown / 保存 + 推送。

use anyhow::Result;
use log::{error, info};

use crate::chart_generator::ChartGenerator;
use crate::notification::NotificationService;
use crate::strategy::core::BacktestSummary;

use super::{reporting, AnalysisResult};

/// 发送当日汇总通知：生成图表，拼装日报，保存到 reports/，并推送。
///
/// `chain_analysis_section`：产业链联动分析（涨停聚类 + LLM 分析），
/// 仅在当日有涨停数据时非空，将作为报告第一部分。
pub(super) async fn send_summary_notification(
    notifier: &NotificationService,
    results: &[AnalysisResult],
    backtest_summary: Option<&BacktestSummary>,
    regime_section: Option<&str>,
    chain_analysis_section: Option<&str>,
) -> Result<()> {
    info!("生成分析汇总报告...");

    let date_str = chrono::Local::now().format("%Y%m%d").to_string();

    let chart_filename = format!("reports/stock_chart_{}.png", date_str);
    info!("生成分析图表: {}", chart_filename);
    let _chart_path = match ChartGenerator::generate_summary_chart(results, &chart_filename) {
        Ok(path) => {
            info!("✓ 图表生成成功: {:?}", path);
            Some(path)
        }
        Err(e) => {
            error!("图表生成失败: {}", e);
            None
        }
    };

    // 日报使用 pipeline 的 AnalysisResult 类型，不做转换
    let stock_report = notifier.generate_daily_report(results, regime_section);

    // 合并产业链分析（如果有涨停数据）到报告头部
    let report = compose_summary_report(stock_report, chain_analysis_section);

    if let Some(summary) = backtest_summary {
        let backtest_report = reporting::build_backtest_report(summary);
        let backtest_filename = format!("backtest_report_{}.md", date_str);
        notifier.save_report_to_file(&backtest_report, Some(&backtest_filename))?;
        info!(
            "✓ 多因子回测报告已保存到本地: reports/{}",
            backtest_filename
        );
    }

    let filename = format!("stock_analysis_{}.md", date_str);
    notifier.save_report_to_file(&report, Some(&filename))?;

    match notifier.send(&report).await {
        Ok(_) => info!("✓ 股票分析报告推送成功"),
        Err(e) => error!("推送通知失败: {}", e),
    }

    Ok(())
}

fn compose_summary_report(stock_report: String, chain_analysis_section: Option<&str>) -> String {
    if let Some(chain) = chain_analysis_section {
        if chain.trim().is_empty() {
            stock_report
        } else {
            format!("{}\n\n---\n\n{}", chain.trim(), stock_report)
        }
    } else {
        stock_report
    }
}

#[cfg(test)]
mod tests {
    use super::compose_summary_report;

    #[test]
    fn summary_composition_preserves_stock_report_and_nonempty_chain() {
        assert_eq!(compose_summary_report("stocks".into(), None), "stocks");
        assert_eq!(
            compose_summary_report("stocks".into(), Some(" \n")),
            "stocks"
        );
        assert_eq!(
            compose_summary_report("stocks".into(), Some("  chain  \n")),
            "chain\n\n---\n\nstocks"
        );
    }
}
