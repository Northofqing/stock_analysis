//! One authenticated exact-window observation into an isolated evidence store.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;
use stock_analysis::data_gateway::observed_history_diagnostic;

#[derive(Parser)]
#[command(
    about = "Read one completed historical window into an existing isolated private observed-evidence directory"
)]
struct Args {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    output_root: PathBuf,
    #[arg(long, value_enum)]
    exchange: Venue,
    #[arg(long)]
    code: String,
    #[arg(
        long,
        required_unless_present = "candidate_db",
        conflicts_with = "candidate_db"
    )]
    from: Option<String>,
    /// Derive the start from original candidates in a private read-only snapshot.
    /// Prefer an existing stable copy; a changing live source is rejected.
    #[arg(long, conflicts_with = "from")]
    candidate_db: Option<PathBuf>,
    #[arg(long)]
    to: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Venue {
    Shanghai,
    Shenzhen,
}

fn observation_start(args: &Args) -> Result<String, String> {
    let Some(path) = &args.candidate_db else {
        return args
            .from
            .clone()
            .ok_or("historical window start is missing".into());
    };
    let through = chrono::NaiveDate::parse_from_str(&args.to, "%Y-%m-%d")
        .map_err(|_| "historical window end requires a canonical date".to_owned())?;
    if through.to_string() != args.to {
        return Err("historical window end requires a canonical date".into());
    }
    use stock_analysis::database::attribution_reports::{
        AttributionDatabaseAccess, AttributionDatabaseSession,
    };
    let session = AttributionDatabaseSession::open(path, AttributionDatabaseAccess::ReadOnly)
        .map_err(|_| "candidate database read-only snapshot unavailable".to_owned())?;
    stock_analysis::monitor::outcome_data::candidate_observation_start(
        session.database(),
        &args.code,
        through,
    )
    .map(|start| start.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let from = match observation_start(&args) {
        Ok(from) => from,
        Err(error) => {
            eprintln!("historical_observed_probe_failed: {error}");
            return ExitCode::from(2);
        }
    };
    let exchange = match args.exchange {
        Venue::Shanghai => "Shanghai",
        Venue::Shenzhen => "Shenzhen",
    };
    match observed_history_diagnostic(
        &args.bundle,
        &args.output_root,
        exchange,
        &args.code,
        &from,
        &args.to,
    )
    .await
    {
        Ok(summary) => {
            let rows_observed = serde_json::from_str::<serde_json::Value>(&summary)
                .is_ok_and(|value| value["diagnostic_result"] == "StoredObservedRows");
            println!("{summary}");
            if rows_observed {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("historical_observed_probe_failed: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wg06_observed_probe_requires_explicit_bundle_root_and_exact_request_without_limit_override()
    {
        let valid = [
            "probe",
            "--bundle",
            "/isolated/bundle",
            "--output-root",
            "/isolated/evidence",
            "--exchange",
            "shanghai",
            "--code",
            "600519",
            "--from",
            "2026-09-11",
            "--to",
            "2026-09-15",
        ];
        let args = Args::try_parse_from(valid).unwrap();
        assert_eq!(args.code, "600519");
        assert_eq!(args.from.as_deref(), Some("2026-09-11"));
        assert_eq!(args.output_root, PathBuf::from("/isolated/evidence"));
        for option in ["--limit", "--as-of", "--database", "--notify"] {
            let mut invalid = valid.to_vec();
            invalid.extend([option, "1"]);
            assert!(Args::try_parse_from(invalid).is_err());
        }
        assert!(Args::try_parse_from(["probe"]).is_err());
        let mut missing_bundle = valid.to_vec();
        missing_bundle.drain(1..3);
        assert!(Args::try_parse_from(missing_bundle).is_err());
    }

    #[test]
    fn observed_probe_candidate_range_requires_one_start_source_and_explicit_end() {
        let candidate = [
            "probe",
            "--bundle",
            "/isolated/bundle",
            "--output-root",
            "/isolated/evidence",
            "--exchange",
            "shanghai",
            "--code",
            "600519",
            "--candidate-db",
            "/isolated/TEST_CODE_candidates.db",
            "--to",
            "2026-09-30",
        ];
        let args = Args::try_parse_from(candidate).unwrap();
        assert!(args.from.is_none());
        assert!(args.candidate_db.is_some());
        let mut conflict = candidate.to_vec();
        conflict.extend(["--from", "2026-09-11"]);
        assert!(Args::try_parse_from(conflict).is_err());
        assert!(Args::try_parse_from(&candidate[..candidate.len() - 2]).is_err());
    }

    #[test]
    fn observed_probe_candidate_range_preserves_source_and_never_creates_missing_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("TEST_CODE_candidates.db");
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer.execute_batch("CREATE TABLE pushed_stocks(code TEXT,push_time TEXT); CREATE TABLE prediction_tracker(stock_code TEXT,pred_date TEXT,actual_change_t1 REAL,hit_t1 INTEGER,actual_change_t3 REAL,hit_t3 INTEGER,actual_change_t5 REAL,hit_t5 INTEGER); INSERT INTO pushed_stocks VALUES ('TEST_CODE_selected','2026-07-03'); INSERT INTO prediction_tracker(stock_code,pred_date) VALUES ('TEST_CODE_selected','2026-07-02');").unwrap();
        drop(writer);
        let before = std::fs::read(&db_path).unwrap();
        let mut args = Args {
            bundle: dir.path().join("bundle"),
            output_root: dir.path().join("evidence"),
            exchange: Venue::Shenzhen,
            code: "TEST_CODE_selected".into(),
            from: None,
            candidate_db: Some(db_path.clone()),
            to: "2026-09-30".into(),
        };
        assert_eq!(observation_start(&args).unwrap(), "2026-07-02");
        assert_eq!(std::fs::read(&db_path).unwrap(), before);
        args.candidate_db = Some(dir.path().join("TEST_CODE_missing.db"));
        assert!(observation_start(&args).is_err());
        assert!(!args.candidate_db.as_ref().unwrap().exists());
    }
}
