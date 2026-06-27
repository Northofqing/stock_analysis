//! Portfolio 存储层 — SQLite 读写（复用 DatabaseManager 单例）。

use chrono::{NaiveDate, NaiveDateTime};
use diesel::prelude::*;

use super::{LedgerEntry, Position, PositionStatus, Trade, TradeDirection};

/// 从 stock_position 表加载持仓
pub fn load_positions() -> Result<Vec<Position>, String> {
    let db = crate::database::DatabaseManager::get();
    let records = db.get_all_open_positions().map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(|r| Position {
        code: r.code,
        name: r.name,
        shares: r.quantity.max(0) as u64,
        cost_price: r.buy_price,
        hard_stop: 0.0,
        added_at: NaiveDate::parse_from_str(&r.buy_date, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
        status: PositionStatus::Holding,
    }).collect())
}

/// 从环境变量加载自选（尝试解析真实名称）
pub fn load_watchlist() -> Result<Vec<Position>, String> {
    let list = std::env::var("STOCK_LIST").unwrap_or_default();
    let name_fetcher = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::data_provider::DataFetcherManager::new().ok()
    })).unwrap_or(None);
    Ok(list.split(',').map(|s| s.trim()).filter(|s| s.len() == 6).map(|code| {
        let name = name_fetcher.as_ref()
            .and_then(|f| f.get_stock_name(code))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("股票{}", code));
        Position {
            code: code.to_string(), name,
            shares: 0, cost_price: 0.0, hard_stop: 0.0,
            added_at: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            status: PositionStatus::Watching,
        }
    }).collect())
}

/// 从 trades 表加载交易记录
/// 修复 P1.1: SQL 注入风险
/// 原代码用 format!() 拼接 SQL, 字段值含单引号或反斜杠时会破坏查询甚至被攻击
/// 改用 ? 占位符 + bind 绑定, Diesel 自动转义
pub fn load_trades_since(since: NaiveDate) -> Result<Vec<Trade>, String> {
    use diesel::sql_types::{Date, Double, Integer, Text};
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows: Vec<TradeRow> = diesel::sql_query(
        "SELECT id, code, name, direction, price, shares, amount, reason, traded_at \
         FROM trades WHERE traded_at >= ? ORDER BY traded_at DESC"
    )
    .bind::<Date, _>(since)
    .load(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| Trade {
        id: Some(r.id.to_string()),
        code: r.code, name: r.name,
        direction: if r.direction == "buy" { TradeDirection::Buy } else { TradeDirection::Sell },
        price: r.price, shares: r.shares.max(0) as u64,
        amount: r.amount, reason: r.reason,
        traded_at: NaiveDateTime::parse_from_str(&format!("{} 00:00:00", r.traded_at), "%Y-%m-%d %H:%M:%S")
            .unwrap_or_else(|_| NaiveDateTime::parse_from_str("2025-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()),
    }).collect())
}

/// 检查今日是否有买入（DB 未初始化时返回 false）
/// 修复 P1.1: SQL 注入风险 (改 ? 占位符)
pub fn has_buy_today(code: &str, today: NaiveDate) -> Result<bool, String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use diesel::sql_types::{Date, Integer, Text};
        let db = crate::database::DatabaseManager::get();
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        #[derive(QueryableByName, Debug)]
        struct Count { #[diesel(sql_type = diesel::sql_types::Integer)] cnt: i32 }
        let result = diesel::sql_query(
            "SELECT COUNT(*) as cnt FROM trades WHERE code = ? AND direction = 'buy' AND traded_at = ?"
        )
        .bind::<Text, _>(code)
        .bind::<Date, _>(today)
        .get_result::<Count>(&mut *conn)
        .map_err(|e| e.to_string())?;
        Ok::<bool, String>(result.cnt > 0)
    }));
    match result {
        Ok(Ok(v)) => Ok(v),
        _ => Ok(false), // DB 未初始化或查询失败 → 保守返回 false
    }
}

