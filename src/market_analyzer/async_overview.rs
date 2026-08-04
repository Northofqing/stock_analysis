//! Synchronous post-session market-overview adapter.
//!
//! Acquisition belongs to `data_gateway`. This adapter deliberately fails
//! until a released Magic contract can prove one complete A-share market
//! breadth snapshot; it never falls back to consumer-owned wire protocols.

use crate::market_data::MarketOverview;
use anyhow::Result;
use log::warn;

/// Synchronous review entry.
pub fn get_market_overview_blocking() -> Result<MarketOverview> {
    anyhow::bail!(
        "盘后市场概览不可用: 当前 Magic 数据契约没有提供同一完整批次的 A 股指数、全市场广度、成交额与北向资金；BR-164 禁止回退到复盘消费端直连协议"
    )
}

/// 生成市场概览报告文本 (在 --review 模式直接调用, 不需要 MarketAnalyzer)
pub fn generate_market_overview_text_blocking() -> String {
    match get_market_overview_blocking() {
        Ok(overview) => {
            // 用 review.rs 的 generate_market_review 但需要 analyzer
            // 这里直接复用 format_market_report 的逻辑
            format_market_report(&overview)
        }
        Err(e) => {
            warn!("[大盘 blocking] 获取失败: {}", e);
            format!("# 📊 A股市场概览\n\n数据不可用：{e}")
        }
    }
}

/// 生成市场概览报告 (替代 MarketAnalyzer::generate_market_review)
/// 这里我们内联 review.rs 的 format_market_report 逻辑, 避免依赖 analyzer
fn format_market_report(overview: &MarketOverview) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# 📊 A股市场概览 ({})", overview.date);
    let _ = writeln!(s);
    let _ = writeln!(s, "## 一、主要指数");
    for idx in overview.indices.iter().take(5) {
        let _ = writeln!(
            s,
            "- {}: {:.2} ({:+.2}%)",
            idx.name, idx.current, idx.change_pct
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "## 二、涨跌统计");
    let _ = writeln!(s, "| 指标 | 数值 |");
    let _ = writeln!(s, "|------|------|");
    let _ = writeln!(s, "| 上涨家数 | {} |", overview.up_count);
    let _ = writeln!(s, "| 下跌家数 | {} |", overview.down_count);
    let _ = writeln!(s, "| 平盘家数 | {} |", overview.flat_count);
    let _ = writeln!(s, "| 涨停 | {} |", overview.limit_up_count);
    let _ = writeln!(s, "| 跌停 | {} |", overview.limit_down_count);
    // v19.12 修复: 500 只样本累加 ≠ 沪深两市真实成交额, 显式标注 "样本估算" 防误导
    let _ = writeln!(
        s,
        "| 两市成交额 (涨幅榜500只样本估算) | {:.0}亿 |",
        overview.total_amount
    );
    // 修复 P1-3 (2026-06-30 codex review, BR-012): None 时显式打 -, 禁止显示 0.00.
    match overview.north_flow {
        Some(v) => {
            let _ = writeln!(s, "| 北向资金 | {:+.2}亿 |", v);
        }
        None => {
            let _ = writeln!(s, "| 北向资金 | - (数据源缺失) |");
        }
    }
    let _ = writeln!(s);
    if !overview.top_sectors.is_empty() {
        let _ = writeln!(s, "## 三、领涨板块");
        for s2 in overview.top_sectors.iter() {
            let _ = writeln!(s, "- **{}**: {:+.2}%", s2.name, s2.change_pct);
        }
    }
    // 修复: 只有当存在真正下跌的板块时才显示"领跌"段
    // 之前可能显示 +0.5% 这种"最弱涨幅"被错标为"领跌", 误导
    if !overview.bottom_sectors.is_empty()
        && overview.bottom_sectors.iter().any(|s| s.change_pct < 0.0)
    {
        let _ = writeln!(s);
        let _ = writeln!(s, "## 四、领跌板块");
        for s2 in overview.bottom_sectors.iter() {
            let _ = writeln!(s, "- **{}**: {:+.2}%", s2.name, s2.change_pct);
        }
    } else if !overview.bottom_sectors.is_empty() {
        // 全市场普涨, 把最弱的几个标为"涨幅靠后"而不是"领跌"
        let _ = writeln!(s);
        let _ = writeln!(s, "## 四、涨幅靠后板块 (全市场普涨, 无下跌板块)");
        for s2 in overview.bottom_sectors.iter() {
            let _ = writeln!(s, "- **{}**: {:+.2}%", s2.name, s2.change_pct);
        }
    }

    // v19.1: 五、市场情绪判断 (启发式规则, 不接 LLM 避免限流)
    let sentiment = judge_market_sentiment(overview);
    let _ = writeln!(s);
    let _ = writeln!(s, "## 五、市场情绪判断");
    let _ = writeln!(s, "**{}**", sentiment.headline);
    for line in &sentiment.bullets {
        let _ = writeln!(s, "- {}", line);
    }
    s
}

