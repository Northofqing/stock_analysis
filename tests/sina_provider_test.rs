//! Tests for SinaProvider (Task 2 + Task 3).
//!
//! Task 2 tests (3): URL builder, name.
//! Task 3 tests (2): hq URL builder, hq_str parser.

use stock_analysis::data_provider::sina_provider::{build_hq_url, build_kline_url, parse_hq_str};
use stock_analysis::data_provider::sina_provider::SinaProvider;
use stock_analysis::data_provider::DataProvider;

#[test]
fn build_kline_url_format() {
    let url = build_kline_url("600000", 5);
    assert!(url.contains("sh600000"), "URL should include sh600000 symbol, got: {url}");
    assert!(url.contains("scale=240"), "URL should request 240-min scale, got: {url}");
    assert!(url.contains("datalen=5"), "URL should request 5 datalen, got: {url}");
}

#[test]
fn build_kline_url_sz_prefix() {
    let url = build_kline_url("000001", 30);
    assert!(url.contains("sz000001"), "URL should include sz000001 symbol, got: {url}");
}

#[test]
fn sina_provider_name() {
    let p = SinaProvider::new();
    assert_eq!(p.name(), "sina_hq");
}

// ─── Task 3: hq_str 实时价 ─────────────────────────────────

#[test]
fn build_hq_url_format() {
    let url = build_hq_url("600000");
    assert!(
        url.contains("hq.sinajs.cn"),
        "URL should use hq.sinajs.cn, got: {url}"
    );
    assert!(
        url.contains("list=sh600000"),
        "URL should contain list=sh600000, got: {url}"
    );
}

#[test]
fn parse_hq_str_format() {
    // Sina 真实响应格式 (实测):
    // var hq_str_sh600519="平安银行,13.50,13.45,13.48,13.52,13.40,13.47,13.49,12345,16789,...";
    let body = r###"var hq_str_sh600519="平安银行,13.50,13.45,13.48,13.52,13.40,13.47,13.49,12345,16789,100,500,...";"###;
    let quote = parse_hq_str(body, "600519").expect("parse hq_str");
    assert_eq!(quote.current, 13.48);
    assert_eq!(quote.open, 13.50);
    assert_eq!(quote.yesterday_close, 13.45);
    assert_eq!(quote.high, 13.52);
    assert_eq!(quote.low, 13.40);
    assert!(quote.volume > 0.0);
}
