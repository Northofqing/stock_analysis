//! One-shot operator apply for a prebuilt snapshot activation request.
//! Preview first, then apply the exact prepared request. Never creates fills.

use clap::Parser;
use std::path::PathBuf;
use stock_analysis::trading::paper_snapshot_activation::{
    apply_snapshot_paper_activation, open_snapshot_activation_database,
    preview_snapshot_paper_activation, SnapshotPaperActivationRequest,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    request: PathBuf,
    #[arg(long)]
    preview_only: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.request)?;
    let request: SnapshotPaperActivationRequest = serde_json::from_str(&raw)?;
    let mut conn = open_snapshot_activation_database(&args.database, true)?;
    let now = chrono::Utc::now();
    let preview = preview_snapshot_paper_activation(&mut conn, &request, now)?;
    println!(
        "preview mode={} binding={} source_hash={}",
        preview.mode,
        serde_json::to_string(&preview.binding)?,
        preview.prepared_request.seed.source_hash
    );
    if args.preview_only {
        println!(
            "prepared_request={}",
            serde_json::to_string(&preview.prepared_request)?
        );
        return Ok(());
    }
    let prepared = preview.prepared_request;
    let outcome = apply_snapshot_paper_activation(&mut conn, &prepared, chrono::Utc::now())?;
    println!(
        "apply mode={} applied={} already_applied={} binding={} seed_receipt={} closing_receipt={}",
        outcome.mode,
        outcome.applied,
        outcome.already_applied,
        serde_json::to_string(&outcome.binding)?,
        outcome.seed_receipt.is_some(),
        outcome.closing_receipt.is_some()
    );
    println!(
        "projection cash={:?} lots={} seed_equity={:?}",
        outcome.projection.cash,
        outcome.projection.lots.len(),
        outcome.projection.seed_equity
    );
    Ok(())
}
