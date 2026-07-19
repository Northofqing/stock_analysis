use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn isolated_root(label: &str) -> std::path::PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "stock-analysis-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("create isolated tool directory");
    root
}

#[test]
fn v14_e2e_binary_executes_every_local_layer() {
    let root = isolated_root("v14-e2e");
    let output = Command::new(env!("CARGO_BIN_EXE_v14_e2e"))
        .current_dir(&root)
        .output()
        .expect("run v14 e2e binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "v14 e2e failed: {stdout}");
    assert!(stdout.contains("✅ v14.2 端到端实测通过"));
    assert!(stdout.contains("L1: 5 个事件生成 OK"));
    std::fs::remove_dir_all(root).expect("remove isolated v14 directory");
}

#[test]
fn rsi_optimizer_list_executes_the_registered_preset_catalog() {
    let root = isolated_root("rsi-list");
    let output = Command::new(env!("CARGO_BIN_EXE_rsi_optimize"))
        .arg("list")
        .current_dir(&root)
        .output()
        .expect("run rsi preset listing");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "rsi list failed: {stdout}");
    assert!(stdout.contains("daily_v13_strict_rising"));
    assert!(stdout.contains("daily_v12_deep_no_stop"));
    std::fs::remove_dir_all(root).expect("remove isolated rsi directory");
}

#[test]
fn boll_macd_backtest_accepts_an_empty_real_csv_batch() {
    let root = isolated_root("boll-empty");
    let reports = root.join("reports/analysis");
    std::fs::create_dir_all(&reports).expect("create isolated report input directory");
    std::fs::write(
        reports.join("closed_positions_with_ai.csv"),
        "code,name,buy_date,sell_date,buy,sell,return_pct,hold_days,ai_score,buy_advice,buy_trend,sell_score\n",
    )
    .expect("write empty CSV header");
    let output = Command::new(env!("CARGO_BIN_EXE_boll_macd_backtest"))
        .current_dir(&root)
        .output()
        .expect("run empty boll/macd backtest");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "empty backtest failed: {stdout}");
    assert!(stdout.contains("共加载 0 笔"));
    assert!(stdout.contains("回测结果"));
    std::fs::remove_dir_all(root).expect("remove isolated backtest directory");
}

#[test]
fn winrate_simulator_runs_against_an_isolated_real_sqlite_fixture() {
    let root = isolated_root("winrate");
    let database = root.join("winrate.db");
    let schema = "CREATE TABLE prediction_tracker (id INTEGER PRIMARY KEY AUTOINCREMENT, pred_date TEXT NOT NULL, target_date TEXT NOT NULL, theme_name TEXT, stock_code TEXT, pred_direction TEXT NOT NULL, pred_score REAL, pred_detail TEXT, actual_change REAL, actual_result TEXT, hit INTEGER, created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP); INSERT INTO prediction_tracker (pred_date,target_date,theme_name,pred_direction,hit) VALUES ('2026-07-16','2026-07-17','AI算力','up',1),('2026-07-16','2026-07-17','AI算力','up',0),('2026-07-16','2026-07-17','测试高胜率','up',1);";
    let setup = Command::new("sqlite3")
        .args([database.as_os_str(), std::ffi::OsStr::new(schema)])
        .output()
        .expect("seed isolated prediction tracker");
    assert!(setup.status.success(), "sqlite fixture setup failed");

    let output = Command::new(env!("CARGO_BIN_EXE_winrate_simulator"))
        .args([
            "--days",
            "30",
            "--min-samples",
            "1",
            "--blacklist",
            "AI算力",
        ])
        .current_dir(&root)
        .env("STOCK_DB", &database)
        .env("MONITOR_OPERATOR_AUTH_REQUIRED", "false")
        .output()
        .expect("run winrate simulator");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "winrate simulator failed: {stdout}"
    );
    assert!(stdout.contains("调整前 (全量)"));
    assert!(stdout.contains("测试高胜率"));
    std::fs::remove_dir_all(root).expect("remove isolated winrate directory");
}

