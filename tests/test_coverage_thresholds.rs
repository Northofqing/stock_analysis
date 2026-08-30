use serde_json::json;
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

// BR-250: worktree 中的绝对源码路径必须按当前 checkout 计入核心覆盖率。
static NEXT_REPORT_ID: AtomicU64 = AtomicU64::new(0);

struct TestReport(PathBuf);

impl TestReport {
    fn path(&self) -> &Path {
        &self.0
    }
}

struct TestCheckout {
    root: PathBuf,
    checkout: PathBuf,
}

impl TestCheckout {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "stock-analysis-coverage-worktree-{}-{}",
            std::process::id(),
            NEXT_REPORT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let checkout = root
            .join("stock_analysis")
            .join(".worktrees")
            .join("TEST_CODE_branch");
        std::fs::create_dir_all(&checkout).expect("create isolated worktree checkout");
        Self { root, checkout }
    }

    fn path(&self) -> &Path {
        &self.checkout
    }
}

impl Drop for TestCheckout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Drop for TestReport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_report(gateway_covered: u64) -> TestReport {
    let report = std::env::temp_dir().join(format!(
        "stock-analysis-coverage-{}-{}.json",
        std::process::id(),
        NEXT_REPORT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::File::create(&report).expect("create isolated coverage report");
    serde_json::to_writer(
        file,
        &json!({
            "data": [{
                "totals": {"lines": {"covered": 200, "count": 200}},
                "files": [
                    {
                        "filename": "/workspace/stock_analysis/src/risk/limits.rs",
                        "summary": {"lines": {"covered": 100, "count": 100}}
                    },
                    {
                        "filename": "/workspace/stock_analysis/src/data_gateway/review.rs",
                        "summary": {"lines": {"covered": gateway_covered, "count": 100}}
                    }
                ]
            }]
        }),
    )
    .expect("write coverage fixture");
    TestReport(report)
}

fn write_expanded_core_report() -> TestReport {
    let report = std::env::temp_dir().join(format!(
        "stock-analysis-expanded-core-{}-{}.json",
        std::process::id(),
        NEXT_REPORT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::File::create(&report).expect("create expanded core coverage report");
    let filenames = [
        "src/auth/session.rs",
        "src/bin/monitor/main.rs",
        "src/durable_delivery/runtime.rs",
        "src/market_analyzer/mod.rs",
        "src/monitor/scanner.rs",
        "src/portfolio/store.rs",
        "src/selection/ingress_v2.rs",
    ];
    let files = filenames
        .into_iter()
        .map(|filename| {
            json!({
                "filename": format!("/workspace/stock_analysis/{filename}"),
                "summary": {"lines": {"covered": 0, "count": 100}}
            })
        })
        .chain(std::iter::once(json!({
            "filename": "/workspace/stock_analysis/src/risk/limits.rs",
            "summary": {"lines": {"covered": 100, "count": 100}}
        })))
        .collect::<Vec<_>>();
    serde_json::to_writer(
        file,
        &json!({
            "data": [{
                "totals": {"lines": {"covered": 800, "count": 800}},
                "files": files
            }]
        }),
    )
    .expect("write expanded core fixture");
    TestReport(report)
}

fn write_current_checkout_report(checkout: &Path) -> TestReport {
    let report = std::env::temp_dir().join(format!(
        "stock-analysis-worktree-coverage-{}-{}.json",
        std::process::id(),
        NEXT_REPORT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let source = checkout.join("src/risk/limits.rs");
    let file = std::fs::File::create(&report).expect("create worktree coverage report");
    serde_json::to_writer(
        file,
        &json!({
            "data": [{
                "totals": {"lines": {"covered": 100, "count": 100}},
                "files": [{
                    "filename": source,
                    "summary": {"lines": {"covered": 0, "count": 100}}
                }]
            }]
        }),
    )
    .expect("write worktree coverage fixture");
    TestReport(report)
}

fn run_gate(report: &TestReport) -> std::process::Output {
    run_gate_from(report, Path::new(env!("CARGO_MANIFEST_DIR")))
}

fn run_gate_from(report: &TestReport, current_dir: &Path) -> std::process::Output {
    let checker = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/coverage/check_thresholds.py");
    Command::new("python3")
        .arg(checker)
        .args([
            report.path().to_str().expect("UTF-8 report path"),
            "--global-min",
            "80",
            "--core-min",
            "95",
        ])
        .current_dir(current_dir)
        .output()
        .expect("run coverage threshold checker")
}

#[test]
fn unified_gateway_lines_are_part_of_the_core_coverage_gate() {
    let output = run_gate(&write_report(0));
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("core coverage gate failed"));
}

#[test]
fn release_policy_rejects_thresholds_below_the_fixed_floors() {
    let report = write_report(100);
    let checker = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/coverage/check_thresholds.py");
    let output = Command::new("python3")
        .arg(checker)
        .arg(report.path())
        .args(["--global-min", "79.99", "--core-min", "95"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run release coverage policy with lowered floor");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("fixed release floors"));
}

#[test]
fn high_release_coverage_without_provenance_cannot_mint_a_pass() {
    let output = run_gate(&write_report(100));
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires --lcov"));
}

#[test]
fn release_policy_accepts_only_a_complete_bound_source_inventory() {
    let fixture = PrFixture::new(&PrCase::default());
    let output = fixture.run_release();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("100.00%"));
}

#[test]
fn release_policy_rejects_a_focused_or_reduced_source_inventory() {
    let fixture = PrFixture::new(&PrCase::default());
    let mut payload: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&fixture.report).expect("read release coverage report"),
    )
    .expect("parse release coverage report");
    payload["data"][0]["files"]
        .as_array_mut()
        .expect("coverage files array")
        .retain(|entry| {
            !entry["filename"]
                .as_str()
                .expect("coverage filename")
                .ends_with("src/agent/example.rs")
        });
    std::fs::write(
        &fixture.report,
        serde_json::to_vec(&payload).expect("serialize focused report"),
    )
    .expect("write focused report");
    std::fs::write(
        &fixture.lcov,
        lcov_record(&fixture.checkout.path().join("src/risk/limits.rs"), 10, 10),
    )
    .expect("write focused LCOV report");

    let output = fixture.run_release();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("release coverage source inventory mismatch"));
}

#[test]
fn every_registered_production_control_path_counts_as_core() {
    let output = run_gate(&write_expanded_core_report());
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("core coverage gate failed"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("8 files"));
}

#[test]
fn current_worktree_paths_are_counted_as_core_instead_of_disappearing() {
    let checkout = TestCheckout::new();
    let report = write_current_checkout_report(checkout.path());
    let output = run_gate_from(&report, checkout.path());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("core coverage gate failed"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 files"));
}

// BR-252: PR 覆盖率策略必须分别检查核心/其他生产改动，并保持全仓棘轮只升不降。
#[derive(Clone)]
struct PrCase {
    bootstrap: bool,
    change_core: bool,
    change_other: bool,
    rename_other: bool,
    delete_core_tail_only: bool,
    core_patch_covered: usize,
    other_patch_covered: usize,
    report_global: (u64, u64),
    report_core: (u64, u64),
    base_global: (u64, u64),
    base_core: (u64, u64),
    candidate_global: (u64, u64),
    candidate_core: (u64, u64),
}

impl Default for PrCase {
    fn default() -> Self {
        Self {
            bootstrap: true,
            change_core: true,
            change_other: true,
            rename_other: false,
            delete_core_tail_only: false,
            core_patch_covered: 9,
            other_patch_covered: 17,
            report_global: (120, 120),
            report_core: (100, 100),
            base_global: (108, 120),
            base_core: (90, 100),
            candidate_global: (108, 120),
            candidate_core: (90, 100),
        }
    }
}

struct PrFixture {
    checkout: TestCheckout,
    report: PathBuf,
    lcov: PathBuf,
    base_ref: String,
    bootstrap: bool,
}

impl PrFixture {
    fn new(case: &PrCase) -> Self {
        let checkout = TestCheckout::new();
        let root = checkout.path();
        std::fs::create_dir_all(root.join("src/risk")).expect("create core source directory");
        std::fs::create_dir_all(root.join("src/agent")).expect("create other source directory");
        std::fs::create_dir_all(root.join("build_support"))
            .expect("create build support directory");
        std::fs::create_dir_all(root.join("config")).expect("create config directory");

        write_source(
            &root.join("src/risk/limits.rs"),
            "base_core",
            if case.delete_core_tail_only { 20 } else { 10 },
        );
        write_source(&root.join("src/agent/example.rs"), "base_other", 20);
        write_source(
            &root.join("build_support/sample.rs"),
            "base_build_support",
            1,
        );
        git(root, &["init", "-q"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "base source"]);
        let base_source_sha = git_stdout(root, &["rev-parse", "HEAD"]);

        if !case.bootstrap {
            write_coverage_config(root, &base_source_sha, case.base_global, case.base_core);
            git(root, &["add", "config/design_contracts.toml"]);
            git(root, &["commit", "-qm", "base coverage contract"]);
        }
        let base_ref = git_stdout(root, &["rev-parse", "HEAD"]);

        if case.delete_core_tail_only {
            write_source(&root.join("src/risk/limits.rs"), "base_core", 10);
        } else if case.change_core {
            write_source(&root.join("src/risk/limits.rs"), "head_core", 10);
        }
        if case.change_other {
            write_source(&root.join("src/agent/example.rs"), "head_other", 20);
        }
        if case.rename_other {
            git(
                root,
                &["mv", "src/agent/example.rs", "src/agent/renamed.rs"],
            );
        }
        git(root, &["add", "."]);
        git(
            root,
            &["commit", "--allow-empty", "-qm", "candidate source"],
        );
        let candidate_source_sha = git_stdout(root, &["rev-parse", "HEAD"]);
        write_coverage_config(
            root,
            &candidate_source_sha,
            case.candidate_global,
            case.candidate_core,
        );
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "candidate"]);

        let report = root.join("coverage.json");
        let other_source = if case.rename_other {
            root.join("src/agent/renamed.rs")
        } else {
            root.join("src/agent/example.rs")
        };
        let file = std::fs::File::create(&report).expect("create PR coverage report");
        serde_json::to_writer(
            file,
            &json!({
                "type": "llvm.coverage.json.export",
                "version": "3.1.0",
                "cargo_llvm_cov": {
                    "version": "0.8.7",
                    "manifest_path": root.join("Cargo.toml")
                },
                "data": [{
                    "totals": {"lines": {
                        "covered": case.report_global.0,
                        "count": case.report_global.1
                    }},
                    "files": [
                        {
                            "filename": root.join("src/risk/limits.rs"),
                            "summary": {"lines": {
                                "covered": case.report_core.0,
                                "count": case.report_core.1
                            }}
                        },
                        {
                            "filename": &other_source,
                            "summary": {"lines": {"covered": 20, "count": 20}}
                        }
                    ]
                }]
            }),
        )
        .expect("write PR coverage report");

        let lcov = root.join("lcov.info");
        let mut contents = lcov_record(
            &root.join("src/risk/limits.rs"),
            10,
            case.core_patch_covered,
        );
        contents.push_str(&lcov_record(&other_source, 20, case.other_patch_covered));
        std::fs::write(&lcov, contents).expect("write PR LCOV report");

        Self {
            checkout,
            report,
            lcov,
            base_ref,
            bootstrap: case.bootstrap,
        }
    }

    fn run(&self, include_bootstrap_flag: bool) -> std::process::Output {
        self.run_with_path(include_bootstrap_flag, None)
    }

    fn run_with_path(
        &self,
        include_bootstrap_flag: bool,
        path: Option<&std::ffi::OsStr>,
    ) -> std::process::Output {
        let checker =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/coverage/check_thresholds.py");
        let mut command = Command::new("python3");
        command
            .arg(checker)
            .args(["--policy", "pr", "--report"])
            .arg(&self.report)
            .arg("--lcov")
            .arg(&self.lcov)
            .arg("--base-ref")
            .arg(&self.base_ref)
            .current_dir(self.checkout.path());
        if let Some(path) = path {
            command.env("PATH", path);
        }
        if include_bootstrap_flag {
            command.arg("--bootstrap-baseline");
        }
        command.output().expect("run PR coverage policy")
    }

    fn run_release(&self) -> std::process::Output {
        let checker =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/coverage/check_thresholds.py");
        Command::new("python3")
            .arg(checker)
            .args(["--policy", "release", "--report"])
            .arg(&self.report)
            .arg("--lcov")
            .arg(&self.lcov)
            .current_dir(self.checkout.path())
            .output()
            .expect("run release coverage policy")
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=TEST_CODE",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git fixture query");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_owned()
}

fn write_source(path: &Path, prefix: &str, lines: usize) {
    let contents = (1..=lines)
        .map(|line| format!("pub fn {prefix}_{line}() -> usize {{ {line} }}\n"))
        .collect::<String>();
    std::fs::write(path, contents).expect("write source fixture");
}

fn write_coverage_config(root: &Path, source_sha: &str, global: (u64, u64), core: (u64, u64)) {
    let contents = format!(
        r#"[coverage]
schema = 1
source_sha = "{source_sha}"
bootstrap_approved = true
bootstrap_rule = "BR-252"
global_covered = {}
global_count = {}
core_covered = {}
core_count = {}
core_file_count = 1
pr_core_patch_min = 90
pr_other_patch_min = 85
release_global_min = 80
release_core_min = 95
rustc_release = "1.95.0"
rustc_commit = "59807616e1fa2540724bfbac14d7976d7e4a3860"
llvm_version = "22.1.2"
cargo_llvm_cov_version = "0.8.7"
"#,
        global.0, global.1, core.0, core.1
    );
    std::fs::write(root.join("config/design_contracts.toml"), contents)
        .expect("write coverage contract");
}

fn lcov_record(path: &Path, lines: usize, covered: usize) -> String {
    let mut record = format!("TN:\nSF:{}\n", path.display());
    for line in 1..=lines {
        let hits = usize::from(line <= covered);
        record.push_str(&format!("DA:{line},{hits}\n"));
    }
    record.push_str("end_of_record\n");
    record
}

#[test]
fn pr_policy_accepts_exact_core_and_other_patch_thresholds() {
    let fixture = PrFixture::new(&PrCase::default());
    let output = fixture.run(fixture.bootstrap);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("core patch coverage: 9/10 = 90.00%"));
    assert!(stdout.contains("other production patch coverage: 17/20 = 85.00%"));
}

#[test]
fn pr_policy_rejects_each_patch_bucket_below_its_threshold() {
    for case in [
        PrCase {
            core_patch_covered: 8,
            ..PrCase::default()
        },
        PrCase {
            other_patch_covered: 16,
            ..PrCase::default()
        },
    ] {
        let fixture = PrFixture::new(&case);
        let output = fixture.run(fixture.bootstrap);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("patch coverage gate failed"));
    }
}

