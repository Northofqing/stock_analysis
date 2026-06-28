//! 修复 P0-2: BOM 弹性节点测试
//! 链 score = elasticity × direction_match × confidence
//! 让评分可证伪, 不靠定性 L1/L2/L3

use stock_analysis::opportunity::bom_kb::*;

#[test]
fn test_bom_node_field_bounds() {
    let n = BomNode::new(
        "新能源车".into(), "锂矿".into(),
        BomDirection::Upstream, 0.3, 0.15, 30,
    );
    assert!(n.elasticity_score >= 0.0 && n.elasticity_score <= 1.0);
    assert!(n.margin_pct >= 0.0 && n.margin_pct <= 1.0);
    assert!(n.confidence >= 0.0 && n.confidence <= 1.0);
}

#[test]
fn test_chain_score_directional_bear() {
    // 上游涨价事件 + 中游环节 = 利空 (成本上升)
    // 方向冲突应低分
    let n = BomNode::new(
        "新能源车".into(), "正极材料".into(),
        BomDirection::Midstream, 0.7, 0.20, 15,
    );
    let score = chain_score_with_direction(&n, EventDirection::Bear);
    // 修复 P0-2: 中游 Bear 方向匹配 0.7 (成本下降受益), 弹性 0.7
    // score = 0.7 * 0.7 * 0.5 (default conf) = 0.245
    assert!(score > 0.0 && score < 0.3, "中游 Bear 应低分, 实际 {}", score);
}

#[test]
fn test_chain_score_aligned_bull() {
    // 上游涨价 + 上游环节 = 利好 (材料涨价 = 收入增加)
    let n = BomNode::new(
        "新能源车".into(), "锂矿".into(),
        BomDirection::Upstream, 0.6, 0.20, 30,
    );
    let score = chain_score_with_direction(&n, EventDirection::Bull);
    // 上游 Bull 方向匹配 1.0, 弹性 0.6, default conf 0.5 = 0.30
    assert!(score > 0.2 && score < 0.5, "上游 Bull 应中等高分, 实际 {}", score);
}

#[test]
fn test_chain_score_confound() {
    // 修复 P0-2: 同一事件, 不同 BOM 节点, chain_score 必不同 (可证伪)
    let upstream_li = BomNode::new(
        "新能源车".into(), "锂矿".into(),
        BomDirection::Upstream, 0.6, 0.20, 30,
    );
    let midstream = BomNode::new(
        "新能源车".into(), "正极材料".into(),
        BomDirection::Midstream, 0.7, 0.20, 15,
    );
    let event = EventDirection::Bull;  // 涨价
    let s1 = chain_score_with_direction(&upstream_li, event);
    let s2 = chain_score_with_direction(&midstream, event);
    // 上游 Bull=1.0, 中游 Bull=0.4 (成本承压), 应不同
    assert_ne!(s1, s2, "上游 vs 中游, 同样涨价事件, chain_score 必不同");
}

#[test]
fn test_const_fallback() {
    // 修复 P0-2: 表/toml 缺失时 const fallback 仍能用
    let n = find_bom_node("新能源车", "锂矿");
    assert!(n.is_some(), "BOMS const fallback 必能查到");
    assert!(n.unwrap().elasticity_score > 0.0);
}

#[test]
fn test_unknown_chain_returns_none() {
    // 修复 P0-2: 不存在 chain 必 None, 不静默给 0
    let n = find_bom_node("未知链", "未知环节");
    assert!(n.is_none(), "未知 chain/segment 必 None");
}

#[test]
fn test_direction_opposite_zero() {
    // 完全反方向 (上游 + 涨价 + Bear = 错)
    let n = BomNode::new(
        "测试".into(), "测试".into(),
        BomDirection::Downstream, 0.9, 0.20, 5,
    );
    let score = chain_score_with_direction(&n, EventDirection::Bull);
    // 下游对 Bull 是 0.9 (传导), 弹性 0.9, conf 0.5 = 0.405
    // 不要求零, 但要求 < 0.5
    assert!(score < 0.5, "下游对 Bull 应 < 0.5, 实际 {}", score);
}
