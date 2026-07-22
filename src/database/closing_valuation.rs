//! BR-147: immutable persistence for validated closing valuation views.
use crate::portfolio::closing_valuation::{
    ClosingValuationItem, ClosingValuationView as PortfolioValuationView,
};
use diesel::prelude::*;
use sha2::{Digest, Sha256};

#[derive(QueryableByName)]
struct RunRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    run_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    price_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    provider: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    covered: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    total: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    total_market_value: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    total_unrealized_pnl: Option<f64>,
}
#[derive(QueryableByName)]
struct ItemRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    cost_price: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    close: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    market_value: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    unrealized_pnl: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    unrealized_return_pct: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    daily_price_pnl: Option<f64>,
}
#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    _id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveClosingValuationReceipt {
    pub run_id: String,
    pub inserted: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ClosingValuationView {
    pub persisted_run_row_id: i64,
    pub valuation: PortfolioValuationView,
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS closing_valuation_run (id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL UNIQUE, price_date TEXT NOT NULL, provider TEXT NOT NULL, covered INTEGER NOT NULL, total INTEGER NOT NULL, total_market_value REAL, total_unrealized_pnl REAL)",
        "CREATE TABLE IF NOT EXISTS closing_valuation_item (run_id TEXT NOT NULL REFERENCES closing_valuation_run(run_id), code TEXT NOT NULL, name TEXT NOT NULL, quantity INTEGER NOT NULL, cost_price REAL NOT NULL, close REAL, market_value REAL, unrealized_pnl REAL, unrealized_return_pct REAL, daily_price_pnl REAL, PRIMARY KEY(run_id,code))",
        "CREATE TRIGGER IF NOT EXISTS closing_valuation_run_no_update BEFORE UPDATE ON closing_valuation_run BEGIN SELECT RAISE(ABORT, 'closing_valuation_run is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS closing_valuation_run_no_delete BEFORE DELETE ON closing_valuation_run BEGIN SELECT RAISE(ABORT, 'closing_valuation_run is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS closing_valuation_item_no_update BEFORE UPDATE ON closing_valuation_item BEGIN SELECT RAISE(ABORT, 'closing_valuation_item is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS closing_valuation_item_no_delete BEFORE DELETE ON closing_valuation_item BEGIN SELECT RAISE(ABORT, 'closing_valuation_item is append-only'); END",
    ] { diesel::sql_query(sql).execute(conn).map_err(|e| e.to_string())?; }
    Ok(())
}

fn run_id(v: &PortfolioValuationView) -> String {
    let mut h = Sha256::new();
    h.update(b"stock_analysis.closing_valuation.v1\0");
    h.update(v.price_date.to_string().as_bytes());
    h.update(b"\0");
    h.update(v.provider.as_bytes());
    for i in &v.items {
        h.update(format!("\0{}|{}|{}|{:?}", i.code, i.quantity, i.cost_price, i.close).as_bytes());
    }
    format!("cv_v1_{:x}", h.finalize())
}

pub fn save_closing_valuation(
    v: &PortfolioValuationView,
) -> Result<SaveClosingValuationReceipt, String> {
    let rid = run_id(v);
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    conn.transaction(|c| {
        if diesel::sql_query("SELECT id AS _id FROM closing_valuation_run WHERE run_id=?").bind::<diesel::sql_types::Text,_>(&rid).get_result::<IdRow>(c).optional()?.is_some() { return Ok(SaveClosingValuationReceipt { run_id: rid, inserted: false }); }
        diesel::sql_query("INSERT INTO closing_valuation_run(run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl) VALUES (?,?,?,?,?,?,?)").bind::<diesel::sql_types::Text,_>(&rid).bind::<diesel::sql_types::Text,_>(v.price_date.to_string()).bind::<diesel::sql_types::Text,_>(&v.provider).bind::<diesel::sql_types::Integer,_>(v.covered as i32).bind::<diesel::sql_types::Integer,_>(v.total as i32).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(v.total_market_value).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(v.total_unrealized_pnl).execute(c)?;
        for i in &v.items { diesel::sql_query("INSERT INTO closing_valuation_item(run_id,code,name,quantity,cost_price,close,market_value,unrealized_pnl,unrealized_return_pct,daily_price_pnl) VALUES (?,?,?,?,?,?,?,?,?,?)").bind::<diesel::sql_types::Text,_>(&rid).bind::<diesel::sql_types::Text,_>(&i.code).bind::<diesel::sql_types::Text,_>(&i.name).bind::<diesel::sql_types::BigInt,_>(i.quantity as i64).bind::<diesel::sql_types::Double,_>(i.cost_price).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.close).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.market_value).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.unrealized_pnl).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.unrealized_return_pct).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.daily_price_pnl).execute(c)?; }
        Ok(SaveClosingValuationReceipt { run_id: rid, inserted: true })
    }).map_err(|e: diesel::result::Error| e.to_string())
}

pub fn latest_persisted_valuation_view() -> Result<Option<ClosingValuationView>, String> {
    let db = crate::database::DatabaseManager::get();
    let mut c = db.get_conn().map_err(|e| e.to_string())?;
    let row: Option<RunRow> = diesel::sql_query("SELECT id,run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl FROM closing_valuation_run ORDER BY price_date DESC,id DESC LIMIT 1").get_result(&mut c).optional().map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let items: Vec<ClosingValuationItem> = diesel::sql_query("SELECT code,name,quantity,cost_price,close,market_value,unrealized_pnl,unrealized_return_pct,daily_price_pnl FROM closing_valuation_item WHERE run_id=? ORDER BY code").bind::<diesel::sql_types::Text,_>(&row.run_id).load::<ItemRow>(&mut c).map_err(|e| e.to_string())?.into_iter().map(|i| ClosingValuationItem{code:i.code,name:i.name,quantity:i.quantity as u64,cost_price:i.cost_price,close:i.close,market_value:i.market_value,unrealized_pnl:i.unrealized_pnl,unrealized_return_pct:i.unrealized_return_pct,daily_price_pnl:i.daily_price_pnl}).collect();
    Ok(Some(ClosingValuationView {
        persisted_run_row_id: row.id,
        valuation: PortfolioValuationView {
            price_date: chrono::NaiveDate::parse_from_str(&row.price_date, "%Y-%m-%d")
                .map_err(|e| e.to_string())?,
            provider: row.provider,
            covered: row.covered as usize,
            total: row.total as usize,
            items,
            total_market_value: row.total_market_value,
            total_unrealized_pnl: row.total_unrealized_pnl,
        },
    }))
}
