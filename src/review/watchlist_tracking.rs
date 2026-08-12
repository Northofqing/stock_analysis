//! R-13 T+1 关注票核对 — 昨日题材复盘关注名单 → 次日盘后行情核对。
//!
//! 黑盒闭环: A-10 推送成功时名单落库 (catalyst_watchlist_daily), 次日盘后
//! R-13 按快照逐只核对行情 (涨停/一字/连板延续), 结果落
//! catalyst_watchlist_outcome 并推送回填, 让「昨天推的关注票」不再黑盒。
//!
//! 纯计算函数与网络/DB 薄壳分离 (backtest.rs 同款): 单测不依赖网络。

use chrono::NaiveDate;

use crate::data_gateway::historical_bars::HistoricalBarsGateway;
use crate::data_provider::limit_status::LimitStatusCalculator;
use crate::data_provider::KlineData;
use crate::database::catalyst_watchlist::{WatchEntry, WatchOutcome, WatchlistSnapshot};

/// 涨停价容差: limit_status.rs fill_limit_flags 同款 ±0.005 (容忍浮点误差)
const LIMIT_TOLERANCE: f64 = 0.005;
/// 「冲高回落」判定: 未涨停且收盘距最高价回落 ≥ 1%
const PULLBACK_PCT: f64 = 0.01;

/// 单只核对 (纯逻辑, 无 IO)。
/// `close/prev_close/open/high/low` 来自当日/昨日日线; 涨停价按
/// `LimitStatusCalculator::calculate(code, prev_close, name)` 精确计算
/// (主板 10% / 创业科创 20% / 北交 30% / ST 5%)。
pub fn check_entry(
    watch_date: NaiveDate,
    entry: &WatchEntry,
    today: NaiveDate,
    close: f64,
    prev_close: f64,
    open: f64,
    high: f64,
    low: f64,
) -> WatchOutcome {
    let calc = LimitStatusCalculator::new();
    let status = calc.calculate(&entry.code, prev_close, &entry.name);
    let is_limit_up =
        status.limit_up_price > 0.0 && (close - status.limit_up_price).abs() < LIMIT_TOLERANCE;
    let is_one_word = is_limit_up
        && (open - high).abs() < LIMIT_TOLERANCE
        && (high - low).abs() < LIMIT_TOLERANCE;
    let limit_up_type = if is_one_word {
        "一字"
    } else if is_limit_up {
        "封板"
    } else {
        ""
    };
    let streak_today = if is_limit_up { entry.streak + 1 } else { 0 };
    let change_pct = if prev_close > 0.0 {
        ((close - prev_close) / prev_close * 100.0 * 100.0).round() / 100.0
    } else {
        0.0
    };
    WatchOutcome {
        watch_date,
        checked_date: today,
        code: entry.code.clone(),
        name: entry.name.clone(),
        close,
        prev_close,
        change_pct,
        limit_up: is_limit_up,
        limit_up_type: limit_up_type.to_string(),
        streak_today,
        high: Some(high),
        open: Some(open),
    }
}

/// 从日线批次取「最新 + 前一根」。
/// 降序契约: `AdmittedDailyBars.records()` 经 validate_daily_kline_structure
/// 还原为降序 (最新在前, data_quality.rs 还原降序契约), 即 [0]=今日, [1]=昨收。
/// 防御: 若收到升序批次 (旧→新) 兼容识别并 warn 出声 (不静默), 取末尾两根。
pub fn latest_and_prev(records: &[KlineData]) -> Option<(&KlineData, &KlineData)> {
    if records.len() < 2 {
        return None;
    }
    let first = &records[0];
    let last = &records[records.len() - 1];
    if first.date < last.date {
        log::warn!(
            "[R-13] daily batch ascending (oldest first), treating last bar {} as latest",
            last.date
        );
        Some((last, &records[records.len() - 2]))
    } else {
        Some((first, &records[1]))
    }
}

