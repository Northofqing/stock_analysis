//! 修复 Top10#3+#4 (2026-06-29 audit): pipeline/mod.rs (1765 行) 拆 4 个子模块
//!
//! 这个文件: `pipeline/analyze.rs` — 分析单只股票 (analyze_stock, 1020 行)
//!
//! 拆分后 mod.rs 只剩 ~580 行 (struct 定义 + new/with_limit_up_codes + 入口 run + enrich).
//!
//! Rust 允许跨模块 impl, 所以这里直接 `impl AnalysisPipeline { ... }`.

use anyhow::Result;
use log::{error, info, warn};
use std::sync::Arc;

use crate::data_provider::financials::FinancialPeriod;
use crate::data_provider::KlineData;
use crate::search_service::get_search_service;

use super::score_to_advice;
use super::section_utils;
use super::AnalysisPipeline;
use super::AnalysisResult;
use super::{
    extra_context, multi_timeframe, position_tracker, price_stats, score_breakdown,
    technical_report, trade_type, veto_rules,
};

/// BR-121: apply the already validated Boll/MACD evidence without IO or fallback data.
fn apply_boll_macd_adjustment(
    code: &str,
    trend_result: &mut crate::trend_analyzer::TrendAnalysisResult,
    bm: &crate::strategy::BollMacdSignal,
) {
    use crate::strategy::BollMacdAction;
    if bm.action == BollMacdAction::None {
        return;
    }
    let (delta, is_reason) = match bm.action {
        BollMacdAction::UptrendStart => (12, true),
        BollMacdAction::BottomBuy => (10, true),
        BollMacdAction::PreReversal => (3, true),
        BollMacdAction::TopSell => (-15, false),
        BollMacdAction::None => unreachable!("None returns before action adjustment"),
    };
    trend_result.signal_score = (trend_result.signal_score + delta).clamp(0, 100);
    let line = format!(
        "📊 BB+MACD: {} | {} ({:+})",
        bm.action.name(),
        bm.reason,
        delta
    );
    if is_reason {
        trend_result.signal_reasons.push(line);
    } else {
        trend_result.risk_factors.push(line);
    }
    if matches!(bm.action, BollMacdAction::TopSell) {
        use crate::trend_analyzer::BuySignal;
        if matches!(
            trend_result.buy_signal,
            BuySignal::StrongBuy | BuySignal::Buy
        ) {
            trend_result.buy_signal = BuySignal::Hold;
        }
        if trend_result.signal_score >= 60 {
            trend_result.signal_score = 55;
        }
    }
    info!(
        "[{}] 📊 布林+MACD 信号: {} | {} | 评分调整 {:+}",
        code,
        bm.action.name(),
        bm.reason,
        delta
    );
}

/// BR-121: score only present, already validated fundamental evidence.
fn apply_fundamental_adjustments(
    code: &str,
    trend_result: &mut crate::trend_analyzer::TrendAnalysisResult,
    latest: &KlineData,
) {
    use crate::trend_analyzer::BuySignal;
    let mut total_delta: i32 = 0;

    if let Some(hist) = latest.financials_history.as_ref() {
        if let Some(q) = crate::data_provider::assess_quality(hist) {
            if q.risk_score >= 60 {
                total_delta -= 20;
                let summary = q
                    .flags
                    .first()
                    .cloned()
                    .unwrap_or_else(|| q.level.to_string());
                trend_result.risk_factors.push(format!(
                    "💣 财务异常高风险(评分{}/100): {}",
                    q.risk_score, summary
                ));
                if matches!(
                    trend_result.buy_signal,
                    BuySignal::StrongBuy | BuySignal::Buy
                ) {
                    trend_result.buy_signal = BuySignal::Hold;
                }
            } else if q.risk_score >= 30 {
                total_delta -= 8;
                trend_result
                    .risk_factors
                    .push(format!("⚠️ 财务异常需关注(评分{}/100)", q.risk_score));
            }
        }

        let take: Vec<_> = hist.iter().take(4).collect();
        if take.len() >= 3 {
            let roe_chrono: Vec<f64> = take.iter().rev().filter_map(|p| p.roe).collect();
            let gm_chrono: Vec<f64> = take.iter().rev().filter_map(|p| p.gross_margin).collect();
            let cfo_ni: Vec<f64> = take.iter().filter_map(|p| p.cfo_to_ni_ratio()).collect();
            let roe_up =
                roe_chrono.len() >= 3 && roe_chrono.windows(2).all(|w| w[1] >= w[0] - 0.01);
            let gm_up = gm_chrono.len() >= 3 && gm_chrono.windows(2).all(|w| w[1] >= w[0] - 0.01);
            let cfo_ok =
                !cfo_ni.is_empty() && cfo_ni.iter().sum::<f64>() / cfo_ni.len() as f64 >= 0.8;
            if roe_up && gm_up && cfo_ok {
                total_delta += 5;
                trend_result
                    .signal_reasons
                    .push("💎 高质量盈利(ROE/毛利持续上行+CFO健康) +5".to_string());
            }
        }
    }

    if let Some(vh) = latest.valuation_history.as_ref() {
        if vh.sample_days >= 60 {
            if let Some(pe_pct) = vh.pe_percentile {
                if pe_pct < 20.0 {
                    total_delta += 5;
                    trend_result
                        .signal_reasons
                        .push(format!("📉 PE 历史极低估(分位{:.0}%) +5", pe_pct));
                } else if pe_pct > 80.0 {
                    total_delta -= 8;
                    trend_result
                        .risk_factors
                        .push(format!("📈 PE 历史极高估(分位{:.0}%)，回调风险大", pe_pct));
                    if matches!(trend_result.buy_signal, BuySignal::StrongBuy) {
                        trend_result.buy_signal = BuySignal::Buy;
                    }
                }
            }
        }
    }

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
                    trend_result
                        .risk_factors
                        .push(format!("🏦 卖方一致看空(看多仅{:.0}%)", bull));
                }
            }
            if let Some(up) = cs.upside_pct(latest.close) {
                if up > 30.0 {
                    total_delta += 3;
                    trend_result
                        .signal_reasons
                        .push(format!("🎯 目标价均值隐含 {:+.0}% 上行空间 +3", up));
                } else if up < -10.0 {
                    total_delta -= 5;
                    trend_result
                        .risk_factors
                        .push(format!("🎯 现价已高于目标价均值 {:+.0}%", up));
                }
            }
        }
    }

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
                    trend_result
                        .risk_factors
                        .push(format!("ROE 同业落后(P{:.0})", p));
                }
            }
            if let Some(p) = ib.pe_percentile {
                if p <= 20.0 {
                    total_delta += 2;
                    trend_result
                        .signal_reasons
                        .push(format!("💰 PE 同业偏低(P{:.0}) +2", p));
                } else if p >= 80.0 {
                    total_delta -= 3;
                    trend_result
                        .risk_factors
                        .push(format!("PE 同业偏高(P{:.0})", p));
                }
            }
            if let Some(p) = ib.growth_percentile {
                if p >= 80.0 {
                    total_delta += 2;
                    trend_result
                        .signal_reasons
                        .push(format!("🚀 净利同比同业领先(P{:.0}) +2", p));
                } else if p <= 20.0 {
                    total_delta -= 2;
                    trend_result
                        .risk_factors
                        .push(format!("净利同比同业落后(P{:.0})", p));
                }
            }
        }
    }

    let clamped = total_delta.clamp(-25, 25);
    if clamped != 0 {
        trend_result.signal_score = (trend_result.signal_score + clamped).clamp(0, 100);
        info!(
            "[{}] 🧮 基本面评分修正 {:+} → 总评分 {}",
            code, clamped, trend_result.signal_score
        );
    }
}

