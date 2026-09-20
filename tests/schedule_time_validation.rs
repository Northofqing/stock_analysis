//! MU-cli-single: 定时输入校验必须以错误码退出, 而非 panic (越界 HH:MM)
//! 或死循环 (仅含非法值的星期过滤)。
//!
//! 行为 RED: `--schedule --schedule-time '99:99'` 曾在 schedule.rs 的
//! and_hms_opt().expect 处 panic (EXIT=101); 修复后 EXIT=1 并带明确错误。

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn isolated_stock_analysis(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stock_analysis"));
    command.current_dir(root);
    for key in [
        "DATABASE_PATH",
        "STOCK_ENV_MODE",
        "SCHEDULE_TIME",
        "SCHEDULE_ENABLED",
    ] {
        command.env_remove(key);
    }
    // 启动校验要求至少一个非空 AI key; 注入 TEST_CODE 合成凭据使进程能
    // 走到 schedule 校验 (schedule 模式在到达本测试的报错点前不消费该 key)。
    command.env("GEMINI_API_KEY", "TEST_CODE_schedule_validation_key");
    command
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "TEST_CODE_schedule_validation_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    dir
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn schedule_time_out_of_range_exits_with_error_not_panic() {
    let root = temp_root();
    let output = isolated_stock_analysis(&root)
        .args(["--schedule", "--schedule-time", "99:99"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "exit must be graceful error, output={text}"
    );
    assert!(text.contains("无效的定时时间"), "output={text}");
    assert!(!text.contains("panicked"), "output={text}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn schedule_weekdays_without_any_valid_value_exits_with_error() {
    let root = temp_root();
    let output = isolated_stock_analysis(&root)
        .args(["--schedule", "--schedule-time", "10:30", "--weekdays", "8"])
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "exit must be graceful error, output={text}"
    );
    assert!(text.contains("无效的星期过滤"), "output={text}");
    assert!(!text.contains("panicked"), "output={text}");
    std::fs::remove_dir_all(&root).ok();
}
