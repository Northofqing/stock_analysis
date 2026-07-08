//! Tests for SinaProvider skeleton (Task 2).
//!
//! TDD Step 2: these tests should FAIL initially because the
//! `sina_provider` module does not yet exist. After Step 4
//! (implementation) they must pass.

use stock_analysis::data_provider::sina_provider::build_kline_url;
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
