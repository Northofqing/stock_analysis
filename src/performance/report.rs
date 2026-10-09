//! 归因报告渲染 — 全文 markdown + 推送摘要 (spec §4.4).

use super::attribution::{
    DailyAttribution, DailyAttributionDetails, FamilyAggregate, SignalFamily, WindowAttribution,
};

mod paper_account;
pub use paper_account::{
    native_daily_observation_is_permitted, render_effective_daily_details,
    render_native_daily_account_observation,
};

/// Append-only file presentation; the immutable database report is authoritative.
pub fn persist_report_revision(
    directory: &std::path::Path,
    date: chrono::NaiveDate,
    bytes: &[u8],
) -> Result<std::path::PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    std::fs::create_dir_all(directory).map_err(|e| format!("create report directory: {e}"))?;
    let digest = hex::encode(Sha256::digest(bytes));
    let path = directory.join(format!("{date}.{digest}.md"));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("append report revision: {e}"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&path).map_err(|e| format!("read existing report revision: {e}"))?
                != bytes
            {
                return Err("existing report revision bytes differ; no overwrite permitted".into());
            }
        }
        Err(error) => return Err(format!("create report revision: {error}")),
    }
    Ok(path)
}

#[cfg(test)]
#[test]
fn effective_fill_report_artifact_preserves_old_daily_bytes_and_reuses_revision() {
    let dir = tempfile::tempdir().unwrap();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let legacy = dir.path().join("2026-09-15.md");
    std::fs::write(&legacy, b"TEST_CODE_frozen_original").unwrap();
    let first = persist_report_revision(dir.path(), date, b"TEST_CODE_restated_A").unwrap();
    let repeat = persist_report_revision(dir.path(), date, b"TEST_CODE_restated_A").unwrap();
    let second = persist_report_revision(dir.path(), date, b"TEST_CODE_restated_B").unwrap();
    assert_eq!(first, repeat);
    assert_ne!(first, second);
    assert_eq!(std::fs::read(legacy).unwrap(), b"TEST_CODE_frozen_original");
    assert_eq!(std::fs::read(first).unwrap(), b"TEST_CODE_restated_A");
}

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

/// Separate realized activity from the end-of-day inventory valuation. A carried
/// position loss is cumulative, not today's mark-to-market change.
pub fn render_summary(daily: &DailyAttribution, window: &WindowAttribution) -> String {
    render_summary_with_optional_details(daily, window, None)
}

pub fn render_summary_with_details(
    daily: &DailyAttribution,
    window: &WindowAttribution,
    details: &DailyAttributionDetails,
) -> String {
    render_summary_with_optional_details(daily, window, Some(details))
}

