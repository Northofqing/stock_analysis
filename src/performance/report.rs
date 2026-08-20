//! 归因报告渲染 — 全文 markdown + 推送摘要 (spec §4.4).

use super::attribution::{DailyAttribution, FamilyAggregate, SignalFamily, WindowAttribution};

/// 千分位 + 符号金额: -8120 → "-8,120"
fn fmt_money(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let abs = v.abs().round();
    let digits = format!("{abs:.0}");
    let mut out = String::new();
    for (i, ch) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{sign}{}", out.chars().rev().collect::<String>())
}

/// 推送摘要 (~20 行, spec §4.4; 族按 |合计PnL| 降序, 序号由 ranks 数组索引 — 无硬编码)
pub fn render_summary(daily: &DailyAttribution, window: &WindowAttribution) -> String {
    let date = daily.date.format("%Y-%m-%d");
    let today_total: f64 = daily.families.iter().map(|f| f.total_pnl).sum();
    let win_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
    let win_unreal: f64 = window.families.iter().map(|f| f.unrealized_pnl).sum();
    let mut lines = vec![
        format!("📊 虚拟盘归因 {date}"),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
        format!("【今日】合计 {:<12}", fmt_money(today_total)),
        format!("【30天】已实现 {:<8} 期末浮盈 {}", fmt_money(win_realized), fmt_money(win_unreal)),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
    ];
    let mut families: Vec<&FamilyAggregate> = daily.families.iter().collect();
    families.sort_by(|a, b| b.total_pnl.abs().partial_cmp(&a.total_pnl.abs()).unwrap_or(std::cmp::Ordering::Equal));
    let ranks = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];
    for (i, f) in families.iter().enumerate() {
        let rank = ranks.get(i).copied().unwrap_or("•");
        let label = match f.family {
            SignalFamily::PostCloseFundInflow => "盘后资金流入",
            SignalFamily::ExitByRule => "ExitByRule(卖)",
            other => other.as_str(),
        };
        let win = f.win_rate.map(|w| format!("胜率{:.0}%", w * 100.0)).unwrap_or_else(|| "胜率-".to_string());
        lines.push(format!(
            "{} {:<8} {:>6}笔 {:<10} {}",
            rank, label, f.realized_trades, fmt_money(f.realized_pnl), win
        ));
    }
    lines.push("━━━━━━━━━━━━━━━━━━━━".to_string());
    let suspicious: i64 = daily.families.iter().map(|f| f.suspicious_lots).sum();
    let unvalued: i64 = daily.families.iter().map(|f| f.unvalued_lots).sum();
    let unknown: i64 = daily
        .families
        .iter()
        .filter(|f| f.family == SignalFamily::Unknown)
        .map(|f| f.open_lots + f.realized_trades)
        .sum();
    let mut quality = Vec::new();
    if suspicious > 0 {
        quality.push(format!("⚠ 数据存疑 {suspicious}笔"));
    }
    if unvalued > 0 {
        quality.push(format!("⚠ 未估值 {unvalued} lot"));
    }
    if unknown > 0 {
        quality.push(format!("⚠ Unknown {unknown}"));
    }
    if !quality.is_empty() {
        lines.push(quality.join("  |  "));
    }
    lines.join("\n")
}

