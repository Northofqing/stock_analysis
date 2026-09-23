//! 基金模块常量定义
//!
//! 定义基金代码前缀、类型标识、价格系数等常量。
//! 基金与股票共享市场代码，通过代码前缀区分。
//!
//! ## 基金代码分类
//!
//! | 前缀 | 市场 | 类型 | 说明 |
//! |------|------|------|------|
//! | 501-502 | 沪 | LOF | 上市型开放式基金 |
//! | 506 | 沪 | 科创 | 科创板相关基金 |
//! | 508 | 沪 | REITs | 不动产投资信托基金 |
//! | 510 | 沪 | ETF | 大盘/综合 ETF |
//! | 511 | 沪 | 债券 | 债券 ETF |
//! | 512 | 沪 | ETF | 行业 ETF |
//! | 513 | 沪 | ETF | 跨境 ETF |
//! | 515 | 沪 | ETF | 主题 ETF |
//! | 516 | 沪 | ETF | 行业 ETF |
//! | 517 | 沪 | 跨境 | 跨境基金 |
//! | 518 | 沪 | 黄金 | 黄金 ETF |
//! | 519 | 沪 | 开放 | 传统开放式基金 |
//! | 159 | 深 | ETF | 深市 ETF/LOF |
//! | 160-161 | 深 | LOF | 上市型开放式基金 |
//! | 162-164 | 深 | 分级 | 分级基金/结构化基金 |

use serde::Serialize;

/// 沪市基金代码前缀
pub const SH_FUND_PREFIXES: &[&str] = &["50", "51"];

/// 深市基金代码前缀
pub const SZ_FUND_PREFIXES: &[&str] = &["15", "16"];

/// 场内基金价格系数参考值 (ETF/LOF/REITs，3位小数)
///
/// **注意**: TDX 协议返回的价格已经是预处理后的值，无需额外应用此系数。
/// 此常量仅作为文档参考，记录通达信基金价格的标准精度。
/// `get_fund_bars` / `get_fund_quotes` 直接返回协议原始值，与 `get_security_bars` 一致。
///
/// **例外**: 场外基金 (519xxx, OpenEnd) 的 K 线数据存在 100x 偏差，
/// 需在解析层额外处理 (待修复)。
pub const FUND_PRICE_COEFFICIENT: f64 = 0.001;

/// 基金成交量系数 (沪市，协议已预处理，无需额外应用)
pub const SH_FUND_VOL_COEFFICIENT: f64 = 1.0;

/// 基金成交量系数 (深市，协议已预处理，无需额外应用)
pub const SZ_FUND_VOL_COEFFICIENT: f64 = 0.01;

// 市场代码: 复用 crate::protocol::constants
pub use crate::protocol::constants::{MARKET_SH, MARKET_SZ};

/// 基金类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FundType {
    /// ETF (交易型开放式指数基金)
    Etf,
    /// LOF (上市型开放式基金)
    Lof,
    /// REITs (不动产投资信托基金)
    Reits,
    /// 分级基金 (结构化基金)
    Structured,
    /// 传统开放式基金
    OpenEnd,
    /// 债券基金
    Bond,
    /// 货币基金
    Money,
    /// 其他
    Other,
}

impl FundType {
    /// 获取基金类型名称
    pub fn name(&self) -> &'static str {
        match self {
            FundType::Etf => "ETF",
            FundType::Lof => "LOF",
            FundType::Reits => "REITs",
            FundType::Structured => "Structured",
            FundType::OpenEnd => "OpenEnd",
            FundType::Bond => "Bond",
            FundType::Money => "Money",
            FundType::Other => "Other",
        }
    }

    /// 获取基金类型中文名称
    pub fn name_zh(&self) -> &'static str {
        match self {
            FundType::Etf => "交易型开放式指数基金",
            FundType::Lof => "上市型开放式基金",
            FundType::Reits => "不动产投资信托基金",
            FundType::Structured => "分级基金",
            FundType::OpenEnd => "开放式基金",
            FundType::Bond => "债券基金",
            FundType::Money => "货币基金",
            FundType::Other => "其他",
        }
    }
}

