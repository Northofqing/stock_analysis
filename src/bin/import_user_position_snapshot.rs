//! One-shot importer for a user-confirmed complete position snapshot.
use clap::Parser;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use stock_analysis::database::{self, DatabaseManager};
use stock_analysis::portfolio::user_position_snapshot::user_position_snapshot_input_from_json;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    snapshot: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let metadata = std::fs::metadata(&args.snapshot)?;
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return Err("snapshot must be a regular UTF-8 JSON file no larger than 1 MiB".into());
    }
    let json = std::fs::read_to_string(&args.snapshot)?;
    let confirmed_at = chrono::Local::now().fixed_offset();
    let input = user_position_snapshot_input_from_json(&json, confirmed_at)?;
    DatabaseManager::init(Some(args.database))?;
    let receipt = database::user_position_snapshot::save_user_position_snapshot(&input)?;
    let mut receipt_hash = Sha256::new();
    receipt_hash.update(b"stock_analysis.import_user_position_snapshot.receipt.v1\0");
    receipt_hash.update(input.evidence_sha256.as_bytes());
    println!(
        "snapshot_id_hash={:x} inserted={} item_count={}",
        receipt_hash.finalize(),
        receipt.inserted,
        input.items.len()
    );
    Ok(())
}
