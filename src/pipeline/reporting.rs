//! 报告与通知渲染：单股报告、回测汇总 Markdown。
//!
//! 这里只包含纯粹的字符串拼装，不做 IO 与异步调用，便于测试。

use crate::strategy::core::BacktestSummary;

use super::AnalysisResult;

/// 生成单股通知文案（单股推送模式使用）。
pub(super) fn generate_single_report(result: &AnalysisResult) -> String {
    let limit_up_tag = if result.is_limit_up {
        " 🔥涨停"
    } else {
        ""
    };
    let contrarian_tag = if result.contrarian_signal {
        " 🔄反向信号"
    } else {
        ""
    };
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
    s.push_str("> ⚠️ **口径说明**: 本报告为**快照外推（非滚动历史回测）**——在分析时刻按当前因子选股建仓并记录市值快照，\n");
    s.push_str("> 因系统暂无历史因子快照，无法进行真正的逐日滚动回测。以下指标仅供参考，不代表可复现的历史业绩。\n\n");
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
        if summary.total_return > 0.0 {
            "盈利"
        } else {
            "亏损"
        }
    ));
    s.push_str(&format!(
        "| 年化收益率 | {:.2}% | 折算成年化收益 |\n",
        summary.annual_return * 100.0
    ));
    s.push_str(&format!(
        "| 最大回撤 | {:.2}% | {} |\n",
        summary.max_drawdown * 100.0,
        if summary.max_drawdown < 0.1 {
            "风险较低"
        } else if summary.max_drawdown < 0.2 {
            "风险适中"
        } else {
            "风险较高"
        }
    ));
    s.push_str(&format!(
        "| 夏普比率(年化) | {:.3} | 已扣2.5%无风险率 |\n",
        summary.sharpe_ratio
    ));
    s.push_str(&format!(
        "| Sortino比率(年化) | {:.3} | 只惩罚下行波动 |\n",
        summary.sortino_ratio
    ));
    s.push_str(&format!(
        "| Calmar比率 | {:.3} | 年化收益/最大回撤 |\n",
        summary.calmar_ratio
    ));
    s.push_str(&format!(
        "| 平均仓位 | {:.1}% | 暴露率(越低越保守) |\n",
        summary.average_exposure * 100.0
    ));
    s.push_str(&format!(
        "| 最长回撤恢复期 | {} 天 | 净值创新高前的最长水下天数 |\n",
        summary.max_dd_duration_days
    ));

    // 基准对标：仅在有真实基准数据时展示，缺失则如实标注
    match summary.benchmark_name.as_deref() {
        Some(name) => {
            if let Some(bench_total) = summary.benchmark_total_return {
                s.push_str(&format!(
                    "| 基准总收益 | {:.2}% | {}同期 |\n",
                    bench_total * 100.0,
                    name
                ));
            }
            if let Some(bench_annual) = summary.benchmark_annual_return {
                s.push_str(&format!(
                    "| 基准年化收益 | {:.2}% | {}年化 |\n",
                    bench_annual * 100.0,
                    name
                ));
            }
            if let Some(excess) = summary.excess_return {
                s.push_str(&format!(
                    "| 超额收益 | {:.2}% | 策略-基准总收益 |\n",
                    excess * 100.0
                ));
            }
            if let Some(alpha) = summary.alpha {
                s.push_str(&format!(
                    "| Alpha(年化) | {:.2}% | CAPM 风险调整后超额 |\n",
                    alpha * 100.0
                ));
            }
            if let Some(beta) = summary.beta {
                s.push_str(&format!("| Beta | {:.3} | 相对基准的系统性敞口 |\n", beta));
            }
            if let Some(ir) = summary.information_ratio {
                s.push_str(&format!(
                    "| 信息比率 | {:.3} | 超额收益/跟踪误差(年化) |\n",
                    ir
                ));
            }
        }
        None => {
            s.push_str("| 基准对标 | 基准数据缺失 | 未获取到沪深300或无足够对齐样本 |\n");
        }
    }

    s.push_str(&format!(
        "| 交易次数 | {} 次 | 总交易次数 |\n",
        summary.total_trades
    ));
    s.push_str(&format!(
        "| 胜率 | {:.1}% | 盈利交易占比 |\n",
        summary.win_rate * 100.0
    ));

    s.push_str("\n## 风险免责声明\n\n");
    s.push_str("- 股票池为静态清单，存在**幸存者偏差**（未含历史退市/ST/被剔除标的）\n");
    s.push_str("- 涨跌停、停牌日期间无法成交的假设未完全真实化\n");
    s.push_str("- 成本模型采用固定百分比滑点，未考虑市场流动性变化\n");
    s.push_str("- 回测中基准对标为参考，实际业绩可能存在偏差\n\n");

    s.push_str("## 策略说明\n\n");
    s.push_str("**多因子选股策略**: 基于市值、市盈率、市净率、换手率等多因子综合评分，选出得分最高的3只股票进行等权重配置。\n\n");

    if let Some(chart_path) = &summary.chart_path {
        s.push_str(&format!("**回测图表**: {}\n\n", chart_path));
    }

    s
}

