//! 单股分析入口（从 analyzer.rs 拆分）。
//!
//! 仅保留 `analyze_stock` / `analyze_stock_with_extras` 两个对外方法。

use anyhow::{anyhow, Result};
use log::info;

use super::GeminiAnalyzer;

impl GeminiAnalyzer {
    /// 简化的股票分析方法（用于pipeline）
    pub async fn analyze_stock(
        &self,
        code: &str,
        kline_data: &[crate::data_provider::KlineData],
        macro_context: Option<&str>,
    ) -> Result<String> {
        self.analyze_stock_with_extras(code, None, kline_data, macro_context, None, None, None)
            .await
    }

    /// 扩展版：允许调用方注入真实口径的资金流 / 分时 / 龙虎榜席位等额外 prompt 片段。
    /// 当 `tech_assessment` 提供时，AI 必须按系统评分规则解释同一份评分（同一把尺子，不同表述）。
    pub async fn analyze_stock_with_extras(
        &self,
        code: &str,
        _name: Option<&str>,
        kline_data: &[crate::data_provider::KlineData],
        macro_context: Option<&str>,
        extra_context: Option<&str>,
        _news_context: Option<&str>,
        tech_assessment: Option<&crate::analyzer::TechAssessment<'_>>,
    ) -> Result<String> {
        if kline_data.is_empty() {
            return Err(anyhow!("数据为空"));
        }

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

        // ========== MACD (6,13,5) ==========
        if data_len >= 13 {
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
            let ema6 = ema(6, &chron);
            let ema13 = ema(13, &chron);
            let diff: Vec<f64> = ema6.iter().zip(ema13.iter()).map(|(a,b)| a-b).collect();
            let dea = ema(5, &diff);
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

        // ========== SKDJ(40,5) ==========
        if data_len >= 40 {
            let chron: Vec<&crate::data_provider::KlineData> = kline_data.iter().rev().collect();
            let mut k_val = 50.0;
            let mut d_val = 50.0;
            let n_len = chron.len();
            let start = n_len.saturating_sub(60).max(39); // 保证一定的预热期收敛
            let alpha = 2.0 / (5.0 + 1.0);
            for i in start..n_len {
                let window_start = i.saturating_sub(39);
                let window = &chron[window_start..=i];
                let hh = window.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
                let ll = window.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
                let rsv = if (hh - ll).abs() < 1e-9 { 50.0 }
                    else { (chron[i].close - ll) / (hh - ll) * 100.0 };
                k_val = alpha * rsv + (1.0 - alpha) * k_val;
                d_val = alpha * k_val + (1.0 - alpha) * d_val;
            }
            let j_val = 3.0 * k_val - 2.0 * d_val;
            context.push_str(&format!(
                "【SKDJ】K={:.2} D={:.2} J={:.2}\n",
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

        // ========== 估值历史分位（近 3 年 PE/PB） ==========
        if let Some(vh) = latest.valuation_history.as_ref() {
            if vh.sample_days >= 30 {
                context.push_str(&format!(
                    "\n【估值分位】（近 {} 个交易日, {} ~ {}）\n",
                    vh.sample_days,
                    vh.oldest_date.as_deref().unwrap_or("-"),
                    vh.newest_date.as_deref().unwrap_or("-"),
                ));
                let label = |pct: f64| -> &'static str {
                    if pct < 20.0 { "极低估⭐" }
                    else if pct < 40.0 { "低估" }
                    else if pct < 60.0 { "中位" }
                    else if pct < 80.0 { "高估" }
                    else { "极高估⚠️" }
                };
                if let (Some(cur), Some(pct)) = (vh.current_pe, vh.pe_percentile) {
                    context.push_str(&format!(
                        "PE TTM: {:.2}  历史分位 {:.1}% ({})  区间 [{:.2}, {:.2}] 中位 {:.2}\n",
                        cur, pct, label(pct),
                        vh.pe_min.unwrap_or(f64::NAN),
                        vh.pe_max.unwrap_or(f64::NAN),
                        vh.pe_median.unwrap_or(f64::NAN),
                    ));
                }
                if let (Some(cur), Some(pct)) = (vh.current_pb, vh.pb_percentile) {
                    context.push_str(&format!(
                        "PB MRQ: {:.2}  历史分位 {:.1}% ({})  区间 [{:.2}, {:.2}] 中位 {:.2}\n",
                        cur, pct, label(pct),
                        vh.pb_min.unwrap_or(f64::NAN),
                        vh.pb_max.unwrap_or(f64::NAN),
                        vh.pb_median.unwrap_or(f64::NAN),
                    ));
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

        // ========== 财务趋势（多期序列） ==========
        if let Some(hist) = latest.financials_history.as_ref() {
            // 仅在 >=2 期时才输出趋势；取最近 8 期（约 2 年季报）以控制 prompt 体积
            let show: Vec<_> = hist.iter().take(8).collect();
            if show.len() >= 2 {
                context.push_str("\n【财务趋势】（最近多期，由新到旧）\n");
                for p in &show {
                    let date = p.report_date.as_deref().unwrap_or("-");
                    let mut parts: Vec<String> = Vec::new();
                    if let Some(v) = p.eps { parts.push(format!("EPS {:.3}", v)); }
                    if let Some(v) = p.roe { parts.push(format!("ROE {:.2}%", v)); }
                    if let Some(v) = p.gross_margin { parts.push(format!("毛利 {:.2}%", v)); }
                    if let Some(v) = p.net_margin { parts.push(format!("净利 {:.2}%", v)); }
                    if let Some(v) = p.revenue_yoy { parts.push(format!("营收YoY {:.2}%", v)); }
                    if let Some(v) = p.net_profit_yoy { parts.push(format!("净利YoY {:.2}%", v)); }
                    if let Some(v) = p.op_cash_flow_ps { parts.push(format!("每股CFO {:.3}", v)); }
                    context.push_str(&format!("{}: {}\n", date, parts.join(" | ")));
                }

                // 趋势判定：ROE / 毛利率 / 营收YoY 是否单调上升或下降
                let trend = |get: fn(&crate::data_provider::FinancialPeriod) -> Option<f64>| -> Option<&'static str> {
                    let vals: Vec<f64> = show.iter().filter_map(|p| get(p)).collect();
                    if vals.len() < 3 { return None; }
                    // hist 是从新到旧，反转为时间正序后做趋势判断
                    let chrono: Vec<f64> = vals.iter().rev().cloned().collect();
                    let up = chrono.windows(2).all(|w| w[1] >= w[0] - 0.01);
                    let down = chrono.windows(2).all(|w| w[1] <= w[0] + 0.01);
                    if up && !down { Some("持续上行") }
                    else if down && !up { Some("持续下行") }
                    else { None }
                };
                if let Some(t) = trend(|p| p.roe) {
                    context.push_str(&format!("ROE趋势: {}\n", t));
                }
                if let Some(t) = trend(|p| p.gross_margin) {
                    context.push_str(&format!("毛利率趋势: {}\n", t));
                }
                if let Some(t) = trend(|p| p.revenue_yoy) {
                    context.push_str(&format!("营收增速趋势: {}\n", t));
                }
            }

            // ========== 盈利质量（经营现金流 vs 净利润） ==========
            let ratios: Vec<(String, f64)> = show
                .iter()
                .filter_map(|p| {
                    p.cfo_to_ni_ratio()
                        .map(|r| (p.report_date.clone().unwrap_or_else(|| "-".into()), r))
                })
                .collect();
            if !ratios.is_empty() {
                context.push_str("\n【盈利质量】（CFO/净利润，>=1 优秀 / 0.5-1 健康 / <0.5 偏弱 / <=0 风险）\n");
                for (date, r) in &ratios {
                    let tag = if *r <= 0.0 { "风险⚠️" }
                              else if *r < 0.5 { "偏弱" }
                              else if *r < 1.0 { "健康" }
                              else { "优秀" };
                    context.push_str(&format!("{}: {:.2} ({})\n", date, r, tag));
                }
                let avg = ratios.iter().map(|(_, r)| r).sum::<f64>() / ratios.len() as f64;
                context.push_str(&format!("近{}期均值: {:.2}\n", ratios.len(), avg));
                if avg < 0.3 {
                    context.push_str("⚠️ 盈利质量风险：CFO/净利润长期偏低，需警惕应收账款堆积或利润含金量不足\n");
                } else if avg < 0.6 {
                    context.push_str("提示：CFO/净利润偏弱，建议关注应收/存货周转是否恶化\n");
                }
            }

            // ========== 杜邦分解（ROE = 净利率 × 总资产周转率 × 权益乘数） ==========
            let dupont_rows: Vec<(String, f64, f64, f64, f64, Option<f64>)> = show
                .iter()
                .filter_map(|p| {
                    let (nm, at, em, theo) = p.dupont()?;
                    Some((
                        p.report_date.clone().unwrap_or_else(|| "-".into()),
                        nm, at, em, theo, p.roe,
                    ))
                })
                .collect();
            if !dupont_rows.is_empty() {
                context.push_str("\n【杜邦分解】ROE = 净利率 × 总资产周转率 × 权益乘数\n");
                context.push_str("报告期 | 净利率% | 周转率(次) | 权益乘数 | 理论ROE% | 实际ROE%\n");
                for (date, nm, at, em, theo, actual) in &dupont_rows {
                    let actual_str = actual
                        .map(|v| format!("{:.2}", v))
                        .unwrap_or_else(|| "-".into());
                    context.push_str(&format!(
                        "{} | {:.2} | {:.2} | {:.2} | {:.2} | {}\n",
                        date, nm, at, em, theo, actual_str
                    ));
                }
                // 趋势判定：取首尾两期对比驱动因子
                if dupont_rows.len() >= 2 {
                    let (_, nm_l, at_l, em_l, _, _) = &dupont_rows[0];
                    let (_, nm_e, at_e, em_e, _, _) = &dupont_rows[dupont_rows.len() - 1];
                    let d_nm = nm_l - nm_e;
                    let d_at = at_l - at_e;
                    let d_em = em_l - em_e;
                    let mut drivers: Vec<String> = Vec::new();
                    if d_nm.abs() > 1.0 {
                        drivers.push(format!("净利率{:+.2}pp", d_nm));
                    }
                    if d_at.abs() > 0.05 {
                        drivers.push(format!("周转率{:+.2}次", d_at));
                    }
                    if d_em.abs() > 0.1 {
                        drivers.push(format!("权益乘数{:+.2}", d_em));
                    }
                    if !drivers.is_empty() {
                        context.push_str(&format!(
                            "驱动因子（最新 vs 最早）: {}\n",
                            drivers.join("、")
                        ));
                    }
                    if d_em > 0.3 && d_nm < 0.0 {
                        context.push_str(
                            "⚠️ ROE 上升主要由加杠杆驱动而非盈利能力改善，注意债务风险\n",
                        );
                    } else if d_nm > 0.0 && d_at > 0.0 {
                        context.push_str("💎 ROE 改善由盈利与运营双轮驱动，质量较高\n");
                    }
                }
            }

            // ========== 财务异常信号（启发式红旗评分） ==========
            if let Some(q) = crate::data_provider::assess_quality(hist) {
                if !q.flags.is_empty() {
                    context.push_str(&format!(
                        "\n【财务异常信号】风险评分 {}/100 ({})\n",
                        q.risk_score, q.level
                    ));
                    for flag in &q.flags {
                        context.push_str(&format!("• {}\n", flag));
                    }
                }
            }
        }

        if let Some(sharpe) = latest.sharpe_ratio {
            context.push_str(&format!("\n夏普比率: {:.2}\n", sharpe));
        }

        // ========== 卖方一致预期 ==========
        if let Some(cs) = latest.consensus.as_ref() {
            if cs.report_count > 0 {
                context.push_str(&format!(
                    "\n【卖方一致预期】近6个月 {} 份研报 / {} 家券商覆盖\n",
                    cs.report_count, cs.broker_count
                ));
                if let Some(eps_t) = cs.eps_this_year_avg {
                    let mut eps_line = format!("当年EPS预测均值: {:.2}", eps_t);
                    if let Some(eps_n) = cs.eps_next_year_avg {
                        let growth = if eps_t.abs() > 1e-6 {
                            format!(" (隐含同比 {:+.1}%)", (eps_n - eps_t) / eps_t.abs() * 100.0)
                        } else {
                            String::new()
                        };
                        eps_line.push_str(&format!(" | 明年: {:.2}{}", eps_n, growth));
                    }
                    if let Some(eps_n2) = cs.eps_next2_year_avg {
                        eps_line.push_str(&format!(" | 后年: {:.2}", eps_n2));
                    }
                    context.push_str(&format!("{}\n", eps_line));
                }
                if !cs.rating_distribution.is_empty() {
                    let mut parts: Vec<(String, u32)> = cs
                        .rating_distribution
                        .iter()
                        .map(|(k, v)| (k.clone(), *v))
                        .collect();
                    parts.sort_by(|a, b| b.1.cmp(&a.1));
                    let dist_str: Vec<String> = parts
                        .iter()
                        .map(|(k, v)| format!("{} {}", k, v))
                        .collect();
                    let bull = cs.bullish_ratio().unwrap_or(0.0);
                    context.push_str(&format!(
                        "评级分布: {} | 看多比例: {:.0}%\n",
                        dist_str.join(" / "),
                        bull
                    ));
                }
                if let (Some(low), Some(high)) =
                    (cs.target_price_low_avg, cs.target_price_high_avg)
                {
                    let cur = latest.close;
                    let upside = cs.upside_pct(cur).unwrap_or(0.0);
                    context.push_str(&format!(
                        "目标价区间: ¥{:.2} ~ ¥{:.2} (当前 ¥{:.2}, 上限空间 {:+.1}%)\n",
                        low, high, cur, upside
                    ));
                } else if let Some(high) = cs.target_price_high_avg {
                    let cur = latest.close;
                    let upside = cs.upside_pct(cur).unwrap_or(0.0);
                    context.push_str(&format!(
                        "目标价均值: ¥{:.2} (当前 ¥{:.2}, 空间 {:+.1}%)\n",
                        high, cur, upside
                    ));
                }
                if let Some(date) = &cs.latest_report_date {
                    context.push_str(&format!("最近研报: {}\n", date));
                }
                if !cs.recent_reports.is_empty() {
                    context.push_str("近3份研报:\n");
                    for r in &cs.recent_reports {
                        context.push_str(&format!(
                            "• [{}] {} | {} | {}\n",
                            r.publish_date, r.org_name, r.rating, r.title
                        ));
                    }
                }
            }
        }

        // ========== 行业横向对标 ==========
        if let Some(ib) = latest.industry.as_ref() {
            if ib.peer_count >= 3 {
                context.push_str(&format!(
                    "\n【行业对标】{}（{}，共 {} 家同业）\n",
                    ib.industry_name, ib.board_code, ib.peer_count
                ));
                let fmt_opt = |v: Option<f64>| match v {
                    Some(x) => format!("{:.2}", x),
                    None => "-".to_string(),
                };
                let fmt_pct = |v: Option<f64>| match v {
                    Some(x) => format!("{:.0}%", x),
                    None => "-".to_string(),
                };
                context.push_str("指标      | 个股       | 行业中位数 | 百分位\n");
                context.push_str(&format!(
                    "PE(TTM)   | {:>10} | {:>10} | {:>6} ({})\n",
                    fmt_opt(ib.stock_pe),
                    fmt_opt(ib.median_pe),
                    fmt_pct(ib.pe_percentile),
                    "越低越便宜"
                ));
                context.push_str(&format!(
                    "PB        | {:>10} | {:>10} | {:>6} ({})\n",
                    fmt_opt(ib.stock_pb),
                    fmt_opt(ib.median_pb),
                    fmt_pct(ib.pb_percentile),
                    "越低越便宜"
                ));
                context.push_str(&format!(
                    "ROE(单季) | {:>10} | {:>10} | {:>6} ({})\n",
                    fmt_opt(ib.stock_roe),
                    fmt_opt(ib.median_roe),
                    fmt_pct(ib.roe_percentile),
                    "越高越好"
                ));
                context.push_str(&format!(
                    "净利同比% | {:>10} | {:>10} | {:>6} ({})\n",
                    fmt_opt(ib.stock_growth),
                    fmt_opt(ib.median_growth),
                    fmt_pct(ib.growth_percentile),
                    "越高越好"
                ));
                // 行业地位定性结论
                let mut tags: Vec<String> = Vec::new();
                if let Some(p) = ib.roe_percentile {
                    if p >= 75.0 {
                        tags.push("💎 ROE 领先同业（前 25%）".to_string());
                    } else if p <= 25.0 {
                        tags.push("⚠️ ROE 落后同业（后 25%）".to_string());
                    }
                }
                if let Some(p) = ib.pe_percentile {
                    if p <= 25.0 {
                        tags.push("💰 估值低于多数同业（前 25% 便宜）".to_string());
                    } else if p >= 75.0 {
                        tags.push("📈 估值高于多数同业（后 25% 偏贵）".to_string());
                    }
                }
                if let Some(p) = ib.growth_percentile {
                    if p >= 75.0 {
                        tags.push("🚀 业绩增速领先同业".to_string());
                    } else if p <= 25.0 {
                        tags.push("📉 业绩增速落后同业".to_string());
                    }
                }
                if !tags.is_empty() {
                    context.push_str(&format!("行业地位: {}\n", tags.join("; ")));
                }
            }
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

        // 系统技术评分（与 AI 共用同一把尺子）
        let rubric_section = match tech_assessment {
            Some(ta) => {
                let reasons = if ta.reasons.is_empty() {
                    "（无显著加分项）".to_string()
                } else {
                    ta.reasons.iter().map(|s| format!("  · {}", s)).collect::<Vec<_>>().join("\n")
                };
                let risks = if ta.risks.is_empty() {
                    "（无显著扣分项）".to_string()
                } else {
                    ta.risks.iter().map(|s| format!("  · {}", s)).collect::<Vec<_>>().join("\n")
                };
                let breakdown_line = match ta.score_breakdown {
                    Some(sb) => format!(
                        "  五维评分: 技术 {}/100 | 盈利质量 {}/100 | 估值安全 {}/100 | 资金面 {}/100 | 增长可持续 {}/100\n",
                        sb.technical,
                        sb.fundamental_quality,
                        sb.valuation_safety,
                        sb.capital_flow,
                        sb.growth_sustainability,
                    ),
                    None => String::new(),
                };
                let trade_type_line = match ta.trade_type {
                    Some(tt) if !tt.trim().is_empty() => format!("  系统判定交易类型: {}\n", tt),
                    _ => String::new(),
                };
                let veto_line = if ta.veto_flags.is_empty() {
                    String::new()
                } else {
                    let flags = ta.veto_flags
                        .iter()
                        .map(|s| format!("  · {}", s))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("  已触发风险否决信号（必须在【风险提示】段中逐条覆盖）:\n{}\n", flags)
                };
                format!(
                    "\n\n[系统技术评分 - 你与系统共用同一把尺子]\n\
                    评分规则: 均线排列 35 + 乖离率 30 + 量能 20 + 动量指标 ±10 + 夏普 5 + 支撑位 10 ± BB+MACD 共振 ≤15，0-100。\n\
                    档位规则: 80-100 强烈建议买入 | 60-79 建议买入 | 40-59 观望 | 20-39 建议减仓 | 0-19 建议卖出。\n\
                    系统评分: {score}/100 → {advice}\n\
                    系统趋势状态: {trend}\n\
                    {breakdown}{trade_type}{veto}\
                    评分加分项:\n{reasons}\n\
                    评分扣分项:\n{risks}\n",
                    score = ta.score,
                    advice = ta.advice,
                    trend = ta.trend_status,
                    breakdown = breakdown_line,
                    trade_type = trade_type_line,
                    veto = veto_line,
                    reasons = reasons,
                    risks = risks,
                )
            }
            None => String::new(),
        };

        // 仅当注入了系统评分时，加严输出约束，确保 AI 与评分同一标准
        let alignment_rules = if let Some(ta) = tech_assessment {
            // Phase 4: 技术趋势与操作建议方向相反时，强制独立段落披露
            let trend = ta.trend_status;
            let advice = ta.advice;
            let bearish_trend = trend.contains("空头") || trend.contains("下跌");
            let bullish_advice = advice.contains("买入") || advice.contains("加仓");
            let bullish_trend = trend.contains("多头") || trend.contains("上涨");
            let bearish_advice = advice.contains("卖出") || advice.contains("减仓");
            let contrarian_clause = if (bearish_trend && bullish_advice) || (bullish_trend && bearish_advice) {
                format!(
                    "\n【一致性强制规则 - 触发】系统趋势状态为『{trend}』但操作建议为『{advice}』，方向相反。\n\
                    必须在分析的开头（【技术面】之前）插入一个独立段落 ##【⚠️ 逆势布局逻辑】，明确说明：\n\
                      a) 这是一次『逆势/左侧』操作，非顺势之举；\n\
                      b) 量化此类信号的历史胜率（若无可靠统计样本，必须直白注明『缺乏统计依据』，不得编造数字）；\n\
                      c) 明确给出触发离场的反向证据（例如『若 5 日内未能站上 MA10 则止损』）。\n\
                    严禁使用『虽然...但是...』『尽管...仍然...』『风险可控』等淡化矛盾的话术。\n",
                    trend = trend,
                    advice = advice,
                )
            } else {
                String::new()
            };
            let veto_clause = if ta.veto_flags.is_empty() {
                String::new()
            } else {
                let bulleted = ta.veto_flags
                    .iter()
                    .map(|s| format!("  · {}", s))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "\n【风险否决强制规则 - 触发】已触发以下系统否决信号：\n{bulleted}\n\
                    【风险提示】段必须逐条引用上述每一条信号原文（可改写措辞但语义不得弱化），并给出对应的应对措施；\n\
                    严禁忽略、合并或淡化这些信号；不得在【操作建议】中提出比系统建议更激进的仓位（系统建议已包含仓位上限）。\n",
                    bulleted = bulleted,
                )
            };
            format!(
                "\n输出硬性约束（评分一致性）：\n\
                1) 【技术面】必须按上文\"评分加分项/扣分项\"逐项复述，不得引入未在评分明细中的技术指标。\n\
                2) 【操作建议】结论必须严格等于上文\"系统评分→档位\"映射，不得自行降档或升档。\n\
                3) 【消息面】包括投资者关系、行业新闻、产业上下游、政策法规等，但仅作背景说明，不得作为评分的主要依据。\n\
                4) 【基本面】【主力资金】【宏观影响】仅作背景说明；若发现严重背离，写入【风险提示】，不得改变操作建议档位。\n\
                5) 不要输出任何与系统评分相矛盾的总结性结论。\n\
                {contrarian}{veto}",
                contrarian = contrarian_clause,
                veto = veto_clause,
            )
        } else { String::new() };

        let prompt = format!(
            "请基于以下数据分析该股，仅输出【宏观影响】【消息面】【技术面】【主力资金】【基本面】【操作建议（含买入价/目标价/止损位）】【风险提示】七段，每段不超过 3 句。\n\
            输出格式约束：使用 Markdown 标题（##/###）、不要使用引号块、不要复述输入数据中的章节标题（例如\"系统技术评分\"\"宏观市场背景\"），仅输出七段中文段落，每段以【XX】开头。\n\
            若上下文含【布林+MACD 共振信号】段：TopSell→不得建议买入，评分压在 50 以下；BottomBuy→可买入但仓位≤30%；UptrendStart→可加仓至 60%；PreReversal→仅观察。{alignment}\n\n{ctx}{rubric}{macro_}",
            alignment = alignment_rules,
            ctx = context,
            rubric = rubric_section,
            macro_ = macro_section
        );

        // 标准模式：单次 LLM 调用，不触发工具循环（深度多智能体走 deep_analyzer 路径）
        info!(">>> [{}] 标准模式：单次 LLM 调用", code);
        self.call_api_with_retry_ex(&prompt, Self::TEXT_SYSTEM_PROMPT).await
    }
}
