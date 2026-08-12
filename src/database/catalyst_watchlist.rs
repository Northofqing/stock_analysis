//! T+1 关注票跟踪: A-10 题材催化复盘名单快照 + 次日核对结果 (append-only)。
//!
//! 所见即所得: 名单在 A-10 推送成功时落盘 (含上游 chain_intelligence_members
//! 的 instrument_id 与 streak), 次日盘后 R-13 按快照核对, 结果单独落 outcome 表。

use chrono::NaiveDate;
use diesel::prelude::*;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEntry {
    pub code: String,
    pub name: String,
    /// 关注日 (watch_date) 当日的连板数, 来自 chain_intelligence_members.streak
    pub streak: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchlistSnapshot {
    pub watch_date: NaiveDate,
    pub leading: Vec<WatchEntry>,
    pub other: Vec<WatchEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchOutcome {
    pub watch_date: NaiveDate,
    pub checked_date: NaiveDate,
    pub code: String,
    pub name: String,
    pub close: f64,
    pub prev_close: f64,
    pub change_pct: f64,
    pub limit_up: bool,
    /// "" (未涨停) | "封板" | "一字"
    pub limit_up_type: String,
    pub streak_today: i64,
    pub high: Option<f64>,
    pub open: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveWatchlistReceipt {
    pub run_id: String,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcomeReceipt {
    pub inserted: usize,
    pub skipped: usize,
}

// diesel 反序列化必需字段: 部分字段仅用于 SQL 查询/排序, 运行时读取标记 allow
#[allow(dead_code)]
#[derive(QueryableByName)]
struct WatchRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    watch_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    position: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    ordinal: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    streak: i64,
}

#[allow(dead_code)]
#[derive(QueryableByName)]
struct OutcomeRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    watch_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    checked_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    close: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    prev_close: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    change_pct: f64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    limit_up: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    limit_up_type: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    streak_today: i64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    high: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    open: Option<f64>,
}

#[allow(dead_code)]
#[derive(QueryableByName)]
struct RunKeyRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    run_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    watch_date: String,
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    for sql in [
        // 父表: 一份名单一个 run_id (内容哈希幂等), 与 closing_valuation_run 同款
        "CREATE TABLE IF NOT EXISTS catalyst_watchlist_run (id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE, watch_date TEXT NOT NULL)",
        // 子表: 名单成员, 复合主键 (run_id, position, ordinal)
        "CREATE TABLE IF NOT EXISTS catalyst_watchlist_daily (run_id TEXT NOT NULL REFERENCES catalyst_watchlist_run(run_id), watch_date TEXT NOT NULL, position TEXT NOT NULL, ordinal INTEGER NOT NULL, code TEXT NOT NULL, name TEXT NOT NULL, streak INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(run_id, position, ordinal))",
        "CREATE TABLE IF NOT EXISTS catalyst_watchlist_outcome (id INTEGER PRIMARY KEY AUTOINCREMENT, watch_date TEXT NOT NULL, checked_date TEXT NOT NULL, code TEXT NOT NULL, name TEXT NOT NULL, close REAL NOT NULL, prev_close REAL NOT NULL, change_pct REAL NOT NULL, limit_up INTEGER NOT NULL DEFAULT 0, limit_up_type TEXT NOT NULL DEFAULT '', streak_today INTEGER NOT NULL DEFAULT 0, high REAL, open REAL, UNIQUE(watch_date, checked_date, code))",
        "CREATE TRIGGER IF NOT EXISTS catalyst_watchlist_run_no_update BEFORE UPDATE ON catalyst_watchlist_run BEGIN SELECT RAISE(ABORT, 'catalyst_watchlist_run is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS catalyst_watchlist_run_no_delete BEFORE DELETE ON catalyst_watchlist_run BEGIN SELECT RAISE(ABORT, 'catalyst_watchlist_run is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS catalyst_watchlist_daily_no_update BEFORE UPDATE ON catalyst_watchlist_daily BEGIN SELECT RAISE(ABORT, 'catalyst_watchlist_daily is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS catalyst_watchlist_daily_no_delete BEFORE DELETE ON catalyst_watchlist_daily BEGIN SELECT RAISE(ABORT, 'catalyst_watchlist_daily is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS catalyst_watchlist_outcome_no_update BEFORE UPDATE ON catalyst_watchlist_outcome BEGIN SELECT RAISE(ABORT, 'catalyst_watchlist_outcome is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS catalyst_watchlist_outcome_no_delete BEFORE DELETE ON catalyst_watchlist_outcome BEGIN SELECT RAISE(ABORT, 'catalyst_watchlist_outcome is append-only'); END",
    ] {
        diesel::sql_query(sql).execute(conn).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run_id(watch_date: NaiveDate, leading: &[WatchEntry], other: &[WatchEntry]) -> String {
    let mut h = Sha256::new();
    h.update(b"stock_analysis.catalyst_watchlist.v1\0");
    h.update(watch_date.to_string().as_bytes());
    for entry in leading.iter().chain(other.iter()) {
        h.update(format!("\0{}|{}|{}", entry.code, entry.name, entry.streak).as_bytes());
    }
    format!("cw_v1_{:x}", h.finalize())
}

/// A-10 推送成功后调用: 名单快照落盘, 内容哈希幂等 (同一名单重复写跳过)。
pub fn save_watchlist(
    watch_date: NaiveDate,
    leading: &[WatchEntry],
    other: &[WatchEntry],
) -> Result<SaveWatchlistReceipt, String> {
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    save_watchlist_with_conn(&mut conn, watch_date, leading, other)
}

fn save_watchlist_with_conn(
    conn: &mut SqliteConnection,
    watch_date: NaiveDate,
    leading: &[WatchEntry],
    other: &[WatchEntry],
) -> Result<SaveWatchlistReceipt, String> {
    let rid = run_id(watch_date, leading, other);
    let watch_date_s = watch_date.format("%Y-%m-%d").to_string();
    conn.transaction(|c| {
        let existing: Option<RunKeyRow> = diesel::sql_query(
            "SELECT run_id, watch_date FROM catalyst_watchlist_run WHERE run_id=? LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(&rid)
        .get_result(c)
        .optional()?;
        if existing.is_some() {
            return Ok(SaveWatchlistReceipt {
                run_id: rid,
                inserted: false,
            });
        }
        diesel::sql_query("INSERT INTO catalyst_watchlist_run(run_id,watch_date) VALUES (?,?)")
            .bind::<diesel::sql_types::Text, _>(&rid)
            .bind::<diesel::sql_types::Text, _>(&watch_date_s)
            .execute(c)?;
        let mut insert_row = |position: &str, ordinal: usize, e: &WatchEntry| {
            diesel::sql_query(
                "INSERT INTO catalyst_watchlist_daily(run_id,watch_date,position,ordinal,code,name,streak) VALUES (?,?,?,?,?,?,?)",
            )
            .bind::<diesel::sql_types::Text, _>(&rid)
            .bind::<diesel::sql_types::Text, _>(&watch_date_s)
            .bind::<diesel::sql_types::Text, _>(position)
            .bind::<diesel::sql_types::Integer, _>(ordinal as i32)
            .bind::<diesel::sql_types::Text, _>(&e.code)
            .bind::<diesel::sql_types::Text, _>(&e.name)
            .bind::<diesel::sql_types::BigInt, _>(e.streak)
            .execute(c)
        };
        for (i, e) in leading.iter().enumerate() {
            insert_row("leading", i, e)?;
        }
        for (i, e) in other.iter().enumerate() {
            insert_row("other", i, e)?;
        }
        Ok(SaveWatchlistReceipt {
            run_id: rid,
            inserted: true,
        })
    })
    .map_err(|e: diesel::result::Error| e.to_string())
}

/// 取 watch_date < checked_date 的最近一份名单快照 (无则 None, 调用方出声)。
pub fn latest_watchlist_before(
    checked_date: NaiveDate,
) -> Result<Option<WatchlistSnapshot>, String> {
    let db = crate::database::DatabaseManager::get();
    let mut c = db.get_conn().map_err(|e| e.to_string())?;
    latest_watchlist_before_with_conn(&mut c, checked_date)
}

fn latest_watchlist_before_with_conn(
    c: &mut SqliteConnection,
    checked_date: NaiveDate,
) -> Result<Option<WatchlistSnapshot>, String> {
    let bound = checked_date.format("%Y-%m-%d").to_string();
    let key: Option<RunKeyRow> = diesel::sql_query(
        "SELECT run_id, watch_date FROM catalyst_watchlist_run WHERE watch_date < ? ORDER BY watch_date DESC, id DESC LIMIT 1",
    )
    .bind::<diesel::sql_types::Text, _>(&bound)
    .get_result(&mut *c)
    .optional()
    .map_err(|e| e.to_string())?;
    let Some(key) = key else {
        return Ok(None);
    };
    let rows: Vec<WatchRow> = diesel::sql_query(
        "SELECT watch_date,position,ordinal,code,name,streak FROM catalyst_watchlist_daily WHERE run_id=? ORDER BY position, ordinal",
    )
    .bind::<diesel::sql_types::Text, _>(&key.run_id)
    .load::<WatchRow>(&mut *c)
    .map_err(|e| e.to_string())?;
    let mut leading = Vec::new();
    let mut other = Vec::new();
    for r in rows {
        let entry = WatchEntry {
            code: r.code,
            name: r.name,
            streak: r.streak,
        };
        match r.position.as_str() {
            "leading" => leading.push(entry),
            "other" => other.push(entry),
            other_position => {
                return Err(format!(
                    "catalyst_watchlist_daily has unexpected position {other_position:?}"
                ))
            }
        }
    }
    if leading.is_empty() && other.is_empty() {
        return Err(format!(
            "catalyst_watchlist_daily run {} has no members",
            key.run_id
        ));
    }
    Ok(Some(WatchlistSnapshot {
        watch_date: NaiveDate::parse_from_str(&key.watch_date, "%Y-%m-%d")
            .map_err(|e| e.to_string())?,
        leading,
        other,
    }))
}

/// R-13 核对完成后调用: 结果落盘, UNIQUE(watch_date, checked_date, code) 幂等。
pub fn save_outcomes(outcomes: &[WatchOutcome]) -> Result<SaveOutcomeReceipt, String> {
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    save_outcomes_with_conn(&mut conn, outcomes)
}

fn save_outcomes_with_conn(
    conn: &mut SqliteConnection,
    outcomes: &[WatchOutcome],
) -> Result<SaveOutcomeReceipt, String> {
    let mut inserted = 0usize;
    let mut skipped = 0usize;
    conn.transaction(|c| {
        for o in outcomes {
            let watch_date_s = o.watch_date.format("%Y-%m-%d").to_string();
            let checked_date_s = o.checked_date.format("%Y-%m-%d").to_string();
            let existing: Option<OutcomeRow> = diesel::sql_query(
                "SELECT watch_date,checked_date,code,name,close,prev_close,change_pct,limit_up,limit_up_type,streak_today,high,open FROM catalyst_watchlist_outcome WHERE watch_date=? AND checked_date=? AND code=? LIMIT 1",
            )
            .bind::<diesel::sql_types::Text, _>(&watch_date_s)
            .bind::<diesel::sql_types::Text, _>(&checked_date_s)
            .bind::<diesel::sql_types::Text, _>(&o.code)
            .get_result(c)
            .optional()?;
            if existing.is_some() {
                skipped += 1;
                continue;
            }
            diesel::sql_query(
                "INSERT INTO catalyst_watchlist_outcome(watch_date,checked_date,code,name,close,prev_close,change_pct,limit_up,limit_up_type,streak_today,high,open) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind::<diesel::sql_types::Text, _>(&watch_date_s)
            .bind::<diesel::sql_types::Text, _>(&checked_date_s)
            .bind::<diesel::sql_types::Text, _>(&o.code)
            .bind::<diesel::sql_types::Text, _>(&o.name)
            .bind::<diesel::sql_types::Double, _>(o.close)
            .bind::<diesel::sql_types::Double, _>(o.prev_close)
            .bind::<diesel::sql_types::Double, _>(o.change_pct)
            .bind::<diesel::sql_types::Integer, _>(o.limit_up as i32)
            .bind::<diesel::sql_types::Text, _>(&o.limit_up_type)
            .bind::<diesel::sql_types::BigInt, _>(o.streak_today)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(o.high)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(o.open)
            .execute(c)?;
            inserted += 1;
        }
        Ok(SaveOutcomeReceipt { inserted, skipped })
    })
    .map_err(|e: diesel::result::Error| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    /// 每个测试独立 :memory: 连接 (user_position_snapshot 同款模式),
    /// 避免共享 DatabaseManager 单例的并行写锁冲突。
    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("catalyst watchlist schema");
        conn
    }

    #[test]
    fn save_watchlist_is_idempotent() {
        let mut conn = connection();
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let leading = vec![WatchEntry {
            code: "600721".into(),
            name: "百花医药".into(),
            streak: 1,
        }];
        let other = vec![WatchEntry {
            code: "600833".into(),
            name: "第一医药".into(),
            streak: 1,
        }];
        let first = save_watchlist_with_conn(&mut conn, date, &leading, &other).unwrap();
        assert!(first.inserted);
        let second = save_watchlist_with_conn(&mut conn, date, &leading, &other).unwrap();
        assert!(!second.inserted);
        assert_eq!(first.run_id, second.run_id);
    }

    #[test]
    fn latest_watchlist_before_picks_most_recent_past_snapshot() {
        let mut conn = connection();
        let older = NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
        let newer = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        save_watchlist_with_conn(
            &mut conn,
            older,
            &[WatchEntry {
                code: "600001".into(),
                name: "老票".into(),
                streak: 2,
            }],
            &[],
        )
        .unwrap();
        save_watchlist_with_conn(
            &mut conn,
            newer,
            &[WatchEntry {
                code: "600721".into(),
                name: "百花医药".into(),
                streak: 1,
            }],
            &[WatchEntry {
                code: "600833".into(),
                name: "第一医药".into(),
                streak: 1,
            }],
        )
        .unwrap();
        let checked = NaiveDate::from_ymd_opt(2026, 8, 12).unwrap();
        let snapshot = latest_watchlist_before_with_conn(&mut conn, checked)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.watch_date, newer);
        assert_eq!(snapshot.leading.len(), 1);
        assert_eq!(snapshot.other.len(), 1);
        assert_eq!(snapshot.leading[0].code, "600721");
        assert_eq!(snapshot.leading[0].streak, 1);
        // 同一日期多次核对仍是同一名单
        let again = latest_watchlist_before_with_conn(&mut conn, checked)
            .unwrap()
            .unwrap();
        assert_eq!(again, snapshot);
    }

    #[test]
    fn no_watchlist_before_returns_none() {
        let mut conn = connection();
        let checked = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        assert!(
            latest_watchlist_before_with_conn(&mut conn, checked)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn save_outcomes_is_idempotent_per_code() {
        let mut conn = connection();
        let o = WatchOutcome {
            watch_date: NaiveDate::from_ymd_opt(2026, 8, 11).unwrap(),
            checked_date: NaiveDate::from_ymd_opt(2026, 8, 12).unwrap(),
            code: "600721".into(),
            name: "百花医药".into(),
            close: 14.02,
            prev_close: 12.75,
            change_pct: 9.96,
            limit_up: true,
            limit_up_type: "封板".into(),
            streak_today: 2,
            high: Some(14.03),
            open: Some(13.38),
        };
        let first = save_outcomes_with_conn(&mut conn, &[o.clone()]).unwrap();
        assert_eq!(first.inserted, 1);
        assert_eq!(first.skipped, 0);
        let second = save_outcomes_with_conn(&mut conn, &[o]).unwrap();
        assert_eq!(second.inserted, 0);
        assert_eq!(second.skipped, 1);
    }

    #[test]
    fn append_only_triggers_reject_update_and_delete() {
        let mut conn = connection();
        let date = NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        save_watchlist_with_conn(
            &mut conn,
            date,
            &[WatchEntry {
                code: "600721".into(),
                name: "百花医药".into(),
                streak: 1,
            }],
            &[],
        )
        .unwrap();
        let update_err = diesel::sql_query(
            "UPDATE catalyst_watchlist_daily SET name='改' WHERE code='600721'",
        )
        .execute(&mut conn);
        assert!(update_err.is_err());
        let delete_err =
            diesel::sql_query("DELETE FROM catalyst_watchlist_daily WHERE code='600721'")
                .execute(&mut conn);
        assert!(delete_err.is_err());
    }
}
