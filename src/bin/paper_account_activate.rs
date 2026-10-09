//! Explicit preview/apply operator path for a confirmed snapshot paper epoch.
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use stock_analysis::trading::paper_snapshot_activation::{
    apply_snapshot_paper_activation, open_snapshot_activation_database,
    preview_snapshot_paper_activation, SnapshotPaperActivationRequest,
};

#[derive(Debug, Parser)]
#[command(about = "Preview a snapshot paper activation; --apply requires its prepared source hash")]
struct Args {
    /// An explicit existing monitor database file.
    #[arg(long)]
    db: PathBuf,
    /// A versioned snapshot activation request. Preview returns prepared_request.
    #[arg(long)]
    manifest: PathBuf,
    /// Install the exact prepared activation atomically.
    #[arg(long)]
    apply: bool,
}
fn run(args: Args) -> anyhow::Result<()> {
    let bytes = std::fs::read(args.manifest)?;
    anyhow::ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "activation request exceeds 2 MiB"
    );
    let request: SnapshotPaperActivationRequest = serde_json::from_slice(&bytes)?;
    let mut conn = open_snapshot_activation_database(&args.db, args.apply)?;
    let outcome = if args.apply {
        apply_snapshot_paper_activation(&mut conn, &request, chrono::Utc::now())?
    } else {
        preview_snapshot_paper_activation(&mut conn, &request, chrono::Utc::now())?
    };
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}
fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("snapshot paper activation failed: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_paths_and_apply_authority() {
        let args = Args::try_parse_from([
            "paper_account_activate",
            "--db",
            "TEST_CODE.db",
            "--manifest",
            "TEST_CODE.json",
        ])
        .unwrap();
        assert!(!args.apply);
        let args = Args::try_parse_from([
            "paper_account_activate",
            "--db",
            "TEST_CODE.db",
            "--manifest",
            "TEST_CODE.json",
            "--apply",
        ])
        .unwrap();
        assert!(args.apply);
        for args in [
            vec!["paper_account_activate"],
            vec!["paper_account_activate", "--db", "TEST_CODE.db"],
            vec!["paper_account_activate", "--manifest", "TEST_CODE.json"],
            vec![
                "paper_account_activate",
                "--db",
                "TEST_CODE.db",
                "--manifest",
                "TEST_CODE.json",
                "--seed",
            ],
        ] {
            assert!(Args::try_parse_from(args).is_err());
        }
    }
}
