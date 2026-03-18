//! 股票分析流程调度器
//!
//! 负责协调各模块完成完整的分析流程

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use log::{error, info, warn};
use std::sync::Arc;
use std::path::PathBuf;

use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::{DataFetcherManager, KlineData};
use crate::search_service::get_search_service;
use crate::database::DatabaseManager;
use crate::notification::{self, NotificationService};
use crate::trend_analyzer::StockTrendAnalyzer;
use crate::chart_generator::ChartGenerator;
use crate::multi_factor_strategy::{MultiFactorEngine, MultiFactorConfig, StockFactors};
use crate::backtest::{BacktestEngine, BacktestConfig, BacktestSummary};

/// 分析结果
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub code: String,
    pub name: String,
    pub sentiment_score: i32,
    pub operation_advice: String,
    pub trend_prediction: String,
    pub analysis_content: String,
    // 盈利指标
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
    /// 模拟持仓买入价格
    pub position_buy_price: Option<f64>,
    /// 模拟持仓买入日期
    pub position_buy_date: Option<String>,
    /// 模拟持仓收益率（%）
    pub position_return: Option<f64>,
    /// 模拟持仓数量（股）
    pub position_quantity: Option<i32>,
}

impl AnalysisResult {
    /// 获取情绪emoji
    pub fn get_emoji(&self) -> &str {
        match self.sentiment_score {
            90..=100 => "🚀",
            70..=89 => "📈",
            50..=69 => "➡️",
            30..=49 => "⚠️",
            _ => "📉",
        }
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
        
        // 获取最新K线的夏普比率
        let sharpe_ratio = data.first().and_then(|d| d.sharpe_ratio);
        
        // 获取股票名称
        //let stock_name = self.data_manager.get_stock_name(code).unwrap_or_else(|| format!("股票{}", code));
        // 1. 趋势分析
        let mut trend_result = self.trend_analyzer.analyze_with_kline(data, code);
        
        // 将夏普比率添加到趋势分析结果中
        trend_result.sharpe_ratio = sharpe_ratio;

        info!(
            "[{}] 趋势: {}, 买入信号: {}, 评分: {}",
            code, trend_result.trend_status, trend_result.buy_signal, trend_result.signal_score
        );

        // 2. 构建详细的技术分析内容
        let mut analysis_content = String::from("# 技术分析\n\n");
        
        // 核心指标表格
        analysis_content.push_str("## 📊 核心技术指标\n\n");
        analysis_content.push_str("| 指标 | 数值 | 状态 |\n");
        analysis_content.push_str("|------|------|------|\n");
        analysis_content.push_str(&format!("| 趋势状态 | {} | {} |\n", 
            trend_result.trend_status,
            match trend_result.signal_score {
                70..=100 => "✅ 良好",
                50..=69 => "⚠️ 中性",
                _ => "🔴 偏弱"
            }
        ));
        analysis_content.push_str(&format!("| 买入信号 | {} | 评分: {}/100 |\n", 
            trend_result.buy_signal, trend_result.signal_score));
        analysis_content.push_str(&format!("| MA排列 | {} | - |\n", trend_result.ma_alignment));
        analysis_content.push_str(&format!("| 量能状态 | {} | 量比: {:.2} |\n", 
            trend_result.volume_status, trend_result.volume_ratio_5d));
        analysis_content.push_str(&format!("| 趋势强度 | {:.1}% | {} |\n", 
            trend_result.trend_strength * 100.0,
            if trend_result.trend_strength > 0.7 { "强势" } 
            else if trend_result.trend_strength > 0.4 { "中等" } 
            else { "较弱" }
        ));
        
        // 添加夏普比率
        if let Some(sharpe) = trend_result.sharpe_ratio {
            analysis_content.push_str(&format!("| 夏普比率 | {:.3} | {} |\n",
                sharpe,
                if sharpe >= 2.0 { "🌟 优秀" }
                else if sharpe >= 1.0 { "✅ 良好" }
                else if sharpe >= 0.5 { "⚡ 一般" }
                else if sharpe >= 0.0 { "⚠️ 偏低" }
                else { "🔴 风险大于收益" }
            ));
        }
        
        // 价格与均线数据表格
        analysis_content.push_str("\n## 📈 价格与均线数据\n\n");
        analysis_content.push_str("| 项目 | 价格(元) | 乖离率 | 状态 |\n");
        analysis_content.push_str("|------|---------|--------|------|\n");
        analysis_content.push_str(&format!("| 当前价 | {:.2} | - | - |\n", trend_result.current_price));
        analysis_content.push_str(&format!("| MA5 | {:.2} | {:.2}% | {} |\n", 
            trend_result.ma5, 
            trend_result.bias_ma5,
            if trend_result.support_ma5 { "✅ 支撑有效" } else { "⚠️ 无支撑" }
        ));
        analysis_content.push_str(&format!("| MA10 | {:.2} | {:.2}% | {} |\n", 
            trend_result.ma10, 
            trend_result.bias_ma10,
            if trend_result.support_ma10 { "✅ 支撑有效" } else { "⚠️ 无支撑" }
        ));
        analysis_content.push_str(&format!("| MA20 | {:.2} | {:.2}% | - |\n", 
            trend_result.ma20, trend_result.bias_ma20));
        analysis_content.push_str(&format!("| MA60 | {:.2} | - | 中期趋势 |\n", trend_result.ma60));
        
        // 支撑位与压力位
        if !trend_result.support_levels.is_empty() || !trend_result.resistance_levels.is_empty() {
            analysis_content.push_str("\n## 🎯 关键价位\n\n");
            analysis_content.push_str("| 类型 | 价位(元) | 说明 |\n");
            analysis_content.push_str("|------|---------|------|\n");
            
            for (idx, level) in trend_result.resistance_levels.iter().enumerate() {
                analysis_content.push_str(&format!("| 🔴 压力位{} | {:.2} | 突破后看涨 |\n", idx + 1, level));
            }
            
            analysis_content.push_str(&format!("| 📍 当前价 | {:.2} | - |\n", trend_result.current_price));
            
            for (idx, level) in trend_result.support_levels.iter().enumerate() {
                analysis_content.push_str(&format!("| 🟢 支撑位{} | {:.2} | 跌破需警惕 |\n", idx + 1, level));
            }
        }
        
        // 量能分析
        analysis_content.push_str("\n## 📊 量能分析\n\n");
        analysis_content.push_str(&format!("- **量能状态**: {}\n", trend_result.volume_status));
        analysis_content.push_str(&format!("- **量比(5日)**: {:.2}倍 {}\n", 
            trend_result.volume_ratio_5d,
            if trend_result.volume_ratio_5d > 2.0 { "(显著放量)" }
            else if trend_result.volume_ratio_5d > 1.2 { "(温和放量)" }
            else if trend_result.volume_ratio_5d > 0.8 { "(量能正常)" }
            else { "(缩量)" }
        ));
        analysis_content.push_str(&format!("- **量能趋势**: {}\n", trend_result.volume_trend));
        
        // 添加盈利指标（如果有）
        let latest = &data[0];
        if latest.pe_ratio.is_some() || latest.pb_ratio.is_some() {
            analysis_content.push_str("\n## 盈利水平指标\n\n");
            
            if let Some(pe) = latest.pe_ratio {
                let pe_assessment = if pe < 0.0 {
                    "⚠️ 亏损状态"
                } else if pe < 15.0 {
                    "✅ 估值合理"
                } else if pe < 30.0 {
                    "⚠️ 估值适中"
                } else {
                    "🔴 估值偏高"
                };
                analysis_content.push_str(&format!("- **市盈率(PE)**: {:.2} {}\n", pe, pe_assessment));
            }
            
            if let Some(pb) = latest.pb_ratio {
                let pb_assessment = if pb < 1.0 {
                    "✅ 可能被低估"
                } else if pb < 3.0 {
                    "✅ 市净率正常"
                } else {
                    "🔴 市净率较高"
                };
                analysis_content.push_str(&format!("- **市净率(PB)**: {:.2} {}\n", pb, pb_assessment));
            }
            
            if let Some(turnover) = latest.turnover_rate {
                let turnover_assessment = if turnover < 3.0 {
                    "交投清淡"
                } else if turnover < 10.0 {
                    "正常换手"
                } else {
                    "活跃交易"
                };
                analysis_content.push_str(&format!("- **换手率**: {:.2}% ({})\n", turnover, turnover_assessment));
            }
            
            if let Some(market_cap) = latest.market_cap {
                analysis_content.push_str(&format!("- **总市值**: {:.2}亿元\n", market_cap));
            }
            
            if let Some(circ_cap) = latest.circulating_cap {
                analysis_content.push_str(&format!("- **流通市值**: {:.2}亿元\n", circ_cap));
            }
        }

        if !trend_result.signal_reasons.is_empty() {
            analysis_content.push_str("\n## 信号原因\n");
            for reason in &trend_result.signal_reasons {
                analysis_content.push_str(&format!("- {}\n", reason));
            }
        }

        if !trend_result.risk_factors.is_empty() {
            analysis_content.push_str("\n## 风险因素\n");
            for risk in &trend_result.risk_factors {
                analysis_content.push_str(&format!("- {}\n", risk));
            }
        }

        // 添加作战计划（评分>=60的股票）
        if trend_result.signal_score >= 60 {
            analysis_content.push_str("\n## 🎯 作战计划\n\n");
            
            // 建议买入价位（当前价或回踩支撑位）
            let current_price = trend_result.current_price;
            let buy_price = if trend_result.bias_ma5 > 0.0 && trend_result.bias_ma5 < 3.0 {
                // 接近MA5，可以考虑当前价
                current_price
            } else if !trend_result.support_levels.is_empty() {
                // 建议回踩支撑位买入
                trend_result.support_levels[0]
            } else {
                // 默认当前价
                current_price
            };
            
            // 止损位（MA10下方2%或最近支撑位下方2%）
            let stop_loss = if trend_result.support_ma10 {
                trend_result.ma10 * 0.98
            } else if !trend_result.support_levels.is_empty() {
                trend_result.support_levels[0] * 0.98
            } else {
                current_price * 0.95 // 默认5%止损
            };
            
            // 目标价位（根据压力位或上涨空间）
            let target_price = if !trend_result.resistance_levels.is_empty() {
                trend_result.resistance_levels[0]
            } else {
                // 默认目标10-15%
                current_price * 1.12
            };
            
            analysis_content.push_str(&format!("- **建议买入价**: {:.2}元 ", buy_price));
            if trend_result.bias_ma5 > 0.0 && trend_result.bias_ma5 < 3.0 {
                analysis_content.push_str("(当前价位，接近MA5支撑)\n");
            } else if !trend_result.support_levels.is_empty() {
                analysis_content.push_str("(等待回踩支撑位)\n");
            } else {
                analysis_content.push_str("(当前价位)\n");
            }
            
            analysis_content.push_str(&format!("- **止损价位**: {:.2}元 (跌破-{:.1}%)\n", 
                stop_loss, (1.0 - stop_loss / current_price) * 100.0));
            
            analysis_content.push_str(&format!("- **目标价位**: {:.2}元 (预期+{:.1}%)\n", 
                target_price, (target_price / current_price - 1.0) * 100.0));
            
            // 仓位建议
            let position_suggestion = if trend_result.signal_score >= 80 {
                "建议仓位: 50-70% (强势信号)"
            } else if trend_result.signal_score >= 70 {
                "建议仓位: 30-50% (中等信号)"
            } else {
                "建议仓位: 20-30% (试探性建仓)"
            };
            analysis_content.push_str(&format!("- **{}**\n", position_suggestion));
            
            // 操作策略
            analysis_content.push_str("\n**操作策略**:\n");
            if trend_result.support_ma5 || trend_result.support_ma10 {
                analysis_content.push_str("- 当前在均线支撑位附近，可分批建仓\n");
            } else {
                analysis_content.push_str("- 等待回踩均线支撑再介入，不追高\n");
            }
            
            if !trend_result.support_levels.is_empty() {
                analysis_content.push_str(&format!(
                    "- 重要支撑位: {:.2}元，跌破需重新评估\n",
                    trend_result.support_levels[0]
                ));
            }
            
            if !trend_result.resistance_levels.is_empty() {
                analysis_content.push_str(&format!(
                    "- 上方压力位: {:.2}元，突破后可加仓\n",
                    trend_result.resistance_levels[0]
                ));
            }
            
            analysis_content.push_str("- 严格执行止损，避免深套\n");
        }
        
        // 获取真实股票名称（同步 HTTP 调用，放到 blocking 线程池）
        let dm = self.data_manager.clone();
        let code_owned = code.to_string();
        let stock_name = tokio::task::spawn_blocking(move || {
            dm.get_stock_name(&code_owned)
        }).await.ok().flatten().unwrap_or_else(|| format!("股票{}", code));
        info!("[{}] 搜索最新新闻...", code);
        // 新闻搜索（如果启用）
        let news_context = if self.use_news_search {
            let search_service = get_search_service();
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
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

        // AI增强分析（包含新闻）
        if let Some(ref ai) = self.ai_analyzer {
            // 构建包含新闻的prompt
            let _prompt = if let Some(ref news) = news_context {
                format!(
                    "{}

# 最新新闻\n{}",
                    format!(
                        "股票代码: {}\n最新价: {:.2}\n涨跌幅: {:.2}%",
                        code, data[0].close, data[0].pct_chg
                    ),
                    news
                )
            } else {
                format!(
                    "股票代码: {}\n最新价: {:.2}\n涨跌幅: {:.2}%",
                    code, data[0].close, data[0].pct_chg
                )
            };

            match ai.analyze_stock(code, data, macro_context).await {
                Ok(ai_result) => {
                    // info!("[{}] AI分析结果:\n{}", code, ai_result);
                    analysis_content.push_str("\n# AI分析\n\n");
                    analysis_content.push_str(&ai_result);
                    if news_context.is_some() {
                        analysis_content.push_str("\n\n# 相关新闻\n\n");
                        analysis_content.push_str(news_context.as_ref().unwrap());
                    }
                }
                Err(e) => {
                    warn!("[{}] AI分析失败: {}", code, e);
                }
            }
        } else if let Some(ref news) = news_context {
            // 没有AI但有新闻，也添加到报告
            analysis_content.push_str("\n# 相关新闻\n\n");
            analysis_content.push_str(news);
        }

        // 生成操作建议
        let operation_advice = match trend_result.signal_score {
            80..=100 => "强烈建议买入",
            60..=79 => "建议买入",
            40..=59 => "观望",
            20..=39 => "建议减仓",
            _ => "建议卖出",
        }
        .to_string();

        // 计算52周与季度价格区间
        let data_len = data.len();
        let week52_len = data_len.min(250);
        let (high_52w, low_52w, pos_52w) = if week52_len >= 5 {
            let h = data[..week52_len].iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let l = data[..week52_len].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos = if (h - l).abs() > 0.001 {
                (data[0].close - l) / (h - l) * 100.0
            } else { 50.0 };
            (Some(h), Some(l), Some(pos))
        } else { (None, None, None) };

        let quarter_len = data_len.min(60);
        let (high_quarter, low_quarter, pos_quarter) = if quarter_len >= 5 {
            let h = data[..quarter_len].iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let l = data[..quarter_len].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos = if (h - l).abs() > 0.001 {
                (data[0].close - l) / (h - l) * 100.0
            } else { 50.0 };
            (Some(h), Some(l), Some(pos))
        } else { (None, None, None) };

        // 计算近期涨幅和波动率
        let chg_5d: Option<f64> = if data_len >= 2 {
            Some(data[..data_len.min(5)].iter().map(|k| k.pct_chg).sum())
        } else { None };
        let chg_10d: Option<f64> = if data_len >= 10 {
            Some(data[..10].iter().map(|k| k.pct_chg).sum())
        } else { None };
        let volatility: Option<f64> = if data_len >= 5 {
            let recent = data_len.min(10);
            let returns: Vec<f64> = data[..recent].iter().map(|k| k.pct_chg).collect();
            let mean = returns.iter().sum::<f64>() / returns.len() as f64;
            let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
            Some(var.sqrt())
        } else { None };

        let result = AnalysisResult {
            code: code.to_string(),
            name: stock_name,
            sentiment_score: trend_result.signal_score,
            operation_advice,
            trend_prediction: format!("{}", trend_result.trend_status),
            analysis_content,
            // 从最新K线获取盈利指标
            pe_ratio: data[0].pe_ratio,
            pb_ratio: data[0].pb_ratio,
            turnover_rate: data[0].turnover_rate,
            market_cap: data[0].market_cap,
            circulating_cap: data[0].circulating_cap,
            // 均线与乖离率
            current_price: Some(trend_result.current_price),
            ma5: Some(trend_result.ma5),
            ma10: Some(trend_result.ma10),
            ma20: Some(trend_result.ma20),
            ma60: Some(trend_result.ma60),
            ma_alignment: Some(trend_result.ma_alignment.clone()),
            bias_ma5: Some(trend_result.bias_ma5),
            // 量能
            volume_ratio_5d: Some(trend_result.volume_ratio_5d),
            // 价格区间
            high_52w, low_52w, pos_52w,
            high_quarter, low_quarter, pos_quarter,
            // 近期走势
            chg_5d, chg_10d, volatility,
            // 财务指标
            eps: data[0].eps,
            roe: data[0].roe,
            gross_margin: data[0].gross_margin,
            net_margin: data[0].net_margin,
            revenue_yoy: data[0].revenue_yoy,
            net_profit_yoy: data[0].net_profit_yoy,
            sharpe_ratio: trend_result.sharpe_ratio,
            is_limit_up: self.limit_up_codes.contains(code),
            position_buy_price: None,
            position_buy_date: None,
            position_return: None,
            position_quantity: None,
        };

        Ok(result)
    }

    /// 处理单只股票的完整流程
    async fn process_stock(&self, code: String, macro_context: Arc<String>) -> Option<AnalysisResult> {
        info!("========== 开始处理 {} ==========", code);

        // 整体超时保护：单只股票最多处理 120 秒，避免任何环节卡死拖垮全局
        match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.process_stock_inner(code.clone(), macro_context),
        ).await {
            Ok(result) => result,
            Err(_) => {
                error!("[{}] 处理超时（120s），跳过", code);
                None
            }
        }
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

        // 4. 模拟持仓跟踪
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            let current_price = result.current_price.unwrap_or(data[0].close);

            // 检查是否有持仓中的记录
            match db.get_open_position(&code) {
                Ok(Some(pos)) => {
                    // 有持仓：计算收益率
                    let return_rate = (current_price / pos.buy_price - 1.0) * 100.0;
                    result.position_buy_price = Some(pos.buy_price);
                    result.position_buy_date = Some(pos.buy_date.clone());
                    result.position_return = Some(return_rate);
                    result.position_quantity = Some(pos.quantity);
                    info!("[{}] 持仓收益率: {:.2}%（买入价 {:.2}）", code, return_rate, pos.buy_price);

                    // 更新数据库中的收益率
                    if let Err(e) = db.update_position_return(pos.id, current_price, return_rate) {
                        warn!("[{}] 更新持仓收益率失败: {}", code, e);
                    }

                    // 如果建议卖出，自动平仓
                    if result.operation_advice.contains("卖出") {
                        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                        match db.close_position(pos.id, current_price, &today) {
                            Ok(_) => info!("[{}] 持仓已平仓，收益率: {:.2}%", code, return_rate),
                            Err(e) => warn!("[{}] 平仓失败: {}", code, e),
                        }
                    }
                }
                Ok(None) => {
                    // 无持仓：如果建议买入，记录模拟买入（10手=1000股）
                    if result.operation_advice.contains("买入") {
                        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                        let new_position = crate::models::NewStockPosition {
                            code: code.clone(),
                            name: result.name.clone(),
                            buy_date: today,
                            buy_price: current_price,
                            quantity: 1000,
                            status: "open".to_string(),
                        };
                        match db.save_position(&new_position) {
                            Ok(_) => info!("[{}] 模拟买入 1000 股 @ {:.2}", code, current_price),
                            Err(e) => warn!("[{}] 记录模拟买入失败: {}", code, e),
                        }
                    }
                }
                Err(e) => {
                    warn!("[{}] 查询持仓失败: {}", code, e);
                }
            }
        }