/// 根据代码前缀分类基金类型
///
/// # Example
/// ```
/// use magic_tdx_rs::fund::constants::{classify_fund, FundType, MARKET_SH, MARKET_SZ};
///
/// assert_eq!(classify_fund(MARKET_SH, "510300"), FundType::Etf);
/// assert_eq!(classify_fund(MARKET_SH, "508000"), FundType::Reits);
/// assert_eq!(classify_fund(MARKET_SZ, "159915"), FundType::Etf);
/// ```
pub fn classify_fund(market: u8, code: &str) -> FundType {
    if code.len() < 3 {
        return FundType::Other;
    }
    let prefix3 = &code[..3];
    match (market, prefix3) {
        // REITs
        (_, "508") => FundType::Reits,

        // ETF (沪市)
        (_, "510" | "512" | "513" | "515" | "516") => FundType::Etf,

        // 债券 ETF
        (_, "511") => FundType::Bond,

        // LOF (沪市)
        (_, "501" | "502") => FundType::Lof,

        // 科创基金
        (_, "506") => FundType::Etf,

        // 跨境基金
        (_, "517") => FundType::Etf,

        // 黄金 ETF
        (_, "518") => FundType::Etf,

        // 传统开放式基金
        (_, "519") => FundType::OpenEnd,

        // 深市 ETF
        (0, "159") => FundType::Etf,

        // 深市 LOF
        (0, "160" | "161") => FundType::Lof,

        // 深市分级基金
        (0, "162" | "163" | "164") => FundType::Structured,

        // 其他
        _ => FundType::Other,
    }
}

/// 判断是否为沪市基金
fn is_sh_fund(code: &str) -> bool {
    SH_FUND_PREFIXES.iter().any(|p| code.starts_with(p))
}

/// 判断是否为深市基金
fn is_sz_fund(code: &str) -> bool {
    SZ_FUND_PREFIXES.iter().any(|p| code.starts_with(p))
}

/// 判断是否为基金
///
/// 根据市场代码和代码前缀判断是否为基金。
///
/// # Example
/// ```
/// use magic_tdx_rs::fund::constants::{is_fund, MARKET_SH, MARKET_SZ};
///
/// assert!(is_fund(MARKET_SH, "510300"));
/// assert!(is_fund(MARKET_SZ, "159915"));
/// assert!(!is_fund(MARKET_SH, "600519"));
/// ```
pub fn is_fund(market: u8, code: &str) -> bool {
    match market {
        MARKET_SH => is_sh_fund(code),
        MARKET_SZ => is_sz_fund(code),
        _ => false,
    }
}

/// 自动判断市场代码
///
/// 根据代码前缀自动判断市场:
/// - 以 0, 3, 15, 16, 20 开头 → 深圳 (0)
/// - 以 5, 6, 9 开头 → 上海 (1)
pub fn auto_market_code(code: &str) -> u8 {
    match code.chars().next() {
        Some('0') | Some('3') | Some('1') | Some('2') => MARKET_SZ,
        Some('5') | Some('6') | Some('9') => MARKET_SH,
        _ => MARKET_SZ, // 默认深圳
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_fund_etf() {
        assert_eq!(classify_fund(MARKET_SH, "510300"), FundType::Etf);
        assert_eq!(classify_fund(MARKET_SH, "512000"), FundType::Etf);
        assert_eq!(classify_fund(MARKET_SH, "513000"), FundType::Etf);
        assert_eq!(classify_fund(MARKET_SZ, "159915"), FundType::Etf);
    }

    #[test]
    fn test_classify_fund_reits() {
        assert_eq!(classify_fund(MARKET_SH, "508000"), FundType::Reits);
        assert_eq!(classify_fund(MARKET_SH, "508001"), FundType::Reits);
    }

    #[test]
    fn test_classify_fund_bond() {
        assert_eq!(classify_fund(MARKET_SH, "511010"), FundType::Bond);
    }

    #[test]
    fn test_classify_fund_lof() {
        assert_eq!(classify_fund(MARKET_SH, "501001"), FundType::Lof);
        assert_eq!(classify_fund(MARKET_SZ, "160105"), FundType::Lof);
    }

    #[test]
    fn test_classify_fund_structured() {
        assert_eq!(classify_fund(MARKET_SZ, "162006"), FundType::Structured);
    }

    #[test]
    fn test_classify_fund_open_end() {
        assert_eq!(classify_fund(MARKET_SH, "519001"), FundType::OpenEnd);
    }

    #[test]
    fn test_classify_fund_other() {
        assert_eq!(classify_fund(MARKET_SH, "600519"), FundType::Other);
        assert_eq!(classify_fund(MARKET_SZ, "000858"), FundType::Other);
    }

    #[test]
    fn test_is_fund() {
        assert!(is_fund(MARKET_SH, "510300"));
        assert!(is_fund(MARKET_SZ, "159915"));
        assert!(!is_fund(MARKET_SH, "600519"));
        assert!(!is_fund(MARKET_SZ, "000858"));
    }

    #[test]
    fn test_auto_market_code() {
        assert_eq!(auto_market_code("510300"), MARKET_SH);
        assert_eq!(auto_market_code("600519"), MARKET_SH);
        assert_eq!(auto_market_code("159915"), MARKET_SZ);
        assert_eq!(auto_market_code("000858"), MARKET_SZ);
    }

    #[test]
    fn test_fund_type_name() {
        assert_eq!(FundType::Etf.name(), "ETF");
        assert_eq!(FundType::Reits.name(), "REITs");
        assert_eq!(FundType::Lof.name_zh(), "上市型开放式基金");
    }
}
