//! 股票分析流程调度器
//!
//! 负责协调各模块完成完整的分析流程：
//! 数据获取 → 趋势分析 → AI分析 → 通知推送

mod backtest_runner;
mod extra_context;
mod macro_news;
mod position_tracker;
mod price_stats;
mod reporting;
mod summary_notify;
mod technical_report;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::{DataFetcherManager, KlineData};
use crate::search_service::get_search_service;
use crate::database::DatabaseManager;
use crate::notification::NotificationService;
use crate::trend_analyzer::StockTrendAnalyzer;
use crate::traits::ScoreDisplay;

/// 股票综合分析结果
///
/// 由 `AnalysisPipeline` 生成，贯穿整个通知与报告流程。
/// `notification` 模块直接使用此类型，无需额外的转换结构体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub code: String,
    pub name: String,
    pub sentiment_score: i32,
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
}

impl ScoreDisplay for AnalysisResult {
    fn sentiment_score(&self) -> i32 { self.sentiment_score }
    fn operation_advice(&self) -> &str { &self.operation_advice }
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
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_workers: 3,
            dry_run: false,
            send_notification: true,
            single_notify: false,
        }
    }
}

/// 股票分析流程调度器
pub struct AnalysisPipeline {
    data_manager: Arc<DataFetcherManager>,
    trend_analyzer: Arc<StockTrendAnalyzer>,
    ai_analyzer: Option<Arc<GeminiAnalyzer>>,
    use_news_search: bool, // 是否使用新闻搜索
    notifier: Arc<NotificationService>,
    config: PipelineConfig,
    /// 当日涨停股票代码集合
    limit_up_codes: Arc<std::collections::HashSet<String>>,
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
                Some(Arc::new(GeminiAnalyzer::from_env()))
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
        })
    }

    /// 设置当日涨停股票代码集合
    pub fn with_limit_up_codes(mut self, codes: std::collections::HashSet<String>) -> Self {
        self.limit_up_codes = Arc::new(codes);
        self
    }

    /// 获取并保存单只股票数据
    async fn fetch_and_save_data(&self, code: &str) -> Result<Vec<KlineData>> {
        info!("[{}] 开始获取数据...", code);

        // 从数据源获取数据
        // 使用 spawn_blocking 将同步 TCP/HTTP 调用放到独立的阻塞线程池，
        // 不占用 tokio worker 线程，避免饿死异步任务（timeout/新闻搜索/AI 调用）。
        let dm = self.data_manager.clone();
        let code_owned = code.to_string();
        let (data, source) = tokio::task::spawn_blocking(move || {
            dm.get_daily_data(&code_owned, 30)
        }).await.context("spawn_blocking panicked")?.context("获取数据失败")?;

        if data.is_empty() {
            warn!("[{}] 获取到的数据为空", code);
            return Ok(data);
        }

        info!("[{}] 从 {} 获取到 {} 条数据", code, source, data.len());

        // 保存到数据库
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            match db.save_kline_data(code, &data, &source) {
                Ok(count) => info!("[{}] 已保存 {} 条K线数据到数据库", code, count),
                Err(e) => warn!("[{}] 保存K线数据到数据库失败: {}", code, e),
            }
        }

        Ok(data)
    }

    /// 分析单只股票
    async fn analyze_stock(&self, code: &str, data: &[KlineData], macro_context: Option<&str>) -> Result<AnalysisResult> {
        info!("[{}] 开始分析...", code);

        if data.is_empty() {
            return Err(anyhow::anyhow!("数据为空"));
        }

        // 1. 趋势分析（夏普比率从最新 K 线取，不在 trend_analyzer 里重复算）
        let sharpe_ratio = data.first().and_then(|d| d.sharpe_ratio);
        let mut trend_result = self.trend_analyzer.analyze_with_kline(data, code);
        trend_result.sharpe_ratio = sharpe_ratio;

        // 1.5 布林带 + MACD 共振信号（4 条核心规则 + 反误区过滤）
        // 把信号加成纳入 signal_score，并在评分理由/风险因素里记一笔
        let bm = crate::strategy::detect_boll_macd_signal(data);
        if bm.action != crate::strategy::BollMacdAction::None {
            use crate::strategy::BollMacdAction;
            let (delta, is_reason) = match bm.action {
                BollMacdAction::UptrendStart => (12, true),  // 主升浪启动：强买
                BollMacdAction::BottomBuy => (10, true),     // 下轨抄底：反转
                BollMacdAction::PreReversal => (3, true),    // 准备变盘：中性提示
                BollMacdAction::TopSell => (-15, false),     // 顶部减仓：强压评分
                BollMacdAction::None => (0, true),
            };
            trend_result.signal_score = (trend_result.signal_score + delta).clamp(0, 100);
            let line = format!("📊 BB+MACD: {} | {} ({:+})", bm.action.name(), bm.reason, delta);
            if is_reason {
                trend_result.signal_reasons.push(line);
            } else {
                trend_result.risk_factors.push(line);
            }
            // 评分跌破 65 分时降级买入信号（避免顶部 TopSell 仍报"买入"）
            if matches!(bm.action, BollMacdAction::TopSell) {
                use crate::trend_analyzer::BuySignal;
                if matches!(trend_result.buy_signal, BuySignal::StrongBuy | BuySignal::Buy) {
                    trend_result.buy_signal = BuySignal::Hold;
                }
            }
            info!("[{}] 📊 布林+MACD 信号: {} | {} | 评分调整 {:+}", code, bm.action.name(), bm.reason, delta);
        }

        info!(
            "[{}] 趋势: {}, 买入信号: {}, 评分: {}",
            code, trend_result.trend_status, trend_result.buy_signal, trend_result.signal_score
        );

        // 2. 技术分析 Markdown
        let mut analysis_content = technical_report::build_technical_markdown(&trend_result);

        // 3. 获取股票名称（同步 HTTP，放 blocking 线程池）
        let dm = self.data_manager.clone();
        let code_owned = code.to_string();
        let stock_name = tokio::task::spawn_blocking(move || dm.get_stock_name(&code_owned))
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| format!("股票{}", code));

        info!("[{}] 搜索最新新闻...", code);
        let news_context = if self.use_news_search {
            let search_service = get_search_service();
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                search_service.search_stock_news(code, &stock_name, 3),
            )
            .await
            {
                Ok(response) => {
                    if response.success && !response.results.is_empty() {
                        info!("[{}] 获取到 {} 条新闻", code, response.results.len());
                        Some(response.to_context(3))
                    } else {
                        warn!("[{}] 新闻搜索未找到结果", code);
                        None
                    }
                }
                Err(_) => {
                    warn!("[{}] 新闻搜索超时", code);
                    None
                }
            }
        } else {
            None
        };

        // 4. 真实资金/分时/龙虎榜 + 筹码分布上下文（不管 AI 是否启用，都抓一次给通知展示）
        let extra_context = extra_context::fetch_extra_context(code, data).await;

        // 5. AI 增强分析
        if let Some(ref ai) = self.ai_analyzer {
            match ai
                .analyze_stock_with_extras(
                    code,
                    Some(stock_name.as_str()),
                    data,
                    macro_context,
                    extra_context.as_deref(),
                    news_context.as_deref(),
                )
                .await
            {
                Ok(ai_result) => {
                    analysis_content.push_str("\n# AI分析\n\n");
                    analysis_content.push_str(&ai_result);
                    if let Some(ref news) = news_context {
                        analysis_content.push_str("\n\n# 相关新闻\n\n");
                        analysis_content.push_str(news);
                    }
                }
                Err(e) => warn!("[{}] AI分析失败: {}", code, e),
            }
        } else if let Some(ref news) = news_context {
            analysis_content.push_str("\n# 相关新闻\n\n");
            analysis_content.push_str(news);
        }

        // 6. 操作建议
        let operation_advice = match trend_result.signal_score {
            80..=100 => "强烈建议买入",
            60..=79 => "建议买入",
            40..=59 => "观望",
            20..=39 => "建议减仓",
            _ => "建议卖出",
        }
        .to_string();

        // 7. 价格区间 / 近期统计
        let stats = price_stats::compute_price_stats(data);

        let result = AnalysisResult {
            code: code.to_string(),
            name: stock_name,
            sentiment_score: trend_result.signal_score,
            operation_advice,
            trend_prediction: format!("{}", trend_result.trend_status),
            analysis_summary: analysis_content,
            technical_analysis: None,
            news_summary: None,
            buy_reason: None,
            risk_warning: None,
            ma_analysis: Some(trend_result.ma_alignment.clone()),
            volume_analysis: None,
            pe_ratio: data[0].pe_ratio,
            pb_ratio: data[0].pb_ratio,
            turnover_rate: data[0].turnover_rate,
            market_cap: data[0].market_cap,
            circulating_cap: data[0].circulating_cap,
            current_price: Some(trend_result.current_price),
            ma5: Some(trend_result.ma5),
            ma10: Some(trend_result.ma10),
            ma20: Some(trend_result.ma20),
            ma60: Some(trend_result.ma60),
            ma_alignment: Some(trend_result.ma_alignment.clone()),
            bias_ma5: Some(trend_result.bias_ma5),
            volume_ratio_5d: Some(trend_result.volume_ratio_5d),
            high_52w: stats.high_52w,
            low_52w: stats.low_52w,
            pos_52w: stats.pos_52w,
            high_quarter: stats.high_quarter,
            low_quarter: stats.low_quarter,
            pos_quarter: stats.pos_quarter,
            chg_5d: stats.chg_5d,
            chg_10d: stats.chg_10d,
            volatility: stats.volatility,
            eps: data[0].eps,
            roe: data[0].roe,
            gross_margin: data[0].gross_margin,
            net_margin: data[0].net_margin,
            revenue_yoy: data[0].revenue_yoy,
            net_profit_yoy: data[0].net_profit_yoy,
            sharpe_ratio: trend_result.sharpe_ratio,
            is_limit_up: self.limit_up_codes.contains(code),
            contrarian_signal: false,
            contrarian_reason: None,
            boll_macd: Some(bm),
            position_buy_price: None,
            position_buy_date: None,
            position_return: None,
            position_quantity: None,
            position_status: None,
            position_sell_price: None,
            position_sell_date: None,
            money_flow_section: extra_context,
        };

        Ok(result)
    }

    /// 处理单只股票的完整流程（含 120s 超时保护）
    async fn process_stock(&self, code: String, macro_context: Arc<String>) -> Option<AnalysisResult> {
        let start = std::time::Instant::now();
        info!("========== [{}] 开始处理 ==========", code);

        // 整体超时保护：单只股票最多处理 120 秒，避免任何环节卡死拖垮全局
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.process_stock_inner(code.clone(), macro_context),
        ).await {
            Ok(r) => r,
            Err(_) => {
                error!("[{}] 处理超时（120s），跳过", code);
                None
            }
        };

        let elapsed = start.elapsed();
        match &result {
            Some(r) => info!("[{}] ✓ 处理完成 ({:.1}s)：{} 评分 {}", code, elapsed.as_secs_f32(), r.operation_advice, r.sentiment_score),
            None    => warn!("[{}] ✗ 处理失败或超时 ({:.1}s)", code, elapsed.as_secs_f32()),
        }
        result
    }

    async fn process_stock_inner(&self, code: String, macro_context: Arc<String>) -> Option<AnalysisResult> {
        // 1. 获取数据
        let data = match self.fetch_and_save_data(&code).await {
            Ok(d) => d,
            Err(e) => {
                error!("[{}] 获取数据失败: {}", code, e);
                return None;
            }
        };

        if data.is_empty() {
            warn!("[{}] 数据为空，跳过分析", code);
            return None;
        }

        // 2. 跳过分析（dry-run模式）
        if self.config.dry_run {
            info!("[{}] dry-run模式，跳过分析", code);
            return None;
        }

        // 3. 分析
        let mc = if macro_context.is_empty() { None } else { Some(macro_context.as_str()) };
        let mut result = match self.analyze_stock(&code, &data, mc).await {
            Ok(r) => r,
            Err(e) => {
                error!("[{}] 分析失败: {}", code, e);
                return None;
            }
        };

        info!(
            "[{}] 分析完成: {}, 评分 {}",
            code, result.operation_advice, result.sentiment_score
        );

        // 3.5 反向择时信号：sentiment_score<40 且技术面企稳 → 反向买入信号
        // 基于历史回测：评分<40 区间 T1胜率 56.91% / T5胜率 55.62% / T5均涨 +2.40%，跑赢市场基准
        let contrarian = crate::strategy::detect_contrarian_signal(&data, result.sentiment_score);
        if contrarian.triggered {
            info!("[{}] 🔄 触发反向择时信号 | {}", code, contrarian.reason);
            result.contrarian_signal = true;
            result.contrarian_reason = Some(contrarian.reason);
        }
        // 注：布林+MACD 共振信号已在 analyze_stock 中提前检测并影响 signal_score

        // 4. 模拟持仓跟踪 & 四大铁律（受 POSITION_TRACKING_ENABLED 控制，默认开启）
        let position_tracking_enabled = std::env::var("POSITION_TRACKING_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true);
        if position_tracking_enabled {
            position_tracker::track_position(&code, &data, &mut result);
        }

        // 5. 保存分析结果到数据库
        position_tracker::save_analysis_result(&code, &data, &result);

        // 6. 单股推送（如果启用）
        if self.config.single_notify && self.config.send_notification {
            let report = self.generate_single_report(&result);
            let code_clone = code.clone();
            match self.notifier.send(&report).await {
                Ok(_) => info!("[{}] 单股推送成功", code_clone),
                Err(e) => error!("[{}] 单股推送失败: {}", code_clone, e),
            }
        }

        Some(result)
    }

    /// 运行完整分析流程
    pub async fn run(&self, stock_codes: &[String], prefetched_macro: Option<String>) -> Result<Vec<AnalysisResult>> {
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
        let macro_context = macro_news::resolve_macro_context(prefetched_macro, self.use_news_search).await;

        info!("📋 分析股票列表（{} 只）: {:?}", stock_codes.len(), stock_codes);
        let results: Vec<AnalysisResult> = stream::iter(stock_codes.iter())
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

        // 运行多因子回测
        let backtest_summary = if !results.is_empty() && !self.config.dry_run {
            info!("===== 开始多因子回测 =====");
            match self.run_multi_factor_backtest(&results).await {
                Ok(summary) => {
                    info!("回测完成: 总收益 {:.2}%, 年化收益 {:.2}%, 最大回撤 {:.2}%, 夏普比率 {:.2}",
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

        // 运行布林带+Z-Score 均值回归回测
        if !results.is_empty() && !self.config.dry_run {
            info!("===== 开始布林带+Z-Score 均值回归回测 =====");
            match self.run_bollinger_zscore_backtest(&results).await {
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
        if !results.is_empty() && !self.config.dry_run {
            info!("===== 开始 RSI 超买超卖策略回测 =====");
            match self.run_rsi_backtest(&results).await {
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
            summary_notify::send_summary_notification(&self.notifier, &results, backtest_summary.as_ref()).await?;
        }

        Ok(results)
    }

    /// 生成单股报告
    fn generate_single_report(&self, result: &AnalysisResult) -> String {
        reporting::generate_single_report(result)
    }
}
