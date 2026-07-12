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
