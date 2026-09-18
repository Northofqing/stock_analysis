//! P0 死信哨兵 (2026-09-03): watchdog_deadline — 「预期推送没出现 → 主动出声」。
//!
//! 每个交易日三条轨道注册 deadline (news_first_wave 09:45 / attribution_1505
//! 15:20 / review_evening 19:40)。轨道自身完成一轮即 satisfy — 宽口径 (决策
//! D3): 有推送尝试 (attempted>0, 含治理拒绝) 即算活; 逾期 (expected_before)
//! 仍未满足 → 死信触发一次 PushKind::Watchdog, (family, business_date) 幂等。
//!
//! 时间戳全部为本地 (Asia/Shanghai) naive 字符串 "%Y-%m-%dT%H:%M:%S" —
//! 同进程内写入/比较固定格式, 字典序即时间序。跨进程同 TZ, 契约见下方列名。

use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sql_types::Text;

use super::DbConnection;

/// 到期待触发: expected_before <= now 且未满足未触发。
#[derive(QueryableByName, Debug, PartialEq, Eq)]
struct DeadlineRow {
    #[diesel(sql_type = Text)]
    family: String,
    #[diesel(sql_type = Text)]
    business_date: String,
    #[diesel(sql_type = Text)]
    expected_before: String,
}

pub fn create_schema(conn: &mut diesel::SqliteConnection) -> Result<(), Box<dyn std::error::Error>> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS watchdog_deadline (
            family TEXT NOT NULL,
            business_date TEXT NOT NULL,
            expected_before TEXT NOT NULL,
            satisfied_at TEXT,
            fired_at TEXT,
            PRIMARY KEY (family, business_date)
        )",
    )
    .execute(conn)?;
    Ok(())
}

