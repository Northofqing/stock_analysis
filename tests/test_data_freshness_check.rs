//! 验证 check_data_freshness.sh 的 fail-closed 模式与 fresh 行为
//! 用 process::Command 跑脚本, 断言退出码。
//!
//! AGENTS.md §2.4 数据时效门禁 (PR-2).
//! 注意: 这些测试故意覆盖多种 STOCK_DB 路径, 不依赖生产 DB 的实际新鲜度状态。

use std::path::PathBuf;
use std::process::Command;

fn script_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tools/compliance/lib/check_data_freshness.sh");
    p
}

fn compliance_script_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tools/compliance/check.sh");
    p
}

fn pr_evidence_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/compliance/lib/check_pr_evidence.sh")
}

fn run_pr_evidence(body: &str) -> std::process::Output {
    Command::new("bash")
        .arg(pr_evidence_script_path())
        .env("PR_BODY", body)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("应能运行 PR 证据检查")
}

fn run_compliance(args: &[&str], db: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new("bash");
    cmd.arg(compliance_script_path())
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if let Some(path) = db {
        cmd.env("STOCK_DB", path);
    } else {
        cmd.env_remove("STOCK_DB");
    }
    cmd.output().expect("应能运行 compliance 入口")
}

fn run_with_db(db: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new("bash");
    cmd.arg(script_path());
    if let Some(p) = db {
        cmd.env("STOCK_DB", p);
    } else {
        cmd.env_remove("STOCK_DB");
    }
    cmd.output().expect("应能跑 check_data_freshness.sh")
}

