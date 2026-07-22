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
        "V12_E2E_REAL_PUSH",
        "STOCK_ANALYSIS_PUSH_V6_ENABLE",
        "EVENT_AUDIT_DIR",
        "PUSH_LOG_DIR",
        "DISPATCHER_LOG_DIR",
        "REVIEW_AUDIT_DIR",
        "ALERT_WEBHOOK_URL",
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

#[test]
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
        combined_output.contains("--v13-diag requires --test"),
        "missing explicit mode error: {combined_output}"
    );
    assert!(
        !root.join("data").exists(),
        "invalid diagnostic mode initialized runtime state"
    );

    std::fs::remove_dir_all(root).expect("remove isolated diagnostic directory");
}

#[test]
fn normal_process_initializes_governance_before_waiting_for_market() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-startup-governance-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    let database_path = root.join("startup.db");
    let stdout_path = root.join("monitor.stdout.log");
    let stderr_path = root.join("monitor.stderr.log");
    let stdout_file = std::fs::File::create(&stdout_path).expect("create monitor stdout log");
    let stderr_file = std::fs::File::create(&stderr_path).expect("create monitor stderr log");

    let mut child = isolated_monitor_command(&root)
        .env("DATABASE_PATH", &database_path)
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
        if combined_output.contains("[AccountMode-hook] 启动评估")
            && combined_output.contains("[DataMode-hook] 模式")
        {
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
        combined_output.contains("[AccountMode-hook] 启动评估"),
        "normal startup must evaluate AccountMode before session wait; output={combined_output}"
    );
    assert!(
        combined_output.contains("[DataMode-hook] 模式"),
        "normal startup must evaluate DataMode before session wait; output={combined_output}"
    );
    assert!(
        !combined_output.contains("governance banner unavailable"),
        "long-running loops started before governance context; output={combined_output}"
    );
    assert!(database_path.exists(), "startup database was not created");

    std::fs::remove_dir_all(root).expect("remove isolated startup directory");
}

