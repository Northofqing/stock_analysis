//! 2026-08-20 Attribution Research Loop — 交付物 A 核心模块.
//!
//! 设计: docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §4.
//! 数据来源: paper_trades (plan_id + virtual_reason), 证据 E3-E7.
//! 归因口径: 已实现 (FIFO 带 lot 归属) + 未实现浮盈 (未平仓 lot × 收盘价).

use chrono::NaiveDate;
use diesel::RunQueryDsl;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// 卖出发生日期 (窗口归因以 emit_from 谓词按此判断是否入窗).
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

/// FIFO 匹配 (当日语义): 语义与 performance/snapshot.rs::realized_pnls_for_date 逐条对齐
/// (id>0, code 非空, price>0 finite, qty>0 且 %100==0, 时间序校验, oversell 拒绝,
/// 非 finite PnL 拒绝), 区别: 匹配时携带入场 lot 的 plan_id/family/suspicious 归属.
/// 跨 lot 匹配时 PnL 按数量比例拆分 (每段生成一条 TradeAttribution).
/// 发射谓词 = 仅当日卖出 (fifo_match_from 的 emit_from=None 特例, compute_daily 语义).
/// 返回 (当日已实现归因列表, 未平仓 lot 列表).
pub fn fifo_match(
    rows: &[AttributionFillRow],
    target_date: NaiveDate,
) -> Result<(Vec<TradeAttribution>, Vec<OpenLot>), String> {
    fifo_match_from(rows, target_date, None)
}

