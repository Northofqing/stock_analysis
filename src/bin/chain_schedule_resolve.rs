//! Inspect or resolve an uncertain scheduled chain-report send after checking
//! the external channel logs. This command never sends a notification.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use stock_analysis::app::chain_schedule::{ChainPhase, ChainScheduleStore, ManualResolution};

#[derive(Parser)]
struct Args {
    #[arg(long, value_parser = ["preopen", "postclose"])]
    phase: String,
    #[arg(long)]
    date: String,
    #[arg(long, value_parser = ["delivered", "retry"])]
    resolution: Option<String>,
    #[arg(long)]
    note: Option<String>,
    #[arg(long)]
    database: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let phase = match args.phase.as_str() {
        "preopen" => ChainPhase::Preopen,
        "postclose" => ChainPhase::Postclose,
        _ => unreachable!("clap phase validation"),
    };
    let date =
        NaiveDate::parse_from_str(&args.date, "%Y-%m-%d").context("--date 必须是 YYYY-MM-DD")?;
    let store = args
        .database
        .map_or_else(ChainScheduleStore::production, ChainScheduleStore::new);
    if let Some(resolution) = args.resolution {
        let note = args
            .note
            .as_deref()
            .context("裁定时必须提供 --note 核对依据")?;
        let resolution = match resolution.as_str() {
            "delivered" => ManualResolution::Delivered,
            "retry" => ManualResolution::Retry,
            _ => unreachable!("clap resolution validation"),
        };
        store.resolve_uncertain(phase, date, resolution, note)?;
    } else if args.note.is_some() {
        bail!("--note 仅可与 --resolution 一起使用");
    }
    let (status, attempt) = store.inspect(phase, date)?;
    println!(
        "phase={} date={} status={status:?} database={}",
        phase.as_str(),
        date,
        store.path().display()
    );
    if let Some(attempt) = attempt {
        println!(
            "attempt={} state={} report={} created_at={} updated_at={} note={}",
            attempt.attempt_no,
            attempt.state,
            attempt.report_path,
            attempt.created_at,
            attempt.updated_at,
            attempt.resolution_note.as_deref().unwrap_or("")
        );
    }
    Ok(())
}
