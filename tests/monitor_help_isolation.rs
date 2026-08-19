//! BR-051/BR-136/BR-141: terminal commands must run outside the bare-service gate
//! without initializing production data or hiding event-writer failures.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

fn isolated_monitor_command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_monitor"));
    command.current_dir(root);
    for key in [
        "DATABASE_PATH",
        "MAGICLAW_DB_PATH",
        "MONITOR_ENABLED",
        "STOCK_ENV_MODE",
        "V10_DRY_RUN_PUSH",
        "DURABLE_DELIVERY_TEST_CODE",
        "V12_E2E_REAL_PUSH",
        "STOCK_ANALYSIS_PUSH_V6_ENABLE",
        "EVENT_AUDIT_DIR",
        "PUSH_LOG_DIR",
        "DISPATCHER_LOG_DIR",
        "REVIEW_AUDIT_DIR",
        "ALERT_WEBHOOK_URL",
        "BR196_FEISHU_APP_ID",
        "BR196_FEISHU_CONVERSATION_ID",
        "BR196_FEISHU_TENANT_ID",
        "BR196_LIVE_FEISHU_ACCEPTANCE",
        "CUSTOM_WEBHOOK_URL",
        "DINGTALK_WEBHOOK",
        "DISCORD_WEBHOOK",
        "FEISHU_APP_ID",
        "FEISHU_APP_SECRET",
        "FEISHU_TO",
        "FEISHU_WEBHOOK",
        "FEISHU_WEBHOOK_URL",
        "MAGICLAW_API_ADDR",
        "MAGICLAW_API_TOKEN",
        "MAGICLAW_BIN",
        "MAGICLAW_HOME",
        "MAGICLAW_PROJECT_ID",
        "MAGICLAW_SEND_TYPE",
        "SERVER_CHAN_KEY",
        "SLACK_WEBHOOK",
        "TELEGRAM_BOT_TOKEN",
        "WECHAT_WEBHOOK",
    ] {
        command.env_remove(key);
    }
    command
}

fn initialized_database_path(output: &str) -> std::path::PathBuf {
    let marker = "初始化数据库: ";
    output
        .lines()
        .find_map(|line| {
            line.split_once(marker)
                .map(|(_, path)| std::path::PathBuf::from(path.trim()))
        })
        .expect("monitor output must identify its bound database")
}

fn bound_durable_test_code(output: &str) -> String {
    let marker = "[DurableDelivery][BR-192] test namespace bound code=";
    let test_code = output
        .lines()
        .find_map(|line| {
            line.split_once(marker)
                .map(|(_, test_code)| test_code.trim().to_owned())
        })
        .expect("monitor output must identify its bound durable TEST_CODE namespace");
    assert!(
        test_code.starts_with("TEST_CODE_MONITOR_")
            && test_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
        "monitor emitted an invalid durable TEST_CODE namespace: {test_code}"
    );
    test_code
}

fn manifest_durable_test_namespace(test_code: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data/test")
        .join(test_code)
}

fn br196_summary_count(summary: &str, key: &str) -> usize {
    summary
        .split_ascii_whitespace()
        .find_map(|field| field.strip_prefix(key))
        .unwrap_or_else(|| panic!("BR-196 summary is missing {key}: {summary}"))
        .parse::<usize>()
        .unwrap_or_else(|error| panic!("BR-196 summary has invalid {key}: {error}: {summary}"))
}

