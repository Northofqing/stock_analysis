use stock_analysis::opportunity::event_extractor::adapter::*;
use stock_analysis::search_service::{SearchResult, NewsType, Sentiment};

fn minimal_sr() -> SearchResult {
    SearchResult {
        title: "".into(), snippet: "".into(), url: "".into(),
        source: "".into(), published_date: Some("2026-06-27 10:30:00".into()),
        news_type: NewsType::Other,
        sentiment: Sentiment::Neutral,
        importance: 0, relevance: 0.0, keywords: vec![],
    }
}

#[test]
fn test_adapter_search_result_to_raw() {
    let sr = SearchResult {
        title: "CO2激光突破".into(),
        snippet: "半导体晶圆制造取得重大突破".into(),
        url: "https://a.com".into(),
        source: "东方财富".into(),
        published_date: Some("2026-06-27 10:30:00".into()),
        news_type: NewsType::Industry,
        sentiment: Sentiment::Positive,
        importance: 8, relevance: 0.9, keywords: vec![],
    };
    let raw = SearchResultAdapter::to_raw(&sr).unwrap();
    assert_eq!(raw.title, "CO2激光突破");
    assert!(!raw.body.is_empty());
    assert_eq!(raw.source, "东方财富");
    assert_eq!(raw.source_priority, 3);
    assert_eq!(raw.source_type, SourceType::Search);
}

#[test]
fn test_adapter_missing_published_date_fails() {
    let mut sr = minimal_sr();
    sr.published_date = None;
    sr.title = "test".into();
    sr.source = "jin10".into();
    let result = SearchResultAdapter::to_raw(&sr);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("published_date"));
}

#[test]
fn test_adapter_flash_without_body() {
    let mut sr = minimal_sr();
    sr.title = "快讯标题".into();
    sr.snippet = "".into();
    sr.source = "jin10".into();
    let raw = SearchResultAdapter::to_raw(&sr).unwrap();
    assert!(raw.body.is_empty());
    assert_eq!(raw.source_priority, 2);
}

#[test]
fn test_adapter_source_priority_mapping() {
    let mut sr = minimal_sr();
    sr.source = "巨潮".into();
    sr.title = "公告".into();
    sr.news_type = NewsType::Announcement;
    let raw = SearchResultAdapter::to_raw(&sr).unwrap();
    assert_eq!(raw.source_priority, 4);
    assert_eq!(raw.source_type, SourceType::Announcement);
}

use stock_analysis::opportunity::event_extractor::rule_filter::*;

fn raw_item(title: &str) -> RawNewsItem {
    RawNewsItem {
        title: title.into(), body: "".into(), source: "test".into(),
        source_priority: 1, source_type: SourceType::Search,
        published_at: chrono::Local::now(), url: None,
    }
}

#[test]
fn test_rule_filter_discards_noise() {
    let rm = RuleFilter::filter(&raw_item("A股收评：三大指数低开高走"));
    assert!(!rm.matched);
    assert!(rm.discard_reason.is_some());
}

#[test]
fn test_rule_filter_keeps_tech_break() {
    let rm = RuleFilter::filter(&raw_item("CO2 激光在半导体晶圆制造中取得重大突破"));
    assert!(rm.matched);
    assert_eq!(rm.event_type, Some(EventType::TechBreak));
}

#[test]
fn test_rule_filter_keeps_policy() {
    let rm = RuleFilter::filter(&raw_item("工信部：5G-A 商用部署进入新阶段"));
    assert!(rm.matched);
    assert_eq!(rm.event_type, Some(EventType::Policy));
}

#[test]
fn test_rule_filter_discards_fund() {
    let rm = RuleFilter::filter(&raw_item("XX 基金净值突破 2 元"));
    assert!(!rm.matched);
}

#[test]
fn test_rule_filter_unknown_keyword_passes() {
    let rm = RuleFilter::filter(&raw_item("某公司召开年度股东大会"));
    assert!(rm.matched, "关键词未知必保留 (AI fallback)");
    assert_eq!(rm.event_type, Some(EventType::Other));
}

use stock_analysis::opportunity::event_extractor::classifier::*;

#[test]
fn test_classifier_parse_valid_json() {
    let json = r#"{"is_event":true,"event_type":"Policy","direction":"Bull","subject":"工信部","confidence":0.9}"#;
    let out = EventClassifier::parse_response(json).unwrap();
    assert!(out.is_event);
    assert_eq!(out.event_type.unwrap(), EventType::Policy);
    assert_eq!(out.direction.unwrap(), Direction::Bull);
    assert_eq!(out.subject.unwrap(), "工信部");
    assert!((out.confidence - 0.9).abs() < 0.01);
}

