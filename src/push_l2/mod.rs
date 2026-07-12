//! push_l2 — v14.2 Layer 2: Template Registry (模板注册层)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.2 落地.
//! W3.1 状态: 只落地 `template.rs` (Template trait + 配套类型).
//! 后续 W3.2 会加: build.rs + TOML 解析 + 静态注册 inventory.

pub mod template;

// 重新导出主要类型, 方便 L4 dispatcher 一行 use
pub use template::{
    BannerContext, DataMode, FailureStrategy, RenderedText, Template, TemplateCategory,
    TemplateMetadata, Validate, ValidationError, ValidationErrorKind, ValidationErrors,
};