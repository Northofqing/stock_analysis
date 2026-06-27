//! 修复 P0-1: MarketEvent 标准中间件测试
//! MarketEvent 是 v9 流水线全链路标准中间件 (NS1)
//! ② 事件抽取 → ③ 映射 → ④ 公司 → ⑤ 回测 → ⑥ 资金 → ⑦ 评分 都消费它

use stock_analysis::signal::market_event::*;

#[test]
fn test_market_event_default_strength_certainty() {
    // 修复 P0-1: strength 和 certainty 正交, 缺一不可
    // 强信号 (传闻) + 0 确信度 → 不能解读为"无信号"
    let e = MarketEvent::new(
        EventType::Policy,
        "工信部".to_string(),
        Some("5G-A".to_string()),
        Direction::Bull,
        80,
        0,
    );
    assert_eq!(e.strength, 80);
    assert_eq!(e.certainty, 0);
    assert!(!e.ai_degraded);  // 默认 false
}

#[test]
fn test_event_id_format() {
    // event_id = sha256 hex (64 字符)
    let e = MarketEvent::new(
        EventType::Policy, "test".to_string(), None,
        Direction::Bull, 50, 50,
    );
    assert_eq!(e.event_id.len(), 64);
    assert!(e.event_id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_event_id_distinct_by_subject() {
    // 同样 EventType + Direction, subject 不同 → event_id 不同
    let a = MarketEvent::new(EventType::Policy, "A".to_string(), None, Direction::Bull, 50, 50);
    let b = MarketEvent::new(EventType::Policy, "B".to_string(), None, Direction::Bull, 50, 50);
    assert_ne!(a.event_id, b.event_id);
}

#[test]
fn test_strength_certainty_bounded() {
    // 修复: 超过 100 必 clamp (u8 不能负, 用 saturating_sub 模拟)
    let e = MarketEvent::new(
        EventType::Policy, "x".into(), None, Direction::Bull, 255, 5,
    );
    assert!(e.strength <= 100, "strength {} 必 clamp 到 ≤100", e.strength);
    assert_eq!(e.strength, 100);
    assert_eq!(e.certainty, 5);
}

#[test]
fn test_ai_degraded_flag_persists() {
    let mut e = MarketEvent::new(
        EventType::Policy, "x".into(), None, Direction::Bull, 50, 50,
    );
    e.ai_degraded = true;
    assert!(e.ai_degraded);
    // 量化产品经理要求: ai_degraded=true 时下游必须降权, 不能编造
}

#[test]
fn test_chains_initially_empty() {
    // 修复 P0-1 职责切分: ② 抽取阶段 chains 恒为空, 由 ③ 映射填充
    let e = MarketEvent::new(
        EventType::Policy, "x".into(), None, Direction::Bull, 50, 50,
    );
    assert!(e.chains.is_empty(), "MarketEvent 构造时 chains 必为空, 由 ③ 阶段填充");
}

#[test]
fn test_source_ref_provenance() {
    // 修复 P0-1: provenance 落审计 (跨源验证)
    let mut e = MarketEvent::new(
        EventType::Policy, "x".into(), None, Direction::Bull, 50, 50,
    );
    e.provenance.push(SourceRef {
        provider: "东财".into(),
        url: Some("https://example.com/a".into()),
        fetched_at: chrono::Local::now(),
    });
    e.provenance.push(SourceRef {
        provider: "新浪".into(),
        url: None,
        fetched_at: chrono::Local::now(),
    });
    assert_eq!(e.provenance.len(), 2);
    assert_eq!(e.provenance[0].provider, "东财");
    assert_eq!(e.provenance[1].provider, "新浪");
}

#[test]
fn test_event_type_classification() {
    // 修复 P0-1: EventType 枚举化, 不允许字符串乱填
    let e = MarketEvent::new(
        EventType::Mna, "公司A".into(), Some("公司B".into()),
        Direction::Bull, 70, 80,
    );
    assert_eq!(e.event_type, EventType::Mna);
    assert_eq!(e.object, Some("公司B".into()));
}