#[test]
fn pr_policy_reports_an_unchanged_bucket_as_na_instead_of_one_hundred_percent() {
    let case = PrCase {
        change_core: false,
        ..PrCase::default()
    };
    let fixture = PrFixture::new(&case);
    let output = fixture.run(fixture.bootstrap);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("core patch coverage: N/A (0 executable changed lines)"));
}

#[test]
fn pr_policy_rejects_a_global_or_core_ratchet_regression() {
    for case in [
        PrCase {
            report_global: (107, 120),
            ..PrCase::default()
        },
        PrCase {
            report_core: (89, 100),
            ..PrCase::default()
        },
    ] {
        let fixture = PrFixture::new(&case);
        let output = fixture.run(fixture.bootstrap);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("coverage ratchet failed"));
    }
}

#[test]
fn pr_policy_rejects_a_candidate_baseline_below_the_base_branch() {
    let case = PrCase {
        bootstrap: false,
        candidate_global: (107, 120),
        ..PrCase::default()
    };
    let fixture = PrFixture::new(&case);
    let output = fixture.run(false);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("candidate baseline regression"));
}

#[test]
fn initial_baseline_requires_an_explicit_bootstrap_flag() {
    let fixture = PrFixture::new(&PrCase::default());
    let output = fixture.run(false);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--bootstrap-baseline"));
}