fn assert_br196_explicit_dry_run_summary(output: &str) {
    let summary = output
        .lines()
        .find(|line| line.contains("template_test_summary manifest_version=BR196_V2"))
        .unwrap_or_else(|| panic!("BR-196 manifest summary is missing: {output}"));
    let family_active = summary
        .split_ascii_whitespace()
        .find_map(|field| field.strip_prefix("family=A"))
        .and_then(|counts| counts.split('/').next())
        .unwrap_or_else(|| panic!("BR-196 active-family count is missing: {summary}"))
        .parse::<usize>()
        .unwrap_or_else(|error| {
            panic!("BR-196 active-family count is invalid: {error}: {summary}")
        });
    let rendered = br196_summary_count(summary, "rendered=");
    let explicit_dry_run = br196_summary_count(summary, "explicit_dry_run_family_total=");

    assert_eq!(
        rendered, family_active,
        "BR-196 must render the active manifest families: {summary}"
    );
    assert_eq!(
        explicit_dry_run, family_active,
        "BR-196 dry-run must account for every active manifest family: {summary}"
    );
    assert!(
        summary
            .split_ascii_whitespace()
            .any(|field| field == "smoke=3/3"),
        "BR-196 exact-three non-counted governance smoke is incomplete: {summary}"
    );
    for zero_count in [
        "external_process_attempted=0",
        "batches_attempted=0",
        "batches_pushed=0",
        "families_pushed=0",
        "receipt_audit_appended=0",
        "failed=0",
    ] {
        assert!(
            summary
                .split_ascii_whitespace()
                .any(|field| field == zero_count),
            "BR-196 dry-run transport invariant {zero_count} is missing: {summary}"
        );
    }
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn production_process_rejects_dry_run_before_opening_durable_runtime() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-production-dry-run-rejection-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated production dry-run directory");

    let output = isolated_monitor_command(&root)
        .arg("--backfill-st-type")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
        .output()
        .expect("run normal monitor with forbidden production dry-run");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains(
            "[DurableDelivery][BR-192] production configuration rejected: V10_DRY_RUN_PUSH=1"
        ),
        "production dry-run rejection was not explicit: {combined_output}"
    );
    assert!(
        !root.join("data/durable_delivery.sqlite3").exists(),
        "production dry-run opened the production durable database"
    );
    assert!(
        !combined_output.contains("TEST_CODE_MAGICLAW_DRY_RUN"),
        "production synthesized a TEST_CODE authoritative receipt: {combined_output}"
    );
    for forbidden_startup_marker in [
        "[AuditDegraded][BR-144]",
        "[DB init][BR-051][BR-183] core database bound",
        "[DurableDelivery][BR-192] startup fixed point reached",
    ] {
        assert!(
            !combined_output.contains(forbidden_startup_marker),
            "production dry-run crossed the pre-runtime rejection boundary {forbidden_startup_marker}: {combined_output}"
        );
    }
    let operator_guide = include_str!("../CLAUDE.md");
    assert!(
        operator_guide.contains(
            "cargo run --bin monitor -- --review     # manual post-market review (do NOT set V10_DRY_RUN_PUSH=1: BR-192 rejects it)"
        ) && !operator_guide.contains("需要阻止外发时显式设置 `V10_DRY_RUN_PUSH=1`"),
        "operator documentation must not reintroduce production dry-run"
    );

    std::fs::remove_dir_all(root).expect("remove isolated production dry-run directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn p01_compensation_lease_loser_exits_before_provider_or_durable_side_effects() {
    use fs2::FileExt;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-p01-lease-loser-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated P-01 lease-loser directory");
    let lease_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data/locks/production/monitor-delivery.lock");
    std::fs::create_dir_all(lease_path.parent().expect("production lease parent"))
        .expect("create production lease parent");
    let lease = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lease_path)
        .expect("open production lease");
    let test_owns_lease = match lease.try_lock_exclusive() {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("inspect production monitor lease: {error}"),
    };

    let output = isolated_monitor_command(&root)
        // This date has no admitted calendar authority. If an independently
        // running resident releases the lease during the subprocess race, the
        // command still exits before provider/durable/audit/sink initialization.
        .args(["--compensate=P-01", "--business-date=1900-01-01"])
        .env("MONITOR_ENABLED", "true")
        .output()
        .expect("run P-01 compensation lease loser");
    if test_owns_lease {
        fs2::FileExt::unlock(&lease).expect("release production monitor lease");
    }
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains("monitor_instance_already_running"),
        "P-01 lease loser must fail with the stable singleton code: {combined_output}"
    );
    assert!(
        !root.join("data").exists(),
        "P-01 lease loser created provider/durable/audit state"
    );
    for forbidden in [
        "[P-01][BR-241] compensation_",
        "[DB init][BR-051][BR-183] core database bound",
        "[DurableDelivery][BR-192]",
        "开始推送",
    ] {
        assert!(
            !combined_output.contains(forbidden),
            "P-01 lease loser crossed forbidden boundary {forbidden}: {combined_output}"
        );
    }

    std::fs::remove_dir_all(root).expect("remove isolated P-01 lease-loser directory");
}

