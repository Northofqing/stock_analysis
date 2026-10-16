//! 建议执行跟踪 (2026-10-16).
//!
//! 设计: 每次"操作建议"推送成功 → 落一行 (executed=NULL 待确认);
//!       "执行结果"回填源 (后续): V2 paper 估值对比 / 持仓快照 diff / 手工确认.
//!
//! 背景: 旧 execution_tracking 表 schema 硬绑 paper_trade_id NOT NULL (V1 遗产),
//!       V1 于 2026-09-24 退役后结构性无法写新行 —— 本表为独立替代.
//!       写入为 best-effort: 失败只告警, 不影响推送主流程.

use diesel::prelude::*;
use diesel::sql_types::Text;

/// 建表 (幂等). 由 init_with_scope 调用.
pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS advice_outcome_tracking (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         occurrence_identity TEXT NOT NULL,\
         business_date TEXT NOT NULL,\
         push_kind TEXT NOT NULL,\
         code TEXT NOT NULL,\
         pushed_at TEXT NOT NULL,\
         executed INTEGER,\
         execution_source TEXT,\
         outcome_pct REAL,\
         note TEXT,\
         created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         UNIQUE(occurrence_identity))",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_advice_outcome_business_date ON advice_outcome_tracking(business_date)",
    )
    .execute(conn)
    .ok();
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_advice_outcome_code ON advice_outcome_tracking(code)",
    )
    .execute(conn)
    .ok();
    Ok(())
}

/// 记录一次建议推送 (幂等: occurrence_identity 唯一). 返回 true = 新插入.
pub fn record_advice_push(
    conn: &mut SqliteConnection,
    occurrence_identity: &str,
    business_date: &str,
    push_kind: &str,
    code: &str,
    pushed_at: &str,
) -> Result<bool, String> {
    let rows = diesel::sql_query(
        "INSERT OR IGNORE INTO advice_outcome_tracking \
         (occurrence_identity, business_date, push_kind, code, pushed_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(occurrence_identity)
    .bind::<Text, _>(business_date)
    .bind::<Text, _>(push_kind)
    .bind::<Text, _>(code)
    .bind::<Text, _>(pushed_at)
    .execute(conn)
    .map_err(|e| format!("advice_outcome insert failed: {e}"))?;
    Ok(rows > 0)
}
