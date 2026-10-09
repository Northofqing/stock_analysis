//! Local H16 descriptive review. Reads a detached snapshot; never initializes,
//! backfills, sends, seeds, or changes strategy configuration.
use chrono::{DateTime, FixedOffset, NaiveDate};
use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession,
};

#[path = "weekly_outcome_review/paper_account.rs"]
mod paper_account;
#[path = "weekly_outcome_review/registry.rs"]
mod registry;
#[path = "weekly_outcome_review/report.rs"]
mod report;
#[path = "weekly_outcome_review/scorecard.rs"]
mod scorecard;

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
    /// Descriptive registry; default is the build's embedded config/signal_registry.toml.
    #[arg(long)]
    registry: Option<PathBuf>,
    /// Sidecar manifest path; defaults to OUTPUT with extension evidence.json.
    /// An identical existing manifest may be reused for the other report format.
    #[arg(long)]
    evidence_manifest: Option<PathBuf>,
    /// Original source label supplied by the backup wrapper/operator, not qualification.
    #[arg(long)]
    source_label: Option<String>,
    /// Private wrapper provenance binding this detached snapshot to its original physical target.
    #[arg(long)]
    snapshot_source_manifest: Option<PathBuf>,
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
    let registry = registry::RegistryInput::load(args.registry.as_deref())?;
    let observed_at = args
        .observed_at
        .unwrap_or_else(stock_analysis::monitor::prediction::shanghai_now);
    let period =
        report::Period::new(args.from, args.to, observed_at).map_err(anyhow::Error::msg)?;
    // This API copies existing bytes into an immutable private read-only pool.
    // It does not run DatabaseManager::init or any application DDL.
    ensure_normalized_snapshot(&args.database)?;
    let before = file_sha256(&args.database)?;
    let source = args
        .snapshot_source_manifest
        .as_ref()
        .map(|path| paper_account::SnapshotSource::load(path, &args.database, &before))
        .transpose()?;
    let session =
        AttributionDatabaseSession::open(&args.database, AttributionDatabaseAccess::ReadOnly)?;
    let mut review = report::read_with_paper_source(session.database(), period, source.as_ref());
    ensure_normalized_snapshot(&args.database)?;
    anyhow::ensure!(
        before == file_sha256(&args.database)?,
        "source main changed during report read"
    );
    if let Some(source) = &source {
        source.verify_original()?;
    }
    review.input_source = Some(report::InputSource {
        database_path: args.database.display().to_string(),
        source_main_sha256: before.clone(),
        original_source_label: args.source_label,
        temporary_snapshot_deleted_after_run: args.temporary_snapshot,
        boundary: "AttributionDatabaseSession::ReadOnly detached snapshot; no initialization or application writes",
    });
    scorecard::attach(&mut review, registry, &before);
    let manifest = serde_json::to_string_pretty(review.evidence_manifest.as_ref().unwrap())? + "\n";
    let manifest_path = args.evidence_manifest.or_else(|| {
        args.output
            .as_ref()
            .map(|path| path.with_extension("evidence.json"))
    });
    if let (Some(output), Some(manifest)) = (&args.output, &manifest_path) {
        anyhow::ensure!(
            artifact_destination(output)? != artifact_destination(manifest)?,
            "report and manifest paths must differ after resolving parent directories"
        );
    }
    let reuse_manifest = if let Some(path) = &manifest_path {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {
                anyhow::ensure!(
                    std::fs::read(path)? == manifest.as_bytes(),
                    "existing evidence manifest differs; refusing overwrite"
                );
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        }
    } else {
        false
    };
    let mut rendered = match args.format {
        Format::Markdown => review.markdown(),
        Format::Json => serde_json::to_string_pretty(&review)?,
    };
    rendered.push('\n');
    if let Some(output) = args.output {
        write_new_private(&output, rendered.as_bytes())?;
    } else {
        print!("{rendered}");
    }
    if let Some(path) = manifest_path {
        persist_manifest(&path, manifest.as_bytes(), reuse_manifest)?;
    }
    Ok(())
}

/// Output parents must already exist. Resolving them handles relative/absolute,
/// parent-component and symlinked-directory aliases without creating anything.
fn artifact_destination(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("artifact path must name a file"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    Ok(parent.canonicalize()?.join(filename))
}
fn persist_manifest(path: &std::path::Path, bytes: &[u8], reuse: bool) -> anyhow::Result<()> {
    if reuse {
        anyhow::ensure!(
            std::fs::read(path)? == bytes,
            "reused evidence manifest changed; refusing success"
        );
        Ok(())
    } else {
        // A path appearing since preflight is never silently considered valid.
        write_new_private(path, bytes)
    }
}

fn write_new_private(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}
fn ensure_normalized_snapshot(path: &std::path::Path) -> anyhow::Result<()> {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    anyhow::ensure!(
        !wal.exists() || std::fs::metadata(wal)?.len() == 0,
        "input has nonempty WAL; use the read-only backup wrapper for normalized snapshot identity"
    );
    Ok(())
}
fn bytes_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
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
