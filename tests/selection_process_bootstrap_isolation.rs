//! BR-051/BR-179 executable boundary tests for the process-owned bootstrap.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    exists: bool,
    len: Option<u64>,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: Option<u64>,
    #[cfg(unix)]
    inode: Option<u64>,
}

fn fingerprint(path: &std::path::Path) -> FileFingerprint {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            FileFingerprint {
                exists: true,
                len: Some(metadata.len()),
                modified: metadata.modified().ok(),
                #[cfg(unix)]
                device: Some(metadata.dev()),
                #[cfg(unix)]
                inode: Some(metadata.ino()),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileFingerprint {
            exists: false,
            len: None,
            modified: None,
            #[cfg(unix)]
            device: None,
            #[cfg(unix)]
            inode: None,
        },
        Err(error) => panic!("fingerprint {}: {error}", path.display()),
    }
}

fn production_fingerprints() -> Vec<(std::path::PathBuf, FileFingerprint)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        "data/stock_analysis.db",
        "data/stock_analysis.db-wal",
        "data/stock_analysis.db-shm",
        "data/magiclaw.db",
        "data/audit/production/selection-audit.jsonl",
        "data/audit/production/selection-audit.lock",
    ]
    .into_iter()
    .map(|relative| {
        let path = root.join(relative);
        let state = fingerprint(&path);
        (path, state)
    })
    .collect()
}

fn isolated_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "TEST_CODE_selection-bootstrap-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated bootstrap root");
    root
}

fn monitor(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_monitor"));
    command
        .current_dir(root)
        .env("DATABASE_PATH", root.join("caller-controlled.db"))
        .env(
            "MAGICLAW_DB_PATH",
            root.join("caller-controlled-magiclaw.db"),
        )
        .env_remove("MONITOR_ENABLED");
    command
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_storage_free(
    root: &std::path::Path,
    production_before: &[(std::path::PathBuf, FileFingerprint)],
) {
    assert!(
        !root.join("caller-controlled.db").exists(),
        "bootstrap honored caller-controlled DATABASE_PATH"
    );
    assert!(
        !root.join("caller-controlled-magiclaw.db").exists(),
        "bootstrap honored caller-controlled MAGICLAW_DB_PATH"
    );
    assert!(
        !root.join("data").exists(),
        "terminal/rejected bootstrap created an operational namespace"
    );
    for (path, before) in production_before {
        assert_eq!(
            &fingerprint(path),
            before,
            "terminal/rejected bootstrap changed production state at {}",
            path.display()
        );
    }
}

#[test]
fn exact_help_is_a_storage_free_terminal_process() {
    let root = isolated_root("help");
    let production_before = production_fingerprints();
    let output = monitor(&root)
        .arg("--help")
        .output()
        .expect("run exact help");
    let combined = combined_output(&output);

    assert!(output.status.success(), "help failed: {combined}");
    assert!(combined.contains("monitor --help"), "{combined}");
    assert!(
        combined.contains("monitor --compensate=P-01 --business-date=YYYY-MM-DD"),
        "P-01 compensation command is missing from help: {combined}"
    );
    assert_storage_free(&root, &production_before);
    std::fs::remove_dir_all(root).expect("remove help root");
}

#[test]
fn exact_version_is_a_storage_free_terminal_process() {
    let root = isolated_root("version");
    let production_before = production_fingerprints();
    let output = monitor(&root)
        .arg("--version")
        .output()
        .expect("run exact version");
    let combined = combined_output(&output);

    assert!(output.status.success(), "version failed: {combined}");
    assert!(combined.contains(env!("CARGO_PKG_VERSION")), "{combined}");
    assert_storage_free(&root, &production_before);
    std::fs::remove_dir_all(root).expect("remove version root");
}

#[test]
fn p01_compensation_cli_publishes_only_an_opaque_typed_request() {
    let source = include_str!("../src/selection/process_bootstrap.rs");

    assert!(source.contains("pub struct P01CompensationRequest"));
    assert!(source.contains("_private: ()"));
    assert!(source.contains("pub fn p01_compensation_request(&self)"));
    assert!(!source.contains("pub fn p01_compensation_business_date(&self)"));
    assert!(source.contains("explicit_args.len() != 2"));
    assert!(source.contains("argument == \"--compensate=P-01\""));
}

#[test]
fn invalid_argv_is_a_storage_free_rejected_process() {
    let root = isolated_root("invalid");
    let production_before = production_fingerprints();
    let output = monitor(&root)
        .arg("--unsupported-bootstrap-flag")
        .output()
        .expect("run invalid argv");
    let combined = combined_output(&output);

    assert_eq!(output.status.code(), Some(2), "{combined}");
    assert!(combined.contains("selection_cli_invalid"), "{combined}");
    assert_storage_free(&root, &production_before);
    std::fs::remove_dir_all(root).expect("remove invalid root");
}

#[test]
fn operational_argv_runs_core_with_unverified_selection_disabled() {
    let root = isolated_root("operational-core");
    let production_before = production_fingerprints();
    let output = monitor(&root)
        .args(["--test", "--review"])
        .env("STOCK_LIST", "")
        .env("V10_DRY_RUN_PUSH", "1")
        .env("MONITOR_REVIEW_TIMEOUT_SECS", "1")
        .output()
        .expect("run operational argv");
    let combined = combined_output(&output);

    assert!(matches!(output.status.code(), Some(0 | 2)), "{combined}");
    assert!(
        !combined.contains("selection_operational_binding_unavailable"),
        "{combined}"
    );
    assert!(
        combined.contains(
            "[selection-v2][BR-183] capability=disabled reason_code=board_artifact_unverified providers=0 database_operations=0 sinks=0 schedulers=0"
        ),
        "{combined}"
    );
    assert!(
        combined.contains("[复盘] --review 终端模式启动"),
        "{combined}"
    );
    for (path, before) in &production_before {
        assert_eq!(
            &fingerprint(path),
            before,
            "isolated operational test changed production state at {}",
            path.display()
        );
    }
    std::fs::remove_dir_all(root).expect("remove operational root");
}
