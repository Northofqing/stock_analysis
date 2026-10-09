//! Offline explicit-input preview; never starts monitor or initializes DB.
use clap::Parser;
use std::path::PathBuf;
use stock_analysis::offline_products::{
    io,
    sell_reminder::{self, EvidencePack, ImportedPreview},
    shanghai_clock, Clock,
};
#[derive(Parser)]
#[command(about="只读卖出预览：显式稳定快照/观察包；无来源资格时明确不可用",group(clap::ArgGroup::new("input").required(true).args(["database","evidence","reinspect"])))]
struct Args {
    #[arg(long)]
    database: Option<PathBuf>,
    #[arg(long)]
    evidence: Option<PathBuf>,
    #[arg(long)]
    reinspect: Option<PathBuf>,
    #[arg(long,value_parser=shanghai_clock)]
    as_of: Clock,
    #[arg(long)]
    json: Option<PathBuf>,
    #[arg(long)]
    markdown: Option<PathBuf>,
    #[arg(long)]
    handling_template: Option<PathBuf>,
}
fn run(args: Args) -> anyhow::Result<()> {
    io::preflight_outputs(
        &[
            args.json.as_deref(),
            args.markdown.as_deref(),
            args.handling_template.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>(),
    )?;
    let result = if let Some(path) = args.database {
        sell_reminder::diagnose_database(&path, args.as_of)?
    } else if let Some(path) = args.evidence {
        let pack: EvidencePack = io::read_json(&path)?;
        sell_reminder::preview_observed(&pack, args.as_of)
    } else {
        let report: ImportedPreview = io::read_json(args.reinspect.as_ref().unwrap())?;
        sell_reminder::reinspect_imported(report, args.as_of).map_err(anyhow::Error::msg)?
    };
    if let Some(path) = args.json {
        io::write_new_private(&path, &serde_json::to_vec_pretty(&result)?)?;
    }
    if let Some(path) = args.markdown {
        io::write_new_private(&path, result.markdown().as_bytes())?;
    } else {
        print!("{}", result.markdown());
    }
    if let Some(path) = args.handling_template {
        io::write_new_private(
            &path,
            &serde_json::to_vec_pretty(&result.handling_template())?,
        )?;
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sell_reminder_preview_failed: {e}");
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
            "preview",
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
    fn explicit_clock_and_exclusive_input_required() {
        assert!(Args::try_parse_from(["preview", "--json", "a", "--markdown", "b"]).is_err());
        assert!(Args::try_parse_from([
            "preview",
            "--evidence",
            "e",
            "--database",
            "d",
            "--as-of",
            "2026-09-28T15:05:00+08:00",
            "--json",
            "a",
            "--markdown",
            "b"
        ])
        .is_err());
    }
    #[test]
    fn observation_exports_same_result_and_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.json");
        std::fs::write(
            &input,
            r#"{"schema":"sell-evidence-observed/v1","account":null,"lots":[],"securities":[]}"#,
        )
        .unwrap();
        let before = io::file_hash(&input).unwrap();
        let args = || Args {
            database: None,
            evidence: Some(input.clone()),
            reinspect: None,
            as_of: shanghai_clock("2026-09-28T15:05:00+08:00").unwrap(),
            json: Some(dir.path().join("out.json")),
            markdown: Some(dir.path().join("out.md")),
            handling_template: Some(dir.path().join("human.json")),
        };
        run(args()).unwrap();
        let saved: serde_json::Value = io::read_json(&dir.path().join("out.json")).unwrap();
        let pack: EvidencePack = io::read_json(&input).unwrap();
        let report = sell_reminder::preview_observed(
            &pack,
            shanghai_clock("2026-09-28T15:05:00+08:00").unwrap(),
        );
        assert_eq!(saved, serde_json::to_value(&report).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.md")).unwrap(),
            report.markdown()
        );
        assert!(run(args()).is_err());
        assert_eq!(before, io::file_hash(&input).unwrap());
    }
}
