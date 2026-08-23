use serde_json::json;
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
            "75",
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
fn complete_gateway_and_trading_coverage_pass_the_core_gate() {
    let output = run_gate(&write_report(100));
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 files"));
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
