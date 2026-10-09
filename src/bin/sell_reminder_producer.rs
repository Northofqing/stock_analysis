//! 15:02 independent observation producer. No monitor startup, order or sink.
use clap::Parser;
use std::path::PathBuf;
use stock_analysis::offline_products::{
    io,
    sell_reminder::{producer, EvidencePack},
    shanghai_clock, Clock,
};

#[derive(Parser)]
#[command(about = "独立盘后卖出 producer：来源未准入时 NotReady；只读、无发送", group(clap::ArgGroup::new("input").required(true).args(["database", "evidence"])))]
struct Args {
    /// Existing detached stable snapshot; never initializes the database.
    #[arg(long)]
    database: Option<PathBuf>,
    /// Raw observations only; cannot assert dispatch source qualification.
    #[arg(long)]
    evidence: Option<PathBuf>,
    /// Explicit Shanghai clock for reproducible offline observation.
    #[arg(long, value_parser = shanghai_clock, requires = "completed_at")]
    started_at: Option<Clock>,
    #[arg(long, value_parser = shanghai_clock, requires = "started_at")]
    completed_at: Option<Clock>,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long)]
    markdown: Option<PathBuf>,
}
fn now() -> Clock {
    chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
}
fn run(args: Args) -> anyhow::Result<()> {
    // Start before output preflight/input reads so the timing receipt covers
    // those operations. Process launch overhead remains an external SLI.
    let started_at = args.started_at.unwrap_or_else(now);
    io::preflight_outputs(
        &[args.json.as_deref(), args.markdown.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
    )?;
    let result = if let Some(path) = args.evidence {
        let pack: EvidencePack = io::read_json(&path)?;
        let completed_at = args.completed_at.unwrap_or_else(now);
        producer::produce_observed(&pack, started_at, completed_at).map_err(anyhow::Error::msg)?
    } else {
        // The database reader runs before taking the live completion clock.
        let path = args.database.as_ref().unwrap();
        let preview_at = args.completed_at.unwrap_or_else(now);
        let production = producer::produce_database_observed(path, started_at, preview_at)?;
        let completed_at = args.completed_at.unwrap_or_else(now);
        // Re-observe the report's finished timing only. Never reclassify source
        // authority or turn a serialized preview into a candidate.
        producer::finish_database_observation(production, completed_at)
            .map_err(anyhow::Error::msg)?
    };
    if let Some(path) = args.json {
        io::write_new_private(&path, &serde_json::to_vec_pretty(result.receipt())?)?;
    }
    if let Some(path) = args.markdown {
        io::write_new_private(&path, result.markdown().as_bytes())?;
    } else {
        print!("{}", result.markdown());
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sell_reminder_producer_failed: {error}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observations_require_exact_paired_clock_override_and_never_overwrite() {
        assert!(Args::try_parse_from([
            "producer",
            "--evidence",
            "e.json",
            "--started-at",
            "2026-09-28T15:02:00+08:00"
        ])
        .is_err());
        assert!(Args::try_parse_from(["producer", "--evidence", "e", "--database", "d"]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.json");
        std::fs::write(
            &input,
            br#"{"schema":"sell-evidence-observed/v1","account":null,"lots":[],"securities":[]}"#,
        )
        .unwrap();
        let hash_before = io::file_hash(&input).unwrap();
        let args = || Args {
            database: None,
            evidence: Some(input.clone()),
            started_at: Some(shanghai_clock("2026-09-28T15:02:00+08:00").unwrap()),
            completed_at: Some(shanghai_clock("2026-09-28T15:03:00+08:00").unwrap()),
            json: Some(dir.path().join("receipt.json")),
            markdown: None,
        };
        run(args()).unwrap();
        let receipt: serde_json::Value = io::read_json(&dir.path().join("receipt.json")).unwrap();
        assert_eq!(receipt["state"], "NotReady");
        assert_eq!(receipt["dispatch_candidate_count"], 0);
        assert_eq!(receipt["elapsed_ms"], 60_000);
        assert!(run(args()).is_err());
        assert_eq!(hash_before, io::file_hash(&input).unwrap());
    }
}
