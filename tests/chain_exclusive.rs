//! BR-002: 一条快讯最多命中 1 条产业链
//!
//! 修复 R-5: map_news_to_chains() 必须保证一条标题最多返回 1 条 chain hit
//! 测试策略: 选一条会同时匹配多条产业链的标题 → 断言 hits.len() <= 1

use stock_analysis::config::{activate_chain_rules_snapshot, ChainRuleConfig};
use stock_analysis::opportunity::chain_mapper::map_news_to_chains;

#[test]
fn test_map_news_returns_at_most_one_chain() {
    activate_chain_rules_snapshot(vec![
        ChainRuleConfig {
            chain: "TEST_CODE_HIGH_PRIORITY".into(),
            logic: "TEST_CODE_PRIMARY_LOGIC".into(),
            board_keyword: "半导体".into(),
            keywords: vec!["半导体".into()],
            priority: 100,
            category: "TEST_CODE_CATEGORY".into(),
            generic: false,
            enabled: true,
        },
        ChainRuleConfig {
            chain: "TEST_CODE_LOWER_PRIORITY".into(),
            logic: "TEST_CODE_SECONDARY_LOGIC".into(),
            board_keyword: "封测".into(),
            keywords: vec!["封测".into()],
            priority: 90,
            category: "TEST_CODE_CATEGORY".into(),
            generic: false,
            enabled: true,
        },
    ]);
    // "半导体" 会同时匹配 半导体/半导体-制造代工/半导体-封测 等多条规则
    // 修复后只保留优先级最高的一条
    let hits = map_news_to_chains("半导体技术突破带动封测涨价")
        .expect("explicit in-memory rule snapshot must be available");
    assert!(
        hits.len() <= 1,
        "BR-002: 一条快讯最多 1 条产业链, 实际 {} 条",
        hits.len()
    );
}
