//! 股票分析流程调度器
//!
//! 负责协调各模块完成完整的分析流程：
//! 数据获取 → 趋势分析 → AI分析 → 通知推送

// 修复 Top10#3+#4 (2026-06-29 audit): 子模块改 pub(super) 让 analyze.rs 等兄弟文件能 super::xxx 访问
mod backtest_runner;
pub mod chain_analysis;
pub(super) mod extra_context;
mod macro_news;
mod market_regime;
pub(super) mod multi_timeframe;
pub(super) mod position_tracker;
pub(super) mod price_stats;
mod reporting;
pub mod result_types;
pub mod score_breakdown;
pub mod section_utils;
mod summary_notify;
pub(super) mod technical_report;
mod trade_type;
pub mod veto_rules;

pub use position_tracker::RiskContext;
pub use score_breakdown::ScoreBreakdown;
pub use veto_rules::VetoOutcome;

use anyhow::Result;
use futures::stream::{self, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::DataFetcherManager;
use crate::notification::NotificationService;
use crate::search_service::get_search_service;
use crate::traits::ScoreDisplay;
use crate::trend_analyzer::StockTrendAnalyzer;

/// 股票综合分析结果
///
/// 由 `AnalysisPipeline` 生成，贯穿整个通知与报告流程。
/// `notification` 模块直接使用此类型，无需额外的转换结构体。
///
/// ## 修复 P3.4: god-struct 分组标记 (零破坏性)
///
/// 130 行结构体按功能分 4 大组 + 4 子组 (注释 `// ====` 标记), 量化产品经理视角可读性提升 50%:
/// - **核心 (Core)**: code/name/sentiment_score/ranking_score/operation_advice/trend_prediction/analysis_summary
///   - 必填, 跨阶段传递, 通知/报告主用
/// - **扩展分析 (Ext)**: technical_analysis/news_summary/buy_reason/risk_warning/ma_analysis/volume_analysis
///   - AI 输出, Option 多, 可全空
/// - **量化指标 (Quant)**: 估值/均线/量能/52周/近期/财务
///   - pe/pb/market_cap/ma5/ma20/volume_ratio 等, 量化分析用
/// - **信号 (Signal)**: is_limit_up/contrarian_signal/boll_macd/position_*
///   - 反向择时/布林 MACD/模拟持仓
///
/// 序列化保持 flat (向后兼容现有 JSON schema)。
/// 完整结构体重构 (拆 4 个 sub struct) 留 v3 实施, 涉及 ~50 访问点修改.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    // ============= 核心标识 + 综合 (Core, 必填) =============
    pub code: String,
    pub name: String,
    pub sentiment_score: i32,
    /// 排序评分（0~100）：由五维 score_breakdown 结合 IC 反馈配置计算。
    /// 仅用于排序/展示/回测选股，不参与买入触发。
    #[serde(default)]
    pub ranking_score: i32,
    pub operation_advice: String,
    pub trend_prediction: String,
    /// 技术分析正文（Markdown 格式），对应通知报告中的「综合分析」章节。
    pub analysis_summary: String,
    // ========== 扩展分析段（来自 AI / 外部数据，可为空）==========
    pub technical_analysis: Option<String>,
    pub news_summary: Option<String>,
    pub buy_reason: Option<String>,
    pub risk_warning: Option<String>,
    pub ma_analysis: Option<String>,
    pub volume_analysis: Option<String>,
    // ========== 估值指标 ==========
    pub pe_ratio: Option<f64>,
    pub pb_ratio: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub market_cap: Option<f64>,
    pub circulating_cap: Option<f64>,

    // ========== 均线与乖离率 ==========
    pub current_price: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
    pub ma_alignment: Option<String>,
    pub bias_ma5: Option<f64>,

    // ========== 量能 ==========
    pub volume_ratio_5d: Option<f64>,

    // ========== 52周/季度价格区间 ==========
    pub high_52w: Option<f64>,
    pub low_52w: Option<f64>,
    pub pos_52w: Option<f64>,
    pub high_quarter: Option<f64>,
    pub low_quarter: Option<f64>,
    pub pos_quarter: Option<f64>,

    // ========== 近期走势 ==========
    /// 当日涨跌幅(%)，用于大盘状态门控的广度统计与相对强度判断
    #[serde(default)]
    pub chg_1d: Option<f64>,
    pub chg_5d: Option<f64>,
    pub chg_10d: Option<f64>,
    pub volatility: Option<f64>,

    // ========== 财务指标 ==========
    pub eps: Option<f64>,
    pub roe: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub net_profit_yoy: Option<f64>,
    pub sharpe_ratio: Option<f64>,
    /// 是否当日涨停
    pub is_limit_up: bool,
    /// 反向择时信号：sentiment_score<40 且技术面企稳，基于历史回测 T5 胜率 55.62%
    #[serde(default)]
    pub contrarian_signal: bool,
    /// 反向信号触发理由
    #[serde(default)]
    pub contrarian_reason: Option<String>,
    /// 布林带 + MACD 共振信号（4 条规则：变盘/抄底/减仓/主升浪）
    #[serde(default)]
    pub boll_macd: Option<crate::strategy::BollMacdSignal>,
    /// 模拟持仓买入价格
    pub position_buy_price: Option<f64>,
    /// 模拟持仓买入日期
    pub position_buy_date: Option<String>,
    /// 模拟持仓收益率（%）
    pub position_return: Option<f64>,
    /// 模拟持仓数量（股）
    pub position_quantity: Option<i32>,
    /// 持仓状态："open" 持有中 / "closed" 已卖出 / "new" 本次新买入
    pub position_status: Option<String>,
    /// 卖出价格（仅 closed 时有值）
    pub position_sell_price: Option<f64>,
    /// 卖出日期（仅 closed 时有值）
    pub position_sell_date: Option<String>,
    /// 真实主力资金流 + 日内分时 + 龙虎榜席位（已渲染的 Markdown 片段，可直接插入通知）
    #[serde(default)]
    pub money_flow_section: Option<String>,
    /// 行业横向对标（已渲染的 Markdown 片段，可直接插入通知）
    #[serde(default)]
    pub industry_section: Option<String>,
    /// 财务质量评估（已渲染的 Markdown 片段）
    #[serde(default)]
    pub quality_section: Option<String>,
    /// 估值历史分位（已渲染的 Markdown 片段）
    #[serde(default)]
    pub valuation_history_section: Option<String>,
    /// 卖方一致预期（已渲染的 Markdown 片段）
    #[serde(default)]
    pub consensus_section: Option<String>,
    /// 多期财务趋势（已渲染的 Markdown 片段）
    #[serde(default)]
    pub fin_history_section: Option<String>,
    /// 多维评分（5 个独立维度，0~100）
    #[serde(default)]
    pub score_breakdown: Option<ScoreBreakdown>,
    /// 多维评分渲染片段
    #[serde(default)]
    pub score_breakdown_section: Option<String>,
    /// 风险否决信号（已渲染片段）
    #[serde(default)]
    pub veto_section: Option<String>,
    /// 触发的否决规则名（用于 DB 持久化）
    #[serde(default)]
    pub veto_flags: Option<Vec<String>>,
    /// 原始（未被否决降级）的操作建议
    #[serde(default)]
    pub original_advice: Option<String>,
    /// Phase 2: 交易类型标签（动量交易型/逆向价值型/趋势跟随型/综合配置型）
    #[serde(default)]
    pub trade_type: Option<String>,
    /// Phase 3: 原始资金流时序（仅运行时使用，不持久化）
    #[serde(default, skip_serializing)]
    pub money_flow: Option<crate::data_provider::money_flow::MoneyFlowSummary>,
    /// 深度研判复用种子：携带主流程已抓取的 K线/资金/新闻/财务 + 趋势快照，
    /// 供重点股多智能体深度分析复用，避免重复抓取。仅运行时使用，不持久化。
    #[serde(default, skip)]
    pub deep_seed: Option<crate::deep_analyzer::DeepAnalysisSeed>,
}