fn local_ts(now: &NaiveDateTime) -> String {
    now.format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 幂等注册。返回 true = 新行插入; false = 已存在 (含已满足/已触发, 不动)。
pub fn register_deadline(
    conn: &mut DbConnection,
    family: &str,
    business_date: &str,
    expected_before: &NaiveDateTime,
) -> Result<bool, Box<dyn std::error::Error>> {
    let inserted = diesel::sql_query(
        "INSERT OR IGNORE INTO watchdog_deadline (family, business_date, expected_before)
         VALUES (?, ?, ?)",
    )
    .bind::<Text, _>(family)
    .bind::<Text, _>(business_date)
    .bind::<Text, _>(local_ts(expected_before))
    .execute(conn)?;
    Ok(inserted > 0)
}

/// 宽口径满足: 该 (family, date) 有推送尝试即满足。已触发的行不复活。
/// 返回 true = 状态变更; false = 无行 / 已满足 / 已触发。
pub fn satisfy_deadline(
    conn: &mut DbConnection,
    family: &str,
    business_date: &str,
    now: &NaiveDateTime,
) -> Result<bool, Box<dyn std::error::Error>> {
    let changed = diesel::sql_query(
        "UPDATE watchdog_deadline SET satisfied_at = ?
         WHERE family = ? AND business_date = ?
           AND satisfied_at IS NULL AND fired_at IS NULL",
    )
    .bind::<Text, _>(local_ts(now))
    .bind::<Text, _>(family)
    .bind::<Text, _>(business_date)
    .execute(conn)?;
    Ok(changed > 0)
}

/// 逾期且未满足未触发 → (family, business_date, expected_before)。
pub fn due_deadlines(
    conn: &mut DbConnection,
    now: &NaiveDateTime,
) -> Result<Vec<(String, String, String)>, Box<dyn std::error::Error>> {
    let rows: Vec<DeadlineRow> = diesel::sql_query(
        "SELECT family, business_date, expected_before FROM watchdog_deadline
         WHERE expected_before <= ? AND satisfied_at IS NULL AND fired_at IS NULL
         ORDER BY expected_before",
    )
    .bind::<Text, _>(local_ts(now))
    .load(conn)?;
    Ok(rows
        .into_iter()
        .map(|row| (row.family, row.business_date, row.expected_before))
        .collect())
}

/// 触发后记账 — (family, date) 幂等, 不复活。
pub fn mark_fired(
    conn: &mut DbConnection,
    family: &str,
    business_date: &str,
    now: &NaiveDateTime,
) -> Result<bool, Box<dyn std::error::Error>> {
    let changed = diesel::sql_query(
        "UPDATE watchdog_deadline SET fired_at = ?
         WHERE family = ? AND business_date = ? AND fired_at IS NULL",
    )
    .bind::<Text, _>(local_ts(now))
    .bind::<Text, _>(family)
    .bind::<Text, _>(business_date)
    .execute(conn)?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn fixture() -> (std::path::PathBuf, DbConnection) {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("TEST_CODE clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "TEST_CODE_watchdog_deadline_{}_{}.sqlite",
            std::process::id(),
            nonce
        ));
        let url = path.to_string_lossy().into_owned();
        let pool = super::super::build_sqlite_pool_with_size(url, 1)
            .expect("TEST_CODE isolated watchdog pool");
        let mut conn = pool.get().expect("TEST_CODE watchdog connection");
        create_schema(&mut conn).expect("TEST_CODE watchdog schema");
        (path, conn)
    }

    fn at(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 3)
            .expect("TEST_CODE date")
            .and_hms_opt(hour, minute, 0)
            .expect("TEST_CODE time")
    }

    #[test]
    fn register_is_idempotent_and_due_is_driven_by_expected_before() {
        let (_path, mut conn) = fixture();
        assert!(register_deadline(&mut conn, "review_evening", "2026-09-03", &at(19, 40))
            .expect("first register"));
        assert!(!register_deadline(&mut conn, "review_evening", "2026-09-03", &at(19, 40))
            .expect("second register must be idempotent"));
        assert!(due_deadlines(&mut conn, &at(19, 30)).expect("due before")
            .is_empty(), "未到期不触发");
        let due = due_deadlines(&mut conn, &at(19, 41)).expect("due after");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0], ("review_evening".to_owned(), "2026-09-03".to_owned(), "2026-09-03T19:40:00".to_owned()));
    }

    #[test]
    fn satisfy_is_wide_and_fired_rows_never_resurrect() {
        let (_path, mut conn) = fixture();
        register_deadline(&mut conn, "review_evening", "2026-09-03", &at(19, 40))
            .expect("register");
        assert!(satisfy_deadline(&mut conn, "review_evening", "2026-09-03", &at(19, 10))
            .expect("satisfy"));
        assert!(!satisfy_deadline(&mut conn, "review_evening", "2026-09-03", &at(19, 11))
            .expect("double satisfy no-op"));
        assert!(due_deadlines(&mut conn, &at(20, 0)).expect("due after satisfy").is_empty(),
            "已满足不触发");

        // 触发后再满足: 不复活, 幂等。
        register_deadline(&mut conn, "news_first_wave", "2026-09-03", &at(9, 45))
            .expect("register news");
        assert!(mark_fired(&mut conn, "news_first_wave", "2026-09-03", &at(9, 46))
            .expect("fire"));
        assert!(!satisfy_deadline(&mut conn, "news_first_wave", "2026-09-03", &at(9, 47))
            .expect("fired row stays fired"));
        assert!(due_deadlines(&mut conn, &at(10, 0)).expect("due after fire").is_empty());
        assert!(!mark_fired(&mut conn, "news_first_wave", "2026-09-03", &at(9, 50))
            .expect("double fire no-op"));
    }

    #[test]
    fn families_and_dates_are_independent() {
        let (_path, mut conn) = fixture();
        register_deadline(&mut conn, "news_first_wave", "2026-09-03", &at(9, 45))
            .expect("news register");
        register_deadline(&mut conn, "attribution_1505", "2026-09-03", &at(15, 20))
            .expect("attribution register");
        satisfy_deadline(&mut conn, "news_first_wave", "2026-09-03", &at(9, 32))
            .expect("news satisfy");
        // 今日 16:00: 仅 attribution 逾期 (news 已满足)。
        let due = due_deadlines(&mut conn, &at(16, 0)).expect("due query");
        assert_eq!(due.len(), 1, "news 已满足不触发");
        assert_eq!(due[0].0, "attribution_1505");
        assert_eq!(due[0].1, "2026-09-03");
        // 次日注册 news 09-04: 次日 10:00 两行都到期 (字典序: 09-03 在前)。
        let next_day_09_45 = NaiveDate::from_ymd_opt(2026, 9, 4)
            .expect("TEST_CODE date")
            .and_hms_opt(9, 45, 0)
            .expect("TEST_CODE time");
        register_deadline(&mut conn, "news_first_wave", "2026-09-04", &next_day_09_45)
            .expect("next day register");
        let next_day_10 = NaiveDate::from_ymd_opt(2026, 9, 4)
            .expect("TEST_CODE date")
            .and_hms_opt(10, 0, 0)
            .expect("TEST_CODE time");
        let due = due_deadlines(&mut conn, &next_day_10).expect("due query next day");
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].0, "attribution_1505");
        assert_eq!(due[1].0, "news_first_wave");
        assert_eq!(due[1].1, "2026-09-04");
    }
}
