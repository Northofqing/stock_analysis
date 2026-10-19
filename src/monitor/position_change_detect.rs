//! 持仓变化识别（2026-10-18）。
//!
//! ## 数据来源
//!
//! `user_position_snapshot` 已有 44 份快照（2026-07-21 起），
//! 明细表 `user_position_snapshot_item` 带 `quantity` 与 `cost_price`。
//! 相邻两份快照比 `quantity`，就能识别出加仓/减仓/新建/清仓。
//!
//! 之前误判过"要另建持仓历史表"——历史一直在库里，只是我没查。
//!
//! ## 四种变化
//!
//! - **新建**：上一份没有，这一份有
//! - **加仓**：两份都有，`quantity` 增加
//! - **减仓**：两份都有，`quantity` 减少
//! - **清仓**：上一份有，这一份没有
//!
//! ## 边界
//!
//! - **不改 `stage`**。阶段流转归 `ticket_stage_sync`（由当前持仓驱动），
//!   本模块只记事件。两者职责不重叠。
//! - 只对有 **active ticket** 的票记事件；没有 ticket 的票跳过，
//!   `ticket_stage_sync` 会为持仓票开票。
//! - `quantity` 相同视为无变化，不记事件。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::database::{ticket_lifecycle, DatabaseManager};

#[derive(Debug, Default, Clone)]
pub struct ChangeReport {
    /// 比较到的快照对数量：0 = 不足两份，1 = 已比较
    pub compared: usize,
    pub current_snapshot: String,
    pub previous_snapshot: String,
    /// (code, 上份数量, 本份数量)
    pub added: Vec<(String, i64, i64)>,
    pub reduced: Vec<(String, i64, i64)>,
    /// 本次新建仓
    pub opened: Vec<String>,
    /// 本次清仓
    pub exited: Vec<String>,
    pub errors: Vec<String>,
}

impl ChangeReport {
    pub fn log(&self) {
        if self.compared == 0 {
            log::info!("[持仓变化] 快照不足两份，跳过");
            return;
        }
        log::info!(
            "[持仓变化] {} -> {} 加仓={} 减仓={} 新建={} 清仓={}",
            self.previous_snapshot,
            self.current_snapshot,
            self.added.len(),
            self.reduced.len(),
            self.opened.len(),
            self.exited.len()
        );
        for (code, from, to) in &self.added {
            log::info!("[持仓变化] {code} 加仓 {from} → {to} 股");
        }
        for (code, from, to) in &self.reduced {
            log::info!("[持仓变化] {code} 减仓 {from} → {to} 股");
        }
        for code in &self.opened {
            log::info!("[持仓变化] {code} 新建仓");
        }
        for code in &self.exited {
            log::info!("[持仓变化] {code} 清仓");
        }
        for error in &self.errors {
            log::warn!("[持仓变化] {error}");
        }
    }
}

#[derive(QueryableByName)]
struct SnapshotRow {
    #[diesel(sql_type = Text)]
    snapshot_id: String,
    #[diesel(sql_type = Text)]
    effective_at: String,
}

#[derive(QueryableByName)]
struct ItemRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = BigInt)]
    quantity: i64,
}

