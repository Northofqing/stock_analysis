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
    pub run_id: String,
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
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    save_closing_valuation_with_conn(&mut conn, v)
}

fn save_closing_valuation_with_conn(
    conn: &mut SqliteConnection,
    v: &PortfolioValuationView,
) -> Result<SaveClosingValuationReceipt, String> {
    let rid = run_id(v);
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
    latest_persisted_valuation_view_with_conn(&mut c)
}

/// 指定价格日期的收盘估值 (BR-233 修复: R-07 做T 基准必须用 review_date
/// 对应的估值, 不能 fallback 到上一交易日 — 周六补投场景 latest 是 8/6,
/// 会把周四收盘价冒充周五价)。缺失返回 None, 调用方负责出声。
pub fn persisted_valuation_view_for_date(
    price_date: chrono::NaiveDate,
) -> Result<Option<ClosingValuationView>, String> {
    let db = crate::database::DatabaseManager::get();
    let mut c = db.get_conn().map_err(|e| e.to_string())?;
    persisted_valuation_view_for_date_conn(&mut c, price_date)
}

fn persisted_valuation_view_for_date_conn(
    c: &mut SqliteConnection,
    price_date: chrono::NaiveDate,
) -> Result<Option<ClosingValuationView>, String> {
    let row: Option<RunRow> = diesel::sql_query(
        "SELECT id,run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl FROM closing_valuation_run WHERE price_date=?1 ORDER BY id DESC LIMIT 1",
    )
    .bind::<diesel::sql_types::Text, _>(price_date.format("%Y-%m-%d").to_string())
    .get_result(c)
    .optional()
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    materialize_valuation_view(c, row)
}

fn latest_persisted_valuation_view_with_conn(
    c: &mut SqliteConnection,
) -> Result<Option<ClosingValuationView>, String> {
    let row: Option<RunRow> = diesel::sql_query("SELECT id,run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl FROM closing_valuation_run ORDER BY price_date DESC,id DESC LIMIT 1").get_result(&mut *c).optional().map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    materialize_valuation_view(c, row)
}

