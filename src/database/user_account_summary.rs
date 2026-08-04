//! BR-146: user-confirmed account summary, separate from real-account facts.
use diesel::connection::SimpleConnection;
use diesel::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct UserAccountSummary {
    pub effective_at: String,
    pub total_assets: f64,
    pub securities_market_value: f64,
    pub available_cash: f64,
    pub position_ratio_pct: f64,
    pub daily_pnl: f64,
    pub source: String,
}

#[derive(QueryableByName)]
struct SummaryRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    effective_at: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    total_assets: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    securities_market_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    available_cash: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    position_ratio_pct: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    daily_pnl: f64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source: String,
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS user_account_summary (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            effective_at TEXT NOT NULL,
            total_assets REAL NOT NULL CHECK(total_assets > 0),
            securities_market_value REAL NOT NULL CHECK(securities_market_value >= 0),
            available_cash REAL NOT NULL CHECK(available_cash >= 0),
            position_ratio_pct REAL NOT NULL CHECK(
                position_ratio_pct >= 0 AND position_ratio_pct <= 100
            ),
            daily_pnl REAL NOT NULL,
            source TEXT NOT NULL,
            recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TRIGGER IF NOT EXISTS user_account_summary_no_update
         BEFORE UPDATE ON user_account_summary
         BEGIN
             SELECT RAISE(ABORT, 'user_account_summary is append-only');
         END;
         CREATE TRIGGER IF NOT EXISTS user_account_summary_no_delete
         BEFORE DELETE ON user_account_summary
         BEGIN
             SELECT RAISE(ABORT, 'user_account_summary is append-only');
         END;",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save(summary: &UserAccountSummary) -> Result<(), String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| e.to_string())?;
    save_with_conn(&mut conn, summary)
}

fn save_with_conn(conn: &mut SqliteConnection, summary: &UserAccountSummary) -> Result<(), String> {
    diesel::sql_query("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source) VALUES (?,?,?,?,?,?,?)")
        .bind::<diesel::sql_types::Text,_>(&summary.effective_at)
        .bind::<diesel::sql_types::Double,_>(summary.total_assets)
        .bind::<diesel::sql_types::Double,_>(summary.securities_market_value)
        .bind::<diesel::sql_types::Double,_>(summary.available_cash)
        .bind::<diesel::sql_types::Double,_>(summary.position_ratio_pct)
        .bind::<diesel::sql_types::Double,_>(summary.daily_pnl)
        .bind::<diesel::sql_types::Text,_>(&summary.source).execute(&mut *conn).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn latest() -> Result<Option<UserAccountSummary>, String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| e.to_string())?;
    latest_with_conn(&mut conn)
}

fn latest_with_conn(conn: &mut SqliteConnection) -> Result<Option<UserAccountSummary>, String> {
    let row: Option<SummaryRow> = diesel::sql_query("SELECT effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source FROM user_account_summary ORDER BY effective_at DESC,id DESC LIMIT 1").get_result(&mut *conn).optional().map_err(|e| e.to_string())?;
    Ok(row.map(|r| UserAccountSummary {
        effective_at: r.effective_at,
        total_assets: r.total_assets,
        securities_market_value: r.securities_market_value,
        available_cash: r.available_cash,
        position_ratio_pct: r.position_ratio_pct,
        daily_pnl: r.daily_pnl,
        source: r.source,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        create_schema(&mut conn).expect("account summary schema");
        conn
    }

    fn summary(effective_at: &str, daily_pnl: f64) -> UserAccountSummary {
        UserAccountSummary {
            effective_at: effective_at.to_owned(),
            total_assets: 100_000.0,
            securities_market_value: 70_000.0,
            available_cash: 30_000.0,
            position_ratio_pct: 70.0,
            daily_pnl,
            source: "TEST_CODE_USER_CONFIRMED".to_owned(),
        }
    }

    #[test]
    fn sqlite_latest_round_trip_and_append_only_trigger() {
        let mut conn = connection();
        assert!(latest_with_conn(&mut conn).expect("empty read").is_none());
        save_with_conn(&mut conn, &summary("2026-07-24T15:00:00+08:00", -10.0))
            .expect("older summary");
        save_with_conn(&mut conn, &summary("2026-07-25T15:00:00+08:00", 20.0))
            .expect("newer summary");
        let latest = latest_with_conn(&mut conn)
            .expect("latest read")
            .expect("persisted summary");
        assert_eq!(latest.effective_at, "2026-07-25T15:00:00+08:00");
        assert_eq!(latest.daily_pnl, 20.0);
        assert_eq!(latest.source, "TEST_CODE_USER_CONFIRMED");

        let mutation =
            diesel::sql_query("UPDATE user_account_summary SET total_assets=1 WHERE id=1")
                .execute(&mut conn)
                .expect_err("append-only trigger");
        assert!(mutation.to_string().contains("append-only"));
    }

    #[test]
    fn sqlite_constraints_reject_invalid_account_values_without_partial_rows() {
        for (field, value) in [
            ("total_assets", -1.0),
            ("securities_market_value", -1.0),
            ("available_cash", -1.0),
            ("position_ratio_pct", 101.0),
        ] {
            let mut conn = connection();
            let mut invalid = summary("2026-07-24T15:00:00+08:00", 0.0);
            match field {
                "total_assets" => invalid.total_assets = value,
                "securities_market_value" => invalid.securities_market_value = value,
                "available_cash" => invalid.available_cash = value,
                "position_ratio_pct" => invalid.position_ratio_pct = value,
                _ => unreachable!(),
            }
            assert!(save_with_conn(&mut conn, &invalid).is_err(), "{field}");
            assert!(latest_with_conn(&mut conn)
                .expect("constraint rollback")
                .is_none());
        }
    }
}
