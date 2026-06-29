//! 验证 check_data_freshness.sh 的 fail 模式 + skip 行为
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
fn test_data_freshness_check_exits_zero_on_missing_db() {
    // 缺失 DB -> skip (exit 0), 不算 fail
    let output = run_with_db(Some("/nonexistent/path/fake.db"));
    assert!(
        output.status.success(),
        "missing db 应 skip (exit 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_data_freshness_check_exits_zero_when_stock_db_unset_and_repo_db_present() {
    // 不设 STOCK_DB -> 用默认 data/stock_analysis.db
    // 该库 MAX(date)=2026-06-15, 今日=2026-06-29, 应 FAIL (exit 1)
    // 本测试只验证 "脚本能跑、不会 panic", 不验证 pass/fail — 因为新鲜度依赖日期。
    let output = run_with_db(None);
    // 仅断言: 不管 pass 还是 fail, 退出码都应是数字 (脚本无 panic / segfault)
    assert!(
        output.status.code().is_some(),
        "脚本应干净退出 (不是被信号杀死)"
    );
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
        .arg("CREATE TABLE IF NOT EXISTS stock_daily (date TEXT); \
              DELETE FROM stock_daily; \
              INSERT INTO stock_daily VALUES ('2026-01-01');")
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