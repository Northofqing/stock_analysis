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
        LlmProvider, LlmRegistry,
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
    /// Save Markdown from the same comparison; requires --format json --output.
    #[arg(long, requires = "output")]
    markdown_output: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t=Format::Markdown)]
    format: Format,
}
async fn run(args: Args) -> anyhow::Result<()> {
    let limits = assistant_review::cli_limits(args.ceiling_micro_cny);
    run_with(args, limits, None).await
}
// Injection keeps local fake tests on the exact load/compare/serialize/emission path.
async fn run_with(
    args: Args,
    limits: Limits,
    provider_override: Option<&dyn LlmProvider>,
) -> anyhow::Result<()> {
    preflight_outputs(&args)?;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(limits.wall_ms);
    // Publication is inside the declared whole budget; timeout drops the model future.
    let publication_ms = 2_000.min(limits.wall_ms / 2);
    let model_deadline = deadline - Duration::from_millis(publication_ms);
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
    let registry = (args.model && provider_override.is_none()).then(LlmRegistry::from_env);
    let provider = registry.as_ref().and_then(|r| r.select("assistant_review"));
    let limits_wall_ms = limits.wall_ms;
    let mut result = assistant_review::compare(
        &pack,
        provider_override.or(provider.as_deref()),
        pricing.as_ref(),
        limits,
        model_deadline,
    )
    .await?;
    result["deadline_budgets"] = serde_json::json!({"whole_run_ms":limits_wall_ms, "publication_reserved_ms":publication_ms,"model_work_ms":limits_wall_ms-publication_ms});
    result["pricing_diagnostic"] = serde_json::json!(pricing_diagnostic);
    result["elapsed_ms"] = serde_json::json!(started.elapsed().as_millis());
    emit_comparison(&result, &args, deadline, &mut std::io::stdout().lock())
}
fn preflight_outputs(args: &Args) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    anyhow::ensure!(
        args.markdown_output.is_none()
            || (matches!(args.format, Format::Json) && args.output.is_some()),
        "markdown_output_requires_json_file"
    );
    let mut destinations = std::collections::BTreeSet::new();
    for path in [args.output.as_deref(), args.markdown_output.as_deref()]
        .into_iter()
        .flatten()
    {
        let parent = path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let metadata = std::fs::symlink_metadata(parent)?;
        anyhow::ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.mode() & 0o077 == 0
                && metadata.uid() == unsafe { libc::geteuid() },
            "private_owned_output_directory_required"
        );
        let destination = parent.canonicalize()?.join(
            path.file_name()
                .ok_or_else(|| anyhow::anyhow!("output_file_required"))?,
        );
        anyhow::ensure!(
            destinations.insert(destination),
            "output_destinations_must_differ"
        );
        match std::fs::symlink_metadata(path) {
            Ok(_) => anyhow::bail!("output_already_exists"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
fn emit_comparison(
    result: &serde_json::Value,
    args: &Args,
    deadline: Instant,
    stdout: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    let bytes = match args.format {
        Format::Json => serde_json::to_vec_pretty(result)?,
        Format::Markdown => assistant_review::markdown(result)?.into_bytes(),
    };
    let markdown = args
        .markdown_output
        .as_ref()
        .map(|_| assistant_review::markdown(result).map(String::into_bytes))
        .transpose()?;
    // Render and cap both artifacts before publishing either. create_new still
    // enforces no-overwrite when a path appears after preflight.
    anyhow::ensure!(
        bytes.len() <= 8 * 1024 * 1024
            && markdown
                .as_ref()
                .is_none_or(|value| value.len() <= 8 * 1024 * 1024),
        "output_limit"
    );
    preflight_outputs(args)?;
    anyhow::ensure!(Instant::now() < deadline, "whole_run_deadline");
    emit(&bytes, args.output.as_deref(), stdout)?;
    if let Some(markdown) = markdown {
        let result = (|| {
            anyhow::ensure!(Instant::now() < deadline, "whole_run_deadline");
            emit(&markdown, args.markdown_output.as_deref(), stdout)
        })();
        if result.is_err() {
            eprintln!("assistant_review: partial_output_json_written_markdown_failed");
        }
        result?;
    }
    Ok(())
}
fn emit(
    bytes: &[u8],
    output: Option<&std::path::Path>,
    stdout: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    // Both destinations retain the same finite output cap after the larger report loader.
    anyhow::ensure!(bytes.len() <= 8 * 1024 * 1024, "output_limit");
    if let Some(path) = output {
        assistant_review::write_new_private(path, bytes)?;
    } else {
        stdout.write_all(bytes)?;
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use stock_analysis::llm::{
        bounded::{BoundedFailure, BoundedJsonRequest, BoundedResponse, SingleAttemptPermit},
        LlmError,
    };
    struct PendingFake {
        calls: AtomicUsize,
        dropped: AtomicBool,
    }
    struct CancelGuard<'a>(&'a AtomicBool);
    impl Drop for CancelGuard<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    #[async_trait::async_trait]
    impl LlmProvider for PendingFake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn model(&self) -> &str {
            "fake-model"
        }
        fn bounded_endpoint(&self) -> Option<String> {
            Some("https://fake.invalid/chat/completions".into())
        }
        async fn chat_json(&self, _: &str, _: &str) -> Result<Value, LlmError> {
            panic!("legacy path")
        }
        async fn chat_json_bounded_with_receipt(
            &self,
            _: BoundedJsonRequest<'_>,
            _: SingleAttemptPermit,
        ) -> Result<BoundedResponse, BoundedFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let _guard = CancelGuard(&self.dropped);
            std::future::pending().await
        }
    }
    #[test]
    fn emission_cap_covers_stdout_and_file_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("oversized.json");
        let oversized = vec![b'x'; 8 * 1024 * 1024 + 1];
        for destination in [None, Some(output.as_path())] {
            let mut stdout = Vec::new();
            assert!(emit(&oversized, destination, &mut stdout).is_err());
            assert!(stdout.is_empty());
            assert!(!output.exists());
        }
        let exact = vec![b'x'; 8 * 1024 * 1024];
        let mut stdout = Vec::new();
        emit(&exact, None, &mut stdout).unwrap();
        assert_eq!(stdout, exact);
    }
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
        assert!(matches!(a.format, Format::Markdown));
        assert!(!a.model);
        assert_eq!(a.ceiling_micro_cny, 0);
        assert!(a.reviewed_pricing.is_none());
        assert!(Args::try_parse_from(["assistant_review", "--report", "r"]).is_err());
        let explicit_json = Args::try_parse_from([
            "assistant_review",
            "--report",
            "r",
            "--manifest",
            "m",
            "--as-of",
            "2026-10-08T16:00:00+08:00",
            "--completed-session",
            "2026-10-08",
            "--format",
            "json",
        ])
        .unwrap();
        assert!(matches!(explicit_json.format, Format::Json));
    }
    #[test]
    fn dual_output_preflight_rejects_alias_existing_markdown_and_shared_parent_before_compare() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("comparison.json");
        let markdown = dir.path().join("comparison.md");
        let mut args = Args::try_parse_from([
            "assistant_review",
            "--report",
            "not-read-report",
            "--manifest",
            "not-read-manifest",
            "--as-of",
            "2026-10-08T16:00:00+08:00",
            "--completed-session",
            "2026-10-08",
            "--format",
            "json",
            "--output",
            output.to_str().unwrap(),
            "--markdown-output",
            markdown.to_str().unwrap(),
        ])
        .unwrap();
        preflight_outputs(&args).unwrap();
        args.format = Format::Markdown;
        assert!(preflight_outputs(&args).is_err());
        args.format = Format::Json;
        args.markdown_output = Some(output.clone());
        assert!(preflight_outputs(&args).is_err());
        args.markdown_output = Some(markdown.clone());
        assistant_review::write_new_private(&markdown, b"existing").unwrap();
        assert!(preflight_outputs(&args).is_err());
        assert!(!output.exists());
        std::fs::remove_file(&markdown).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(preflight_outputs(&args).is_err());
        assert!(!output.exists());
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
            markdown_output: Some(dir.path().join("comparison.md")),
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
        assert_eq!(
            std::fs::read_to_string(dir.path().join("comparison.md")).unwrap(),
            assistant_review::markdown(&comparison).unwrap()
        );
        assert!(run(args()).await.is_err());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        assert_eq!(std::fs::read(&rpath).unwrap(), rb);
        assert_eq!(std::fs::read(&mpath).unwrap(), mb);
        let bad_pricing = dir.path().join("bad-pricing.json");
        assistant_review::write_new_private(&bad_pricing, b"not JSON").unwrap();
        let mut invalid = args();
        invalid.reviewed_pricing = Some(bad_pricing);
        invalid.output = Some(dir.path().join("pricing-degraded.json"));
        invalid.markdown_output = None;
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
        // Same CLI emission path with a local pending future; no environment or endpoints.
        let price = json!({"schema_version":"assistant-reviewed-pricing-v1","reviewed_by":"localfake-test","contract_version":"fixture", "valid_until":"2099-01-01T00:00:00Z", "provider":"fake","requested_model":"fake-model","endpoint":"https://fake.invalid/chat/completions","upstream_models":["fake-model"],"currency":"CNY","billing_scope":"prompt_completion_only_no_hidden_tokens","input_bound_method":"utf8_bytes_plus_reviewed_framing","framing_tokens":20,"max_output_tokens":8192,"input_micro_cny_per_million":1,"output_micro_cny_per_million":1,"fixed_max_micro_cny":1});
        let price_path = dir.path().join("fake-pricing.json");
        assistant_review::write_new_private(&price_path, &serde_json::to_vec(&price).unwrap())
            .unwrap();
        let fake = PendingFake {
            calls: AtomicUsize::new(0),
            dropped: AtomicBool::new(false),
        };
        for format in [Format::Json, Format::Markdown] {
            let mut timed = args();
            timed.reviewed_pricing = Some(price_path.clone());
            let timed_path = dir.path().join(if matches!(format, Format::Json) {
                "timeout.json"
            } else {
                "timeout.md"
            });
            timed.output = Some(timed_path.clone());
            timed.format = format;
            timed.markdown_output = None;
            let before = Instant::now();
            tokio::time::timeout(
                Duration::from_millis(1000),
                run_with(
                    timed,
                    Limits {
                        wall_ms: 1000,
                        ceiling_micro_cny: 100,
                        ..Limits::default()
                    },
                    Some(&fake),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(before.elapsed() < Duration::from_millis(1000));
            assert!(fake.dropped.load(Ordering::SeqCst));
            let text = std::fs::read_to_string(timed_path).unwrap();
            if matches!(format, Format::Json) {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["arms"][1]["fallback_reason"], "deadline");
                assert_eq!(value["arms"][1]["model_receipt"], Value::Null);
                assert_eq!(value["arms"][1]["raw_model_output_state"], "unavailable");
                assert_eq!(
                    value["arms"][2]["fallback_reason"],
                    "prior_attempt_uncertain_or_invalid"
                );
                assert_eq!(value["run_reservations"]["attempt_slots_issued"], 1);
                assert!(
                    value["run_reservations"]["retained_maximum_micro_cny"]
                        .as_u64()
                        .unwrap()
                        > 0
                );
                assert_eq!(value["run_reservations"]["refunds_micro_cny"], 0);
                assert_eq!(value["deadline_budgets"]["publication_reserved_ms"], 500);
            } else {
                assert!(text.contains("本周"));
                assert!(text.contains("deadline"));
                assert!(text.contains("retained_maximum_micro_cny"));
            }
        }
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2); // one per separate run
        let md = assistant_review::markdown(&comparison).unwrap();
        assert!(md.contains("degraded_template"));
        assert!(md.contains(comparison["report_sha256"].as_str().unwrap()));
    }
}