fn materialize_valuation_view(
    c: &mut SqliteConnection,
    row: RunRow,
) -> Result<Option<ClosingValuationView>, String> {
    let items: Vec<ClosingValuationItem> = diesel::sql_query("SELECT code,name,quantity,cost_price,close,market_value,unrealized_pnl,unrealized_return_pct,daily_price_pnl FROM closing_valuation_item WHERE run_id=? ORDER BY code").bind::<diesel::sql_types::Text,_>(&row.run_id).load::<ItemRow>(&mut *c).map_err(|e| e.to_string())?.into_iter().map(|i| ClosingValuationItem{code:i.code,name:i.name,quantity:i.quantity as u64,cost_price:i.cost_price,close:i.close,market_value:i.market_value,unrealized_pnl:i.unrealized_pnl,unrealized_return_pct:i.unrealized_return_pct,daily_price_pnl:i.daily_price_pnl}).collect();
    Ok(Some(ClosingValuationView {
        persisted_run_row_id: row.id,
        run_id: row.run_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use diesel::connection::SimpleConnection;

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("closing valuation schema");
        conn
    }

    fn item(code: &str, close: Option<f64>) -> ClosingValuationItem {
        ClosingValuationItem {
            code: code.to_owned(),
            name: format!("TEST_CODE_{code}"),
            quantity: 100,
            cost_price: 10.0,
            close,
            market_value: close.map(|value| value * 100.0),
            unrealized_pnl: close.map(|value| (value - 10.0) * 100.0),
            unrealized_return_pct: close.map(|value| (value / 10.0 - 1.0) * 100.0),
            daily_price_pnl: None,
        }
    }

    fn view(items: Vec<ClosingValuationItem>) -> PortfolioValuationView {
        PortfolioValuationView {
            price_date: NaiveDate::from_ymd_opt(2026, 7, 24).expect("date"),
            provider: "TEST_CODE_MAGIC_TDX".to_owned(),
            covered: items.iter().filter(|item| item.close.is_some()).count(),
            total: items.len(),
            total_market_value: None,
            total_unrealized_pnl: None,
            items,
        }
    }

    #[test]
    fn for_date_returns_exact_price_date_not_latest() {
        let mut conn = connection();
        let mut prior = view(vec![item("TEST_CODE_000001", Some(11.0))]);
        prior.price_date = NaiveDate::from_ymd_opt(2026, 8, 6).expect("date");
        let inserted_prior =
            save_closing_valuation_with_conn(&mut conn, &prior).expect("insert prior date");
        assert!(inserted_prior.inserted);

        let mut latest = view(vec![item("TEST_CODE_000001", Some(22.0))]);
        latest.price_date = NaiveDate::from_ymd_opt(2026, 8, 7).expect("date");
        let inserted_latest =
            save_closing_valuation_with_conn(&mut conn, &latest).expect("insert latest date");
        assert!(inserted_latest.inserted);

        // latest 是 8/7; for_date(8/6) 必须返回 8/6 而不是 latest (BR-233)
        let prior_view = persisted_valuation_view_for_date_conn(
            &mut conn,
            NaiveDate::from_ymd_opt(2026, 8, 6).expect("date"),
        )
        .expect("for_date read")
        .expect("prior persisted");
        assert_eq!(prior_view.valuation.price_date, prior.price_date);
        assert_eq!(
            prior_view
                .valuation
                .items
                .first()
                .expect("item")
                .close,
            Some(11.0)
        );

        let latest_view = persisted_valuation_view_for_date_conn(
            &mut conn,
            NaiveDate::from_ymd_opt(2026, 8, 7).expect("date"),
        )
        .expect("for_date read")
        .expect("latest persisted");
        assert_eq!(latest_view.valuation.price_date, latest.price_date);
        assert_eq!(
            latest_view
                .valuation
                .items
                .first()
                .expect("item")
                .close,
            Some(22.0)
        );

        // 不存在的日期 → None (调用方出声, 不用 latest 冒充)
        assert!(
            persisted_valuation_view_for_date_conn(
                &mut conn,
                NaiveDate::from_ymd_opt(2026, 8, 8).expect("date")
            )
            .expect("for_date read")
            .is_none()
        );
    }

    #[test]
    fn sqlite_round_trip_is_idempotent_and_append_only() {
        let mut conn = connection();
        assert!(latest_persisted_valuation_view_with_conn(&mut conn)
            .expect("empty read")
            .is_none());

        let value = view(vec![
            item("TEST_CODE_000001", Some(11.0)),
            item("TEST_CODE_600000", None),
        ]);
        let inserted =
            save_closing_valuation_with_conn(&mut conn, &value).expect("first immutable insert");
        assert!(inserted.inserted);
        let duplicate =
            save_closing_valuation_with_conn(&mut conn, &value).expect("idempotent insert");
        assert!(!duplicate.inserted);
        assert_eq!(duplicate.run_id, inserted.run_id);

        let persisted = latest_persisted_valuation_view_with_conn(&mut conn)
            .expect("latest read")
            .expect("persisted view");
        assert_eq!(persisted.run_id, inserted.run_id);
        assert_eq!(persisted.valuation.provider, "TEST_CODE_MAGIC_TDX");
        assert_eq!(persisted.valuation.items.len(), 2);
        assert_eq!(persisted.valuation.items[0].code, "TEST_CODE_000001");
        assert_eq!(persisted.valuation.items[1].close, None);

        let mutation =
            diesel::sql_query("UPDATE closing_valuation_run SET provider='TEST_CODE_TAMPERED'")
                .execute(&mut conn)
                .expect_err("append-only trigger");
        assert!(mutation.to_string().contains("append-only"));
    }

    #[test]
    fn duplicate_item_rolls_back_parent_and_invalid_stored_date_is_explicit() {
        let mut conn = connection();
        let duplicate = view(vec![
            item("TEST_CODE_000001", Some(11.0)),
            item("TEST_CODE_000001", Some(12.0)),
        ]);
        assert!(save_closing_valuation_with_conn(&mut conn, &duplicate).is_err());
        assert!(latest_persisted_valuation_view_with_conn(&mut conn)
            .expect("rollback leaves no run")
            .is_none());

        diesel::sql_query(
            "INSERT INTO closing_valuation_run(
                run_id,price_date,provider,covered,total,total_market_value,total_unrealized_pnl
             ) VALUES ('TEST_CODE_BAD_DATE','not-a-date','TEST_CODE_PROVIDER',0,0,NULL,NULL)",
        )
        .execute(&mut conn)
        .expect("malformed historical row");
        let error = latest_persisted_valuation_view_with_conn(&mut conn)
            .expect_err("invalid persisted date must fail");
        assert!(!error.is_empty());
    }
}
