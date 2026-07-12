//! 修复 P0-3: launch_gate 上线门槛测试
//! 量化产品经理要求:
//!  - 沙盘 → 灰度: 12 周 + 200 样本 + 60% 胜率 + Calmar 1.0
//!  - 灰度 → 实盘: 30 天 + 55% 胜率
//!  - 灰度 → 沙盘 (回退): 胜率 < 50%

use stock_analysis::opportunity::launch_gate::*;

fn metrics(shadow_days: u32, winrate_samples: u32, winrate_pct: f64, calmar: f64) -> StageMetrics {
    StageMetrics {
        shadow_days,
        winrate_samples,
        winrate_pct,
        calmar_ratio: calmar,
        gray_days: 0,
    }
}

#[test]
fn test_shadow_stays_under_threshold() {
    // 修复 P0-3: 任一条件不满足必停留在沙盘
    let m = metrics(30, 100, 0.55, 0.5);
    assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), None);
}

#[test]
fn test_shadow_to_gray_at_threshold() {
    // 修复 P0-3: 12 周 + 200 样本 + 60% 胜率 + Calmar 1.0 → 灰度
    let m = metrics(84, 250, 0.65, 1.2);
    assert_eq!(
        LaunchGate::check_transition(LaunchStage::Shadow, &m),
        Some(LaunchStage::Gray)
    );
}

#[test]
fn test_shadow_partial_fails_samples() {
    // 修复 P0-3: 样本不够, 不切换
    let m = metrics(84, 199, 0.65, 1.2);
    assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), None);
}

#[test]
fn test_shadow_partial_fails_winrate() {
    // 修复 P0-3: 胜率 < 60%, 不切换
    let m = metrics(84, 250, 0.55, 1.2);
    assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), None);
}

#[test]
fn test_shadow_partial_fails_calmar() {
    // 修复 P0-3: Calmar < 1.0, 不切换
    let m = metrics(84, 250, 0.65, 0.8);
    assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), None);
}

#[test]
fn test_shadow_partial_fails_days() {
    // 修复 P0-3: 天数 < 84 (12 周), 不切换
    let m = metrics(50, 250, 0.65, 1.2);
    assert_eq!(LaunchGate::check_transition(LaunchStage::Shadow, &m), None);
}

#[test]
fn test_gray_to_live() {
    // 修复 P0-3: 灰度 → 实盘: 30 天 + 55% 胜率
    let mut m = metrics(84, 250, 0.65, 1.2);
    m.gray_days = 30;
    let next = LaunchGate::check_transition(LaunchStage::Gray, &m);
    assert_eq!(next, Some(LaunchStage::Live));
}

#[test]
fn test_gray_back_to_shadow_underperformance() {
    // 修复 P0-3: 灰度 → 沙盘 (回退): 胜率 < 50% 或触发风控
    let mut m = metrics(84, 250, 0.45, 1.2);
    m.gray_days = 30;
    let next = LaunchGate::check_transition(LaunchStage::Gray, &m);
    assert_eq!(next, Some(LaunchStage::Shadow));
}

#[test]
fn test_gray_needs_30_days() {
    // 修复 P0-3: 灰度 < 30 天不能升级
    let mut m = metrics(84, 250, 0.65, 1.2);
    m.gray_days = 20;
    assert_eq!(LaunchGate::check_transition(LaunchStage::Gray, &m), None);
}

#[test]
fn test_live_no_auto_transition() {
    // 实盘阶段只能由人工/风控回退, LaunchGate::check_transition 不自动转
    let m = metrics(84, 250, 0.65, 1.2);
    assert_eq!(LaunchGate::check_transition(LaunchStage::Live, &m), None);
}
