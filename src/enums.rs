//! 枚举类型定义
//!
//! 集中管理系统中使用的枚举类型，提供类型安全和代码可读性。

use serde::{Deserialize, Serialize};

/// 报告类型枚举
///
/// 用于 API 触发分析时选择推送的报告格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ReportType {
    /// 精简报告
    #[default]
    Simple,
    /// 完整报告
    Full,
}

impl std::str::FromStr for ReportType {
    type Err = std::convert::Infallible;

    /// 从字符串安全地转换为枚举值；未知值保持历史默认值 `Simple`。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().trim() {
            "full" => Ok(Self::Full),
            _ => Ok(Self::Simple),
        }
    }
}

impl ReportType {
    /// 获取用于显示的名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Simple => "精简报告",
            Self::Full => "完整报告",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_type_from_str() {
        assert_eq!("simple".parse(), Ok(ReportType::Simple));
        assert_eq!("SIMPLE".parse(), Ok(ReportType::Simple));
        assert_eq!("full".parse(), Ok(ReportType::Full));
        assert_eq!("invalid".parse(), Ok(ReportType::Simple));
    }

    #[test]
    fn test_display_name() {
        assert_eq!(ReportType::Simple.display_name(), "精简报告");
        assert_eq!(ReportType::Full.display_name(), "完整报告");
    }
}
