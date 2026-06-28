//! 修复 P0-1: dual_score 评分模型测试
//! event_risk_score (NS3 风险评估) + trade_signal_score (胜率信号, 可选)

use stock_analysis::opportunity::score::*;

fn base_inputs() -> ScoreInputs {
    ScoreInputs {
        event_strength: 80,
        event_certainty: 90,
        chain_match_score: 75,
        flow_score: Some(70.0),
        cross_source_count: 2,
        quality_score: Some(60.0),
        winrate_score: None,  // 默认无 winrate 数据
    }
}

#[test]
fn test_dual_score_default_event_risk_only() {
    // 修复 P0-1: 默认仅 event_risk_score, trade_signal=None (无 winrate 数据)
    let inputs = base_inputs();
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    assert!(score.event_risk_score >= 60);
    assert!(score.trade_signal_score.is_none(), "无 winrate 数据时 trade_signal 必为 None");
    assert!(score.notes.iter().any(|n| n.contains("无回测") || n.contains("无样本")),
            "notes 必须标注无历史样本");
}

#[test]
fn test_dual_score_with_winrate() {
    // 修复 P0-1: winrate 有真实数据时, trade_signal_score 也算
    let mut inputs = base_inputs();
    inputs.winrate_score = Some(0.65);
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    assert!(score.trade_signal_score.is_some());
    let tss = score.trade_signal_score.unwrap();
    assert!(tss >= 50 && tss <= 100);
    assert!(score.data_sufficiency.winrate_sufficient);
}

#[test]
fn test_winrate_zero_no_data() {
    // 修复 P1-2: 样本不足时 winrate = 0, 不假装 50
    let mut inputs = base_inputs();
    inputs.winrate_score = Some(0.0);
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    let tss = score.trade_signal_score.unwrap_or(100);
    assert_eq!(tss, 0, "无数据 winrate 必为 0, 不能 100 假装中性");
    assert!(!score.data_sufficiency.winrate_sufficient);
}

#[test]
fn test_event_risk_floor_70_no_winrate() {
    // 修复 P0-1: 无 winrate 时 event_risk_score 封顶 70 (B6 单源封顶规则的延伸)
    let mut inputs = ScoreInputs {
        event_strength: 100, event_certainty: 100,
        chain_match_score: 100, flow_score: Some(100.0),
        cross_source_count: 5, quality_score: Some(100.0),
        winrate_score: None,
    };
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    // 即便所有项满分, 无 winrate 时总分封顶 70
    assert!(score.event_risk_score <= 70,
            "无 winrate 时 event_risk_score 封顶 70, 实际 {}", score.event_risk_score);
}

#[test]
fn test_data_sufficiency_tracking() {
    // 修复 P0-1: data_sufficiency 区分真假 50
    let inputs = ScoreInputs {
        event_strength: 80, event_certainty: 90,
        chain_match_score: 75, flow_score: None,
        cross_source_count: 1, quality_score: None, winrate_score: None,
    };
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    assert!(!score.data_sufficiency.event_risk_sufficient);
    assert!(!score.data_sufficiency.has_intraday_flow);
    assert!(score.notes.iter().any(|n| n.contains("数据不足")),
            "≥ 2 项 data_sufficiency=false 必标注 数据不足");
}

#[test]
fn test_score_parts_count() {
    // 修复 P0-1: 评分可追溯, parts 含 5 项明细
    let inputs = base_inputs();
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    assert!(score.parts.len() >= 5, "parts 必含 ≥ 5 项明细");
    for part in &score.parts {
        assert!(part.value >= 0.0 && part.value <= 100.0, "{} value 越界", part.name);
        assert!(part.weight > 0.0 && part.weight <= 1.0, "{} weight 越界", part.name);
    }
}

#[test]
fn test_score_part_data_sufficiency_flag() {
    // 修复 P0-1: 每项 part 携带 data_sufficiency 标识 (区分真弱 vs 数据不足)
    let inputs = ScoreInputs {
        event_strength: 80, event_certainty: 90,
        chain_match_score: 75, flow_score: None,  // 缺
        cross_source_count: 1, quality_score: None,  // 缺
        winrate_score: None,
    };
    let score = compute_dual_score(&inputs, "v9.1-2026-06");
    let flow_part = score.parts.iter().find(|p| p.name == "flow");
    assert!(flow_part.is_some());
    let flow_part = flow_part.unwrap();
    assert!(!flow_part.data_sufficiency, "flow 缺数据时 data_sufficiency=false");
}

#[test]
fn test_weight_version_recorded() {
    // 修复 P0-1: weight_version 落审计, 上线后做胜率/夏普跟踪时按版本回溯
    let inputs = base_inputs();
    let score = compute_dual_score(&inputs, "v9.1-2026-06-test");
    assert_eq!(score.weight_version, "v9.1-2026-06-test");
}