#[test]
fn initial_baseline_requires_a_tracked_br252_bootstrap_approval() {
    for (from, to) in [
        ("bootstrap_approved = true", "bootstrap_approved = false"),
        (
            "bootstrap_rule = \"BR-252\"",
            "bootstrap_rule = \"TEST_CODE\"",
        ),
    ] {
        let fixture = PrFixture::new(&PrCase::default());
        let config = fixture.checkout.path().join("config/design_contracts.toml");
        let contract = std::fs::read_to_string(&config)
            .expect("read coverage contract")
            .replace(from, to);
        std::fs::write(config, contract).expect("write unapproved bootstrap contract");
        let output = fixture.run(true);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("tracked BR-252 bootstrap approval")
        );
    }
}

#[test]
fn coverage_contract_rejects_thresholds_below_hard_policy_floors() {
    for (from, to, field) in [
        (
            "pr_core_patch_min = 90",
            "pr_core_patch_min = 89",
            "pr_core_patch_min",
        ),
        (
            "pr_other_patch_min = 85",
            "pr_other_patch_min = 84",
            "pr_other_patch_min",
        ),
        (
            "release_global_min = 80",
            "release_global_min = 79",
            "release_global_min",
        ),
        (
            "release_core_min = 95",
            "release_core_min = 94",
            "release_core_min",
        ),
    ] {
        let fixture = PrFixture::new(&PrCase::default());
        let config = fixture.checkout.path().join("config/design_contracts.toml");
        let contract = std::fs::read_to_string(&config)
            .expect("read coverage contract")
            .replace(from, to);
        std::fs::write(config, contract).expect("write lowered coverage contract");
        let output = fixture.run(fixture.bootstrap);
        assert_eq!(output.status.code(), Some(2), "{field}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("hard policy floor"),
            "{field}: {output:?}"
        );
    }
}

