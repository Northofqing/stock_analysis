//! 持仓账本 — 系统中唯一的 Position 定义。
//!
//! 所有模块通过这里的 API 获取持仓/交易信息，不再各自读环境变量或 DB。
//! API 是纯函数，不定义 trait（单用户，单实现）。

pub mod closing_valuation;
mod store;
pub mod user_position_snapshot;
pub use store::{live_rolling_sharpe, strategy_correlation_matrix};

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

// ============================================================================
// 公共结构（系统中唯一的 Position / Trade 定义）
// ============================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Position {
    pub code: String,
    pub name: String,
    pub shares: u64,
    pub cost_price: f64,
    /// User-/strategy-established hard stop. Missing until persisted evidence exists.
    pub hard_stop: Option<f64>,
    pub added_at: NaiveDate,
    pub status: PositionStatus,
    /// 修复 P1.6: 板块字段 (用于板块集中度检查)
    /// 量化分析师要求: 同板块持仓总市值不能超 single_sector_max_pct
    /// 默认 "其他" 表示未分类, 后续可接东财/同花顺的板块数据自动填充
    #[serde(default = "default_sector")]
    pub sector: String,
    /// v53: ST/*ST 标识 (T-16 ST 涨跌幅变更 dispatcher 数据源)
    /// 默认 false, 由 broker/exchange 推送更新, 或手工设置
    /// `*ST` 用 star_st 字段, `ST` 用 is_st
    #[serde(default)]
    pub is_st: bool,
    #[serde(default)]
    pub star_st: bool,
}

