//! 批 3（2026-10-24）：周预算、板块集中度、按预算排序（3.1 / 3.2 / 3.3）。
//!
//! 口径（operator 2026-10-24 拍板 **C：两源取 max**）：
//! - 已用 A（快照差分）：本周初快照 → 当前快照，逐票 `max(0, 现在数量 - 周初数量) × 现在的成本价`；
//! - 已用 B（台账）：本周 `trades.direction='buy'` 的成交额 ∪ `stock_position.buy_date` 本周的买入成本；
//! - `used = max(A, B)`：A 缺周初快照时 A=0，自动落到 B；B 没人记时 A 兜底。
//!
//! 预算 = 最近一次账户总资产 × 30%；单板块本周新增上限 = 总资产 × 15%（预算的一半）。
//! 拿不到总资产时返回 Err，调用方只打印「不可用」，不猜数字（BR-222）。

use crate::database::DatabaseManager;
use diesel::sql_types::{Nullable, Text};
use diesel::RunQueryDsl as _;
use std::collections::HashMap;

/// 周预算占账户总资产的比例。
pub const BUDGET_PCT: f64 = 0.30;
/// 单板块本周新增上限（占总资产）。
pub const BOARD_CAP_PCT: f64 = 0.15;

#[derive(Debug, Clone)]
pub struct WeeklyBudget {
    pub as_of: String,
    pub week_start: String,
    pub total_assets: f64,
    pub budget: f64,
    /// 本周已用（A/B 取大者）
    pub used: f64,
    /// 已用来源说明：`快照` / `台账` / `快照=台账` / `无周初快照(仅台账)`
    pub used_source: String,
    pub snapshot_used: f64,
    pub ledger_used: f64,
    /// 板块 → 本周新增成本
    pub per_board: HashMap<String, f64>,
}

impl WeeklyBudget {
    pub fn remaining(&self) -> f64 {
        (self.budget - self.used).max(0.0)
    }

    pub fn board_cap(&self) -> f64 {
        self.total_assets * BOARD_CAP_PCT
    }

    pub fn board_used(&self, board: &str) -> f64 {
        self.per_board.get(board).copied().unwrap_or(0.0)
    }

    /// 正文头部一行。
    pub fn summary_line(&self) -> String {
        format!(
            "💰 本周预算：已用 {:.0} / 预算 {:.0}（{}）｜剩余 {:.0}；单板块上限 {:.0}",
            self.used,
            self.budget,
            self.used_source,
            self.remaining(),
            self.board_cap()
        )
    }
}

pub fn load(db: &DatabaseManager, as_of: &str) -> Result<WeeklyBudget, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let week_start = monday_of(as_of)?;
    let total_assets = total_assets(&mut conn, as_of)?
        .ok_or_else(|| format!("无账户总资产（user_account_summary / real_account_snapshot 都没有 {as_of} 前的记录）"))?;

    let (snapshot_used, snapshot_costs, has_base) = snapshot_delta(&mut conn, &week_start, as_of)?;
    let (ledger_used, ledger_costs) = ledger_buys(&mut conn, &week_start, as_of)?;

    let (used, used_source) = if !has_base {
        (ledger_used, "无周初快照(仅台账)".to_string())
    } else if snapshot_used > ledger_used {
        (snapshot_used, "快照".to_string())
    } else if ledger_used > snapshot_used {
        (ledger_used, "台账".to_string())
    } else {
        (snapshot_used, "快照=台账".to_string())
    };

    // 板块归属：两张来源合并，同票取较大成本（避免同票两源各算一次）。
    let mut by_code: HashMap<String, f64> = snapshot_costs;
    for (code, cost) in ledger_costs {
        let entry = by_code.entry(code).or_insert(0.0);
        if cost > *entry {
            *entry = cost;
        }
    }
    let mut per_board: HashMap<String, f64> = HashMap::new();
    for (code, cost) in by_code {
        if let Some(board) = first_board(&mut conn, &code)? {
            *per_board.entry(board).or_insert(0.0) += cost;
        }
    }

    Ok(WeeklyBudget {
        as_of: as_of.to_string(),
        week_start,
        total_assets,
        budget: total_assets * BUDGET_PCT,
        used,
        used_source,
        snapshot_used,
        ledger_used,
        per_board,
    })
}