/// FIFO 匹配核心 (发射谓词参数化, CRIT-1 修复):
/// - `emit_from = None`    → 仅发射 `timestamp.date() == target_date` 的卖出
///   (与旧 fifo_match 行为逐字节一致; fifo_match 2-arg wrapper 保持公开 API 稳定,
///   compute_daily 与既有日级测试不受影响).
/// - `emit_from = Some(d)` → 发射 `timestamp.date() >= d` 的全部卖出 (compute_window
///   30 天窗口语义; FIFO 匹配仍对全部 rows 执行 — 窗口前买入照常被窗口卖出消耗).
/// 校验 (身份/时间戳/越界/无序/oversell 等) 与 emit_from 无关, 全部 rows 一视同仁.
pub fn fifo_match_from(
    rows: &[AttributionFillRow],
    target_date: NaiveDate,
    emit_from: Option<NaiveDate>,
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
                    let date = timestamp.date();
                    let emit = match emit_from {
                        None => date == target_date,
                        Some(from) => date >= from,
                    };
                    if emit {
                        realized.push(TradeAttribution {
                            sell_id: row.id,
                            code: row.code.clone(),
                            pnl: portion_pnl,
                            entry_plan_id: lot.plan_id.clone(),
                            entry_family: lot.family,
                            exit_reason: row.virtual_reason.clone(),
                            suspicious: lot.suspicious,
                            sell_date: date,
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

/// 单族聚合 (spec §4.2).
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyAggregate {
    pub family: SignalFamily,
    pub realized_trades: i64,
    pub realized_pnl: f64,
    pub open_lots: i64,
    pub unrealized_pnl: f64,
    pub total_pnl: f64,
    pub wins: i64,
    pub losses: i64,
    pub win_rate: Option<f64>,
    pub unvalued_lots: i64,
    pub suspicious_lots: i64,
    /// 可疑 lot 已实现影响金额 (spec §4.4.2; realized-only — 未平仓可疑 lot 只计入
    /// suspicious_lots, 金额待其卖出实现后归入).
    pub suspicious_pnl: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyAttribution {
    pub date: NaiveDate,
    pub families: Vec<FamilyAggregate>,
    /// Top 盈亏交易明细 (当日, spec §4.4 item 5): 盈利 (pnl>0) ≤5 在前, 亏损 (pnl<0) ≤5 在后.
    pub top_trades: Vec<TradeAttribution>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowAttribution {
    pub days: u32,
    pub end: NaiveDate,
    pub families: Vec<FamilyAggregate>,
}

/// 聚合: 已实现 (卖出归因) + 未实现浮盈 (未平仓 lot × close).
/// 缺失 close → unvalued_lots 计数, 浮盈记 0 (不静默: 计数与报告明示).
/// suspicious_pnl 仅计已实现 (可疑卖出归因的 pnl 合计; 未平仓可疑 lot 只计
/// suspicious_lots 计数, 金额待卖出后归入).
pub fn aggregate_families(
    attributions: &[TradeAttribution],
    open: &[OpenLot],
    prices: &HashMap<String, f64>,
) -> Vec<FamilyAggregate> {
    use std::collections::BTreeMap;
    // 注意: rustc 1.95 拒绝「闭包返回指向捕获变量的引用」(captured variable cannot
    // escape FnMut closure body), 故用嵌套 fn 而非闭包实现 entry 复用.
    fn ensure<'a>(
        map: &'a mut BTreeMap<SignalFamily, FamilyAggregate>,
        family: SignalFamily,
    ) -> &'a mut FamilyAggregate {
        map.entry(family).or_insert_with(|| FamilyAggregate {
            family,
            realized_trades: 0,
            realized_pnl: 0.0,
            open_lots: 0,
            unrealized_pnl: 0.0,
            total_pnl: 0.0,
            wins: 0,
            losses: 0,
            win_rate: None,
            unvalued_lots: 0,
            suspicious_lots: 0,
            suspicious_pnl: 0.0,
        })
    }
    let mut map: BTreeMap<SignalFamily, FamilyAggregate> = BTreeMap::new();
    for a in attributions {
        let row = ensure(&mut map, a.entry_family);
        row.realized_trades += 1;
        row.realized_pnl += a.pnl;
        if a.pnl > 0.0 {
            row.wins += 1;
        } else {
            row.losses += 1;
        }
        if a.suspicious {
            row.suspicious_lots += 1;
            row.suspicious_pnl += a.pnl; // realized-only 影响金额 (spec §4.4.2)
        }
    }
    for lot in open {
        let row = ensure(&mut map, lot.family);
        row.open_lots += 1;
        if lot.suspicious {
            row.suspicious_lots += 1;
        }
        match prices.get(&lot.code).copied().filter(|p| p.is_finite() && *p > 0.0) {
            Some(close) => row.unrealized_pnl += (close - lot.cost_price) * lot.remaining_qty as f64,
            None => row.unvalued_lots += 1,
        }
    }
    let mut families: Vec<FamilyAggregate> = map.into_values().collect();
    for row in &mut families {
        row.total_pnl = row.realized_pnl + row.unrealized_pnl;
        row.win_rate = (row.realized_trades > 0)
            .then_some(row.wins as f64 / row.realized_trades as f64);
    }
    families.sort_by_key(|f| f.family);
    families
}

const FILLS_UNTIL_SQL: &str = "SELECT id, code, direction, fill_price, quantity, \
     datetime(ts, 'localtime') AS local_ts, plan_id, virtual_reason \
     FROM paper_trades \
     WHERE datetime(ts, 'localtime') < datetime(?, '+1 day') AND status = 'Filled' \
     ORDER BY datetime(ts, 'localtime') ASC, id ASC";

/// 查询截至日期 (含) 的全部 Filled 成交 (与 snapshot.rs 查询同构, 多带 plan_id/virtual_reason).
pub fn query_fills_until(date: NaiveDate) -> Result<Vec<AttributionFillRow>, String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {e}"))?;
    let date_str = date.format("%Y-%m-%d").to_string();
    diesel::sql_query(FILLS_UNTIL_SQL)
        .bind::<diesel::sql_types::Text, _>(&date_str)
        .load::<AttributionFillRow>(&mut conn)
        .map_err(|e| format!("query paper_trades attribution: {e}"))
}

/// Top 盈亏交易明细 (spec §4.4 item 5, 当日): 盈利 (pnl>0) 按 pnl 降序 ≤5 在前,
/// 亏损 (pnl<0) 按 pnl 升序 (最负在前) ≤5 在后; pnl==0 不入列.
fn top_trades(attributions: &[TradeAttribution]) -> Vec<TradeAttribution> {
    let mut winners: Vec<&TradeAttribution> = attributions.iter().filter(|a| a.pnl > 0.0).collect();
    winners.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));
    let mut losers: Vec<&TradeAttribution> = attributions.iter().filter(|a| a.pnl < 0.0).collect();
    losers.sort_by(|a, b| a.pnl.partial_cmp(&b.pnl).unwrap_or(std::cmp::Ordering::Equal));
    winners.truncate(5);
    losers.truncate(5);
    winners.into_iter().chain(losers).cloned().collect()
}

/// 当日归因: 已实现 (当日卖出 FIFO 全局匹配) + 浮盈 (截至当日未平仓 × close).
pub fn compute_daily(
    date: NaiveDate,
    prices: &HashMap<String, f64>,
) -> Result<DailyAttribution, String> {
    let rows = query_fills_until(date)?;
    let (attributions, open) = fifo_match(&rows, date)?;
    let top_trades = top_trades(&attributions);
    let families = aggregate_families(&attributions, &open, prices);
    Ok(DailyAttribution { date, families, top_trades })
}

/// 30 天滚动窗口 (spec §4.5): 已实现 = 窗口内每日卖出 FIFO 全局匹配 (对历史全部 lot),
/// 浮盈 = 期末未平仓 × close. 窗口 = end−(days−1) ..= end 共 days 个自然日.
pub fn compute_window(
    end: NaiveDate,
    days: u32,
    prices: &HashMap<String, f64>,
) -> Result<WindowAttribution, String> {
    let rows = query_fills_until(end)?;
    aggregate_window(end, days, &rows, prices)
}

/// 窗口聚合纯函数 (不触 DB, 供单测直测): start = end − (days−1) 天 (含首尾共 days 个
/// 自然日); FIFO 匹配跑全部 rows (窗口前买入照常被窗口卖出消耗), 发射谓词 =
/// `timestamp.date() >= start` (CRIT-1: 已实现必须为窗口累计, 非单日).
pub fn aggregate_window(
    end: NaiveDate,
    days: u32,
    rows: &[AttributionFillRow],
    prices: &HashMap<String, f64>,
) -> Result<WindowAttribution, String> {
    let start = end
        .checked_sub_signed(chrono::Duration::days(i64::from(days) - 1))
        .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch"));
    let (attributions, open) = fifo_match_from(rows, end, Some(start))?;
    let families = aggregate_families(&attributions, &open, prices);
    Ok(WindowAttribution { days, end, families })
}

/// 建表 DDL (spec §4.3). const 供单测文本断言 (Step 1 测试依赖此 const).
const DDL_SQL: &str = "CREATE TABLE IF NOT EXISTS paper_attribution_daily (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            date            TEXT NOT NULL,
            signal_family   TEXT NOT NULL,
            realized_trades INTEGER NOT NULL DEFAULT 0,
            realized_pnl    REAL NOT NULL DEFAULT 0.0,
            open_lots       INTEGER NOT NULL DEFAULT 0,
            unrealized_pnl  REAL NOT NULL DEFAULT 0.0,
            total_pnl       REAL NOT NULL DEFAULT 0.0,
            wins            INTEGER NOT NULL DEFAULT 0,
            losses          INTEGER NOT NULL DEFAULT 0,
            win_rate        REAL,
            unvalued_lots   INTEGER NOT NULL DEFAULT 0,
            suspicious_lots INTEGER NOT NULL DEFAULT 0,
            created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(date, signal_family)
        )";

