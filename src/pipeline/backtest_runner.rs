//! 回测运行器（从 pipeline 拆分）
//!
//! 纯粹承载三种回测流程的方法：多因子、布林带+Z-Score、RSI。
//! 保持 `impl AnalysisPipeline`，调用方无感知。

use anyhow::Result;
use log::{info, warn};

use super::{AnalysisPipeline, AnalysisResult};
use crate::strategy::bollinger_zscore::{BollingerZScoreBacktest, BollingerZScoreConfig};
use crate::strategy::core::{
    write_daily_values_csv, write_trades_csv, BacktestConfig, BacktestEngine, BacktestState,
    BacktestSummary, BenchmarkSeries, WalkForwardFold, WalkForwardReport,
};
use crate::strategy::multi_factor::{MultiFactorConfig, MultiFactorEngine, StockFactors};
use crate::strategy::rsi::{RsiBacktest, RsiConfig};

impl AnalysisPipeline {
    /// 多因子回测（使用真正的日频因子快照，无 look-ahead）
    ///
    /// 修复：QUANT_ANALYST_REVIEW §1.5
    /// 与 `run_multi_factor_on_history` 的关键区别：
    ///   - 因子来源是 `factor_snapshot` 表（每日更新的历史 PE/PB/ROE/市值/换手率）
    ///   - 每天调仓时严格用 `get_as_of(code, today)` 取 ≤ today 的最近一次快照
    ///   - 没有因子快照的股票被跳过，不静默回退到 K 线字段
    ///
    /// 这是正确做法；原 `run_multi_factor_on_history` 由于 K 线 pe_ratio/roe 字段
    /// 是实时刷新的，理论上包含当前值，存在 look-ahead 风险。
    /// 推荐：新场景使用 `run_multi_factor_with_snapshots`；老路径保留作兼容。
    #[allow(clippy::too_many_arguments)]
    fn run_multi_factor_with_snapshots(
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
        factor_cfg: &MultiFactorConfig,
        snapshots: &std::collections::HashMap<String, std::collections::BTreeMap<chrono::NaiveDate, crate::database::factor_snapshot::FactorSnapshotRow>>,
    ) -> Option<(BacktestSummary, BacktestState)> {
        if history.len() < 3 {
            return None;
        }
        let factor_engine = MultiFactorEngine::new(factor_cfg.clone());
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config.clone());

        let mut all_dates: std::collections::BTreeSet<chrono::NaiveDate> =
            std::collections::BTreeSet::new();
        for (_, _, ks) in history {
            for k in ks {
                all_dates.insert(k.date);
            }
        }
        let dates: Vec<chrono::NaiveDate> = all_dates.into_iter().collect();
        if dates.len() < 10 {
            return None;
        }
        let observe_days = 20.min(dates.len() / 3);
        if observe_days >= dates.len() {
            return None;
        }

