//! 交易环境硬隔离守卫。
//!
//! AGENTS 2.5:
//! - 生产环境拒绝 TEST_CODE* 标的
//! - 测试环境拒绝真实标的下单

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingEnv {
    Prod,
    Test,
}

pub fn current_env() -> TradingEnv {
    match std::env::var("STOCK_ENV_MODE") {
        Ok(v) if v.eq_ignore_ascii_case("test") => TradingEnv::Test,
        _ => TradingEnv::Prod,
    }
}

pub fn is_test_code(code: &str) -> bool {
    code.starts_with("TEST_CODE")
}

pub fn validate_symbol_for_env(code: &str, env: TradingEnv) -> Result<(), String> {
    let test = is_test_code(code);
    match (env, test) {
        (TradingEnv::Prod, true) => Err(format!(
            "生产环境拒绝 TEST_CODE 标的: {}",
            code
        )),
        (TradingEnv::Test, false) => Err(format!(
            "测试环境拒绝真实标的下单: {}",
            code
        )),
        _ => Ok(()),
    }
}

pub fn validate_symbol_for_current_env(code: &str) -> Result<(), String> {
    validate_symbol_for_env(code, current_env())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prod_rejects_test_code() {
        let r = validate_symbol_for_env("TEST_CODE_000001", TradingEnv::Prod);
        assert!(r.is_err());
    }

    #[test]
    fn test_test_rejects_real_code() {
        let r = validate_symbol_for_env("000001", TradingEnv::Test);
        assert!(r.is_err());
    }

    #[test]
    fn test_prod_accepts_real_code() {
        let r = validate_symbol_for_env("000001", TradingEnv::Prod);
        assert!(r.is_ok());
    }

    #[test]
    fn test_test_accepts_test_code() {
        let r = validate_symbol_for_env("TEST_CODE_000001", TradingEnv::Test);
        assert!(r.is_ok());
    }
}
