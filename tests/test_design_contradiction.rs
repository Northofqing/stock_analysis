//! 验证 check_design_contradiction.sh 行为
//! AGENTS.md §2.9 设计矛盾禁令 (PR-3).
//!
//! R-2: 推送门 (event_risk_score_threshold) > 评分封顶 (clamp_max) 即矛盾
//! R-7: source_score 常量失衡, 必须有边界证明
//!
//! 故意制造矛盾验证 fail 模式 (Step 3.8): 临时改 config threshold 调高,
//! 跑脚本, 断言 exit code = 1。
//!
//! 注意: 第二个测试会改写 config/opportunity.toml, 因此用文件锁串行化,
//! 避免与第一个测试在 cargo test 默认并行调度下产生竞态。

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn script_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tools/compliance/lib/check_design_contradiction.sh");
    p
}

fn config_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("config/opportunity.toml");
    p
}

fn run_check() -> std::process::Output {
    Command::new("bash")
        .arg(script_path())
        .output()
        .expect("应能跑 check_design_contradiction.sh")
}

#[test]
fn test_design_contradiction_passes_current_config() {
    // 取锁确保不与 fail-mode 测试并行 (后者会改写 config)
    let _guard = CONFIG_LOCK.lock().unwrap();
    // 当前 config 已对齐 (threshold 60, clamp 70), 应 pass
    // 注: PR-3 的 Step 3.3 把 threshold 改成 60 后, 此测试才能 pass
    let output = run_check();
    assert!(
        output.status.success(),
        "aligned config 应 pass (exit 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_design_contradiction_fails_on_threshold_exceeding_clamp() {
    // 故意制造矛盾: 临时把 threshold 改到 80 (> clamp 70)
    // 跑完恢复, 避免污染 working tree
    let _guard = CONFIG_LOCK.lock().unwrap();

    let cfg = config_path();
    let backup = format!("{}.bak", cfg.display());
    let original = std::fs::read_to_string(&cfg).expect("应能读 opportunity.toml");
    std::fs::write(&backup, &original).expect("应能备份 opportunity.toml");

    // 替换 threshold: 找到 event_risk_score_threshold = N, 改成 80
    let mutated = original.replace(
        "event_risk_score_threshold = 60",
        "event_risk_score_threshold = 80",
    );
    assert!(
        mutated != original,
        "应能找到 threshold = 60 这一行 (前置条件: config 必须已对齐)"
    );
    std::fs::write(&cfg, &mutated).expect("应能写 opportunity.toml");

    let output = run_check();

    // 恢复 config
    std::fs::copy(&backup, &cfg).expect("应能恢复 opportunity.toml");
    let _ = std::fs::remove_file(&backup);

    assert!(
        !output.status.success(),
        "threshold=80 (> clamp 70) 应 FAIL (exit != 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("§2.9") && stderr.contains("设计矛盾"),
        "stderr 应说明 §2.9 矛盾原因, got: {stderr}"
    );
}
