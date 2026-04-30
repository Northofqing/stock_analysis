//! 单股分析入口（从 analyzer.rs 拆分）。
//!
//! 负责 `analyze_stock` 与 `analyze` 两个对外方法。

use anyhow::{anyhow, Result};
use log::{debug, error, info};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::types::AnalysisResult;
use super::GeminiAnalyzer;

impl GeminiAnalyzer {
    /// 简化的股票分析方法（用于pipeline）
    pub async fn analyze_stock(
        &self,
        code: &str,
        kline_data: &[crate::data_provider::KlineData],
        macro_context: Option<&str>,
    ) -> Result<String> {
        self.analyze_stock_with_extras(code, None, kline_data, macro_context, None, None)
            .await
    }

    /// 扩展版：允许调用方注入真实口径的资金流 / 分时 / 龙虎榜席位等额外 prompt 片段。
    pub async fn analyze_stock_with_extras(
        &self,
        code: &str,
        name: Option<&str>,
        kline_data: &[crate::data_provider::KlineData],
        macro_context: Option<&str>,
        extra_context: Option<&str>,
        news_context: Option<&str>,
    ) -> Result<String> {
        if kline_data.is_empty() {
            return Err(anyhow!("数据为空"));
        }

        // ========== 多 Agent 流水线已从个股分析中移除 ==========
        // 多 Agent 模块（agents/）目前仅保留供新闻/宏观分析改造使用，
        // 个股分析统一走下方单 prompt 模式，避免 6+ 次 LLM 调用带来的成本与失败率。
        let _ = self.config.agent_pipeline; // 保留字段，避免破坏配置兼容
        // 原 if self.config.agent_pipeline && self.is_available() { run_text_pipeline... } 已删除

        // 构建简化的分析上下文
        let latest = &kline_data[0];
        
        // 基础行情数据
        let mut context = format!(
            "股票代码: {}\n\
            最新价: {:.2}\n\
            开盘: {:.2}\n\
            最高: {:.2}\n\
            最低: {:.2}\n\
            成交量: {:.0}\n\
            成交额: {:.0}\n\
            涨跌幅: {:.2}%\n",
            code,
            latest.close,
            latest.open,
            latest.high,
            latest.low,
            latest.volume,
            latest.amount,
            latest.pct_chg
        );

        // ========== 均线系统与乖离率（从历史K线计算） ==========
        let closes: Vec<f64> = kline_data.iter().map(|k| k.close).collect();
        let data_len = closes.len();

        let calc_ma = |period: usize| -> Option<f64> {
            if data_len >= period {
                Some(closes[..period].iter().sum::<f64>() / period as f64)
            } else {
                None
            }
        };

        let ma5 = calc_ma(5);
        let ma10 = calc_ma(10);
        let ma20 = calc_ma(20);
        let ma60 = calc_ma(60);

        context.push_str("\n【均线系统】\n");
        if let Some(v) = ma5 { context.push_str(&format!("MA5: {:.2}\n", v)); }
        if let Some(v) = ma10 { context.push_str(&format!("MA10: {:.2}\n", v)); }
        if let Some(v) = ma20 { context.push_str(&format!("MA20: {:.2}\n", v)); }
        if let Some(v) = ma60 { context.push_str(&format!("MA60: {:.2}\n", v)); }

        // 乖离率（仅数字）
        if let Some(m5) = ma5 {
            if m5 > 0.0 {
                let bias5 = (latest.close - m5) / m5 * 100.0;
                context.push_str(&format!("MA5乖离率: {:.2}%\n", bias5));
            }
        }
        if let Some(m10) = ma10 {
            if m10 > 0.0 {
                let bias10 = (latest.close - m10) / m10 * 100.0;
                context.push_str(&format!("MA10乖离率: {:.2}%\n", bias10));
            }
        }
        if let Some(m20) = ma20 {
            if m20 > 0.0 {
                let bias20 = (latest.close - m20) / m20 * 100.0;
                context.push_str(&format!("MA20乖离率: {:.2}%\n", bias20));
            }
        }

        // ========== 量能分析（仅数字） ==========
        context.push_str("\n【量能】\n");
        if data_len >= 5 {
            let vol_5d_avg = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
            if vol_5d_avg > 0.0 {
                context.push_str(&format!("5日量比: {:.2}\n", latest.volume / vol_5d_avg));
            }
        }
        if data_len >= 10 {
            let vol_10d_avg = kline_data[..10].iter().map(|k| k.volume).sum::<f64>() / 10.0;
            if vol_10d_avg > 0.0 {
                context.push_str(&format!("10日量比: {:.2}\n", latest.volume / vol_10d_avg));
            }
        }
        if let Some(turnover) = latest.turnover_rate {
            context.push_str(&format!("换手率: {:.2}%\n", turnover));
        }

        // ========== 涨跌停识别（仅事实） ==========
        let is_gem = code.starts_with("300") || code.starts_with("301");
        let is_star = code.starts_with("688");
        let is_bj = code.starts_with("8") || code.starts_with("9") || code.starts_with("4");
        let limit_pct = if is_gem || is_star { 20.0 }
            else if is_bj { 30.0 }
            else { 10.0 };
        if latest.pct_chg >= limit_pct - 0.3 {
            let mut consec = 1;
            for k in kline_data[1..].iter().take(10) {
                if k.pct_chg >= limit_pct - 0.3 { consec += 1; } else { break; }
            }
            context.push_str(&format!("\n【涨跌停】涨停 {}板 (涨停阈值 {}%)\n", consec, limit_pct));
        } else if latest.pct_chg <= -(limit_pct - 0.3) {
            context.push_str(&format!("\n【涨跌停】跌停 (跌幅 {:.2}%)\n", latest.pct_chg));
        } else if latest.pct_chg >= 5.0 {
            context.push_str(&format!("\n【涨跌停】大涨 {:.2}%（未涨停）\n", latest.pct_chg));
        }

        // ========== MACD (12,26,9) ==========
        if data_len >= 26 {
            // 注意：kline_data[0] 是最新，计算 EMA 需要按时间顺序（旧→新）
            let mut chron: Vec<f64> = closes.iter().rev().copied().collect();
            // 仅取最近 60 根以提升效率（足够让 EMA 收敛）
            if chron.len() > 120 { chron = chron[chron.len()-120..].to_vec(); }
            let ema = |period: usize, src: &[f64]| -> Vec<f64> {
                let alpha = 2.0 / (period as f64 + 1.0);
                let mut out = Vec::with_capacity(src.len());
                let mut prev = src[0];
                out.push(prev);
                for &v in &src[1..] {
                    prev = alpha * v + (1.0 - alpha) * prev;
                    out.push(prev);
                }
                out
            };
            let ema12 = ema(12, &chron);
            let ema26 = ema(26, &chron);
            let diff: Vec<f64> = ema12.iter().zip(ema26.iter()).map(|(a,b)| a-b).collect();
            let dea = ema(9, &diff);
            let n = diff.len();
            let macd = 2.0 * (diff[n-1] - dea[n-1]);
            let macd_signal = if diff[n-1] > dea[n-1] && n >= 2 && diff[n-2] <= dea[n-2] {
                "金叉"
            } else if diff[n-1] < dea[n-1] && n >= 2 && diff[n-2] >= dea[n-2] {
                "死叉"
            } else if diff[n-1] > dea[n-1] {
                "多头"
            } else {
                "空头"
            };
            context.push_str(&format!(
                "\n【MACD】DIF={:.3} DEA={:.3} HIST={:.3} {}\n",
                diff[n-1], dea[n-1], macd, macd_signal
            ));
        }

        // ========== RSI(14) - Wilder 平滑 ==========
        if data_len >= 15 {
            let chron: Vec<f64> = closes.iter().rev().copied().collect();
            let mut gains = 0.0;
            let mut losses = 0.0;
            for i in 1..=14 {
                let diff = chron[i] - chron[i-1];
                if diff > 0.0 { gains += diff; } else { losses -= diff; }
            }
            let mut avg_gain = gains / 14.0;
            let mut avg_loss = losses / 14.0;
            for i in 15..chron.len() {
                let diff = chron[i] - chron[i-1];
                let (g, l) = if diff > 0.0 { (diff, 0.0) } else { (0.0, -diff) };
                avg_gain = (avg_gain * 13.0 + g) / 14.0;
                avg_loss = (avg_loss * 13.0 + l) / 14.0;
            }
            let rsi = if avg_loss.abs() < 1e-9 { 100.0 } else {
                100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
            };
            context.push_str(&format!("【RSI14】{:.2}\n", rsi));
        }

        // ========== KDJ(9,3,3) ==========
        if data_len >= 9 {
            let chron: Vec<&crate::data_provider::KlineData> = kline_data.iter().rev().collect();
            let mut k_val = 50.0;
            let mut d_val = 50.0;
            let n = chron.len();
            let start = n.saturating_sub(30).max(8); // 让 KDJ 迭代收敛
            for i in start..n {
                let window_start = i.saturating_sub(8);
                let window = &chron[window_start..=i];
                let hh = window.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
                let ll = window.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
                let rsv = if (hh - ll).abs() < 1e-9 { 50.0 }
                    else { (chron[i].close - ll) / (hh - ll) * 100.0 };
                k_val = 2.0 / 3.0 * k_val + 1.0 / 3.0 * rsv;
                d_val = 2.0 / 3.0 * d_val + 1.0 / 3.0 * k_val;
            }
            let j_val = 3.0 * k_val - 2.0 * d_val;
            context.push_str(&format!(
                "【KDJ】K={:.2} D={:.2} J={:.2}\n",
                k_val, d_val, j_val
            ));
        }

        // ========== 价格区间（52周 / 季度） ==========
        context.push_str("\n【价格区间】\n");
        let week52_len = data_len.min(250);
        if week52_len >= 5 {
            let week52_data = &kline_data[..week52_len];
            let high_52w = week52_data.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let low_52w = week52_data.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos_in_range = if (high_52w - low_52w).abs() > 0.001 {
                (latest.close - low_52w) / (high_52w - low_52w) * 100.0
            } else { 50.0 };
            context.push_str(&format!(
                "52周: H {:.2} / L {:.2} / 位置 {:.1}%\n",
                high_52w, low_52w, pos_in_range
            ));
        }
        let quarter_len = data_len.min(60);
        if quarter_len >= 5 {
            let quarter_data = &kline_data[..quarter_len];
            let high_q = quarter_data.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let low_q = quarter_data.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos_q = if (high_q - low_q).abs() > 0.001 {
                (latest.close - low_q) / (high_q - low_q) * 100.0
            } else { 50.0 };
            context.push_str(&format!(
                "季度: H {:.2} / L {:.2} / 位置 {:.1}%\n",
                high_q, low_q, pos_q
            ));
        }

        // ========== 近期走势汇总（不再逐行列出 K 线） ==========
        let recent_len = data_len.min(10);
        if recent_len >= 2 {
            context.push_str("\n【近期走势】\n");
            let chg_5d: f64 = kline_data[..data_len.min(5)].iter().map(|k| k.pct_chg).sum();
            context.push_str(&format!("近5日累计涨幅: {:.2}%\n", chg_5d));
            if recent_len >= 10 {
                let chg_10d: f64 = kline_data[..10].iter().map(|k| k.pct_chg).sum();
                context.push_str(&format!("近10日累计涨幅: {:.2}%\n", chg_10d));
            }
            let returns: Vec<f64> = kline_data[..recent_len].iter().map(|k| k.pct_chg).collect();
            let mean_ret = returns.iter().sum::<f64>() / returns.len() as f64;
            let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>() / returns.len() as f64;
            context.push_str(&format!("近期日波动率: {:.2}%\n", variance.sqrt()));
        }

        // ========== 估值指标（仅数字） ==========
        if latest.pe_ratio.is_some() || latest.pb_ratio.is_some() || latest.market_cap.is_some() {
            context.push_str("\n【估值】\n");
            if let Some(pe) = latest.pe_ratio {
                context.push_str(&format!("PE: {:.2}\n", pe));
            }
            if let Some(pb) = latest.pb_ratio {
                context.push_str(&format!("PB: {:.2}\n", pb));
            }
            if let Some(market_cap) = latest.market_cap {
                context.push_str(&format!("总市值: {:.2}亿\n", market_cap));
                if let Some(circ_cap) = latest.circulating_cap {
                    let circulation_ratio = (circ_cap / market_cap) * 100.0;
                    context.push_str(&format!("流通市值: {:.2}亿 (流通比 {:.1}%)\n",
                        circ_cap, circulation_ratio));
                }
            }
        }

        // ========== 财务指标（仅数字） ==========
        let has_financials = latest.eps.is_some() || latest.roe.is_some()
            || latest.gross_margin.is_some() || latest.revenue_yoy.is_some();
        if has_financials {
            context.push_str("\n【财务】\n");
            if let Some(eps) = latest.eps {
                context.push_str(&format!("EPS: {:.3}元\n", eps));
            }
            if let Some(roe) = latest.roe {
                context.push_str(&format!("ROE: {:.2}%\n", roe));
            }
            if let Some(gm) = latest.gross_margin {
                context.push_str(&format!("毛利率: {:.2}%\n", gm));
            }
            if let Some(nm) = latest.net_margin {
                context.push_str(&format!("净利率: {:.2}%\n", nm));
            }
            if let Some(rev_yoy) = latest.revenue_yoy {
                context.push_str(&format!("营收YoY: {:.2}%\n", rev_yoy));
            }
            if let Some(profit_yoy) = latest.net_profit_yoy {
                context.push_str(&format!("净利润YoY: {:.2}%\n", profit_yoy));
            }
        }

        if let Some(sharpe) = latest.sharpe_ratio {
            context.push_str(&format!("\n夏普比率: {:.2}\n", sharpe));
        }

        context.push_str(&format!(
            "\n最近{}天数据点数: {}",
            kline_data.len(),
            kline_data.len()
        ));

        // 额外上下文（真实主力资金流 / 日内分时 / 龙虎榜席位等），直接追加到正文
        if let Some(extra) = extra_context {
            if !extra.trim().is_empty() {
                context.push_str(extra);
            }
        }

        // 布林带 + MACD 共振信号（4 条核心规则 + 反误区过滤）
        let bm = crate::strategy::detect_boll_macd_signal(kline_data);
        if bm.action != crate::strategy::BollMacdAction::None {
            context.push_str("\n【布林+MACD 共振信号（强约束）】\n");
            context.push_str(&format!(
                "动作: {} | 收盘 ¥{:.2} | 上轨 ¥{:.2} / 中轨 ¥{:.2} / 下轨 ¥{:.2}\n",
                bm.action.name(), bm.close, bm.upper, bm.middle, bm.lower
            ));
            context.push_str(&format!(
                "带宽 {:.2}% (5日变化 {:+.2}%) | MACD DIF/DEA/HIST = {:.3}/{:.3}/{:.3} | 背离: {:?}\n",
                bm.band_width_pct, bm.band_change_pct,
                bm.macd_dif, bm.macd_dea, bm.macd_hist, bm.macd_div
            ));
            context.push_str(&format!("解读: {}\n", bm.reason));
        }

        // 宏观市场背景（如有则注入 prompt）
        let macro_section = match macro_context {
            Some(mc) if !mc.is_empty() => format!(
                "\n\n---\n\n## 📡 宏观市场背景（请评估下列最新事件对本股的潜在影响）\n\n{}\n\n---",
                mc
            ),
            _ => String::new(),
        };

        let prompt = format!(
            "请基于以下数据分析该股，按【宏观影响】【技术面】【主力资金】【基本面】【操作建议（含买入价/目标价/止损位）】【风险提示】六段输出，每段不超过 3 句。\n\
            若上下文含【布林+MACD 共振信号】段：TopSell→不得建议买入，评分压在 50 以下；BottomBuy→可买入但仓位≤30%；UptrendStart→可加仓至 60%；PreReversal→仅观察。\n\n{}{}",
            context,
            macro_section
        );

        // 调用API（使用文本分析专用系统提示词）
        self.call_api_with_retry_ex(&prompt, Self::TEXT_SYSTEM_PROMPT).await
    }

