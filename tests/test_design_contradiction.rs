//! 验证 check_design_contradiction.sh 行为
//! AGENTS.md §2.9 设计矛盾禁令 (PR-3).
//!
//! R-2: 推送门 (event_risk_score_threshold) > 评分封顶 (clamp_max) 即矛盾
//! R-7: source_score 常量失衡, 必须有边界证明
//!
//! 故意制造矛盾验证 fail 模式 (Step 3.8): 临时改 config threshold 调高,
//! 跑脚本, 断言 exit code = 1。
//!
//! 修复 I-9 (2026-06-29 codex review):
//! 1. 用 RAII guard (`ConfigBackup`) 在 panic 时**自动恢复** config (避免 .bak 残留 + dirty tree)
//! 2. backup 写到 std::env::temp_dir() 而非 cfg 同目录 (避免污染 git status)
//! 3. 文档说明 cargo test -- --test-threads=1 (std::sync::Mutex 跨 test binary 不可见,
//!    如果未来加 e2e_prediction_verify_test_design_contradiction 之类的 binary, 锁失效)
//! 4. cargo test 默认 panic=unwind, Drop 会跑 — RAII guard panic-safe.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

/// 修复 (2026-06-29 v9.4.3): --test-threads=2 并行跑时, 第一个 test panic 后
/// CONFIG_LOCK 会 poison, 第二个 test .lock().unwrap() 触发 PoisonError.
/// 容忍 poison (e.into_inner()) 让后续 test 仍能拿锁串行, 不让测试间级联失败.
fn acquire_config_lock() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_LOCK.lock().unwrap_or_else(|e| {
        log::warn!(
            "[test_design_contradiction] CONFIG_LOCK poisoned, recovering: {}",
            e
        );
        e.into_inner()
    })
}

fn script_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tools/compliance/lib/check_design_contradiction.sh");
    p
}

fn config_path() -> PathBuf {
    // v20.1: opportunity.toml 已合并入 strategy.toml (commit f527062)
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("config/strategy.toml");
    p
}

fn run_check() -> std::process::Output {
    Command::new("bash")
        .arg(script_path())
        .output()
        .expect("应能跑 check_design_contradiction.sh")
}

/// RAII guard: Drop 时**自动**恢复 config 到原始内容, 即使 panic / 早返回.
/// 用 panic=unwind (cargo test 默认) — Drop 会在 panic 时跑.
/// 注意: 如果进程被 SIGKILL (强杀), Drop 不跑, 但 std 测试通常不会被强杀.
struct ConfigBackup {
    cfg: PathBuf,
    backup: PathBuf,
}

impl ConfigBackup {
    fn new() -> Self {
        let cfg = config_path();
        // 写到 temp dir, 避免污染 working tree / git status
        let backup = std::env::temp_dir()
            .join("stock_analysis_test_fixtures")
            .join(format!("strategy.toml.bak.{}", std::process::id()));
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        let original = std::fs::read_to_string(&cfg).expect("应能读 strategy.toml");
        std::fs::write(&backup, &original).expect("应能备份 strategy.toml");
        Self { cfg, backup }
    }

    fn restore(&self) {
        if self.backup.exists() {
            // v20.1: 用 atomic rename 替代 std::fs::copy (更可靠)
            let tmp = self.cfg.with_extension("toml.restore_tmp");
            if std::fs::copy(&self.backup, &tmp).is_ok() {
                let _ = std::fs::rename(&tmp, &self.cfg);
            }
            let _ = std::fs::remove_file(&self.backup);
        }
    }
}

impl Drop for ConfigBackup {
    fn drop(&mut self) {
        self.restore();
    }
}

