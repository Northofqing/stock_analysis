//! 持仓账本 — 系统中唯一的 Position 定义。
//!
//! 所有模块通过这里的 API 获取持仓/交易信息，不再各自读环境变量或 DB。
//! API 是纯函数，不定义 trait（单用户，单实现）。

mod store;

use chrono::{NaiveDate, NaiveDateTime};

// ============================================================================
// 公共结构（系统中唯一的 Position / Trade 定义）
// ============================================================================

#[derive(Debug, Clone)]
pub struct Position {
    pub code: String,
    pub name: String,
    pub shares: u64,
    pub cost_price: f64,
    pub hard_stop: f64,
    pub added_at: NaiveDate,
    pub status: PositionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionStatus { Holding, Watching }

impl PositionStatus {
    pub fn label(&self) -> &'static str {
        match self { PositionStatus::Holding => "持仓", PositionStatus::Watching => "自选" }
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
pub enum TradeDirection { Buy, Sell }

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
        if !codes.contains(&w.code) { codes.push(w.code.clone()); }
    }
    Ok(codes)
}

/// 所有标的的 name（用于实体关联）
pub fn get_all_names() -> Result<Vec<(String, String)>, String> {
    let positions = get_positions()?;
    let watchlist = get_watchlist()?;
    let mut result: Vec<(String, String)> = positions.iter()
        .map(|p| (p.code.clone(), p.name.clone())).collect();
    for w in &watchlist {
        if !result.iter().any(|(c, _)| c == &w.code) {
            result.push((w.code.clone(), w.name.clone()));
        }
    }
    Ok(result)
}

/// 获取指定 code 的持仓（None = 不在持仓中）
pub fn find_position(code: &str) -> Option<Position> {
    get_positions().ok()?.into_iter().find(|p| p.code == code)
}

/// 判断是否 T+1 锁仓（今日买入的不可卖出）
pub fn is_t1_locked(code: &str) -> bool {
    let today = chrono::Local::now().date_naive();
    crate::portfolio::store::has_buy_today(code, today).unwrap_or(false)
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
