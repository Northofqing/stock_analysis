//! 基金工具函数
//!
//! 提供基金代码验证等辅助功能。

use super::constants::is_fund;
#[cfg(test)]
use super::constants::{MARKET_SH, MARKET_SZ};

/// 基金代码验证错误
#[derive(Debug, Clone)]
pub enum FundError {
    /// 不是有效的基金代码
    InvalidFundCode(String),
}

impl std::fmt::Display for FundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FundError::InvalidFundCode(code) => write!(f, "Invalid fund code: {}", code),
        }
    }
}

impl std::error::Error for FundError {}

/// 验证基金代码
///
/// 检查给定的市场代码和代码是否为有效的基金。
///
/// # Errors
///
/// 如果不是有效的基金代码，返回 `FundError::InvalidFundCode`。
///
/// # Example
/// ```
/// use magic_tdx_rs::fund::utils::validate_fund_code;
/// use magic_tdx_rs::fund::constants::{MARKET_SH, MARKET_SZ};
///
/// assert!(validate_fund_code(MARKET_SH, "510300").is_ok());
/// assert!(validate_fund_code(MARKET_SZ, "159915").is_ok());
/// assert!(validate_fund_code(MARKET_SH, "600519").is_err());
/// ```
pub fn validate_fund_code(market: u8, code: &str) -> Result<(), FundError> {
    if !is_fund(market, code) {
        return Err(FundError::InvalidFundCode(format!(
            "{}:{} is not a fund",
            market, code
        )));
    }
    Ok(())
}

/// 批量验证基金代码
pub fn validate_fund_stocks(stocks: &[(u8, &str)]) -> Result<(), FundError> {
    for (market, code) in stocks {
        validate_fund_code(*market, code)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_fund_code() {
        assert!(validate_fund_code(MARKET_SH, "510300").is_ok());
        assert!(validate_fund_code(MARKET_SZ, "159915").is_ok());
        assert!(validate_fund_code(MARKET_SH, "600519").is_err());
        assert!(validate_fund_code(MARKET_SZ, "000858").is_err());
    }

    #[test]
    fn test_validate_fund_stocks() {
        let valid = vec![(MARKET_SH, "510300"), (MARKET_SZ, "159915")];
        assert!(validate_fund_stocks(&valid).is_ok());

        let invalid = vec![(MARKET_SH, "510300"), (MARKET_SH, "600519")];
        assert!(validate_fund_stocks(&invalid).is_err());
    }
}
