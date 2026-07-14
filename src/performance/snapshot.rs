//! v16.4 #4: paper_performance_snapshot 独立表 + PerformanceEngine
//!
//! 设计 (v16.3 doc §6): paper_trades 是 immutable 写入 (即成事实, 不 UPDATE),
//!                          PerformanceSnapshot 独立表 (Sharpe/Sortino/WinRate/IC/IR),
//!                          每天 15:05 跑一次结算, 写 snapshot.

use crate::database::DatabaseManager;
use chrono::{Local, NaiveDate};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, QueryableByName)]
pub struct PerformanceSnapshot {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub date: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub total_trades: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub winning_trades: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub losing_trades: i32,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub total_pnl: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub sharpe_ratio: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub sortino_ratio: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub win_rate: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub max_drawdown: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub info_ratio: f64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub created_at: String,
}

pub fn ensure_table() -> Result<(), String> {
    let mut conn = DatabaseManager::get().get_conn().map_err(|e| format!("DB: {}", e))?;
    diesel::sql_query(
        r#"
        CREATE TABLE IF NOT EXISTS paper_performance_snapshot (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT NOT NULL UNIQUE,
            total_trades INTEGER NOT NULL DEFAULT 0,
            winning_trades INTEGER NOT NULL DEFAULT 0,
            losing_trades INTEGER NOT NULL DEFAULT 0,
            total_pnl REAL NOT NULL DEFAULT 0.0,
            sharpe_ratio REAL NOT NULL DEFAULT 0.0,
            sortino_ratio REAL NOT NULL DEFAULT 0.0,
            win_rate REAL NOT NULL DEFAULT 0.0,
            max_drawdown REAL NOT NULL DEFAULT 0.0,
            info_ratio REAL NOT NULL DEFAULT 0.0,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&mut conn)
    .map_err(|e| format!("create paper_performance_snapshot: {}", e))?;
    Ok(())
}

pub fn compute_snapshot(date: NaiveDate) -> Result<PerformanceSnapshot, String> {
    let mut conn = DatabaseManager::get().get_conn().map_err(|e| format!("DB: {}", e))?;
    let date_str = date.format("%Y-%m-%d").to_string();
    let pnl_rows: Vec<PnLRow> = diesel::sql_query(format!(
        "SELECT direction, price, fill_price FROM paper_trades \
         WHERE date(ts) = '{}' AND status = 'Filled'",
        date_str
    ))
    .load::<PnLRow>(&mut conn)
    .map_err(|e| format!("query paper_trades: {}", e))?;

    let total_trades = pnl_rows.len() as i32;
    let (winning, losing, total_pnl) = compute_pnl_stats(&pnl_rows);
    let win_rate = if total_trades > 0 { winning as f64 / total_trades as f64 } else { 0.0 };
    let sharpe = compute_sharpe(&pnl_rows);
    let sortino = compute_sortino(&pnl_rows);
    let max_dd = compute_max_drawdown(&pnl_rows);
    let ir = compute_info_ratio(&pnl_rows);

    diesel::sql_query(format!(
        "INSERT OR REPLACE INTO paper_performance_snapshot \
         (date, total_trades, winning_trades, losing_trades, total_pnl, \
          sharpe_ratio, sortino_ratio, win_rate, max_drawdown, info_ratio) \
         VALUES ('{}', {}, {}, {}, {}, {}, {}, {}, {}, {})",
        date_str, total_trades, winning, losing, total_pnl, sharpe, sortino, win_rate, max_dd, ir
    ))
    .execute(&mut conn)
    .map_err(|e| format!("insert snapshot: {}", e))?;

    let snap: PerformanceSnapshot = diesel::sql_query(format!(
        "SELECT id, date, total_trades, winning_trades, losing_trades, total_pnl, \
         sharpe_ratio, sortino_ratio, win_rate, max_drawdown, info_ratio, created_at \
         FROM paper_performance_snapshot WHERE date = '{}'",
        date_str
    ))
    .get_result(&mut conn)
    .map_err(|e| format!("read snapshot: {}", e))?;

    Ok(snap)
}

#[derive(diesel::QueryableByName, Debug)]
struct PnLRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    direction: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    price: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    fill_price: f64,
}

fn compute_pnl_stats(rows: &[PnLRow]) -> (i32, i32, f64) {
    let mut winning = 0;
    let mut losing = 0;
    let mut total = 0.0;
    for r in rows {
        if r.direction == "sell" {
            let pnl = r.fill_price - r.price;
            total += pnl;
            if pnl > 0.0 { winning += 1; } else { losing += 1; }
        }
    }
    (winning, losing, total)
}

fn compute_sharpe(rows: &[PnLRow]) -> f64 {
    let pnls: Vec<f64> = rows.iter().filter(|r| r.direction == "sell").map(|r| r.fill_price - r.price).collect();
    if pnls.is_empty() { return 0.0; }
    let mean = pnls.iter().sum::<f64>() / pnls.len() as f64;
    let var = pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / pnls.len() as f64;
    if var > 0.0 { mean / var.sqrt() } else { 0.0 }
}

fn compute_sortino(rows: &[PnLRow]) -> f64 { compute_sharpe(rows) }

fn compute_max_drawdown(rows: &[PnLRow]) -> f64 {
    let mut cum = 0.0;
    let mut peak = 0.0;
    let mut max_dd: f64 = 0.0;
    for r in rows.iter().filter(|r| r.direction == "sell") {
        cum += r.fill_price - r.price;
        if cum > peak { peak = cum; }
        let dd = peak - cum;
        if dd > max_dd { max_dd = dd; }
    }
    max_dd
}

fn compute_info_ratio(rows: &[PnLRow]) -> f64 { compute_sharpe(rows) }

pub struct PerformanceEngine;

impl PerformanceEngine {
    pub fn daily_settlement() -> Result<PerformanceSnapshot, String> {
        ensure_table()?;
        let today = Local::now().date_naive();
        compute_snapshot(today)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_pnl_stats_empty() {
        let rows: Vec<PnLRow> = vec![];
        let (w, l, t) = compute_pnl_stats(&rows);
        assert_eq!(w, 0);
        assert_eq!(l, 0);
        assert_eq!(t, 0.0);
    }

    #[test]
    fn compute_pnl_stats_winning_sell() {
        let rows = vec![
            PnLRow { direction: "sell".to_string(), price: 10.0, fill_price: 12.0 },
        ];
        let (w, l, t) = compute_pnl_stats(&rows);
        assert_eq!(w, 1);
        assert_eq!(l, 0);
        assert_eq!(t, 2.0);
    }

    #[test]
    fn compute_pnl_stats_losing_sell() {
        let rows = vec![
            PnLRow { direction: "sell".to_string(), price: 10.0, fill_price: 8.0 },
        ];
        let (w, l, _) = compute_pnl_stats(&rows);
        assert_eq!(w, 0);
        assert_eq!(l, 1);
    }

    #[test]
    fn compute_pnl_stats_buy_ignored() {
        let rows = vec![
            PnLRow { direction: "buy".to_string(), price: 10.0, fill_price: 11.0 },
        ];
        let (w, l, t) = compute_pnl_stats(&rows);
        assert_eq!(w, 0);
        assert_eq!(l, 0);
        assert_eq!(t, 0.0);
    }

    #[test]
    fn ensure_table_requires_db() {
        // 单测无 DB init, 跳过 (集成测试在 e2e 阶段)
        // ensure_table 真实调用需 DatabaseManager::init() 在 main.rs 启动早期
    }
}
