//! push_l2/template.rs — Template trait + RenderedText (v14.2 §3.2)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.2 + §3.3.2 落地.
//! W3.1 范围: trait 骨架 + 简单 InlineRegistry (W3.2 才加 build.rs TOML).
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2 — Validate trait 仅允许 Drop + RetryWithBackoff
//!   - BR-004 (no silent fallback to cost_price) — 模板渲染绝不静默填 0.0
//!   - 所有 Input 字段必须显式 Option<T>, 不允许 unwrap_or(0.0)

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

// ============================================================================
// §3.2 Template Trait
// ============================================================================

/// 模板标识 + 版本号 (用于灰度发布, b-008 §7.1)
pub trait Template: Send + Sync {
    /// 唯一 ID (例: "limit_up_v2")
    const ID: &'static str;
    /// 版本号 (例: 2)
    const VERSION: u32;

    /// Input 类型 (强类型, 业务方传错字段编译期失败)
    type Input: Clone + fmt::Debug;

    /// 模板元数据 (cooldown / quiet hours / data_mode_min 等, b-008 §3.5)
    fn metadata() -> TemplateMetadata;

    /// 核心渲染函数
    fn render(input: &Self::Input) -> RenderedText;

    /// 验证 input (W3.1 仅提供 trait, L3 Render 模块补具体实现)
    fn validate(input: &Self::Input) -> Result<(), ValidationErrors>;
}

/// 模板元数据 (governance 信息)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    /// 模板类别 (用于分类 governance)
    pub category: TemplateCategory,
    /// 静默期是否抑制 (02:00-06:00)
    pub quiet_hours_respect: bool,
    /// 冻结模式是否抑制
    pub frozen_mode_respect: bool,
    /// 最低数据质量要求 (低于此 data_mode 不推送)
    pub data_mode_min: DataMode,
    /// 同模板冷却秒数 (例: 涨停 60s, 持仓 5min)
    pub cooldown_secs: u64,
    /// 单用户每日推送上限 (None = 无限制)
    pub max_per_user_per_day: Option<u32>,
    /// 数据源全挂时是否仍推送 (用于 DataSourceDown 主动告警, b-008 §4.1)
    pub always_send_on_data_source_down: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateCategory {
    /// 持仓相关 (持仓健康度 / 仓位变化)
    Holding,
    /// 涨跌停
    LimitUp,
    /// 板块轮动
    Sector,
    /// 新闻催化
    News,
    /// 风控告警
    Risk,
    /// 数据源告警 (b-008 §4.1)
    DataSource,
    /// 盘后复盘
    PostSession,
    /// 静默时段治理
    QuietHour,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DataMode {
    /// 全部数据源正常
    Full,
    /// 部分数据源降级 (≤ 1 个失败)
    Degraded,
    /// 多数数据源失败 (2 个失败, 还可用)
    Unsafe,
    /// 全部数据源失败
    Down,
}

// ============================================================================
// §3.3.2 Validate trait + FailureStrategy
// ============================================================================

/// 数据契约验证失败处理策略 (b-009 R-1 修订: 仅 Drop + RetryWithBackoff)
///
/// **红线**: DegradeWithDefault / DegradeWithNa 已从 v14.2 删除 (触犯 AGENTS.md §2.1/§2.2)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailureStrategy {
    /// 验证失败 → 丢弃 + log warn (合规默认)
    #[default]
    Drop,
    /// 重试 max_retries 次后仍失败 → 丢弃 + log error
    RetryWithBackoff { max_retries: u32, backoff_ms: u64 },
}

/// 验证 trait (W3.1 占位, L3 Render 模块补具体实现)
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationErrors>;
    fn on_failure_strategy() -> FailureStrategy {
        FailureStrategy::default()
    }
}

#[derive(Debug, Clone)]
pub struct ValidationErrors {
    pub errors: Vec<ValidationError>,
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub kind: ValidationErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationErrorKind {
    Missing,
    OutOfRange,
    WrongType,
    InvalidEnum,
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} validation errors:", self.errors.len())?;
        for e in &self.errors {
            write!(f, "\n  - {} ({:?}): {}", e.field, e.kind, e.message)?;
        }
        Ok(())
    }
}

