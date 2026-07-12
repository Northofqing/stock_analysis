//! 领域 Newtype 定义
//!
//! 渐进迁移策略：新代码使用 newtype，旧代码保持裸 String/f64。
//! 目标：让类型系统阻止价格赋给成交量、百分比赋给价格等错误。

use std::fmt;

/// A-share 股票代码（6 位数字，以 0/3/6 开头）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StockCode(String);

impl StockCode {
    pub fn new(code: impl Into<String>) -> Result<Self, String> {
        let code: String = code.into();
        if code.len() == 6
            && code.chars().all(|c| c.is_ascii_digit())
            && matches!(code.chars().next().unwrap(), '0' | '3' | '6')
        {
            Ok(Self(code))
        } else {
            Err(format!("invalid stock code: {code}"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn market_prefix(&self) -> char {
        self.0.chars().next().unwrap()
    }
}

impl fmt::Display for StockCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 价格（人民币元）
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Price(f64);

impl Price {
    pub fn new(value: f64) -> Self {
        Self(value)
    }
    pub fn value(&self) -> f64 {
        self.0
    }
}

/// 百分比（例如 5.0 表示 5%）
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Percent(f64);

impl Percent {
    pub fn new(value: f64) -> Self {
        Self(value)
    }
    pub fn value(&self) -> f64 {
        self.0
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}%", self.0)
    }
}