#[test]
fn p01_compensation_source_binds_cli_and_production_lease_to_one_task_local_kind() {
    let main = include_str!("../src/bin/monitor/main.rs");
    let entry = main
        .split_once("async fn main()")
        .expect("monitor entrypoint")
        .1;
    let runtime = include_str!("../src/bin/monitor/durable_delivery_runtime.rs");

    let lease = entry
        .find("let _monitor_instance_lease")
        .expect("normal monitor lease");
    let request = entry
        .find(".p01_compensation_request()")
        .expect("opaque P-01 CLI request");
    let capability = entry
        .find("durable_delivery_runtime::authorize_p01_compensation_scope")
        .expect("typed production-lease capability");
    let runtime_init = entry
        .find("preflight_runtime_delivery_audit")
        .expect("runtime audit initialization");
    assert!(lease < request && request < capability && capability < runtime_init);

    assert!(runtime.contains("tokio::task_local!"));
    assert!(runtime.contains("struct P01CompensationCapability"));
    assert!(runtime.contains("p01_compensation_claim_is_authorized"));
    assert!(runtime.contains("push_kind == DurablePushKind::PreopenNewsHot"));
    assert!(runtime.contains("canonical.render_mode == \"Compensation\""));
    assert!(!runtime.contains("P01_COMPENSATION_SCOPE_ACTIVE"));
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_process_uses_physical_test_code_durable_namespace_without_network() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-test-durable-namespace-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated test durable directory");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--backfill-st-type"])
        .env("MONITOR_ENABLED", "true")
        .output()
        .expect("run isolated test monitor");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "output={combined_output}");
    assert!(
        combined_output
            .contains("[DurableDelivery][BR-192] test namespace bound code=TEST_CODE_MONITOR_"),
        "test durable namespace was not explicit: {combined_output}"
    );
    let test_code = bound_durable_test_code(&combined_output);
    let durable_namespace = manifest_durable_test_namespace(&test_code);
    assert!(
        durable_namespace.join("durable_delivery.sqlite3").is_file(),
        "isolated invocation must own its exact repository-anchored TEST_CODE durable database"
    );
    assert!(
        !root.join("data/durable_delivery.sqlite3").exists(),
        "test invocation opened the production durable database"
    );
    for network_marker in ["开始推送", "authoritative test delivery skipped network"] {
        assert!(
            !combined_output.contains(network_marker),
            "isolated terminal dry-run reached a network/sink path marker {network_marker}: {combined_output}"
        );
    }

    std::fs::remove_dir_all(&durable_namespace)
        .expect("remove exact repository-anchored TEST_CODE durable namespace");
    std::fs::remove_dir_all(root).expect("remove isolated test durable directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn help_exits_without_creating_runtime_state() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-help-isolation-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = isolated_monitor_command(&root)
        .arg("--help")
        .env_remove("DATABASE_PATH")
        .output()
        .expect("run monitor --help");

    assert!(
        output.status.success(),
        "help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Usage: monitor"),
        "help text missing: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8_lossy(&output.stderr);
    for required in ["--test", "--review", "dry-run", "real account"] {
        assert!(
            help.contains(required),
            "help contract missing {required:?}: {help}"
        );
    }
    assert!(
        !root.join("data").exists(),
        "help command created runtime data under {}",
        root.display()
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn disabled_bare_monitor_exits_before_runtime_state() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-disabled-isolation-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = isolated_monitor_command(&root)
        .env_remove("MONITOR_ENABLED")
        .env_remove("DATABASE_PATH")
        .output()
        .expect("run disabled bare monitor");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "disabled bare monitor should exit cleanly; output={combined_output}"
    );
    assert!(
        combined_output.contains("[monitor] disabled: MONITOR_ENABLED is not true"),
        "disabled lifecycle decision was not visible; output={combined_output}"
    );
    assert!(
        !root.join("data").exists(),
        "disabled bare monitor created runtime data under {}",
        root.display()
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_only_diagnostic_without_test_flag_fails_before_runtime_state() {
    let root =
        std::env::temp_dir().join(format!("monitor-v13-diag-contract-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create isolated diagnostic directory");

    let output = isolated_monitor_command(&root)
        .arg("--v13-diag")
        .env_remove("MONITOR_ENABLED")
        .env_remove("DATABASE_PATH")
        .output()
        .expect("run invalid diagnostic mode");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains("selection_diag_requires_test"),
        "missing explicit mode error: {combined_output}"
    );
    assert!(
        !root.join("data").exists(),
        "invalid diagnostic mode initialized runtime state"
    );

    std::fs::remove_dir_all(root).expect("remove isolated diagnostic directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn br194_test_review_blocks_all_source_providers_and_sinks_before_account_gate() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-startup-governance-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    let caller_database_path = root.join("startup.db");
    let stdout_path = root.join("monitor.stdout.log");
    let stderr_path = root.join("monitor.stderr.log");
    let stdout_file = std::fs::File::create(&stdout_path).expect("create monitor stdout log");
    let stderr_file = std::fs::File::create(&stderr_path).expect("create monitor stderr log");

    let mut child = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", &caller_database_path)
        .env("STOCK_LIST", "")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
        .env_remove("ALERT_WEBHOOK_URL")
        .env_remove("CUSTOM_WEBHOOK_URL")
        .env_remove("DINGTALK_WEBHOOK")
        .env_remove("DISCORD_WEBHOOK")
        .env_remove("FEISHU_WEBHOOK_URL")
        .env_remove("SLACK_WEBHOOK")
        .env_remove("TELEGRAM_BOT_TOKEN")
        .env_remove("WECHAT_WEBHOOK")
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .spawn()
        .expect("spawn normal monitor process");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    loop {
        let combined_output = format!(
            "{}{}",
            std::fs::read_to_string(&stdout_path).unwrap_or_default(),
            std::fs::read_to_string(&stderr_path).unwrap_or_default()
        );
        if combined_output.contains("[B-005-C][BR-110][BR-140] 完成") {
            break;
        }
        if child
            .try_wait()
            .expect("poll isolated monitor process")
            .is_some()
            || std::time::Instant::now() >= deadline
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if child
        .try_wait()
        .expect("poll isolated monitor process before cleanup")
        .is_none()
    {
        child.kill().expect("terminate isolated monitor process");
    }
    child.wait().expect("collect isolated monitor status");
    let combined_output = format!(
        "{}{}",
        std::fs::read_to_string(&stdout_path).unwrap_or_default(),
        std::fs::read_to_string(&stderr_path).unwrap_or_default()
    );

    assert!(
        combined_output.contains("[B-005-C] 盘后批量 dispatcher 开始"),
        "review dispatcher marker missing; output={combined_output}"
    );
    for disabled_task in [
        "R-04:disabled",
        "R-08:disabled",
        "R-09:disabled",
        "A-10:disabled",
        "A-01:disabled",
    ] {
        assert!(
            combined_output.contains(disabled_task),
            "test review must preflight-disable {disabled_task}; output={combined_output}"
        );
    }
    for forbidden in [
        "[龙虎榜]",
        "[东财全市场]",
        "开始推送",
        "[飞书] 开始推送",
        "[AccountMode-hook] 启动评估",
    ] {
        assert!(
            !combined_output.contains(forbidden),
            "test review crossed a provider/sink/account acquisition boundary {forbidden}; output={combined_output}"
        );
    }
    assert!(
        !caller_database_path.exists(),
        "opaque bootstrap honored a caller-controlled database path"
    );
    assert!(
        combined_output.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ),
        "test process did not report its disabled selection capability; output={combined_output}"
    );

    std::fs::remove_dir_all(root).expect("remove isolated startup directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn br194_terminal_replay_cli_rejects_ordinal_override_before_database_open() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-br194-replay-cli-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated replay CLI directory");

    let output = isolated_monitor_command(&root)
        .args([
            "--br194-audited-terminal-replay",
            "--business-date",
            "2026-07-29",
            "--task",
            "R-04",
            "--replay-ordinal",
            "9",
        ])
        .output()
        .expect("run rejected BR-194 terminal replay");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains("replay ordinal is coordinator-owned and cannot be overridden"),
        "missing explicit ordinal rejection: {combined_output}"
    );
    assert!(
        !root.join("data").exists(),
        "invalid replay arguments initialized database/runtime state"
    );
    assert!(
        !combined_output.contains("初始化数据库")
            && !combined_output.contains("DurableDelivery")
            && !combined_output.contains("开始推送"),
        "invalid replay arguments crossed database/provider/sink initialization: {combined_output}"
    );

    std::fs::remove_dir_all(root).expect("remove isolated replay CLI directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn br194_terminal_replay_cli_rejects_duplicates_and_nontrading_dates_before_database_open() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let cases: &[(&[&str], &str)] = &[
        (
            &[
                "--business-date",
                "2026-07-29",
                "--business-date",
                "2026-07-30",
                "--task",
                "R-04",
            ],
            "--business-date must be specified exactly once",
        ),
        (
            &[
                "--business-date",
                "2026-07-29",
                "--task",
                "R-04",
                "--task",
                "R-09",
            ],
            "--task must be specified exactly once",
        ),
        (
            &["--business-date", "2026-07-25", "--task", "R-04"],
            "business date is not an A-share trading day",
        ),
        (
            &["--business-date", "2026-01-01", "--task", "R-09"],
            "business date is not an A-share trading day",
        ),
        (
            &["--business-date", "2024-07-30", "--task", "R-04"],
            "A-share trading-calendar authority unavailable",
        ),
    ];

    for (arguments, expected) in cases {
        let root = std::env::temp_dir().join(format!(
            "monitor-br194-replay-reject-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create isolated replay rejection directory");
        let mut command = isolated_monitor_command(&root);
        command.arg("--br194-audited-terminal-replay");
        command.args(*arguments);
        let output = command
            .output()
            .expect("run rejected BR-194 terminal replay");
        let combined_output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(2), "output={combined_output}");
        assert!(
            combined_output.contains(expected),
            "missing {expected:?}: {combined_output}"
        );
        assert!(
            !root.join("data").exists()
                && !combined_output.contains("初始化数据库")
                && !combined_output.contains("DurableDelivery")
                && !combined_output.contains("开始推送"),
            "invalid replay arguments crossed initialization: {combined_output}"
        );
        std::fs::remove_dir_all(root).expect("remove isolated replay rejection directory");
    }
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_mode_ignores_caller_supplied_production_database_path() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-test-db-rejection-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", "./data/stock_analysis.db")
        .env_remove("ALERT_WEBHOOK_URL")
        .env("STOCK_ENV_MODE", "test")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
        .output()
        .expect("run monitor with forbidden production DB path");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        matches!(output.status.code(), Some(0 | 2)),
        "strict review must report a truthful success or fail-closed status; output={combined_output}"
    );
    if output.status.success() {
        assert!(
            combined_output.contains("[复盘] 盘后分析完成")
                || combined_output.contains("delivered="),
            "successful review must expose completion evidence; output={combined_output}"
        );
    } else {
        assert!(
            combined_output.contains("[复盘]") && combined_output.contains("exit 2"),
            "failed review must expose its fail-closed reason; output={combined_output}"
        );
    }
    assert!(
        combined_output.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ),
        "disabled selection capability summary missing: {combined_output}",
    );
    assert!(
        combined_output.contains("[DB init][BR-051][BR-183] core database bound mode=test path=")
            && combined_output.contains("TEST_CODE_monitor_"),
        "test mode did not bind an isolated TEST_CODE database; output={combined_output}",
    );
    assert!(
        !root.join("data/stock_analysis.db").exists(),
        "test mode opened the forbidden production DB path"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_mode_ignores_the_repository_dotenv_production_database_default() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-test-dotenv-isolation-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    std::fs::write(
        root.join(".env"),
        "DATABASE_PATH=./data/stock_analysis.db\n\
         MAGICLAW_DB_PATH=./data/magiclaw.db\n\
         STOCK_LIST=605178\n",
    )
    .expect("write production-oriented dotenv default");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--push-dry-run"])
        .env_remove("DATABASE_PATH")
        .env_remove("STOCK_LIST")
        .output()
        .expect("run isolated E2E with repository dotenv");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        output.status.success(),
        "dotenv production default leaked into test mode: {combined_output}"
    );
    assert!(
        combined_output.contains("[v70] E2E 完成"),
        "isolated E2E did not reach its completion marker: {combined_output}"
    );
    assert_br196_explicit_dry_run_summary(&combined_output);
    assert!(
        combined_output.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ),
        "test mode did not report the disabled selection capability"
    );
    assert!(
        !root.join("data/stock_analysis.db").exists(),
        "test mode opened the dotenv production database"
    );
    assert!(
        !root.join("data/magiclaw.db").exists(),
        "test mode opened the dotenv production Magiclaw database"
    );
    assert!(
        combined_output.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ),
        "test mode did not preserve the disabled selection summary: {combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn bare_test_mode_fails_closed_without_live_acceptance_opt_in() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-test-missing-feishu-target-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = isolated_monitor_command(&root)
        .arg("--test")
        .env("DATABASE_PATH", root.join("caller.db"))
        .env("MAGICLAW_DB_PATH", root.join("caller.db"))
        .env("STOCK_LIST", "")
        .env("STOCK_ENV_MODE", "test")
        .output()
        .expect("run bare test without a Feishu target");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains("live_acceptance_not_opted_in")
            && combined_output.contains("target_resolution=0")
            && combined_output.contains("external_process_attempted=0")
            && combined_output.contains("receipt_audit_appended=0"),
        "bare test did not fail before BR-196 authority construction: {combined_output}"
    );
    assert!(
        !combined_output.contains("[v70] E2E 完成"),
        "bare test claimed completion without a validated Feishu receipt: {combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn bare_test_live_opt_in_does_not_fall_back_to_default_feishu_target() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-test-no-target-fallback-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = isolated_monitor_command(&root)
        .arg("--test")
        .env("BR196_LIVE_FEISHU_ACCEPTANCE", "1")
        .env("DATABASE_PATH", root.join("caller.db"))
        .env("MAGICLAW_DB_PATH", root.join("caller.db"))
        .env("STOCK_LIST", "")
        .env("STOCK_ENV_MODE", "test")
        .output()
        .expect("run opted-in bare test without explicit BR-196 target fields");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains("BR-196 target identity missing field BR196_FEISHU_TENANT_ID"),
        "bare test did not fail before default target or process resolution: {combined_output}"
    );
    assert!(
        !combined_output.contains("MagicLaw spawn failed")
            && !combined_output.contains("[飞书] 开始推送"),
        "bare test reached an external notification process: {combined_output}"
    );

    let test_code = bound_durable_test_code(&combined_output);
    let durable_namespace = manifest_durable_test_namespace(&test_code);
    assert!(
        !durable_namespace.join("template_delivery_audit").exists(),
        "missing explicit target fields created a live receipt audit"
    );
    std::fs::remove_dir_all(durable_namespace)
        .expect("remove exact BR-196 durable TEST_CODE namespace");
    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn review_command_runs_without_service_enablement() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-review-without-enablement-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    let database_path = root.join("review.db");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", &database_path)
        .env("MAGICLAW_DB_PATH", &database_path)
        .env("STOCK_LIST", "TEST_CODE_000001")
        .env("STOCK_ENV_MODE", "test")
        .env("V10_DRY_RUN_PUSH", "1")
        .env("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "1")
        .env_remove("MONITOR_ENABLED")
        .env_remove("ALERT_WEBHOOK_URL")
        .env_remove("WECHAT_WEBHOOK")
        .output()
        .expect("run isolated strict review without service switch");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "strict review must execute and fail closed without service enablement; output={combined_output}"
    );
    assert!(
        combined_output.contains("[复盘] --review 终端模式启动"),
        "review command was short-circuited before execution; output={combined_output}"
    );
    assert!(
        !combined_output.contains("[jsonl_writer] fatal error")
            && !combined_output.contains("background task failed"),
        "event writer did not initialize cleanly; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn event_writer_initialization_failure_exits_nonzero() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-event-writer-init-failure-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    std::fs::write(root.join("data"), b"blocks runtime directory")
        .expect("create event directory blocker");
    let database_path = root.join("review.db");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", &database_path)
        .env("MAGICLAW_DB_PATH", &database_path)
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run monitor with blocked event directory");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "writer initialization failure must be terminal; output={combined_output}"
    );
    assert!(
        combined_output.contains("[event_bus.jsonl] initialization failed"),
        "writer initialization error was not explicit; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn corrupt_history_exits_nonzero_without_service_enablement() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-history-corrupt-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let event_dir = root.join("data/test/event_bus");
    std::fs::create_dir_all(&event_dir).expect("create isolated event directory");
    std::fs::write(event_dir.join("2026-07-21.jsonl"), b"{not-json}\n")
        .expect("seed corrupt history");
    let database_path = root.join("history.db");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--history", "--date=2026-07-21"])
        .env("DATABASE_PATH", &database_path)
        .env("MAGICLAW_DB_PATH", &database_path)
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run history against corrupt isolated JSONL");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "corrupt history must not report success; output={combined_output}"
    );
    assert!(
        combined_output.contains("[history] query failed"),
        "history failure was not explicit; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn corrupt_history_success_rate_exits_nonzero() {
    let root = std::env::temp_dir().join(format!(
        "monitor-history-rate-corrupt-{}",
        std::process::id()
    ));
    let event_dir = root.join("data/test/event_bus");
    std::fs::create_dir_all(&event_dir).expect("create isolated event directory");
    std::fs::write(event_dir.join("2026-07-21.jsonl"), b"{not-json}\n")
        .expect("seed corrupt history");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--history", "--success-rate", "--date=2026-07-21"])
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run corrupt history statistics");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(1), "output={combined_output}");
    assert!(
        combined_output.contains("[history] success_rate query failed"),
        "statistics failure was not explicit: {combined_output}"
    );
    std::fs::remove_dir_all(root).expect("remove isolated history directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn replay_missing_source_exits_nonzero_without_service_enablement() {
    let root = std::env::temp_dir().join(format!("monitor-replay-missing-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create isolated replay directory");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--replay=2099-12-31"])
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run replay with missing source");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(1), "output={combined_output}");
    assert!(
        combined_output.contains("[replay] failed"),
        "replay failure was not explicit: {combined_output}"
    );
    std::fs::remove_dir_all(root).expect("remove isolated replay directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn unknown_explicit_flag_never_enters_long_running_service() {
    let root = std::env::temp_dir().join(format!("monitor-unknown-flag-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create isolated CLI directory");
    let database_path = root.join("unknown.db");

    let output = isolated_monitor_command(&root)
        .arg("--unknown-flag")
        .env("DATABASE_PATH", &database_path)
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run unknown explicit flag");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(combined_output.contains("selection process bootstrap failed"));
    assert!(combined_output.contains("selection_cli_invalid"));
    assert!(!combined_output.contains("等待交易时段"));
    std::fs::remove_dir_all(root).expect("remove isolated CLI directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn removed_legacy_outcome_backfill_flag_is_rejected_before_runtime_state() {
    let root =
        std::env::temp_dir().join(format!("monitor-removed-backfill-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create isolated directory");
    for flag in [
        "--backfill-outcome=",
        "--backfill-outcome=../../escape",
        "--backfill-outcome=2026-07-21",
    ] {
        let output = isolated_monitor_command(&root)
            .arg(flag)
            .env("DATABASE_PATH", root.join("must-not-exist.db"))
            .env_remove("MONITOR_ENABLED")
            .output()
            .expect("run removed outcome backfill flag");
        let combined_output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(2), "output={combined_output}");
        assert!(
            combined_output.contains("unrecognized flag"),
            "{combined_output}"
        );
        assert!(!root.join("must-not-exist.db").exists());
    }
    std::fs::remove_dir_all(root).expect("remove isolated directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn registered_push_and_backfill_flags_reach_truthful_terminal_handlers() {
    for (label, flag, marker) in [
        ("st", "--backfill-st-type", "--backfill-st-type 模式启动"),
        (
            "chain",
            "--backfill-chain-name",
            "--backfill-chain-name 模式启动",
        ),
    ] {
        let root =
            std::env::temp_dir().join(format!("monitor-handler-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create handler directory");
        let output = isolated_monitor_command(&root)
            .args(["--test", flag])
            .env("DATABASE_PATH", root.join("handler.db"))
            .env_remove("MONITOR_ENABLED")
            .output()
            .expect("run registered handler");
        let combined_output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "output={combined_output}");
        assert!(combined_output.contains(marker), "output={combined_output}");
        assert!(!combined_output.contains("等待交易时段"));
        std::fs::remove_dir_all(root).expect("remove handler directory");
    }

    let root = std::env::temp_dir().join(format!("monitor-handler-dry-run-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create dry-run handler directory");
    let output = isolated_monitor_command(&root)
        .args(["--test", "--push-dry-run"])
        .env("DATABASE_PATH", root.join("dry-run.db"))
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run push dry-run handler");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "output={combined_output}");
    assert!(
        combined_output.contains("[v30] --test 模式启动")
            && combined_output.contains("[BR-196] V2 acceptance start"),
        "BR-196 dry-run did not reach the E2E terminal handler: {combined_output}"
    );
    assert_br196_explicit_dry_run_summary(&combined_output);
    assert!(combined_output.contains("[v70] E2E 完成"));
    assert!(!combined_output.contains("等待交易时段"));
    std::fs::remove_dir_all(root).expect("remove dry-run handler directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn fresh_test_database_starts_without_lock_errors() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-fresh-db-lock-check-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    let database_path = root.join("fresh.db");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", &database_path)
        .env_remove("ALERT_WEBHOOK_URL")
        .env("STOCK_LIST", "TEST_CODE_000001")
        .env("STOCK_ENV_MODE", "test")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
        .env("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "0")
        .output()
        .expect("run monitor with a fresh isolated database");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        matches!(output.status.code(), Some(0 | 2)),
        "review may succeed only through independent non-account reports; output={combined_output}"
    );
    if output.status.code() == Some(0) {
        assert!(
            combined_output.contains("盘后分析完成"),
            "successful review must have a truthful terminal marker; output={combined_output}"
        );
    } else {
        assert!(
            combined_output.contains("严格盘后复盘没有任何确认投递"),
            "zero-delivery review must fail explicitly; output={combined_output}"
        );
    }
    assert!(
        !combined_output.contains("database is locked"),
        "fresh database startup must not race WAL initialization; output={combined_output}"
    );
    assert!(
        combined_output.contains(
            "[复盘依赖][BR-194] dependency=legacy_account_gate status=unavailable"
        ) && combined_output.contains("stage=acquire_batch")
            && combined_output.contains("reason_code=account_metrics_incomplete")
            && combined_output.contains("retryable=true")
            && combined_output.contains("source_provider=none")
            && combined_output.contains("source_time=none"),
        "fresh database must preserve the typed account dependency failure; output={combined_output}"
    );
    assert!(
        !combined_output.contains("[AccountMode-hook] 启动评估"),
        "review startup must not restore the retired account-notification batch gate; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

fn assert_isolated_e2e_reaches_the_final_completion_marker(
    label: &str,
    arguments: &[&str],
    repetitions: usize,
) {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-isolated-e2e-{}-{}-{}",
        label,
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated e2e directory");
    let caller_database_path = root.join("e2e.db");
    let mut last_bound_database_path = None;
    let mut durable_namespaces = Vec::with_capacity(repetitions);

    for run in 1..=repetitions {
        let output = isolated_monitor_command(&root)
            .args(arguments)
            .env("DATABASE_PATH", &caller_database_path)
            .env("MAGICLAW_DB_PATH", &caller_database_path)
            .env("STOCK_LIST", "")
            .env("STOCK_ENV_MODE", "test")
            .env("MONITOR_ENABLED", "true")
            .env("V10_DRY_RUN_PUSH", "1")
            .env_remove("ALERT_WEBHOOK_URL")
            .env_remove("CUSTOM_WEBHOOK_URL")
            .env_remove("DINGTALK_WEBHOOK")
            .env_remove("DISCORD_WEBHOOK")
            .env_remove("FEISHU_APP_ID")
            .env_remove("FEISHU_APP_SECRET")
            .env_remove("FEISHU_TO")
            .env_remove("FEISHU_WEBHOOK")
            .env_remove("SERVER_CHAN_KEY")
            .env_remove("SLACK_WEBHOOK")
            .env_remove("TELEGRAM_BOT_TOKEN")
            .env_remove("WECHAT_WEBHOOK")
            .output()
            .expect("run isolated monitor e2e");

        let combined_output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "isolated e2e run {run} failed: {combined_output}"
        );
        let test_code = bound_durable_test_code(&combined_output);
        durable_namespaces.push(manifest_durable_test_namespace(&test_code));
        assert!(
            combined_output.contains("[v70] E2E 完成"),
            "run {run} exited zero without the final e2e commit marker: {combined_output}"
        );
        assert_br196_explicit_dry_run_summary(&combined_output);
        assert!(
            !combined_output.contains("governance banner unavailable"),
            "isolated E2E run {run} did not install its TEST_CODE governance context: {combined_output}"
        );
        for unavailable in [
            "capability_unavailable=review_lhb_counted_binding_unavailable",
            "capability_unavailable=review_signal_counted_binding_unavailable",
        ] {
            assert!(
                combined_output.contains(unavailable),
                "isolated E2E run {run} did not expose counted capability boundary {unavailable}: {combined_output}"
            );
        }
        assert!(
            !combined_output.contains("counted_binding_required"),
            "isolated E2E run {run} attempted a generic counted delivery: {combined_output}"
        );
        assert!(
            !combined_output.contains("[DataGateway][R-03]")
                && !combined_output.contains("[DataGateway][A-10"),
            "isolated E2E run {run} reached an external review gateway: {combined_output}"
        );
        let bound_database_path = initialized_database_path(&combined_output);
        assert!(
            bound_database_path.components().any(|component| component
                .as_os_str()
                .to_string_lossy()
                .starts_with("TEST_CODE_monitor_")),
            "isolated E2E did not use an invocation-unique TEST_CODE database: {}",
            bound_database_path.display()
        );
        assert_ne!(bound_database_path, caller_database_path);
        last_bound_database_path = Some(bound_database_path);
    }
    assert!(
        !caller_database_path.exists(),
        "isolated E2E honored caller-controlled DATABASE_PATH"
    );
    assert!(
        !root.join("data/stock_analysis.db").exists(),
        "isolated e2e created a production database path"
    );
    assert!(
        !root.join("data/d01_recommendations").exists(),
        "isolated e2e wrote the production recommendation namespace"
    );
    assert!(
        !root.join("data/test/d01_recommendations").exists(),
        "BR-196 formatting acceptance must not persist obsolete recommendation fixtures"
    );
    let trade_count = Command::new("sqlite3")
        .args([
            last_bound_database_path
                .as_deref()
                .expect("at least one bound database")
                .as_os_str(),
            std::ffi::OsStr::new("SELECT COUNT(*) FROM trades WHERE code='TEST_CODE_TRADE_V2'"),
        ])
        .output()
        .expect("query isolated fixture trades");
    assert!(trade_count.status.success(), "fixture trade query failed");
    assert_eq!(
        String::from_utf8_lossy(&trade_count.stdout).trim(),
        "0",
        "BR-196 formatting acceptance must not persist obsolete trade fixtures"
    );
    let analytics_path = root.join("data/test/push_analytics.db");
    let analytics_count = Command::new("sqlite3")
        .args([
            analytics_path.as_os_str(),
            std::ffi::OsStr::new("SELECT COUNT(*) FROM push_analytics"),
        ])
        .output()
        .expect("query isolated push analytics");
    assert!(
        analytics_count.status.success(),
        "isolated L7 audit query failed"
    );
    assert!(
        String::from_utf8_lossy(&analytics_count.stdout)
            .trim()
            .parse::<u64>()
            .is_ok_and(|count| count > 0),
        "isolated E2E did not persist any L7 delivery decision"
    );

    for durable_namespace in durable_namespaces {
        std::fs::remove_dir_all(&durable_namespace)
            .expect("remove exact repository-anchored E2E durable TEST_CODE namespace");
    }
    std::fs::remove_dir_all(&root).expect("remove isolated e2e directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn explicit_test_dry_run_reaches_the_final_completion_marker() {
    assert_isolated_e2e_reaches_the_final_completion_marker(
        "test-dry-run",
        &["--test", "--push-dry-run"],
        2,
    );
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_dry_run_isolates_inherited_health_webhook_before_http() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-test-health-webhook-isolation-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated webhook test directory");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--push-dry-run"])
        .env(
            "ALERT_WEBHOOK_URL",
            "http://127.0.0.1:9/TEST_CODE_FORBIDDEN_WEBHOOK",
        )
        .output()
        .expect("run dry-run with inherited health webhook");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "output={combined_output}");
    assert!(
        combined_output.contains(
            "[webhook][BR-051][BR-196] test environment notification isolated: health_check_fail"
        ) && combined_output.contains("[health] 测试环境已隔离健康告警外发"),
        "test webhook isolation was not explicit: {combined_output}"
    );
    assert!(
        !combined_output.contains("webhook POST")
            && !combined_output.contains("TEST_CODE_FORBIDDEN_WEBHOOK"),
        "test mode attempted to resolve or send the configured webhook: {combined_output}"
    );

    let test_code = bound_durable_test_code(&combined_output);
    std::fs::remove_dir_all(manifest_durable_test_namespace(&test_code))
        .expect("remove exact webhook-test durable TEST_CODE namespace");
    std::fs::remove_dir_all(root).expect("remove isolated webhook test directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn br196_dry_run_uses_scoped_governance_clock_under_caller_quiet_policy() {
    let root = std::env::temp_dir().join(format!(
        "monitor-br196-scoped-governance-clock-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create scoped-governance-clock directory");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--push-dry-run"])
        .env("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "1")
        .output()
        .expect("run BR-196 dry-run under explicit caller quiet policy");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(output.status.success(), "output={combined_output}");
    assert!(
        combined_output.contains("[v70] E2E 完成"),
        "scoped BR-196 governance clock did not reach completion: {combined_output}"
    );
    assert!(
        !combined_output.contains("governance smoke non-Pushed"),
        "caller quiet policy leaked into the BR-196 exact-six smoke: {combined_output}"
    );

    let test_code = bound_durable_test_code(&combined_output);
    std::fs::remove_dir_all(manifest_durable_test_namespace(&test_code))
        .expect("remove exact BR-196 durable TEST_CODE namespace");
    std::fs::remove_dir_all(root).expect("remove scoped-governance-clock directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn explicit_e2e_reaches_the_final_completion_marker() {
    assert_isolated_e2e_reaches_the_final_completion_marker(
        "explicit",
        &["--test", "--e2e", "--push-dry-run"],
        1,
    );
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn v13_diagnostics_commit_an_isolated_report_without_external_market_calls() {
    let root = std::env::temp_dir().join(format!("monitor-v13-diag-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create isolated diagnostic directory");
    let legacy_audit_dir = root.join("data/test/event_audit");
    std::fs::create_dir_all(&legacy_audit_dir).expect("create legacy audit directory");
    let legacy_audit =
        legacy_audit_dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
    let legacy_bytes = b"{historical-forked-test-audit}\n";
    std::fs::write(&legacy_audit, legacy_bytes).expect("seed immutable corrupt legacy audit");
    let database_path = root.join("diag.db");
    let output = isolated_monitor_command(&root)
        .args(["--test", "--v13-diag"])
        .env("DATABASE_PATH", &database_path)
        .env("MAGICLAW_DB_PATH", &database_path)
        .env("STOCK_ENV_MODE", "test")
        .env("STOCK_LIST", "")
        .env("MONITOR_ENABLED", "true")
        .env_remove("ALERT_WEBHOOK_URL")
        .env_remove("FEISHU_APP_ID")
        .env_remove("FEISHU_APP_SECRET")
        .env_remove("WECHAT_WEBHOOK")
        .output()
        .expect("run isolated v13 diagnostics");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "v13 diagnostics failed: {combined}"
    );
    assert!(combined.contains("总步骤: 14"));
    assert!(combined.contains("BR-051 isolated diagnostics skip external"));
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("data/v13_diag_report.json")).expect("read diagnostic report"),
    )
    .expect("parse diagnostic report");
    assert_eq!(report["total_steps"], 14);
    let test_code = bound_durable_test_code(&combined);
    let durable_namespace = manifest_durable_test_namespace(&test_code);
    assert!(
        durable_namespace.join("event_audit").is_dir(),
        "repository-anchored TEST_CODE audit namespace was not initialized"
    );
    assert_eq!(
        std::fs::read(&legacy_audit).expect("read preserved legacy audit"),
        legacy_bytes,
        "legacy audit evidence must remain byte-for-byte unchanged"
    );
    std::fs::remove_dir_all(&durable_namespace)
        .expect("remove exact repository-anchored TEST_CODE diagnostic namespace");
    std::fs::remove_dir_all(root).expect("remove isolated diagnostic directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_binding_ignores_caller_memory_database_override() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-memory-db-rejection-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", ":memory:")
        .env_remove("ALERT_WEBHOOK_URL")
        .env("STOCK_LIST", "TEST_CODE_000001")
        .env("STOCK_ENV_MODE", "test")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
        .output()
        .expect("run monitor with an in-memory database");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        matches!(output.status.code(), Some(0 | 2)),
        "review may complete only through independent non-account reports; output={combined_output}"
    );
    assert!(
        combined_output.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ) && !combined_output.contains("journal_mode mismatch"),
        "caller memory override reached database construction: output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
#[serial_test::serial(durable_physical_isolation)]
fn test_binding_ignores_caller_database_parent_override() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-db-parent-failure-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"blocks create_dir_all").expect("create blocking regular file");
    let database_path = blocker.join("fresh.db");

    let output = isolated_monitor_command(&root)
        .args(["--test", "--review"])
        .env("DATABASE_PATH", &database_path)
        .env_remove("ALERT_WEBHOOK_URL")
        .env("STOCK_LIST", "TEST_CODE_000001")
        .env("STOCK_ENV_MODE", "test")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
        .output()
        .expect("run monitor with an invalid database parent");

    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        matches!(output.status.code(), Some(0 | 2)),
        "review may complete only through independent non-account reports; output={combined_output}"
    );
    assert!(
        combined_output.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ) && !combined_output.contains("[DB init] 创建目录"),
        "caller database parent override reached database construction: output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}
