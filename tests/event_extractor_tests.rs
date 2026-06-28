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
use stock_analysis::signal::market_event::EventType;

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
