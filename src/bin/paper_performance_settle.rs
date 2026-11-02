//! One-shot derived performance snapshot for a specific paper ledger date.
//! This appends only `DerivedSnapshotV1`; it never changes cash, lots, fills,
//! or historical marks.

use std::path::PathBuf;

use chrono::NaiveDate;
use stock_analysis::database::DatabaseManager;
use stock_analysis::performance::compute_snapshot;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().collect();
    let database = arg_value(&args, "--database").ok_or("缺少 --database")?;
    let date_text = arg_value(&args, "--date").ok_or("缺少 --date YYYY-MM-DD")?;
    let date = NaiveDate::parse_from_str(&date_text, "%Y-%m-%d")?;

    std::env::set_var("DATABASE_PATH", &database);
    DatabaseManager::init_retained_monitor(Some(PathBuf::from(database)))?;
    let snapshot = compute_snapshot(date)?;
    println!(
        "derived_snapshot date={} id={} total_trades={} total_pnl={} win_rate={:?} created_at={}",
        snapshot.date,
        snapshot.id,
        snapshot.total_trades,
        snapshot.total_pnl,
        snapshot.win_rate,
        snapshot.created_at
    );
    Ok(())
}
