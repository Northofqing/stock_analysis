//! 枚举类型定义
//! 
//! 集中管理系统中使用的枚举类型，提供类型安全和代码可读性。

use serde::{Deserialize, Serialize};

/// 报告类型枚举
/// 
/// 用于 API 触发分析时选择推送的报告格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportType {
    /// 精简报告
    Simple,
    /// 完整报告
    Full,
}

impl ReportType {
    /// 从字符串安全地转换为枚举值
    pub fn from_str(value: &str) -> Self {
        match value.to_lowercase().trim() {
            "full" => Self::Full,
            _ => Self::Simple,
        }
    }

    /// 获取用于显示的名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Simple => "精简报告",
            Self::Full => "完整报告",
        }
    }
}

impl Default for ReportType {
    fn default() -> Self {
        Self::Simple
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_type_from_str() {
        assert_eq!(ReportType::from_str("simple"), ReportType::Simple);
        assert_eq!(ReportType::from_str("SIMPLE"), ReportType::Simple);
        assert_eq!(ReportType::from_str("full"), ReportType::Full);
        assert_eq!(ReportType::from_str("invalid"), ReportType::Simple);
    }

    #[test]
    fn test_display_name() {
        assert_eq!(ReportType::Simple.display_name(), "精简报告");
        assert_eq!(ReportType::Full.display_name(), "完整报告");
    }
}
