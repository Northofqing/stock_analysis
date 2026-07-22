use crate::portfolio::closing_valuation::{
    ClosingValuationItem, ClosingValuationView as PortfolioValuationView,
};
use diesel::prelude::*;
use sha2::{Digest, Sha256};

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
        if let Some((_,)) = diesel::sql_query("SELECT id FROM closing_valuation_run WHERE run_id=?").bind::<diesel::sql_types::Text,_>(&rid).get_result::<(i64,)>(c).optional().map_err(|e| e.to_string())? { return Ok(SaveClosingValuationReceipt { run_id: rid, inserted: false }); }
        diesel::sql_query("INSERT INTO closing_valuation_run(run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl) VALUES (?,?,?,?,?,?,?)").bind::<diesel::sql_types::Text,_>(&rid).bind::<diesel::sql_types::Text,_>(v.price_date.to_string()).bind::<diesel::sql_types::Text,_>(&v.provider).bind::<diesel::sql_types::Integer,_>(v.covered as i32).bind::<diesel::sql_types::Integer,_>(v.total as i32).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(v.total_market_value).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(v.total_unrealized_pnl).execute(c).map_err(|e| e.to_string())?;
        for i in &v.items { diesel::sql_query("INSERT INTO closing_valuation_item(run_id,code,name,quantity,cost_price,close,market_value,unrealized_pnl,unrealized_return_pct,daily_price_pnl) VALUES (?,?,?,?,?,?,?,?,?,?)").bind::<diesel::sql_types::Text,_>(&rid).bind::<diesel::sql_types::Text,_>(&i.code).bind::<diesel::sql_types::Text,_>(&i.name).bind::<diesel::sql_types::BigInt,_>(i.quantity as i64).bind::<diesel::sql_types::Double,_>(i.cost_price).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.close).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.market_value).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.unrealized_pnl).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.unrealized_return_pct).bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>,_>(i.daily_price_pnl).execute(c).map_err(|e| e.to_string())?; }
        Ok(SaveClosingValuationReceipt { run_id: rid, inserted: true })
    })
}

pub fn latest_persisted_valuation_view() -> Result<Option<ClosingValuationView>, String> {
    let db = crate::database::DatabaseManager::get();
    let mut c = db.get_conn().map_err(|e| e.to_string())?;
    let row: Option<(i64,String,String,String,i32,i32,Option<f64>,Option<f64>)> = diesel::sql_query("SELECT id,run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl FROM closing_valuation_run ORDER BY price_date DESC,id DESC LIMIT 1").get_result(&mut c).optional().map_err(|e| e.to_string())?;
    let Some((id, rid, date, provider, covered, total, mv, pnl)) = row else {
        return Ok(None);
    };
    let items: Vec<ClosingValuationItem> = diesel::sql_query("SELECT code,name,quantity,cost_price,close,market_value,unrealized_pnl,unrealized_return_pct,daily_price_pnl FROM closing_valuation_item WHERE run_id=? ORDER BY code").bind::<diesel::sql_types::Text,_>(&rid).load::<(String,String,i64,f64,Option<f64>,Option<f64>,Option<f64>,Option<f64>,Option<f64>)>(&mut c).map_err(|e| e.to_string())?.into_iter().map(|(code,name,q,cost,close,mv,pnl,ret,dp)| ClosingValuationItem{code,name,quantity:q as u64,cost_price:cost,close,market_value:mv,unrealized_pnl:pnl,unrealized_return_pct:ret,daily_price_pnl:dp}).collect();
    Ok(Some(ClosingValuationViewPersisted {
        persisted_run_row_id: id,
        valuation: ClosingValuationView {
            price_date: chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map_err(|e| e.to_string())?,
            provider,
            covered: covered as usize,
            total: total as usize,
            items,
            total_market_value: mv,
            total_unrealized_pnl: pnl,
        },
    }))
}
