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

/// 千分位 + 显式正号: -8120 → "-8,120", 8120 → "+8,120"
/// (spec §4.4 摘要示例 `⚠ 数据存疑 27笔 (+¥582k)` 的符号约定 — 正数带 "+").
fn fmt_signed_money(v: f64) -> String {
    if v >= 0.0 {
        format!("+{}", fmt_money(v))
    } else {
        fmt_money(v)
    }
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
        format!(
            "【30天】已实现 {:<8} 期末浮盈 {}",
            fmt_money(win_realized),
            fmt_money(win_unreal)
        ),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
    ];
    let mut families: Vec<&FamilyAggregate> = daily.families.iter().collect();
    families.sort_by(|a, b| {
        b.total_pnl
            .abs()
            .partial_cmp(&a.total_pnl.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let ranks = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];
    for (i, f) in families.iter().enumerate() {
        let rank = ranks.get(i).copied().unwrap_or("•");
        let label = match f.family {
            SignalFamily::PostCloseFundInflow => "盘后资金流入",
            SignalFamily::ExitByRule => "ExitByRule(卖)",
            other => other.as_str(),
        };
        let win = f
            .win_rate
            .map(|w| format!("胜率{:.0}%", w * 100.0))
            .unwrap_or_else(|| "胜率-".to_string());
        lines.push(format!(
            "{} {:<8} {:>6}笔 {:<10} {}",
            rank,
            label,
            f.realized_trades,
            fmt_money(f.realized_pnl),
            win
        ));
    }
    lines.push("━━━━━━━━━━━━━━━━━━━━".to_string());
    let suspicious: i64 = daily.families.iter().map(|f| f.suspicious_lots).sum();
    let suspicious_pnl: f64 = daily.families.iter().map(|f| f.suspicious_pnl).sum();
    let unvalued: i64 = daily.families.iter().map(|f| f.unvalued_lots).sum();
    let unknown: i64 = daily
        .families
        .iter()
        .filter(|f| f.family == SignalFamily::Unknown)
        .map(|f| f.open_lots + f.realized_trades)
        .sum();
    let mut quality = Vec::new();
    if suspicious > 0 {
        // spec §4.4.2 影响金额: 已实现口径 (suspicious_pnl), 正数带 "+"
        quality.push(format!(
            "⚠ 数据存疑 {suspicious}笔 ({})",
            fmt_signed_money(suspicious_pnl)
        ));
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
    let mut suspicious_count: i64 = 0;
    let mut suspicious_total: f64 = 0.0;
    for f in &daily.families {
        if f.suspicious_lots > 0 || f.unvalued_lots > 0 {
            // spec §4.4.2: 可疑 lot 计数/族/影响金额 (已实现口径, 正数带 "+")
            out.push(format!(
                "- {}: 存疑 {} lot ({}) / 未估值 {} lot",
                f.family.as_str(),
                f.suspicious_lots,
                fmt_signed_money(f.suspicious_pnl),
                f.unvalued_lots
            ));
        }
        suspicious_count += f.suspicious_lots;
        suspicious_total += f.suspicious_pnl;
    }
    if suspicious_count > 0 {
        out.push(format!(
            "- 数据存疑 合计 {suspicious_count}笔 ({})",
            fmt_signed_money(suspicious_total)
        ));
    }
    if !daily
        .families
        .iter()
        .any(|f| f.suspicious_lots > 0 || f.unvalued_lots > 0)
    {
        out.push("- 无数据质量问题".to_string());
    }
    out.push(String::new());
    out.push("## 今日归因".to_string());
    out.push("| 信号族 | 已实现 | 浮盈 | 合计 | 笔数 | 胜率 |".to_string());
    out.push("|---|---|---|---|---|---|".to_string());
    for f in &daily.families {
        out.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            f.family.as_str(),
            fmt_money(f.realized_pnl),
            fmt_money(f.unrealized_pnl),
            fmt_money(f.total_pnl),
            f.realized_trades,
            f.win_rate
                .map(|w| format!("{:.0}%", w * 100.0))
                .unwrap_or_else(|| "-".to_string())
        ));
    }
    out.push(String::new());
    out.push("## 30 天滚动窗口".to_string());
    out.push("| 信号族 | 已实现累计 | 期末浮盈 | 合计 | 胜率 |".to_string());
    out.push("|---|---|---|---|---|".to_string());
    for f in &window.families {
        out.push(format!(
            "| {} | {} | {} | {} | {} |",
            f.family.as_str(),
            fmt_money(f.realized_pnl),
            fmt_money(f.unrealized_pnl),
            fmt_money(f.total_pnl),
            f.win_rate
                .map(|w| format!("{:.0}%", w * 100.0))
                .unwrap_or_else(|| "-".to_string())
        ));
    }
    out.push(String::new());
    out.push("## Top 亏损/盈利交易明细".to_string());
    if daily.top_trades.is_empty() {
        out.push("无".to_string());
    } else {
        // spec §4.4 item 5: 当日, 盈利/亏损各 ≤5, 每行含 code/plan_id/盈亏/入场族
        out.push("| 方向 | 代码 | 入场plan | 盈亏 | 入场族 |".to_string());
        out.push("|---|---|---|---|---|".to_string());
        for t in &daily.top_trades {
            let side = if t.pnl >= 0.0 { "盈利" } else { "亏损" };
            out.push(format!(
                "| {} | {} | {} | {} | {} |",
                side,
                t.code,
                t.entry_plan_id,
                fmt_money(t.pnl),
                t.entry_family.as_str()
            ));
        }
    }
    out.push(String::new());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::attribution::TradeAttribution;
    use chrono::NaiveDate;

    // TEST_CODE fixture exposes every aggregate dimension used by rendering assertions.
    #[allow(clippy::too_many_arguments)]
    fn family(
        f: SignalFamily,
        realized: f64,
        unreal: f64,
        trades: i64,
        wins: i64,
        lots: i64,
        unvalued: i64,
        suspicious: i64,
        suspicious_pnl: f64,
    ) -> FamilyAggregate {
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
            suspicious_pnl,
        }
    }

    fn daily() -> DailyAttribution {
        DailyAttribution {
            date: NaiveDate::from_ymd_opt(2026, 8, 20).expect("date"),
            families: vec![
                family(
                    SignalFamily::NewsCatalyst,
                    -8120.0,
                    -56000.0,
                    506,
                    192,
                    473,
                    0,
                    0,
                    0.0,
                ),
                family(
                    SignalFamily::PostCloseFundInflow,
                    -3900.0,
                    1200.0,
                    270,
                    84,
                    135,
                    12,
                    27,
                    582000.0,
                ),
            ],
            top_trades: vec![],
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
        assert!(text.contains("+582,000")); // spec §4.4.2 影响金额 (已实现口径, 正数带 "+")
        assert!(text.contains("未估值"));
    }

    #[test]
    fn full_markdown_has_sections() {
        let md = render_full_markdown(&daily(), &window());
        assert!(md.contains("# 虚拟盘归因"));
        assert!(md.contains("## 数据质量审计"));
        assert!(md.contains("## 今日归因"));
        assert!(md.contains("## 30 天滚动窗口"));
        assert!(md.contains("## Top 亏损/盈利交易明细"));
    }

    #[test]
    fn full_markdown_audit_shows_count_and_impact_amount() {
        let md = render_full_markdown(&daily(), &window());
        // spec §4.4.2: 计数 + 影响金额, 族级与合计两级
        assert!(md.contains("PostCloseFundInflow: 存疑 27 lot (+582,000) / 未估值 12 lot"));
        assert!(md.contains("数据存疑 合计 27笔 (+582,000)"));
    }

    #[test]
    fn full_markdown_top_trades_rows_carry_four_required_fields() {
        let mut d = daily();
        let date = d.date;
        d.top_trades = vec![
            TradeAttribution {
                sell_id: 1,
                code: "TEST_CODE_600000".to_string(),
                pnl: 8120.0,
                entry_plan_id: "news-1".to_string(),
                entry_family: SignalFamily::NewsCatalyst,
                exit_reason: "BR-234四大铁律卖出".to_string(),
                suspicious: false,
                sell_date: date,
            },
            TradeAttribution {
                sell_id: 2,
                code: "TEST_CODE_600001".to_string(),
                pnl: -1200.0,
                entry_plan_id: "fund-2".to_string(),
                entry_family: SignalFamily::PostCloseFundInflow,
                exit_reason: "BR-234四大铁律卖出".to_string(),
                suspicious: false,
                sell_date: date,
            },
        ];
        let md = render_full_markdown(&d, &window());
        assert!(md.contains("| 盈利 | TEST_CODE_600000 | news-1 | 8,120 | NewsCatalyst |"));
        assert!(md.contains("| 亏损 | TEST_CODE_600001 | fund-2 | -1,200 | PostCloseFundInflow |"));
    }

    #[test]
    fn full_markdown_top_trades_section_prints_dang_when_empty() {
        // 空明细不静默: 打印一行 "无"
        let md = render_full_markdown(&daily(), &window());
        assert!(md.contains("## Top 亏损/盈利交易明细\n无"));
    }

    #[test]
    fn no_test_strings_leak_into_output() {
        // v15 规则: 测试文本不进生产路径 (spec Global Constraints)
        let text = render_summary(&daily(), &window());
        for forbidden in [
            "first",
            "second",
            "mock",
            "stub",
            "test kept",
            "placeholder",
            "fake",
            "sample",
        ] {
            assert!(
                !text.contains(forbidden),
                "forbidden test string leaked: {forbidden}"
            );
        }
    }
}
