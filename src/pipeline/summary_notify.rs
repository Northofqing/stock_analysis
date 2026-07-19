//! 汇总通知：生成图表 / 日报 Markdown / 保存 + 推送。

use anyhow::Result;
use log::{error, info};
use std::path::Path;

use crate::chart_generator::ChartGenerator;
use crate::notification::NotificationService;
use crate::strategy::core::BacktestSummary;

use super::{reporting, AnalysisResult};

struct SummaryArtifacts {
    report: String,
    filename: String,
    backtest: Option<(String, String)>,
}

fn build_summary_artifacts(
    notifier: &NotificationService,
    results: &[AnalysisResult],
    backtest_summary: Option<&BacktestSummary>,
    regime_section: Option<&str>,
    chain_analysis_section: Option<&str>,
    date: &str,
) -> SummaryArtifacts {
    let stock_report = notifier.generate_daily_report(results, regime_section);
    let report = compose_summary_report(stock_report, chain_analysis_section);
    let backtest = backtest_summary.map(|summary| {
        (
            reporting::build_backtest_report(summary),
            format!("backtest_report_{date}.md"),
        )
    });
    SummaryArtifacts {
        report,
        filename: format!("stock_analysis_{date}.md"),
        backtest,
    }
}

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
    send_summary_notification_to(
        notifier,
        results,
        backtest_summary,
        regime_section,
        chain_analysis_section,
        Path::new("reports"),
    )
    .await
}

pub(super) async fn send_summary_notification_to(
    notifier: &NotificationService,
    results: &[AnalysisResult],
    backtest_summary: Option<&BacktestSummary>,
    regime_section: Option<&str>,
    chain_analysis_section: Option<&str>,
    output_dir: &Path,
) -> Result<()> {
    info!("生成分析汇总报告...");

    let date_str = chrono::Local::now().format("%Y%m%d").to_string();

    let chart_filename = output_dir.join(format!("stock_chart_{}.png", date_str));
    info!("生成分析图表: {}", chart_filename.display());
    let _chart_path =
        match ChartGenerator::generate_summary_chart(results, &chart_filename.to_string_lossy()) {
            Ok(path) => {
                info!("✓ 图表生成成功: {:?}", path);
                Some(path)
            }
            Err(e) => {
                error!("图表生成失败: {}", e);
                None
            }
        };

    let artifacts = build_summary_artifacts(
        notifier,
        results,
        backtest_summary,
        regime_section,
        chain_analysis_section,
        &date_str,
    );

    if let Some((backtest_report, backtest_filename)) = artifacts.backtest {
        notifier.save_report_to_dir(&backtest_report, &backtest_filename, output_dir)?;
        info!(
            "✓ 多因子回测报告已保存到本地: reports/{}",
            backtest_filename
        );
    }

    notifier.save_report_to_dir(&artifacts.report, &artifacts.filename, output_dir)?;

    match notifier.send(&artifacts.report).await {
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
    use super::{build_summary_artifacts, compose_summary_report, send_summary_notification_to};

    fn result() -> super::AnalysisResult {
        serde_json::from_value(serde_json::json!({
            "code": "TEST_CODE_000001",
            "name": "测试股票",
            "sentiment_score": 60,
            "operation_advice": "观望",
            "trend_prediction": "震荡",
            "analysis_summary": "TEST_CODE_标准分析",
            "is_limit_up": false,
            "contrarian_signal": false
        }))
        .expect("analysis result")
    }

    fn backtest() -> super::BacktestSummary {
        super::BacktestSummary {
            initial_capital: 100_000.0,
            final_value: 110_000.0,
            total_return: 0.1,
            annual_return: 0.2,
            max_drawdown: 0.05,
            sharpe_ratio: 1.0,
            sortino_ratio: 1.2,
            calmar_ratio: 4.0,
            average_exposure: 0.6,
            total_trades: 10,
            win_rate: 0.6,
            chart_path: None,
            benchmark_annual_return: None,
            alpha: None,
            benchmark_name: None,
            benchmark_total_return: None,
            excess_return: None,
            beta: None,
            information_ratio: None,
            max_dd_duration_days: 5,
        }
    }

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

    #[test]
    fn summary_artifacts_preserve_chain_regime_and_optional_backtest_evidence() {
        let notifier = crate::notification::NotificationService::new(Default::default());
        let value = result();
        let summary = backtest();
        let artifacts = build_summary_artifacts(
            &notifier,
            std::slice::from_ref(&value),
            Some(&summary),
            Some("TEST_CODE_市场状态"),
            Some("TEST_CODE_产业链"),
            "20260718",
        );
        assert_eq!(artifacts.filename, "stock_analysis_20260718.md");
        assert!(artifacts.report.starts_with("TEST_CODE_产业链"));
        assert!(artifacts.report.contains("TEST_CODE_市场状态"));
        let (backtest_report, backtest_filename) = artifacts.backtest.expect("backtest artifact");
        assert_eq!(backtest_filename, "backtest_report_20260718.md");
        assert!(backtest_report.contains("10.00%"));

        let without_backtest =
            build_summary_artifacts(&notifier, &[value], None, None, None, "20260719");
        assert!(without_backtest.backtest.is_none());
        assert_eq!(without_backtest.filename, "stock_analysis_20260719.md");
    }

    #[tokio::test]
    async fn summary_commit_writes_reports_and_uses_disabled_real_notifier_adapter() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let output_dir = std::env::temp_dir().join(format!(
            "stock-analysis-summary-{}-{suffix}",
            std::process::id()
        ));
        let notifier = crate::notification::NotificationService::new(Default::default());
        let value = result();
        let summary = backtest();

        send_summary_notification_to(
            &notifier,
            &[value],
            Some(&summary),
            Some("TEST_CODE_市场状态"),
            Some("TEST_CODE_产业链"),
            &output_dir,
        )
        .await
        .expect("isolated summary commit");

        let names = std::fs::read_dir(&output_dir)
            .expect("summary output directory")
            .map(|entry| {
                entry
                    .expect("summary output entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name.starts_with("stock_analysis_")));
        assert!(names
            .iter()
            .any(|name| name.starts_with("backtest_report_")));
        // Chart rendering is intentionally best-effort (for example a CI host may
        // not have a CJK font); mandatory Markdown artifacts must still commit.
        std::fs::remove_dir_all(output_dir).expect("remove isolated summary artifacts");
    }
}
