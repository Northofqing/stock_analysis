//! One-shot BR-103 importer for an ignored, user-attested evidence manifest.

use std::path::PathBuf;

use clap::Parser;
use stock_analysis::database::account_snapshot::{
    account_snapshot_input_from_json, save_account_snapshot,
};
use stock_analysis::database::DatabaseManager;

#[derive(Parser)]
#[command(about = "Import one validated real-account snapshot without printing account values")]
struct Args {
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    evidence: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let metadata = std::fs::metadata(&args.evidence)?;
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return Err("BR-103 evidence must be a regular JSON file no larger than 1 MiB".into());
    }
    let json = std::fs::read_to_string(&args.evidence)?;
    let input = account_snapshot_input_from_json(&json)?;
    DatabaseManager::init(Some(args.database))?;
    let receipt = save_account_snapshot(&input)?;
    println!(
        "account_snapshot_id={} inserted={} daily_pnl_is_null={}",
        receipt.id,
        receipt.inserted,
        input.daily_pnl.is_none()
    );
    Ok(())
}
