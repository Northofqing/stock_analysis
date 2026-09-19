//! g5b 深链归因的 "无 provider" 跳过分支不得绕过循环尾部的 sleep。
//!
//! 缺陷: 该分支用裸 `continue` 回到 loop 头部, 跳过了循环尾部
//! `sleep(30s)` (paper 决策循环, main.rs 内), 在未配置 DEEPSEEK_API_KEY
//! 且当日有告警时造成无退避空转 (日志与 CPU 双刷)。
//!
//! 为什么是源码形状守卫而非行为测试: 空转只在运行中的常驻循环里可观察,
//! 需要真实运行 monitor 并等待多轮迭代, 不适合单元/集成测试。此守卫的
//! 作用是**防止该 sleep 被再次静默移除**, 它不构成行为 RED —— 本次修复的
//! 前提证据是对循环边界与尾部 sleep 位置的静态核实 (loop 8725-9342,
//! continue 于分支处, 尾部 sleep 在其后)。

#[test]
fn g5b_no_provider_branch_sleeps_before_continuing() {
    let source = include_str!("../src/bin/monitor/main.rs");

    let warn_at = source
        .find("[g5b] 深链归因: 无可用 LLM provider")
        .expect("g5b no-provider warning must still exist");
    let tail = &source[warn_at..];

    let continue_at = tail
        .find("continue;")
        .expect("g5b no-provider branch must still skip with continue");

    let between = &tail[..continue_at];
    assert!(
        between.contains("sleep"),
        "g5b 无 provider 分支必须在 continue 前 sleep, 否则绕过循环尾部 sleep 空转"
    );
}
