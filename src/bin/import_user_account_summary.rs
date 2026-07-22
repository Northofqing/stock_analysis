//! BR-146: import a user-confirmed account summary, never a broker snapshot.
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use stock_analysis::database::{self, DatabaseManager};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    summary: PathBuf,
}
#[derive(Deserialize)]
struct Raw {
    effective_at: String,
    total_assets: f64,
    securities_market_value: f64,
    available_cash: f64,
    position_ratio_pct: f64,
    daily_pnl: f64,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let raw: Raw = serde_json::from_str(&std::fs::read_to_string(args.summary)?)?;
    let summary = database::user_account_summary::UserAccountSummary {
        effective_at: raw.effective_at,
        total_assets: raw.total_assets,
        securities_market_value: raw.securities_market_value,
        available_cash: raw.available_cash,
        position_ratio_pct: raw.position_ratio_pct,
        daily_pnl: raw.daily_pnl,
        source: "user_confirmed_screenshot".into(),
    };
    DatabaseManager::init(Some(args.database))?;
    database::user_account_summary::save(&summary)?;
    println!(
        "user_confirmed_account_summary saved effective_at={} source={}",
        summary.effective_at, summary.source
    );
    Ok(())
}