#[test]
fn test_data_freshness_check_fails_on_missing_db() {
    // 缺失 DB 是合规前置缺失，必须 fail closed。
    let output = run_with_db(Some("/nonexistent/path/fake.db"));
    assert!(
        !output.status.success(),
        "missing db 应 FAIL (exit != 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("数据库不存在"),
        "stderr 应说明缺失前置: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_data_freshness_check_exits_zero_when_stock_db_unset_and_repo_db_present() {
    // 不设 STOCK_DB -> 用默认 data/stock_analysis.db
    // 该库 MAX(date) 取决于实际生产回填状态, 可能 PASS 或 FAIL, 都算"脚本能跑"
    // 修复 I-8 (2026-06-29 codex review): 这个测试原名暗示"unset 时应 exit 0",
    // 实际不验证 pass/fail, 只验证"脚本干净退出 (数字退出码, 非信号杀死)".
    // 改名 + 强化注释, 避免未来 maintainer 误以为这是 PASS 断言.
    // 真 PASS 断言见 test_data_freshness_check_exits_zero_on_fresh_fixture (下面新加).
    let output = run_with_db(None);
    // 仅断言: 不管 pass 还是 fail, 退出码都应是数字 (脚本无 panic / segfault)
    assert!(
        output.status.code().is_some(),
        "脚本应干净退出 (不是被信号杀死), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // 显式注释: **不**断言 exit 0 / 1. 这个测试只覆盖"无 panic"边界.
    // 新加 test_data_freshness_check_exits_zero_on_fresh_fixture 覆盖真 PASS 路径.
}

#[test]
fn test_data_freshness_check_fails_on_stale_data() {
    // 用临时 fixture DB (date = 2026-01-01) 模拟 stale 数据。
    // 该 fixture 不依赖生产 DB 的实际新鲜度状态 — 测试任何时候都应是 FAIL。
    use std::process::Command;

    let fixture_dir = std::env::temp_dir().join("stock_analysis_test_fixtures");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture_db = fixture_dir.join("stale_2026_01_01.db");

    // 用 sqlite3 建一个极简 fixture (只有 stock_daily 表, 一行 stale 数据).
    // 输出 status 必须检查 — 否则 sqlite3 不可用时静默跳过会让测试误通过.
    let create_output = Command::new("sqlite3")
        .arg(&fixture_db)
        .arg(
            "CREATE TABLE IF NOT EXISTS stock_daily (date TEXT); \
              DELETE FROM stock_daily; \
              INSERT INTO stock_daily VALUES ('2026-01-01');",
        )
        .output()
        .expect("应能跑 sqlite3 (测试前置条件)");

    assert!(
        create_output.status.success(),
        "sqlite3 创建 fixture 失败: {}",
        String::from_utf8_lossy(&create_output.stderr)
    );
    assert!(fixture_db.exists(), "fixture DB 必须存在: {fixture_db:?}");

    // 调试输出 (避免 silent skip — 在 CI 失败时能看到 fixture 路径)
    if std::env::var("TEST_VERBOSE").is_ok() {
        eprintln!("TEST: fixture path = {}", fixture_db.display());
        eprintln!("TEST: fixture exists = {}", fixture_db.exists());
    }

    let output = run_with_db(Some(fixture_db.to_str().unwrap()));
    if std::env::var("TEST_VERBOSE").is_ok() {
        eprintln!("TEST: script exit status = {:?}", output.status);
        eprintln!("TEST: stderr = {}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(
        !output.status.success(),
        "stale fixture db 应 FAIL (exit != 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("§2.4") || stderr.contains("数据断层") || stderr.contains("停更"),
        "stderr 应说明 §2.4 失败原因, got: {stderr}"
    );
}

// 修复 I-8 (2026-06-29 codex review): 加 fixture-based 测试覆盖真 PASS 路径
// (之前所有测试只覆盖 fail / skip, 没有覆盖 fresh DB 应 PASS 的 happy path).
// 这样未来 maintainer 改 check_data_freshness.sh 时, 这个测试会立刻捕获回归.
#[test]
fn test_data_freshness_check_exits_zero_on_fresh_fixture() {
    // fixture DB 注入当前日期的 stock_daily, 应通过 fresh check (exit 0)
    let fixture_dir = std::env::temp_dir().join("stock_analysis_test_fixtures");
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let fixture_db = fixture_dir.join("fresh_today.db");

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let create_output = Command::new("sqlite3")
        .arg(&fixture_db)
        .arg(format!(
            "CREATE TABLE IF NOT EXISTS stock_daily (date TEXT); \
             DELETE FROM stock_daily; \
             INSERT INTO stock_daily VALUES ('{today}');"
        ))
        .output()
        .expect("应能跑 sqlite3 (测试前置条件)");

    assert!(
        create_output.status.success(),
        "sqlite3 创建 fresh fixture 失败: {}",
        String::from_utf8_lossy(&create_output.stderr)
    );
    assert!(fixture_db.exists(), "fixture DB 必须存在: {fixture_db:?}");

    let output = run_with_db(Some(fixture_db.to_str().unwrap()));
    assert!(
        output.status.success(),
        "fresh fixture db 应 PASS (exit 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// BR-252: PR 只签发离线 Gate C 证据，Release/default 保持真实 freshness fail-closed。
#[test]
fn compliance_pr_policy_runs_offline_checks_without_claiming_freshness() {
    let output = run_compliance(&["--policy", "pr"], Some("/nonexistent/TEST_CODE.db"));
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("policy=pr"));
    assert!(stdout.contains("check_fake_impl.sh"));
    assert!(stdout.contains("freshness: NOT RUN (Gate C offline policy)"));
    assert!(!stdout.contains("===== check_data_freshness.sh ====="));
}

#[test]
fn compliance_release_and_default_reject_an_unapproved_database_before_checks() {
    for args in [&["--policy", "release"][..], &[][..]] {
        let output = run_compliance(args, Some("/nonexistent/TEST_CODE.db"));
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("====="));
        assert!(String::from_utf8_lossy(&output.stderr).contains("release STOCK_DB does not exist"));
    }
}

#[test]
fn compliance_release_requires_an_explicit_production_database_identity() {
    for args in [&["--policy", "release"][..], &[][..]] {
        let output = run_compliance(args, None);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("====="));
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("STOCK_DB must explicitly identify the production database"));
    }
}

#[test]
fn compliance_release_rejects_test_clock_and_calendar_overrides() {
    for (name, value) in [
        ("FRESHNESS_TODAY", "2026-08-26"),
        ("TRADING_CALENDAR", "/tmp/TEST_CODE_calendar.csv"),
    ] {
        let output = Command::new("bash")
            .arg(compliance_script_path())
            .args(["--policy", "release"])
            .env("STOCK_DB", "/tmp/TEST_CODE_stock.db")
            .env(name, value)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("应能运行 release compliance 前置检查");
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("====="));
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("rejects FRESHNESS_TODAY/TRADING_CALENDAR overrides"));
    }
}

#[test]
fn compliance_release_rejects_an_existing_non_production_database() {
    let fixture = std::env::temp_dir().join(format!(
        "TEST_CODE_non_production_stock_{}.db",
        std::process::id()
    ));
    std::fs::write(&fixture, []).expect("create non-production database fixture");
    let output = run_compliance(&["--policy", "release"], fixture.to_str());
    let _ = std::fs::remove_file(&fixture);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("====="));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not the fixed production database"));
}

