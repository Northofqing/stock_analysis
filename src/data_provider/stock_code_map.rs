//! Registered business rules: BR-064.
//! Stock code format conversion helpers.
//!
//! Provides bidirectional mapping between our 6-digit internal stock codes
//! and the various formats used by upstream data sources:
//!
//! | Source       | Format examples           | Helper(s)                              |
//! | ------------ | ------------------------- | -------------------------------------- |
//! | Internal     | `"600000"`, `"000001"`    | (this is the canonical form)           |
//! | QMT          | `"600000.SH"`, `"000001.SZ"` | `to_qmt_symbol` / `from_qmt_symbol`  |
//! | QMT enum     | `qmt_parser::Market::Sh/Sz` | `market_of`                          |
//! | Sina HQ      | `"sh600000"`, `"sz000001"` | `to_sina`                            |
//! | Baostock     | `"sh.600000"`, `"sz.000001"` | `to_baostock` / `from_baostock`     |
//!
//! Classification rule (consistent across all helpers):
//! - 6/9/5 prefix → Shanghai (Sh) — 主板 / 科创板 / B 股
//! - 0/2/3 prefix → Shenzhen (Sz) — 主板 / 中小板 / 创业板

use qmt_parser::Market;

/// Our 6-digit code → QMT `code.market` format.
///
/// Examples:
/// - `"000001"` → `"000001.SZ"`
/// - `"600000"` → `"600000.SH"`
pub fn to_qmt_symbol(code: &str) -> String {
    let suffix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => ".SH",
        _ => ".SZ",
    };
    format!("{}{}", code, suffix)
}

/// QMT `code.market` → our 6-digit code.
///
/// Tolerant: if the input has no `.` separator, returns it unchanged.
pub fn from_qmt_symbol(qmt_code: &str) -> String {
    qmt_code.split('.').next().unwrap_or(qmt_code).to_string()
}

/// Our code → QMT `Market` enum.
pub fn market_of(code: &str) -> Market {
    match code.chars().next() {
        Some('6') | Some('9') | Some('5') => Market::Sh,
        _ => Market::Sz,
    }
}

/// Sina HQ interface format: `"sh600000"` / `"sz000001"`.
pub fn to_sina(code: &str) -> String {
    let prefix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => "sh",
        _ => "sz",
    };
    format!("{}{}", prefix, code)
}

/// Baostock format: `"sh.600000"` / `"sz.000001"` (dot in the middle).
pub fn to_baostock(code: &str) -> String {
    let prefix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => "sh",
        _ => "sz",
    };
    format!("{}.{}", prefix, code)
}

/// Baostock → our 6-digit code. Tolerant: input without a prefix passes through.
pub fn from_baostock(bs_code: &str) -> String {
    bs_code.split('.').nth(1).unwrap_or(bs_code).to_string()
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    #[test]
    fn qmt_roundtrip() {
        for c in ["600000", "000001", "688001", "301000", "900900"] {
            assert_eq!(
                from_qmt_symbol(&to_qmt_symbol(c)),
                c,
                "roundtrip failed for {c}"
            );
        }
    }

    #[test]
    fn sina_prefix_consistent_with_market() {
        // 6 开头 = sh
        assert!(to_sina("600000").starts_with("sh"));
        // 0 开头 = sz
        assert!(to_sina("000001").starts_with("sz"));
    }

    #[test]
    fn baostock_roundtrip() {
        for c in ["600000", "000001", "688001", "301000"] {
            assert_eq!(
                from_baostock(&to_baostock(c)),
                c,
                "roundtrip failed for {c}"
            );
        }
    }
}