fn default_sector() -> String {
    "其他".to_string()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionStatus {
    #[default]
    Holding,
    Watching,
}

impl PositionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            PositionStatus::Holding => "持仓",
            PositionStatus::Watching => "自选",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub id: Option<String>,
    pub code: String,
    pub name: String,
    pub direction: TradeDirection,
    pub price: f64,
    pub shares: u64,
    pub amount: f64,
    pub reason: String,
    pub traded_at: NaiveDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDirection {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub date: NaiveDate,
    pub total_value: f64,
    pub cash: f64,
    pub market_value: f64,
    pub daily_pnl: f64,
}

// ============================================================================
// 公共 API
// ============================================================================

/// 获取当前持仓列表
pub fn get_positions() -> Result<Vec<Position>, String> {
    crate::portfolio::store::load_positions()
}

/// 获取真实持仓批次及最旧来源时间，用于 30 秒新鲜度门。
pub fn get_positions_with_source_time(
) -> Result<(Vec<Position>, Option<chrono::DateTime<chrono::Local>>), String> {
    crate::portfolio::store::load_positions_with_source_time()
}

/// BR-097: position/account snapshots are valid for at most 30 seconds.
pub fn position_source_is_fresh(
    source_time: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let age_ms = now
        .signed_duration_since(source_time.with_timezone(&chrono::Utc))
        .num_milliseconds();
    (0..=30_000).contains(&age_ms)
}

/// 获取自选列表（来自环境变量 STOCK_LIST，用于向后兼容）
pub fn get_watchlist() -> Result<Vec<Position>, String> {
    crate::portfolio::store::load_watchlist()
}

/// 所有关注标的的 code（持仓 ∪ 自选）
pub fn get_all_codes() -> Result<Vec<String>, String> {
    let positions = get_positions()?;
    let watchlist = get_watchlist()?;
    let mut codes: Vec<String> = positions.iter().map(|p| p.code.clone()).collect();
    for w in &watchlist {
        if !codes.contains(&w.code) {
            codes.push(w.code.clone());
        }
    }
    Ok(codes)
}

/// 所有标的的 name（用于实体关联）
pub fn get_all_names() -> Result<Vec<(String, String)>, String> {
    let positions = get_positions()?;
    let watchlist = get_watchlist()?;
    let mut result: Vec<(String, String)> = positions
        .iter()
        .map(|p| (p.code.clone(), p.name.clone()))
        .collect();
    for w in &watchlist {
        if !result.iter().any(|(c, _)| c == &w.code) {
            result.push((w.code.clone(), w.name.clone()));
        }
    }
    Ok(result)
}

/// 获取指定 code 的持仓（`Ok(None)` = 成功查询且不在持仓中）。
pub fn find_position(code: &str) -> Result<Option<Position>, String> {
    Ok(get_positions()?.into_iter().find(|p| p.code == code))
}

/// v53: 获取 ST/*ST 持仓 (T-16 ST 涨跌幅变更 dispatcher 数据源)
///   简化版: 当前 is_st/star_st 都默认 false, 真实来源待 broker 接入
///   真实意图: 每天 9:30 推一次, 给所有 ST/*ST 持仓提醒涨跌幅变更
///
/// 修复 review #14: 只返回 code 列表, 避免 50 × Position (含 3 个 String)
/// deep clone. 调用方按需 find_position(code) 取单只详情.
pub fn get_st_positions() -> Result<Vec<Position>, String> {
    Ok(get_positions()?
        .into_iter()
        .filter(|position| position.is_st || position.star_st)
        .collect())
}

/// 判断是否 T+1 锁仓（今日买入的不可卖出）
///
/// 修复 review #14: 原 `unwrap_or(false)` 在 DB 错误时返回 false,
/// 即"未锁仓" → 调用方可能当日卖出今日买入的票, 违反 T+1 制度.
/// 现在返回 `Result<bool, String>` 让调用方显式处理失败, 不可静默.
pub fn is_t1_locked(code: &str) -> Result<bool, String> {
    let today = chrono::Local::now().date_naive();
    crate::portfolio::store::has_buy_today(code, today)
}

/// 获取今日交易
pub fn get_today_trades() -> Result<Vec<Trade>, String> {
    let today = chrono::Local::now().date_naive();
    crate::portfolio::store::load_trades_since(today)
}

/// 获取历史交易（最近 N 天）
pub fn get_trade_history(days: u32) -> Result<Vec<Trade>, String> {
    let since = chrono::Local::now().date_naive() - chrono::Duration::days(days as i64);
    crate::portfolio::store::load_trades_since(since)
}

/// 记录每日净值快照
pub fn snapshot_ledger(entry: LedgerEntry) -> Result<(), String> {
    crate::portfolio::store::save_ledger(entry)
}

/// 净值时间序列
pub fn get_equity_curve(days: u32) -> Result<Vec<LedgerEntry>, String> {
    let since = chrono::Local::now().date_naive() - chrono::Duration::days(days as i64);
    crate::portfolio::store::load_ledger(since)
}

/// BR-103/2.4: load a ledger curve only when its newest row is the required
/// report date. Historical rows must never masquerade as today's account NAV.
pub fn get_equity_curve_as_of(
    days: u32,
    required_as_of: chrono::NaiveDate,
) -> Result<Vec<LedgerEntry>, String> {
    let curve = get_equity_curve(days)?;
    validate_equity_curve_as_of(&curve, required_as_of)?;
    Ok(curve)
}

fn validate_equity_curve_as_of(
    curve: &[LedgerEntry],
    required_as_of: chrono::NaiveDate,
) -> Result<(), String> {
    let latest = curve
        .last()
        .ok_or_else(|| format!("ledger 当日净值缺失: required={required_as_of}"))?;
    if latest.date != required_as_of {
        return Err(format!(
            "ledger 净值已过期: latest={} required={required_as_of}",
            latest.date
        ));
    }
    Ok(())
}

#[cfg(test)]
mod freshness_tests {
    use super::*;

    #[test]
    fn br097_position_source_freshness_has_exact_thirty_second_boundary() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-18T02:15:35Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(position_source_is_fresh(
            (now - chrono::Duration::seconds(30)).with_timezone(&chrono::Local),
            now
        ));
        assert!(!position_source_is_fresh(
            (now - chrono::Duration::milliseconds(30_001)).with_timezone(&chrono::Local),
            now
        ));
    }

    #[test]
    fn br103_equity_curve_requires_the_report_date() {
        let latest = chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        let curve = vec![LedgerEntry {
            date: latest,
            total_value: 100.0,
            cash: 40.0,
            market_value: 60.0,
            daily_pnl: 1.0,
        }];
        assert!(validate_equity_curve_as_of(&curve, latest).is_ok());
        assert!(validate_equity_curve_as_of(
            &curve,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap()
        )
        .is_err());
        assert!(validate_equity_curve_as_of(&[], latest).is_err());
    }
}