/// 本周一（含当天）。
fn monday_of(as_of: &str) -> Result<String, String> {
    let date = chrono::NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
        .map_err(|error| format!("日期非法 {as_of}: {error}"))?;
    let offset = chrono::Datelike::weekday(&date).num_days_from_monday() as i64;
    Ok((date - chrono::Duration::days(offset))
        .format("%Y-%m-%d")
        .to_string())
}

fn total_assets(conn: &mut diesel::SqliteConnection, as_of: &str) -> Result<Option<f64>, String> {
    #[derive(diesel::QueryableByName)]
    struct AssetRow {
        #[diesel(sql_type = diesel::sql_types::Double)]
        total_assets: f64,
    }
    let from_summary: Option<AssetRow> = diesel::sql_query(
        "SELECT total_assets FROM user_account_summary \
          WHERE substr(effective_at, 1, 10) <= ?1 ORDER BY effective_at DESC LIMIT 1",
    )
    .bind::<Text, _>(as_of.to_owned())
    .get_result(&mut *conn)
    .ok();
    if let Some(row) = from_summary {
        return Ok(Some(row.total_assets));
    }
    let from_real: Option<AssetRow> = diesel::sql_query(
        "SELECT total_assets FROM real_account_snapshot \
          WHERE snapshot_date <= ?1 ORDER BY snapshot_date DESC LIMIT 1",
    )
    .bind::<Text, _>(as_of.to_owned())
    .get_result(&mut *conn)
    .ok();
    Ok(from_real.map(|row| row.total_assets))
}

/// 快照差分：周初快照 → 当前快照，返回（已用, 逐票新增成本, 是否有周初快照）。
fn snapshot_delta(
    conn: &mut diesel::SqliteConnection,
    week_start: &str,
    as_of: &str,
) -> Result<(f64, HashMap<String, f64>, bool), String> {
    #[derive(diesel::QueryableByName)]
    struct SnapshotRow {
        #[diesel(sql_type = Text)]
        snapshot_id: String,
    }
    #[derive(diesel::QueryableByName)]
    struct ItemRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        quantity: i64,
        #[diesel(sql_type = diesel::sql_types::Double)]
        cost_price: f64,
    }

    let current: Option<String> = diesel::sql_query(
        "SELECT snapshot_id FROM user_position_snapshot \
          WHERE substr(effective_at, 1, 10) <= ?1 ORDER BY effective_at DESC LIMIT 1",
    )
    .bind::<Text, _>(as_of.to_owned())
    .get_result::<SnapshotRow>(conn)
    .ok()
    .map(|row| row.snapshot_id);
    let Some(current) = current else {
        return Ok((0.0, HashMap::new(), false));
    };
    let base: Option<String> = diesel::sql_query(
        "SELECT snapshot_id FROM user_position_snapshot \
          WHERE effective_at < ?1 ORDER BY effective_at DESC LIMIT 1",
    )
    .bind::<Text, _>(week_start.to_owned())
    .get_result::<SnapshotRow>(conn)
    .ok()
    .map(|row| row.snapshot_id);

    let current_items: Vec<ItemRow> =
        diesel::sql_query("SELECT code, quantity, cost_price FROM user_position_snapshot_item WHERE snapshot_id = ?1")
            .bind::<Text, _>(current)
            .load(conn)
            .map_err(|error| format!("当前快照明细查询失败: {error}"))?;
    let base_qty: HashMap<String, i64> = match &base {
        Some(id) => {
            let rows: Vec<ItemRow> = diesel::sql_query(
                "SELECT code, quantity, cost_price FROM user_position_snapshot_item WHERE snapshot_id = ?1",
            )
            .bind::<Text, _>(id.clone())
            .load(conn)
            .map_err(|error| format!("周初快照明细查询失败: {error}"))?;
            rows.into_iter().map(|row| (row.code, row.quantity)).collect()
        }
        None => HashMap::new(),
    };

    let mut total = 0.0;
    let mut costs: HashMap<String, f64> = HashMap::new();
    for item in current_items {
        let before = base_qty.get(&item.code).copied().unwrap_or(0);
        let added = item.quantity - before;
        if added > 0 {
            let cost = added as f64 * item.cost_price;
            total += cost;
            costs.insert(item.code, cost);
        }
    }
    Ok((total, costs, base.is_some()))
}

