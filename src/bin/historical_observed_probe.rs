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
    #[arg(long)]
    from: String,
    #[arg(long)]
    to: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Venue {
    Shanghai,
    Shenzhen,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let exchange = match args.exchange {
        Venue::Shanghai => "Shanghai",
        Venue::Shenzhen => "Shenzhen",
    };
    match observed_history_diagnostic(
        &args.bundle,
        &args.output_root,
        exchange,
        &args.code,
        &args.from,
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
}