/// BR-121: render complete industry evidence; missing values remain explicit `-`.
fn render_industry_section(
    ib: &crate::data_provider::industry::IndustryBenchmark,
) -> Option<String> {
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
}

fn render_quality_report(q: &crate::data_provider::financials::QualityReport) -> Option<String> {
    if q.flags.is_empty() && q.risk_score == 0 {
        return None;
    }
    let icon = match q.level {
        "优秀" | "良好" => "🟢",
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
}

fn render_valuation_history_section(
    vh: &crate::data_provider::valuation_history::ValuationHistory,
) -> Option<String> {
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
}

fn render_consensus_section(
    cs: &crate::data_provider::consensus::ConsensusData,
    current_price: f64,
) -> Option<String> {
    if cs.report_count == 0 {
        return None;
    }
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
        parts.sort_by_key(|part| std::cmp::Reverse(part.1));
        let dist: Vec<String> = parts.iter().map(|(k, v)| format!("{} {}", k, v)).collect();
        let bull = cs.bullish_ratio().unwrap_or(0.0);
        s.push_str(&format!(
            "**评级分布**：{} | 看多比例 {:.0}%\n",
            dist.join(" / "),
            bull
        ));
    }
    match (cs.target_price_low_avg, cs.target_price_high_avg) {
        (Some(low), Some(high)) => {
            let upside = cs.upside_pct(current_price).unwrap_or(0.0);
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
                low, high, current_price, upside, tag
            ));
        }
        (None, Some(high)) => {
            let upside = cs.upside_pct(current_price).unwrap_or(0.0);
            s.push_str(&format!(
                "**目标价均值**：¥{:.2}（当前 ¥{:.2}，空间 {:+.1}%）\n",
                high, current_price, upside
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
}

fn render_financial_history_section(hist: &[FinancialPeriod]) -> Option<String> {
    let show: Vec<&FinancialPeriod> = hist.iter().take(6).collect();
    if show.len() < 2 {
        return None;
    }
    let fmt_opt = |v: Option<f64>| match v {
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
            fmt_opt(cfo_ni),
        ));
    }
    let trend = |f: fn(&FinancialPeriod) -> Option<f64>| -> Option<&'static str> {
        let vals: Vec<f64> = show.iter().filter_map(|p| f(p)).collect();
        if vals.len() < 3 {
            return None;
        }
        let up = vals.windows(2).all(|w| w[0] >= w[1]);
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
    let ratios: Vec<f64> = show.iter().filter_map(|p| p.cfo_to_ni_ratio()).collect();
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
}

/// BR-085: derive risk inputs only from the validated K-line batch.
fn position_risk_evidence(
    data: &[KlineData],
) -> Option<(crate::monitor::risk::MarketRegime, Option<f64>)> {
    let latest = data.first()?;
    let regime = crate::monitor::risk::classify_market(0.5, latest.pct_chg);
    let ranges: Vec<f64> = data
        .iter()
        .take(14)
        .map(|bar| bar.high - bar.low)
        .filter(|range| range.is_finite() && *range > 0.0)
        .collect();
    let atr = if ranges.is_empty() {
        None
    } else {
        Some(ranges.iter().sum::<f64>() / ranges.len() as f64)
    };
    Some((regime, atr))
}