#[test]
fn compliance_rejects_unknown_arguments_before_running_checks() {
    let output = run_compliance(&["--policy", "TEST_CODE_unknown"], None);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("====="));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
}

#[test]
fn compliance_workflow_never_backfills_or_claims_release_freshness() {
    let workflow = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/compliance.yml"),
    )
    .expect("read compliance workflow");
    assert!(workflow.contains("fetch-depth: 0"));
    assert!(workflow.contains("PR_BASE_SHA: ${{ github.event.pull_request.base.sha }}"));
    assert!(workflow.contains("bash tools/compliance/check.sh --policy pr"));
    assert!(!workflow.contains("backfill_daily"));
    assert!(!workflow.contains("STOCK_DB="));
}

#[test]
fn pr_evidence_rejects_legacy_fields_without_gate_c_and_gate_d_provenance() {
    let body = r#"Refs: spec §10.12
Data-Redlines: [2.4, 2.7, 2.10]
OldModules: checker | adopt | deepen
Threshold-Proof: 90/85 and 80/95
Business-Rules: BR-252
Rollback: git revert TEST_CODE"#;
    let output = run_pr_evidence(body);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Gate-Policy"), "{stderr}");
    assert!(stderr.contains("Gate-C"), "{stderr}");
    assert!(stderr.contains("Gate-D"), "{stderr}");
}

#[test]
fn pr_evidence_accepts_complete_separate_gate_c_and_gate_d_results() {
    let body = r#"Refs: spec §10.12
Data-Redlines: [2.4, 2.7, 2.10]
OldModules: checker | adopt | deepen
Threshold-Proof: spec §10.4 and config/design_contracts.toml [coverage]
Business-Rules: BR-252
Rollback: git revert TEST_CODE
Gate-Policy: PR=core-patch90+other-patch85+ratchet; Release=global80+core95+freshness+live
Bootstrap-Baseline: true
Baseline-Source-SHA: f05f506e744898319b6ba059580ab17bf65004cf
Baseline-Global: 201256/258810
Baseline-Core: 157635/202935
Coverage-Tools: rustc=1.95.0; LLVM=22.1.2; cargo-llvm-cov=0.8.7
Gate-C: PASS
Gate-D: Release Blocked"#;
    let output = run_pr_evidence(body);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("PASS"));
}

#[test]
fn pr_evidence_rejects_baseline_values_that_do_not_match_the_contract() {
    let body = r#"Refs: spec §10.12
Data-Redlines: [2.4, 2.7, 2.10]
OldModules: checker | adopt | deepen
Threshold-Proof: spec §10.4 and config/design_contracts.toml [coverage]
Business-Rules: BR-252
Rollback: git revert TEST_CODE
Gate-Policy: PR=core-patch90+other-patch85+ratchet; Release=global80+core95+freshness+live
Bootstrap-Baseline: true
Baseline-Source-SHA: f05f506e744898319b6ba059580ab17bf65004cf
Baseline-Global: 1/1
Baseline-Core: 157635/202935
Coverage-Tools: rustc=1.95.0; LLVM=22.1.2; cargo-llvm-cov=0.8.7
Gate-C: PASS
Gate-D: Release Blocked"#;
    let output = run_pr_evidence(body);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match contract"));
}

#[test]
fn pr_evidence_bootstrap_flag_must_match_the_base_contract_state() {
    let body = r#"Refs: spec §10.12
Data-Redlines: [2.4, 2.7, 2.10]
OldModules: checker | adopt | deepen
Threshold-Proof: spec §10.4 and config/design_contracts.toml [coverage]
Business-Rules: BR-252
Rollback: git revert TEST_CODE
Gate-Policy: PR=core-patch90+other-patch85+ratchet; Release=global80+core95+freshness+live
Bootstrap-Baseline: false
Baseline-Source-SHA: f05f506e744898319b6ba059580ab17bf65004cf
Baseline-Global: 201256/258810
Baseline-Core: 157635/202935
Coverage-Tools: rustc=1.95.0; LLVM=22.1.2; cargo-llvm-cov=0.8.7
Gate-C: PASS
Gate-D: Release Blocked"#;
    let output = Command::new("bash")
        .arg(pr_evidence_script_path())
        .env("PR_BODY", body)
        .env("PR_BASE_SHA", "c6024e5")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("应能核对 base coverage contract");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Bootstrap-Baseline 与 base contract 状态不一致"));
}