/// 插入 SQL. const 供单测文本断言 (Step 1 测试依赖此 const).
const PERSIST_SQL: &str = "INSERT OR REPLACE INTO paper_attribution_daily \
             (date, signal_family, realized_trades, realized_pnl, open_lots, unrealized_pnl, \
              total_pnl, wins, losses, win_rate, unvalued_lots, suspicious_lots) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

/// 建表 (spec §4.3 DDL). 幂等, 与 paper_performance_snapshot 并行, 不 UPDATE 历史行.
pub fn ensure_attribution_table() -> Result<(), String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {e}"))?;
    diesel::sql_query(DDL_SQL)
        .execute(&mut conn)
        .map_err(|e| format!("create paper_attribution_daily: {e}"))?;
    Ok(())
}

/// 写入当日归因 (INSERT OR REPLACE, 当日重算幂等).
pub fn persist_daily(daily: &DailyAttribution) -> Result<(), String> {
    ensure_attribution_table()?;
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {e}"))?;
    let date_str = daily.date.format("%Y-%m-%d").to_string();
    for row in &daily.families {
        diesel::sql_query(PERSIST_SQL)
            .bind::<diesel::sql_types::Text, _>(&date_str)
            .bind::<diesel::sql_types::Text, _>(row.family.as_str())
            .bind::<diesel::sql_types::BigInt, _>(row.realized_trades)
            .bind::<diesel::sql_types::Double, _>(row.realized_pnl)
            .bind::<diesel::sql_types::BigInt, _>(row.open_lots)
            .bind::<diesel::sql_types::Double, _>(row.unrealized_pnl)
            .bind::<diesel::sql_types::Double, _>(row.total_pnl)
            .bind::<diesel::sql_types::BigInt, _>(row.wins)
            .bind::<diesel::sql_types::BigInt, _>(row.losses)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(row.win_rate)
            .bind::<diesel::sql_types::BigInt, _>(row.unvalued_lots)
            .bind::<diesel::sql_types::BigInt, _>(row.suspicious_lots)
            .execute(&mut conn)
            .map_err(|e| format!("insert paper_attribution_daily: {e}"))?;
    }
    Ok(())
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

    #[test]
    fn window_realized_is_cumulative_across_days() {
        // CRIT-1 回归锚点: 窗口已实现 = 窗口内每日卖出累计, 非仅末日单日.
        let end = NaiveDate::from_ymd_opt(2026, 7, 20).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 200, "2026-07-16 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 100, "2026-07-17 14:00:00", "s1", "BR-234四大铁律卖出"),
            fill(3, "TEST_CODE_600000", "sell", 12.0, 100, "2026-07-20 14:00:00", "s2", "BR-234四大铁律卖出"),
        ];
        let window = aggregate_window(end, 30, &rows, &HashMap::new()).expect("valid window");
        let window_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
        // 7/17 卖出 (11.0-10.0)*100 = +100; 7/20 卖出 (12.0-10.0)*100 = +200 → 累计 +300
        assert_eq!(window_realized, 300.0);
        assert_eq!(window.families.iter().map(|f| f.realized_trades).sum::<i64>(), 2);
        // 对照: 当日口径只含 7/20 卖出
        let (daily_attributions, _) = fifo_match(&rows, end).expect("valid FIFO fills");
        assert_eq!(daily_attributions.len(), 1);
        assert_eq!(daily_attributions[0].pnl, 200.0);
    }

    #[test]
    fn window_includes_exactly_days() {
        // 30 自然日含首尾: start = end − 29; end−29 卖出入窗, end−30 卖出出窗 (off-by-one 锚点).
        let end = NaiveDate::from_ymd_opt(2026, 7, 20).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 300, "2026-06-01 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 100, "2026-06-20 14:00:00", "s1", "BR-234四大铁律卖出"), // end−30 → 出窗
            fill(3, "TEST_CODE_600000", "sell", 12.0, 100, "2026-06-21 14:00:00", "s2", "BR-234四大铁律卖出"), // end−29 → 入窗
        ];
        let window = aggregate_window(end, 30, &rows, &HashMap::new()).expect("valid window");
        let window_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
        // 只有 6/21 卖出 (12.0-10.0)*100 = +200; 若 6/20 误入窗则 +300 (与旧 31 天 off-by-one 同形)
        assert_eq!(window_realized, 200.0);
        assert_eq!(window.families.iter().map(|f| f.realized_trades).sum::<i64>(), 1);
    }

    #[test]
    fn daily_emission_unchanged_with_emit_from_none() {
        // CRIT-1 守卫: fifo_match 2-arg wrapper 与显式 None 均只发射当日卖出 (日级契约不变).
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 200, "2026-07-16 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 100, "2026-07-17 14:00:00", "s1", "BR-234四大铁律卖出"),
            fill(3, "TEST_CODE_600000", "sell", 12.0, 100, "2026-07-18 14:00:00", "s2", "BR-234四大铁律卖出"),
        ];
        let (attributions, _) = fifo_match(&rows, target).expect("valid FIFO fills");
        assert_eq!(attributions.len(), 1);
        assert_eq!(attributions[0].pnl, 200.0);
        assert_eq!(attributions[0].sell_date, target);
        let (from_none, _) = fifo_match_from(&rows, target, None).expect("valid FIFO fills");
        assert_eq!(from_none, attributions); // 显式 None 与 wrapper 逐字节一致
    }

    #[test]
    fn fifo_rejects_invalid_identity_timestamp_and_late_fills() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let err = fifo_match(&[fill(0, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst")], target)
            .expect_err("id<=0 must fail");
        assert!(err.contains("identity invalid"));
        let empty_code = fill(1, "", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst");
        let err = fifo_match(&[empty_code], target).expect_err("empty code must fail");
        assert!(err.contains("identity invalid"));
        let bad_ts = fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "not-a-timestamp", "p1", "NewsCatalyst");
        let err = fifo_match(&[bad_ts], target).expect_err("bad timestamp must fail");
        assert!(err.contains("timestamp invalid"));
        let late = fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-19 10:00:00", "p1", "NewsCatalyst");
        let err = fifo_match(&[late], target).expect_err("later than settlement must fail");
        assert!(err.contains("later than settlement date"));
    }

    #[test]
    fn fifo_rejects_unordered_fills_invalid_direction_and_quantity() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let unordered = vec![
            fill(2, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst"),
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 09:00:00", "p2", "NewsCatalyst"),
        ];
        let err = fifo_match(&unordered, target).expect_err("unordered fills must fail");
        assert!(err.contains("not ordered"));
        let bad_dir = fill(1, "TEST_CODE_600000", "hold", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst");
        let err = fifo_match(&[bad_dir], target).expect_err("invalid direction must fail");
        assert!(err.contains("direction invalid"));
        let bad_qty = fill(1, "TEST_CODE_600000", "buy", 10.0, 150, "2026-07-18 10:00:00", "p1", "NewsCatalyst");
        let err = fifo_match(&[bad_qty], target).expect_err("invalid quantity must fail");
        assert!(err.contains("quantity invalid"));
    }

    #[test]
    fn fifo_rejects_sell_without_matched_buys() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let sell_only = vec![fill(1, "TEST_CODE_600000", "sell", 11.0, 100, "2026-07-18 14:00:00", "s1", "BR-234四大铁律卖出")];
        let err = fifo_match(&sell_only, target).expect_err("sell without buys must fail");
        assert!(err.contains("no matched buy lots"));
        // 注: non-finite PnL 分支 (fifo_match 末尾) 在 price/quantity 前置校验下不可达,
        // 不做直测 — 与 snapshot.rs 移植副本同理由.
    }

    #[test]
    fn top_trades_keeps_five_per_side_ordered() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        // 6 盈利 (+10..+60) + 6 亏损 (-10..-60), 各截断到 ≤5
        let mut attributions = Vec::new();
        for i in 1..=6 {
            attributions.push(TradeAttribution {
                sell_id: i,
                code: format!("TEST_CODE_60000{i}"),
                pnl: (i * 10) as f64,
                entry_plan_id: format!("w{i}"),
                entry_family: SignalFamily::NewsCatalyst,
                exit_reason: "BR-234四大铁律卖出".to_string(),
                suspicious: false,
                sell_date: target,
            });
            attributions.push(TradeAttribution {
                sell_id: 10 + i,
                code: format!("TEST_CODE_6000{i}0"),
                pnl: -(i * 10) as f64,
                entry_plan_id: format!("l{i}"),
                entry_family: SignalFamily::ExitByRule,
                exit_reason: "BR-234四大铁律卖出".to_string(),
                suspicious: false,
                sell_date: target,
            });
        }
        let top = top_trades(&attributions);
        assert_eq!(top.len(), 10); // 盈利 5 + 亏损 5
        assert_eq!(top[0].pnl, 60.0); // 盈利降序在前
        assert_eq!(top[4].pnl, 20.0);
        assert_eq!(top[5].pnl, -60.0); // 亏损升序 (最负在前) 在后
        assert_eq!(top[9].pnl, -20.0);
    }

    #[test]
    fn aggregate_families_sums_realized_and_unrealized() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "news-1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "buy", 12.0, 200, "2026-07-18 09:31:00", "fund-2", "MainNetInflow"),
            fill(3, "TEST_CODE_600000", "sell", 15.0, 200, "2026-07-18 14:00:00", "sell-3", "BR-234四大铁律卖出:结构止损"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        // T2 review Minor-2 (carried): 锁 open lot 契约 — plan_id/family 贯通 fifo_match → 聚合
        assert_eq!(open[0].plan_id, "fund-2");
        assert_eq!(open[0].family, SignalFamily::MainNetInflow);
        let mut prices = HashMap::new();
        prices.insert("TEST_CODE_600000".to_string(), 16.0);
        let families = aggregate_families(&attributions, &open, &prices);

        let news = families.iter().find(|f| f.family == SignalFamily::NewsCatalyst).expect("news family");
        assert_eq!(news.realized_pnl, 500.0);
        assert_eq!(news.realized_trades, 1);
        assert_eq!(news.wins, 1);
        assert_eq!(news.losses, 0);
        assert_eq!(news.win_rate, Some(1.0));
        assert_eq!(news.unrealized_pnl, 0.0);
        assert_eq!(news.open_lots, 0);

        let fund = families.iter().find(|f| f.family == SignalFamily::MainNetInflow).expect("fund family");
        assert_eq!(fund.realized_pnl, 300.0);
        // 剩余 100 股 × (16.0 - 12.0) = +400 浮盈
        assert_eq!(fund.unrealized_pnl, 400.0);
        assert_eq!(fund.open_lots, 1);
        assert_eq!(fund.total_pnl, 700.0);
    }

    #[test]
    fn missing_close_price_counts_unvalued_not_silent() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "news-1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "buy", 12.0, 100, "2026-07-18 09:31:00", "news-2", "NewsCatalyst"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let prices = HashMap::new(); // 无任何收盘价
        let families = aggregate_families(&attributions, &open, &prices);
        let news = families.iter().find(|f| f.family == SignalFamily::NewsCatalyst).expect("news family");
        assert_eq!(news.open_lots, 2);
        assert_eq!(news.unvalued_lots, 2);
        assert_eq!(news.unrealized_pnl, 0.0); // 未估值不填零假装, 但计数出声
        assert_eq!(news.suspicious_lots, 0);
    }

    #[test]
    fn suspicious_lots_are_counted_per_family() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "p1", "盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let families = aggregate_families(&attributions, &open, &HashMap::new());
        let fund = families.iter().find(|f| f.family == SignalFamily::PostCloseFundInflow).expect("fund family");
        assert_eq!(fund.suspicious_lots, 1);
    }

    #[test]
    fn ddl_const_declares_unique_per_date_and_family() {
        // 当日重算幂等锚点 (spec §4.3): UNIQUE(date, signal_family) + INSERT OR REPLACE
        assert!(DDL_SQL.contains("CREATE TABLE IF NOT EXISTS paper_attribution_daily"));
        assert!(DDL_SQL.contains("UNIQUE(date, signal_family)"));
        assert!(DDL_SQL.contains("unvalued_lots"));
        assert!(DDL_SQL.contains("suspicious_lots"));
    }

    #[test]
    fn persist_const_has_12_bind_slots_matching_12_columns() {
        // INSERT OR REPLACE (当日幂等, 与 snapshot 同模式) + 12 列 ↔ 12 个绑定占位
        assert!(PERSIST_SQL.contains("INSERT OR REPLACE INTO paper_attribution_daily"));
        let cols = PERSIST_SQL.split('(').nth(2).expect("column list").split(',').count();
        let binds = PERSIST_SQL.matches('?').count();
        assert_eq!(cols, binds, "columns ({cols}) must equal bind slots ({binds})");
    }
}
