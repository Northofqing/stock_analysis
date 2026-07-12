//! Tests for `data_provider::news_item` (review #16)
//!
//! 覆盖:
//! - `content_hash` 确定性 (同输入同输出, 64 字符 hex)
//! - `content_hash` 对不同输入产生不同 hash
//! - `NewsItem` serde_json 序列化含 source 字段
//!
//! 范围: 仅测试纯函数 + serde, 不触发 DB (DB insert 测试见 Step 8, 当前 task 不要求).

use chrono::Utc;
use stock_analysis::data_provider::news_item::{content_hash, NewsItem};

#[test]
fn content_hash_deterministic() {
    let h1 = content_hash("title1", "summary1");
    let h2 = content_hash("title1", "summary1");
    assert_eq!(h1, h2, "同输入应产生同 hash");
    assert_eq!(h1.len(), 64, "SHA256 hex 必须 64 字符");
}

#[test]
fn content_hash_differs_for_diff_input() {
    let h1 = content_hash("title1", "summary1");
    let h2 = content_hash("title1", "summary2");
    assert_ne!(h1, h2, "改 summary 应改变 hash");
}

#[test]
fn news_item_serializes() {
    let item = NewsItem {
        source: "sina_financial".into(),
        external_id: "https://example.com/1".into(),
        category: "财经要闻".into(),
        code: None,
        title: "Test".into(),
        summary: "Summary".into(),
        url: "https://example.com/1".into(),
        source_name: "新浪财经".into(),
        published_at: Utc::now(),
        fetched_at: Utc::now(),
        content_hash: content_hash("Test", "Summary"),
    };
    let json = serde_json::to_string(&item).unwrap();
    assert!(json.contains("sina_financial"), "json 应含 source 字段值");
}