/// 台账买入：trades 本周买入 ∪ stock_position 本周建仓，返回（已用, 逐票成本）。
fn ledger_buys(
    conn: &mut diesel::SqliteConnection,
    week_start: &str,
    as_of: &str,
) -> Result<(f64, HashMap<String, f64>), String> {
    #[derive(diesel::QueryableByName)]
    struct BuyRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        cost: Option<f64>,
    }

    let trades: Vec<BuyRow> = diesel::sql_query(
        "SELECT code, COALESCE(amount, price * shares) AS cost FROM trades \
          WHERE direction = 'buy' AND substr(traded_at, 1, 10) >= ?1 AND substr(traded_at, 1, 10) <= ?2",
    )
    .bind::<Text, _>(week_start.to_owned())
    .bind::<Text, _>(as_of.to_owned())
    .load(conn)
    .unwrap_or_default();

    let positions: Vec<BuyRow> = diesel::sql_query(
        "SELECT code, buy_price * quantity AS cost FROM stock_position \
          WHERE buy_date >= ?1 AND buy_date <= ?2",
    )
    .bind::<Text, _>(week_start.to_owned())
    .bind::<Text, _>(as_of.to_owned())
    .load(conn)
    .unwrap_or_default();

    let sum = |rows: &[BuyRow]| -> HashMap<String, f64> {
        let mut map: HashMap<String, f64> = HashMap::new();
        for row in rows {
            if let Some(cost) = row.cost {
                *map.entry(row.code.clone()).or_insert(0.0) += cost;
            }
        }
        map
    };
    let from_trades = sum(&trades);
    let from_positions = sum(&positions);
    // 两本台账可能记同一笔（manual_broker 同时写 trades 与 stock_position）：
    // 取较大的那本，避免同一笔买入算两遍。
    let trades_total: f64 = from_trades.values().sum();
    let positions_total: f64 = from_positions.values().sum();
    if positions_total > trades_total {
        Ok((positions_total, from_positions))
    } else {
        Ok((trades_total, from_trades))
    }
}

fn first_board(conn: &mut diesel::SqliteConnection, code: &str) -> Result<Option<String>, String> {
    #[derive(diesel::QueryableByName)]
    struct BoardRow {
        #[diesel(sql_type = Nullable<Text>)]
        board_name: Option<String>,
    }
    let row: Option<BoardRow> =
        diesel::sql_query("SELECT board_name FROM board_member WHERE instrument_code = ?1 LIMIT 1")
            .bind::<Text, _>(code.to_owned())
            .get_result(conn)
            .ok();
    Ok(row.and_then(|row| row.board_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monday_is_stable_for_every_weekday() {
        // 2026-10-19 是周一；10-23 是周五，两者应落在同一个周一。
        assert_eq!(monday_of("2026-10-19").unwrap(), "2026-10-19");
        assert_eq!(monday_of("2026-10-23").unwrap(), "2026-10-19");
        assert_eq!(monday_of("2026-10-25").unwrap(), "2026-10-19");
        // 跨周：10-26 是下周一。
        assert_eq!(monday_of("2026-10-26").unwrap(), "2026-10-26");
    }

    #[test]
    fn budget_and_board_cap_are_30_and_15_percent() {
        let budget = WeeklyBudget {
            as_of: "2026-10-23".into(),
            week_start: "2026-10-19".into(),
            total_assets: 50_000.0,
            budget: 15_000.0,
            used: 6_000.0,
            used_source: "快照".into(),
            snapshot_used: 6_000.0,
            ledger_used: 0.0,
            per_board: HashMap::from([("风电".to_string(), 3_000.0)]),
        };
        assert!((budget.remaining() - 9_000.0).abs() < 1e-6);
        assert!((budget.board_cap() - 7_500.0).abs() < 1e-6);
        assert!((budget.board_used("风电") - 3_000.0).abs() < 1e-6);
        assert_eq!(budget.board_used("不存在"), 0.0);
    }
}