#[test]
fn pr_policy_rejects_an_unknown_contract_schema() {
    let fixture = PrFixture::new(&PrCase::default());
    let config = fixture.checkout.path().join("config/design_contracts.toml");
    let contract = std::fs::read_to_string(&config)
        .expect("read coverage contract")
        .replace("schema = 1", "schema = 99");
    std::fs::write(config, contract).expect("write unknown coverage schema");
    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown coverage contract schema"));
}

#[test]
fn pr_policy_rejects_source_changes_after_the_registered_source_sha() {
    let fixture = PrFixture::new(&PrCase::default());
    write_source(
        &fixture.checkout.path().join("src/agent/example.rs"),
        "after_report",
        20,
    );
    git(fixture.checkout.path(), &["add", "."]);
    git(
        fixture.checkout.path(),
        &["commit", "-qm", "source changed after report authority"],
    );

    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("coverage inputs changed after coverage.source_sha"));
}

#[test]
fn pr_policy_rejects_build_support_changes_after_the_registered_source_sha() {
    let fixture = PrFixture::new(&PrCase::default());
    write_source(
        &fixture
            .checkout
            .path()
            .join("build_support/sample.rs"),
        "after_report",
        2,
    );
    git(fixture.checkout.path(), &["add", "."]);
    git(
        fixture.checkout.path(),
        &[
            "commit",
            "-qm",
            "build support changed after report authority",
        ],
    );

    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("coverage inputs changed after coverage.source_sha"));
}

