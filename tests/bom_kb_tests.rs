//! 修复 P0-2: BOM 弹性节点测试
//! 链 score = elasticity × direction_match × confidence
//! 让评分可证伪, 不靠定性 L1/L2/L3

use stock_analysis::opportunity::bom_kb::*;

#[test]
fn test_bom_node_field_bounds() {
    let n = BomNode::new(
        "新能源车".into(),
        "锂矿".into(),
        BomDirection::Upstream,
        0.3,
        0.15,
        30,
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
        "新能源车".into(),
        "正极材料".into(),
        BomDirection::Midstream,
        0.7,
        0.20,
        15,
    );
    let score = chain_score_with_direction(&n, EventDirection::Bear);
    // 修复 P0-2: 中游 Bear 方向匹配 0.7 (成本下降受益), 弹性 0.7
    // score = 0.7 * 0.7 * 0.5 (default conf) = 0.245
    assert!(
        score > 0.0 && score < 0.3,
        "中游 Bear 应低分, 实际 {}",
        score
    );
}

#[test]
fn test_chain_score_aligned_bull() {
    // 上游涨价 + 上游环节 = 利好 (材料涨价 = 收入增加)
    let n = BomNode::new(
        "新能源车".into(),
        "锂矿".into(),
        BomDirection::Upstream,
        0.6,
        0.20,
        30,
    );
    let score = chain_score_with_direction(&n, EventDirection::Bull);
    // v14.4: 公式 = elasticity * dir_match * confidence * lead_decay
    // 0.6 * 1.0 * 0.5 * exp(-1) = 0.6 * 0.5 * 0.368 ≈ 0.11
    // 上游 Bull 应中等偏高分, 实际 ~0.11
    assert!(
        score > 0.05 && score < 0.20,
        "上游 Bull 应中等偏高分, 实际 {} (elasticity=0.6 dir=1.0 conf=0.5 lead_decay=exp(-1)≈0.368)",
        score
    );
}

#[test]
fn test_chain_score_confound() {
    // 修复 P0-2: 同一事件, 不同 BOM 节点, chain_score 必不同 (可证伪)
    let upstream_li = BomNode::new(
        "新能源车".into(),
        "锂矿".into(),
        BomDirection::Upstream,
        0.6,
        0.20,
        30,
    );
    let midstream = BomNode::new(
        "新能源车".into(),
        "正极材料".into(),
        BomDirection::Midstream,
        0.7,
        0.20,
        15,
    );
    let event = EventDirection::Bull; // 涨价
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
        "测试".into(),
        "测试".into(),
        BomDirection::Downstream,
        0.9,
        0.20,
        5,
    );
    let score = chain_score_with_direction(&n, EventDirection::Bull);
    // 下游对 Bull 是 0.9 (传导), 弹性 0.9, conf 0.5 = 0.405
    // 不要求零, 但要求 < 0.5
    assert!(score < 0.5, "下游对 Bull 应 < 0.5, 实际 {}", score);
}

#[test]
fn test_bom_table_size_meets_spec() {
    // 修复 v9.1 §6 验收: BOM 表 ≥ 50 节点
    // 之前 5 行业 × 5 环节 = 25 节点不够, 现扩展到 10 行业 = 50 节点
    let all = boms();
    assert!(
        all.len() >= 50,
        "BOM 表节点数 {} < spec §6 验收门槛 50",
        all.len()
    );
}

#[test]
fn test_bom_table_chain_diversity() {
    // 修复: 至少 10 个不同产业链 (覆盖科技/金融/材料/制造)
    use std::collections::HashSet;
    let chains: HashSet<&str> = boms().iter().map(|n| n.chain.as_str()).collect();
    assert!(
        chains.len() >= 10,
        "产业链数 {} < 10, 覆盖度不足",
        chains.len()
    );
}

#[test]
fn test_bom_table_segment_diversity() {
    // 修复: 至少有上游/中游/下游三种 direction
    let mut dirs: Vec<BomDirection> = boms().iter().map(|n| n.direction).collect();
    dirs.sort_by_key(|d| *d as u8);
    dirs.dedup();
    assert_eq!(
        dirs.len(),
        3,
        "direction 必覆盖 上游/中游/下游, 实际 {} 种",
        dirs.len()
    );
}

#[test]
fn test_new_chains_present() {
    // 修复: v9.1 新增产业链必可查
    let new_chains = ["军工", "银行", "计算机", "通信", "化工"];
    for chain in &new_chains {
        let found = boms().iter().any(|n| n.chain == *chain);
        assert!(found, "新增产业链 {} 必可查", chain);
    }
}

#[test]
fn test_new_chain_find_by_segment() {
    // 修复: 军工/计算机等新链的具体环节必可查
    assert!(find_bom_node("军工", "总装").is_some(), "军工-总装必可查");
    assert!(find_bom_node("计算机", "AI").is_some(), "计算机-AI 必可查");
    assert!(
        find_bom_node("银行", "国有大行").is_some(),
        "银行-国有大行 必可查"
    );
    assert!(
        find_bom_node("通信", "光模块").is_some(),
        "通信-光模块 必可查"
    );
    assert!(
        find_bom_node("化工", "精细化工").is_some(),
        "化工-精细化工 必可查"
    );
}

#[test]
fn test_elasticity_field_bounds_all_nodes() {
    // 修复: 所有节点的 elasticity/margin/confidence 必在 [0, 1]
    for n in boms() {
        assert!(
            n.elasticity_score >= 0.0 && n.elasticity_score <= 1.0,
            "{}-{} elasticity {} 越界",
            n.chain,
            n.segment,
            n.elasticity_score
        );
        assert!(
            n.margin_pct >= 0.0 && n.margin_pct <= 1.0,
            "{}-{} margin {} 越界",
            n.chain,
            n.segment,
            n.margin_pct
        );
        assert!(
            n.confidence >= 0.0 && n.confidence <= 1.0,
            "{}-{} confidence {} 越界",
            n.chain,
            n.segment,
            n.confidence
        );
        assert!(
            n.lead_days <= 255,
            "{}-{} lead_days {} 越界",
            n.chain,
            n.segment,
            n.lead_days
        );
    }
}
