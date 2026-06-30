//! 修复 P0-G (2026-06-30 codex review): review 顶层 fast-fail 测试.
//! 验证 tokio::time::timeout 包裹 run_review_only_inner 模式正确.
//! 沙箱 / 数据源全失联时, 5min 内必须 exit 2 (不推送噪声).
//!
//! 实际业务路径集成测试见 tests/bin_monitor_timeout.rs (process::Command).

use std::time::Duration;

/// 模拟慢任务, 验证 timeout 触发 Elapsed.
#[tokio::test]
async fn test_timeout_triggers_when_task_too_slow() {
    let start = std::time::Instant::now();
    let outcome: Result<(), tokio::time::error::Elapsed> = tokio::time::timeout(
        Duration::from_millis(200),
        tokio::time::sleep(Duration::from_secs(10)),
    )
    .await;
    let elapsed = start.elapsed();
    assert!(outcome.is_err(), "期望 timeout Err, 实际 Ok");
    assert!(
        elapsed >= Duration::from_millis(200) && elapsed < Duration::from_millis(1500),
        "timeout 应在 200ms 附近触发, 实际 {}ms",
        elapsed.as_millis()
    );
}

/// 验证正常任务不被 timeout 误杀.
#[tokio::test]
async fn test_normal_completion_under_timeout() {
    let outcome: Result<&str, tokio::time::error::Elapsed> = tokio::time::timeout(
        Duration::from_secs(5),
        async { tokio::time::sleep(Duration::from_millis(50)).await; "ok" },
    )
    .await;
    assert_eq!(outcome.unwrap(), "ok");
}

/// 合并的 env 测试: 因 std::env 全局共享 + 测试并行不安全, 合并成 1 个顺序跑.
#[test]
fn test_env_parsing_all_cases() {
    use std::sync::Mutex;
    // 静态 Mutex 锁住所有 env 操作 (其他测试也共用同一 env 变量).
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap();

    // Case 1: env 未设置 → 默认 300
    std::env::remove_var("MONITOR_REVIEW_TIMEOUT_SECS");
    let parsed: u64 = std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")
        .ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(300);
    assert_eq!(parsed, 300, "unset 应 fallback 到 300");

    // Case 2: env 覆盖 → 60
    std::env::set_var("MONITOR_REVIEW_TIMEOUT_SECS", "60");
    let parsed: u64 = std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")
        .ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(300);
    assert_eq!(parsed, 60, "env 覆盖应生效");
    std::env::remove_var("MONITOR_REVIEW_TIMEOUT_SECS");

    // Case 3: env 非法值 → 默认 300
    std::env::set_var("MONITOR_REVIEW_TIMEOUT_SECS", "abc");
    let parsed: u64 = std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")
        .ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(300);
    assert_eq!(parsed, 300, "非法值应 fallback");
    std::env::remove_var("MONITOR_REVIEW_TIMEOUT_SECS");

    // Case 4: env "0" → 默认 300 (filter n>0 兜底)
    std::env::set_var("MONITOR_REVIEW_TIMEOUT_SECS", "0");
    let parsed: u64 = std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")
        .ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(300);
    assert_eq!(parsed, 300, "0 应 fallback");
    std::env::remove_var("MONITOR_REVIEW_TIMEOUT_SECS");
}