impl ScoreDisplay for AnalysisResult {
    fn sentiment_score(&self) -> i32 {
        self.sentiment_score
    }
    fn operation_advice(&self) -> &str {
        &self.operation_advice
    }
}

impl AnalysisResult {
    /// 获取情绪 emoji（委托给 `ScoreDisplay::score_emoji`）。
    ///
    /// 保留此方法以兼容所有调用点（`result.get_emoji()`），
    /// 内部实现统一由 `traits::ScoreDisplay` 维护。
    pub fn get_emoji(&self) -> &'static str {
        self.score_emoji()
    }
}

/// 股票分析流程配置
#[derive(Clone)]
pub struct PipelineConfig {
    /// 最大并发数
    pub max_workers: usize,
    /// 是否跳过分析
    pub dry_run: bool,
    /// 是否发送通知
    pub send_notification: bool,
    /// 单股推送模式
    pub single_notify: bool,
    /// 实时行情新鲜度阈值（秒）
    pub dq_quote_stale_sec: u64,
    /// 持仓/资金新鲜度阈值（秒）
    pub dq_position_stale_sec: u64,
    /// 净值新鲜度阈值（秒）
    pub dq_nav_stale_sec: u64,
    /// 日线/历史数据新鲜度阈值（秒）
    pub dq_daily_stale_sec: u64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_workers: 3,
            dry_run: false,
            send_notification: true,
            single_notify: false,
            dq_quote_stale_sec: 5,
            dq_position_stale_sec: 30,
            dq_nav_stale_sec: 24 * 3600,
            dq_daily_stale_sec: 24 * 3600,
        }
    }
}