/// 核对整份名单 (网络/DB 薄壳, 调用方放入 spawn_blocking)。
/// 每只拉 2 根日线: 最新 = 今日 (必须日期匹配且已 settled, 盘后 19:00 满足),
/// 前一根 close = 昨收。单只失败/数据不齐 → warn 出声跳过 (skipped);
/// 全部跳过 → Err (调用方按 failed 重试语义处理)。
pub fn check_watchlist_today(
    snapshot: &WatchlistSnapshot,
    today: NaiveDate,
) -> Result<(Vec<WatchOutcome>, Vec<String>), String> {
    let gateway = HistoricalBarsGateway::new();
    let mut outcomes = Vec::new();
    let mut skipped = Vec::new();
    for entry in snapshot.leading.iter().chain(snapshot.other.iter()) {
        match gateway.required_daily_bars(&entry.code, 2) {
            Ok(batch) => {
                let Some((latest, prev)) = latest_and_prev(batch.records()) else {
                    log::warn!(
                        "[R-13] {} {} fewer than 2 daily bars, skip",
                        entry.code,
                        entry.name
                    );
                    skipped.push(entry.code.clone());
                    continue;
                };
                if latest.date != today {
                    log::warn!(
                        "[R-13] {} {} latest bar {} != checked {}, skip",
                        entry.code,
                        entry.name,
                        latest.date,
                        today
                    );
                    skipped.push(entry.code.clone());
                    continue;
                }
                if !latest.settled {
                    log::warn!(
                        "[R-13] {} {} latest bar not settled yet, skip",
                        entry.code,
                        entry.name
                    );
                    skipped.push(entry.code.clone());
                    continue;
                }
                outcomes.push(check_entry(
                    snapshot.watch_date,
                    entry,
                    today,
                    latest.close,
                    prev.close,
                    latest.open,
                    latest.high,
                    latest.low,
                ));
            }
            Err(error) => {
                log::warn!(
                    "[R-13] {} {} daily bars unavailable: {error}, skip",
                    entry.code,
                    entry.name
                );
                skipped.push(entry.code.clone());
            }
        }
    }
    if outcomes.is_empty() {
        return Err(format!(
            "R-13 all {} watchlist entries skipped (data unavailable)",
            skipped.len()
        ));
    }
    Ok((outcomes, skipped))
}

/// 结论规则 (纯逻辑, 非 LLM): 由前排命中率 + 全体涨停数推导。
pub fn conclusion(leading: &[WatchOutcome], all: &[WatchOutcome]) -> String {
    let limit_up_count = all.iter().filter(|o| o.limit_up).count();
    let leading_limit = leading.iter().filter(|o| o.limit_up).count();
    if !leading.is_empty() {
        if leading_limit == leading.len() {
            format!("前排扩散 {leading_limit}/{} 兑现，题材情绪延续", leading.len())
        } else if leading_limit > 0 {
            format!("前排 {leading_limit}/{} 兑现，题材分歧加剧", leading.len())
        } else {
            "前排熄火，题材退潮，关注接力风险".to_string()
        }
    } else if limit_up_count >= 3 {
        "题材情绪延续".to_string()
    } else if limit_up_count >= 1 {
        "题材分歧加剧".to_string()
    } else {
        "题材退潮，关注接力风险".to_string()
    }
}

