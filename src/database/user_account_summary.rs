//! BR-146: user-confirmed account summary, separate from real-account facts.
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
    diesel::sql_query("CREATE TABLE IF NOT EXISTS user_account_summary (id INTEGER PRIMARY KEY AUTOINCREMENT, effective_at TEXT NOT NULL, total_assets REAL NOT NULL CHECK(total_assets > 0), securities_market_value REAL NOT NULL CHECK(securities_market_value >= 0), available_cash REAL NOT NULL CHECK(available_cash >= 0), position_ratio_pct REAL NOT NULL CHECK(position_ratio_pct >= 0 AND position_ratio_pct <= 100), daily_pnl REAL NOT NULL, source TEXT NOT NULL, recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)").execute(conn).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save(summary: &UserAccountSummary) -> Result<(), String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| e.to_string())?;
    diesel::sql_query("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source) VALUES (?,?,?,?,?,?,?)")
        .bind::<diesel::sql_types::Text,_>(&summary.effective_at)
        .bind::<diesel::sql_types::Double,_>(summary.total_assets)
        .bind::<diesel::sql_types::Double,_>(summary.securities_market_value)
        .bind::<diesel::sql_types::Double,_>(summary.available_cash)
        .bind::<diesel::sql_types::Double,_>(summary.position_ratio_pct)
        .bind::<diesel::sql_types::Double,_>(summary.daily_pnl)
        .bind::<diesel::sql_types::Text,_>(&summary.source).execute(&mut conn).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn latest() -> Result<Option<UserAccountSummary>, String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| e.to_string())?;
    let row: Option<SummaryRow> = diesel::sql_query("SELECT effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source FROM user_account_summary ORDER BY effective_at DESC,id DESC LIMIT 1").get_result(&mut conn).optional().map_err(|e| e.to_string())?;
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