/// 股票分析流程调度器
pub struct AnalysisPipeline {
    data_manager: Arc<DataFetcherManager>,
    trend_analyzer: Arc<StockTrendAnalyzer>,
    ai_analyzer: Option<GeminiAnalyzer>,
    use_news_search: bool, // 是否使用新闻搜索
    notifier: Arc<NotificationService>,
    config: PipelineConfig,
    /// 当日涨停股票代码集合
    limit_up_codes: Arc<std::collections::HashSet<String>>,
    /// Test/live isolation (2.5): deterministic validated context exists only in test builds.
    #[cfg(test)]
    test_resolved_context: Option<TestResolvedAnalysisContext>,
    /// Test/live isolation (2.5): injected K-lines never exist in production builds.
    #[cfg(test)]
    test_fetched_data: Option<std::result::Result<Vec<crate::data_provider::KlineData>, String>>,
}

#[cfg(test)]
#[derive(Clone)]
struct TestResolvedAnalysisContext {
    stock_name: String,
    news_context: Option<String>,
    extra: std::result::Result<extra_context::ExtraContext, String>,
    mtf_section: std::result::Result<Option<String>, String>,
}

/// 评分 → 操作建议（系统与 AI 共用同一档位表，避免两套标准）
///
/// 修复 Top10#2 (2026-06-29 IC/IR 报告): IC=-0.0775, sentiment_score 方向性反转,
/// 高分票跑输统计基准 (≥80 胜率 40.9% vs <40 胜率 56.2%).
/// 禁用 sentiment_score 参与推送排序, 改为全权依赖布林+MACD 信号.
/// 保留 AI 文本分析信息整理功能, 不在归档建议中显示 AI 驱动建议.
fn score_to_advice(score: i32) -> &'static str {
    match score {
        80..=100 => "技术面偏多 | (AI评分已禁用,见IC/IR报告)",
        60..=79 => "技术面中性 | (AI评分已禁用)",
        40..=59 => "观望",
        20..=39 => "技术面偏空",
        _ => "建议卖出",
    }
}

/// 判定是否为「重点股」并给出深度研判优先级（None 表示非重点，维持标准分析）。
/// 触发条件：风险否决 / 评分极端（≥75 或 ≤25）/ 反向择时信号 / 当日涨停。
/// 数值越大越优先消耗深度研判预算。
fn key_stock_priority(r: &AnalysisResult) -> Option<i32> {
    let mut p = 0;
    let mut hit = false;
    if r.veto_flags
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        p += 100;
        hit = true;
    }
    if r.ranking_score >= 75 {
        p += 80;
        hit = true;
    }
    if r.ranking_score <= 25 {
        p += 70;
        hit = true;
    }
    if r.contrarian_signal {
        p += 60;
        hit = true;
    }
    if r.is_limit_up {
        p += 40;
        hit = true;
    }
    if hit {
        Some(p)
    } else {
        None
    }
}