#[test]
fn deletion_only_changes_are_na_instead_of_artificially_covered() {
    let case = PrCase {
        change_core: false,
        change_other: false,
        delete_core_tail_only: true,
        ..PrCase::default()
    };
    let fixture = PrFixture::new(&case);
    let output = fixture.run(fixture.bootstrap);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("core patch coverage: N/A (0 executable changed lines)"));
    assert!(stdout.contains("other production patch coverage: N/A (0 executable changed lines)"));
}

#[test]
fn rename_only_source_changes_preserve_report_identity_and_return_na() {
    let fixture = PrFixture::new(&PrCase {
        change_core: false,
        change_other: false,
        rename_other: true,
        ..PrCase::default()
    });
    let output = fixture.run(fixture.bootstrap);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("core patch coverage: N/A"));
    assert!(stdout.contains("other production patch coverage: N/A"));
}

#[test]
fn pr_policy_returns_input_error_for_missing_base_or_lcov() {
    let mut missing_base = PrFixture::new(&PrCase::default());
    missing_base.base_ref = "TEST_CODE_missing_base".to_owned();
    let output = missing_base.run(missing_base.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("cat-file"));

    let missing_lcov = PrFixture::new(&PrCase::default());
    std::fs::remove_file(&missing_lcov.lcov).expect("remove LCOV fixture");
    let output = missing_lcov.run(missing_lcov.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("LCOV report"));
}

