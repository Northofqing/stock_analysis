//! 日报生成（原 notification.rs 报告块）

use chrono::Local;

use super::service::{AnalysisResult, NotificationService};

impl NotificationService {
    /// 生成 Markdown 格式的日报
    pub fn generate_daily_report(&self, results: &[AnalysisResult]) -> String {
        let report_date = Local::now().format("%Y-%m-%d").to_string();
        let now = Local::now().format("%H:%M:%S").to_string();

        let mut lines = vec![
            format!("# 📅 {} A股自选股智能分析报告", report_date),
            String::new(),
            format!("> 共分析 **{}** 只股票 | 报告生成时间：{}", results.len(), now),
            String::new(),
            "---".to_string(),
            String::new(),
        ];

        // 按评分排序
        let mut sorted_results = results.to_vec();
        sorted_results.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));

        // 统计信息
        let buy_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "买入" | "加仓" | "强烈买入" | "建议买入" | "强烈建议买入"))
            .count();
        let sell_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "卖出" | "减仓" | "建议减仓" | "建议卖出" |"强烈卖出"))
            .count();
        let hold_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "持有" | "观望"))
            .count();
        let avg_score: f64 = results.iter().map(|r| r.sentiment_score as f64).sum::<f64>()
            / results.len() as f64;

        lines.extend(vec![
            "## 📊 操作建议汇总".to_string(),
            String::new(),
            "| 指标 | 数值 |".to_string(),
            "|------|------|".to_string(),
            format!("| 🟢 建议买入/加仓 | **{}** 只 |", buy_count),
            format!("| 🟡 建议持有/观望 | **{}** 只 |", hold_count),
            format!("| 🔴 建议减仓/卖出 | **{}** 只 |", sell_count),
            format!("| 📈 平均看多评分 | **{:.1}** 分 |", avg_score),
            String::new(),
        ]);

        // 当日涨停股票汇总
        let limit_up_results: Vec<&AnalysisResult> = sorted_results.iter().filter(|r| r.is_limit_up).collect();
        if !limit_up_results.is_empty() {
            lines.extend(vec![
                "---".to_string(),
                String::new(),
                format!("## 🔥 当日涨停股票（{} 只）", limit_up_results.len()),
                String::new(),
                "| 股票 | 代码 | 评分 | 操作建议 | 趋势 |".to_string(),
                "|------|------|------|---------|------|".to_string(),
            ]);
            for r in &limit_up_results {
                lines.push(format!(
                    "| {} {} | {} | {}分 | {} | {} |",
                    r.get_emoji(), r.name, r.code, r.sentiment_score, r.operation_advice, r.trend_prediction
                ));
            }
            lines.push(String::new());
        }

        // 反向择时信号汇总（sentiment_score<40 + 技术面企稳，历史 T5 胜率 55.62%）
        let contrarian_results: Vec<&AnalysisResult> = sorted_results.iter().filter(|r| r.contrarian_signal).collect();
        if !contrarian_results.is_empty() {
            lines.extend(vec![
                "---".to_string(),
                String::new(),
                format!("## 🔄 反向择时信号（{} 只，超跌企稳）", contrarian_results.len()),
                String::new(),
                "> 基于历史回测：AI 评分 <40 区间的 T1 胜率 56.91% / T5 胜率 55.62% / T5 均涨 +2.40%，显著跑赢市场基准。本区间经『技术面企稳』二次过滤后输出。".to_string(),
                String::new(),
                "| 股票 | 代码 | 评分 | 触发理由 |".to_string(),
                "|------|------|------|---------|".to_string(),
            ]);
            for r in &contrarian_results {
                let reason = r.contrarian_reason.as_deref().unwrap_or("-");
                lines.push(format!(
                    "| {} | {} | {}分 | {} |",
                    r.name, r.code, r.sentiment_score, reason
                ));
            }
            lines.push(String::new());
        }

        // 模拟持仓汇总
        let position_results: Vec<&AnalysisResult> = sorted_results.iter().filter(|r| r.position_return.is_some()).collect();
        if !position_results.is_empty() {
            let total_profit: f64 = position_results.iter().map(|r| {
                let buy_price = r.position_buy_price.unwrap_or(0.0);
                let qty = r.position_quantity.unwrap_or(0) as f64;
                let ret = r.position_return.unwrap_or(0.0);
                buy_price * qty * ret / 100.0
            }).sum();

            lines.extend(vec![
                "---".to_string(),
                String::new(),
                format!("## 💰 模拟持仓跟踪（{} 只）", position_results.len()),
                String::new(),
                "| 股票 | 代码 | 买入价 | 现价 | 收益率 | 浮动盈亏 | 买入日期 |".to_string(),
                "|------|------|--------|------|--------|---------|---------|".to_string(),
            ]);
            for r in &position_results {
                let buy_price = r.position_buy_price.unwrap_or(0.0);
                let ret = r.position_return.unwrap_or(0.0);
                let qty = r.position_quantity.unwrap_or(0) as f64;
                let profit = buy_price * qty * ret / 100.0;
                let cur_price = r.current_price.unwrap_or(0.0);
                let date = r.position_buy_date.as_deref().unwrap_or("-");
                let ret_str = if ret >= 0.0 { format!("🟢 {:+.2}%", ret) } else { format!("🔴 {:+.2}%", ret) };
                lines.push(format!(
                    "| {} | {} | {:.2} | {:.2} | {} | {:+.2} | {} |",
                    r.name, r.code, buy_price, cur_price, ret_str, profit, date
                ));
            }
            lines.push(String::new());
            let total_emoji = if total_profit >= 0.0 { "📈" } else { "📉" };
            lines.push(format!("**{} 持仓总浮动盈亏：{:+.2} 元**", total_emoji, total_profit));
            lines.push(String::new());
        }

        lines.extend(vec![
            "---".to_string(),
            String::new(),
            "## 📈 个股详细分析".to_string(),
            String::new(),
        ]);

        // 逐个股票的详细分析
        for result in &sorted_results {
            let emoji = result.get_emoji();
            let limit_up_tag = if result.is_limit_up { " 🔥涨停" } else { "" };
            let contrarian_tag = if result.contrarian_signal { " 🔄反向信号" } else { "" };

            lines.push(format!("### {} {} ({}){}{}", emoji, result.name, result.code, limit_up_tag, contrarian_tag));
            lines.push(String::new());
            lines.push(format!(
                "**操作建议：{}** | **综合评分：{}分** | **趋势预测：{}**",
                result.operation_advice, result.sentiment_score, result.trend_prediction
            ));
            lines.push(String::new());

            // 模拟持仓收益展示
            if let (Some(buy_price), Some(return_rate), Some(quantity)) = (result.position_buy_price, result.position_return, result.position_quantity) {
                let return_emoji = if return_rate > 0.0 { "📈" } else { "📉" };
                let buy_date_str = result.position_buy_date.as_deref().unwrap_or("未知");
                let profit = buy_price * quantity as f64 * return_rate / 100.0;

                if result.position_status.as_deref() == Some("closed") {
                    // 本次触发卖出，显示实现盈亏
                    let sell_price = result.position_sell_price.unwrap_or(0.0);
                    let sell_date = result.position_sell_date.as_deref().unwrap_or("-");
                    lines.push(format!(
                        "**✅ 模拟持仓（已卖出）**：买入 {:.2} → 卖出 {:.2} | {} 股 | 收益率 **{:+.2}%** | 实现盈亏 **{:+.2} 元** | 买入 {} / 卖出 {}",
                        buy_price, sell_price, quantity, return_rate, profit, buy_date_str, sell_date
                    ));
                } else {
                    let status_tag = if result.position_status.as_deref() == Some("new") { "🟢 新建仓" } else { "📊 持仓中" };
                    lines.push(format!(
                        "**{} 模拟持仓（{}）**：买入价 {:.2} | {} 股 | 收益率 **{:+.2}%** | 浮动盈亏 **{:+.2} 元** | 买入日期 {}",
                        return_emoji, status_tag, buy_price, quantity, return_rate, profit, buy_date_str
                    ));
                }
                lines.push(String::new());
            }

            // 操作理由
            if let Some(buy_reason) = &result.buy_reason {
                lines.push(format!("**💡 操作理由**：{}", buy_reason));
                lines.push(String::new());
            }

            // // ========== 均线与价格位置 ==========
            // let has_ma_data = result.current_price.is_some() && result.ma5.is_some();
            // if has_ma_data {
            //     lines.push("#### 📈 均线与价格位置".to_string());
            //     lines.push(String::new());
            //     lines.push("| 项目 | 价格 | 说明 |".to_string());
            //     lines.push("|------|------|------|".to_string());

            //     if let Some(price) = result.current_price {
            //         lines.push(format!("| 当前价 | {:.2} | - |", price));
            //     }
            //     if let Some(ma5) = result.ma5 {
            //         let bias_str = result.bias_ma5
            //             .map(|b| {
            //                 let warn = if b.abs() > 5.0 { " ⚠️偏离过大" } else { "" };
            //                 format!("乖离率: {:.2}%{}", b, warn)
            //             })
            //             .unwrap_or_default();
            //         lines.push(format!("| MA5 | {:.2} | {} |", ma5, bias_str));
            //     }
            //     if let Some(ma10) = result.ma10 {
            //         lines.push(format!("| MA10 | {:.2} | - |", ma10));
            //     }
            //     if let Some(ma20) = result.ma20 {
            //         lines.push(format!("| MA20 | {:.2} | - |", ma20));
            //     }
            //     if let Some(ma60) = result.ma60 {
            //         lines.push(format!("| MA60 | {:.2} | 中期趋势 |", ma60));
            //     }

            //     if let Some(ref alignment) = result.ma_alignment {
            //         lines.push(format!("| 排列状态 | {} | - |", alignment));
            //     }

            //     lines.push(String::new());
            // }

            // // ========== 52周/季度价格区间 ==========
            // let has_range = result.high_52w.is_some() || result.high_quarter.is_some();
            // if has_range {
            //     lines.push("#### 📏 价格区间".to_string());
            //     lines.push(String::new());
            //     lines.push("| 区间 | 最高 | 最低 | 当前位置 |".to_string());
            //     lines.push("|------|------|------|---------|".to_string());

            //     if let (Some(h), Some(l), Some(p)) = (result.high_52w, result.low_52w, result.pos_52w) {
            //         let pos_desc = if p > 80.0 { "接近高点 ⚠️" }
            //             else if p < 20.0 { "接近低点 ✅" }
            //             else { "" };
            //         lines.push(format!("| 52周 | {:.2} | {:.2} | {:.1}% {} |", h, l, p, pos_desc));
            //     }
            //     if let (Some(h), Some(l), Some(p)) = (result.high_quarter, result.low_quarter, result.pos_quarter) {
            //         lines.push(format!("| 近一季 | {:.2} | {:.2} | {:.1}% |", h, l, p));
            //     }

            //     lines.push(String::new());
            // }

            // ========== 量能与近期走势 ==========
            let has_momentum = result.volume_ratio_5d.is_some() || result.chg_5d.is_some();
            if has_momentum {
                lines.push("#### 📊 量能与近期走势".to_string());
                lines.push(String::new());

                if let Some(vr) = result.volume_ratio_5d {
                    let vol_status = if vr > 2.0 { "显著放量" }
                        else if vr > 1.2 { "温和放量" }
                        else if vr > 0.8 { "量能平稳" }
                        else { "明显缩量" };
                    lines.push(format!("- **5日量比**: {:.2} ({})", vr, vol_status));
                }
                if let Some(chg) = result.chg_5d {
                    lines.push(format!("- **近5日涨幅**: {:.2}%", chg));
                }
                if let Some(chg) = result.chg_10d {
                    lines.push(format!("- **近10日涨幅**: {:.2}%", chg));
                }
                if let Some(vol) = result.volatility {
                    let vol_level = if vol > 5.0 { "⚠️ 波动剧烈" }
                        else if vol > 3.0 { "波动较大" }
                        else { "波动正常" };
                    lines.push(format!("- **日波动率**: {:.2}% ({})", vol, vol_level));
                }

                lines.push(String::new());
            }

            // ========== 估值指标 ==========
            let has_valuation = result.pe_ratio.is_some() || result.pb_ratio.is_some()
                || result.market_cap.is_some() || result.turnover_rate.is_some();
            if has_valuation {
                lines.push("#### 💰 估值指标".to_string());
                lines.push(String::new());
                lines.push("| 指标 | 数值 | 评估 |".to_string());
                lines.push("|------|------|------|".to_string());

                if let Some(pe) = result.pe_ratio {
                    let a = if pe < 0.0 { "亏损" }
                        else if pe < 15.0 { "✅ 合理" }
                        else if pe < 30.0 { "⚠️ 适中" }
                        else { "🔴 偏高" };
                    lines.push(format!("| PE | {:.2} | {} |", pe, a));
                }
                if let Some(pb) = result.pb_ratio {
                    let a = if pb < 1.0 { "✅ 可能低估" }
                        else if pb < 3.0 { "正常" }
                        else { "🔴 偏高" };
                    lines.push(format!("| PB | {:.2} | {} |", pb, a));
                }
                if let Some(t) = result.turnover_rate {
                    let a = if t < 1.0 { "极度清淡" }
                        else if t < 3.0 { "清淡" }
                        else if t < 7.0 { "正常" }
                        else if t < 15.0 { "活跃" }
                        else { "火热" };
                    lines.push(format!("| 换手率 | {:.2}% | {} |", t, a));
                }
                if let Some(mc) = result.market_cap {
                    let cap = if mc < 50.0 { "小盘" }
                        else if mc < 300.0 { "中盘" }
                        else if mc < 1000.0 { "大盘" }
                        else { "超大盘" };
                    lines.push(format!("| 总市值 | {:.2}亿 | {} |", mc, cap));
                }
                if let Some(cc) = result.circulating_cap {
                    lines.push(format!("| 流通市值 | {:.2}亿 | - |", cc));
                }

                lines.push(String::new());
            }

            // ========== 财务指标 ==========
            let has_financials = result.eps.is_some() || result.roe.is_some()
                || result.gross_margin.is_some() || result.revenue_yoy.is_some();
            if has_financials {
                lines.push("#### 📋 财务指标".to_string());
                lines.push(String::new());
                lines.push("| 指标 | 数值 | 评估 |".to_string());
                lines.push("|------|------|------|".to_string());

                if let Some(eps) = result.eps {
                    let a = if eps < 0.0 { "亏损" }
                        else if eps < 0.5 { "较弱" }
                        else if eps < 2.0 { "正常" }
                        else { "✅ 优秀" };
                    lines.push(format!("| EPS | {:.3}元 | {} |", eps, a));
                }
                if let Some(roe) = result.roe {
                    let a = if roe < 5.0 { "较低" }
                        else if roe < 15.0 { "正常" }
                        else if roe < 25.0 { "✅ 优秀" }
                        else { "🌟 卓越" };
                    lines.push(format!("| ROE | {:.2}% | {} |", roe, a));
                }
                if let Some(gm) = result.gross_margin {
                    let a = if gm < 20.0 { "竞争激烈" }
                        else if gm < 40.0 { "正常" }
                        else { "✅ 高壁垒" };
                    lines.push(format!("| 毛利率 | {:.2}% | {} |", gm, a));
                }
                if let Some(nm) = result.net_margin {
                    lines.push(format!("| 净利率 | {:.2}% | - |", nm));
                }
                if let Some(r) = result.revenue_yoy {
                    let a = if r < 0.0 { "🔴 下滑" }
                        else if r < 10.0 { "缓慢" }
                        else if r < 30.0 { "✅ 稳健" }
                        else { "🚀 高速" };
                    lines.push(format!("| 营收同比 | {:.2}% | {} |", r, a));
                }
                if let Some(p) = result.net_profit_yoy {
                    let a = if p < -20.0 { "🔴 大幅下滑" }
                        else if p < 0.0 { "⚠️ 下滑" }
                        else if p < 20.0 { "稳定" }
                        else { "✅ 高速增长" };
                    lines.push(format!("| 净利润同比 | {:.2}% | {} |", p, a));
                }
                if let Some(sr) = result.sharpe_ratio {
                    let a = if sr < 0.0 { "🔴 亏损" }
                        else if sr < 1.0 { "一般" }
                        else if sr < 2.0 { "✅ 良好" }
                        else { "🌟 优秀" };
                    lines.push(format!("| 夏普比率 | {:.2} | {} |", sr, a));
                }

                lines.push(String::new());
            }

            // 技术面分析（原有文本）
            let mut tech_lines = Vec::new();
            if let Some(tech) = &result.technical_analysis {
                tech_lines.push(format!("**综合**：{}", tech));
            }
            if let Some(vol) = &result.volume_analysis {
                tech_lines.push(format!("**量能**：{}", vol));
            }
            if !tech_lines.is_empty() {
                lines.push("#### 🔍 技术面分析".to_string());
                lines.extend(tech_lines);
                lines.push(String::new());
            }

            // 消息面
            if let Some(news) = &result.news_summary {
                lines.push("#### 📰 消息面".to_string());
                lines.push(news.clone());
                lines.push(String::new());
            }

            // 真实主力资金流 / 日内分时 / 龙虎榜席位
            if let Some(mf) = &result.money_flow_section {
                if !mf.trim().is_empty() {
                    lines.push("#### 💰 资金面（真实口径）".to_string());
                    lines.push(String::new());
                    lines.extend(format_money_flow_section(mf));
                    lines.push(String::new());
                }
            }

            // 布林+MACD 共振信号（4 条核心规则）
            if let Some(bm) = &result.boll_macd {
                if bm.action != crate::strategy::BollMacdAction::None {
                    lines.push("#### 📊 布林+MACD 共振信号".to_string());
                    lines.push(String::new());
                    lines.push("| 项目 | 数值 | 说明 |".to_string());
                    lines.push("|------|------|------|".to_string());
                    let icon = if bm.action.is_buy() {
                        "🟢"
                    } else if bm.action.is_sell() {
                        "🔴"
                    } else {
                        "🟡"
                    };
                    lines.push(format!(
                        "| 信号动作 | {} {} | {} |",
                        icon, bm.action.name(), bm.reason
                    ));
                    lines.push(format!(
                        "| 布林轨道 | 上 ¥{:.2} / 中 ¥{:.2} / 下 ¥{:.2} | 收 ¥{:.2} |",
                        bm.upper, bm.middle, bm.lower, bm.close
                    ));
                    lines.push(format!(
                        "| 带宽 | {:.2}% | 5日变化 {:+.2}%（>0张口/<0收口） |",
                        bm.band_width_pct, bm.band_change_pct
                    ));
                    lines.push(format!(
                        "| MACD | DIF {:.3} / DEA {:.3} / HIST {:.3} | 背离: {:?} |",
                        bm.macd_dif, bm.macd_dea, bm.macd_hist, bm.macd_div
                    ));
                    lines.push(String::new());
                }
            }

            // 综合分析
            lines.push("#### 📝 综合分析".to_string());
            lines.push(result.analysis_summary.clone());
            lines.push(String::new());

            // 风险提示
            if let Some(risk) = &result.risk_warning {
                lines.push(format!("⚠️ **风险提示**：{}", risk));
                lines.push(String::new());
            }

            lines.push(String::new());
            lines.push("---".to_string());
            lines.push(String::new());
        }

        // 底部信息
        lines.push(String::new());
        lines.push(format!(
            "*报告生成时间：{}*",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        ));

        lines.join("\n")
    }

    /// 生成精简版日报（用于企业微信）
    pub fn generate_wechat_summary(&self, results: &[AnalysisResult]) -> String {
        let report_date = Local::now().format("%Y-%m-%d").to_string();

        let mut sorted_results = results.to_vec();
        sorted_results.sort_by(|b, a| a.sentiment_score.cmp(&b.sentiment_score));

        let buy_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "买入" | "加仓" | "强烈买入"))
            .count();
        let sell_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "卖出" | "减仓" | "强烈卖出"))
            .count();
        let hold_count = results
            .iter()
            .filter(|r| matches!(r.operation_advice.as_str(), "持有" | "观望"))
            .count();
        let avg_score: f64 = results.iter().map(|r| r.sentiment_score as f64).sum::<f64>()
            / results.len() as f64;

        let mut lines = vec![
            format!("## 📅 {} A股分析报告", report_date),
            String::new(),
            format!(
                "> 共 **{}** 只 | 🟢买入:{} 🟡持有:{} 🔴卖出:{} | 均分:{:.0}",
                results.len(),
                buy_count,
                hold_count,
                sell_count,
                avg_score
            ),
            String::new(),
        ];

        for result in &sorted_results {
            let emoji = result.get_emoji();
            lines.push(format!("### {} {}({})", emoji, result.name, result.code));
            lines.push(format!(
                "**{}** | 评分:{} | {}",
                result.operation_advice, result.sentiment_score, result.trend_prediction
            ));

            // 紧凑的核心指标行
            let mut indicators = Vec::new();
            if let Some(price) = result.current_price {
                indicators.push(format!("价:{:.2}", price));
            }
            if let Some(ref align) = result.ma_alignment {
                indicators.push(align.clone());
            }
            if let Some(bias) = result.bias_ma5 {
                if bias.abs() > 3.0 {
                    indicators.push(format!("乖离:{:.1}%⚠️", bias));
                }
            }
            if let Some(vr) = result.volume_ratio_5d {
                let vs = if vr > 2.0 { "放量" } else if vr < 0.8 { "缩量" } else { "" };
                if !vs.is_empty() {
                    indicators.push(format!("量比{:.1}({})", vr, vs));
                }
            }
            if let Some(p) = result.pos_52w {
                indicators.push(format!("52周位:{:.0}%", p));
            }
            if !indicators.is_empty() {
                lines.push(format!("📈 {}", indicators.join(" | ")));
            }

            // 紧凑的估值/财务行
            let mut val_parts = Vec::new();
            if let Some(pe) = result.pe_ratio {
                val_parts.push(format!("PE:{:.1}", pe));
            }
            if let Some(pb) = result.pb_ratio {
                val_parts.push(format!("PB:{:.1}", pb));
            }
            if let Some(roe) = result.roe {
                val_parts.push(format!("ROE:{:.1}%", roe));
            }
            if let Some(chg) = result.chg_5d {
                val_parts.push(format!("5日:{:+.1}%", chg));
            }
            if !val_parts.is_empty() {
                lines.push(format!("💰 {}", val_parts.join(" | ")));
            }

            if let Some(reason) = &result.buy_reason {
                let truncated = if reason.len() > 80 {
                    format!("{}...", &reason[..77])
                } else {
                    reason.clone()
                };
                lines.push(format!("💡 {}", truncated));
            }

            if let Some(risk) = &result.risk_warning {
                let truncated = if risk.len() > 50 {
                    format!("{}...", &risk[..47])
                } else {
                    risk.clone()
                };
                lines.push(format!("⚠️ {}", truncated));
            }

            lines.push(String::new());
        }

        lines.push("---".to_string());
        lines.push("*AI生成，仅供参考，不构成投资建议*".to_string());
        lines.push(format!(
            "*详细报告见 reports/report_{}.md*",
            report_date.replace("-", "")
        ));

        lines.join("\n")
    }

}