#[test]
fn command_line_failure_and_help_paths_are_explicit() {
    let root = isolated_root("cli-matrix");
    let winrate_help = Command::new(env!("CARGO_BIN_EXE_winrate_simulator"))
        .arg("--help")
        .current_dir(&root)
        .env("MONITOR_OPERATOR_AUTH_REQUIRED", "false")
        .output()
        .expect("run winrate help");
    assert!(winrate_help.status.success());
    assert!(String::from_utf8_lossy(&winrate_help.stdout).contains("--min-samples"));

    let rsi_invalid = Command::new(env!("CARGO_BIN_EXE_rsi_optimize"))
        .arg("not-a-command")
        .current_dir(&root)
        .output()
        .expect("run invalid rsi command");
    assert_eq!(rsi_invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rsi_invalid.stderr).contains("未知命令"));

    let lhb_help = Command::new(env!("CARGO_BIN_EXE_lhb_query"))
        .arg("--help")
        .current_dir(&root)
        .output()
        .expect("run lhb help");
    assert!(lhb_help.status.success());
    assert!(String::from_utf8_lossy(&lhb_help.stdout).contains("龙虎榜数据查询工具"));
    std::fs::remove_dir_all(root).expect("remove isolated CLI directory");
}

fn isolated_main_command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stock_analysis"));
    command
        .current_dir(root)
        .env("DATABASE_PATH", root.join("main.db"))
        .env("STOCK_ENV_MODE", "test")
        .env("STOCK_LIST", "")
        .env("DEEPSEEK_API_KEY", "TEST_CODE_KEY")
        .env("MONITOR_OPERATOR_AUTH_REQUIRED", "false")
        .env("MACRO_AI_ENABLED", "false")
        .env("SECTOR_RESONANCE_ENABLED", "false")
        .env("LHB_APPEND_ENABLED", "false")
        .env("LIMIT_UP_APPEND_ENABLED", "false")
        .env("POSITION_TRACKING_ENABLED", "false")
        .env("STOCK_FILTER_DELISTED", "false")
        .env_remove("FEISHU_APP_ID")
        .env_remove("FEISHU_APP_SECRET")
        .env_remove("WECHAT_WEBHOOK");
    command
}

#[test]
fn main_binary_runs_an_empty_isolated_deep_analysis_batch() {
    let root = isolated_root("main-empty");
    let output = isolated_main_command(&root)
        .args(["--deep-analysis", "--no-notify"])
        .output()
        .expect("run isolated empty main batch");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "isolated main failed: {combined}");
    assert!(combined.contains("Multi-Agent 深度分析（共 0 只）"));
    assert!(root.join("main.db").exists());
    std::fs::remove_dir_all(root).expect("remove isolated main directory");
}

#[test]
fn main_binary_rejects_an_invalid_schedule_without_waiting() {
    let root = isolated_root("main-schedule");
    let output = isolated_main_command(&root)
        .args(["--schedule", "--schedule-time", "invalid"])
        .output()
        .expect("run invalid isolated schedule");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "invalid schedule returned success"
    );
    assert!(combined.contains("无效的定时时间格式"));
    std::fs::remove_dir_all(root).expect("remove isolated schedule directory");
}

#[test]
fn agent_test_binary_accepts_an_empty_isolated_stock_batch() {
    let root = isolated_root("agent-empty");
    let output = Command::new(env!("CARGO_BIN_EXE_agent_test"))
        .current_dir(&root)
        .env("STOCK_LIST", "")
        .env("DEEPSEEK_API_KEY", "TEST_CODE_KEY")
        .env_remove("DOUBAO_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .output()
        .expect("run isolated empty agent batch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "empty agent batch failed: {stdout}"
    );
    assert!(stdout.contains("获取到 0 只待评股票"));
    assert!(root.join("data/stock.db").exists());
    std::fs::remove_dir_all(root).expect("remove isolated agent directory");
}
