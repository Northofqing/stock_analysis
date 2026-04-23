//! 回测运行器（从 pipeline 拆分）
//!
//! 纯粹承载三种回测流程的方法：多因子、布林带+Z-Score、RSI。
//! 保持 `impl AnalysisPipeline`，调用方无感知。

use anyhow::Result;
use log::{info, warn};

use super::{AnalysisPipeline, AnalysisResult};
use crate::strategy::bollinger_zscore::{BollingerZScoreBacktest, BollingerZScoreConfig};
use crate::strategy::core::{BacktestConfig, BacktestEngine, BacktestSummary};
use crate::strategy::multi_factor::{MultiFactorConfig, MultiFactorEngine, StockFactors};
use crate::strategy::rsi::{RsiBacktest, RsiConfig};

impl AnalysisPipeline {
    /// 运行多因子回测
    pub(super) async fn run_multi_factor_backtest(&self, results: &[AnalysisResult]) -> Result<BacktestSummary> {
        // 1. 准备因子数据
        let stock_factors: Vec<StockFactors> = results
            .iter()
            .map(|r| StockFactors {
                code: r.code.clone(),
                name: r.name.clone(),
                market_cap: r.market_cap,
                roe: None, // 暂时没有ROE数据
                pe: r.pe_ratio,
                pb: r.pb_ratio,
                turnover_rate: r.turnover_rate,
            })
            .collect();

        // 2. 配置多因子策略
        let multi_factor_config = MultiFactorConfig::default();
        let multi_factor_engine = MultiFactorEngine::new(multi_factor_config);

        // 3. 计算股票得分并选出top N
        let scores = multi_factor_engine.calculate_scores(&stock_factors)?;
        info!("多因子评分完成，前3名: {:?}", 
            scores.iter().take(3).map(|s| format!("{}({:.1}分)", s.name, s.total_score)).collect::<Vec<_>>()
        );

        // 4. 简化回测：假设在分析时刻买入top N股票，持有到现在
        let backtest_config = BacktestConfig::default();
        let mut backtest_engine = BacktestEngine::new(backtest_config.clone());

        // 选出得分最高的N只股票
        let top_stocks: Vec<_> = scores
            .iter()
            .take(backtest_config.position_count)
            .collect();

        // 获取这些股票的最新价格
        let mut target_stocks = Vec::new();
        for stock_score in &top_stocks {
            // 从results中找到对应的股票获取价格
            if let Some(result) = results.iter().find(|r| r.code == stock_score.code) {
                // 尝试获取最新价格
                if let Ok((data, _)) = self.data_manager.get_daily_data(&result.code, 1) {
                    if let Some(latest) = data.last() {
                        target_stocks.push((
                            result.code.clone(),
                            result.name.clone(),
                            latest.close,
                        ));
                    }
                }
            }
        }

        // 执行调仓（买入）
        let now = chrono::Local::now();
        backtest_engine.rebalance(&target_stocks, now)?;

        // 记录初始市值
        backtest_engine.record_daily_value(now);

        // 简化：假设持有一段时间后市值
        // 这里只是示例，实际应该用历史数据进行完整回测
        let state = backtest_engine.get_state();
        let mut summary = BacktestSummary::from_state(state, backtest_config.initial_capital);

        // 生成回测图表
        let chart_path = format!("reports/backtest_chart_{}.png", now.format("%Y%m%d_%H%M%S"));
        match summary.generate_chart(state, &chart_path) {
            Ok(path) => {
                info!("回测图表已生成: {}", path.display());
                summary.set_chart_path(path.to_string_lossy().to_string());
            }
            Err(e) => {
                warn!("生成回测图表失败: {}", e);
            }
        }

        Ok(summary)
    }

    /// 运行布林带+Z-Score 均值回归策略回测
    pub(super) async fn run_bollinger_zscore_backtest(&self, results: &[AnalysisResult]) -> Result<BacktestSummary> {
        let config = BollingerZScoreConfig::default();
        let engine = BollingerZScoreBacktest::new(config);

        // 为评分前 20 的股票拉取较长历史数据（250 日）
        let mut sorted = results.to_vec();
        sorted.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));

        let top_codes: Vec<_> = sorted.iter().take(3).collect();
        let mut stocks_data: Vec<(String, String, Vec<crate::data_provider::KlineData>)> = Vec::new();

        for r in &top_codes {
            match self.data_manager.get_daily_data(&r.code, 7000) {
                Ok((data, _)) if data.len() >= 30 => {
                    stocks_data.push((r.code.clone(), r.name.clone(), data));
                }
                Ok(_) => {
                    warn!("[{}] K线数据不足30条，跳过布林带回测", r.code);
                }
                Err(e) => {
                    warn!("[{}] 拉取历史数据失败: {}", r.code, e);
                }
            }
        }

        if stocks_data.is_empty() {
            anyhow::bail!("无有效股票数据用于布林带回测");
        }

        info!("布林带回测：共 {} 只股票参与", stocks_data.len());
        let result = engine.run_portfolio(&stocks_data)?;

        // 生成并保存报告
        let date_str = chrono::Local::now().format("%Y%m%d").to_string();
        let report = result.generate_report();
        let report_filename = format!("bollinger_zscore_backtest_{}.md", date_str);
        self.notifier.save_report_to_file(&report, Some(&report_filename))?;
        info!("✓ 布林带+Z-Score回测报告已保存: reports/{}", report_filename);

        // 生成图表
        let chart_path = format!("reports/bollinger_zscore_chart_{}.png", date_str);
        match result.generate_chart(&chart_path) {
            Ok(path) => info!("✓ 布林带回测图表已生成: {}", path.display()),
            Err(e) => warn!("布林带回测图表生成失败: {}", e),
        }

        Ok(result.to_summary())
    }

    /// 运行 RSI 超买超卖策略回测
    pub(super) async fn run_rsi_backtest(&self, results: &[AnalysisResult]) -> Result<BacktestSummary> {
        // 使用 2026-04-18 优化定稿的 v10 配置（93.75% 胜率）；见 reports/rsi_optimization_log.md
        let config = RsiConfig::preset_daily_v10_no_stop();
        let engine = RsiBacktest::new(config);

        // 取评分前 20 的股票拉取历史K线（250日）
        let mut sorted = results.to_vec();
        sorted.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));

        let top_codes: Vec<_> = sorted.iter().take(3).collect();
        let mut stocks_data: Vec<(String, String, Vec<crate::data_provider::KlineData>)> =
            Vec::new();

        for r in &top_codes {
            match self.data_manager.get_daily_data(&r.code, 7000) {
                Ok((data, _)) if data.len() >= 20 => {
                    stocks_data.push((r.code.clone(), r.name.clone(), data));
                }
                Ok(_) => {
                    warn!("[{}] K线数据不足20条，跳过RSI回测", r.code);
                }
                Err(e) => {
                    warn!("[{}] 拉取历史数据失败: {}", r.code, e);
                }
            }
        }

        if stocks_data.is_empty() {
            anyhow::bail!("无有效股票数据用于RSI回测");
        }

        info!("RSI 回测：共 {} 只股票参与", stocks_data.len());
        let result = engine.run_portfolio(&stocks_data)?;

        // 保存报告
        let date_str = chrono::Local::now().format("%Y%m%d").to_string();
        let report = result.generate_report();
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

        Ok(result.to_summary())
    }
}