/// 渲染「分市场状态」拆解 Markdown 区块（牛/震荡/熊）。
pub(super) fn build_regime_section(report: &crate::strategy::core::RegimeReport) -> String {
    let mut s = String::new();
    s.push_str("\n## 📈 分市场状态表现（C）\n\n");
    s.push_str(&format!(
        "> 以沪深300在 **{} 个交易日**内的累计涨幅判定趋势：> +{:.0}% 牛市，< {:.0}% 熊市，其余震荡。\n\n",
        report.window,
        report.bull_threshold * 100.0,
        report.bear_threshold * 100.0
    ));
    s.push_str("| 市场状态 | 天数 | 策略累计收益 | 基准累计收益 | 策略超额 | 策略日胜率 |\n");
    s.push_str("|----------|------|--------------|--------------|----------|------------|\n");
    for st in &report.stats {
        s.push_str(&format!(
            "| {} | {} | {:.2}% | {:.2}% | {:.2}% | {:.1}% |\n",
            st.kind.label(),
            st.days,
            st.strat_return * 100.0,
            st.bench_return * 100.0,
            (st.strat_return - st.bench_return) * 100.0,
            st.up_day_rate * 100.0
        ));
    }
    s.push_str("\n_注：累计收益按区间日收益复利计算；用于判断策略在不同市场环境下的适应性，而非单一总收益。_\n\n");
    s
}

/// 渲染「时间样本外切分」对比 Markdown 区块（前段 vs 后段）。
pub(super) fn build_oos_section(
    cutoff: &str,
    in_sample: &BacktestSummary,
    out_sample: &BacktestSummary,
) -> String {
    let mut s = String::new();
    s.push_str("\n## 🧪 时间样本外切分（A）\n\n");
    s.push_str(&format!(
        "> 以交易日 **{}** 为界，将历史拆为前段（样本内）与后段（样本外）各自独立回测。\n",
        cutoff
    ));
    s.push_str(
        "> 策略参数固定（未在前段寻优），此处用于检验**业绩在两段时间内的一致性/衰减**。\n\n",
    );
    s.push_str("| 指标 | 前段(样本内) | 后段(样本外) | 衰减 |\n");
    s.push_str("|------|--------------|--------------|------|\n");
    let row = |name: &str, a: f64, b: f64, pct: bool| -> String {
        if pct {
            format!(
                "| {} | {:.2}% | {:.2}% | {:+.2}pp |\n",
                name,
                a * 100.0,
                b * 100.0,
                (b - a) * 100.0
            )
        } else {
            format!("| {} | {:.3} | {:.3} | {:+.3} |\n", name, a, b, b - a)
        }
    };
    s.push_str(&row(
        "总收益率",
        in_sample.total_return,
        out_sample.total_return,
        true,
    ));
    s.push_str(&row(
        "年化收益率",
        in_sample.annual_return,
        out_sample.annual_return,
        true,
    ));
    s.push_str(&row(
        "最大回撤",
        in_sample.max_drawdown,
        out_sample.max_drawdown,
        true,
    ));
    s.push_str(&row(
        "夏普比率",
        in_sample.sharpe_ratio,
        out_sample.sharpe_ratio,
        false,
    ));
    s.push_str(&row("胜率", in_sample.win_rate, out_sample.win_rate, true));
    s.push_str(&format!(
        "| 交易次数 | {} | {} | {:+} |\n",
        in_sample.total_trades,
        out_sample.total_trades,
        out_sample.total_trades as i64 - in_sample.total_trades as i64
    ));
    s.push_str("\n_注：后段指标显著低于前段提示策略可能过拟合或环境漂移；接近则稳健性较好。_\n\n");
    s
}