#[test]
fn test_mode_rejects_production_database_with_nonzero_exit() {
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

    assert_eq!(
        output.status.code(),
        Some(2),
        "BR-051 rejection must not report success; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("BR-051"),
        "explicit rejection missing: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !root.join("data/stock_analysis.db").exists(),
        "test mode opened the forbidden production DB path"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
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

    assert_eq!(output.status.code(), Some(1), "output={combined_output}");
    assert!(combined_output.contains("[event] CLI error"));
    assert!(!combined_output.contains("等待交易时段"));
    std::fs::remove_dir_all(root).expect("remove isolated CLI directory");
}

#[test]
fn invalid_or_corrupt_outcome_backfill_is_never_reported_as_success() {
    let invalid_root = std::env::temp_dir().join(format!(
        "monitor-invalid-backfill-date-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&invalid_root).expect("create invalid backfill directory");
    for flag in ["--backfill-outcome=", "--backfill-outcome=../../escape"] {
        let output = isolated_monitor_command(&invalid_root)
            .arg(flag)
            .env("DATABASE_PATH", invalid_root.join("invalid.db"))
            .env_remove("MONITOR_ENABLED")
            .output()
            .expect("run invalid outcome backfill");
        let combined_output = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(1), "output={combined_output}");
        assert!(
            combined_output.contains("malformed date"),
            "{combined_output}"
        );
    }
    std::fs::remove_dir_all(invalid_root).expect("remove invalid backfill directory");

    let corrupt_root =
        std::env::temp_dir().join(format!("monitor-corrupt-backfill-{}", std::process::id()));
    let source_dir = corrupt_root.join("data/d01_recommendations");
    std::fs::create_dir_all(&source_dir).expect("create corrupt outcome directory");
    std::fs::write(source_dir.join("2026-07-21.jsonl"), b"{not-json}\n")
        .expect("seed corrupt outcome file");
    let output = isolated_monitor_command(&corrupt_root)
        .arg("--backfill-outcome=2026-07-21")
        .env("DATABASE_PATH", corrupt_root.join("corrupt.db"))
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run corrupt outcome backfill");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(2), "output={combined_output}");
    assert!(
        combined_output.contains("[v70+] 回填失败"),
        "{combined_output}"
    );
    assert!(!combined_output.contains("[v70+] 回填完成"));
    std::fs::remove_dir_all(corrupt_root).expect("remove corrupt backfill directory");
}

#[test]
fn registered_push_and_backfill_flags_reach_truthful_terminal_handlers() {
    for (label, flag, marker) in [
        ("st", "--backfill-st-type", "--backfill-st-type 模式启动"),
        (
            "chain",
            "--backfill-chain-name",
            "--backfill-chain-name 模式启动",
        ),
        (
            "outcome",
            "--backfill-outcome=2099-12-31",
            "--backfill-outcome 模式启动",
        ),
    ] {
        let root =
            std::env::temp_dir().join(format!("monitor-handler-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create handler directory");
        let output = isolated_monitor_command(&root)
            .arg(flag)
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
        .arg("--push-dry-run")
        .env("DATABASE_PATH", root.join("dry-run.db"))
        .env_remove("MONITOR_ENABLED")
        .output()
        .expect("run push dry-run handler");
    let combined_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined_output.contains("--push-dry-run 模式启动"));
    if combined_output.contains("--push-dry-run 失败") {
        assert_eq!(output.status.code(), Some(2), "output={combined_output}");
        assert!(!combined_output.contains("--push-dry-run 完成"));
    } else {
        assert!(output.status.success(), "output={combined_output}");
        assert!(combined_output.contains("--push-dry-run 完成"));
    }
    assert!(!combined_output.contains("等待交易时段"));
    std::fs::remove_dir_all(root).expect("remove dry-run handler directory");
}

#[test]
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
    assert_eq!(
        output.status.code(),
        Some(2),
        "missing fresh real account evidence must fail closed; output={combined_output}"
    );
    assert!(
        !combined_output.contains("database is locked"),
        "fresh database startup must not race WAL initialization; output={combined_output}"
    );
    assert!(
        combined_output.contains("[AccountMode-hook][BR-108]")
            && combined_output.contains("BR-103 real account snapshot is missing")
            && combined_output.contains("event_type=push.delivery.audit")
            && combined_output.contains("kind=account_mode_v1 outcome=Pushed"),
        "fresh database must emit an audited conservative account alert; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

fn assert_isolated_e2e_reaches_the_final_completion_marker(label: &str, arguments: &[&str]) {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-isolated-e2e-{}-{}-{}",
        label,
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated e2e directory");
    let database_path = root.join("e2e.db");

    let output = isolated_monitor_command(&root)
        .args(arguments)
        .env("DATABASE_PATH", &database_path)
        .env("MAGICLAW_DB_PATH", &database_path)
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
        "isolated e2e failed: {combined_output}"
    );
    assert!(
        combined_output.contains("[v70] E2E 完成"),
        "exit zero without the final e2e commit marker: {combined_output}"
    );
    assert!(
        !combined_output.contains("governance banner unavailable"),
        "isolated E2E did not install its TEST_CODE governance context: {combined_output}"
    );
    assert!(
        database_path.exists(),
        "isolated e2e database was not created"
    );
    assert!(
        !root.join("data/stock_analysis.db").exists(),
        "isolated e2e created a production database path"
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

    std::fs::remove_dir_all(&root).expect("remove isolated e2e directory");
}

#[test]
fn bare_test_alias_reaches_the_final_completion_marker() {
    assert_isolated_e2e_reaches_the_final_completion_marker("bare", &["--test"]);
}

#[test]
fn explicit_e2e_reaches_the_final_completion_marker() {
    assert_isolated_e2e_reaches_the_final_completion_marker("explicit", &["--test", "--e2e"]);
}

#[test]
fn v13_diagnostics_commit_an_isolated_report_without_external_market_calls() {
    let root = std::env::temp_dir().join(format!("monitor-v13-diag-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create isolated diagnostic directory");
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
    std::fs::remove_dir_all(root).expect("remove isolated diagnostic directory");
}

#[test]
fn memory_database_fails_closed_with_explicit_journal_mode_error() {
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
    assert_eq!(
        output.status.code(),
        Some(2),
        "DB init must fail closed; output={combined_output}"
    );
    assert!(
        combined_output.contains("journal_mode") && combined_output.contains("memory"),
        "non-WAL journal mode must be explicit; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}

#[test]
fn database_parent_creation_failure_exits_nonzero() {
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
    assert_eq!(
        output.status.code(),
        Some(2),
        "database parent creation failure must fail closed; output={combined_output}"
    );
    assert!(
        combined_output.contains("[DB init] 创建目录") && combined_output.contains("失败"),
        "database parent failure must be explicit; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}