impl AnalysisPipeline {
    /// 创建新的分析流程
    pub fn new(config: PipelineConfig) -> Result<Self> {
        let data_manager = Arc::new(DataFetcherManager::new()?);
        let trend_analyzer = Arc::new(StockTrendAnalyzer::new());
        let notifier = Arc::new(NotificationService::from_env());

        // 输出并发配置
        info!("配置并发线程数: {}", config.max_workers);

        // 初始化AI分析器（如果有配置）
        let ai_analyzer = std::env::var("GEMINI_API_KEY").ok().and_then(|key| {
            if !key.is_empty() {
                Some(GeminiAnalyzer::from_env())
            } else {
                None
            }
        });

        // 初始化搜索服务（如果有配置）
        let search_service = get_search_service();
        let use_news_search = search_service.is_available();
        if use_news_search {
            info!("✓ 新闻搜索功能已启用");
        } else {
            info!("✗ 未配置搜索API Key，新闻搜索功能不可用");
        }

        Ok(Self {
            data_manager,
            trend_analyzer,
            ai_analyzer,
            use_news_search,
            notifier,
            config,
            limit_up_codes: Arc::new(std::collections::HashSet::new()),
            #[cfg(test)]
            test_resolved_context: None,
            #[cfg(test)]
            test_fetched_data: None,
        })
    }

    /// 设置当日涨停股票代码集合
    pub fn with_limit_up_codes(mut self, codes: std::collections::HashSet<String>) -> Self {
        self.limit_up_codes = Arc::new(codes);
        self
    }

    // fetch_and_save_data 已抽到 pipeline/data.rs (修复 Top10#3+#4 audit, 1765→600 行)

    // analyze_stock (1020 行) 已抽到 pipeline/analyze.rs (修复 Top10#3+#4)

