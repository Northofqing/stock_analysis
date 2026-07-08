//! Tests for stock_code_map helper module.
//!
//! Covers QMT format (3), Sina format (2), Baostock format (2) = 7 tests.

use stock_analysis::data_provider::stock_code_map::{
    from_baostock, from_qmt_symbol, market_of, to_baostock, to_qmt_symbol, to_sina,
};

// ----- QMT format -----

#[test]
fn to_qmt_symbol_sz_main_board() {
    assert_eq!(to_qmt_symbol("000001"), "000001.SZ");
    assert_eq!(to_qmt_symbol("301000"), "301000.SZ"); // 创业板
    assert_eq!(to_qmt_symbol("002415"), "002415.SZ"); // 中小板
}

#[test]
fn to_qmt_symbol_sh_main_board() {
    assert_eq!(to_qmt_symbol("600000"), "600000.SH");
    assert_eq!(to_qmt_symbol("688001"), "688001.SH"); // 科创板
    assert_eq!(to_qmt_symbol("900900"), "900900.SH"); // B 股
}

#[test]
fn from_qmt_symbol_strips_suffix() {
    assert_eq!(from_qmt_symbol("000001.SZ"), "000001");
    assert_eq!(from_qmt_symbol("600000.SH"), "600000");
    assert_eq!(from_qmt_symbol("999999"), "999999"); // 无后缀也 OK
}

// ----- market_of -----

#[test]
fn market_of_six_prefix_is_sh() {
    use qmt_parser::Market;
    assert!(matches!(market_of("600000"), Market::Sh));
    assert!(matches!(market_of("688001"), Market::Sh));
    assert!(matches!(market_of("900900"), Market::Sh));
}

#[test]
fn market_of_other_prefix_is_sz() {
    use qmt_parser::Market;
    assert!(matches!(market_of("000001"), Market::Sz));
    assert!(matches!(market_of("002415"), Market::Sz));
    assert!(matches!(market_of("300750"), Market::Sz));
}

// ----- Sina format (from sina/baostock plan Task 1) -----

#[test]
fn to_sina_sh_main_board() {
    assert_eq!(to_sina("600000"), "sh600000");
    assert_eq!(to_sina("688001"), "sh688001"); // 科创板
    assert_eq!(to_sina("900900"), "sh900900"); // B 股
}

#[test]
fn to_sina_sz() {
    assert_eq!(to_sina("000001"), "sz000001");
    assert_eq!(to_sina("301000"), "sz301000"); // 创业板
    assert_eq!(to_sina("002415"), "sz002415"); // 中小板
}

// ----- Baostock format -----

#[test]
fn to_baostock_format() {
    assert_eq!(to_baostock("600000"), "sh.600000");
    assert_eq!(to_baostock("000001"), "sz.000001");
    assert_eq!(to_baostock("688001"), "sh.688001");
}

#[test]
fn from_baostock_strips_prefix() {
    assert_eq!(from_baostock("sh.600000"), "600000");
    assert_eq!(from_baostock("sz.000001"), "000001");
    assert_eq!(from_baostock("600000"), "600000"); // 无前缀容错
}
