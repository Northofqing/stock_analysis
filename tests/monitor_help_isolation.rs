//! BR-051: terminal help must not initialize production data or audit paths.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn help_exits_without_creating_runtime_state() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-help-isolation-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");

    let output = Command::new(env!("CARGO_BIN_EXE_monitor"))
        .arg("--help")
        .current_dir(&root)
        .env_remove("DATABASE_PATH")
        .env_remove("EVENT_AUDIT_DIR")
        .env_remove("PUSH_LOG_DIR")
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
    assert!(
        !root.join("data").exists(),
        "help command created runtime data under {}",
        root.display()
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
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

    let output = Command::new(env!("CARGO_BIN_EXE_monitor"))
        .args(["--test", "--review"])
        .current_dir(&root)
        .env("DATABASE_PATH", "./data/stock_analysis.db")
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
fn fresh_test_database_starts_without_lock_errors() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "monitor-fresh-db-lock-check-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated working directory");
    let database_path = root.join("fresh.db");

    let output = Command::new(env!("CARGO_BIN_EXE_monitor"))
        .args(["--test", "--review"])
        .current_dir(&root)
        .env("DATABASE_PATH", &database_path)
        .env("STOCK_LIST", "TEST_CODE_000001")
        .env("STOCK_ENV_MODE", "test")
        .env("MONITOR_ENABLED", "true")
        .env("V10_DRY_RUN_PUSH", "1")
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
        "missing same-day real ledger must fail closed; output={combined_output}"
    );
    assert!(
        !combined_output.contains("database is locked"),
        "fresh database startup must not race WAL initialization; output={combined_output}"
    );

    std::fs::remove_dir_all(&root).expect("remove isolated working directory");
}