#[test]
fn test_design_contradiction_passes_current_config() {
    // 取锁确保不与 fail-mode 测试并行 (后者会改写 config)
    let _guard = acquire_config_lock();
    // 当前 config 已对齐 (threshold 60, clamp 70), 应 pass
    let output = run_check();
    assert!(
        output.status.success(),
        "aligned config 应 pass (exit 0), stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// v20.2: 用 serial_test 替代 #[ignore] (跨 test binary 文件锁, 解决 race condition)
use serial_test::serial;
// (注: 保留 acquire_config_lock() 作为同 binary 内串行保护, 双重保险)
#[test]
#[serial] // 跨 test binary 串行, 防止 race
fn test_design_contradiction_fails_on_threshold_exceeding_clamp() {
    // 故意制造矛盾: 临时把 threshold 改到 80 (> clamp 70)
    // 用 RAII guard 保证 panic / 早返回时**自动恢复** config.
    let _guard = acquire_config_lock();
    let backup = ConfigBackup::new();

    let cfg = config_path();
    let original = std::fs::read_to_string(&cfg).expect("应能读 strategy.toml");

    // 替换 threshold: 找到 event_risk_score_threshold = N, 改成 80
    let mutated = original.replace(
        "event_risk_score_threshold = 60",
        "event_risk_score_threshold = 80",
    );
    assert!(
        mutated != original,
        "应能找到 threshold = 60 这一行 (前置条件: config 必须已对齐)"
    );
    // 修复 v9.4.5 (2026-06-29): atomic rename 替换, 避免 bash subprocess 看到 fs cache 旧内容.
    // --test-threads=2 并行跑时, std::fs::write 写入后立即 fork bash subprocess 读 cfg,
    // bash 可能从 page cache 读到旧内容 (60) 而非新内容 (80), 触发 "应 FAIL" panic.
    // std::fs::rename 在 POSIX 同文件系统下是 atomic, subprocess 之后 open 一定看到新内容.
    let tmp = cfg.with_extension("toml.tmp");
    std::fs::write(&tmp, &mutated).expect("应能写 tmp config");
    std::fs::rename(&tmp, &cfg).expect("应能 rename tmp → cfg");

    // 修复 v9.4.3: 在 run_check 之前打印 cfg 状态, 排查 --test-threads=2 并行跑时偶发 "stderr 空"
    if std::env::var("TEST_VERBOSE").is_ok() {
        eprintln!(
            "TEST (before run_check): cfg threshold line = {}",
            std::fs::read_to_string(&cfg)
                .ok()
                .and_then(|s| s
                    .lines()
                    .find(|l| l.contains("event_risk_score_threshold"))
                    .map(String::from))
                .unwrap_or_else(|| "NOT FOUND".to_string())
        );
    }
    let output = run_check();
    // 显式 restore (Drop 也会跑, 这里提前清理让 backup 早释放)
    backup.restore();

    if std::env::var("TEST_VERBOSE").is_ok() {
        eprintln!("TEST: script exit status = {:?}", output.status);
        eprintln!("TEST: stderr = {}", String::from_utf8_lossy(&output.stderr));
        eprintln!("TEST: stdout = {}", String::from_utf8_lossy(&output.stdout));
    }

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

// 修复 I-9 (2026-06-29 codex review): 验证 RAII guard 在 panic 时也恢复 config.
// 这是修复 I-9 的关键: 不依赖手工 .bak 清理.
// 注: cargo test -- --test-threads=1 才能保证两个 test_design_contradiction_* 不并发.
//     如果未来加 test_design_contradiction_* 在另一个 test binary, 锁失效 (std::sync::Mutex 跨 binary 不可见).
//     修复方向: 改用 file lock (fs2 crate) 或 file-based mutex.
#[test]
fn test_config_backup_restores_on_panic() {
    let cfg = config_path();
    let original = std::fs::read_to_string(&cfg).expect("应能读 strategy.toml");

    // 用 inner scope 模拟 panic — guard 在 scope 退出时 Drop
    let result = std::panic::catch_unwind(|| {
        let _backup = ConfigBackup::new();
        // 故意改坏 config
        let mutated = original.replace(
            "event_risk_score_threshold = 60",
            "event_risk_score_threshold = 999",
        );
        std::fs::write(&cfg, &mutated).expect("应能写坏 config");
        // 模拟 panic
        panic!("simulated panic to test RAII guard");
        // _backup 在这里 Drop, 应恢复 config
    });

    assert!(result.is_err(), "catch_unwind 应捕获 panic");

    // 验证 config 已恢复
    let after = std::fs::read_to_string(&cfg).expect("应能再读 config");
    assert_eq!(after, original, "RAII guard 应在 panic 后恢复 config");
    assert!(!after.contains("999"), "config 不应残留改坏的值");
}