/// 渲染 R-13 推送文本 (纯逻辑)。outcomes 顺序 = 前排 → 其余 (与名单一致)。
pub fn render_watchlist_tracking(snapshot: &WatchlistSnapshot, outcomes: &[WatchOutcome]) -> String {
    let leading_len = snapshot.leading.len();
    let leading = &outcomes[..leading_len.min(outcomes.len())];
    let limit_up_count = outcomes.iter().filter(|o| o.limit_up).count();
    let avg_change = outcomes.iter().map(|o| o.change_pct).sum::<f64>() / outcomes.len() as f64;
    let checked_date = outcomes[0].checked_date.format("%m-%d");
    let mut text = format!(
        "📋 昨日关注回填（{} → {}）\n昨日关注 {} 只 | 涨停 {} | 平均 {:+.1}%\n",
        snapshot.watch_date.format("%m-%d"),
        checked_date,
        outcomes.len(),
        limit_up_count,
        avg_change
    );
    for o in outcomes {
        let desc = if o.limit_up {
            if o.limit_up_type == "一字" {
                if o.streak_today >= 2 {
                    format!("一字涨停 → {}连板", o.streak_today)
                } else {
                    "一字涨停 首板".to_string()
                }
            } else if o.streak_today >= 2 {
                format!("涨停封板 → {}连板", o.streak_today)
            } else {
                "涨停封板 首板".to_string()
            }
        } else if let Some(high) = o.high {
            if high > o.close && (high - o.close) / o.close >= PULLBACK_PCT {
                format!("未板（冲高回落 高{high:.2}）")
            } else {
                "未板".to_string()
            }
        } else {
            "未板".to_string()
        };
        text.push_str(&format!("· {} {} {:+.2}% {}\n", o.code, o.name, o.change_pct, desc));
    }
    text.push_str(&format!("结论: {}", conclusion(leading, outcomes)));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::catalyst_watchlist::WatchEntry;
    use crate::data_provider::KlineData;

    fn entry(code: &str, name: &str, streak: i64) -> WatchEntry {
        WatchEntry {
            code: code.to_string(),
            name: name.to_string(),
            streak,
        }
    }

    fn kline(date: &str, close: f64) -> KlineData {
        KlineData {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 100.0,
            amount: 1000.0,
            pct_chg: 0.0,
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
            adjust: crate::data_provider::AdjustType::None,
        }
    }

    #[test]
    fn latest_and_prev_descending_contract() {
        // 生产契约: 降序 (最新在前, [0]=今日 8/12, [1]=昨日 8/11)
        let bars = vec![kline("2026-08-12", 12.75), kline("2026-08-11", 11.59)];
        let (latest, prev) = latest_and_prev(&bars).expect("2 bars");
        assert_eq!(latest.date, NaiveDate::from_ymd_opt(2026, 8, 12).unwrap());
        assert_eq!(latest.close, 12.75);
        assert_eq!(prev.date, NaiveDate::from_ymd_opt(2026, 8, 11).unwrap());
        assert_eq!(prev.close, 11.59);
    }

    #[test]
    fn latest_and_prev_ascending_defensive() {
        // 防御: 升序批次 (旧→新) 取末尾两根, 不静默
        let bars = vec![kline("2026-08-11", 11.59), kline("2026-08-12", 12.75)];
        let (latest, prev) = latest_and_prev(&bars).expect("2 bars");
        assert_eq!(latest.date, NaiveDate::from_ymd_opt(2026, 8, 12).unwrap());
        assert_eq!(latest.close, 12.75);
        assert_eq!(prev.close, 11.59);
    }

    #[test]
    fn latest_and_prev_fewer_than_two() {
        assert!(latest_and_prev(&[]).is_none());
        assert!(latest_and_prev(&[kline("2026-08-12", 12.75)]).is_none());
    }

    #[test]
    fn check_entry_limit_up_sealed() {
        // 主板 10%: 昨收 11.59 → 涨停价 12.75 (round_to_cent(11.59*1.1))
        let e = entry("600721", "百花医药", 1);
        let o = check_entry(
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            &e,
            NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            12.75,
            11.59,
            12.20,
            12.80,
            12.10,
        );
        assert!(o.limit_up);
        assert_eq!(o.limit_up_type, "封板");
        assert_eq!(o.streak_today, 2); // 昨日 1 板 + 今日涨停
        assert!((o.change_pct - 10.01).abs() < 1e-9);
        assert_eq!(o.code, "600721");
        assert_eq!(o.checked_date, NaiveDate::from_ymd_opt(2026, 8, 12).unwrap());
    }

    #[test]
    fn check_entry_one_word_limit() {
        // 一字板: open == high == low == close == 涨停价
        let e = entry("603758", "秦安股份", 1);
        let o = check_entry(
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            &e,
            NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            12.98,
            11.80,
            12.98,
            12.98,
            12.98,
        );
        assert!(o.limit_up);
        assert_eq!(o.limit_up_type, "一字");
        assert_eq!(o.streak_today, 2);
    }

    #[test]
    fn check_entry_not_limit_breaks_streak() {
        let e = entry("600833", "第一医药", 1);
        let o = check_entry(
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            &e,
            NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            10.14,
            10.00,
            10.05,
            10.40,
            10.01,
        );
        assert!(!o.limit_up);
        assert_eq!(o.limit_up_type, "");
        assert_eq!(o.streak_today, 0); // 断板
        assert!((o.change_pct - 1.40).abs() < 1e-9);
    }

    #[test]
    fn check_entry_st_stock_five_percent() {
        // ST 5%: 昨收 10.00 → 涨停价 10.50
        let e = entry("600001", "ST百花", 0);
        let o = check_entry(
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            &e,
            NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            10.50,
            10.00,
            10.10,
            10.52,
            10.05,
        );
        assert!(o.limit_up);
        assert_eq!(o.limit_up_type, "封板");
        // 10.49 (未到 10.50) 不算涨停
        let not_up = check_entry(
            NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            &e,
            NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            10.49,
            10.00,
            10.10,
            10.52,
            10.05,
        );
        assert!(!not_up.limit_up);
    }

    fn outcome(
        code: &str,
        name: &str,
        change_pct: f64,
        limit_up: bool,
        limit_up_type: &str,
        streak_today: i64,
        high: Option<f64>,
    ) -> WatchOutcome {
        WatchOutcome {
            watch_date: NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            checked_date: NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            code: code.to_string(),
            name: name.to_string(),
            close: 10.0,
            prev_close: 10.0,
            change_pct,
            limit_up,
            limit_up_type: limit_up_type.to_string(),
            streak_today,
            high,
            open: None,
        }
    }

    #[test]
    fn conclusion_full_leading_hit_extends() {
        let leading = vec![
            outcome("600721", "百花医药", 9.96, true, "封板", 2, Some(14.03)),
            outcome("603758", "秦安股份", 10.02, true, "一字", 2, Some(12.98)),
        ];
        assert_eq!(conclusion(&leading, &leading), "前排扩散 2/2 兑现，题材情绪延续");
    }

    #[test]
    fn conclusion_partial_leading_divergence() {
        let leading = vec![
            outcome("600721", "百花医药", 9.96, true, "封板", 2, Some(14.03)),
            outcome("600833", "第一医药", 1.40, false, "", 0, Some(10.20)),
        ];
        assert_eq!(conclusion(&leading, &leading), "前排 1/2 兑现，题材分歧加剧");
    }

    #[test]
    fn conclusion_leading_all_dead() {
        let leading = vec![outcome("600833", "第一医药", -2.0, false, "", 0, Some(10.20))];
        assert_eq!(conclusion(&leading, &leading), "前排熄火，题材退潮，关注接力风险");
    }

    #[test]
    fn render_includes_all_sections() {
        let snapshot = WatchlistSnapshot {
            watch_date: NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            leading: vec![
                entry("600721", "百花医药", 1),
                entry("603758", "秦安股份", 1),
            ],
            other: vec![entry("600833", "第一医药", 1)],
        };
        let outcomes = vec![
            outcome("600721", "百花医药", 9.96, true, "封板", 2, Some(14.03)),
            outcome("603758", "秦安股份", 10.02, true, "一字", 2, Some(12.98)),
            outcome("600833", "第一医药", 1.40, false, "", 0, Some(10.40)),
        ];
        let text = render_watchlist_tracking(&snapshot, &outcomes);
        assert!(text.contains("📋 昨日关注回填（08-11 → 08-12）"));
        assert!(text.contains("昨日关注 3 只 | 涨停 2 | 平均 +7.1%"));
        assert!(text.contains("· 600721 百花医药 +9.96% 涨停封板 → 2连板"));
        assert!(text.contains("· 603758 秦安股份 +10.02% 一字涨停 → 2连板"));
        // 冲高回落: high 10.40 vs close 10.00 → 回落 4%
        assert!(text.contains("· 600833 第一医药 +1.40% 未板（冲高回落 高10.40）"));
        assert!(text.ends_with("结论: 前排扩散 2/2 兑现，题材情绪延续"));
    }

    #[test]
    fn render_no_pullback_label_without_gap() {
        let snapshot = WatchlistSnapshot {
            watch_date: NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            leading: vec![entry("600833", "第一医药", 1)],
            other: vec![],
        };
        let outcomes = vec![outcome(
            "600833",
            "第一医药",
            1.40,
            false,
            "",
            0,
            Some(10.05),
        )];
        let text = render_watchlist_tracking(&snapshot, &outcomes);
        assert!(text.contains("· 600833 第一医药 +1.40% 未板\n"));
    }
}
