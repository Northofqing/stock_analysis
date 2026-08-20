//! 2026-08-20 Attribution Research Loop — 交付物 A 核心模块.
//!
//! 设计: docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §4.
//! 数据来源: paper_trades (plan_id + virtual_reason), 证据 E3-E7.
//! 归因口径: 已实现 (FIFO 带 lot 归属) + 未实现浮盈 (未平仓 lot × 收盘价).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 入场信号族 (归因维度). spec §4.1.
/// Ord 派生供 Task 3 的 BTreeMap 聚合排序使用.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SignalFamily {
    NewsCatalyst,
    VolumeSurge,
    MainNetInflow,
    Breakout,
    PostCloseFundInflow,
    ExitByRule,
    Unknown,
}

impl SignalFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalFamily::NewsCatalyst => "NewsCatalyst",
            SignalFamily::VolumeSurge => "VolumeSurge",
            SignalFamily::MainNetInflow => "MainNetInflow",
            SignalFamily::Breakout => "Breakout",
            SignalFamily::PostCloseFundInflow => "PostCloseFundInflow",
            SignalFamily::ExitByRule => "ExitByRule",
            SignalFamily::Unknown => "Unknown",
        }
    }
}

/// virtual_reason → 信号族. 规则表见 spec §4.1; 未命中 → Unknown (报告明示, 不静默).
pub fn signal_family_of(reason: &str) -> SignalFamily {
    let r = reason.trim();
    if r.starts_with("NewsCatalyst") {
        return SignalFamily::NewsCatalyst;
    }
    if r.starts_with("VolumeSurge") {
        return SignalFamily::VolumeSurge;
    }
    if r.starts_with("MainNetInflow") {
        return SignalFamily::MainNetInflow;
    }
    if r.starts_with("Breakout") {
        return SignalFamily::Breakout;
    }
    if r.starts_with("盘后资金净流入") || r.contains("收盘价买入") {
        return SignalFamily::PostCloseFundInflow;
    }
    if r.starts_with("BR-") {
        return SignalFamily::ExitByRule;
    }
    SignalFamily::Unknown
}

/// 提取 `涨幅+X.X%` 数值; 无 → None.
pub fn parse_change_pct(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("涨幅")?;
    let value = rest.split('%').next()?.trim();
    value.parse::<f64>().ok()
}

/// 提取 `量比X.X` 数值; 无 → None.
pub fn parse_volume_ratio(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("量比")?;
    let value: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    value.parse::<f64>().ok()
}

/// 可疑数据: |涨幅| > 25 或 量比 ≤ 0 (spec §4.1; 证据 E6: 涨幅+858.9% ×27、量比0.0).
/// 可疑 lot 仍计入所属族 PnL, 由报告「数据质量」节单独标注 — 不删除, 不静默.
pub fn is_suspicious_reason(reason: &str) -> bool {
    if let Some(pct) = parse_change_pct(reason) {
        if pct.abs() > 25.0 {
            return true;
        }
    }
    if let Some(ratio) = parse_volume_ratio(reason) {
        if ratio <= 0.0 {
            return true;
        }
    }
    false
}

#[derive(diesel::QueryableByName, Debug)]
pub struct AttributionFillRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub local_ts: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub virtual_reason: String,
}

/// 已实现交易归因 — 每笔卖出按匹配到的入场 lot 拆分归属.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeAttribution {
    pub sell_id: i64,
    pub code: String,
    pub pnl: f64,
    pub entry_plan_id: String,
    pub entry_family: SignalFamily,
    pub exit_reason: String,
    pub suspicious: bool,
    /// 卖出发生日期 (Task 3 compute_window 按此过滤窗口).
    pub sell_date: NaiveDate,
}

/// 未平仓 lot (FIFO 匹配剩余).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenLot {
    pub code: String,
    pub plan_id: String,
    pub family: SignalFamily,
    pub suspicious: bool,
    pub remaining_qty: i64,
    pub cost_price: f64,
}

