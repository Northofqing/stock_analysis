//! AGENTS.md §2.9 / BR-014 / BR-096 fail-closed contract regression tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn fixture_dir() -> PathBuf {
    let path = std::env::temp_dir()
        .join("stock_analysis_design_contract_tests")
        .join(format!(
            "{}-{}",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&path).expect("应能创建临时合同目录");
    path
}

fn run_check(config: Option<&Path>, source: Option<&Path>) -> Output {
    let mut command = Command::new("bash");
    command.arg(repo_path(
        "tools/compliance/lib/check_design_contradiction.sh",
    ));
    if let Some(path) = config {
        command.env("DESIGN_THRESHOLD_CONFIG", path);
    }
    if let Some(path) = source {
        command.env("DESIGN_SCORE_SOURCE", path);
    }
    command
        .output()
        .expect("应能运行 check_design_contradiction.sh")
}

fn valid_source() -> &'static str {
    r#"
pub const EVENT_RISK_SCORE_MAX: u8 = 100;
fn valid_push_threshold(push_threshold: u8) -> bool { push_threshold <= 100 }
fn cap(push_threshold: u8) -> u8 {
    push_threshold.checked_sub(1)
        .filter(|_| valid_push_threshold(push_threshold))
        .unwrap_or(0)
}
"#
}

#[test]
fn current_threshold_contract_passes() {
    let output = run_check(None, None);
    assert!(
        output.status.success(),
        "当前合同应通过: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn threshold_above_score_domain_fails() {
    let dir = fixture_dir();
    let config = dir.join("strategy.toml");
    let source = dir.join("score.rs");
    std::fs::write(&config, "opportunity_push_threshold = 101\n").unwrap();
    std::fs::write(&source, valid_source()).unwrap();

    let output = run_check(Some(&config), Some(&source));
    assert!(!output.status.success(), "101 分推送门必须 fail closed");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("必须位于 1..=100"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn obsolete_nested_threshold_is_not_accepted_as_root_config() {
    let dir = fixture_dir();
    let config = dir.join("strategy.toml");
    let source = dir.join("score.rs");
    std::fs::write(&config, "[push]\nevent_risk_score_threshold = 60\n").unwrap();
    std::fs::write(&source, valid_source()).unwrap();

    let output = run_check(Some(&config), Some(&source));
    assert!(!output.status.success(), "未被运行时读取的嵌套阈值不得通过");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("配置根级缺少整数字段 opportunity_push_threshold"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_rust_bound_fails_instead_of_guessing_nearby_numbers() {
    let dir = fixture_dir();
    let config = dir.join("strategy.toml");
    let source = dir.join("score.rs");
    std::fs::write(&config, "opportunity_push_threshold = 75\n").unwrap();
    std::fs::write(
        &source,
        "// event_risk_score 附近有很多数字 70, 80, 100，但没有合同常量\n",
    )
    .unwrap();

    let output = run_check(Some(&config), Some(&source));
    assert!(!output.status.success(), "缺少 Rust 合同常量时必须失败");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Rust 实现缺少常量 EVENT_RISK_SCORE_MAX"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