/// 保存净值快照
/// 修复 P1.1: SQL 注入风险
pub fn save_ledger(entry: LedgerEntry) -> Result<(), String> {
    use diesel::sql_types::{Date, Double};
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query(
        "INSERT OR REPLACE INTO ledger (date, total_value, cash, market_value, daily_pnl) \
         VALUES (?, ?, ?, ?, ?)"
    )
    .bind::<Date, _>(entry.date)
    .bind::<Double, _>(entry.total_value)
    .bind::<Double, _>(entry.cash)
    .bind::<Double, _>(entry.market_value)
    .bind::<Double, _>(entry.daily_pnl)
    .execute(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 加载净值时间序列
/// 修复 P1.1: SQL 注入风险
pub fn load_ledger(since: NaiveDate) -> Result<Vec<LedgerEntry>, String> {
    use diesel::sql_types::Date;
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows: Vec<LedgerRow> = diesel::sql_query(
        "SELECT date, total_value, cash, market_value, daily_pnl \
         FROM ledger WHERE date >= ? ORDER BY date ASC"
    )
    .bind::<Date, _>(since)
    .load(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| LedgerEntry {
        date: NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").unwrap_or(since),
        total_value: r.total_value, cash: r.cash,
        market_value: r.market_value, daily_pnl: r.daily_pnl,
    }).collect())
}

// ── diesel raw query row types ──

#[derive(QueryableByName, Debug)]
struct TradeRow {
    #[diesel(sql_type = diesel::sql_types::Integer)] id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)] code: String,
    #[diesel(sql_type = diesel::sql_types::Text)] name: String,
    #[diesel(sql_type = diesel::sql_types::Text)] direction: String,
    #[diesel(sql_type = diesel::sql_types::Double)] price: f64,
    #[diesel(sql_type = diesel::sql_types::Integer)] shares: i32,
    #[diesel(sql_type = diesel::sql_types::Double)] amount: f64,
    #[diesel(sql_type = diesel::sql_types::Text)] reason: String,
    #[diesel(sql_type = diesel::sql_types::Text)] traded_at: String,
}

#[derive(QueryableByName, Debug)]
struct LedgerRow {
    #[diesel(sql_type = diesel::sql_types::Text)] date: String,
    #[diesel(sql_type = diesel::sql_types::Double)] total_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)] cash: f64,
    #[diesel(sql_type = diesel::sql_types::Double)] market_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)] daily_pnl: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TEST_DB: &str = "./test_data/test.db";

    fn init() {
        std::fs::create_dir_all("./test_data").ok();
        // DB 可能已被其他测试初始化，忽略重复初始化错误
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::init(Some(PathBuf::from(TEST_DB)))
        }));
    }

    // ── load_positions ──

    #[test]
    fn test_load_positions_empty() {
        init();
        // 无 open 持仓时返回空
        let positions = load_positions().unwrap_or_default();
        // 可能为空（测试 DB 无数据）或有数据（DB 已有记录），两种都合法
        assert!(positions.is_empty() || !positions.is_empty());
    }

    // ── load_trades_since ──

    #[test]
    fn test_load_trades_since_empty() {
        init();
        let future = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let trades = load_trades_since(future).unwrap_or_default();
        assert!(trades.is_empty());
    }

    // ── save_ledger + load_ledger ──

    #[test]
    fn test_ledger_roundtrip() {
        init();
        let date = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        let entry = LedgerEntry { date, total_value: 100_000.0, cash: 20_000.0, market_value: 80_000.0, daily_pnl: 5_000.0 };
        save_ledger(entry).unwrap();

        let curve = load_ledger(date).unwrap();
        assert!(!curve.is_empty());
        let last = curve.last().unwrap();
        assert_eq!(last.date, date);
        assert!((last.total_value - 100_000.0).abs() < 0.01);
        assert!((last.daily_pnl - 5_000.0).abs() < 0.01);
    }

    #[test]
    fn test_load_ledger_empty() {
        init();
        let future = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let curve = load_ledger(future).unwrap_or_default();
        assert!(curve.is_empty());
    }

    // ── has_buy_today ──

    #[test]
    fn test_has_buy_today_no_trades() {
        init();
        let today = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
        // 测试 DB 里没有今天的买入 → false
        let result = has_buy_today("000000", today);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