#[test]
fn pr_policy_rejects_report_source_mismatch_and_tool_drift() {
    let mismatch = PrFixture::new(&PrCase::default());
    std::fs::write(
        &mismatch.lcov,
        lcov_record(&mismatch.checkout.path().join("src/risk/limits.rs"), 10, 9),
    )
    .expect("replace LCOV with incomplete source set");
    let output = mismatch.run(mismatch.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("source file sets differ"));

    let drift = PrFixture::new(&PrCase::default());
    let config = drift.checkout.path().join("config/design_contracts.toml");
    let contents = std::fs::read_to_string(&config)
        .expect("read coverage config")
        .replace(
            "cargo_llvm_cov_version = \"0.8.7\"",
            "cargo_llvm_cov_version = \"0.0.0\"",
        );
    std::fs::write(config, contents).expect("write drifted coverage config");
    let output = drift.run(drift.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("tool identity mismatch"));
}

#[test]
fn pr_policy_rejects_a_coverage_path_that_escapes_the_repository() {
    let fixture = PrFixture::new(&PrCase::default());
    std::fs::write(
        &fixture.lcov,
        "TN:\nSF:/tmp/TEST_CODE_escape.rs\nDA:1,1\nend_of_record\n",
    )
    .expect("write escaping LCOV report");
    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("escapes repository"));
}

#[test]
fn pr_policy_rejects_a_malformed_git_diff() {
    let fixture = PrFixture::new(&PrCase::default());
    let fake_bin = fixture.checkout.path().join("TEST_CODE_fake_bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake git directory");
    let fake_git = fake_bin.join("git");
    std::fs::write(
        &fake_git,
        "#!/bin/sh\ncase \" $* \" in\n  *\" diff --find-renames --unified=0 \"*) printf '%s\\n' '+++ b/src/risk/limits.rs' '@@ malformed @@'; exit 0 ;;\nesac\nexec /usr/bin/git \"$@\"\n",
    )
    .expect("write fake git proxy");
    let mut permissions = std::fs::metadata(&fake_git)
        .expect("read fake git metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_git, permissions).expect("make fake git executable");
    let path = std::env::join_paths(std::iter::once(fake_bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").expect("PATH is set"),
    )))
    .expect("construct test PATH");

    let output = fixture.run_with_path(fixture.bootstrap, Some(&path));
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot parse Git diff hunk"));
}

#[test]
fn pr_policy_rejects_missing_or_wrong_report_provenance_metadata() {
    for field in ["type", "cargo_llvm_cov"] {
        let fixture = PrFixture::new(&PrCase::default());
        let mut payload: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&fixture.report).expect("read coverage report fixture"),
        )
        .expect("parse coverage report fixture");
        payload
            .as_object_mut()
            .expect("coverage report object")
            .remove(field);
        std::fs::write(
            &fixture.report,
            serde_json::to_vec(&payload).expect("serialize report without provenance"),
        )
        .expect("write report without provenance");

        let output = fixture.run(fixture.bootstrap);
        assert_eq!(output.status.code(), Some(2), "{field}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("report provenance"),
            "{field}: {output:?}"
        );
    }
}

#[test]
fn pr_policy_rejects_a_core_file_count_scope_mismatch() {
    let fixture = PrFixture::new(&PrCase::default());
    let config = fixture.checkout.path().join("config/design_contracts.toml");
    let contract = std::fs::read_to_string(&config)
        .expect("read coverage contract")
        .replace("core_file_count = 1", "core_file_count = 2");
    std::fs::write(config, contract).expect("write mismatched core scope");

    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("core file count mismatch"));
}