#[test]
fn test_classifier_parse_non_event() {
    let json = r#"{"is_event":false,"event_type":null,"direction":null,"subject":null,"confidence":0.3}"#;
    let out = EventClassifier::parse_response(json).unwrap();
    assert!(!out.is_event);
    assert!(out.event_type.is_none());
}

#[test]
fn test_classifier_parse_garbage_returns_none() {
    let out = EventClassifier::parse_response("hello world");
    assert!(out.is_none(), "AI garbage must not panic, return None");
}

#[test]
fn test_classifier_build_prompt_uses_first_100_chars() {
    let body = "a".repeat(200);
    let prompt = EventClassifier::build_prompt("test title", &body);
    assert!(prompt.contains("test title"));
    assert!(prompt.len() < 500, "prompt should be concise");
}

use stock_analysis::opportunity::event_extractor::core::*;
use stock_analysis::opportunity::event_extractor::*;
use stock_analysis::signal::market_event::{EventType, Direction};

fn raw_t(title: &str, source: &str, st: SourceType) -> RawNewsItem {
    RawNewsItem { title: title.into(), body: "".into(), source: source.into(), source_priority: 1, source_type: st, published_at: chrono::Local::now(), url: None }
}

fn search_result(title: &str, date: &str) -> SearchResult {
    SearchResult { title: title.into(), snippet: "".into(), url: "".into(), source: "test".into(), published_date: Some(date.into()), news_type: NewsType::Other, sentiment: Sentiment::Neutral, importance: 0, relevance: 0.0, keywords: vec![] }
}

#[test]
fn test_core_parse_deep_json() {
    let json = r#"{"event_type":"TechBreak","direction":"Bull","subject":"CO2激光设备","object":"晶圆制造","strength":70,"certainty":60,"reason":"行业级技术突破"}"#;
    let raw = raw_t("CO2 激光突破", "cls", SourceType::Search);
    let me = EventExtractorCore::parse_deep_response(&raw, json).unwrap();
    assert_eq!(me.event_type, EventType::TechBreak);
    assert_eq!(me.strength, 70);
    assert_eq!(me.certainty, 60);
}

#[test]
fn test_core_quick_only_strength_lookup() {
    assert_eq!(strength_for_event_type(EventType::TechBreak, 0.85), 55);
}

#[test]
fn test_core_quick_only_certainty_lookup() {
    assert_eq!(certainty_for_source(SourceType::Flash, 0.85), 68);
}

#[test]
fn test_core_quick_only_yields_market_event() {
    let raw = raw_t("工信部政策", "cls", SourceType::Flash);
    let co = stock_analysis::opportunity::event_extractor::classifier::ClassifierOutput {
        is_event: true, event_type: Some(EventType::Policy),
        direction: Some(Direction::Bull), subject: Some("工信部".into()), confidence: 0.9,
    };
    let me = EventExtractorCore::from_quick_only(&raw, &co);
    assert_eq!(me.event_type, EventType::Policy);
    assert!(me.strength >= 20 && me.strength <= 80);
}

#[test]
fn test_extract_batch_rules_only() {
    let items = vec![
        search_result("工信部: 5G-A 商用进入新阶段", "2026-06-27 10:00:00"),
        search_result("A股收评：三大指数低开高走", "2026-06-27 10:00:00"),
        search_result("碳酸锂价格上调 5000 元", "2026-06-27 10:00:00"),
    ];
    let events = extract_batch_rules_only(&items);
    assert_eq!(events.len(), 2, "工信部 + 碳酸锂 → 2 个事件; 收评 → 丢弃");
    for e in &events {
        assert!(e.ai_degraded, "rules-only path must degrade");
    }
}

#[test]
fn test_extract_incremental_filters_stale() {
    let now = chrono::Local::now();
    let old_time = (now - chrono::Duration::hours(10)).format("%Y-%m-%d %H:%M:%S").to_string();
    let old = search_result("旧快讯", &old_time);
    let events = extract_incremental_rules_only(&[old], chrono::Duration::minutes(5));
    assert!(events.is_empty(), "stale > 5min must be discarded");
}

#[test]
fn test_extract_batch_empty_input() {
    let events = extract_batch_rules_only(&[]);
    assert!(events.is_empty());
}