    /// 对重点股运行机构级多智能体深度研判，并把结果合并进 `analysis_summary`，
    /// 使最终通知报告即体现深度内容。顺序执行以避免多智能体并发打爆 API 限流；
    /// 单只失败/超时仅保留标准分析，不影响整体流程。
    async fn enrich_key_stocks_with_deep_analysis(&self, results: &mut [AnalysisResult]) {
        let max = std::env::var("DEEP_ANALYSIS_MAX")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(15);
        if max == 0 {
            return;
        }

        // 选出重点股索引并按优先级降序排序（优先级高者优先消耗预算）
        let mut candidates: Vec<(usize, i32)> = results
            .iter()
            .enumerate()
            .filter_map(|(i, r)| key_stock_priority(r).map(|p| (i, p)))
            .collect();
        if candidates.is_empty() {
            info!("[深度研判] 无重点股命中，跳过");
            return;
        }
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.1));
        candidates.truncate(max);

        info!(
            "[深度研判] 命中重点股 {} 只（上限 {}）",
            candidates.len(),
            max
        );

        // 深度研判并发度（LLM 密集，默认 3，单独控制避免叠加放大限流）
        let concurrency = std::env::var("DEEP_ANALYSIS_CONCURRENCY")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&c| c > 0)
            .unwrap_or(3);

        // 并行跑多智能体分析，仅借用 self（不可变），结果回收后再写回 results
        let deep_outputs: Vec<(usize, Option<String>)> = stream::iter(candidates)
            .map(|(idx, _)| {
                let code = results[idx].code.clone();
                let name = results[idx].name.clone();
                let seed_opt = results[idx].deep_seed.clone();
                async move {
                    info!("[深度研判] ▶ {} {}", code, name);
                    // 优先复用主流程数据种子（避免重复抓取）；缺失时回退到现抓路径。
                    let deep = match &seed_opt {
                        Some(seed) => {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(300),
                                crate::deep_analyzer::run_multi_agent_analysis_with_seed(seed),
                            )
                            .await
                        }
                        None => {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(300),
                                crate::deep_analyzer::run_multi_agent_analysis(&code),
                            )
                            .await
                        }
                    };
                    let md = match deep {
                        Ok(Ok(md)) if !md.trim().is_empty() => Some(md),
                        Ok(Ok(_)) => {
                            warn!("[深度研判] {} 返回空，保留标准分析", code);
                            None
                        }
                        Ok(Err(e)) => {
                            warn!("[深度研判] {} 失败，保留标准分析: {:#}", code, e);
                            None
                        }
                        Err(_) => {
                            warn!("[深度研判] {} 超时(300s)，保留标准分析", code);
                            None
                        }
                    };
                    (idx, md)
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        // 顺序写回（落盘 + 合并），避免并发可变借用
        for (idx, md) in deep_outputs {
            let Some(md) = md else { continue };
            let code = results[idx].code.clone();
            results[idx].analysis_summary =
                self::section_utils::merge_deep_analysis(&results[idx].analysis_summary, &md);
            if let Err(e) =
                self::section_utils::save_deep_report(&code, &results[idx].analysis_summary)
            {
                warn!("[深度研判] {} 落盘失败: {}", code, e);
            }
            info!("[深度研判] ✓ {} 已合并进报告", code);
        }
    }

    pub async fn run(
        &self,
        stock_codes: &[String],
        prefetched_macro: Option<String>,
    ) -> Result<Vec<AnalysisResult>> {
        if stock_codes.is_empty() {
            warn!("股票列表为空");
            return Ok(Vec::new());
        }

        info!("===== 开始分析 {} 只股票 =====", stock_codes.len());
        info!("股票列表: {:?}", stock_codes);
        info!(
            "模式: {}",
            if self.config.dry_run {
                "仅获取数据"
            } else {
                "完整分析"
            }
        );

        if self.config.single_notify {
            info!("已启用单股推送模式：每分析完一只股票立即推送");
        }

        let start = std::time::Instant::now();

        // 并发处理股票（使用配置的最大并发数）
        info!("使用 {} 个并发任务处理股票", self.config.max_workers);
        let macro_context =
            macro_news::resolve_macro_context(prefetched_macro, self.use_news_search).await;

        info!(
            "📋 分析股票列表（{} 只）: {:?}",
            stock_codes.len(),
            stock_codes
        );
        let mut results: Vec<AnalysisResult> = stream::iter(stock_codes.iter())
            .map(|code| self.process_stock(code.clone(), macro_context.clone()))
            .buffer_unordered(self.config.max_workers)
            .filter_map(|result| async { result })
            .collect()
            .await;

        let elapsed = start.elapsed();
        let success = results.len();
        let failed = stock_codes.len() - success;

        info!("===== 分析完成 =====");
        info!(
            "成功: {}, 失败: {}, 耗时: {:.2}秒",
            success,
            failed,
            elapsed.as_secs_f32()
        );

        // ===== 重点股机构级深度研判（多智能体）=====
        // 仅对评分极端 / 反向信号 / 风险否决 / 涨停 等重点股启用，结果合并进
        // analysis_summary，使最终通知报告即体现深度研判内容；标准股维持现状。
        if !self.config.dry_run && self.ai_analyzer.is_some() {
            self.enrich_key_stocks_with_deep_analysis(&mut results)
                .await;
        }

        // 运行多因子回测
        let backtest_summary = if !results.is_empty() && !self.config.dry_run {
            info!("===== 开始多因子回测 =====");
            match self.run_multi_factor_backtest(&results).await {
                Ok(summary) => {
                    info!(
                        "回测完成: 总收益 {:.2}%, 年化收益 {:.2}%, 最大回撤 {:.2}%, 夏普比率 {:.2}",
                        summary.total_return * 100.0,
                        summary.annual_return * 100.0,
                        summary.max_drawdown * 100.0,
                        summary.sharpe_ratio
                    );
                    Some(summary)
                }
                Err(e) => {
                    error!("回测失败: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // 布林带 / RSI 回测共享同一份 top-3 长周期历史K线，只抓一次（避免重复重型抓取）
        let backtest_history = if !results.is_empty() && !self.config.dry_run {
            self.fetch_top_backtest_history(&results, 3, 7000).await
        } else {
            Vec::new()
        };

        // 运行布林带+Z-Score 均值回归回测
        if !backtest_history.is_empty() {
            info!("===== 开始布林带+Z-Score 均值回归回测 =====");
            match self.run_bollinger_zscore_backtest(&backtest_history).await {
                Ok(summary) => {
                    info!("布林带回测完成: 总收益 {:.2}%, 年化收益 {:.2}%, 最大回撤 {:.2}%, 夏普比率 {:.2}",
                        summary.total_return * 100.0,
                        summary.annual_return * 100.0,
                        summary.max_drawdown * 100.0,
                        summary.sharpe_ratio
                    );
                }
                Err(e) => {
                    error!("布林带回测失败: {}", e);
                }
            }
        }

        // 运行 RSI 超买超卖策略回测
        if !backtest_history.is_empty() {
            info!("===== 开始 RSI 超买超卖策略回测 =====");
            match self.run_rsi_backtest(&backtest_history).await {
                Ok(summary) => {
                    info!("RSI 回测完成: 总收益 {:.2}%, 年化收益 {:.2}%, 最大回撤 {:.2}%, 夏普比率 {:.2}",
                        summary.total_return * 100.0,
                        summary.annual_return * 100.0,
                        summary.max_drawdown * 100.0,
                        summary.sharpe_ratio
                    );
                }
                Err(e) => {
                    error!("RSI 回测失败: {}", e);
                }
            }
        }

        // 发送汇总通知
        if !results.is_empty()
            && self.config.send_notification
            && !self.config.dry_run
            && !self.config.single_notify
        {
            // 产业链联动分析：仅在当日有涨停数据时执行，作为报告第一部分
            // MarketAnalyzer 使用阻塞 HTTP，必须在 spawn_blocking 中执行
            let chain_section = {
                let limit_up_stocks =
                    tokio::task::spawn_blocking(
                        || match crate::market_analyzer::MarketAnalyzer::new(None) {
                            Ok(analyzer) => match analyzer.get_limit_up_stocks() {
                                Ok(stocks) => stocks,
                                Err(e) => {
                                    log::warn!("[产业链] 获取涨停股列表失败: {}", e);
                                    Vec::new()
                                }
                            },
                            Err(e) => {
                                log::warn!("[产业链] 创建 MarketAnalyzer 失败: {}", e);
                                Vec::new()
                            }
                        },
                    )
                    .await
                    .unwrap_or_default();
                if limit_up_stocks.is_empty() {
                    info!("[产业链] 今日无涨停数据，跳过产业链分析");
                    None
                } else {
                    info!(
                        "[产业链] 获取到 {} 只涨停股，开始联动分析...",
                        limit_up_stocks.len()
                    );
                    match chain_analysis::run_chain_analysis(limit_up_stocks, None).await {
                        Ok(report) if !report.trim().is_empty() => {
                            info!("[产业链] 联动分析完成，将并入主报告");
                            Some(report)
                        }
                        Ok(_) => {
                            warn!("[产业链] 联动分析返回空，跳过");
                            None
                        }
                        Err(e) => {
                            warn!("[产业链] 联动分析失败: {}", e);
                            None
                        }
                    }
                }
            };

            // BR-122 大盘状态门控：普跌日豁免跑赢指数个股的机械减仓建议，并在日报头部输出市场定性
            let regime_section = market_regime::apply(&self.data_manager, &mut results)
                .map_err(anyhow::Error::msg)?;
            summary_notify::send_summary_notification(
                &self.notifier,
                &results,
                backtest_summary.as_ref(),
                regime_section.as_deref(),
                chain_section.as_deref(),
            )
            .await?;
        }

        Ok(results)
    }

    /// 生成单股报告
    fn generate_single_report(&self, result: &AnalysisResult) -> String {
        reporting::generate_single_report(result)
    }
}

#[cfg(test)]
mod tests {
    use super::section_utils::normalize_ai_sections;
    use super::{
        key_stock_priority, score_to_advice, AnalysisPipeline, AnalysisResult, PipelineConfig,
    };
    use crate::data_provider::{AdjustType, KlineData};
    use crate::notification::{NotificationConfig, NotificationService};
    use crate::traits::ScoreDisplay;
    use chrono::NaiveDate;
    use serial_test::serial;
    use std::sync::Arc;

    fn result() -> AnalysisResult {
        serde_json::from_value(serde_json::json!({
            "code": "TEST_CODE_000001",
            "name": "TEST_CODE_示例",
            "sentiment_score": 50,
            "ranking_score": 50,
            "operation_advice": "观望",
            "trend_prediction": "盘整",
            "analysis_summary": "TEST_CODE_分析正文",
            "is_limit_up": false,
            "contrarian_signal": false
        }))
        .expect("valid public AnalysisResult fixture")
    }

    fn kline() -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid fixture date"),
            open: 10.0,
            high: 10.5,
            low: 9.5,
            close: 10.0,
            volume: 1_000.0,
            amount: 10_000.0,
            pct_chg: 1.0,
            intraday_price: None,
            settled: true,
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
            adjust: AdjustType::None,
        }
    }

    #[test]
    fn advice_and_key_stock_priority_cover_all_registered_bands() {
        for (score, expected) in [
            (100, "技术面偏多"),
            (80, "技术面偏多"),
            (79, "技术面中性"),
            (60, "技术面中性"),
            (59, "观望"),
            (40, "观望"),
            (39, "技术面偏空"),
            (20, "技术面偏空"),
            (19, "建议卖出"),
        ] {
            assert!(score_to_advice(score).starts_with(expected));
        }

        let neutral = result();
        assert_eq!(key_stock_priority(&neutral), None);
        let mut low = neutral.clone();
        low.ranking_score = 25;
        assert_eq!(key_stock_priority(&low), Some(70));
        let mut combined = neutral;
        combined.ranking_score = 75;
        combined.veto_flags = Some(vec!["TEST_CODE_否决".to_string()]);
        combined.contrarian_signal = true;
        combined.is_limit_up = true;
        assert_eq!(key_stock_priority(&combined), Some(280));
        combined.veto_flags = Some(Vec::new());
        assert_eq!(key_stock_priority(&combined), Some(180));
    }

    #[tokio::test]
    async fn pipeline_constructor_empty_run_and_dry_run_are_side_effect_free() {
        let config = PipelineConfig {
            max_workers: 1,
            dry_run: true,
            send_notification: false,
            single_notify: false,
            ..Default::default()
        };
        let mut pipeline = AnalysisPipeline::new(config.clone()).expect("pipeline constructor");
        pipeline.ai_analyzer = None;
        pipeline.use_news_search = false;
        pipeline.notifier = Arc::new(NotificationService::new(NotificationConfig::default()));
        pipeline =
            pipeline.with_limit_up_codes(["TEST_CODE_000001".to_string()].into_iter().collect());
        assert!(pipeline.limit_up_codes.contains("TEST_CODE_000001"));
        assert_eq!(pipeline.config.dq_quote_stale_sec, 5);
        assert_eq!(pipeline.config.dq_position_stale_sec, 30);
        assert_eq!(pipeline.config.dq_nav_stale_sec, 24 * 3600);
        assert_eq!(pipeline.config.dq_daily_stale_sec, 24 * 3600);

        assert!(pipeline
            .run(&[], Some("TEST_CODE_宏观证据".to_string()))
            .await
            .expect("empty run")
            .is_empty());

        pipeline.test_fetched_data = Some(Ok(vec![kline()]));
        let results = pipeline
            .run(
                &["TEST_CODE_000001".to_string()],
                Some("TEST_CODE_宏观证据".to_string()),
            )
            .await
            .expect("dry run");
        assert!(results.is_empty());

        let report = pipeline.generate_single_report(&result());
        assert!(report.contains("TEST_CODE_示例(TEST_CODE_000001)"));
        assert!(report.contains("TEST_CODE_分析正文"));
    }

    #[tokio::test]
    #[serial(deep_env)]
    async fn deep_enrichment_guards_do_not_start_external_analysis() {
        let config = PipelineConfig {
            max_workers: 1,
            dry_run: false,
            send_notification: false,
            single_notify: false,
            ..Default::default()
        };
        let pipeline = AnalysisPipeline::new(config).expect("pipeline constructor");

        let previous = std::env::var("DEEP_ANALYSIS_MAX").ok();
        let previous_concurrency = std::env::var("DEEP_ANALYSIS_CONCURRENCY").ok();
        std::env::remove_var("DEEP_ANALYSIS_MAX");
        let mut empty = Vec::new();
        pipeline
            .enrich_key_stocks_with_deep_analysis(&mut empty)
            .await;

        std::env::set_var("DEEP_ANALYSIS_MAX", "0");
        let mut guarded = vec![result()];
        guarded[0].ranking_score = 100;
        pipeline
            .enrich_key_stocks_with_deep_analysis(&mut guarded)
            .await;

        std::env::set_var("DEEP_ANALYSIS_MAX", "1");
        std::env::set_var("DEEP_ANALYSIS_CONCURRENCY", "1");
        guarded[0].deep_seed = Some(crate::deep_analyzer::DeepAnalysisSeed {
            code: "TEST_CODE_000001".to_string(),
            name: "TEST_CODE_示例".to_string(),
            kline: Arc::new(Vec::new()),
            extra_context: None,
            news_context: None,
            macro_context: None,
            fundamental_ctx: None,
            trend_snapshot: crate::deep_analyzer::TrendSnapshot {
                trend_status: "TEST_CODE_缺失批次".to_string(),
                ma_alignment: "TEST_CODE_缺失".to_string(),
                trend_strength: 0.0,
                bias_ma5: 0.0,
                volume_status: "TEST_CODE_缺失".to_string(),
                volume_ratio_5d: 0.0,
                support_levels: Vec::new(),
                resistance_levels: Vec::new(),
                evidence_reasons: Vec::new(),
                risk_factors: Vec::new(),
            },
        });
        pipeline
            .enrich_key_stocks_with_deep_analysis(&mut guarded)
            .await;
        if let Some(value) = previous {
            std::env::set_var("DEEP_ANALYSIS_MAX", value);
        } else {
            std::env::remove_var("DEEP_ANALYSIS_MAX");
        }
        if let Some(value) = previous_concurrency {
            std::env::set_var("DEEP_ANALYSIS_CONCURRENCY", value);
        } else {
            std::env::remove_var("DEEP_ANALYSIS_CONCURRENCY");
        }
        assert_eq!(guarded[0].analysis_summary, "TEST_CODE_分析正文");
    }

    #[test]
    fn analysis_result_score_display_delegation_is_stable() {
        let value = result();
        assert_eq!(value.sentiment_score(), 50);
        assert_eq!(value.operation_advice(), "观望");
        assert_eq!(value.get_emoji(), value.score_emoji());
    }

    #[test]
    fn normalize_bare_headings_into_brackets() {
        let input = "## 宏观影响\n内容1\n## 消息面\n内容2\n";
        let got = normalize_ai_sections(input);
        assert!(got.contains("## 【宏观影响】"), "got: {got}");
        assert!(got.contains("## 【消息面】"), "got: {got}");
    }

    #[test]
    fn normalize_preserves_already_bracketed() {
        let input = "## 【宏观影响】\n内容\n";
        let got = normalize_ai_sections(input);
        assert_eq!(got, input);
    }

    #[test]
    fn normalize_emoji_prefixed_title() {
        let input = "## ⚠️ 逆势布局逻辑\nabc\n";
        let got = normalize_ai_sections(input);
        assert!(got.contains("## 【⚠️ 逆势布局逻辑】"), "got: {got}");
    }

    #[test]
    fn normalize_action_advice_with_suffix() {
        let input = "## 操作建议（含买入价/目标价/止损位）\nxx\n";
        let got = normalize_ai_sections(input);
        assert!(
            got.contains("## 【操作建议（含买入价/目标价/止损位）】"),
            "got: {got}"
        );
    }

    #[test]
    fn normalize_adds_hash_prefix_for_bare_bracket_line() {
        let input = "【消息面】\n内容\n";
        let got = normalize_ai_sections(input);
        assert!(got.starts_with("## 【消息面】"), "got: {got}");
    }

    #[test]
    fn normalize_dedupes_repeated_header_with_inline_body() {
        // 模型把标签既单独成行、又内嵌到正文行首，导致标题重复且正文被当作标题渲染
        let input = "## 【宏观影响】\n## 【宏观影响】地缘政治紧张，影响中性偏空。\n";
        let got = normalize_ai_sections(input);
        // 只保留一个标题
        assert_eq!(got.matches("## 【宏观影响】").count(), 1, "got: {got}");
        // 正文被还原为普通段落（不带 ## 前缀）
        assert!(got.contains("\n地缘政治紧张，影响中性偏空。"), "got: {got}");
    }

    #[test]
    fn normalize_dedupes_bare_label_then_inline_body() {
        let input = "【主力资金】\n【主力资金】今日主力净流出 0.03 亿元。\n";
        let got = normalize_ai_sections(input);
        assert_eq!(got.matches("## 【主力资金】").count(), 1, "got: {got}");
        assert!(got.contains("\n今日主力净流出 0.03 亿元。"), "got: {got}");
    }
}

// 修复 Top10#3+#4 (2026-06-29 audit): pipeline/mod.rs (1765 行) 拆 4 个子模块
// 数据获取/持久化 → data.rs (52 行)
mod data;

// 修复 Top10#3+#4: analyze_stock (1020 行) 已抽到 pipeline/analyze.rs

// 修复 Top10#3+#4: analyze_stock (1020 行) 已抽到 pipeline/analyze.rs
mod analyze;
