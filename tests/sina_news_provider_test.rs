//! Tests for SinaNewsProvider (Task 10).
//!
//! 覆盖:
//! - URL 构造: 财经要闻 (lid=1686) + 个股新闻 (lid=2516, k=code)
//! - parse_sina_news_body: JSON → Vec<NewsItem> (含 content_hash 长度)
use stock_analysis::data_provider::sina_news_provider::{
    build_stock_news_url, build_top_news_url, decode_sina_bytes, parse_sina_news_body,
};

#[test]
fn build_top_news_url_format() {
    let url = build_top_news_url(20);
    assert!(url.contains("feed.mix.sina.com.cn"));
    assert!(url.contains("lid=1686"));
    // Task 14 C2 fix: pageid=155 (实测 code:0, pageid=153 返 "未注册")
    assert!(url.contains("pageid=155"));
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

/// Task 14: 解析 Sina 财经要闻 (pageid=155, lid=1686) 实测响应.
///
/// 真实响应 (curl 抓取):
/// - `media_name` 经常为空字符串
/// - `intro` 字段是 summary
/// - 顶级字段是 `result.data` (数组)
///
/// 验证 media_name 缺失时, source_name fallback 仍为非空.
#[test]
fn parse_sina_top_news_real_response() {
    let body = r#"{
        "result": {
            "status": {"code": 0, "msg": "succ"},
            "timestamp": "Thu Jul 09 10:45:30 +0800 2026",
            "data": [
                {
                    "url": "https://finance.sina.com.cn/jjxw/2026-07-09/doc-inihenaz1479043.shtml",
                    "title": "巴中平昌：打通乡镇线上水费缴纳'最后一公里'",
                    "intro": "新华社客户端...",
                    "ctime": 1783564792,
                    "media_name": ""
                }
            ]
        }
    }"#;
    let items = parse_sina_news_body(body, "财经要闻", None).unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].title.contains("巴中平昌"));
    assert!(items[0].url.contains("doc-inihenaz1479043"));
    assert_eq!(items[0].summary, "新华社客户端...");
    // media_name 为空时, source_name 应 fallback 到默认 ("新浪财经"), 非空
    assert!(!items[0].source_name.is_empty());
}

// ============================================================================
// Batch 1 P0 #1: Sina news GBK silent decode — UTF-8 first, GBK fallback
// ============================================================================

/// P0 #1: Sina news API 实际返 UTF-8 (含中文), 强制 GBK 解会乱码.
/// decode_sina_bytes 应优先识别 UTF-8, 不会把中文弄成 mojibake.
#[test]
fn decode_sina_bytes_prefers_utf8_for_chinese() {
    // 真实 Sina news 响应: UTF-8 中文
    let body = r#"{"result":{"data":[{"url":"https://example.com/1","title":"测试中文标题","intro":"测试摘要","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let decoded = decode_sina_bytes(body.as_bytes());
    // UTF-8 路径: 中文原样保留
    assert!(
        decoded.contains("测试中文标题"),
        "UTF-8 decode 失败, got: {decoded:?}"
    );
    // 不应被 GBK mojibake 替换 (不会出现 U+FFFD 或乱码)
    assert!(!decoded.contains('\u{FFFD}'));
}

/// P0 #1: GBK 编码 body fallback — 旧版接口返 GBK 时, 也能拿到可解析字符串.
#[test]
fn decode_sina_bytes_falls_back_to_gbk() {
    use encoding_rs::GBK;
    // 构造一个 GBK 编码的 JSON
    let utf8_body = r#"{"result":{"data":[{"url":"https://example.com/x","title":"测试","intro":"摘要","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let (gbk_bytes, _, _) = GBK.encode(utf8_body);
    // GBK 编码后的 bytes 应该不是 valid UTF-8
    assert!(std::str::from_utf8(&gbk_bytes).is_err());

    // decode_sina_bytes: UTF-8 失败 → fallback GBK
    let decoded = decode_sina_bytes(&gbk_bytes);
    assert!(
        decoded.contains("测试"),
        "GBK fallback decode 失败, got: {decoded:?}"
    );

    // fallback 拿到的 body 应可被 parse
    let items = parse_sina_news_body(&decoded, "财经要闻", None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "测试");
}
