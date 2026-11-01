//! Operator-approved formal repair for one missing historical paper close.
//!
//! Default is dry-run: prints the codes/prices/sources and writes nothing.
//! `--apply` appends one `Mark(closing=true)` fact with `repair=true` evidence.
//!
//!     cd <runtime-root>
//!     <repo>/target/release/paper_close_repair --database data/stock_analysis.db \
//!       --date 2026-10-30 [--apply]

use std::path::PathBuf;

use chrono::NaiveDate;
use serde_json::json;
use stock_analysis::database::DatabaseManager;
use stock_analysis::trading::paper_ledger_runtime::repair_closing_valuation_from_stock_daily;

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
    let apply = args.iter().any(|arg| arg == "--apply");

    std::env::set_var("DATABASE_PATH", &database);
    // Must match the production monitor mode; plain `init` opens the broader
    // test schema and can migrate the production store.
    DatabaseManager::init_retained_monitor(Some(PathBuf::from(database)))?;
    let (preview, receipt) = repair_closing_valuation_from_stock_daily(date, apply)?;
    let codes: Vec<_> = preview
        .codes
        .iter()
        .map(|(code, close, source)| json!({"code": code, "close": close, "source": source}))
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "dry_run": !apply,
            "price_date": preview.price_date.to_string(),
            "as_of": preview.as_of.to_rfc3339(),
            "already_closed": preview.already_closed,
            "codes": codes,
            "receipt": receipt.map(|value| json!({
                "version": value.version,
                "event_hash": value.event_hash,
                "already_applied": value.already_applied,
                "status": format!("{:?}", value.status),
            })),
        }))?
    );
    Ok(())
}