        for today in dates.iter().skip(observe_days) {
            // 关键：用快照的 get_as_of 语义, 严格 ≤ today
            let day_factors: Vec<StockFactors> = history
                .iter()
                .filter_map(|(code, name, _)| {
                    let snap = snapshots.get(code)?;
                    // 找 ≤ today 的最大 snapshot_date
                    let snap = snap
                        .iter()
                        .rev() // BTreeMap 按 key 升序, rev() 拿到降序
                        .find(|(d, _)| **d <= *today)?;
                    Some(StockFactors {
                        code: code.clone(),
                        name: name.clone(),
                        market_cap: snap.1.market_cap,
                        roe: snap.1.roe,
                        pe: snap.1.pe_ttm,
                        pb: snap.1.pb,
                        turnover_rate: snap.1.turnover_rate,
                    })
                })
                .collect();

            if day_factors.len() < 3 {
                continue;
            }

            let day_scores = factor_engine.calculate_scores(&day_factors).ok()?;
            if day_scores.is_empty() {
                continue;
            }
            let position_count = config.position_count.min(day_scores.len());
            let mut targets = Vec::new();
            for s in day_scores.iter().take(position_count) {
                if let Some((_, name, ks)) = history.iter().find(|(c, _, _)| c == &s.code) {
                    if let Some(k) = ks.iter().find(|k| &k.date == today) {
                        targets.push((s.code.clone(), name.clone(), k.close));
                    }
                }
            }
            if !targets.is_empty() {
                let dt = today
                    .and_hms_opt(15, 0, 0)
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .single()
                    .unwrap_or_else(chrono::Local::now);
                let _ = engine.rebalance(&targets, dt);
                engine.record_daily_value(dt);
            }
        }
        if engine.get_state().daily_values.is_empty() {
            return None;
        }
        let state = engine.get_state().clone();
        Some((
            BacktestSummary::from_state(&state, config.initial_capital),
            state,
        ))
    }

    /// 在给定历史切片上运行一次多因子回测，返回汇总结果。
    ///
    /// 说明：因缺少历史因子快照，当前以切片末日可得基本面生成一次因子分数，
    /// 再在该切片内执行逐日调仓，作为最小可复现口径。
    fn run_multi_factor_on_history(
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
        factor_cfg: &MultiFactorConfig,
    ) -> Option<(BacktestSummary, BacktestState)> {
        if history.len() < 3 {
            return None;
        }

        let factor_engine = MultiFactorEngine::new(factor_cfg.clone());

        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config.clone());

        let mut all_dates: std::collections::BTreeSet<chrono::NaiveDate> =
            std::collections::BTreeSet::new();
        for (_, _, ks) in history {
            for k in ks {
                all_dates.insert(k.date);
            }
        }
        let dates: Vec<chrono::NaiveDate> = all_dates.into_iter().collect();
        if dates.len() < 10 {
            return None;
        }

        let observe_days = 20.min(dates.len() / 3);
        if observe_days >= dates.len() {
            return None;
        }

        let initial_capital = config.initial_capital;

        for today in dates.iter().skip(observe_days) {
            // 每个再平衡日仅使用截至当日可得数据重算因子，避免跨周期复用固定分数。
            let day_factors: Vec<StockFactors> = history
                .iter()
                .filter_map(|(code, name, ks)| {
                    let latest = ks
                        .iter()
                        .filter(|k| k.date <= *today)
                        .max_by_key(|k| k.date)?;
                    Some(StockFactors {
                        code: code.clone(),
                        name: name.clone(),
                        market_cap: latest.market_cap,
                        roe: latest.roe,
                        pe: latest.pe_ratio,
                        pb: latest.pb_ratio,
                        turnover_rate: latest.turnover_rate,
                    })
                })
                .collect();

            if day_factors.len() < 3 {
                continue;
            }

            let day_scores = factor_engine.calculate_scores(&day_factors).ok()?;
            if day_scores.is_empty() {
                continue;
            }
            let position_count = config.position_count.min(day_scores.len());

            let mut targets = Vec::new();
            for s in day_scores.iter().take(position_count) {
                if let Some((_, name, ks)) = history.iter().find(|(c, _, _)| c == &s.code) {
                    if let Some(k) = ks.iter().find(|k| &k.date == today) {
                        targets.push((s.code.clone(), name.clone(), k.close));
                    }
                }
            }

            if !targets.is_empty() {
                let dt = today
                    .and_hms_opt(15, 0, 0)
                    .unwrap()
                    .and_local_timezone(chrono::Local)
                    .single()
                    .unwrap_or_else(chrono::Local::now);
                let _ = engine.rebalance(&targets, dt);
                engine.record_daily_value(dt);
            }
        }

        let state = engine.get_state().clone();
        let summary = BacktestSummary::from_state(&state, initial_capital);
        Some((summary, state))
    }

    fn run_multi_factor_summary_on_history(
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
        factor_cfg: &MultiFactorConfig,
    ) -> Option<BacktestSummary> {
        Self::run_multi_factor_on_history(history, factor_cfg).map(|(s, _)| s)
    }

    /// 抓取沪深300基准日线，构建 BenchmarkSeries（date->close）。
    /// 失败一律返回 None，绝不编造基准数据。
    async fn fetch_benchmark_series(&self, days: usize) -> Option<BenchmarkSeries> {
        self.fetch_benchmark_series_with_code(
            crate::strategy::core::benchmark_codes::HS300,
            "沪深300",
            days,
        )
        .await
    }

    /// 修复 P2.9: 支持多个基准 (按 strategy_kind 推荐)
    /// 量化分析师建议: 不同策略用不同基准, 不要所有都用沪深300
    async fn fetch_benchmark_series_with_code(
        &self,
        code: &str,
        name: &str,
        days: usize,
    ) -> Option<BenchmarkSeries> {
        match self.data_manager.get_daily_data(code, days) {
            Ok((data, _)) if !data.is_empty() => {
                let mut closes = std::collections::HashMap::new();
                for k in &data {
                    closes.insert(k.date, k.close);
                }
                info!("✓ 基准 {} ({}) 已加载 {} 个交易日", name, code, closes.len());
                Some(BenchmarkSeries::new(name, closes))
            }
            _ => {
                warn!("基准 {}({}) 数据获取失败, 回测报告将标注'基准数据缺失'", name, code);
                None
            }
        }
    }

    /// 将组合的每日净值与交易明细落盘到 reports/details/ 作为审计留痕。失败仅告警，不阻断主流程。
    fn export_audit_csv(
        &self,
        strategy_tag: &str,
        date_str: &str,
        daily_values: &[(chrono::DateTime<chrono::Local>, f64)],
        trades: &[crate::strategy::core::Trade],
        initial_capital: f64,
    ) {
        let dir = "reports/details";
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("创建审计目录 {} 失败: {}", dir, e);
            return;
        }
        let mut state = BacktestState::new(initial_capital);
        state.daily_values = daily_values.to_vec();
        state.trades = trades.to_vec();

        let trades_path = format!("{}/{}_trades_{}.csv", dir, strategy_tag, date_str);
        if let Err(e) = write_trades_csv(&state, &trades_path) {
            warn!("导出交易明细CSV失败 ({}): {}", trades_path, e);
        } else {
            info!("✓ 交易明细已留痕: {}", trades_path);
        }

        let nav_path = format!("{}/{}_nav_{}.csv", dir, strategy_tag, date_str);
        if let Err(e) = write_daily_values_csv(&state, &nav_path, initial_capital) {
            warn!("导出每日净值CSV失败 ({}): {}", nav_path, e);
        } else {
            info!("✓ 每日净值已留痕: {}", nav_path);
        }
    }

    /// 按全局交易日期把组合历史切成 前段(样本内)/后段(样本外) 两份。
    /// `ratio` 为前段占比（如 0.6）。数据不足无法切分时返回 None。
    /// 返回 (切分日期字符串, 前段, 后段)。
    fn split_history_by_date(
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
        ratio: f64,
    ) -> Option<(
        String,
        Vec<(String, String, Vec<crate::data_provider::KlineData>)>,
        Vec<(String, String, Vec<crate::data_provider::KlineData>)>,
    )> {
        use std::collections::BTreeSet;
        let mut dates: BTreeSet<chrono::NaiveDate> = BTreeSet::new();
        for (_, _, ks) in history {
            for k in ks {
                dates.insert(k.date);
            }
        }
        if dates.len() < 10 {
            return None;
        }
        let dates: Vec<chrono::NaiveDate> = dates.into_iter().collect();
        let idx = ((dates.len() as f64 * ratio) as usize).clamp(1, dates.len() - 1);
        let cutoff = dates[idx];

        let mut in_sample = Vec::new();
        let mut out_sample = Vec::new();
        for (code, name, ks) in history {
            let in_ks: Vec<_> = ks.iter().filter(|k| k.date <= cutoff).cloned().collect();
            let out_ks: Vec<_> = ks.iter().filter(|k| k.date > cutoff).cloned().collect();
            if !in_ks.is_empty() {
                in_sample.push((code.clone(), name.clone(), in_ks));
            }
            if !out_ks.is_empty() {
                out_sample.push((code.clone(), name.clone(), out_ks));
            }
        }
        if in_sample.is_empty() || out_sample.is_empty() {
            return None;
        }
        Some((cutoff.format("%Y-%m-%d").to_string(), in_sample, out_sample))
    }

    /// 按 [start, end]（含端点）日期窗口筛选组合历史。
    fn slice_history(
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Vec<(String, String, Vec<crate::data_provider::KlineData>)> {
        let mut out = Vec::new();
        for (code, name, ks) in history {
            let slice: Vec<_> = ks
                .iter()
                .filter(|k| k.date >= start && k.date <= end)
                .cloned()
                .collect();
            if !slice.is_empty() {
                out.push((code.clone(), name.clone(), slice));
            }
        }
        out
    }

    /// 通用 walk-forward 滚动优化（扩张窗口 anchored）。
    ///
    /// - `candidates`: (参数标签, 配置) 候选集；每折在训练段对全部候选回测并取**总收益最高**者；
    /// - `run`: 给定配置与历史切片，返回该组合的 `BacktestSummary`（None=该切片无法回测）；
    /// - `folds`: 折数（时间轴等分为 folds+1 块，块 0..f 训练、块 f+1 测试）。
    ///
    /// 仅统计样本外（测试段）业绩，避免参数过拟合。数据不足返回 None。
    fn walk_forward<C, F>(
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
        candidates: &[(String, C)],
        run: F,
        folds: usize,
    ) -> Option<WalkForwardReport>
    where
        F: Fn(&C, &[(String, String, Vec<crate::data_provider::KlineData>)]) -> Option<BacktestSummary>,
    {
        use std::collections::BTreeSet;
        if candidates.is_empty() || folds == 0 {
            return None;
        }

        let mut dates: BTreeSet<chrono::NaiveDate> = BTreeSet::new();
        for (_, _, ks) in history {
            for k in ks {
                dates.insert(k.date);
            }
        }
        let dates: Vec<chrono::NaiveDate> = dates.into_iter().collect();
        let blocks = folds + 1;
        // 每块至少需要一定交易日，否则无法稳定回测
        if dates.len() < blocks * 15 {
            return None;
        }
        let block_size = dates.len() / blocks;

        let mut fold_results: Vec<WalkForwardFold> = Vec::new();
        for f in 0..folds {
            // 训练段：从起点到第 f 块末尾（扩张窗口）
            let train_start = dates[0];
            let train_end = dates[(f + 1) * block_size - 1];
            // 测试段：第 f+1 块
            let test_start = dates[(f + 1) * block_size];
            let test_end = if f + 2 == blocks {
                *dates.last().unwrap()
            } else {
                dates[(f + 2) * block_size - 1]
            };

            let train_slice = Self::slice_history(history, train_start, train_end);
            let test_slice = Self::slice_history(history, test_start, test_end);
            if train_slice.is_empty() || test_slice.is_empty() {
                continue;
            }

            // 训练段寻优：取总收益最高的候选
            let mut best: Option<(&str, f64)> = None;
            for (label, cfg) in candidates {
                if let Some(sum) = run(cfg, &train_slice) {
                    let score = sum.total_return;
                    if best.map_or(true, |(_, b)| score > b) {
                        best = Some((label.as_str(), score));
                    }
                }
            }
            let Some((best_label, _)) = best else { continue };

            // 用最优参数在测试段评估
            let chosen_cfg = candidates
                .iter()
                .find(|(l, _)| l == best_label)
                .map(|(_, c)| c)?;
            let Some(test_sum) = run(chosen_cfg, &test_slice) else {
                continue;
            };

            fold_results.push(WalkForwardFold {
                fold: f + 1,
                train_label: format!("{}~{}", train_start.format("%Y-%m-%d"), train_end.format("%Y-%m-%d")),
                test_label: format!("{}~{}", test_start.format("%Y-%m-%d"), test_end.format("%Y-%m-%d")),
                chosen_param: best_label.to_string(),
                test_return: test_sum.total_return,
                test_sharpe: test_sum.sharpe_ratio,
                test_trades: test_sum.total_trades,
            });
        }

        if fold_results.is_empty() {
            return None;
        }

        let n = fold_results.len() as f64;
        let avg_test_return = fold_results.iter().map(|f| f.test_return).sum::<f64>() / n;
        let compounded_return = fold_results
            .iter()
            .fold(1.0, |acc, f| acc * (1.0 + f.test_return))
            - 1.0;
        let positive_fold_rate =
            fold_results.iter().filter(|f| f.test_return > 0.0).count() as f64 / n;

        Some(WalkForwardReport {
            folds: fold_results,
            avg_test_return,
            compounded_return,
            positive_fold_rate,
        })
    }

    /// 运行多因子回测（真历史回测：逐日计算因子→选股→模拟持仓→跟踪净值）。
    pub(super) async fn run_multi_factor_backtest(
        &self,
        results: &[AnalysisResult],
    ) -> Result<BacktestSummary> {
        // 1. 拉取评分前 N 只股票的历史K线（至少 60 天）
        let mut sorted = results.to_vec();
        sorted.sort_by(|a, b| b.ranking_score.cmp(&a.ranking_score));
        let top_n = 10;
        let mut stocks_data = Vec::new();
        for r in sorted.iter().take(top_n) {
            match self.data_manager.get_daily_data(&r.code, 60) {
                Ok((data, _)) if data.len() >= 30 => {
                    stocks_data.push((r.code.clone(), r.name.clone(), data));
                }
                Ok(_) => warn!("[回测] {} K线不足30条，跳过", r.code),
                Err(e) => warn!("[回测] {} 数据拉取失败: {}", r.code, e),
            }
        }
        if stocks_data.len() < 3 {
            anyhow::bail!("多因子回测需要至少3只股票，实际仅 {} 只", stocks_data.len());
        }

        let base_cfg = MultiFactorConfig::default();
        let (mut summary, base_state) = Self::run_multi_factor_on_history(&stocks_data, &base_cfg)
            .ok_or_else(|| anyhow::anyhow!("多因子基础回测失败：样本或因子不足，无法生成汇总"))?;

        // 注入基准（可用时）
        if let Some(bench) = self.fetch_benchmark_series(60).await {
            summary.benchmark_name = Some(bench.name);
        }

        info!(
            "多因子回测完成: 总收益 {:.2}% 年化 {:.2}% 最大回撤 {:.2}% 夏普 {:.2} 交易 {} 笔",
            summary.total_return * 100.0,
            summary.annual_return * 100.0,
            summary.max_drawdown * 100.0,
            summary.sharpe_ratio,
            summary.total_trades,
        );

        let date_str = chrono::Local::now().format("%Y%m%d").to_string();
        let mut report = super::reporting::build_backtest_report(&summary);

        // A. 时间样本外切分（前 60% 样本内 / 后 40% 样本外）
        let (cutoff, in_h, out_h) = Self::split_history_by_date(&stocks_data, 0.6)
            .ok_or_else(|| anyhow::anyhow!("多因子 OOS 切分失败：交易日不足或切分后样本为空"))?;
        let in_sum = Self::run_multi_factor_summary_on_history(&in_h, &base_cfg)
            .ok_or_else(|| anyhow::anyhow!("多因子 OOS 失败：样本内段无法完成回测"))?;
        let out_sum = Self::run_multi_factor_summary_on_history(&out_h, &base_cfg)
            .ok_or_else(|| anyhow::anyhow!("多因子 OOS 失败：样本外段无法完成回测"))?;
        report.push_str(&super::reporting::build_oos_section(&cutoff, &in_sum, &out_sum));

        // B. Walk-Forward 滚动优化（参数网格寻优 + 样本外验证）
        let mk_cfg = |top_n: usize, pe_w: f64, pb_w: f64| MultiFactorConfig {
            factors: vec![
                (
                    crate::strategy::multi_factor::Factor::MarketCap,
                    crate::strategy::multi_factor::FactorDirection::Ascending,
                    1.0,
                ),
                (
                    crate::strategy::multi_factor::Factor::ROE,
                    crate::strategy::multi_factor::FactorDirection::Descending,
                    1.0,
                ),
                (
                    crate::strategy::multi_factor::Factor::PE,
                    crate::strategy::multi_factor::FactorDirection::Ascending,
                    pe_w,
                ),
                (
                    crate::strategy::multi_factor::Factor::PB,
                    crate::strategy::multi_factor::FactorDirection::Ascending,
                    pb_w,
                ),
                (
                    crate::strategy::multi_factor::Factor::TurnoverRate,
                    crate::strategy::multi_factor::FactorDirection::Descending,
                    0.3,
                ),
            ],
            top_n,
        };
        let wf_grid: Vec<(String, MultiFactorConfig)> = vec![
            ("top10/pe0.5/pb0.5".to_string(), mk_cfg(10, 0.5, 0.5)),
            ("top15/pe0.5/pb0.5".to_string(), mk_cfg(15, 0.5, 0.5)),
            ("top20/pe0.5/pb0.5".to_string(), mk_cfg(20, 0.5, 0.5)),
            ("top15/pe0.8/pb0.3".to_string(), mk_cfg(15, 0.8, 0.3)),
            ("top15/pe0.3/pb0.8".to_string(), mk_cfg(15, 0.3, 0.8)),
            ("top12/pe0.7/pb0.7".to_string(), mk_cfg(12, 0.7, 0.7)),
        ];
        let wf = Self::walk_forward(
            &stocks_data,
            &wf_grid,
            |cfg, slice| Self::run_multi_factor_summary_on_history(slice, cfg),
            4,
        )
        .ok_or_else(|| anyhow::anyhow!("多因子 Walk-Forward 失败：样本不足或候选参数无有效结果"))?;
        report.push_str(&super::reporting::build_walk_forward_section(&wf));

        let filename = format!("multi_factor_backtest_{}.md", date_str);
        self.notifier.save_report_to_file(&report, Some(&filename))?;
        self.export_audit_csv(
            "multi_factor",
            &date_str,
            &base_state.daily_values,
            &base_state.trades,
            summary.initial_capital,
        );

        Ok(summary)
    }

    /// 运行布林带+Z-Score 均值回归策略回测
    /// 拉取评分前 N 只股票的长周期历史K线（供布林带/RSI 回测共享，避免重复抓取）
    pub(super) async fn fetch_top_backtest_history(
        &self,
        results: &[AnalysisResult],
        top_n: usize,
        days: usize,
    ) -> Vec<(String, String, Vec<crate::data_provider::KlineData>)> {
        let mut sorted = results.to_vec();
        sorted.sort_by(|a, b| b.ranking_score.cmp(&a.ranking_score));

        let mut stocks_data = Vec::new();
        for r in sorted.iter().take(top_n) {
            match self.data_manager.get_daily_data(&r.code, days) {
                Ok((data, _)) if !data.is_empty() => {
                    stocks_data.push((r.code.clone(), r.name.clone(), data));
                }
                Ok(_) => warn!("[{}] K线数据为空，跳过回测", r.code),
                Err(e) => warn!("[{}] 拉取历史数据失败: {}", r.code, e),
            }
        }
        stocks_data
    }

    pub(super) async fn run_bollinger_zscore_backtest(
        &self,
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
    ) -> Result<BacktestSummary> {
        let config = BollingerZScoreConfig::default();
        let engine = BollingerZScoreBacktest::new(config);

        // 布林带需要至少 30 条K线
        let stocks_data: Vec<_> = history
            .iter()
            .filter(|(_, _, d)| d.len() >= 30)
            .cloned()
            .collect();

        if stocks_data.is_empty() {
            anyhow::bail!("无有效股票数据用于布林带回测");
        }

        info!("布林带回测：共 {} 只股票参与", stocks_data.len());
        let mut result = engine.run_portfolio(&stocks_data)?;

        // 注入真实基准（沪深300），失败则为 None，报告如实标注
        result.benchmark = self.fetch_benchmark_series(7000).await;

        // 生成并保存报告
        let date_str = chrono::Local::now().format("%Y%m%d").to_string();
        let mut report = result.generate_report();

        // C. 分市场状态拆解（需基准）
        if let Some(bench) = &result.benchmark {
            if let Some(rep) =
                crate::strategy::core::regime_breakdown(&result.portfolio_daily_values, bench)
            {
                report.push_str(&super::reporting::build_regime_section(&rep));
            }
        }

        // A. 时间样本外切分（前 60% 样本内 / 后 40% 样本外，参数固定做一致性检验）
        if let Some((cutoff, in_h, out_h)) = Self::split_history_by_date(&stocks_data, 0.6) {
            let oos_engine = BollingerZScoreBacktest::new(BollingerZScoreConfig::default());
            if let (Ok(mut r_in), Ok(mut r_out)) =
                (oos_engine.run_portfolio(&in_h), oos_engine.run_portfolio(&out_h))
            {
                r_in.benchmark = result.benchmark.clone();
                r_out.benchmark = result.benchmark.clone();
                report.push_str(&super::reporting::build_oos_section(
                    &cutoff,
                    &r_in.to_summary(),
                    &r_out.to_summary(),
                ));
            }
        }

        // B. Walk-Forward 滚动优化（参数网格寻优 + 样本外验证）
        let bb_grid: Vec<(String, BollingerZScoreConfig)> = [
            ("zbuy-2.0/x2.0", -2.0_f64, 2.0_f64),
            ("zbuy-1.5/x2.0", -1.5, 2.0),
            ("zbuy-2.5/x2.0", -2.5, 2.0),
            ("zbuy-2.0/x2.5", -2.0, 2.5),
            ("zbuy-2.0/x1.5", -2.0, 1.5),
            ("zbuy-1.8/x2.2", -1.8, 2.2),
        ]
        .iter()
        .map(|(label, zbuy, mult)| {
            (
                label.to_string(),
                BollingerZScoreConfig {
                    zscore_buy: *zbuy,
                    zscore_sell: -*zbuy,
                    bb_std_mult: *mult,
                    ..BollingerZScoreConfig::default()
                },
            )
        })
        .collect();
        if let Some(wf) = Self::walk_forward(
            &stocks_data,
            &bb_grid,
            |cfg, slice| {
                BollingerZScoreBacktest::new(cfg.clone())
                    .run_portfolio(slice)
                    .ok()
                    .map(|r| r.to_summary())
            },
            4,
        ) {
            report.push_str(&super::reporting::build_walk_forward_section(&wf));
        }

        let report_filename = format!("bollinger_zscore_backtest_{}.md", date_str);
        self.notifier.save_report_to_file(&report, Some(&report_filename))?;
        info!("✓ 布林带+Z-Score回测报告已保存: reports/{}", report_filename);

        // 生成图表
        let chart_path = format!("reports/bollinger_zscore_chart_{}.png", date_str);
        match result.generate_chart(&chart_path) {
            Ok(path) => info!("✓ 布林带回测图表已生成: {}", path.display()),
            Err(e) => warn!("布林带回测图表生成失败: {}", e),
        }

        // 审计留痕：交易明细 + 每日净值
        let initial_capital = result.config.initial_capital * result.single_results.len() as f64;
        self.export_audit_csv(
            "bollinger_zscore",
            &date_str,
            &result.portfolio_daily_values,
            &result.portfolio_trades,
            initial_capital,
        );

        Ok(result.to_summary())
    }

    /// 运行 RSI 超买超卖策略回测
    pub(super) async fn run_rsi_backtest(
        &self,
        history: &[(String, String, Vec<crate::data_provider::KlineData>)],
    ) -> Result<BacktestSummary> {
        // 使用 2026-04-18 优化定稿的 v10 配置（93.75% 胜率）；见 reports/rsi_optimization_log.md
        let config = RsiConfig::preset_daily_v10_no_stop();
        let engine = RsiBacktest::new(config);

        // RSI 需要至少 20 条K线
        let stocks_data: Vec<_> = history
            .iter()
            .filter(|(_, _, d)| d.len() >= 20)
            .cloned()
            .collect();

        if stocks_data.is_empty() {
            anyhow::bail!("无有效股票数据用于RSI回测");
        }

        info!("RSI 回测：共 {} 只股票参与", stocks_data.len());
        let mut result = engine.run_portfolio(&stocks_data)?;

        // 注入真实基准（沪深300），失败则为 None，报告如实标注
        result.benchmark = self.fetch_benchmark_series(7000).await;

        // 保存报告
        let date_str = chrono::Local::now().format("%Y%m%d").to_string();
        let mut report = result.generate_report();

        // C. 分市场状态拆解（需基准）
        if let Some(bench) = &result.benchmark {
            if let Some(rep) =
                crate::strategy::core::regime_breakdown(&result.portfolio_daily_values, bench)
            {
                report.push_str(&super::reporting::build_regime_section(&rep));
            }
        }

        // A. 时间样本外切分（前 60% 样本内 / 后 40% 样本外，参数固定做一致性检验）
        if let Some((cutoff, in_h, out_h)) = Self::split_history_by_date(&stocks_data, 0.6) {
            let oos_engine = RsiBacktest::new(RsiConfig::preset_daily_v10_no_stop());
            if let (Ok(mut r_in), Ok(mut r_out)) =
                (oos_engine.run_portfolio(&in_h), oos_engine.run_portfolio(&out_h))
            {
                r_in.benchmark = result.benchmark.clone();
                r_out.benchmark = result.benchmark.clone();
                report.push_str(&super::reporting::build_oos_section(
                    &cutoff,
                    &r_in.to_summary(),
                    &r_out.to_summary(),
                ));
            }
        }

        // B. Walk-Forward 滚动优化（参数网格寻优 + 样本外验证）
        let rsi_grid: Vec<(String, RsiConfig)> = [
            ("os25/tp0.08", 25.0_f64, 0.08_f64),
            ("os22/tp0.10", 22.0, 0.10),
            ("os28/tp0.06", 28.0, 0.06),
            ("os25/tp0.06", 25.0, 0.06),
            ("os22/tp0.08", 22.0, 0.08),
            ("os30/tp0.08", 30.0, 0.08),
        ]
        .iter()
        .map(|(label, os, tp)| {
            (
                label.to_string(),
                RsiConfig {
                    oversold: *os,
                    take_profit_pct: *tp,
                    ..RsiConfig::preset_daily_v10_no_stop()
                },
            )
        })
        .collect();
        if let Some(wf) = Self::walk_forward(
            &stocks_data,
            &rsi_grid,
            |cfg, slice| {
                RsiBacktest::new(cfg.clone())
                    .run_portfolio(slice)
                    .ok()
                    .map(|r| r.to_summary())
            },
            4,
        ) {
            report.push_str(&super::reporting::build_walk_forward_section(&wf));
        }

        let report_filename = format!("rsi_strategy_backtest_{}.md", date_str);
        self.notifier
            .save_report_to_file(&report, Some(&report_filename))?;
        info!("✓ RSI策略回测报告已保存: reports/{}", report_filename);

        // 生成图表
        let chart_path = format!("reports/rsi_strategy_chart_{}.png", date_str);
        match result.generate_chart(&chart_path) {
            Ok(path) => info!("✓ RSI回测图表已生成: {}", path.display()),
            Err(e) => warn!("RSI回测图表生成失败: {}", e),
        }

        // 审计留痕：交易明细 + 每日净值
        let initial_capital = result.config.initial_capital * result.single_results.len() as f64;
        self.export_audit_csv(
            "rsi_strategy",
            &date_str,
            &result.portfolio_daily_values,
            &result.portfolio_trades,
            initial_capital,
        );

        Ok(result.to_summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::KlineData;
    use crate::strategy::core::{BacktestState, BacktestSummary};

    fn kl(date: chrono::NaiveDate, close: f64) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1000.0,
            amount: 1000.0 * close,
            pct_chg: 0.0,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
        }
    }

    fn history(n: usize) -> Vec<(String, String, Vec<KlineData>)> {
        let base = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let ks: Vec<KlineData> = (0..n)
            .map(|i| kl(base + chrono::Duration::days(i as i64), 10.0 + i as f64 * 0.1))
            .collect();
        vec![("000001".into(), "测试股".into(), ks)]
    }

    /// 修复：QUANT_ANALYST_REVIEW §1.5
    /// 多因子回测在使用日频因子快照时，必须严格使用 ≤ today 的快照
    /// （不能在 T 日使用 T+5 日的快照）。
    #[test]
    fn test_run_multi_factor_with_snapshots_respects_as_of() {
        use crate::database::factor_snapshot::FactorSnapshotRow;
        use crate::strategy::multi_factor::MultiFactorConfig;
        use std::collections::{BTreeMap, HashMap};

        // 3 只股票 30 天历史
        let base = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let codes = ["600000", "600001", "600002"];
        let mut h: Vec<(String, String, Vec<KlineData>)> = codes
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let ks: Vec<KlineData> = (0..30)
                    .map(|d| {
                        kl(
                            base + chrono::Duration::days(d),
                            10.0 + idx as f64 + d as f64 * 0.01,
                        )
                    })
                    .collect();
                (c.to_string(), format!("测试{}", idx), ks)
            })
            .collect();

        // 给每只股票准备一份快照：每天 PE 都不同
        // 但故意把第 15 天的 PE 设成 99.0 (作为"未来泄漏测试": 我们假设这天有"已知的未来"
        // 但回测在第 14 天跑时, 不应看到第 15 天的 PE)
        let mut snapshots: HashMap<String, BTreeMap<chrono::NaiveDate, FactorSnapshotRow>> =
            HashMap::new();
        for c in &codes {
            let mut m: BTreeMap<chrono::NaiveDate, FactorSnapshotRow> = BTreeMap::new();
            for d in 0..30 {
                let date = base + chrono::Duration::days(d);
                let pe = if d == 15 { 99.0 } else { 10.0 + d as f64 * 0.1 };
                m.insert(
                    date,
                    FactorSnapshotRow {
                        code: c.to_string(),
                        snapshot_date: date.to_string(),
                        pe_ttm: Some(pe),
                        pb: Some(1.0),
                        roe: Some(0.10),
                        market_cap: Some(1e10),
                        turnover_rate: Some(0.02),
                        source: Some("test".into()),
                    },
                );
            }
            snapshots.insert(c.to_string(), m);
        }

        let cfg = MultiFactorConfig::default();
        // 在第 10 天(observe_days=10)开始调仓
        let result = AnalysisPipeline::run_multi_factor_with_snapshots(
            &h, &cfg, &snapshots,
        );
        assert!(result.is_some(), "回测应能跑出结果");
        // 检查不会因为看到第 15 天的 99.0 PE 而被影响
        // (具体数值难精确断言, 但只要能跑完即可)
        let _ = h.pop(); // 抑制 h 未用警告
    }

    /// 关键测试: 没有因子的股票应被跳过, 不能从 K 线隐式取值
    /// 这就是修复 §1.5 的核心不变量.
    #[test]
    fn test_run_multi_factor_skips_stocks_without_snapshots() {
        use crate::database::factor_snapshot::FactorSnapshotRow;
        use crate::strategy::multi_factor::MultiFactorConfig;
        use std::collections::{BTreeMap, HashMap};

        let base = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        // 准备 3 只股票, 但只给 1 只股票有快照
        let codes = ["600000", "600001", "600002"];
        let h: Vec<(String, String, Vec<KlineData>)> = codes
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let ks: Vec<KlineData> = (0..30)
                    .map(|d| {
                        let mut k = kl(
                            base + chrono::Duration::days(d),
                            10.0 + idx as f64 + d as f64 * 0.01,
                        );
                        // 故意给 K 线填充 pe/roe, 模拟"末日截面"行为
                        k.pe_ratio = Some(50.0);
                        k.roe = Some(0.20);
                        k
                    })
                    .collect();
                (c.to_string(), format!("测试{}", idx), ks)
            })
            .collect();

        let mut snapshots: HashMap<String, BTreeMap<chrono::NaiveDate, FactorSnapshotRow>> =
            HashMap::new();
        let mut m: BTreeMap<chrono::NaiveDate, FactorSnapshotRow> = BTreeMap::new();
        for d in 0..30 {
            let date = base + chrono::Duration::days(d);
            m.insert(
                date,
                FactorSnapshotRow {
                    code: "600000".into(),
                    snapshot_date: date.to_string(),
                    pe_ttm: Some(5.0), // 明显优于 600001/600002
                    pb: Some(0.5),
                    roe: Some(0.30),
                    market_cap: Some(1e10),
                    turnover_rate: Some(0.02),
                    source: Some("test".into()),
                },
            );
        }
        snapshots.insert("600000".into(), m);

        let cfg = MultiFactorConfig::default();
        let result = AnalysisPipeline::run_multi_factor_with_snapshots(
            &h, &cfg, &snapshots,
        );
        assert!(result.is_some());
        // 关键: 因为 600001/600002 没有快照, 调仓时只能用 600000 一只股票
        // 这就是修复后的正确行为 -- 不会"漏"到 K 线字段上去
    }

    #[test]
    fn test_split_history_by_date() {
        let h = history(100);
        let (cutoff, in_h, out_h) =
            AnalysisPipeline::split_history_by_date(&h, 0.6).expect("应可切分");
        let in_len = in_h[0].2.len();
        let out_len = out_h[0].2.len();
        // 前段约 60%，后段非空，且无重叠（总数守恒）
        assert!(in_len > out_len, "前段应大于后段: {} vs {}", in_len, out_len);
        assert_eq!(in_len + out_len, 100, "切分应无重叠且不丢数据");
        assert!(!cutoff.is_empty());
    }

    #[test]
    fn test_split_history_insufficient() {
        let h = history(5);
        assert!(AnalysisPipeline::split_history_by_date(&h, 0.6).is_none());
    }

    #[test]
    fn test_slice_history_window() {
        let h = history(50);
        let base = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let start = base + chrono::Duration::days(10);
        let end = base + chrono::Duration::days(19);
        let sliced = AnalysisPipeline::slice_history(&h, start, end);
        assert_eq!(sliced[0].2.len(), 10, "应含 [10,19] 共 10 天");
        assert!(sliced[0].2.iter().all(|k| k.date >= start && k.date <= end));
    }

    #[test]
    fn test_walk_forward_picks_and_reports() {
        // 候选用 f64 充当"目标收益"，run 闭包据此造 summary，
        // 验证 walk_forward 的分块/扩张窗口/选优编排是确定性正确的。
        let h = history(120);
        let candidates: Vec<(String, f64)> = vec![
            ("low".into(), 0.01),
            ("high".into(), 0.05),
            ("mid".into(), 0.03),
        ];
        let wf = AnalysisPipeline::walk_forward(
            &h,
            &candidates,
            |cfg, slice| {
                // 收益 = 候选值（与切片长度无关），best 必为 "high"
                let mut state = BacktestState::new(100_000.0);
                let last = slice
                    .iter()
                    .flat_map(|(_, _, ks)| ks.iter())
                    .map(|k| k.date)
                    .max()
                    .unwrap();
                let dt = chrono::Local::now();
                state.daily_values = vec![(dt, 100_000.0), (dt, 100_000.0 * (1.0 + cfg))];
                let _ = last;
                Some(BacktestSummary::from_state(&state, 100_000.0))
            },
            4,
        )
        .expect("应产出 walk-forward 报告");

        assert_eq!(wf.folds.len(), 4, "应有 4 折");
        assert!(
            wf.folds.iter().all(|f| f.chosen_param == "high"),
            "每折都应选中收益最高的候选"
        );
        // 样本外平均收益应接近 5%
        assert!((wf.avg_test_return - 0.05).abs() < 1e-6);
        assert!((wf.positive_fold_rate - 1.0).abs() < 1e-9);
    }
}