impl AnalysisPipeline {
    /// 分析单只股票
    async fn analyze_stock(
        &self,
        code: &str,
        data: &[KlineData],
        kline_arc: Arc<Vec<KlineData>>,
        macro_context: Option<&str>,
    ) -> Result<AnalysisResult> {
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
        apply_boll_macd_adjustment(code, &mut trend_result, &bm);

        // 1.6 基本面评分修正（财务质量 + 估值分位）
        //     - 异常评分 ≥60：高风险，-20 并降档
        //     - 异常评分 30~59：中风险，-8 风险提示
        //     - PE 分位 <20%（极低估）：+5
        //     - PE 分位 >80%（极高估）：-8 风险提示且 StrongBuy→Buy
        //     - 高质量盈利（ROE 上行 + 毛利率上行 + CFO/NI≥0.8）：+5
        apply_fundamental_adjustments(code, &mut trend_result, &data[0]);

        // // === 补充风控修正（核心拦截器，解决系统"精神分裂"问题）===
        // // 已重构为 VetoChain 策略模式 (src/risk/veto_chain.rs + veto_rules_live.rs)
        // // 现在在数据获取完成后执行（见下方 "VetoChain 实时否决" 区块）

        // 2. 技术分析 Markdown
        let mut analysis_content = technical_report::build_technical_markdown(&trend_result);

        // 3-4. 并行抓取三路互相独立的上下文，整体只等最慢一路：
        //   ① 股票名→新闻（新闻依赖股票名，内部串行）
        //   ② 真实资金/分时/龙虎榜/筹码分布（不管 AI 是否启用都抓一次给通知展示）
        //   ③ 多周期下钻（60min/15min，仅在日线买入信号触发时）
        let mtf_trigger = {
            use crate::strategy::BollMacdAction;
            use crate::trend_analyzer::BuySignal;
            trend_result.signal_score >= 60
                || matches!(
                    bm.action,
                    BollMacdAction::BottomBuy | BollMacdAction::UptrendStart
                )
                || matches!(
                    trend_result.buy_signal,
                    BuySignal::StrongBuy | BuySignal::Buy
                )
        };

        let name_news_fut = async {
            // 股票名称（同步 HTTP，放 blocking 线程池）
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
            (stock_name, news_context)
        };

        let mtf_fut = async {
            if mtf_trigger {
                info!("[{}] 触发多周期下钻（60min/15min 寻找精准入场点）", code);
                multi_timeframe::fetch_multi_timeframe_section(code).await
            } else {
                Ok(None)
            }
        };

        #[cfg(test)]
        let ((stock_name, news_context), extra, mtf_section_opt) =
            if let Some(context) = self.test_resolved_context.as_ref() {
                let name_news = match context.name_news.as_ref() {
                    Some(resolved) => resolved.clone(),
                    None => name_news_fut.await,
                };
                (
                    name_news,
                    context.extra.clone(),
                    context.mtf_section.clone(),
                )
            } else {
                tokio::join!(
                    name_news_fut,
                    extra_context::fetch_extra_context(code, data),
                    mtf_fut
                )
            };
        #[cfg(not(test))]
        let ((stock_name, news_context), extra, mtf_section_opt) = tokio::join!(
            name_news_fut,
            extra_context::fetch_extra_context(code, data),
            mtf_fut
        );

        let extra = extra.map_err(anyhow::Error::msg)?;
        let mut extra_context = extra.section;
        let money_flow_raw = extra.money_flow;

        let mtf_section_opt = mtf_section_opt.map_err(anyhow::Error::msg)?;
        if let Some(mtf_section) = mtf_section_opt {
            extra_context = match extra_context {
                Some(mut s) => {
                    s.push_str(&mtf_section);
                    Some(s)
                }
                None => Some(mtf_section),
            };
        }

        // ===== VetoChain 实时否决 (替代原注释代码 686-740, 已重构为策略模式) =====
        // 执行时机: 数据全部获取后 → VetoChain → score_to_advice
        // 与 veto_rules (Phase 1/3 估值否决) 互补: VetoChain 做技术/资金/基本面实时拦截
        {
            use crate::trend_analyzer::{BuySignal, TrendStatus};
            let veto_config = crate::config::get_veto_config();
            if let Some(chain) = crate::risk::veto_rules_live::build_chain(
                &crate::risk::veto_chain::VetoChainConfig {
                    enabled: veto_config.enabled,
                    mode: veto_config
                        .mode
                        .parse()
                        .unwrap_or(crate::risk::veto_chain::VetoMode::DryRun),
                    bias_rate_enabled: veto_config.bias_rate_enabled,
                    bearish_alignment_enabled: veto_config.bearish_alignment_enabled,
                    main_flow_enabled: veto_config.main_flow_enabled,
                    fundamental_enabled: veto_config.fundamental_enabled,
                },
            ) {
                let is_buy = matches!(
                    trend_result.buy_signal,
                    BuySignal::StrongBuy | BuySignal::Buy
                );
                let is_bearish = matches!(
                    trend_result.trend_status,
                    TrendStatus::StrongBear | TrendStatus::Bear
                );
                let mf_days = money_flow_raw.as_ref().map(|mf| mf.days.clone());

                let veto_ctx = crate::risk::veto_chain::VetoContext {
                    code: code.to_string(),
                    current_price: data[0].close,
                    signal_score: trend_result.signal_score,
                    is_buy_signal: is_buy,
                    bias_ma5: trend_result.bias_ma5,
                    is_bearish,
                    money_flow_days: mf_days,
                    pct_chg: Some(data[0].pct_chg),
                    pe_ratio: data[0].pe_ratio,
                    net_profit_yoy: data[0].net_profit_yoy,
                };

                let outcome = chain.evaluate_all(&veto_ctx);

                if !outcome.is_empty() {
                    match veto_config.mode.as_str() {
                        "live" => {
                            // v17.1 (P1 fix): 仅 buy 信号 + force_hold 时压分 (避免影响 Sell/Hold)
                            if outcome.force_hold && is_buy {
                                trend_result.signal_score = 55;
                                trend_result.buy_signal = BuySignal::Hold;
                            } else if is_buy && outcome.total_penalty != 0 {
                                trend_result.signal_score = (trend_result.signal_score
                                    + outcome.total_penalty)
                                    .clamp(0, 100);
                            }
                            // v17.2 (P2 fix): risk flags 始终传播 (不依赖 force_hold 分支)
                            for flag in &outcome.flags {
                                trend_result.risk_factors.push(flag.clone());
                            }
                            if outcome.force_hold && is_buy {
                                info!("[{}] 🛑 VetoChain[live] 拦截: force_hold, 评分压至55", code);
                            }
                        }
                        _ => {
                            // dry_run: 记录日志但不实际修改信号
                            info!(
                                "[{}] 🔍 VetoChain[dry_run] 触发: flags={:?} penalty={} force_hold={} — 未实际拦截",
                                code, outcome.flags, outcome.total_penalty, outcome.force_hold
                            );
                        }
                    }
                }
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
            veto_flags: if veto_pre.flags.is_empty() {
                &empty_veto
            } else {
                &veto_pre.flags
            },
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
                    analysis_content
                        .push_str(&self::section_utils::normalize_ai_sections(&ai_result));
                }
                Err(e) => warn!("[{}] AI分析失败: {}", code, e),
            }
        }
        if let Some(ref news) = news_context {
            analysis_content.push_str("\n# 相关新闻\n\n");
            analysis_content.push_str(news);
        }

        // 7. 价格区间 / 近期统计
        let stats = price_stats::compute_price_stats(data);

        // 8. 行业横向对标渲染（如有）
        let industry_section = data[0].industry.as_ref().and_then(render_industry_section);

        // 9. 财务质量评估渲染
        let quality_section = data[0]
            .financials_history
            .as_ref()
            .and_then(|hist| crate::data_provider::assess_quality(hist))
            .and_then(|q| render_quality_report(&q));

        // 10. 估值历史分位渲染
        let valuation_history_section = data[0]
            .valuation_history
            .as_ref()
            .and_then(render_valuation_history_section);

        // 11. 卖方一致预期渲染
        let consensus_section = data[0]
            .consensus
            .as_ref()
            .and_then(|cs| render_consensus_section(cs, data[0].close));