    /// 分析单只股票
    pub async fn analyze(
        &mut self,
        context: &HashMap<String, Value>,
        news_context: Option<&str>,
    ) -> AnalysisResult {
        let code = context
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        // 获取股票名称
        let name = self.get_stock_name(context, &code);

        // 检查可用性
        if !self.is_available() {
            return AnalysisResult {
                code,
                name,
                sentiment_score: 50,
                trend_prediction: "震荡".to_string(),
                operation_advice: "持有".to_string(),
                confidence_level: "低".to_string(),
                dashboard: None,
                trend_analysis: String::new(),
                short_term_outlook: String::new(),
                medium_term_outlook: String::new(),
                technical_analysis: String::new(),
                ma_analysis: String::new(),
                volume_analysis: String::new(),
                pattern_analysis: String::new(),
                fundamental_analysis: String::new(),
                sector_position: String::new(),
                company_highlights: String::new(),
                news_summary: String::new(),
                market_sentiment: String::new(),
                hot_topics: String::new(),
                analysis_summary: "AI 分析功能未启用（未配置 API Key）".to_string(),
                key_points: String::new(),
                risk_warning: "请配置 API Key 后重试".to_string(),
                buy_reason: String::new(),
                raw_response: None,
                search_performed: false,
                data_sources: String::new(),
                success: false,
                error_message: Some("API Key 未配置".to_string()),
            };
        }

        // 请求前延迟
        if self.config.request_delay > 0.0 {
            debug!(
                "[LLM] 请求前等待 {:.1} 秒...",
                self.config.request_delay
            );
            tokio::time::sleep(Duration::from_secs_f64(self.config.request_delay)).await;
        }

        info!("========== AI 分析 {}({}) ==========", name, code);
        info!("[LLM配置] 模型: {}", self.current_model.borrow());
        info!(
            "[LLM配置] 是否包含新闻: {}",
            if news_context.is_some() { "是" } else { "否" }
        );

        // 多 Agent 流水线在 analyze_stock_with_extras 中处理。
        // analyze() 仍走旧版单 prompt + JSON 结构化路径（用于历史调用方）。

        // 旧版：单 Agent prompt
        let prompt = self.format_prompt(context, &name, news_context);
        info!("[LLM配置] Prompt 长度: {} 字符", prompt.len());

        // 调用 API
        let start_time = Instant::now();
        match self.call_api_with_retry(&prompt).await {
            Ok(response_text) => {
                let elapsed = start_time.elapsed().as_secs_f64();
                info!(
                    "[LLM返回] API 响应成功, 耗时 {:.2}s, 响应长度 {} 字符",
                    elapsed,
                    response_text.len()
                );

                // 解析响应
                let mut result = self.parse_response(&response_text, &code, &name);
                result.raw_response = Some(response_text);
                result.search_performed = news_context.is_some();

                info!(
                    "[LLM解析] {}({}) 分析完成: {}, 评分 {}",
                    name, code, result.trend_prediction, result.sentiment_score
                );

                result
            }
            Err(e) => {
                error!("AI 分析 {}({}) 失败: {}", name, code, e);
                AnalysisResult {
                    code,
                    name,
                    sentiment_score: 50,
                    trend_prediction: "震荡".to_string(),
                    operation_advice: "持有".to_string(),
                    confidence_level: "低".to_string(),
                    dashboard: None,
                    trend_analysis: String::new(),
                    short_term_outlook: String::new(),
                    medium_term_outlook: String::new(),
                    technical_analysis: String::new(),
                    ma_analysis: String::new(),
                    volume_analysis: String::new(),
                    pattern_analysis: String::new(),
                    fundamental_analysis: String::new(),
                    sector_position: String::new(),
                    company_highlights: String::new(),
                    news_summary: String::new(),
                    market_sentiment: String::new(),
                    hot_topics: String::new(),
                    analysis_summary: format!("分析过程出错: {}", &e.to_string()[..100.min(e.to_string().len())]),
                    key_points: String::new(),
                    risk_warning: "分析失败，请稍后重试或手动分析".to_string(),
                    buy_reason: String::new(),
                    raw_response: None,
                    search_performed: false,
                    data_sources: String::new(),
                    success: false,
                    error_message: Some(e.to_string()),
                }
            }
        }
    }

}
