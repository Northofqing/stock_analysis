//! Tests for BaostockProvider URL/format helpers.
//!
//! 覆盖 Task 5 (BaostockProvider 骨架):
//! - build_login_url / build_logout_url — URL 构造
//! - build_kline_query_body — form body 包含所有关键字段 (code/fields/adjustflag/sessionid)
//! - parse_baostock_response — key=value 行解析 (含 Missing 返回 None)

use stock_analysis::data_provider::baostock_provider::{
    build_kline_query_body, build_login_url, build_logout_url, parse_baostock_response,
};

#[test]
fn parse_kline_body_format() {
    // Baostock 响应格式 (实测):
    // code,date,open,high,low,close,volume,amount
    // sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50
    let body = "code,date,open,high,low,close,volume,amount\n\
                sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50\n\
                sh.600000,2024-01-16,13.55,13.70,13.50,13.65,15000,20000.00\n";
    let klines = stock_analysis::data_provider::baostock_provider::parse_kline_body(body, "600000").unwrap();
    assert_eq!(klines.len(), 2);
    assert_eq!(klines[0].open, 13.50);
    assert_eq!(klines[0].close, 13.55);
    assert_eq!(klines[0].volume, 12345.0);
    assert_eq!(klines[0].amount, 16789.50);
    assert_eq!(klines[1].date, chrono::NaiveDate::from_ymd_opt(2024, 1, 16).unwrap());
}

#[test]
fn test_build_login_url() {
    assert_eq!(build_login_url(), "http://baostock.com/baostock/Login");
}

#[test]
fn test_build_logout_url() {
    assert_eq!(build_logout_url(), "http://baostock.com/baostock/Logout");
}

#[test]
fn build_kline_query_body_format() {
    let body = build_kline_query_body(
        "sh.600000",
        "date,open,high,low,close",
        "20240101",
        "20241231",
        "session_xxx",
    );
    assert!(body.contains("QueryHistoryKLinePlus"));
    assert!(body.contains("code=sh.600000"));
    assert!(body.contains("adjustflag=2")); // 前复权
    assert!(body.contains("sessionid=session_xxx"));
}

#[test]
fn parse_baostock_response_extracts_field() {
    let body = "sessionId=ABC123\nErrorCode=0\nErrorMsg=success\n";
    assert_eq!(
        parse_baostock_response(body, "sessionId").unwrap(),
        Some("ABC123".to_string())
    );
    assert_eq!(
        parse_baostock_response(body, "ErrorCode").unwrap(),
        Some("0".to_string())
    );
    assert_eq!(
        parse_baostock_response(body, "Missing").unwrap(),
        None
    );
}