// ============================================================================
// RenderedText 输出
// ============================================================================

/// 渲染结果 (推送给用户的内容)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedText {
    /// 渲染后的正文 (含 banner + body)
    pub body: String,
    /// 元数据 (用于 analytics / 调试, 不影响推送内容)
    pub metadata: HashMap<String, String>,
}

impl RenderedText {
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ============================================================================
// BannerContext (W3.1 简化版)
// ============================================================================

/// Banner 上下文 — v14.2 §3.3.1 唯一强制的全局模板
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BannerContext {
    /// 推送时间戳
    pub ts: DateTime<Local>,
    /// 模板 ID
    pub template_id: String,
    /// 模板版本号
    pub template_version: u32,
    /// 数据质量
    pub data_mode: DataMode,
    /// 是否在静默期
    pub is_quiet_hour: bool,
}

impl BannerContext {
    /// 渲染 banner (W3.1 简化版, §3.3.1 完整规则在 W4 L4 dispatcher 实现)
    pub fn render(&self) -> String {
        let ts_str = self.ts.format("%H:%M").to_string();
        let data_marker = match self.data_mode {
            DataMode::Full => "",
            DataMode::Degraded => " ⚠️数据降级",
            DataMode::Unsafe => " ⚠️数据不安全",
            DataMode::Down => " ⛔数据全挂",
        };
        let quiet_marker = if self.is_quiet_hour {
            " 🌙静默期"
        } else {
            ""
        };
        format!(
            "📡 {} v{} | {}{}{}",
            self.template_id, self.template_version, ts_str, data_marker, quiet_marker
        )
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_strategy_default_is_drop() {
        // b-009 R-1: 默认必须是 Drop, 绝不允许 DegradeWithDefault
        assert_eq!(FailureStrategy::default(), FailureStrategy::Drop);
    }

    #[test]
    fn rendered_text_constructor() {
        let r = RenderedText::new("hello");
        assert_eq!(r.body, "hello");
        assert!(r.metadata.is_empty());
    }

    #[test]
    fn rendered_text_with_meta() {
        let r = RenderedText::new("body").with_meta("source", "limit_up");
        assert_eq!(r.metadata.get("source"), Some(&"limit_up".to_string()));
    }

    #[test]
    fn banner_renders_with_full_data() {
        let ctx = BannerContext {
            ts: Local::now(),
            template_id: "test_v1".to_string(),
            template_version: 1,
            data_mode: DataMode::Full,
            is_quiet_hour: false,
        };
        let banner = ctx.render();
        assert!(banner.contains("test_v1"));
        assert!(banner.contains("v1"));
        assert!(!banner.contains("数据降级"));
        assert!(!banner.contains("静默期"));
    }

    #[test]
    fn banner_marks_degraded_data() {
        let ctx = BannerContext {
            ts: Local::now(),
            template_id: "x".to_string(),
            template_version: 1,
            data_mode: DataMode::Degraded,
            is_quiet_hour: false,
        };
        assert!(ctx.render().contains("数据降级"));
    }

    #[test]
    fn banner_marks_quiet_hour() {
        let ctx = BannerContext {
            ts: Local::now(),
            template_id: "x".to_string(),
            template_version: 1,
            data_mode: DataMode::Full,
            is_quiet_hour: true,
        };
        assert!(ctx.render().contains("静默期"));
    }

    #[test]
    fn data_mode_ordering() {
        // Down > Unsafe > Degraded > Full (越严重越大)
        assert!(DataMode::Down > DataMode::Unsafe);
        assert!(DataMode::Unsafe > DataMode::Degraded);
        assert!(DataMode::Degraded > DataMode::Full);
    }

    #[test]
    fn validation_error_display() {
        let err = ValidationErrors {
            errors: vec![ValidationError {
                field: "entry_price".to_string(),
                kind: ValidationErrorKind::Missing,
                message: "entry_price is None".to_string(),
            }],
        };
        let s = format!("{}", err);
        assert!(s.contains("entry_price"));
        assert!(s.contains("Missing"));
        assert!(s.contains("1 validation"));
    }
}