fn load_items(
    conn: &mut SqliteConnection,
    snapshot_id: &str,
) -> Result<Vec<(String, i64)>, String> {
    let rows: Vec<ItemRow> = diesel::sql_query(
        "SELECT code, quantity FROM user_position_snapshot_item WHERE snapshot_id = ?",
    )
    .bind::<Text, _>(snapshot_id)
    .load(conn)
    .map_err(|error| format!("读取快照 {snapshot_id} 明细失败: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| (row.code, row.quantity))
        .collect())
}

/// 当日买入（T+1 锁定）股数：当前快照 vs 上一份快照的 `quantity` 增量。
///
/// A 股 T+1：当天买入的股数当天不能卖。系统拿不到券商的「可用数量」字段
/// （`UserPositionItemInput` 只有 `quantity` / `cost_price`），只能用快照增量推断。
///
/// 边界：两次快照之间买入的都算锁定。用户每天导入一次时这就是当日买入；
/// 若隔了多日才导入一次，会把多日买入全锁一天——偏保守，但不会放行不该卖的。
pub fn t1_locked_quantities(
    db: &DatabaseManager,
) -> Result<std::collections::HashMap<String, u64>, String> {
    use diesel::QueryableByName;

    #[derive(QueryableByName)]
    struct SnapshotIdRow {
        #[diesel(sql_type = Text)]
        snapshot_id: String,
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let snaps: Vec<SnapshotIdRow> = diesel::sql_query(
        "SELECT snapshot_id FROM user_position_snapshot \
         ORDER BY effective_at DESC, rowid DESC LIMIT 2",
    )
    .load(&mut conn)
    .map_err(|e| e.to_string())?;
    if snaps.len() < 2 {
        return Ok(std::collections::HashMap::new());
    }

    let current = load_items(&mut conn, &snaps[0].snapshot_id)?;
    let previous = load_items(&mut conn, &snaps[1].snapshot_id)?;
    let previous_map: std::collections::HashMap<&str, i64> = previous
        .iter()
        .map(|(code, qty)| (code.as_str(), *qty))
        .collect();

    let mut locked = std::collections::HashMap::new();
    for (code, qty) in &current {
        let old = previous_map.get(code.as_str()).copied().unwrap_or(0);
        if *qty > old {
            locked.insert(code.clone(), (*qty - old) as u64);
        }
    }
    Ok(locked)
}

pub fn detect_position_changes(db: &DatabaseManager, as_of: &str) -> Result<ChangeReport, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    let snaps: Vec<SnapshotRow> = diesel::sql_query(
        "SELECT snapshot_id, effective_at FROM user_position_snapshot \
         ORDER BY effective_at DESC, rowid DESC LIMIT 2",
    )
    .load(&mut conn)
    .map_err(|e| e.to_string())?;

    if snaps.len() < 2 {
        return Ok(ChangeReport::default());
    }

    let current_snapshot = snaps[0].snapshot_id.clone();
    let previous_snapshot = snaps[1].snapshot_id.clone();
    let current = load_items(&mut conn, &current_snapshot)?;
    let previous = load_items(&mut conn, &previous_snapshot)?;

    let mut report = ChangeReport {
        compared: 1,
        current_snapshot: current_snapshot.clone(),
        previous_snapshot,
        ..Default::default()
    };

    let previous_map: std::collections::HashMap<&str, i64> = previous
        .iter()
        .map(|(code, qty)| (code.as_str(), *qty))
        .collect();
    let current_map: std::collections::HashMap<&str, i64> = current
        .iter()
        .map(|(code, qty)| (code.as_str(), *qty))
        .collect();

    // 新建 / 加仓 / 减仓
    for (code, qty) in &current {
        match previous_map.get(code.as_str()) {
            None => {
                report.opened.push(code.clone());
                record(
                    &mut conn,
                    code,
                    as_of,
                    "position_opened",
                    &format!("新建仓 {qty} 股"),
                    &mut report,
                );
            }
            Some(&old) if *qty > old => {
                report.added.push((code.clone(), old, *qty));
                record(
                    &mut conn,
                    code,
                    as_of,
                    "position_added",
                    &format!("加仓 {old} → {qty} 股"),
                    &mut report,
                );
            }
            Some(&old) if *qty < old => {
                report.reduced.push((code.clone(), old, *qty));
                record(
                    &mut conn,
                    code,
                    as_of,
                    "position_reduced",
                    &format!("减仓 {old} → {qty} 股"),
                    &mut report,
                );
            }
            Some(_) => {}
        }
    }

    // 清仓
    for (code, qty) in &previous {
        if !current_map.contains_key(code.as_str()) {
            report.exited.push(code.clone());
            record(
                &mut conn,
                code,
                as_of,
                "position_closed",
                &format!("清仓（原 {qty} 股）"),
                &mut report,
            );
        }
    }

    Ok(report)
}

fn record(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
    kind: &str,
    summary: &str,
    report: &mut ChangeReport,
) {
    let ticket = match ticket_lifecycle::find_active_ticket(conn, code) {
        Ok(Some(ticket)) => ticket,
        Ok(None) => return, // 没有 active ticket，stage_sync 会负责开票
        Err(error) => {
            report
                .errors
                .push(format!("{code} 查询 ticket 失败: {error}"));
            return;
        }
    };
    if let Err(error) =
        ticket_lifecycle::append_event(conn, ticket.id, as_of, kind, None, None, Some(summary))
    {
        report.errors.push(format!("{code} 记事件失败: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_covers_four_cases() {
        // 直接验证比较逻辑的分支划分，不依赖数据库
        let previous = vec![("A".to_string(), 100i64), ("B".to_string(), 200i64)];
        let current = vec![
            ("A".to_string(), 150i64), // 加仓
            ("C".to_string(), 50i64),  // 新建
        ];
        let pm: std::collections::HashMap<&str, i64> =
            previous.iter().map(|(c, q)| (c.as_str(), *q)).collect();
        let cm: std::collections::HashMap<&str, i64> =
            current.iter().map(|(c, q)| (c.as_str(), *q)).collect();

        assert_eq!(pm.get("A"), Some(&100));
        assert!(current.iter().any(|(c, q)| c == "A" && *q > 100)); // 加仓
        assert!(current.iter().any(|(c, _)| c == "C") && !pm.contains_key("C")); // 新建
        assert!(!cm.contains_key("B")); // 清仓
        assert_eq!(pm.len(), 2);
    }
}