        // 12. 多期财务趋势渲染
        let fin_history_section = data[0]
            .financials_history
            .as_deref()
            .and_then(render_financial_history_section);

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
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n\n"))
            }
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
            ranking_score: trend_result.signal_score,
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
            chg_1d: Some(data[0].pct_chg),
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
        result.ranking_score = score_breakdown::compute_ranking_score(&sb);
        result.score_breakdown = Some(sb);

        // ===== Phase 2: 交易类型标注 =====
        result.trade_type = trade_type_pre;

        Ok(result)
    }

    /// 处理单只股票的完整流程（含 120s 超时保护）
    pub(super) async fn process_stock(
        &self,
        code: String,
        macro_context: Arc<str>,
    ) -> Option<AnalysisResult> {
        let start = std::time::Instant::now();
        info!("========== [{}] 开始处理 ==========", code);

        // 整体超时保护：单只股票最多处理 120 秒，避免任何环节卡死拖垮全局
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            self.process_stock_inner(code.clone(), macro_context),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                error!("[{}] 处理超时（120s），跳过", code);
                None
            }
        };

        let elapsed = start.elapsed();
        match &result {
            Some(r) => info!(
                "[{}] ✓ 处理完成 ({:.1}s)：{} 评分 {}",
                code,
                elapsed.as_secs_f32(),
                r.operation_advice,
                r.sentiment_score
            ),
            None => warn!(
                "[{}] ✗ 处理失败或超时 ({:.1}s)",
                code,
                elapsed.as_secs_f32()
            ),
        }
        result
    }

    async fn process_stock_inner(
        &self,
        code: String,
        macro_context: Arc<str>,
    ) -> Option<AnalysisResult> {
        // 1. 获取数据
        #[cfg(test)]
        let fetched_data = match self.test_fetched_data.as_ref() {
            Some(result) => result.clone().map_err(anyhow::Error::msg),
            None => self.fetch_and_save_data(&code).await,
        };
        #[cfg(not(test))]
        let fetched_data = self.fetch_and_save_data(&code).await;
        let data = match fetched_data {
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
        let mc = if macro_context.is_empty() {
            None
        } else {
            Some(&*macro_context)
        };
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
        #[cfg(test)]
        let position_tracking_enabled =
            position_tracking_enabled && self.test_fetched_data.is_none();
        if position_tracking_enabled {
            if let Some((regime, atr)) = position_risk_evidence(&data) {
                let risk_ctx = position_tracker::RiskContext::from_env(regime, atr);
                if let Err(error) =
                    position_tracker::track_position(&code, &data, &mut result, &risk_ctx)
                {
                    error!("[{}] BR-124 持仓跟踪失败: {}", code, error);
                    return None;
                }
            }
        }

        // 5. 保存分析结果到数据库
        if let Err(error) = position_tracker::save_analysis_result(&code, &data, &result) {
            error!("[{}] BR-124 分析结果保存失败: {}", code, error);
            return None;
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
}

#[cfg(test)]
mod tests {
    use super::{
        apply_boll_macd_adjustment, apply_fundamental_adjustments, position_risk_evidence,
        render_consensus_section, render_financial_history_section, render_industry_section,
        render_quality_report, render_valuation_history_section,
    };
    use crate::data_provider::consensus::{ConsensusData, RecentReport};
    use crate::data_provider::financials::{FinancialPeriod, QualityReport};
    use crate::data_provider::industry::IndustryBenchmark;
    use crate::data_provider::valuation_history::ValuationHistory;
    use crate::data_provider::{AdjustType, DataFetcherManager, KlineData};
    use crate::indicators::DivergenceType;
    use crate::notification::NotificationService;
    use crate::strategy::{BollMacdAction, BollMacdSignal};
    use crate::trend_analyzer::{BuySignal, StockTrendAnalyzer, TrendAnalysisResult};
    use chrono::NaiveDate;
    use diesel::prelude::*;
    use std::sync::Arc;

    use super::AnalysisPipeline;

    fn boll_signal(action: BollMacdAction) -> BollMacdSignal {
        BollMacdSignal {
            action,
            reason: "TEST_CODE_共振证据".to_string(),
            close: 10.0,
            upper: 11.0,
            middle: 10.0,
            lower: 9.0,
            band_width_pct: 20.0,
            band_change_pct: 5.0,
            macd_dif: 0.1,
            macd_dea: 0.05,
            macd_hist: 0.1,
            macd_div: DivergenceType::None,
            vol_ratio: 1.2,
        }
    }

    #[test]
    fn boll_macd_adjustment_applies_each_registered_action() {
        let cases = [
            (BollMacdAction::None, 70, 0, 0),
            (BollMacdAction::PreReversal, 73, 1, 0),
            (BollMacdAction::BottomBuy, 80, 1, 0),
            (BollMacdAction::UptrendStart, 82, 1, 0),
        ];
        for (action, expected_score, reasons, risks) in cases {
            let mut trend = TrendAnalysisResult {
                signal_score: 70,
                buy_signal: BuySignal::StrongBuy,
                ..TrendAnalysisResult::default()
            };
            apply_boll_macd_adjustment("TEST_CODE_000001", &mut trend, &boll_signal(action));
            assert_eq!(trend.signal_score, expected_score, "{action:?}");
            assert_eq!(trend.signal_reasons.len(), reasons, "{action:?}");
            assert_eq!(trend.risk_factors.len(), risks, "{action:?}");
        }

        let mut top = TrendAnalysisResult {
            signal_score: 100,
            buy_signal: BuySignal::StrongBuy,
            ..TrendAnalysisResult::default()
        };
        apply_boll_macd_adjustment(
            "TEST_CODE_000001",
            &mut top,
            &boll_signal(BollMacdAction::TopSell),
        );
        assert_eq!(top.signal_score, 55);
        assert_eq!(top.buy_signal, BuySignal::Hold);
        assert!(top.signal_reasons.is_empty());
        assert_eq!(top.risk_factors.len(), 1);
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

    fn valuation(percentile: Option<f64>, sample_days: usize) -> ValuationHistory {
        ValuationHistory {
            current_pe: Some(12.0),
            current_pb: Some(1.2),
            pe_percentile: percentile,
            pb_percentile: percentile,
            pe_min: Some(8.0),
            pe_max: Some(30.0),
            pe_median: Some(16.0),
            pb_min: Some(0.8),
            pb_max: Some(3.0),
            pb_median: Some(1.6),
            sample_days,
            oldest_date: Some("2025-01-02".to_string()),
            newest_date: Some("2026-07-18".to_string()),
        }
    }

    fn consensus(
        ratings: &[(&str, u32)],
        broker_count: usize,
        target_low: Option<f64>,
        target_high: Option<f64>,
    ) -> ConsensusData {
        ConsensusData {
            report_count: ratings.iter().map(|(_, count)| *count as usize).sum(),
            broker_count,
            eps_this_year_avg: Some(1.0),
            eps_next_year_avg: Some(1.2),
            eps_next2_year_avg: Some(1.4),
            rating_distribution: ratings
                .iter()
                .map(|(rating, count)| ((*rating).to_string(), *count))
                .collect(),
            target_price_high_avg: target_high,
            target_price_low_avg: target_low,
            latest_report_date: Some("2026-07-18".to_string()),
            recent_reports: Vec::new(),
        }
    }

    fn high_risk_history() -> Vec<FinancialPeriod> {
        vec![
            FinancialPeriod {
                eps: Some(3.0),
                roe: Some(10.0),
                revenue_yoy: Some(120.0),
                net_profit_yoy: Some(200.0),
                gross_margin: Some(50.0),
                op_cash_flow_ps: Some(0.3),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(2.0),
                roe: Some(11.0),
                gross_margin: Some(40.0),
                op_cash_flow_ps: Some(1.6),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(1.0),
                roe: Some(12.0),
                gross_margin: Some(39.0),
                op_cash_flow_ps: Some(0.1),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(0.5),
                roe: Some(13.0),
                gross_margin: Some(38.0),
                op_cash_flow_ps: Some(0.0),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn fundamental_adjustment_preserves_missing_and_rewards_complete_positive_evidence() {
        let mut trend = TrendAnalysisResult {
            signal_score: 50,
            buy_signal: BuySignal::StrongBuy,
            ..TrendAnalysisResult::default()
        };
        apply_fundamental_adjustments("TEST_CODE_EMPTY", &mut trend, &kline());
        assert_eq!(trend.signal_score, 50);

        let mut latest = kline();
        latest.financials_history = Some(vec![
            FinancialPeriod {
                eps: Some(1.3),
                roe: Some(13.0),
                gross_margin: Some(43.0),
                op_cash_flow_ps: Some(1.3),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(1.2),
                roe: Some(12.0),
                gross_margin: Some(42.0),
                op_cash_flow_ps: Some(1.2),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(1.1),
                roe: Some(11.0),
                gross_margin: Some(41.0),
                op_cash_flow_ps: Some(1.1),
                ..Default::default()
            },
        ]);
        latest.valuation_history = Some(valuation(Some(10.0), 60));
        latest.consensus = Some(consensus(&[("买入", 5)], 5, Some(12.0), Some(14.0)));
        latest.industry = Some(IndustryBenchmark {
            peer_count: 5,
            roe_percentile: Some(90.0),
            pe_percentile: Some(10.0),
            growth_percentile: Some(90.0),
            ..Default::default()
        });
        apply_fundamental_adjustments("TEST_CODE_POSITIVE", &mut trend, &latest);
        assert_eq!(trend.signal_score, 73);
        assert_eq!(trend.buy_signal, BuySignal::StrongBuy);
        assert_eq!(trend.signal_reasons.len(), 7);
    }

    #[test]
    fn fundamental_adjustment_caps_negative_evidence_and_downgrades_buys() {
        let mut latest = kline();
        latest.financials_history = Some(high_risk_history());
        latest.valuation_history = Some(valuation(Some(90.0), 60));
        latest.consensus = Some(consensus(&[("卖出", 3)], 3, Some(7.0), Some(8.0)));
        latest.industry = Some(IndustryBenchmark {
            peer_count: 5,
            roe_percentile: Some(20.0),
            pe_percentile: Some(80.0),
            growth_percentile: Some(20.0),
            ..Default::default()
        });
        let mut trend = TrendAnalysisResult {
            signal_score: 50,
            buy_signal: BuySignal::StrongBuy,
            ..TrendAnalysisResult::default()
        };
        apply_fundamental_adjustments("TEST_CODE_NEGATIVE", &mut trend, &latest);
        assert_eq!(trend.signal_score, 25);
        assert_eq!(trend.buy_signal, BuySignal::Hold);
        assert!(trend.risk_factors.len() >= 7);

        let mut valuation_only = kline();
        valuation_only.valuation_history = Some(valuation(Some(81.0), 60));
        let mut valuation_trend = TrendAnalysisResult {
            signal_score: 50,
            buy_signal: BuySignal::StrongBuy,
            ..TrendAnalysisResult::default()
        };
        apply_fundamental_adjustments("TEST_CODE_VALUATION", &mut valuation_trend, &valuation_only);
        assert_eq!(valuation_trend.signal_score, 42);
        assert_eq!(valuation_trend.buy_signal, BuySignal::Buy);
    }

    #[test]
    fn fundamental_adjustment_covers_attention_and_ignored_sample_gates() {
        let mut latest = kline();
        latest.financials_history = Some(vec![
            FinancialPeriod {
                eps: Some(1.0),
                op_cash_flow_ps: Some(0.1),
                revenue_yoy: Some(0.0),
                net_profit_yoy: Some(30.0),
                gross_margin: Some(50.0),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(1.0),
                op_cash_flow_ps: Some(0.5),
                gross_margin: Some(40.0),
                ..Default::default()
            },
        ]);
        latest.valuation_history = Some(valuation(Some(1.0), 59));
        latest.consensus = Some(consensus(&[("买入", 2)], 2, None, Some(20.0)));
        latest.industry = Some(IndustryBenchmark {
            peer_count: 4,
            roe_percentile: Some(100.0),
            ..Default::default()
        });
        let mut trend = TrendAnalysisResult {
            signal_score: 50,
            buy_signal: BuySignal::Buy,
            ..TrendAnalysisResult::default()
        };
        apply_fundamental_adjustments("TEST_CODE_GATES", &mut trend, &latest);
        assert_eq!(trend.signal_score, 42);
        assert_eq!(trend.buy_signal, BuySignal::Buy);
        assert_eq!(trend.risk_factors.len(), 1);
    }

    #[test]
    fn industry_renderer_handles_sample_gate_missing_values_and_rank_tags() {
        assert!(render_industry_section(&IndustryBenchmark {
            peer_count: 2,
            ..Default::default()
        })
        .is_none());
        let positive = render_industry_section(&IndustryBenchmark {
            industry_name: "TEST_CODE_行业".to_string(),
            board_code: "TEST_CODE_BK001".to_string(),
            peer_count: 8,
            stock_pe: Some(10.0),
            median_pe: Some(20.0),
            pe_percentile: Some(20.0),
            roe_percentile: Some(80.0),
            growth_percentile: Some(80.0),
            ..Default::default()
        })
        .expect("qualified industry evidence");
        assert!(positive.contains("ROE 领先同业"));
        assert!(positive.contains("估值低于多数同业"));
        assert!(positive.contains("业绩增速领先同业"));
        assert!(positive.contains("| PB | - | - | - |"));

        let negative = render_industry_section(&IndustryBenchmark {
            peer_count: 8,
            roe_percentile: Some(25.0),
            pe_percentile: Some(75.0),
            growth_percentile: Some(25.0),
            ..Default::default()
        })
        .expect("qualified industry evidence");
        assert!(negative.contains("ROE 落后同业"));
        assert!(negative.contains("估值高于多数同业"));
        assert!(negative.contains("业绩增速落后同业"));
    }

    #[test]
    fn quality_renderer_covers_clean_and_all_display_levels() {
        assert!(render_quality_report(&QualityReport {
            risk_score: 0,
            flags: Vec::new(),
            level: "无明显异常",
        })
        .is_none());
        for (level, icon) in [
            ("优秀", "🟢"),
            ("良好", "🟢"),
            ("一般", "🟡"),
            ("偏弱", "🟠"),
            ("风险", "🔴"),
            ("需关注", "⚪"),
        ] {
            let rendered = render_quality_report(&QualityReport {
                risk_score: 30,
                flags: vec![format!("TEST_CODE_{level}")],
                level,
            })
            .expect("risk evidence should render");
            assert!(rendered.contains(icon));
            assert!(rendered.contains("触发的红旗信号"));
        }
    }

    #[test]
    fn valuation_renderer_covers_sample_gate_ranges_and_all_percentile_bands() {
        assert!(render_valuation_history_section(&valuation(Some(10.0), 29)).is_none());
        for (percentile, label) in [
            (Some(10.0), "历史底部区"),
            (Some(30.0), "偏低"),
            (Some(50.0), "中位"),
            (Some(70.0), "偏高"),
            (Some(90.0), "历史高位"),
        ] {
            let rendered = render_valuation_history_section(&valuation(percentile, 60))
                .expect("qualified valuation evidence");
            assert!(rendered.contains(label));
            assert!(rendered.contains("2025-01-02 ~ 2026-07-18"));
        }
        let mut missing = valuation(None, 60);
        missing.oldest_date = None;
        missing.current_pe = None;
        let rendered = render_valuation_history_section(&missing).expect("sample still qualifies");
        assert!(rendered.contains("近 60 个交易日"));
        assert!(rendered.contains("| PE | - |"));
    }

    #[test]
    fn consensus_renderer_covers_targets_eps_reports_and_missing_branches() {
        assert!(render_consensus_section(&ConsensusData::default(), 10.0).is_none());
        for (high, expected) in [
            (14.0, "显著上行空间"),
            (12.0, "温和上行"),
            (10.0, "持平"),
            (8.0, "已高于目标价"),
        ] {
            let rendered = render_consensus_section(
                &consensus(&[("买入", 2), ("中性", 1)], 3, Some(7.0), Some(high)),
                10.0,
            )
            .expect("qualified consensus evidence");
            assert!(rendered.contains(expected));
            assert!(rendered.contains("看多比例 67%"));
            assert!(rendered.contains("同比 +20.0%"));
        }

        let mut high_only = consensus(&[("增持", 1)], 1, None, Some(11.0));
        high_only.eps_this_year_avg = Some(0.0);
        high_only.recent_reports = (0..4)
            .map(|index| RecentReport {
                title: if index == 0 {
                    "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十".to_string()
                } else {
                    format!("TEST_CODE_短标题{index}")
                },
                org_name: format!("TEST_CODE_券商{index}"),
                publish_date: format!("2026-07-{}", 18 - index),
                rating: "增持".to_string(),
            })
            .collect();
        let rendered = render_consensus_section(&high_only, 10.0).expect("high-only target");
        assert!(rendered.contains("目标价均值"));
        assert!(rendered.contains('…'));
        assert_eq!(rendered.matches("TEST_CODE_券商").count(), 3);
        assert!(!rendered.contains("同比"));

        let no_target = consensus(&[("卖出", 1)], 1, Some(8.0), None);
        let rendered = render_consensus_section(&no_target, 10.0).expect("report still renders");
        assert!(!rendered.contains("目标价"));
    }

    fn period(
        date: Option<&str>,
        roe: Option<f64>,
        revenue: Option<f64>,
        gross: Option<f64>,
        ratio: Option<f64>,
    ) -> FinancialPeriod {
        FinancialPeriod {
            report_date: date.map(str::to_string),
            eps: ratio.map(|_| 1.0),
            roe,
            revenue_yoy: revenue,
            net_profit_yoy: None,
            gross_margin: gross,
            net_margin: None,
            op_cash_flow_ps: ratio,
            ..Default::default()
        }
    }

    #[test]
    fn financial_history_renderer_covers_gate_trends_missing_values_and_ratio_bands() {
        assert!(render_financial_history_section(&[period(
            Some("2026-06-30"),
            Some(10.0),
            None,
            None,
            Some(1.0),
        )])
        .is_none());

        let trending = vec![
            period(Some("2026-06-30"), Some(13.0), Some(13.0), Some(30.0), None),
            period(Some("2026-03-31"), Some(12.0), Some(12.0), Some(31.0), None),
            period(None, Some(11.0), Some(11.0), Some(32.0), None),
            period(Some("2025-09-30"), Some(10.0), Some(10.0), Some(33.0), None),
            period(Some("2025-06-30"), None, None, None, None),
            period(Some("2025-03-31"), None, None, None, None),
            period(Some("2024-12-31"), Some(99.0), Some(99.0), Some(99.0), None),
        ];
        let rendered = render_financial_history_section(&trending).expect("two or more periods");
        assert!(rendered.contains("ROE 持续上行"));
        assert!(rendered.contains("营收增速 持续上行"));
        assert!(rendered.contains("毛利率 持续下行"));
        assert!(rendered.contains("| - |"));
        assert!(!rendered.contains("99.00"));

        for (ratio, label) in [
            (0.2, "偏低"),
            (0.5, "健康下沿"),
            (0.8, "🟢 健康"),
            (1.2, "优秀"),
        ] {
            let history = vec![
                period(Some("2026-06-30"), Some(10.0), None, None, Some(ratio)),
                period(Some("2026-03-31"), Some(11.0), None, None, Some(ratio)),
                period(Some("2025-12-31"), Some(10.5), None, None, Some(ratio)),
            ];
            let rendered = render_financial_history_section(&history).expect("ratio evidence");
            assert!(rendered.contains(label), "ratio={ratio}: {rendered}");
        }
    }

    fn resolved_context(
        extra: Result<super::extra_context::ExtraContext, String>,
        mtf_section: Result<Option<String>, String>,
    ) -> super::super::TestResolvedAnalysisContext {
        super::super::TestResolvedAnalysisContext {
            name_news: Some((
                "TEST_CODE_示例公司".to_string(),
                Some("TEST_CODE_真实新闻证据".to_string()),
            )),
            extra,
            mtf_section,
        }
    }

    fn test_pipeline(
        context: super::super::TestResolvedAnalysisContext,
        limit_up: bool,
    ) -> AnalysisPipeline {
        AnalysisPipeline {
            data_manager: Arc::new(DataFetcherManager::new().expect("test data manager")),
            trend_analyzer: Arc::new(StockTrendAnalyzer::new()),
            ai_analyzer: None,
            use_news_search: false,
            notifier: Arc::new(NotificationService::new(Default::default())),
            config: super::super::PipelineConfig {
                send_notification: false,
                single_notify: false,
                ..Default::default()
            },
            limit_up_codes: Arc::new(if limit_up {
                ["TEST_CODE_000001".to_string()].into_iter().collect()
            } else {
                Default::default()
            }),
            test_resolved_context: Some(context),
            test_fetched_data: None,
            test_backtest_output_dir: None,
            test_summary_context: None,
        }
    }

    fn analysis_bars() -> Vec<KlineData> {
        (0..40)
            .map(|index| {
                let mut bar = kline();
                bar.date = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid fixture date")
                    - chrono::Duration::days(index);
                bar.close = 10.0 - index as f64 * 0.01;
                bar.open = bar.close;
                bar.high = bar.close + 0.2;
                bar.low = bar.close - 0.2;
                bar
            })
            .collect()
    }

    #[test]
    fn position_risk_inputs_use_only_positive_finite_ranges() {
        assert!(position_risk_evidence(&[]).is_none());
        let mut bars = analysis_bars();
        bars[0].high = 10.5;
        bars[0].low = 9.5;
        bars[1].high = 10.0;
        bars[1].low = 10.0;
        bars[2].high = f64::NAN;
        let (_, atr) = position_risk_evidence(&bars).expect("nonempty evidence");
        assert!(atr.is_some_and(|value| value > 0.0 && value.is_finite()));

        for bar in &mut bars {
            bar.high = bar.low;
        }
        assert_eq!(position_risk_evidence(&bars).unwrap().1, None);
    }

    #[tokio::test]
    async fn resolved_test_context_exercises_result_assembly_without_live_transport() {
        let context = resolved_context(
            Ok(super::extra_context::ExtraContext {
                section: Some("TEST_CODE_真实资金证据".to_string()),
                money_flow: None,
            }),
            Ok(Some("TEST_CODE_多周期证据".to_string())),
        );
        let mut pipeline = test_pipeline(context, true);
        pipeline.ai_analyzer = Some(crate::analyzer::GeminiAnalyzer::new(
            crate::analyzer::GeminiConfig {
                max_retries: 1,
                retry_delay: 0.0,
                request_delay: 0.0,
                ..Default::default()
            },
        ));
        let mut values = analysis_bars();
        values[0].financials_history = Some(vec![
            FinancialPeriod {
                eps: Some(1.0),
                op_cash_flow_ps: Some(0.1),
                revenue_yoy: Some(0.0),
                net_profit_yoy: Some(30.0),
                gross_margin: Some(50.0),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(1.0),
                op_cash_flow_ps: Some(0.5),
                gross_margin: Some(40.0),
                ..Default::default()
            },
        ]);
        values[0].valuation_history = Some(valuation(Some(50.0), 60));
        values[0].consensus = Some(consensus(&[("买入", 3)], 3, Some(11.0), Some(12.0)));
        values[0].industry = Some(IndustryBenchmark {
            industry_name: "TEST_CODE_行业".to_string(),
            board_code: "TEST_CODE_BK001".to_string(),
            peer_count: 8,
            stock_pe: Some(10.0),
            median_pe: Some(20.0),
            roe_percentile: Some(50.0),
            ..Default::default()
        });
        let bars = Arc::new(values);

        let result = pipeline
            .analyze_stock(
                "TEST_CODE_000001",
                bars.as_slice(),
                bars.clone(),
                Some("TEST_CODE_宏观证据"),
            )
            .await
            .expect("resolved analysis");

        assert_eq!(result.name, "TEST_CODE_示例公司");
        assert!(result.is_limit_up);
        assert!(result
            .money_flow_section
            .as_deref()
            .is_some_and(|text| text.contains("真实资金证据") && text.contains("多周期证据")));
        assert!(result.analysis_summary.contains("真实新闻证据"));
        assert!(result.industry_section.is_some());
        assert!(result.quality_section.is_some());
        assert!(result.valuation_history_section.is_some());
        assert!(result.consensus_section.is_some());
        assert!(result.fin_history_section.is_some());
        let seed = result.deep_seed.expect("deep-analysis seed");
        assert_eq!(seed.kline.len(), 40);
        assert_eq!(seed.macro_context.as_deref(), Some("TEST_CODE_宏观证据"));
        assert!(seed
            .fundamental_ctx
            .as_deref()
            .is_some_and(|text| text.contains("多期财务趋势") && text.contains("行业横向对标")));
    }

    #[tokio::test]
    async fn resolved_context_failures_are_explicit_and_empty_input_is_rejected() {
        let bars = Arc::new(analysis_bars());
        let extra_failure = test_pipeline(
            resolved_context(Err("TEST_CODE_资金源失败".to_string()), Ok(None)),
            false,
        )
        .analyze_stock("TEST_CODE_000001", bars.as_slice(), bars.clone(), None)
        .await
        .expect_err("extra context failure");
        assert!(extra_failure.to_string().contains("资金源失败"));

        let mtf_failure = test_pipeline(
            resolved_context(
                Ok(super::extra_context::ExtraContext {
                    section: None,
                    money_flow: None,
                }),
                Err("TEST_CODE_多周期源失败".to_string()),
            ),
            false,
        )
        .analyze_stock("TEST_CODE_000001", bars.as_slice(), bars.clone(), None)
        .await
        .expect_err("multi-timeframe failure");
        assert!(mtf_failure.to_string().contains("多周期源失败"));

        let empty = Arc::new(Vec::new());
        let empty_failure = test_pipeline(
            resolved_context(
                Ok(super::extra_context::ExtraContext {
                    section: None,
                    money_flow: None,
                }),
                Ok(None),
            ),
            false,
        )
        .analyze_stock("TEST_CODE_000001", empty.as_slice(), empty.clone(), None)
        .await
        .expect_err("empty K-line input");
        assert_eq!(empty_failure.to_string(), "数据为空");
    }

    #[tokio::test]
    async fn resolved_context_preserves_absent_optional_sections() {
        let mut context = resolved_context(
            Ok(super::extra_context::ExtraContext {
                section: None,
                money_flow: None,
            }),
            Ok(None),
        );
        context.name_news = None;
        let mut pipeline = test_pipeline(context, false);
        pipeline.data_manager = Arc::new(DataFetcherManager::with_cached_name(
            "TEST_CODE_000001",
            "TEST_CODE_缓存公司",
        ));
        let bars = Arc::new(analysis_bars());
        let result = pipeline
            .analyze_stock("TEST_CODE_000001", bars.as_slice(), bars.clone(), None)
            .await
            .expect("analysis without optional evidence");

        assert!(!result.is_limit_up);
        assert_eq!(result.money_flow_section, None);
        assert_eq!(result.industry_section, None);
        assert_eq!(result.quality_section, None);
        assert_eq!(result.valuation_history_section, None);
        assert_eq!(result.consensus_section, None);
        assert_eq!(result.fin_history_section, None);
        assert_eq!(result.name, "TEST_CODE_缓存公司");
        let seed = result.deep_seed.expect("deep-analysis seed");
        assert_eq!(seed.news_context, None);
        assert_eq!(seed.macro_context, None);
        assert_eq!(seed.fundamental_ctx, None);
    }

    #[tokio::test]
    async fn process_stock_uses_isolated_fetched_batch_and_covers_failure_gates() {
        crate::database::DatabaseManager::init(None).expect("BR-124 test database init");
        let context = resolved_context(
            Ok(super::extra_context::ExtraContext {
                section: None,
                money_flow: None,
            }),
            Ok(None),
        );
        let mut pipeline = test_pipeline(context.clone(), false);
        pipeline.test_fetched_data = Some(Ok(analysis_bars()));
        pipeline.config.single_notify = true;
        pipeline.config.send_notification = true;
        let result = pipeline
            .process_stock("TEST_CODE_000001".to_string(), Arc::from(""))
            .await
            .expect("isolated process result");
        assert_eq!(result.code, "TEST_CODE_000001");

        let mut fetch_failure = test_pipeline(context.clone(), false);
        fetch_failure.test_fetched_data = Some(Err("TEST_CODE_日线源失败".to_string()));
        assert!(fetch_failure
            .process_stock_inner("TEST_CODE_000001".to_string(), Arc::from("macro"))
            .await
            .is_none());

        let mut empty = test_pipeline(context.clone(), false);
        empty.test_fetched_data = Some(Ok(Vec::new()));
        assert!(empty
            .process_stock_inner("TEST_CODE_000001".to_string(), Arc::from("macro"))
            .await
            .is_none());

        let mut dry_run = test_pipeline(context.clone(), false);
        dry_run.test_fetched_data = Some(Ok(analysis_bars()));
        dry_run.config.dry_run = true;
        assert!(dry_run
            .process_stock_inner("TEST_CODE_000001".to_string(), Arc::from("macro"))
            .await
            .is_none());

        let mut analysis_failure = test_pipeline(
            resolved_context(Err("TEST_CODE_上下文失败".to_string()), Ok(None)),
            false,
        );
        analysis_failure.test_fetched_data = Some(Ok(analysis_bars()));
        assert!(analysis_failure
            .process_stock("TEST_CODE_000001".to_string(), Arc::from("macro"))
            .await
            .is_none());
    }

    struct FullPipelineGuard {
        code: String,
        output_dir: std::path::PathBuf,
    }

    impl Drop for FullPipelineGuard {
        fn drop(&mut self) {
            if let Some(db) = crate::database::DatabaseManager::try_get() {
                if let Ok(mut connection) = db.get_conn() {
                    let _ = diesel::sql_query("DELETE FROM analysis_result WHERE code = ?")
                        .bind::<diesel::sql_types::Text, _>(&self.code)
                        .execute(&mut connection);
                }
            }
            let _ = std::fs::remove_dir_all(&self.output_dir);
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn full_pipeline_run_commits_local_backtests_without_external_side_effects() {
        crate::database::DatabaseManager::init(None).expect("test database initialization");
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let code = format!("TEST_CODE_PIPELINE_RUN_{suffix}");
        let output_dir = std::env::temp_dir().join(format!(
            "stock-analysis-full-pipeline-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&output_dir).expect("isolated report directory");
        let _guard = FullPipelineGuard {
            code: code.clone(),
            output_dir: output_dir.clone(),
        };

        let context = resolved_context(
            Ok(super::extra_context::ExtraContext {
                section: Some("TEST_CODE_本地资金证据".to_string()),
                money_flow: None,
            }),
            Ok(Some("TEST_CODE_本地多周期证据".to_string())),
        );
        let mut pipeline = test_pipeline(context, false);
        pipeline.config.max_workers = 1;
        pipeline.config.dry_run = false;
        pipeline.config.send_notification = false;
        pipeline.config.single_notify = false;
        pipeline.ai_analyzer = None;
        pipeline.use_news_search = false;
        pipeline.test_fetched_data = Some(Ok(analysis_bars()));
        pipeline.test_backtest_output_dir = Some(output_dir.clone());
        pipeline.test_summary_context = Some(super::super::TestResolvedSummaryContext {
            chain_section: Some("TEST_CODE_本地主线报告".to_string()),
            regime_section: Some("TEST_CODE_本地大盘状态".to_string()),
            output_dir: output_dir.clone(),
        });
        pipeline.config.send_notification = true;

        let results = pipeline
            .run(
                std::slice::from_ref(&code),
                Some("TEST_CODE_本地宏观证据".to_string()),
            )
            .await
            .expect("resolved full pipeline run");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, code);
        assert_eq!(
            results[0]
                .deep_seed
                .as_ref()
                .and_then(|seed| seed.macro_context.as_deref()),
            Some("TEST_CODE_本地宏观证据")
        );

        let artifacts = std::fs::read_dir(&output_dir)
            .expect("isolated reports")
            .collect::<Result<Vec<_>, _>>()
            .expect("report entries");
        assert!(
            artifacts.len() >= 4,
            "Bollinger/RSI reports and mandatory audits must commit: {artifacts:?}"
        );
    }

    #[tokio::test]
    async fn resolved_context_keeps_veto_dry_run_non_mutating() {
        let mut context = resolved_context(
            Ok(super::extra_context::ExtraContext {
                section: None,
                money_flow: None,
            }),
            Ok(None),
        );
        context.name_news = Some(("TEST_CODE_示例公司".to_string(), None));
        let pipeline = test_pipeline(context, false);
        let mut values = analysis_bars();
        for (index, bar) in values.iter_mut().enumerate() {
            let close = 20.0 * 0.985_f64.powi(index as i32);
            bar.open = close;
            bar.high = close + 0.2;
            bar.low = close - 0.2;
            bar.close = close;
            bar.pct_chg = 1.5;
        }
        values[0].pe_ratio = Some(500.0);
        values[0].net_profit_yoy = Some(-35.0);
        let bars = Arc::new(values);

        let result = pipeline
            .analyze_stock("TEST_CODE_000001", bars.as_slice(), bars.clone(), None)
            .await
            .expect("analysis with complete veto evidence");

        assert_eq!(crate::config::get_veto_config().mode, "dry_run");
        assert_eq!(result.sentiment_score, 60);
        let seed = result.deep_seed.expect("deep-analysis seed");
        assert!(!seed
            .trend_snapshot
            .risk_factors
            .iter()
            .any(|flag| flag.contains("基本面极度恶化")));
    }
}