fn render_summary_with_optional_details(
    daily: &DailyAttribution,
    window: &WindowAttribution,
    details: Option<&DailyAttributionDetails>,
) -> String {
    let date = daily.date.format("%Y-%m-%d");
    let daily_realized: f64 = daily.families.iter().map(|f| f.realized_pnl).sum();
    let end_unrealized: f64 = daily.families.iter().map(|f| f.unrealized_pnl).sum();
    let win_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
    let open_lots: i64 = daily.families.iter().map(|f| f.open_lots).sum();
    let sold_segments: i64 = daily.families.iter().map(|f| f.realized_trades).sum();
    let valuation_incomplete = daily
        .families
        .iter()
        .any(|f| f.suspicious_lots > 0 || f.unvalued_lots > 0);
    let mut lines = vec![
        format!("📊 虚拟盘归因 {date}"),
        "━━━━━━━━━━━━━━━━━━━━".into(),
        if valuation_incomplete {
            "重点：数据存疑或缺估值，可信持仓合计不可用；以下价差仅作参考。".into()
        } else if sold_segments == 0 {
            format!("重点：今日无已平仓交易；期末仍持有 {open_lots} 个批次。")
        } else {
            format!(
                "重点：今日已实现 {} 元；期末持仓浮盈 {} 元。",
                fmt_signed_money(daily_realized),
                fmt_signed_money(end_unrealized)
            )
        },
        format!("【今日已实现】{} 元", fmt_signed_money(daily_realized)),
        format!(
            "【期末累计浮盈】{} 元（不是今日涨跌）",
            fmt_signed_money(end_unrealized)
        ),
        format!(
            "【{}天】已实现 {} 元",
            window.days,
            fmt_signed_money(win_realized)
        ),
        "口径：旧策略账本价差；实扣费用未核验，与新持仓起点账户分开。".into(),
        "━━━━━━━━━━━━━━━━━━━━".into(),
    ];
    if let Some(details) = details {
        lines.push(format!(
            "今日成交：买入 {} 笔/{} 股；卖出 {} 笔/{} 股",
            details.buy_fills, details.buy_quantity, details.sell_fills, details.sell_quantity
        ));
        lines.push(format!(
            "持仓明细（{} 个股票/策略组合）：",
            details.holdings.len()
        ));
        for holding in details.holdings.iter().take(6) {
            let value = holding
                .market_value
                .map(fmt_money)
                .unwrap_or_else(|| "不可用".into());
            let pnl = holding
                .unrealized_pnl
                .map(fmt_signed_money)
                .unwrap_or_else(|| "不可用".into());
            let price = holding
                .close_price
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "缺收盘价".into());
            lines.push(format!(
                "• {} {} 股｜成本 {:.3}/收盘 {}",
                holding.code,
                holding.quantity,
                holding.cost_notional / holding.quantity as f64,
                price
            ));
            lines.push(format!(
                "  市值 {value} 元｜期末浮盈 {pnl} 元｜{}{}",
                holding.family.as_str(),
                if holding.suspicious {
                    "｜数据存疑"
                } else {
                    ""
                }
            ));
        }
        if details.holdings.len() > 6 {
            lines.push(format!(
                "其余 {} 项见完整日报。",
                details.holdings.len() - 6
            ));
        }
        if details.holdings.is_empty() {
            lines.push("当前无未平仓持仓。".into());
        }
    } else if open_lots > 0 {
        lines.push("持仓逐股明细不可用；不能用空交易行代替持仓。".into());
    }
    let mut families: Vec<&FamilyAggregate> = daily.families.iter().collect();
    families.sort_by(|a, b| {
        b.total_pnl
            .abs()
            .partial_cmp(&a.total_pnl.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for f in families {
        if f.open_lots == 0 && f.realized_trades == 0 {
            continue;
        }
        let win = f
            .win_rate
            .map(|w| format!("｜卖出匹配段胜率 {:.0}%", w * 100.0))
            .unwrap_or_default();
        lines.push(format!(
            "{}：持仓 {} 批｜卖出匹配 {} 段",
            family_label(f.family),
            f.open_lots,
            f.realized_trades
        ));
        lines.push(format!(
            "  已实现 {}｜期末浮盈 {}{}",
            fmt_signed_money(f.realized_pnl),
            fmt_signed_money(f.unrealized_pnl),
            win
        ));
    }
    for trade in daily.top_trades.iter().take(3) {
        lines.push(format!(
            "平仓重点：{} {} 元｜{}",
            trade.code,
            fmt_signed_money(trade.pnl),
            family_label(trade.entry_family)
        ));
    }
    let suspicious: i64 = daily.families.iter().map(|f| f.suspicious_lots).sum();
    let suspicious_pnl: f64 = daily.families.iter().map(|f| f.suspicious_pnl).sum();
    let unvalued: i64 = daily.families.iter().map(|f| f.unvalued_lots).sum();
    let unknown: i64 = daily
        .families
        .iter()
        .filter(|f| f.family == SignalFamily::Unknown)
        .map(|f| f.open_lots + f.realized_trades)
        .sum();
    if suspicious > 0 {
        lines.push(format!(
            "⚠ 数据存疑 {suspicious}笔 ({})；可信合计不可用，以上为未核验参考。",
            fmt_signed_money(suspicious_pnl)
        ));
    }
    if unvalued > 0 {
        lines.push(format!("⚠ 未估值 {unvalued} lot；完整持仓合计不可用。"));
    }
    if unknown > 0 {
        lines.push(format!("⚠ Unknown {unknown}；不能据此判断策略胜负。"));
    }
    lines.push("下一步：核对持仓估值与成交依据；本报告不签发买卖指令。".into());
    lines.join("\n")
}

fn family_label(family: SignalFamily) -> &'static str {
    match family {
        SignalFamily::PostCloseFundInflow => "盘后资金流入",
        SignalFamily::ExitByRule => "ExitByRule(卖)",
        other => other.as_str(),
    }
}

/// 全文 markdown (spec §4.4 五节)
pub fn render_full_markdown(daily: &DailyAttribution, window: &WindowAttribution) -> String {
    let date = daily.date.format("%Y-%m-%d");
    let mut out = vec![format!("# 虚拟盘归因 {date}"), String::new()];
    out.push("口径：当日卖出价差与期末累计浮盈分列。期末浮盈不是今日损益；实扣手续费未核验。旧策略账本与以实际持仓为起点的新账户分开。".into());
    out.push(String::new());
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
        out.push(
            "- 当前审计未发现已标记的可疑 lot 或缺价；不代表成交价格、实扣费用与收益已全面核验。"
                .into(),
        );
    }
    out.push(String::new());
    out.push("## 今日归因".to_string());
    out.push("| 信号族 | 今日已实现 | 期末累计浮盈 | 已实现加期末浮盈（非今日损益） | 卖出匹配段 | 匹配段胜率 |".to_string());
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
    out.push(format!("## {} 天滚动窗口", window.days));
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

/// Same frozen daily inputs; no database read or accounting recalculation occurs
/// while producing the detailed artifact.
pub fn render_full_markdown_with_details(
    daily: &DailyAttribution,
    window: &WindowAttribution,
    details: &DailyAttributionDetails,
) -> String {
    let mut markdown = render_full_markdown(daily, window);
    markdown.push_str("\n## 今日成交与当前持仓明细\n");
    markdown.push_str(&format!("买入 {} 笔/{} 股；卖出 {} 笔/{} 股。匹配段胜率按卖出消耗的入场 lot 统计，不等于完整策略周期胜率。\n\n", details.buy_fills, details.buy_quantity, details.sell_fills, details.sell_quantity));
    markdown.push_str("| 代码 | 入场族 | 数量 | 剩余成本均价 | 收盘价 | 期末市值 | 期末累计浮盈 | 质量 |\n|---|---|---:|---:|---:|---:|---:|---|\n");
    for h in &details.holdings {
        markdown.push_str(&format!(
            "| {} | {} | {} | {:.3} | {} | {} | {} | {} |\n",
            h.code,
            h.family.as_str(),
            h.quantity,
            h.cost_notional / h.quantity as f64,
            h.close_price
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "不可用".into()),
            h.market_value
                .map(fmt_money)
                .unwrap_or_else(|| "不可用".into()),
            h.unrealized_pnl
                .map(fmt_signed_money)
                .unwrap_or_else(|| "不可用".into()),
            if h.suspicious {
                "存疑，仅作参考"
            } else if h.close_price.is_none() {
                "缺收盘价"
            } else {
                "沿用本次归因估值输入"
            }
        ));
    }
    if details.holdings.is_empty() {
        markdown.push_str("当前无未平仓持仓。\n");
    }
    markdown
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
    fn window_label_reports_actual_clamped_day_count() {
        // BR-255: epoch 生效首月窗口被截断到 effective, WindowAttribution.days
        // 同步为真实跨度 (compute_epoch_window 注释: "首月报告天数诚实")。
        let mut clamped = window();
        clamped.days = 8;
        let text = render_summary(&daily(), &clamped);
        assert!(
            text.contains("【8天】"),
            "推送卡窗口标签必须反映实际天数, actual={text}"
        );
        assert!(!text.contains("【30天】"), "actual={text}");

        let md = render_full_markdown(&daily(), &clamped);
        assert!(
            md.contains("## 8 天滚动窗口"),
            "Markdown 标题必须反映实际天数, actual={md}"
        );
        assert!(!md.contains("30 天滚动窗口"), "actual={md}");
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
    fn carried_position_report() -> (DailyAttribution, WindowAttribution, DailyAttributionDetails) {
        use crate::performance::attribution::{
            aggregate_families, daily_attribution_details, fifo_match, AttributionFillRow,
        };
        let date = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let rows = vec![AttributionFillRow {
            id: 1,
            code: "TEST_CODE_600000".into(),
            direction: "buy".into(),
            fill_price: Some(10.0),
            quantity: 1000,
            local_ts: "2026-10-08 10:00:00".into(),
            plan_id: "TEST_CODE_news_entry".into(),
            virtual_reason: "NewsCatalyst".into(),
        }];
        let prices = std::collections::HashMap::from([("TEST_CODE_600000".into(), 4.3)]);
        let (trades, open) = fifo_match(&rows, date).unwrap();
        let families = aggregate_families(&trades, &open, &prices);
        (
            DailyAttribution {
                date,
                families: families.clone(),
                top_trades: vec![],
            },
            WindowAttribution {
                days: 30,
                end: date,
                families,
            },
            daily_attribution_details(date, &rows, &open, &prices).unwrap(),
        )
    }
    #[test]
    fn carried_loss_is_presented_as_end_valuation_not_todays_loss_or_empty_strategy() {
        let (daily, window, details) = carried_position_report();
        let text = render_summary_with_details(&daily, &window, &details);
        assert!(text.contains("今日无已平仓交易"));
        assert!(text.contains("【今日已实现】+0"));
        assert!(text.contains("【期末累计浮盈】-5,700"));
        assert!(text.contains("TEST_CODE_600000 1000 股｜成本 10.000/收盘 4.300"));
        assert!(text.contains("市值 4,300 元｜期末浮盈 -5,700"));
        assert!(!text.contains("【今日】合计 -5,700"));
        assert!(!text.contains("0笔 0"));
        let markdown = render_full_markdown_with_details(&daily, &window, &details);
        assert!(markdown.contains(
            "| TEST_CODE_600000 | NewsCatalyst | 1000 | 10.000 | 4.300 | 4,300 | -5,700 |"
        ));
        assert!(text.contains("实扣费用未核验"));
    }
    #[test]
    fn missing_or_disputed_valuation_is_unavailable_not_a_trusted_zero() {
        let (mut daily, window, mut details) = carried_position_report();
        details.holdings[0].market_value = None;
        details.holdings[0].close_price = None;
        details.holdings[0].unrealized_pnl = None;
        daily.families[0].unvalued_lots = 1;
        daily.families[0].suspicious_lots = 1;
        let text = render_summary_with_details(&daily, &window, &details);
        assert!(text.contains("市值 不可用 元｜期末浮盈 不可用 元"));
        assert!(text.contains("可信合计不可用"));
        assert!(text.contains("完整持仓合计不可用"));
        assert!(text.contains("以上为未核验参考"));
    }
}