/// 启发式市场情绪判断 (5 维打分)
#[derive(Debug)]
struct MarketSentiment {
    headline: String,
    bullets: Vec<String>,
}

fn judge_market_sentiment(overview: &MarketOverview) -> MarketSentiment {
    let mut score: i32 = 0;
    let mut bullets: Vec<String> = Vec::new();

    // 1. 涨跌家数 (up_count vs down_count)
    let total = overview.up_count + overview.down_count;
    if total > 0 {
        let up_ratio = overview.up_count as f64 / total as f64;
        if up_ratio >= 0.7 {
            score += 30;
            bullets.push(format!(
                "📈 普涨 ({} 上 / {} 下, {:.0}%)",
                overview.up_count,
                overview.down_count,
                up_ratio * 100.0
            ));
        } else if up_ratio >= 0.5 {
            score += 10;
            bullets.push(format!(
                "📊 涨多跌少 ({} / {})",
                overview.up_count, overview.down_count
            ));
        } else if up_ratio >= 0.3 {
            score -= 10;
            bullets.push(format!(
                "📉 跌多涨少 ({} / {})",
                overview.up_count, overview.down_count
            ));
        } else {
            score -= 30;
            bullets.push(format!(
                "💀 普跌 ({} 上 / {} 下)",
                overview.up_count, overview.down_count
            ));
        }
    }

    // 2. 涨停 vs 跌停 (投机情绪)
    let total_limit = overview.limit_up_count + overview.limit_down_count;
    if total_limit > 0 {
        let ratio = overview.limit_up_count as f64 / total_limit as f64;
        if ratio >= 0.8 && overview.limit_up_count >= 30 {
            score += 20;
            bullets.push(format!(
                "🚀 投机热 (涨停 {} / 跌停 {})",
                overview.limit_up_count, overview.limit_down_count
            ));
        } else if ratio <= 0.3 {
            score -= 20;
            bullets.push(format!(
                "🥶 投机冷 (涨停 {} / 跌停 {})",
                overview.limit_up_count, overview.limit_down_count
            ));
        }
    }

    // 3. 北向资金 (外资风向)
    if let Some(n) = overview.north_flow {
        if n > 50.0 {
            score += 15;
            bullets.push(format!("🌏 北向大幅流入 +{:.0}亿 (外资乐观)", n));
        } else if n > 0.0 {
            score += 5;
            bullets.push(format!("🌏 北向小幅流入 +{:.0}亿", n));
        } else if n > -50.0 {
            score -= 5;
            bullets.push(format!("🌏 北向小幅流出 {:.0}亿", n));
        } else {
            score -= 15;
            bullets.push(format!("🌏 北向大幅流出 {:.0}亿 (外资悲观)", n));
        }
    } else {
        bullets.push("🌏 北向资金: - (数据源缺失, 未计算)".to_string());
    }

    // 4. 主要指数强弱
    let sh_index = overview.indices.iter().find(|i| i.code == "000001");
    let sz_index = overview.indices.iter().find(|i| i.code == "399001");
    if let (Some(sh), Some(sz)) = (sh_index, sz_index) {
        if sh.change_pct > 0.5 && sz.change_pct > 0.5 {
            score += 10;
            bullets.push(format!(
                "💪 沪深双涨 (上证 {:+.2}%, 深证 {:+.2}%)",
                sh.change_pct, sz.change_pct
            ));
        } else if sh.change_pct < -0.5 && sz.change_pct < -0.5 {
            score -= 10;
            bullets.push(format!(
                "😰 沪深双跌 (上证 {:+.2}%, 深证 {:+.2}%)",
                sh.change_pct, sz.change_pct
            ));
        } else {
            bullets.push(format!(
                "😐 沪深分化 (上证 {:+.2}%, 深证 {:+.2}%)",
                sh.change_pct, sz.change_pct
            ));
        }
    }

    // 5. 板块普涨普跌
    if !overview.top_sectors.is_empty() && !overview.bottom_sectors.is_empty() {
        let avg_top: f64 = overview
            .top_sectors
            .iter()
            .map(|s| s.change_pct)
            .sum::<f64>()
            / overview.top_sectors.len() as f64;
        let avg_bottom: f64 = overview
            .bottom_sectors
            .iter()
            .map(|s| s.change_pct)
            .sum::<f64>()
            / overview.bottom_sectors.len() as f64;
        if avg_top > 3.0 && avg_bottom < 0.0 {
            score += 10;
            bullets.push(format!(
                "🎯 板块轮动明显 (领涨均 {avg_top:+.1}% / 涨幅靠后均 {avg_bottom:+.1}%)"
            ));
        } else if avg_top < 1.0 && avg_bottom > -1.0 {
            bullets.push(format!(
                "🌀 板块分化弱 (领涨均 {avg_top:+.1}% / 靠后均 {avg_bottom:+.1}%)"
            ));
            score -= 5;
        }
    }

    // 总分 → 标题
    let headline = if score >= 50 {
        "🟢 强势格局: 普涨 + 投机热 + 外资流入, 风险偏好高"
    } else if score >= 20 {
        "🟢 偏强: 局部机会, 注意板块轮动"
    } else if score >= -10 {
        "⚪ 中性: 多空均衡, 精选个股"
    } else if score >= -30 {
        "🔴 偏弱: 普跌 + 投机冷, 控制仓位"
    } else {
        "🔴 极弱: 系统性风险, 防御为主"
    };

    MarketSentiment {
        headline: headline.to_string(),
        bullets,
    }
}

// =============================================================================
// 测试
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_data::MarketIndex;

    #[test]
    fn blocking_overview_fails_explicitly_without_complete_magic_contract() {
        let error = get_market_overview_blocking().unwrap_err().to_string();
        assert!(error.contains("Magic 数据契约"));
        assert!(generate_market_overview_text_blocking().contains("数据不可用"));
    }

    #[test]
    fn format_market_report_basic() {
        let mut overview = MarketOverview::new("2026-06-27".to_string());
        overview.indices = vec![MarketIndex {
            code: "TEST_CODE_000001".into(),
            name: "上证指数".into(),
            current: 4139.90,
            change: 7.29,
            change_pct: 0.18,
            open: Some(4132.61),
            high: Some(4140.0),
            low: Some(4125.0),
            prev_close: 4132.61,
            volume: None,
            amount: None,
            amplitude: None,
        }];
        overview.up_count = 2500;
        overview.down_count = 2000;
        overview.north_flow = Some(12.34);
        let s = format_market_report(&overview);
        assert!(s.contains("上证指数"));
        assert!(s.contains("+0.18%"));
        assert!(s.contains("+12.34亿"));
        assert!(s.contains("2500"));
        assert!(s.contains("2000"));
    }
}