/// 将 `extra_context` 输出的 `【段落】+ 文本` 片段格式化为与日报其它部分一致的 Markdown
/// （子标题 + 表格 / 列表），避免使用代码块包裹原始文本。
fn format_money_flow_section(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut cur_title: Option<String> = None;
    let mut cur_body: Vec<String> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let (Some(start), Some(end)) = (trimmed.find('【'), trimmed.rfind('】')) {
            if start == 0 && end + '】'.len_utf8() == trimmed.len() {
                if let Some(title) = cur_title.take() {
                    blocks.push((title, std::mem::take(&mut cur_body)));
                }
                let title = trimmed[start + '【'.len_utf8()..end].to_string();
                cur_title = Some(title);
                continue;
            }
        }
        cur_body.push(trimmed.to_string());
    }
    if let Some(title) = cur_title.take() {
        blocks.push((title, cur_body));
    }

    for (title, body) in blocks {
        if body.is_empty() {
            continue;
        }
        // 用加粗代替 H5，避免与 AI 综合分析正文里的 H1/H2 嵌套打架
        out.push(format!("**{}**", title));
        out.push(String::new());

        // 通用表格识别：首行含 `|` 即视作表头，连续含 `|` 且列数一致的后续行视作数据行
        let mut idx = 0;
        if let Some(header) = body.first() {
            if header.contains('|') {
                let cols: Vec<&str> = header.split('|').map(|s| s.trim()).collect();
                if cols.len() >= 2 && cols.iter().all(|c| !c.is_empty()) {
                    out.push(format!("| {} |", cols.join(" | ")));
                    out.push(format!(
                        "|{}|",
                        cols.iter().map(|_| "------").collect::<Vec<_>>().join("|")
                    ));
                    idx = 1;
                    while idx < body.len() {
                        let row = &body[idx];
                        if !row.contains('|') {
                            break;
                        }
                        let cells: Vec<&str> = row.split('|').map(|s| s.trim()).collect();
                        if cells.len() != cols.len() {
                            break;
                        }
                        out.push(format!("| {} |", cells.join(" | ")));
                        idx += 1;
                    }
                    out.push(String::new());
                }
            }
        }
        for line in &body[idx..] {
            out.push(format!("- {}", line));
        }
        out.push(String::new());
    }

    // 末尾留白由调用方控制，去掉尾部空行
    while out.last().map(|s| s.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}
