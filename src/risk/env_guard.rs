//! Registered business rules: BR-051.
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
        Ok(_) => TradingEnv::Prod,
        Err(_) => {
            #[cfg(test)]
            {
                TradingEnv::Test
            }
            #[cfg(not(test))]
            {
                if runtime_is_test_process() {
                    TradingEnv::Test
                } else {
                    TradingEnv::Prod
                }
            }
        }
    }
}

/// Integration-test crates compile this library without `cfg(test)`. Cargo
/// still runs their executables from `target/**/deps`, which is never the
/// production monitor location. Detect that process boundary so test audits
/// cannot fall back to production directories or accept real-symbol orders.
pub fn runtime_is_test_process() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .and_then(|path| path.file_name().map(std::ffi::OsStr::to_os_string))
        .is_some_and(|name| name == "deps")
}

pub fn is_test_code(code: &str) -> bool {
    code.starts_with("TEST_CODE")
}

pub fn validate_symbol_for_env(code: &str, env: TradingEnv) -> Result<(), String> {
    let test = is_test_code(code);
    match (env, test) {
        (TradingEnv::Prod, true) => Err(format!("生产环境拒绝 TEST_CODE 标的: {}", code)),
        (TradingEnv::Test, false) => Err(format!("测试环境拒绝真实标的下单: {}", code)),
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

    #[test]
    fn unit_tests_default_to_test_environment() {
        if std::env::var("STOCK_ENV_MODE").is_err() {
            assert_eq!(current_env(), TradingEnv::Test);
        }
    }

    #[test]
    fn cargo_test_process_is_detected_for_runtime_audit_isolation() {
        assert!(runtime_is_test_process());
    }
}
