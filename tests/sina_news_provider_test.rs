//! Tests for SinaNewsProvider (Task 10).
//!
//! 覆盖:
//! - URL 构造: 财经要闻 (lid=1686) + 个股新闻 (lid=2516, k=code)
//! - parse_sina_news_body: JSON → Vec<NewsItem> (含 content_hash 长度)
use stock_analysis::data_provider::sina_news_provider::{
    build_top_news_url, build_stock_news_url, parse_sina_news_body,
};

#[test]
fn build_top_news_url_format() {
    let url = build_top_news_url(20);
    assert!(url.contains("feed.mix.sina.com.cn"));
    assert!(url.contains("lid=1686"));
    assert!(url.contains("num=20"));
}

#[test]
fn build_stock_news_url_format() {
    let url = build_stock_news_url("600000", 20);
    assert!(url.contains("lid=2516"));
    assert!(url.contains("k=600000"));
}

#[test]
fn parse_sina_news_body_extracts_items() {
    let body = r#"{"result":{"data":[{"url":"https://example.com/1","title":"新闻1","intro":"摘要1","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "财经要闻", None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "新闻1");
    assert_eq!(items[0].url, "https://example.com/1");
    assert_eq!(items[0].summary, "摘要1");
    assert_eq!(items[0].category, "财经要闻");
    assert_eq!(items[0].code, None);
    assert_eq!(items[0].content_hash.len(), 64);
}

#[test]
fn parse_sina_news_body_with_code() {
    let body = r#"{"result":{"data":[{"url":"https://example.com/2","title":"股票新闻","intro":"摘要2","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "个股新闻", Some("600000")).unwrap();
    assert_eq!(items[0].code, Some("600000".to_string()));
}