#[test]
fn pr_policy_rejects_a_changed_source_missing_from_both_reports() {
    let fixture = PrFixture::new(&PrCase::default());
    let mut payload: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&fixture.report).expect("read coverage report fixture"),
    )
    .expect("parse coverage report fixture");
    payload["data"][0]["files"]
        .as_array_mut()
        .expect("coverage files array")
        .retain(|entry| {
            !entry["filename"]
                .as_str()
                .expect("coverage filename")
                .ends_with("src/agent/example.rs")
        });
    std::fs::write(
        &fixture.report,
        serde_json::to_vec(&payload).expect("serialize reduced report"),
    )
    .expect("write reduced report");
    std::fs::write(
        &fixture.lcov,
        lcov_record(&fixture.checkout.path().join("src/risk/limits.rs"), 10, 9),
    )
    .expect("write reduced LCOV report");

    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed source is absent"));
}

#[test]
fn pr_policy_accepts_a_hash_bound_reviewed_no_region_source() {
    let fixture = PrFixture::new(&PrCase::default());
    let other = fixture.checkout.path().join("src/agent/example.rs");
    let hash = format!(
        "{:x}",
        Sha256::digest(std::fs::read(&other).expect("read source"))
    );
    let config = fixture.checkout.path().join("config/design_contracts.toml");
    let mut contract = std::fs::read_to_string(&config).expect("read coverage contract");
    contract.push_str(&format!(
        "\n[coverage.reviewed_no_region]\n\"src/agent/example.rs\" = \"{hash}\"\n"
    ));
    std::fs::write(config, contract).expect("write reviewed no-region binding");

    let mut payload: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&fixture.report).expect("read coverage report fixture"),
    )
    .expect("parse coverage report fixture");
    payload["data"][0]["files"]
        .as_array_mut()
        .expect("coverage files array")
        .retain(|entry| {
            !entry["filename"]
                .as_str()
                .expect("coverage filename")
                .ends_with("src/agent/example.rs")
        });
    std::fs::write(
        &fixture.report,
        serde_json::to_vec(&payload).expect("serialize reduced report"),
    )
    .expect("write reduced report");
    std::fs::write(
        &fixture.lcov,
        lcov_record(&fixture.checkout.path().join("src/risk/limits.rs"), 10, 9),
    )
    .expect("write reduced LCOV report");

    let output = fixture.run(fixture.bootstrap);
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("other production patch coverage: N/A")
    );
}

#[test]
fn pr_policy_rejects_a_reviewed_no_region_hash_mismatch() {
    let fixture = PrFixture::new(&PrCase::default());
    let config = fixture.checkout.path().join("config/design_contracts.toml");
    let mut contract = std::fs::read_to_string(&config).expect("read coverage contract");
    contract.push_str(&format!(
        "\n[coverage.reviewed_no_region]\n\"src/agent/example.rs\" = \"{}\"\n",
        "0".repeat(64)
    ));
    std::fs::write(config, contract).expect("write bad reviewed no-region binding");

    let output = fixture.run(fixture.bootstrap);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("SHA-256 mismatch"));
}

#[test]
fn non_bootstrap_pr_cannot_add_a_new_reviewed_no_region_path() {
    let fixture = PrFixture::new(&PrCase {
        bootstrap: false,
        ..PrCase::default()
    });
    let other = fixture.checkout.path().join("src/agent/example.rs");
    let hash = format!(
        "{:x}",
        Sha256::digest(std::fs::read(&other).expect("read source"))
    );
    let config = fixture.checkout.path().join("config/design_contracts.toml");
    let mut contract = std::fs::read_to_string(&config).expect("read coverage contract");
    contract.push_str(&format!(
        "\n[coverage.reviewed_no_region]\n\"src/agent/example.rs\" = \"{hash}\"\n"
    ));
    std::fs::write(config, contract).expect("write added reviewed no-region path");

    let output = fixture.run(false);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot add reviewed no-region paths"));
}

#[test]
fn coverage_workflow_mints_only_pinned_gate_c_evidence() {
    let workflow = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/coverage.yml"),
    )
    .expect("read coverage workflow");
    for required in [
        "fetch-depth: 0",
        "dtolnay/rust-toolchain@1.95.0",
        "components: llvm-tools-preview",
        "tool: cargo-llvm-cov@0.8.7",
        "cargo llvm-cov report --lcov",
        "--policy pr",
        "--base-ref",
        "--bootstrap-baseline",
    ] {
        assert!(
            workflow.contains(required),
            "coverage workflow 缺少 {required}"
        );
    }
    assert!(!workflow.contains("check_thresholds.py target/coverage/coverage.json"));
}