/// FIFO 匹配: 语义与 performance/snapshot.rs::realized_pnls_for_date 逐条对齐
/// (id>0, code 非空, price>0 finite, qty>0 且 %100==0, 时间序校验, oversell 拒绝,
/// 非 finite PnL 拒绝), 区别: 匹配时携带入场 lot 的 plan_id/family/suspicious 归属.
/// 跨 lot 匹配时 PnL 按数量比例拆分 (每段生成一条 TradeAttribution).
/// 返回 (当日已实现归因列表, 未平仓 lot 列表).
pub fn fifo_match(
    rows: &[AttributionFillRow],
    target_date: NaiveDate,
) -> Result<(Vec<TradeAttribution>, Vec<OpenLot>), String> {
    use std::collections::{HashMap, VecDeque};

    #[derive(Clone)]
    struct Lot {
        remaining: u32,
        price: f64,
        plan_id: String,
        family: SignalFamily,
        suspicious: bool,
    }

    let mut lots: HashMap<String, VecDeque<Lot>> = HashMap::new();
    let mut realized = Vec::new();
    let mut previous_order: Option<(chrono::NaiveDateTime, i64)> = None;

    for row in rows {
        if row.id <= 0 || row.code.trim().is_empty() {
            return Err(format!(
                "attribution fill identity invalid: id={} code={:?}",
                row.id, row.code
            ));
        }
        let timestamp =
            chrono::NaiveDateTime::parse_from_str(&row.local_ts, "%Y-%m-%d %H:%M:%S")
                .map_err(|error| format!("attribution fill id={} timestamp invalid: {error}", row.id))?;
        if timestamp.date() > target_date {
            return Err(format!(
                "attribution fill id={} is later than settlement date {}",
                row.id, target_date
            ));
        }
        if previous_order.is_some_and(|previous| previous > (timestamp, row.id)) {
            return Err(format!("attribution fills are not ordered at id={}", row.id));
        }
        previous_order = Some((timestamp, row.id));
        let price = row
            .fill_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| format!("attribution fill id={} fill_price missing/invalid", row.id))?;
        let quantity = u32::try_from(row.quantity)
            .ok()
            .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "attribution fill id={} quantity invalid: {}",
                    row.id, row.quantity
                )
            })?;
        let family = signal_family_of(&row.virtual_reason);
        let suspicious = is_suspicious_reason(&row.virtual_reason);

        match row.direction.as_str() {
            "buy" => lots.entry(row.code.clone()).or_default().push_back(Lot {
                remaining: quantity,
                price,
                plan_id: row.plan_id.clone(),
                family,
                suspicious,
            }),
            "sell" => {
                let queue = lots
                    .get_mut(&row.code)
                    .ok_or_else(|| format!("attribution sell id={} has no matched buy lots", row.id))?;
                let mut remaining = quantity;
                while remaining > 0 {
                    let lot = queue.front_mut().ok_or_else(|| {
                        format!(
                            "attribution sell id={} quantity {} exceeds matched buys",
                            row.id, quantity
                        )
                    })?;
                    let matched = remaining.min(lot.remaining);
                    let portion_pnl = (price - lot.price) * f64::from(matched);
                    if timestamp.date() == target_date {
                        realized.push(TradeAttribution {
                            sell_id: row.id,
                            code: row.code.clone(),
                            pnl: portion_pnl,
                            entry_plan_id: lot.plan_id.clone(),
                            entry_family: lot.family,
                            exit_reason: row.virtual_reason.clone(),
                            suspicious: lot.suspicious,
                            sell_date: timestamp.date(),
                        });
                    }
                    remaining -= matched;
                    lot.remaining -= matched;
                    if lot.remaining == 0 {
                        queue.pop_front(); // 与 snapshot.rs 同构: 已完成 lot 出队
                    }
                }
            }
            other => {
                return Err(format!(
                    "attribution fill id={} direction invalid: {other}",
                    row.id
                ));
            }
        }
    }
    // 非 finite 校验: 全部已实现 PnL 必须 finite (与 snapshot.rs 一致)
    for attribution in &realized {
        if !attribution.pnl.is_finite() {
            return Err(format!("attribution sell id={} PnL is non-finite", attribution.sell_id));
        }
    }
    let open = lots
        .into_iter()
        .flat_map(|(code, queue)| {
            queue.into_iter().map(move |lot| OpenLot {
                code: code.clone(),
                plan_id: lot.plan_id,
                family: lot.family,
                suspicious: lot.suspicious,
                remaining_qty: i64::from(lot.remaining),
                cost_price: lot.price,
            })
        })
        .collect();
    Ok((realized, open))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_from_reason_prefixes() {
        assert_eq!(signal_family_of("NewsCatalyst"), SignalFamily::NewsCatalyst);
        assert_eq!(signal_family_of("VolumeSurge"), SignalFamily::VolumeSurge);
        assert_eq!(signal_family_of("MainNetInflow"), SignalFamily::MainNetInflow);
        assert_eq!(signal_family_of("Breakout"), SignalFamily::Breakout);
        assert_eq!(signal_family_of("BR-234四大铁律卖出:结构止损（破中期趋势）"), SignalFamily::ExitByRule);
        assert_eq!(signal_family_of("盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%"), SignalFamily::PostCloseFundInflow);
        assert_eq!(signal_family_of("均线策略 收盘价买入 量比1.2 涨幅+3%"), SignalFamily::PostCloseFundInflow);
        assert_eq!(signal_family_of("未知原因"), SignalFamily::Unknown);
    }

    #[test]
    fn suspicious_rules_capture_garbage_but_keep_sane() {
        assert!(is_suspicious_reason("盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%"));
        assert!(is_suspicious_reason("... 涨幅+999.0%"));
        assert!(!is_suspicious_reason("... 涨幅+10.0% 量比1.5"));
        assert!(!is_suspicious_reason("NewsCatalyst"));
    }

    #[test]
    fn parse_helpers_extract_structured_fields() {
        let reason = "盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%";
        assert_eq!(parse_change_pct(reason), Some(-2.9));
        assert_eq!(parse_volume_ratio(reason), Some(1.5));
        assert_eq!(parse_change_pct("NewsCatalyst"), None);
        assert_eq!(parse_volume_ratio("NewsCatalyst"), None);
    }

    #[test]
    fn family_names_are_stable_snake_case() {
        assert_eq!(SignalFamily::PostCloseFundInflow.as_str(), "PostCloseFundInflow");
        assert_eq!(SignalFamily::ExitByRule.as_str(), "ExitByRule");
    }

    fn fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        local_ts: &str,
        plan_id: &str,
        virtual_reason: &str,
    ) -> AttributionFillRow {
        AttributionFillRow {
            id,
            code: code.to_string(),
            direction: direction.to_string(),
            fill_price: Some(price),
            quantity,
            local_ts: local_ts.to_string(),
            plan_id: plan_id.to_string(),
            virtual_reason: virtual_reason.to_string(),
        }
    }

    #[test]
    fn fifo_carries_lot_attribution() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "news-1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "buy", 12.0, 200, "2026-07-18 09:31:00", "fund-2", "MainNetInflow"),
            fill(3, "TEST_CODE_600000", "sell", 15.0, 200, "2026-07-18 14:00:00", "sell-3", "BR-234四大铁律卖出:结构止损"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");

        // 200 股卖出: 100 股归 NewsCatalyst lot (10.0→15.0 = +500), 100 股归 MainNetInflow lot (12.0→15.0 = +300)
        assert_eq!(attributions.len(), 2);
        let news: Vec<_> = attributions.iter().filter(|a| a.entry_family == SignalFamily::NewsCatalyst).collect();
        let fund: Vec<_> = attributions.iter().filter(|a| a.entry_family == SignalFamily::MainNetInflow).collect();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].pnl, 500.0);
        assert_eq!(news[0].entry_plan_id, "news-1");
        assert_eq!(fund.len(), 1);
        assert_eq!(fund[0].pnl, 300.0);
        assert_eq!(fund[0].entry_plan_id, "fund-2");
        assert_eq!(attributions.iter().map(|a| a.pnl).sum::<f64>(), 800.0); // 与 snapshot.rs 已知结果一致
        assert_eq!(open.len(), 1); // MainNetInflow lot 剩 100 股
        assert_eq!(open[0].remaining_qty, 100);
        assert_eq!(open[0].cost_price, 12.0);
    }

    #[test]
    fn fifo_rejects_oversell_and_invalid_rows() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let oversell = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 200, "2026-07-18 14:00:00", "s1", "BR-234四大铁律卖出"),
        ];
        let err = fifo_match(&oversell, target).expect_err("oversell must fail");
        assert!(err.contains("exceeds matched buys"));

        let mut missing_price = fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst");
        missing_price.fill_price = None;
        let err = fifo_match(&[missing_price], target).expect_err("missing price must fail");
        assert!(err.contains("fill_price missing/invalid"));
    }

    #[test]
    fn fifo_only_emits_target_date_sells() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 200, "2026-07-16 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 100, "2026-07-17 14:00:00", "s1", "BR-234四大铁律卖出"),
            fill(3, "TEST_CODE_600000", "sell", 12.0, 100, "2026-07-18 14:00:00", "s2", "BR-234四大铁律卖出"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        assert_eq!(attributions.len(), 1); // 只归当日卖出
        assert_eq!(attributions[0].pnl, 200.0);
        assert_eq!(open.len(), 0);
    }
}