        // 5. 保存分析结果到数据库
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            let latest_kline = &data[0];
            let new_result = crate::models::NewAnalysisResult {
                code: result.code.clone(),
                name: result.name.clone(),
                date: chrono::Local::now().date_naive(),
                sentiment_score: result.sentiment_score,
                operation_advice: result.operation_advice.clone(),
                trend_prediction: result.trend_prediction.clone(),
                pe_ratio: result.pe_ratio,
                pb_ratio: result.pb_ratio,
                turnover_rate: result.turnover_rate,
                market_cap: result.market_cap,
                circulating_cap: result.circulating_cap,
                close_price: Some(latest_kline.close),
                pct_chg: Some(latest_kline.pct_chg),
                data_source: None,
            };
            match db.save_analysis_result(&new_result) {
                Ok(_) => info!("[{}] 分析结果已保存到数据库", code),
                Err(e) => warn!("[{}] 保存分析结果失败: {}", code, e),
            }
        }

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
        // 优先使用已获取的宏观新闻（避免重复搜索），否则在线搜索
        let macro_context: Arc<String> = if let Some(mc) = prefetched_macro {
            if !mc.is_empty() {
                info!("✓ 复用已获取的宏观新闻（{} 字符），跳过重复搜索", mc.len());
                Arc::new(mc)
            } else {
                Arc::new(String::new())
            }
        } else if self.use_news_search {
            info!("📡 搜索今日宏观/市场最新新闻（所有股票共享）...");
            let search_service = get_search_service();
            let mc = match tokio::time::timeout(
                std::time::Duration::from_secs(15),
                search_service.search_macro_news(3),
            ).await {
                Ok(text) if !text.is_empty() => {
                    info!("✓ 宏观新闻获取成功，共 {} 字符", text.len());
                    text
                }
                Ok(_) => { warn!("宏观新闻搜索返回为空"); String::new() }
                Err(_) => { warn!("宏观新闻搜索超时(15s)"); String::new() }
            };
            Arc::new(mc)
        } else {
            Arc::new(String::new())
        };

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

        // 发送汇总通知
        if !results.is_empty()
            && self.config.send_notification
            && !self.config.dry_run
            && !self.config.single_notify
        {
            self.send_summary_notification(&results, backtest_summary.as_ref()).await?;
        }

        Ok(results)
    }

    /// 生成单股报告
    fn generate_single_report(&self, result: &AnalysisResult) -> String {
        let limit_up_tag = if result.is_limit_up { " 🔥涨停" } else { "" };
        format!(
            "{} {}({}){}\n\n操作建议: {}\n评分: {}\n\n{}",
            result.get_emoji(),
            result.name,
            result.code,
            limit_up_tag,
            result.operation_advice,
            result.sentiment_score,
            result.analysis_content
        )
    }

    /// 发送汇总通知
    async fn send_summary_notification(&self, results: &[AnalysisResult], backtest_summary: Option<&BacktestSummary>) -> Result<()> {
        info!("生成分析汇总报告...");

        let date_str = chrono::Local::now().format("%Y%m%d").to_string();
        
        // 按评分排序（索引排序，避免深拷贝整个结果集）
        let mut indices: Vec<usize> = (0..results.len()).collect();
        indices.sort_by(|&a, &b| results[b].sentiment_score.cmp(&results[a].sentiment_score));
        let sorted: Vec<&AnalysisResult> = indices.iter().map(|&i| &results[i]).collect();

        // 生成图表
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

        // 转换为 notification 模块的 AnalysisResult
        let notification_results: Vec<notification::AnalysisResult> = sorted
            .iter()
            .map(|r| notification::AnalysisResult::from(*r))
            .collect();

        // 使用 notification 模块的报告生成方法（股票分析报告）
        let report = self.notifier.generate_daily_report(&notification_results);

        // 如果有回测结果，单独保存回测报告到本地（不发送邮件）
        if let Some(summary) = backtest_summary {
            let mut backtest_report = String::new();
            backtest_report.push_str("# 📊 多因子策略回测报告\n\n");
            backtest_report.push_str(&format!("**生成时间**: {}\n\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));
            backtest_report.push_str("---\n\n");
            
            backtest_report.push_str("## 回测结果汇总\n\n");
            backtest_report.push_str("| 指标 | 数值 | 说明 |\n");
            backtest_report.push_str("|------|------|------|\n");
            backtest_report.push_str(&format!(
                "| 初始资金 | ¥{:.2}万 | 回测初始资金 |\n",
                summary.initial_capital / 10000.0
            ));
            backtest_report.push_str(&format!(
                "| 期末资产 | ¥{:.2}万 | 当前总资产 |\n",
                summary.final_value / 10000.0
            ));
            backtest_report.push_str(&format!(
                "| 总收益率 | {:.2}% | {} |\n",
                summary.total_return * 100.0,
                if summary.total_return > 0.0 { "📈 盈利" } else { "📉 亏损" }
            ));
            backtest_report.push_str(&format!(
                "| 年化收益率 | {:.2}% | 折算成年化收益 |\n",
                summary.annual_return * 100.0
            ));
            backtest_report.push_str(&format!(
                "| 最大回撤 | {:.2}% | {} |\n",
                summary.max_drawdown * 100.0,
                if summary.max_drawdown < 0.1 { "🛡️ 风险较低" } else if summary.max_drawdown < 0.2 { "⚠️ 风险适中" } else { "🚨 风险较高" }
            ));
            backtest_report.push_str(&format!(
                "| 夏普比率 | {:.2} | {} |\n",
                summary.sharpe_ratio,
                if summary.sharpe_ratio > 1.0 { "⭐ 优秀" } else if summary.sharpe_ratio > 0.5 { "✅ 良好" } else { "⚠️ 一般" }
            ));
            backtest_report.push_str(&format!(
                "| 交易次数 | {} 次 | 总交易次数 |\n",
                summary.total_trades
            ));
            backtest_report.push_str(&format!(
                "| 胜率 | {:.1}% | 盈利交易占比 |\n",
                summary.win_rate * 100.0
            ));
            
            backtest_report.push_str("\n## 策略说明\n\n");
            backtest_report.push_str("**多因子选股策略**: 基于市值、市盈率、市净率、换手率等多因子综合评分，选出得分最高的20只股票进行等权重配置。\n\n");
            
            if let Some(chart_path) = &summary.chart_path {
                backtest_report.push_str(&format!("**回测图表**: {}\n\n", chart_path));
            }
            
            // 保存回测报告到本地
            let backtest_filename = format!("backtest_report_{}.md", date_str);
            self.notifier.save_report_to_file(&backtest_report, Some(&backtest_filename))?;
            info!("✓ 多因子回测报告已保存到本地: reports/{}", backtest_filename);
        }

        // 保存股票分析报告
        let filename = format!("stock_analysis_{}.md", date_str);
        self.notifier
            .save_report_to_file(&report, Some(&filename))?;

        // 发送文本报告（不附带图片）
        // 图表已保存到本地，可在报告文件中查看
        match self.notifier.send(&report).await {
            Ok(_) => info!("✓ 股票分析报告推送成功"),
            Err(e) => error!("推送通知失败: {}", e),
        }

        Ok(())
    }

    /// 运行多因子回测
    async fn run_multi_factor_backtest(&self, results: &[AnalysisResult]) -> Result<BacktestSummary> {
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
}
