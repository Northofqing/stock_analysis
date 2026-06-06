//! 股票分析流程调度器
//!
//! 负责协调各模块完成完整的分析流程：
//! 数据获取 → 趋势分析 → AI分析 → 通知推送

mod backtest_runner;
mod extra_context;
mod macro_news;
mod multi_timeframe;
mod position_tracker;
mod price_stats;
mod reporting;
pub mod score_breakdown;
mod summary_notify;
mod technical_report;
mod trade_type;
mod veto_rules;

pub use score_breakdown::ScoreBreakdown;
pub use veto_rules::VetoOutcome;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::{DataFetcherManager, KlineData};
use crate::data_provider::financials::FinancialPeriod;
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

/// 评分 → 操作建议（系统与 AI 共用同一档位表，避免两套标准）
fn score_to_advice(score: i32) -> &'static str {
    match score {
        80..=100 => "强烈建议买入",
        60..=79 => "建议买入",
        40..=59 => "观望",
        20..=39 => "建议减仓",
        _ => "建议卖出",
    }
}

/// 判定是否为「重点股」并给出深度研判优先级（None 表示非重点，维持标准分析）。
/// 触发条件：风险否决 / 评分极端（≥75 或 ≤25）/ 反向择时信号 / 当日涨停。
/// 数值越大越优先消耗深度研判预算。
fn key_stock_priority(r: &AnalysisResult) -> Option<i32> {
    let mut p = 0;
    let mut hit = false;
    if r.veto_flags.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
        p += 100;
        hit = true;
    }
    if r.sentiment_score >= 75 {
        p += 80;
        hit = true;
    }
    if r.sentiment_score <= 25 {
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

/// 把深度研判 markdown 合并进标准 `analysis_summary`：
/// 保留「# 技术分析」部分，用机构级深度研判替换原「# AI分析」/「# 相关新闻」段
/// （深度研判已自带消息面/板块等维度）。
fn merge_deep_analysis(standard: &str, deep_md: &str) -> String {
    let cut = ["\n# AI分析", "\n# 相关新闻"]
        .iter()
        .filter_map(|m| standard.find(m))
        .min();
    let tech_part = match cut {
        Some(idx) => &standard[..idx],
        None => standard,
    };
    format!(
        "{}\n\n# 🏛️ 机构级深度研判（多智能体）\n\n{}\n",
        tech_part.trim_end(),
        deep_md.trim()
    )
}

/// 深度研判报告落盘备份到 `reports/details/{date}_{code}.md`。
fn save_deep_report(code: &str, content: &str) -> std::io::Result<()> {
    let date = chrono::Local::now().format("%Y%m%d").to_string();
    let dir = std::path::PathBuf::from("reports/details");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{}_{}.md", date, code)), content)
}

/// 规范化 AI 输出的章节标题：统一为 `## 【XX】` 形式。
///
/// 处理两类常见 AI 偏差：
/// 1. 输出 `【XX】` 但忘记加 `##` 前缀；
/// 2. 输出 `## XX`（去掉了书名号），导致与其它股票渲染样式不一致。
///
/// 已知章节：宏观影响 / 消息面 / 技术面 / 主力资金 / 基本面 /
///   操作建议（可带「含买入价/目标价/止损位」后缀）/ 风险提示 / ⚠️ 逆势布局逻辑。
const AI_SECTIONS: &[&str] = &[
    "宏观影响",
    "消息面",
    "技术面",
    "主力资金",
    "基本面",
    "操作建议",
    "风险提示",
    "逆势布局逻辑",
];

/// 尝试把一行解析为 AI 章节标题行。
///
/// 返回 `(canonical, full_name, content)`：
/// - `canonical`：命中的标准章节名（来自 `AI_SECTIONS`，用于去重判断）
/// - `full_name`：标题方括号内的完整文本（保留 emoji / 后缀，如 "操作建议（含买入价…）"）
/// - `content`：与标题写在同一行时，标题之后的正文（可能为空）
fn parse_ai_section_line(trimmed: &str) -> Option<(&'static str, String, String)> {
    let has_hash = trimmed.starts_with('#');
    let title = trimmed.trim_start_matches('#').trim();

    // 形式 A：`【名称】可选正文`
    if let Some(rest) = title.strip_prefix('【') {
        let end = rest.find('】')?;
        let name = rest[..end].trim();
        let content = rest[end + '】'.len_utf8()..].trim();
        for s in AI_SECTIONS {
            if name.contains(s) {
                return Some((s, name.to_string(), content.to_string()));
            }
        }
        return None;
    }

    // 形式 B：`## 名称`（缺少方括号），仅在带 `#` 前缀时才视为标题，避免误伤正文
    if has_hash {
        for s in AI_SECTIONS {
            if title.contains(s) {
                return Some((s, title.to_string(), String::new()));
            }
        }
    }

    None
}

/// 规范化 AI 输出的章节结构：
/// - 统一为 `## 【章节】` 标题
/// - 去重：连续重复的同一章节标题只保留一个（修复模型把标签既单独成行、又内嵌到正文行首导致的重复标题）
/// - 将与标题写在同一行的正文拆分为独立段落，避免正文被当作标题渲染
fn normalize_ai_sections(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut last_section: Option<&'static str> = None;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            out.push('\n');
            continue;
        }

        if let Some((canonical, name, content)) = parse_ai_section_line(trimmed) {
            // 仅当与上一次输出的章节不同才写标题（连续重复标题会被合并）
            if last_section != Some(canonical) {
                if !out.is_empty() && !out.ends_with("\n\n") {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                }
                out.push_str("## 【");
                out.push_str(&name);
                out.push_str("】\n");
                last_section = Some(canonical);
            }
            if !content.is_empty() {
                out.push_str(&content);
                out.push('\n');
            }
            continue;
        }

        out.push_str(raw_line);
        out.push('\n');
    }
    out
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
    async fn analyze_stock(&self, code: &str, data: &[KlineData], kline_arc: Arc<Vec<KlineData>>, macro_context: Option<&str>) -> Result<AnalysisResult> {
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
                // 【核心修正】强制压低总评分，确保 score_to_advice 不会映射为"建议买入"
                if trend_result.signal_score >= 60 {
                    trend_result.signal_score = 55; // 压至“观望”及以下
                }
            }
            info!("[{}] 📊 布林+MACD 信号: {} | {} | 评分调整 {:+}", code, bm.action.name(), bm.reason, delta);
        }

        // 1.6 基本面评分修正（财务质量 + 估值分位）
        //     - 异常评分 ≥60：高风险，-20 并降档
        //     - 异常评分 30~59：中风险，-8 风险提示
        //     - PE 分位 <20%（极低估）：+5
        //     - PE 分位 >80%（极高估）：-8 风险提示且 StrongBuy→Buy
        //     - 高质量盈利（ROE 上行 + 毛利率上行 + CFO/NI≥0.8）：+5
        if let Some(latest) = data.first() {
            use crate::trend_analyzer::BuySignal;
            let mut total_delta: i32 = 0;

            // (a) 财务异常信号
            if let Some(hist) = latest.financials_history.as_ref() {
                if let Some(q) = crate::data_provider::assess_quality(hist) {
                    if q.risk_score >= 60 {
                        total_delta -= 20;
                        let summary = q.flags.first().cloned().unwrap_or_else(|| q.level.to_string());
                        trend_result
                            .risk_factors
                            .push(format!("💣 财务异常高风险(评分{}/100): {}", q.risk_score, summary));
                        if matches!(trend_result.buy_signal, BuySignal::StrongBuy | BuySignal::Buy) {
                            trend_result.buy_signal = BuySignal::Hold;
                        }
                    } else if q.risk_score >= 30 {
                        total_delta -= 8;
                        trend_result
                            .risk_factors
                            .push(format!("⚠️ 财务异常需关注(评分{}/100)", q.risk_score));
                    }
                }

                // (c) 高质量盈利加分：近 4 期 ROE 与毛利率均单调向上 + CFO/NI 均值≥0.8
                let take: Vec<_> = hist.iter().take(4).collect();
                if take.len() >= 3 {
                    let roe_chrono: Vec<f64> = take.iter().rev().filter_map(|p| p.roe).collect();
                    let gm_chrono: Vec<f64> = take.iter().rev().filter_map(|p| p.gross_margin).collect();
                    let cfo_ni: Vec<f64> = take.iter().filter_map(|p| p.cfo_to_ni_ratio()).collect();
                    let roe_up = roe_chrono.len() >= 3
                        && roe_chrono.windows(2).all(|w| w[1] >= w[0] - 0.01);
                    let gm_up = gm_chrono.len() >= 3
                        && gm_chrono.windows(2).all(|w| w[1] >= w[0] - 0.01);
                    let cfo_ok = !cfo_ni.is_empty()
                        && cfo_ni.iter().sum::<f64>() / cfo_ni.len() as f64 >= 0.8;
                    if roe_up && gm_up && cfo_ok {
                        total_delta += 5;
                        trend_result
                            .signal_reasons
                            .push("💎 高质量盈利(ROE/毛利持续上行+CFO健康) +5".to_string());
                    }
                }
            }

            // (b) 估值分位
            if let Some(vh) = latest.valuation_history.as_ref() {
                if vh.sample_days >= 60 {
                    if let Some(pe_pct) = vh.pe_percentile {
                        if pe_pct < 20.0 {
                            total_delta += 5;
                            trend_result.signal_reasons.push(format!(
                                "📉 PE 历史极低估(分位{:.0}%) +5",
                                pe_pct
                            ));
                        } else if pe_pct > 80.0 {
                            total_delta -= 8;
                            trend_result.risk_factors.push(format!(
                                "📈 PE 历史极高估(分位{:.0}%)，回调风险大",
                                pe_pct
                            ));
                            if matches!(trend_result.buy_signal, BuySignal::StrongBuy) {
                                trend_result.buy_signal = BuySignal::Buy;
                            }
                        }
                    }
                }
            }

            // (d) 卖方一致预期
            if let Some(cs) = latest.consensus.as_ref() {
                if cs.broker_count >= 3 {
                    if let Some(bull) = cs.bullish_ratio() {
                        if bull >= 80.0 && cs.broker_count >= 5 {
                            total_delta += 3;
                            trend_result.signal_reasons.push(format!(
                                "🏦 卖方高度一致看多({}家券商, 看多{:.0}%) +3",
                                cs.broker_count, bull
                            ));
                        } else if bull < 30.0 {
                            total_delta -= 5;
                            trend_result.risk_factors.push(format!(
                                "🏦 卖方一致看空(看多仅{:.0}%)",
                                bull
                            ));
                        }
                    }
                    if let Some(up) = cs.upside_pct(latest.close) {
                        if up > 30.0 {
                            total_delta += 3;
                            trend_result.signal_reasons.push(format!(
                                "🎯 目标价均值隐含 {:+.0}% 上行空间 +3",
                                up
                            ));
                        } else if up < -10.0 {
                            total_delta -= 5;
                            trend_result.risk_factors.push(format!(
                                "🎯 现价已高于目标价均值 {:+.0}%",
                                up
                            ));
                        }
                    }
                }
            }

            // (e) 行业横向对标
            if let Some(ib) = latest.industry.as_ref() {
                if ib.peer_count >= 5 {
                    if let Some(p) = ib.roe_percentile {
                        if p >= 80.0 {
                            total_delta += 3;
                            trend_result.signal_reasons.push(format!(
                                "💎 ROE 同业领先(P{:.0}, {} 家同业) +3",
                                p, ib.peer_count
                            ));
                        } else if p <= 20.0 {
                            total_delta -= 3;
                            trend_result.risk_factors.push(format!(
                                "ROE 同业落后(P{:.0})",
                                p
                            ));
                        }
                    }
                    if let Some(p) = ib.pe_percentile {
                        if p <= 20.0 {
                            total_delta += 2;
                            trend_result.signal_reasons.push(format!(
                                "💰 PE 同业偏低(P{:.0}) +2",
                                p
                            ));
                        } else if p >= 80.0 {
                            total_delta -= 3;
                            trend_result.risk_factors.push(format!(
                                "PE 同业偏高(P{:.0})",
                                p
                            ));
                        }
                    }
                    if let Some(p) = ib.growth_percentile {
                        if p >= 80.0 {
                            total_delta += 2;
                            trend_result.signal_reasons.push(format!(
                                "🚀 净利同比同业领先(P{:.0}) +2",
                                p
                            ));
                        } else if p <= 20.0 {
                            total_delta -= 2;
                            trend_result.risk_factors.push(format!(
                                "净利同比同业落后(P{:.0})",
                                p
                            ));
                        }
                    }
                }
            }

            // 总修正限幅 ±25，避免基本面单一维度主导
            let clamped = total_delta.clamp(-25, 25);
            if clamped != 0 {
                trend_result.signal_score =
                    (trend_result.signal_score + clamped).clamp(0, 100);
                info!(
                    "[{}] 🧮 基本面评分修正 {:+} → 总评分 {}",
                    code, clamped, trend_result.signal_score
                );
            }
        }

        // // === 补充风控修正（核心拦截器，解决系统"精神分裂"问题）===
        // // 1. 技术面极其危险的形态拦截：空头排列 / 乖离率极高
        // use crate::trend_analyzer::{TrendStatus, BuySignal};
        // if trend_result.bias_ma5 > 5.0 {
        //     if trend_result.signal_score >= 60 {
        //         trend_result.signal_score = 55;
        //         trend_result.buy_signal = BuySignal::Hold;
        //         trend_result.risk_factors.push("❌ 乖离率超5%有大幅回调风险，严禁追高，强制降级至观望".to_string());
        //         info!("[{}] 触发风控拦截: 乖离率超5%，评分压至55", code);
        //     }
        // }
        // if matches!(trend_result.trend_status, TrendStatus::StrongBear | TrendStatus::Bear) {
        //     if trend_result.signal_score >= 60 {
        //         trend_result.signal_score = 55;
        //         trend_result.buy_signal = BuySignal::Hold;
        //         trend_result.risk_factors.push("❌ 整体处于空头排列，极其弱势，放弃短线博弈避开接飞刀，强制降级至观望".to_string());
        //         info!("[{}] 触发风控拦截: 空头排列，评分压至55", code);
        //     }
        // }

        // // 2. 资金面拦截：主力大幅出逃严重/拉高出货诱多
        // let svc = crate::data_provider::service::service();
        // let flow_arc = svc.get_money_flow(code, 2).await;
        // if let Some(last_day) = flow_arc.days.last() {
        //     // 单日净流出 < -5000 万
        //     if last_day.main_net < -50_000_000.0 {
        //         if trend_result.signal_score >= 60 {
        //             trend_result.signal_score = 55;
        //             trend_result.buy_signal = BuySignal::Hold;
        //             trend_result.risk_factors.push(format!("❌ 主力资金单日大幅流出({:.2}亿)，风险极高，强制取消买入建议", last_day.main_net / 1_0000_0000.0));
        //             info!("[{}] 触发风控拦截: 主力大幅流出，评分压至55", code);
        //         }
        //     }
        //     // 价涨量增但资金大幅流出（诱多）
        //     if last_day.pct_chg > 4.0 && last_day.main_net < -10_000_000.0 {
        //         if trend_result.signal_score >= 60 {
        //             trend_result.signal_score = 55;
        //             trend_result.buy_signal = BuySignal::Hold;
        //             trend_result.risk_factors.push("❌ 股价大涨但主力净流出(典型诱多/拉高出货)，极其凶险，强制取消买入建议".to_string());
        //             info!("[{}] 触发风控拦截: 价涨量缩/背离诱多，评分压至55", code);
        //         }
        //     }
        // }

        // // 3. 基本面拦截：严重亏损且高估或大幅衰退
        // let pe = data[0].pe_ratio.unwrap_or(0.0);
        // let net_profit_yoy = data[0].net_profit_yoy.unwrap_or(0.0);
        // if (pe < 0.0 || pe > 300.0) && net_profit_yoy < -30.0 {
        //     if trend_result.signal_score >= 60 {
        //         trend_result.signal_score = 55;
        //         trend_result.buy_signal = BuySignal::Hold;
        //         trend_result.risk_factors.push("❌ 基本面极度恶化(业绩大幅下滑且估值畸高/亏损)，底线拦截取消买入".to_string());
        //         info!("[{}] 触发风控拦截: 基本面极度恶化，评分压至55", code);
        //     }
        // }

        // info!(
        //     "[{}] 趋势: {}, 买入信号: {}, 评分: {}",
        //     code, trend_result.trend_status, trend_result.buy_signal, trend_result.signal_score
        // );

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
        let extra = extra_context::fetch_extra_context(code, data).await;
        let mut extra_context = extra.section;
        let money_flow_raw = extra.money_flow;

        // 4.5 多周期下钻（Multi-timeframe）：日线买入信号触发时，去 60min/15min 找精准入场点
        let mtf_trigger = {
            use crate::strategy::BollMacdAction;
            use crate::trend_analyzer::BuySignal;
            trend_result.signal_score >= 60
                || matches!(
                    bm.action,
                    BollMacdAction::BottomBuy | BollMacdAction::UptrendStart
                )
                || matches!(trend_result.buy_signal, BuySignal::StrongBuy | BuySignal::Buy)
        };
        if mtf_trigger {
            info!("[{}] 触发多周期下钻（60min/15min 寻找精准入场点）", code);
            if let Some(mtf_section) = multi_timeframe::fetch_multi_timeframe_section(code).await {
                extra_context = match extra_context {
                    Some(mut s) => {
                        s.push_str(&mtf_section);
                        Some(s)
                    }
                    None => Some(mtf_section),
                };
            }
        }

        // 5. 评分→操作建议（与 AI 共用同一档位表）
        let operation_advice = score_to_advice(trend_result.signal_score).to_string();
        let trend_status_str = format!("{}", trend_result.trend_status);

        // ===== Phase 1/2 提前计算：让 AI 在生成分析前就看到五维评分 + 否决信号 + 交易类型 =====
        let sb_inputs = score_breakdown::ScoreInputs {
            sentiment_score: trend_result.signal_score,
            money_flow: money_flow_raw.as_ref(),
            money_flow_section: extra_context.as_deref(),
            volume_ratio_5d: Some(trend_result.volume_ratio_5d),
        };
        let sb_pre = score_breakdown::compute(&sb_inputs, &data[0]);
        let veto_pre = veto_rules::evaluate(&operation_advice, money_flow_raw.as_ref(), &data[0]);
        let trade_type_pre = trade_type::infer_from_breakdown(&sb_pre);
        let empty_veto: Vec<String> = Vec::new();

        let tech_assessment = crate::analyzer::TechAssessment {
            score: trend_result.signal_score,
            advice: &operation_advice,
            reasons: &trend_result.signal_reasons,
            risks: &trend_result.risk_factors,
            trend_status: &trend_status_str,
            score_breakdown: Some(&sb_pre),
            veto_flags: if veto_pre.flags.is_empty() { &empty_veto } else { &veto_pre.flags },
            trade_type: trade_type_pre.as_deref(),
        };

        // 6. AI 增强分析（AI 与评分同一把尺子：评分明细 + 档位规则注入 prompt）
        if let Some(ref ai) = self.ai_analyzer {
            match ai
                .analyze_stock_with_extras(
                    code,
                    Some(stock_name.as_str()),
                    data,
                    macro_context,
                    extra_context.as_deref(),
                    news_context.as_deref(), 
                    Some(&tech_assessment),
                )
                .await
            {
                Ok(ai_result) => {
                    analysis_content.push_str("\n# AI分析\n\n");
                    analysis_content.push_str(&normalize_ai_sections(&ai_result));
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

        // 7. 价格区间 / 近期统计
        let stats = price_stats::compute_price_stats(data);

        // 8. 行业横向对标渲染（如有）
        let industry_section = data[0].industry.as_ref().and_then(|ib| {
            if ib.peer_count < 3 {
                return None;
            }
            let fmt_opt = |v: Option<f64>| match v {
                Some(x) => format!("{:.2}", x),
                None => "-".to_string(),
            };
            let fmt_pct = |v: Option<f64>| match v {
                Some(x) => format!("P{:.0}", x),
                None => "-".to_string(),
            };
            let mut s = String::new();
            s.push_str(&format!(
                "**同业范围**：{}（{}，共 {} 家同业）\n\n",
                ib.industry_name, ib.board_code, ib.peer_count
            ));
            s.push_str("| 指标 | 个股 | 行业中位数 | 百分位 | 含义 |\n");
            s.push_str("|------|------|------------|--------|------|\n");
            s.push_str(&format!(
                "| PE(TTM) | {} | {} | {} | 越低越便宜 |\n",
                fmt_opt(ib.stock_pe),
                fmt_opt(ib.median_pe),
                fmt_pct(ib.pe_percentile)
            ));
            s.push_str(&format!(
                "| PB | {} | {} | {} | 越低越便宜 |\n",
                fmt_opt(ib.stock_pb),
                fmt_opt(ib.median_pb),
                fmt_pct(ib.pb_percentile)
            ));
            s.push_str(&format!(
                "| ROE(单季%) | {} | {} | {} | 越高越好 |\n",
                fmt_opt(ib.stock_roe),
                fmt_opt(ib.median_roe),
                fmt_pct(ib.roe_percentile)
            ));
            s.push_str(&format!(
                "| 净利同比% | {} | {} | {} | 越高越好 |\n",
                fmt_opt(ib.stock_growth),
                fmt_opt(ib.median_growth),
                fmt_pct(ib.growth_percentile)
            ));
            let mut tags: Vec<&str> = Vec::new();
            if let Some(p) = ib.roe_percentile {
                if p >= 75.0 {
                    tags.push("💎 ROE 领先同业（前 25%）");
                } else if p <= 25.0 {
                    tags.push("⚠️ ROE 落后同业（后 25%）");
                }
            }
            if let Some(p) = ib.pe_percentile {
                if p <= 25.0 {
                    tags.push("💰 估值低于多数同业（便宜）");
                } else if p >= 75.0 {
                    tags.push("📈 估值高于多数同业（偏贵）");
                }
            }
            if let Some(p) = ib.growth_percentile {
                if p >= 75.0 {
                    tags.push("🚀 业绩增速领先同业");
                } else if p <= 25.0 {
                    tags.push("📉 业绩增速落后同业");
                }
            }
            if !tags.is_empty() {
                s.push_str(&format!("\n**行业地位**：{}\n", tags.join("；")));
            }
            Some(s)
        });

        // 9. 财务质量评估渲染
        let quality_section = data[0]
            .financials_history
            .as_ref()
            .and_then(|hist| crate::data_provider::assess_quality(hist))
            .and_then(|q| {
                if q.flags.is_empty() && q.risk_score == 0 {
                    return None;
                }
                let icon = match q.level {
                    "优秀" => "🟢",
                    "良好" => "🟢",
                    "一般" => "🟡",
                    "偏弱" => "🟠",
                    "风险" => "🔴",
                    _ => "⚪",
                };
                let mut s = String::new();
                s.push_str(&format!(
                    "**风险评分**：{} {} / 100（等级：{}）\n",
                    icon, q.risk_score, q.level
                ));
                if !q.flags.is_empty() {
                    s.push_str("\n**触发的红旗信号**：\n");
                    for f in &q.flags {
                        s.push_str(&format!("- ⚠️ {}\n", f));
                    }
                }
                Some(s)
            });

        // 10. 估值历史分位渲染
        let valuation_history_section = data[0].valuation_history.as_ref().and_then(|vh| {
            if vh.sample_days < 30 {
                return None;
            }
            let fmt_opt = |v: Option<f64>| match v {
                Some(x) => format!("{:.2}", x),
                None => "-".to_string(),
            };
            let fmt_pct = |v: Option<f64>| match v {
                Some(x) => format!("P{:.0}", x),
                None => "-".to_string(),
            };
            let tag_for = |p: Option<f64>| match p {
                Some(p) if p <= 20.0 => " 💎 历史底部区",
                Some(p) if p <= 40.0 => " 偏低",
                Some(p) if p < 60.0 => " 中位",
                Some(p) if p < 80.0 => " 偏高",
                Some(_) => " 🔥 历史高位",
                None => "",
            };
            let range = match (&vh.oldest_date, &vh.newest_date) {
                (Some(o), Some(n)) => format!("{} ~ {}", o, n),
                _ => format!("近 {} 个交易日", vh.sample_days),
            };
            let mut s = String::new();
            s.push_str(&format!(
                "**样本区间**：{}（共 {} 个交易日）\n\n",
                range, vh.sample_days
            ));
            s.push_str("| 指标 | 当前 | 历史最低 | 中位 | 最高 | 当前分位 |\n");
            s.push_str("|------|------|---------|------|------|---------|\n");
            s.push_str(&format!(
                "| PE | {} | {} | {} | {} | {}{} |\n",
                fmt_opt(vh.current_pe),
                fmt_opt(vh.pe_min),
                fmt_opt(vh.pe_median),
                fmt_opt(vh.pe_max),
                fmt_pct(vh.pe_percentile),
                tag_for(vh.pe_percentile),
            ));
            s.push_str(&format!(
                "| PB | {} | {} | {} | {} | {}{} |\n",
                fmt_opt(vh.current_pb),
                fmt_opt(vh.pb_min),
                fmt_opt(vh.pb_median),
                fmt_opt(vh.pb_max),
                fmt_pct(vh.pb_percentile),
                tag_for(vh.pb_percentile),
            ));
            Some(s)
        });

        // 11. 卖方一致预期渲染
        let consensus_section = data[0].consensus.as_ref().and_then(|cs| {
            if cs.report_count == 0 {
                return None;
            }
            let cur = data[0].close;
            let mut s = String::new();
            s.push_str(&format!(
                "**研报覆盖**：近 6 个月 {} 份研报 / {} 家券商\n",
                cs.report_count, cs.broker_count
            ));
            if !cs.rating_distribution.is_empty() {
                let mut parts: Vec<(String, u32)> = cs
                    .rating_distribution
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                parts.sort_by(|a, b| b.1.cmp(&a.1));
                let dist: Vec<String> =
                    parts.iter().map(|(k, v)| format!("{} {}", k, v)).collect();
                let bull = cs.bullish_ratio().unwrap_or(0.0);
                s.push_str(&format!(
                    "**评级分布**：{} | 看多比例 {:.0}%\n",
                    dist.join(" / "),
                    bull
                ));
            }
            match (cs.target_price_low_avg, cs.target_price_high_avg) {
                (Some(low), Some(high)) => {
                    let upside = cs.upside_pct(cur).unwrap_or(0.0);
                    let tag = if upside >= 30.0 {
                        " 🚀 显著上行空间"
                    } else if upside >= 10.0 {
                        " ✅ 温和上行"
                    } else if upside >= 0.0 {
                        " 持平"
                    } else {
                        " ⚠️ 已高于目标价"
                    };
                    s.push_str(&format!(
                        "**目标价区间**：¥{:.2} ~ ¥{:.2}（当前 ¥{:.2}，空间 {:+.1}%{}）\n",
                        low, high, cur, upside, tag
                    ));
                }
                (None, Some(high)) => {
                    let upside = cs.upside_pct(cur).unwrap_or(0.0);
                    s.push_str(&format!(
                        "**目标价均值**：¥{:.2}（当前 ¥{:.2}，空间 {:+.1}%）\n",
                        high, cur, upside
                    ));
                }
                _ => {}
            }
            if let Some(e_t) = cs.eps_this_year_avg {
                let mut line = format!("**EPS 预测**：当年 {:.2}", e_t);
                if let Some(e_n) = cs.eps_next_year_avg {
                    let g = if e_t.abs() > 1e-6 {
                        format!("（同比 {:+.1}%）", (e_n - e_t) / e_t.abs() * 100.0)
                    } else {
                        String::new()
                    };
                    line.push_str(&format!(" / 明年 {:.2}{}", e_n, g));
                }
                if let Some(e_n2) = cs.eps_next2_year_avg {
                    line.push_str(&format!(" / 后年 {:.2}", e_n2));
                }
                s.push_str(&line);
                s.push('\n');
            }
            if !cs.recent_reports.is_empty() {
                s.push_str("\n**最近研报**：\n\n");
                s.push_str("| 日期 | 机构 | 评级 | 标题 |\n");
                s.push_str("|------|------|------|------|\n");
                for r in cs.recent_reports.iter().take(3) {
                    let title = if r.title.chars().count() > 28 {
                        format!("{}…", r.title.chars().take(28).collect::<String>())
                    } else {
                        r.title.clone()
                    };
                    s.push_str(&format!(
                        "| {} | {} | {} | {} |\n",
                        r.publish_date, r.org_name, r.rating, title
                    ));
                }
            }
            Some(s)
        });

        // 12. 多期财务趋势渲染
        let fin_history_section = data[0].financials_history.as_ref().and_then(|hist| {
            let show: Vec<&FinancialPeriod> = hist.iter().take(6).collect();
            if show.len() < 2 {
                return None;
            }
            let fmt_opt = |v: Option<f64>| match v {
                Some(x) => format!("{:.2}", x),
                None => "-".to_string(),
            };
            let fmt_ratio = |v: Option<f64>| match v {
                Some(x) => format!("{:.2}", x),
                None => "-".to_string(),
            };
            let mut s = String::new();
            s.push_str("| 报告期 | ROE% | 营收YoY% | 净利YoY% | 毛利率% | 净利率% | CFO/NI |\n");
            s.push_str("|--------|------|---------|---------|--------|--------|--------|\n");
            for p in &show {
                let date = p.report_date.clone().unwrap_or_else(|| "-".into());
                let cfo_ni = p.cfo_to_ni_ratio();
                s.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} |\n",
                    date,
                    fmt_opt(p.roe),
                    fmt_opt(p.revenue_yoy),
                    fmt_opt(p.net_profit_yoy),
                    fmt_opt(p.gross_margin),
                    fmt_opt(p.net_margin),
                    fmt_ratio(cfo_ni),
                ));
            }
            // 趋势提示
            let trend = |f: fn(&FinancialPeriod) -> Option<f64>| -> Option<&'static str> {
                let vals: Vec<f64> =
                    show.iter().filter_map(|p| f(p)).collect();
                if vals.len() < 3 {
                    return None;
                }
                let up = vals.windows(2).all(|w| w[0] >= w[1]); // 最新→旧 递增 = 上行
                let down = vals.windows(2).all(|w| w[0] <= w[1]);
                if up && !down {
                    Some("持续上行")
                } else if down && !up {
                    Some("持续下行")
                } else {
                    None
                }
            };
            let mut hints: Vec<String> = Vec::new();
            if let Some(t) = trend(|p| p.roe) {
                hints.push(format!("ROE {}", t));
            }
            if let Some(t) = trend(|p| p.revenue_yoy) {
                hints.push(format!("营收增速 {}", t));
            }
            if let Some(t) = trend(|p| p.gross_margin) {
                hints.push(format!("毛利率 {}", t));
            }
            if !hints.is_empty() {
                s.push_str(&format!("\n**趋势**：{}\n", hints.join("；")));
            }
            // CFO/NI 平均
            let ratios: Vec<f64> =
                show.iter().filter_map(|p| p.cfo_to_ni_ratio()).collect();
            if !ratios.is_empty() {
                let avg = ratios.iter().sum::<f64>() / ratios.len() as f64;
                let tag = if avg < 0.3 {
                    "⚠️ 偏低，需警惕利润含金量"
                } else if avg < 0.6 {
                    "🟡 健康下沿"
                } else if avg < 1.0 {
                    "🟢 健康"
                } else {
                    "💎 优秀（现金流回款好于账面利润）"
                };
                s.push_str(&format!(
                    "**盈利质量**：近 {} 期 CFO/净利均值 {:.2}（{}）\n",
                    ratios.len(),
                    avg,
                    tag
                ));
            }
            Some(s)
        });

        // 构建深度研判复用种子：复用本流程已抓取的数据（K线 Arc 共享 + 资金/新闻/财务文本），
        // 并携带去结论化的趋势快照（仅证据，不含 signal_score / buy_signal）。
        let trend_snapshot = crate::deep_analyzer::TrendSnapshot {
            trend_status: format!("{}", trend_result.trend_status),
            ma_alignment: trend_result.ma_alignment.clone(),
            trend_strength: trend_result.trend_strength,
            bias_ma5: trend_result.bias_ma5,
            volume_status: format!("{}", trend_result.volume_status),
            volume_ratio_5d: trend_result.volume_ratio_5d,
            support_levels: trend_result.support_levels.clone(),
            resistance_levels: trend_result.resistance_levels.clone(),
            evidence_reasons: trend_result.signal_reasons.clone(),
            risk_factors: trend_result.risk_factors.clone(),
        };
        let fundamental_ctx = {
            let mut parts: Vec<String> = Vec::new();
            if let Some(s) = fin_history_section.as_deref() {
                parts.push(format!("【多期财务趋势】\n{}", s));
            }
            if let Some(s) = valuation_history_section.as_deref() {
                parts.push(format!("【估值历史分位】\n{}", s));
            }
            if let Some(s) = consensus_section.as_deref() {
                parts.push(format!("【卖方一致预期】\n{}", s));
            }
            if let Some(s) = industry_section.as_deref() {
                parts.push(format!("【行业横向对标】\n{}", s));
            }
            if let Some(s) = quality_section.as_deref() {
                parts.push(format!("【财务质量评估】\n{}", s));
            }
            if parts.is_empty() { None } else { Some(parts.join("\n\n")) }
        };
        let deep_seed = crate::deep_analyzer::DeepAnalysisSeed {
            code: code.to_string(),
            name: stock_name.clone(),
            kline: kline_arc,
            extra_context: extra_context.clone(),
            news_context: news_context.clone(),
            macro_context: macro_context.map(|s| s.to_string()),
            fundamental_ctx,
            trend_snapshot,
        };

        let mut result = AnalysisResult {
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
            industry_section,
            quality_section,
            valuation_history_section,
            consensus_section,
            fin_history_section,
            score_breakdown: None,
            score_breakdown_section: None,
            veto_section: None,
            veto_flags: None,
            original_advice: None,
            trade_type: None,
            money_flow: money_flow_raw,
            deep_seed: Some(deep_seed),
        };

        // ===== Phase 1: 多维评分拆解 + 风险否决规则 =====
        // 注：sb_pre / veto_pre / trade_type_pre 已在 AI 调用前计算（用于注入 prompt），此处直接复用。
        let sb = sb_pre;
        let veto = veto_pre;
        result.score_breakdown_section = Some(score_breakdown::render_section(&sb));
        let original_advice = result.operation_advice.clone();
        result.original_advice = Some(original_advice.clone());
        if let Some(new_adv) = veto.downgraded_advice.as_ref() {
            info!(
                "[{}] 否决规则触发，操作建议下调：『{}』 → 『{}』",
                code, original_advice, new_adv
            );
            result.operation_advice = new_adv.clone();
        }
        result.veto_section = veto_rules::render_section(&veto, &original_advice);
        if !veto.flags.is_empty() {
            result.veto_flags = Some(veto.flags.clone());
        }
        result.score_breakdown = Some(sb);

        // ===== Phase 2: 交易类型标注 =====
        result.trade_type = trade_type_pre;

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

        // K 线以 Arc 共享，供后续分析/持仓跟踪/深度研判种子零拷贝复用。
        let data = Arc::new(data);

        // 3. 分析
        let mc = if macro_context.is_empty() { None } else { Some(macro_context.as_str()) };
        let mut result = match self.analyze_stock(&code, &data, data.clone(), mc).await {
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
        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        candidates.truncate(max);

        info!("[深度研判] 命中重点股 {} 只（上限 {}）", candidates.len(), max);

        for (idx, _) in candidates {
            let code = results[idx].code.clone();
            let name = results[idx].name.clone();
            info!("[深度研判] ▶ {} {}", code, name);
            // 优先复用主流程数据种子（避免重复抓取）；缺失时回退到现抓路径。
            let seed_opt = results[idx].deep_seed.clone();
            let deep = match &seed_opt {
                Some(seed) => tokio::time::timeout(
                    std::time::Duration::from_secs(300),
                    crate::deep_analyzer::run_multi_agent_analysis_with_seed(seed),
                )
                .await,
                None => tokio::time::timeout(
                    std::time::Duration::from_secs(300),
                    crate::deep_analyzer::run_multi_agent_analysis(&code),
                )
                .await,
            };
            match deep {
                Ok(Ok(md)) if !md.trim().is_empty() => {
                    results[idx].analysis_summary =
                        merge_deep_analysis(&results[idx].analysis_summary, &md);
                    if let Err(e) = save_deep_report(&code, &results[idx].analysis_summary) {
                        warn!("[深度研判] {} 落盘失败: {}", code, e);
                    }
                    info!("[深度研判] ✓ {} 已合并进报告", code);
                }
                Ok(Ok(_)) => warn!("[深度研判] {} 返回空，保留标准分析", code),
                Ok(Err(e)) => warn!("[深度研判] {} 失败，保留标准分析: {:#}", code, e),
                Err(_) => warn!("[深度研判] {} 超时(300s)，保留标准分析", code),
            }
        }
    }

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
            self.enrich_key_stocks_with_deep_analysis(&mut results).await;
        }

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

#[cfg(test)]
mod tests {
    use super::normalize_ai_sections;

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
        assert!(
            got.contains("\n地缘政治紧张，影响中性偏空。"),
            "got: {got}"
        );
    }

    #[test]
    fn normalize_dedupes_bare_label_then_inline_body() {
        let input = "【主力资金】\n【主力资金】今日主力净流出 0.03 亿元。\n";
        let got = normalize_ai_sections(input);
        assert_eq!(got.matches("## 【主力资金】").count(), 1, "got: {got}");
        assert!(got.contains("\n今日主力净流出 0.03 亿元。"), "got: {got}");
    }
}
