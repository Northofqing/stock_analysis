//! Offline by default; explicit --model enables reviewed bounded assistant_review role only.
use clap::{Parser, ValueEnum};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use stock_analysis::{
    assistant_review::{self, FrozenPack},
    llm::{
        bounded::{Limits, ReviewedPricing},
        LlmRegistry,
    },
};
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Markdown,
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    report: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    registry: Option<PathBuf>,
    #[arg(long)]
    as_of: String,
    #[arg(long)]
    completed_session: chrono::NaiveDate,
    #[arg(long)]
    model: bool,
    #[arg(long)]
    reviewed_pricing: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    ceiling_micro_cny: u64,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t=Format::Json)]
    format: Format,
}
async fn run(args: Args) -> anyhow::Result<()> {
    let limits = Limits {
        ceiling_micro_cny: args.ceiling_micro_cny,
        ..Limits::default()
    };
    let started = Instant::now();
    let deadline = started + Duration::from_millis(limits.wall_ms);
    let pack = FrozenPack::load(
        &args.report,
        &args.manifest,
        args.registry.as_deref(),
        &args.as_of,
        args.completed_session,
    )?;
    let pricing_result = args
        .reviewed_pricing
        .as_deref()
        .map(|p| {
            assistant_review::read_private(p, 32 * 1024)
                .and_then(|b| Ok(serde_json::from_slice::<ReviewedPricing>(&b)?))
        })
        .transpose();
    let (pricing, pricing_diagnostic) = match pricing_result {
        Ok(value) => (value, None),
        Err(_) => (None, Some("reviewed_pricing_invalid_or_unavailable")),
    };
    // No dotenv/logger/DB initialization. Default path never initializes registry or providers.
    let registry = args.model.then(LlmRegistry::from_env);
    let provider = registry.as_ref().and_then(|r| r.select("assistant_review"));
    let mut result = assistant_review::compare(
        &pack,
        provider.as_deref(),
        pricing.as_ref(),
        limits,
        deadline,
    )
    .await?;
    result["pricing_diagnostic"] = serde_json::json!(pricing_diagnostic);
    result["elapsed_ms"] = serde_json::json!(started.elapsed().as_millis());
    let bytes = match args.format {
        Format::Json => serde_json::to_vec_pretty(&result)?,
        Format::Markdown => assistant_review::markdown(&result)?.into_bytes(),
    };
    anyhow::ensure!(Instant::now() < deadline, "whole_run_deadline");
    if let Some(path) = args.output {
        assistant_review::write_new_private(&path, &bytes)?;
    } else {
        use std::io::Write;
        std::io::stdout().lock().write_all(&bytes)?;
    }
    Ok(())
}
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let result =
        tokio::time::timeout(Duration::from_millis(Limits::default().wall_ms), run(args)).await;
    match result {
        Ok(Ok(())) => std::process::ExitCode::SUCCESS,
        _ => {
            eprintln!("assistant_review: invalid_or_unavailable_input_or_output");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    #[test]
    fn explicit_inputs_and_default_offline_mode() {
        let a = Args::try_parse_from([
            "assistant_review",
            "--report",
            "r",
            "--manifest",
            "m",
            "--as-of",
            "2026-10-08T16:00:00+08:00",
            "--completed-session",
            "2026-10-08",
        ])
        .unwrap();
        assert!(!a.model);
        assert_eq!(a.ceiling_micro_cny, 0);
        assert!(a.reviewed_pricing.is_none());
        assert!(Args::try_parse_from(["assistant_review", "--report", "r"]).is_err());
    }
    #[tokio::test]
    async fn offline_cli_new_output_same_loaded_bytes_and_no_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let rpath = dir.path().join("review.json");
        let mpath = dir.path().join("manifest.json");
        let output = dir.path().join("comparison.json");
        let raw = include_str!("../../config/signal_registry.toml");
        let registry: toml::Value = toml::from_str(raw).unwrap();
        let registry = json!({"source":"detached-test","sha256":stock_analysis::llm::bounded::hash(raw.as_bytes()),"content":serde_json::to_value(registry).unwrap(),"authority":"manual"});
        let snapshot = "a".repeat(64);
        let evidence = json!({"input_snapshot_sha256":snapshot,"reader_id":"localfake","sample_scope":"completed week","exclusions":"unknown joins","meaning":"unavailable"});
        let metric = json!({"id":"missing","grade":"unavailable","value":null,"reason":"family symbol PIT monetary authority missing","evidence":evidence});
        let sections =
            json!({"price_observation":[metric],"simulated_fill":[metric],"net_return":[metric]});
        let families: Vec<Value> = registry["content"]["signal"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| json!({"signal_id":s["id"],"sections":sections}))
            .collect();
        let period = json!({"requested_from":"2026-09-28","requested_to":"2026-10-04","observed_at":"2026-10-08T16:00:00+08:00","latest_completed_session":"2026-10-08","period_completed_through":"2026-09-30","completed_sessions":["2026-09-28","2026-09-29","2026-09-30"]});
        let manifest = json!({"schema_version":"weekly-outcome-evidence-manifest-v1","artifact_schema":"weekly-signal-scorecard-v1","input_snapshot_sha256":snapshot,"period":period,"registry":registry,"reader_source_sha256":"b".repeat(64)});
        let report = json!({"report_version":"H16-descriptive-weekly-v1","input_source":{"source_main_sha256":snapshot},"period":period,"evidence_manifest":manifest,
            "scorecard":{"schema_version":"weekly-signal-scorecard-v1","registry":registry,"families":families,"pooled_descriptive_evidence":sections}});
        let rb = serde_json::to_vec(&report).unwrap();
        let mb = serde_json::to_vec(&manifest).unwrap();
        assistant_review::write_new_private(&rpath, &rb).unwrap();
        assistant_review::write_new_private(&mpath, &mb).unwrap();
        let args = || Args {
            report: rpath.clone(),
            manifest: mpath.clone(),
            registry: None,
            as_of: "2026-10-08T16:00:00+08:00".into(),
            completed_session: chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
            model: false,
            reviewed_pricing: None,
            ceiling_micro_cny: 0,
            output: Some(output.clone()),
            format: Format::Json,
        };
        run(args()).await.unwrap();
        let bytes = std::fs::read(&output).unwrap();
        let comparison: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            comparison["report_sha256"],
            stock_analysis::llm::bounded::hash(&rb)
        );
        assert_eq!(
            comparison["manifest_sha256"],
            stock_analysis::llm::bounded::hash(&mb)
        );
        assert_eq!(comparison["arms"][1]["mode"], "degraded_template");
        assert_eq!(comparison["arms"][1]["model_receipt"], Value::Null);
        assert!(run(args()).await.is_err());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        assert_eq!(std::fs::read(&rpath).unwrap(), rb);
        assert_eq!(std::fs::read(&mpath).unwrap(), mb);
        let bad_pricing = dir.path().join("bad-pricing.json");
        assistant_review::write_new_private(&bad_pricing, b"not JSON").unwrap();
        let mut invalid = args();
        invalid.reviewed_pricing = Some(bad_pricing);
        invalid.output = Some(dir.path().join("pricing-degraded.json"));
        run(invalid).await.unwrap();
        let degraded: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("pricing-degraded.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            degraded["pricing_diagnostic"],
            "reviewed_pricing_invalid_or_unavailable"
        );
        assert_eq!(degraded["run_reservations"]["attempt_slots_issued"], 0);
        let md = assistant_review::markdown(&comparison).unwrap();
        assert!(md.contains("degraded_template"));
        assert!(md.contains(comparison["report_sha256"].as_str().unwrap()));
    }
}
