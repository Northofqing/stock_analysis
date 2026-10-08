//! Local H16 descriptive review. Reads a detached snapshot; never initializes,
//! backfills, sends, seeds, or changes strategy configuration.
use chrono::{DateTime, FixedOffset, NaiveDate};
use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession,
};

#[path = "weekly_outcome_review/report.rs"]
mod report;

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Markdown,
    Json,
}

#[derive(Parser)]
#[command(
    about = "Generate a local descriptive weekly review from an existing read-only database snapshot"
)]
struct Args {
    /// Existing stable SQLite snapshot. For a live WAL source use the backup wrapper.
    #[arg(long)]
    database: PathBuf,
    /// Original source label supplied by the backup wrapper/operator, not qualification.
    #[arg(long)]
    source_label: Option<String>,
    /// Mark a private input that will be deleted after this process exits.
    #[arg(long)]
    temporary_snapshot: bool,
    #[arg(long, value_parser = canonical_date)]
    from: NaiveDate,
    #[arg(long, value_parser = canonical_date)]
    to: NaiveDate,
    /// RFC3339 Shanghai +08:00 clock; defaults to the current Shanghai clock.
    #[arg(long, value_parser = shanghai_clock)]
    observed_at: Option<DateTime<FixedOffset>>,
    #[arg(long, value_enum, default_value = "markdown")]
    format: Format,
    /// Create a new report file; an existing file is never overwritten.
    #[arg(long)]
    output: Option<PathBuf>,
}

fn canonical_date(value: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|e| e.to_string())?;
    if date.to_string() != value {
        return Err("date must be YYYY-MM-DD".into());
    }
    Ok(date)
}

fn shanghai_clock(value: &str) -> Result<DateTime<FixedOffset>, String> {
    let now = DateTime::parse_from_rfc3339(value).map_err(|e| e.to_string())?;
    if now.offset().local_minus_utc() != 8 * 3600 {
        return Err("observed-at must use Shanghai +08:00".into());
    }
    Ok(now)
}

fn run(args: Args) -> anyhow::Result<()> {
    let observed_at = args
        .observed_at
        .unwrap_or_else(stock_analysis::monitor::prediction::shanghai_now);
    let period =
        report::Period::new(args.from, args.to, observed_at).map_err(anyhow::Error::msg)?;
    // This API copies existing bytes into an immutable private read-only pool.
    // It does not run DatabaseManager::init or any application DDL.
    let before = file_sha256(&args.database)?;
    let session =
        AttributionDatabaseSession::open(&args.database, AttributionDatabaseAccess::ReadOnly)?;
    let mut review = report::read(session.database(), period);
    anyhow::ensure!(
        before == file_sha256(&args.database)?,
        "source main changed during report read"
    );
    review.input_source = Some(report::InputSource {
        database_path: args.database.display().to_string(),
        source_main_sha256: before,
        original_source_label: args.source_label,
        temporary_snapshot_deleted_after_run: args.temporary_snapshot,
        boundary: "AttributionDatabaseSession::ReadOnly detached snapshot; no initialization or application writes",
    });
    let mut rendered = match args.format {
        Format::Markdown => review.markdown(),
        Format::Json => serde_json::to_string_pretty(&review)?,
    };
    rendered.push('\n');
    if let Some(output) = args.output {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(output)?;
        file.write_all(rendered.as_bytes())?;
    } else {
        print!("{rendered}");
    }
    Ok(())
}

fn file_sha256(path: &std::path::Path) -> anyhow::Result<String> {
    use std::io::Read;
    let mut source = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("weekly_outcome_review_failed: {error}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
#[path = "weekly_outcome_review/tests.rs"]
mod tests;