/// 渲染「Walk-Forward 滚动优化」Markdown 区块（B）。
pub(super) fn build_walk_forward_section(
    report: &crate::strategy::core::WalkForwardReport,
) -> String {
    let mut s = String::new();
    s.push_str("\n## 🔁 Walk-Forward 滚动优化（B）\n\n");
    s.push_str(
        "> 扩张窗口（anchored）：每折在历史样本内做参数寻优，再前推到下一段**未见数据**上验证。\n",
    );
    s.push_str("> 仅统计样本外（test）业绩，避免参数过拟合带来的虚高收益。\n\n");
    s.push_str("| 折 | 训练区间(样本内) | 测试区间(样本外) | 选中参数 | 样本外收益 | 样本外夏普 | 样本外交易 |\n");
    s.push_str("|----|------------------|------------------|----------|------------|------------|------------|\n");
    for f in &report.folds {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {:.2}% | {:.3} | {} |\n",
            f.fold,
            f.train_label,
            f.test_label,
            f.chosen_param,
            f.test_return * 100.0,
            f.test_sharpe,
            f.test_trades
        ));
    }
    s.push_str(&format!(
        "\n- **样本外平均收益**: {:.2}%\n",
        report.avg_test_return * 100.0
    ));
    s.push_str(&format!(
        "- **样本外串联复利**: {:.2}%\n",
        report.compounded_return * 100.0
    ));
    s.push_str(&format!(
        "- **样本外为正折数占比**: {:.1}%\n\n",
        report.positive_fold_rate * 100.0
    ));
    s.push_str("_注：若样本外收益与样本内寻优结果落差大，或为正折数占比偏低，说明参数稳定性不足，不宜实盘。_\n\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::core::{
        RegimeKind, RegimeReport, RegimeStats, WalkForwardFold, WalkForwardReport,
    };

    fn summary() -> BacktestSummary {
        BacktestSummary {
            initial_capital: 100_000.0,
            final_value: 110_000.0,
            total_return: 0.10,
            annual_return: 0.20,
            max_drawdown: 0.05,
            sharpe_ratio: 1.234,
            sortino_ratio: 1.5,
            calmar_ratio: 4.0,
            average_exposure: 0.6,
            total_trades: 12,
            win_rate: 0.75,
            chart_path: None,
            benchmark_annual_return: None,
            alpha: None,
            benchmark_name: None,
            benchmark_total_return: None,
            excess_return: None,
            beta: None,
            information_ratio: None,
            max_dd_duration_days: 8,
        }
    }

    #[test]
    fn single_report_renders_limit_up_and_contrarian_evidence() {
        let result: AnalysisResult = serde_json::from_value(serde_json::json!({
            "code": "TEST_CODE_000001",
            "name": "测试股",
            "sentiment_score": 80,
            "operation_advice": "建议买入",
            "trend_prediction": "上行",
            "analysis_summary": "独立分析正文",
            "is_limit_up": true,
            "contrarian_signal": true,
            "contrarian_reason": "超跌企稳"
        }))
        .expect("valid public AnalysisResult fixture");

        let report = generate_single_report(&result);

        assert!(report.contains("测试股(TEST_CODE_000001) 🔥涨停 🔄反向信号"));
        assert!(report.contains("操作建议: 建议买入"));
        assert!(report.contains("评分: 80\n超跌企稳"));
        assert!(report.ends_with("独立分析正文"));
    }

    #[test]
    fn backtest_report_marks_missing_benchmark_loss_and_drawdown_bands() {
        let mut low = summary();
        low.total_return = -0.10;
        low.max_drawdown = 0.05;
        let low_report = build_backtest_report(&low);
        assert!(low_report.contains("| 总收益率 | -10.00% | 亏损 |"));
        assert!(low_report.contains("| 最大回撤 | 5.00% | 风险较低 |"));
        assert!(low_report.contains("| 基准对标 | 基准数据缺失 |"));
        assert!(!low_report.contains("**回测图表**"));

        let mut medium = summary();
        medium.max_drawdown = 0.10;
        assert!(build_backtest_report(&medium).contains("| 最大回撤 | 10.00% | 风险适中 |"));

        let mut high = summary();
        high.max_drawdown = 0.20;
        assert!(build_backtest_report(&high).contains("| 最大回撤 | 20.00% | 风险较高 |"));
    }

    #[test]
    fn backtest_report_renders_each_real_benchmark_metric_and_chart() {
        let mut value = summary();
        value.benchmark_name = Some("沪深300".into());
        value.benchmark_total_return = Some(0.04);
        value.benchmark_annual_return = Some(0.08);
        value.excess_return = Some(0.06);
        value.alpha = Some(0.03);
        value.beta = Some(0.9);
        value.information_ratio = Some(1.2);
        value.chart_path = Some("reports/test_chart.png".into());

        let report = build_backtest_report(&value);

        assert!(report.contains("| 基准总收益 | 4.00% | 沪深300同期 |"));
        assert!(report.contains("| 基准年化收益 | 8.00% | 沪深300年化 |"));
        assert!(report.contains("| 超额收益 | 6.00% |"));
        assert!(report.contains("| Alpha(年化) | 3.00% |"));
        assert!(report.contains("| Beta | 0.900 |"));
        assert!(report.contains("| 信息比率 | 1.200 |"));
        assert!(report.contains("**回测图表**: reports/test_chart.png"));
    }

    #[test]
    fn regime_oos_and_walk_forward_sections_render_owned_results() {
        let regime = RegimeReport {
            window: 20,
            bull_threshold: 0.05,
            bear_threshold: -0.05,
            stats: vec![
                RegimeStats {
                    kind: RegimeKind::Bull,
                    days: 10,
                    strat_return: 0.12,
                    bench_return: 0.08,
                    up_day_rate: 0.7,
                },
                RegimeStats {
                    kind: RegimeKind::Sideways,
                    days: 8,
                    strat_return: 0.02,
                    bench_return: 0.01,
                    up_day_rate: 0.5,
                },
                RegimeStats {
                    kind: RegimeKind::Bear,
                    days: 6,
                    strat_return: -0.03,
                    bench_return: -0.08,
                    up_day_rate: 0.3,
                },
            ],
        };
        let regime_report = build_regime_section(&regime);
        assert!(regime_report.contains("牛市/上行 | 10 | 12.00% | 8.00% | 4.00% | 70.0%"));
        assert!(regime_report.contains("熊市/下行 | 6 | -3.00% | -8.00% | 5.00% | 30.0%"));

        let mut out_sample = summary();
        out_sample.total_return = 0.05;
        out_sample.annual_return = 0.10;
        out_sample.max_drawdown = 0.08;
        out_sample.sharpe_ratio = 0.8;
        out_sample.win_rate = 0.6;
        out_sample.total_trades = 8;
        let oos = build_oos_section("2026-01-01", &summary(), &out_sample);
        assert!(oos.contains("| 总收益率 | 10.00% | 5.00% | -5.00pp |"));
        assert!(oos.contains("| 夏普比率 | 1.234 | 0.800 | -0.434 |"));
        assert!(oos.contains("| 交易次数 | 12 | 8 | -4 |"));

        let walk = WalkForwardReport {
            folds: vec![WalkForwardFold {
                fold: 1,
                train_label: "2025H1".into(),
                test_label: "2025Q3".into(),
                chosen_param: "top3".into(),
                test_return: 0.03,
                test_sharpe: 1.1,
                test_trades: 4,
            }],
            avg_test_return: 0.03,
            compounded_return: 0.03,
            positive_fold_rate: 1.0,
        };
        let walk_report = build_walk_forward_section(&walk);
        assert!(walk_report.contains("| 1 | 2025H1 | 2025Q3 | top3 | 3.00% | 1.100 | 4 |"));
        assert!(walk_report.contains("**样本外为正折数占比**: 100.0%"));
    }
}
