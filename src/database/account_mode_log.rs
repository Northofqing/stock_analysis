//! v12 PR1-1.5: account_mode_log 表 DB 访问层 (纯 I/O).
//!
//! 不依赖 push / template / bin 模块. 调用方 (push_templates 或 monitor) 自行拼 T-01 + dispatch.
//!
//! 实现: 与 `database::mod.rs` 内 save_prediction 保持一致, 用 raw SQL + 单引号 escape,
//!       避免 Diesel 宏展开的 trait bound 问题 (i64 IntoUpdateTarget / ValidGrouping).

use chrono::Local;
use diesel::prelude::*;

use crate::risk::action_gate::AccountMode;

use super::DatabaseManager;

/// Diesel 返回行 (内部)
#[derive(Debug, Clone, QueryableByName)]
pub struct AccountModeLogRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub ts: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub prev_mode: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub new_mode: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub trigger_reason: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub today_pnl_pct: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    pub consecutive_n: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    pub total_pos_cheng: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub data_complete: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub pushed: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub push_attempted_at: Option<String>,
}

/// 单引号 escape (与 `database::mod.rs::save_prediction` 一致)
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// 插入一行模式变更 (pushed=0), 返回新行 id
pub fn insert_account_mode_change(
    prev: AccountMode,
    new: AccountMode,
    trigger_reason: &str,
    today_pnl_pct: Option<f64>,
    consecutive_n: Option<u32>,
    total_pos_cheng: Option<u8>,
    data_complete: bool,
) -> Result<i64, String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let ts = esc(&Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    let prev_s = mode_label(prev);
    let new_s = mode_label(new);
    let reason_s = esc(trigger_reason);

    let pnl = today_pnl_pct
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let cons = consecutive_n
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let pos = total_pos_cheng
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let dc = if data_complete { 1 } else { 0 };

    let sql = format!(
        "INSERT INTO account_mode_log (ts, prev_mode, new_mode, trigger_reason, today_pnl_pct, consecutive_n, total_pos_cheng, data_complete, pushed) \
         VALUES ('{}', '{}', '{}', '{}', {}, {}, {}, {}, 0)",
        ts, prev_s, new_s, reason_s, pnl, cons, pos, dc
    );
    diesel::sql_query(sql)
        .execute(&mut conn)
        .map_err(|e| format!("insert_account_mode_change: {}", e))?;

    // 取 last_insert_rowid
    #[derive(diesel::QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        id: i64,
    }
    let row: IdRow = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result(&mut conn)
        .map_err(|e| format!("last_insert_rowid: {}", e))?;
    Ok(row.id)
}

/// 标记某行已推送 (含推送尝试时间戳)
pub fn mark_account_mode_pushed(log_id: i64) -> Result<(), String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let ts = esc(&Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    let sql = format!(
        "UPDATE account_mode_log SET pushed = 1, push_attempted_at = '{}' WHERE id = {}",
        ts, log_id
    );
    diesel::sql_query(sql)
        .execute(&mut conn)
        .map_err(|e| format!("mark_account_mode_pushed: {}", e))?;
    Ok(())
}

/// 取最近 N 条变更记录 (倒序, 供单测/审计)
pub fn recent_account_mode_changes(limit: i64) -> Result<Vec<AccountModeLogRow>, String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let sql = format!(
        "SELECT id, ts, prev_mode, new_mode, trigger_reason, today_pnl_pct, consecutive_n, total_pos_cheng, data_complete, pushed, push_attempted_at \
         FROM account_mode_log ORDER BY id DESC LIMIT {}",
        limit
    );
    let rows: Vec<AccountModeLogRow> = diesel::sql_query(sql)
        .load(&mut conn)
        .map_err(|e| format!("recent_account_mode_changes: {}", e))?;
    Ok(rows)
}

/// 取最近一条变更 (供 PR1-1.7 重启时恢复 prev_mode 用)
pub fn latest_account_mode_change() -> Result<Option<AccountModeLogRow>, String> {
    let mut rows = recent_account_mode_changes(1)?;
    Ok(rows.pop())
}

fn mode_label(m: AccountMode) -> &'static str {
    match m {
        AccountMode::Normal => "Normal",
        AccountMode::ReduceOnly => "ReduceOnly",
        AccountMode::Frozen => "Frozen",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_label_strings() {
        assert_eq!(mode_label(AccountMode::Normal), "Normal");
        assert_eq!(mode_label(AccountMode::ReduceOnly), "ReduceOnly");
        assert_eq!(mode_label(AccountMode::Frozen), "Frozen");
    }

    /// DB 集成测试由 `tests/account_mode_log_tests.rs` (PR1-1.7 验收时补) 覆盖.
    /// 这里只测试纯函数 (mode_label).
    #[test]
    fn mode_label_coverage() {
        for m in [
            AccountMode::Normal,
            AccountMode::ReduceOnly,
            AccountMode::Frozen,
        ] {
            assert!(!mode_label(m).is_empty());
        }
    }

    #[test]
    fn esc_escapes_single_quote() {
        assert_eq!(esc("abc"), "abc");
        assert_eq!(esc("O'Brien"), "O''Brien");
        assert_eq!(esc("a'b'c"), "a''b''c");
        assert_eq!(esc(""), "");
    }
}
