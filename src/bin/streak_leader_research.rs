//! Offline research/export only. Existing checked artifacts remain observations.
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use stock_analysis::offline_products::{
    io, shanghai_clock,
    streak_leader_research::{self, EvidencePack, Policy},
    Clock,
};
#[derive(Parser)]
#[command(about = "StreakLeader 离线观察/研究导出；不创建实时准入、不启动虚拟盘")]
struct Args {
    #[arg(long, required_unless_present = "store")]
    evidence: Option<PathBuf>,
    #[arg(long, requires = "artifact_ref")]
    store: Option<PathBuf>,
    #[arg(long, requires = "store")]
    artifact_ref: Option<PathBuf>,
    #[arg(long,value_parser=shanghai_clock)]
    as_of: Clock,
    #[arg(long, default_value_t = 3)]
    max_picks: usize,
    #[arg(long, default_value_t = 100)]
    shares: u32,
    #[arg(long, default_value_t = 100)]
    max_entry_slippage_bps: u32,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long)]
    csv: Option<PathBuf>,
    #[arg(long)]
    markdown: Option<PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactRef {
    capture_sha256: String,
    file_sha256: String,
    byte_length: u64,
}
fn run(args: Args) -> anyhow::Result<()> {
    io::preflight_outputs(
        &[
            args.json.as_deref(),
            args.csv.as_deref(),
            args.markdown.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>(),
    )?;
    let pack = match args.evidence {
        Some(path) => io::read_json(&path)?,
        None => EvidencePack {
            schema: "streak-observed/v1".into(),
            days: vec![],
        },
    };
    let policy = Policy {
        max_picks: args.max_picks,
        shares: args.shares,
        max_entry_slippage_bps: args.max_entry_slippage_bps,
        ..Policy::default()
    };
    let mut result = streak_leader_research::research_observed(&pack, policy, args.as_of)
        .map_err(anyhow::Error::msg)?;
    if let Some(root) = args.store {
        let reference: ArtifactRef = io::read_json(args.artifact_ref.as_ref().unwrap())?;
        let artifact = stock_analysis::data_gateway::read_offline_observed_artifact(
            &root,
            &reference.capture_sha256,
            &reference.file_sha256,
            reference.byte_length,
        )?;
        result.attach_checked_observation(artifact);
    }
    if let Some(path) = args.json {
        io::write_new_private(&path, &serde_json::to_vec_pretty(&result)?)?;
    }
    if let Some(path) = args.csv {
        io::write_new_private(&path, result.csv().as_bytes())?;
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
        Err(e) => {
            eprintln!("streak_leader_research_failed: {e}");
            std::process::ExitCode::from(2)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_human_markdown() {
        let args = Args::try_parse_from([
            "research",
            "--evidence",
            "input.json",
            "--as-of",
            "2026-09-28T15:05:00+08:00",
        ])
        .unwrap();
        assert!(args.json.is_none());
        assert!(args.markdown.is_none());
    }
    #[test]
    fn no_default_source_or_clock() {
        assert!(Args::try_parse_from(["research"]).is_err());
        assert!(Args::try_parse_from([
            "research",
            "--store",
            "x",
            "--as-of",
            "2026-09-28T15:05:00+08:00",
            "--json",
            "a",
            "--csv",
            "b",
            "--markdown",
            "c"
        ])
        .is_err());
    }
    #[test]
    fn zero_samples_stay_unavailable_in_exports() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.json");
        std::fs::write(&input, r#"{"schema":"streak-observed/v1","days":[]}"#).unwrap();
        let args = Args {
            evidence: Some(input.clone()),
            store: None,
            artifact_ref: None,
            as_of: shanghai_clock("2026-09-28T15:05:00+08:00").unwrap(),
            max_picks: 3,
            shares: 100,
            max_entry_slippage_bps: 100,
            json: Some(dir.path().join("out.json")),
            csv: Some(dir.path().join("out.csv")),
            markdown: Some(dir.path().join("out.md")),
        };
        run(args).unwrap();
        let saved: serde_json::Value = io::read_json(&dir.path().join("out.json")).unwrap();
        let pack: EvidencePack = io::read_json(&input).unwrap();
        let report = streak_leader_research::research_observed(
            &pack,
            Policy::default(),
            shanghai_clock("2026-09-28T15:05:00+08:00").unwrap(),
        )
        .unwrap();
        assert!(saved["modeled_win_rate"].is_null());
        assert_eq!(saved, serde_json::to_value(&report).unwrap());
        assert_eq!(
            report.markdown(),
            std::fs::read_to_string(dir.path().join("out.md")).unwrap()
        );
        assert_eq!(
            report.csv(),
            std::fs::read_to_string(dir.path().join("out.csv")).unwrap()
        );
    }
}
