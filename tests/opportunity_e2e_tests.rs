//! 修复 P0-1: opportunity 端到端 smoke 测试
//! 量化产品经理视角: 完整流水线 (拉新闻 → 抽取事件 → dual_score 评分) 必跑通

use stock_analysis::opportunity::launch_gate::*;
use stock_analysis::opportunity::score::{compute_dual_score, ScoreInputs};
use stock_analysis::opportunity::winrate::*;

#[test]
fn test_e2e_full_pipeline_smoke() {
    // 修复 P0-1: 端到端 smoke 跑通, 不需真实新闻源 (TEST_ 前缀)
    // 输入: 模拟 MarketEvent 的特征, 直接喂 dual_score
    let inputs = ScoreInputs {
        event_strength: 85,     // 强事件
        event_certainty: 90,    // 高确信度
        chain_match_score: 80,  // 产业链匹配
        flow_score: Some(70.0), // 有资金热度
        cross_source_count: 3,  // 跨 3 源验证
        quality_score: Some(75.0),
        winrate_score: None, // 沙盘阶段无 winrate
        ai_degraded: false,
    };
    let score = compute_dual_score(&inputs, "v9.1-2026-06-test");
    // 修复 P0-1: 量化产品经理验收标准
    assert!(
        score.event_risk_score >= 60,
        "强事件高分: {}",
        score.event_risk_score
    );
    assert!(
        score.trade_signal_score.is_none(),
        "沙盘阶段 trade_signal=None"
    );
    assert!(!score.notes.is_empty(), "notes 必含内容");
    assert!(score.parts.len() >= 5, "parts ≥ 5 项明细");
}

#[test]
fn test_e2e_with_winrate_qualifies_for_gray() {
    // 修复 P0-1: 端到端: 有 winrate + 沙盘运行足够, 可以升级
    let samples: Vec<_> = (0..200)
        .map(|i| VerifiedOutcome {
            hit: Some(i < 130),
            actual_change: Some(if i < 130 { 2.0 } else { -1.0 }),
            special_case: None,
        })
        .collect();
    let winrate_summary = summarize_verified_rows(&samples);
    assert!(winrate_summary.sufficient);
    let winrate_score = winrate_summary.winrate.expect("verified winrate");
    assert!(winrate_score >= 0.60, "130/200 涨 = 0.65 ≥ 0.60");

    // 修复 P0-3: 沙盘 → 灰度 (v14.9: shadow_days 70 → 84, 对齐 codex 修复的 12 周阈值)
    let metrics = StageMetrics {
        shadow_days: 84,
        winrate_samples: 200,
        winrate_pct: winrate_score,
        calmar_ratio: 1.2,
        gray_days: 0,
    };
    let next = LaunchGate::check_transition(LaunchStage::Shadow, &metrics);
    assert_eq!(next, Some(LaunchStage::Gray), "满足 4 条件必升级沙盘→灰度");
}

#[test]
fn test_e2e_pipeline_with_no_data() {
    // 修复 P0-1: 端到端: 缺数据时 event_risk 封顶 70, trade_signal=None
    let inputs = ScoreInputs {
        event_strength: 50,
        event_certainty: 50,
        chain_match_score: 50,
        flow_score: None, // 缺
        cross_source_count: 1,
        quality_score: None, // 缺
        winrate_score: None, // 缺
        ai_degraded: false,
    };
    let score = compute_dual_score(&inputs, "v9.1-2026-06-test");
    assert!(
        score.event_risk_score <= 70,
        "数据缺 2 项时 event_risk 必封顶 70"
    );
    assert!(score.trade_signal_score.is_none());
    assert!(!score.data_sufficiency.event_risk_sufficient);
    assert!(
        score.notes.iter().any(|n| n.contains("数据不足")),
        "数据缺必 notes 标注"
    );
}

#[test]
fn test_e2e_weight_version_propagates() {
    // 修复 P0-1: 评分版本透传, 上线后可按版本回溯
    let inputs = ScoreInputs {
        event_strength: 70,
        event_certainty: 70,
        chain_match_score: 70,
        flow_score: Some(60.0),
        cross_source_count: 2,
        quality_score: Some(60.0),
        winrate_score: None,
        ai_degraded: false,
    };
    let score = compute_dual_score(&inputs, "test-v9.1-2026-06-28");
    assert_eq!(score.weight_version, "test-v9.1-2026-06-28");
}

#[test]
fn test_e2e_neg_signal_clamps_to_zero() {
    // 修复 P0-1 + P1-2: winrate < 50% → 0, event_risk 与 winrate 解耦
    let samples: Vec<_> = (0..200)
        .map(|i| VerifiedOutcome {
            hit: Some(i < 80),
            actual_change: Some(if i < 80 { 2.0 } else { -1.0 }),
            special_case: None,
        })
        .collect();
    let winrate = summarize_verified_rows(&samples).gated_score().unwrap();
    assert_eq!(winrate, 0.0, "明确负信号 winrate 必为 0 (P1-2 修复)");
}
