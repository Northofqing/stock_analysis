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
        self.analyze_stock_with_extras(code, kline_data, macro_context, None).await
    }

    /// 扩展版：允许调用方注入真实口径的资金流 / 分时 / 龙虎榜席位等额外 prompt 片段。
    pub async fn analyze_stock_with_extras(
        &self,
        code: &str,
        kline_data: &[crate::data_provider::KlineData],
        macro_context: Option<&str>,
        extra_context: Option<&str>,
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

        // 多头/空头排列判断
        if let (Some(m5), Some(m10), Some(m20)) = (ma5, ma10, ma20) {
            let alignment = if m5 > m10 && m10 > m20 {
                "多头排列 ✅ (MA5>MA10>MA20)"
            } else if m5 < m10 && m10 < m20 {
                "空头排列 ❌ (MA5<MA10<MA20)"
            } else {
                "均线粘合/交叉，趋势不明"
            };
            context.push_str(&format!("均线排列: {}\n", alignment));
        }

        // 乖离率
        if let Some(m5) = ma5 {
            if m5 > 0.0 {
                let bias5 = (latest.close - m5) / m5 * 100.0;
                let bias_warning = if bias5.abs() > 5.0 { "⚠️ 偏离过大" }
                    else if bias5.abs() > 2.0 { "注意回归" }
                    else { "正常范围" };
                context.push_str(&format!("MA5乖离率: {:.2}% ({})\n", bias5, bias_warning));
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

        // ========== 量能分析 ==========
        context.push_str("\n【量能分析】\n");
        if data_len >= 5 {
            let vol_5d_avg = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
            if vol_5d_avg > 0.0 {
                let volume_ratio = latest.volume / vol_5d_avg;
                let vol_status = if volume_ratio > 2.0 { "显著放量" }
                    else if volume_ratio > 1.2 { "温和放量" }
                    else if volume_ratio > 0.8 { "量能平稳" }
                    else { "明显缩量" };
                context.push_str(&format!("5日量比: {:.2} ({})\n", volume_ratio, vol_status));
            }
        }
        if data_len >= 10 {
            let vol_10d_avg = kline_data[..10].iter().map(|k| k.volume).sum::<f64>() / 10.0;
            if vol_10d_avg > 0.0 {
                let volume_ratio_10 = latest.volume / vol_10d_avg;
                context.push_str(&format!("10日量比: {:.2}\n", volume_ratio_10));
            }
        }

        // ========== 主力资金动向（代理指标） ==========
        context.push_str("\n【主力资金动向（代理）】\n");
        if data_len >= 5 {
            let vol_5d_avg = kline_data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0;
            let vol_ratio = if vol_5d_avg > 0.0 { latest.volume / vol_5d_avg } else { 1.0 };
            let money_flow = if vol_ratio > 1.5 && latest.pct_chg > 1.0 {
                "🔥 放量上涨 — 主力介入迹象"
            } else if vol_ratio > 1.5 && latest.pct_chg < -1.0 {
                "⚠️ 放量下跌 — 主力出货迹象"
            } else if vol_ratio < 0.7 && latest.pct_chg > 0.5 {
                "缩量上涨 — 惜售但动能不足"
            } else if vol_ratio < 0.7 && latest.pct_chg < -0.5 {
                "缩量下跌 — 抛压减弱"
            } else if vol_ratio > 1.3 && latest.pct_chg.abs() < 1.0 {
                "高换手+横盘 — 筹码交换，关注突破方向"
            } else {
                "量价关系平稳，无明显主力动向"
            };
            context.push_str(&format!("代理判断: {}\n", money_flow));
            if let Some(turnover) = latest.turnover_rate {
                context.push_str(&format!("换手率: {:.2}%（>7%活跃，>15%火热）\n", turnover));
            }
        }

        // ========== 涨跌停识别 ==========
        let is_gem = code.starts_with("300") || code.starts_with("301"); // 创业板
        let is_star = code.starts_with("688"); // 科创板
        let is_bj = code.starts_with("8") || code.starts_with("9") || code.starts_with("4"); // 北交所
        // ST 股通过股票名称判断（若可用），此处保守不考虑 ST
        let limit_pct = if is_gem || is_star { 20.0 }
            else if is_bj { 30.0 }
            else { 10.0 };
        if latest.pct_chg >= limit_pct - 0.3 {
            context.push_str(&format!("\n【涨跌停】🚀 今日涨停（涨幅 {:.2}% / 涨停阈值 {}%）— ", latest.pct_chg, limit_pct));
            // 检查连板数
            let mut consec = 1;
            for k in kline_data[1..].iter().take(10) {
                if k.pct_chg >= limit_pct - 0.3 { consec += 1; } else { break; }
            }
            if consec >= 2 {
                context.push_str(&format!("连续 {} 板！情绪推动风险陡增，建议观望\n", consec));
            } else {
                context.push_str("首板，非追高时机，次日低开可关注\n");
            }
        } else if latest.pct_chg <= -(limit_pct - 0.3) {
            context.push_str(&format!("\n【涨跌停】📉 今日跌停（跌幅 {:.2}%）— 承压严重，规避\n", latest.pct_chg));
        } else if latest.pct_chg >= 5.0 {
            context.push_str(&format!("\n【涨跌停】📈 大涨 {:.2}%（未涨停）— 短期强势，警惕乖离扩大\n", latest.pct_chg));
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
                "金叉 ✅"
            } else if diff[n-1] < dea[n-1] && n >= 2 && diff[n-2] >= dea[n-2] {
                "死叉 ❌"
            } else if diff[n-1] > dea[n-1] {
                "多头区间"
            } else {
                "空头区间"
            };
            context.push_str(&format!(
                "\n【MACD】DIFF: {:.3}, DEA: {:.3}, MACD柱: {:.3} ({})\n",
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
            let rsi_signal = if rsi > 80.0 { "严重超买 🔴" }
                else if rsi > 70.0 { "超买" }
                else if rsi < 20.0 { "严重超卖 🟢" }
                else if rsi < 30.0 { "超卖" }
                else { "正常" };
            context.push_str(&format!("【RSI(14)】{:.2} ({})\n", rsi, rsi_signal));
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
            let kdj_signal = if k_val > 80.0 && d_val > 80.0 { "超买区" }
                else if k_val < 20.0 && d_val < 20.0 { "超卖区" }
                else if k_val > d_val { "多头" }
                else { "空头" };
            context.push_str(&format!(
                "【KDJ】K: {:.2}, D: {:.2}, J: {:.2} ({})\n",
                k_val, d_val, j_val, kdj_signal
            ));
        }

        // ========== 52周（约250个交易日）高低价 ==========
        context.push_str("\n【价格区间指标】\n");
        let week52_len = data_len.min(250);
        if week52_len >= 5 {
            let week52_data = &kline_data[..week52_len];
            let high_52w = week52_data.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let low_52w = week52_data.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos_in_range = if (high_52w - low_52w).abs() > 0.001 {
                (latest.close - low_52w) / (high_52w - low_52w) * 100.0
            } else {
                50.0
            };
            context.push_str(&format!(
                "52周最高: {:.2} | 52周最低: {:.2}\n\
                当前价位于52周区间: {:.1}% (0%=最低, 100%=最高)\n",
                high_52w, low_52w, pos_in_range
            ));
        }

        // 一季度（约60个交易日）高低价
        let quarter_len = data_len.min(60);
        if quarter_len >= 5 {
            let quarter_data = &kline_data[..quarter_len];
            let high_q = quarter_data.iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
            let low_q = quarter_data.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
            let pos_q = if (high_q - low_q).abs() > 0.001 {
                (latest.close - low_q) / (high_q - low_q) * 100.0
            } else {
                50.0
            };
            context.push_str(&format!(
                "近一季最高: {:.2} | 近一季最低: {:.2}\n\
                当前价位于季度区间: {:.1}%\n",
                high_q, low_q, pos_q
            ));
        }

        // ========== 近期走势明细（最近10个交易日） ==========
        let recent_len = data_len.min(10);
        if recent_len >= 2 {
            context.push_str("\n【近期走势】\n");
            context.push_str("日期 | 收盘价 | 涨跌幅 | 成交量\n");
            for k in kline_data[..recent_len].iter() {
                context.push_str(&format!(
                    "{} | {:.2} | {:.2}% | {:.0}\n",
                    k.date, k.close, k.pct_chg, k.volume
                ));
            }

            // 近5日/10日累计涨幅
            let chg_5d: f64 = kline_data[..data_len.min(5)].iter().map(|k| k.pct_chg).sum();
            context.push_str(&format!("近5日累计涨幅: {:.2}%\n", chg_5d));
            if recent_len >= 10 {
                let chg_10d: f64 = kline_data[..10].iter().map(|k| k.pct_chg).sum();
                context.push_str(&format!("近10日累计涨幅: {:.2}%\n", chg_10d));
            }

            // 波动率（近期日收益标准差）
            let returns: Vec<f64> = kline_data[..recent_len].iter().map(|k| k.pct_chg).collect();
            let mean_ret = returns.iter().sum::<f64>() / returns.len() as f64;
            let variance = returns.iter().map(|r| (r - mean_ret).powi(2)).sum::<f64>() / returns.len() as f64;
            let volatility = variance.sqrt();
            context.push_str(&format!("近期日波动率: {:.2}%\n", volatility));
        }

        // ========== 盈利指标（估值+财务） ==========
        if latest.pe_ratio.is_some() || latest.pb_ratio.is_some() || latest.market_cap.is_some() {
            context.push_str("\n【盈利水平指标】\n");
            
            // 估值指标
            if let Some(pe) = latest.pe_ratio {
                let pe_level = if pe < 0.0 { "亏损" }
                    else if pe < 15.0 { "估值合理" }
                    else if pe < 30.0 { "估值适中" }
                    else { "估值偏高" };
                context.push_str(&format!("市盈率(PE): {:.2} ({})\n", pe, pe_level));
            }
            
            if let Some(pb) = latest.pb_ratio {
                let pb_level = if pb < 1.0 { "可能被低估" }
                    else if pb < 3.0 { "市净率正常" }
                    else { "市净率较高" };
                context.push_str(&format!("市净率(PB): {:.2} ({})\n", pb, pb_level));
            }
            
            // 市值规模与流通性
            if let Some(market_cap) = latest.market_cap {
                let cap_type = if market_cap < 50.0 { "小盘股" }
                    else if market_cap < 300.0 { "中盘股" }
                    else if market_cap < 1000.0 { "大盘股" }
                    else { "超大盘股" };
                context.push_str(&format!("总市值: {:.2}亿元 ({})\n", market_cap, cap_type));
                
                if let Some(circ_cap) = latest.circulating_cap {
                    let circulation_ratio = (circ_cap / market_cap) * 100.0;
                    let liquidity = if circulation_ratio < 30.0 { "低流通，控盘严密" }
                        else if circulation_ratio < 70.0 { "中等流通" }
                        else { "高流通，交易自由" };
                    context.push_str(&format!("流通市值: {:.2}亿元 (流通比例: {:.1}%, {})\n", 
                        circ_cap, circulation_ratio, liquidity));
                }
            }
            
            // 交易活跃度
            if let Some(turnover) = latest.turnover_rate {
                let activity = if turnover < 1.0 { "极度清淡，关注度低" }
                    else if turnover < 3.0 { "交投清淡" }
                    else if turnover < 7.0 { "换手正常" }
                    else if turnover < 15.0 { "交易活跃" }
                    else { "换手火热，资金关注度高" };
                context.push_str(&format!("换手率: {:.2}% ({})\n", turnover, activity));
            }
            
            // 估值综合评估
            if let (Some(pe), Some(pb)) = (latest.pe_ratio, latest.pb_ratio) {
                if pe > 0.0 {
                    let pe_pb_ratio = pe / pb.max(0.1);
                    let valuation = if pe_pb_ratio < 3.0 && pe < 20.0 && pb < 2.0 {
                        "整体估值较低，具有投资价值"
                    } else if pe_pb_ratio < 5.0 && pe < 30.0 {
                        "估值适中"
                    } else {
                        "估值偏高，需谨慎"
                    };
                    context.push_str(&format!("估值综合评估: {}\n", valuation));
                }
            }
        }

        // ========== 财务指标（盈利能力+成长性） ==========
        let has_financials = latest.eps.is_some() || latest.roe.is_some()
            || latest.gross_margin.is_some() || latest.revenue_yoy.is_some();
        if has_financials {
            context.push_str("\n【财务指标】\n");

            if let Some(eps) = latest.eps {
                let eps_assessment = if eps < 0.0 { "亏损" }
                    else if eps < 0.5 { "盈利较弱" }
                    else if eps < 2.0 { "盈利正常" }
                    else { "盈利优秀" };
                context.push_str(&format!("每股收益(EPS): {:.3}元 ({})\n", eps, eps_assessment));
            }

            if let Some(roe) = latest.roe {
                let roe_assessment = if roe < 5.0 { "较低" }
                    else if roe < 15.0 { "正常" }
                    else if roe < 25.0 { "优秀" }
                    else { "卓越" };
                context.push_str(&format!("净资产收益率(ROE): {:.2}% ({})\n", roe, roe_assessment));
            }

            if let Some(gm) = latest.gross_margin {
                let gm_assessment = if gm < 20.0 { "竞争激烈" }
                    else if gm < 40.0 { "正常水平" }
                    else if gm < 60.0 { "竞争优势明显" }
                    else { "高壁垒行业" };
                context.push_str(&format!("毛利率: {:.2}% ({})\n", gm, gm_assessment));
            }

            if let Some(nm) = latest.net_margin {
                context.push_str(&format!("净利率: {:.2}%\n", nm));
            }

            if let Some(rev_yoy) = latest.revenue_yoy {
                let growth = if rev_yoy < 0.0 { "营收下滑" }
                    else if rev_yoy < 10.0 { "缓慢增长" }
                    else if rev_yoy < 30.0 { "稳健增长" }
                    else { "高速增长" };
                context.push_str(&format!("营收同比增长: {:.2}% ({})\n", rev_yoy, growth));
            }

            if let Some(profit_yoy) = latest.net_profit_yoy {
                let growth = if profit_yoy < -20.0 { "利润大幅下滑" }
                    else if profit_yoy < 0.0 { "利润下滑" }
                    else if profit_yoy < 20.0 { "利润稳定增长" }
                    else { "利润高速增长" };
                context.push_str(&format!("净利润同比增长: {:.2}% ({})\n", profit_yoy, growth));
            }
        }

        // 夏普比率
        if let Some(sharpe) = latest.sharpe_ratio {
            let sr_assessment = if sharpe < 0.0 { "风险调整后亏损" }
                else if sharpe < 1.0 { "一般" }
                else if sharpe < 2.0 { "良好" }
                else { "优秀" };
            context.push_str(&format!("\n夏普比率: {:.2} ({})\n", sharpe, sr_assessment));
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

        // 宏观市场背景（如有则注入 prompt）
        let macro_section = match macro_context {
            Some(mc) if !mc.is_empty() => format!(
                "\n\n---\n\n## 📡 宏观市场背景（请评估下列最新事件对本股的潜在影响）\n\n{}\n\n---",
                mc
            ),
            _ => String::new(),
        };

        let prompt = format!(
            "请分析以下股票的技术走势和基本面：\n\n{}{}\n\n\
            要求：\n\
            1. 【宏观影响】若有宏观背景信息，先评估国际/政策事件对本股及所属行业的影响；\n\
               - 当前大盘交易环境：牛市/熊市/震荡\n\
               - 情绪面：投资者情绪是偏乐观还是悲观，是否存在恐慌性抛售或过度追高\n\
               - 行业动态：所属行业是否处于景气周期，是否有利好/利空消息\n\
               - 板块联动：若宏观信息提及本股所属板块，强化看多逻辑；若仅提及其他板块，警惕跟涨乏力\n\
               - 地缘政治风险：是否处于敏感时期（如选举、战争、国际冲突等）\n\
            2. 【技术面】分析以下维度：\n\
               - 均线系统：MA5/MA10/MA20排列状态，是否多头/空头排列\n\
               - 乖离率：当前价格偏离MA5的程度，>5%警惕追高风险\n\
               - MACD/RSI/KDJ：金叉死叉、超买超卖信号的综合研判（数据见上下文）\n\
               - 价格位置：当前价位于52周和季度区间的位置，是否接近高点/低点\n\
               - 量价关系：量比变化，放量/缩量配合涨跌的含义\n\
               - 近期走势：近5-10日涨跌趋势和波动率\n\
               - 涨跌停研判：若触及涨跌停或大涨超 5%，按首板/连板/ST/创业板规则区别对待\n\
            3. 【主力资金动向】**若上下文存在\"【主力资金流向（真实口径）】\"段落，以真实数据为准，代理判断仅作补充**：\n\
               - 真实主力净流入 + 股价上涨 → 主力介入，看多倾向\n\
               - 真实主力净流出 + 股价上涨 → **诱多/拉高出货，警惕次日补跌**\n\
               - 真实主力净流入 + 股价下跌 → 主力低吸，可能见底\n\
               - 真实主力净流出 + 股价下跌 → 杀跌趋势，规避\n\
               - 近3日/近5日主力累计净流向比单日更可靠；若近3日持续流出即便今日股价上涨也需警惕\n\
               - 日内形态若为\"冲高回落\"/\"尾盘跳水\"/\"高开低走\" → 次日通常承压，避免追高\n\
               - 日内形态若为\"稳步推高\"/\"尾盘拉升\"/\"低开高走\" → 次日延续强势概率高\n\
               代理判断（无真实资金数据时使用）：放量上涨+高换手看多；放量下跌警戒；缩量回踩均线为理想买点\n\
            4. 【基本面】如果有盈利指标，请重点分析：\n\
               - 估值水平：PE、PB是否合理（PE<15优秀，15-30正常，>30偏高；PB<1低估，1-3正常，>3偏高）\n\
               - 盈利能力：EPS、ROE、毛利率、净利率反映的公司竞争力\n\
               - 成长性：营收和净利润同比增长率判断成长阶段\n\
               - 市值规模：小盘股成长性强但风险高，大盘股稳定但弹性小\n\
               - 公司业务亮点：是否有核心竞争力、行业地位、创新能力等\n\
               - 行业地位：在所属行业中的竞争位置\n\
               - 流通性与换手率：流通比例+换手率反映交易活跃度和可能的控盘情况\n\
               - 估值综合：结合PE、PB、EPS判断当前价格是否合理\n\
               - 夏普比率：风险调整后收益水平\n\
            5. 【操作建议与价位】基于技术面+资金面+基本面+消息面给出明确的操作建议：\n\
               - 操作：买入/加仓/持有/减仓/卖出/观望（乖离率>5%不追高，空头排列不做多，连板观望）\n\
               - **必须给出具体数字**：建议买入价 ¥X.XX 元，目标价（止盈）¥X.XX 元，止损位 ¥X.XX 元\n\
               - 止损位参考：MA20/近期前低/-8% 三者较高者；目标价参考：52周高点/季度高点/+15% 三者较低者\n\
            6. 【风险提示】指出主要风险因素（估值风险、技术风险、流动性风险、\
               52周高点压力、波动率异常、宏观风险、板块轮动风险等）\n\
            \n请简明扼要，重点突出，每个部分不超过3句话。",
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

        // 格式化提示词
        let prompt = self.format_prompt(context, &name, news_context);

        info!("========== AI 分析 {}({}) ==========", name, code);
        info!("[LLM配置] 模型: {}", self.current_model.borrow());
        info!("[LLM配置] Prompt 长度: {} 字符", prompt.len());
        info!(
            "[LLM配置] 是否包含新闻: {}",
            if news_context.is_some() { "是" } else { "否" }
        );

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