/// 全文 markdown (spec §4.4 五节)
pub fn render_full_markdown(daily: &DailyAttribution, window: &WindowAttribution) -> String {
    let date = daily.date.format("%Y-%m-%d");
    let mut out = vec![format!("# 虚拟盘归因 {date}"), String::new()];
    out.push("## 数据质量审计".to_string());
    for f in &daily.families {
        if f.suspicious_lots > 0 || f.unvalued_lots > 0 {
            out.push(format!(
                "- {}: 存疑 {} lot / 未估值 {} lot",
                f.family.as_str(), f.suspicious_lots, f.unvalued_lots
            ));
        }
    }
    if !daily.families.iter().any(|f| f.suspicious_lots > 0 || f.unvalued_lots > 0) {
        out.push("- 无数据质量问题".to_string());
    }
    out.push(String::new());
    out.push("## 今日归因".to_string());
    out.push("| 信号族 | 已实现 | 浮盈 | 合计 | 笔数 | 胜率 |".to_string());
    out.push("|---|---|---|---|---|---|".to_string());
    for f in &daily.families {
        out.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            f.family.as_str(), fmt_money(f.realized_pnl), fmt_money(f.unrealized_pnl),
            fmt_money(f.total_pnl), f.realized_trades,
            f.win_rate.map(|w| format!("{:.0}%", w * 100.0)).unwrap_or_else(|| "-".to_string())
        ));
    }
    out.push(String::new());
    out.push("## 30 天滚动窗口".to_string());
    out.push("| 信号族 | 已实现累计 | 期末浮盈 | 合计 | 胜率 |".to_string());
    out.push("|---|---|---|---|---|".to_string());
    for f in &window.families {
        out.push(format!(
            "| {} | {} | {} | {} | {} |",
            f.family.as_str(), fmt_money(f.realized_pnl), fmt_money(f.unrealized_pnl),
            fmt_money(f.total_pnl),
            f.win_rate.map(|w| format!("{:.0}%", w * 100.0)).unwrap_or_else(|| "-".to_string())
        ));
    }
    out.push(String::new());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn family(f: SignalFamily, realized: f64, unreal: f64, trades: i64, wins: i64, lots: i64, unvalued: i64, suspicious: i64) -> FamilyAggregate {
        FamilyAggregate {
            family: f,
            realized_trades: trades,
            realized_pnl: realized,
            open_lots: lots,
            unrealized_pnl: unreal,
            total_pnl: realized + unreal,
            wins,
            losses: trades - wins,
            win_rate: (trades > 0).then_some(wins as f64 / trades as f64),
            unvalued_lots: unvalued,
            suspicious_lots: suspicious,
        }
    }

    fn daily() -> DailyAttribution {
        DailyAttribution {
            date: NaiveDate::from_ymd_opt(2026, 8, 20).expect("date"),
            families: vec![
                family(SignalFamily::NewsCatalyst, -8120.0, -56000.0, 506, 192, 473, 0, 0),
                family(SignalFamily::PostCloseFundInflow, -3900.0, 1200.0, 270, 84, 135, 12, 27),
            ],
        }
    }

    fn window() -> WindowAttribution {
        WindowAttribution {
            days: 30,
            end: NaiveDate::from_ymd_opt(2026, 8, 20).expect("date"),
            families: daily().families.clone(),
        }
    }

    #[test]
    fn summary_contains_family_lines_and_quality_section() {
        let text = render_summary(&daily(), &window());
        assert!(text.contains("📊 虚拟盘归因"));
        assert!(text.contains("NewsCatalyst"));
        assert!(text.contains("盘后资金流入"));
        assert!(text.contains("-8,120"));
        assert!(text.contains("数据存疑"));
        assert!(text.contains("27"));
        assert!(text.contains("未估值"));
    }

    #[test]
    fn full_markdown_has_sections() {
        let md = render_full_markdown(&daily(), &window());
        assert!(md.contains("# 虚拟盘归因"));
        assert!(md.contains("## 数据质量审计"));
        assert!(md.contains("## 今日归因"));
        assert!(md.contains("## 30 天滚动窗口"));
    }

    #[test]
    fn no_test_strings_leak_into_output() {
        // v15 规则: 测试文本不进生产路径 (spec Global Constraints)
        let text = render_summary(&daily(), &window());
        for forbidden in ["first", "second", "mock", "stub", "test kept", "placeholder", "fake", "sample"] {
            assert!(!text.contains(forbidden), "forbidden test string leaked: {forbidden}");
        }
    }
}
