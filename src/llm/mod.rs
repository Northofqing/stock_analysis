//! LLM 抽象层 — 多 Provider 可插拔 (DeepSeek / MiniMax / OpenAI / Gemini / Claude).
//!
//! 设计目标:
//! - 业务侧只面向 [`LlmProvider`] trait, 不关心底层协议 (OpenAI 兼容 / Anthropic 兼容 / 原生)
//! - [`LlmRegistry::from_env`] 启动时按 env 加载, 业务用 `select(role)` 选
//! - role 优先 (e.g. "ticker_extraction" → MiniMax, "deep_analysis" → DeepSeek)
//! - provider 不可用 → 返回 `None`, 业务降级到规则路径
//!
//! ## 扩展新 provider
//!
//! 1. 在 `providers/` 加 `<name>.rs`, 实现 [`LlmProvider`]
//! 2. 在 [`LlmRegistry::from_env`] 加 env 读取段
//! 3. 业务调用 `registry.select("your_role")`
//!
//! 协议统一: 所有 provider 暴露 `chat_json(system, user) -> Result<Value>`, 业务 prompt 必须
//! 要求模型返回 JSON (system 里写明), provider 负责把响应解析成 Value.

pub mod providers;
pub mod registry;
pub mod ticker_extractor;

pub use registry::{LlmRegistry, LlmRole};
pub use ticker_extractor::{extract_tickers, TickerHit};

use async_openai::{config::OpenAIConfig, Client};
use serde_json::Value;
use std::fmt;

/// 统一 LLM 错误
#[derive(Debug)]
pub enum LlmError {
    /// 没有任何可用 provider (e.g. env 未配置)
    NoProvider { role: &'static str },
    /// HTTP / 网络错误
    Http(String),
    /// 模型返回非 JSON / 解析失败
    Parse(String),
    /// 模型 4xx/5xx
    Api { status: u16, body: String },
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::NoProvider { role } => write!(f, "[LLM] role={} 无可用 provider", role),
            LlmError::Http(e) => write!(f, "[LLM] HTTP 错误: {}", e),
            LlmError::Parse(e) => write!(f, "[LLM] 响应解析失败: {}", e),
            LlmError::Api { status, body } => {
                write!(
                    f,
                    "[LLM] API 错误 (status={}): {}",
                    status,
                    body.chars().take(200).collect::<String>()
                )
            }
        }
    }
}

impl std::error::Error for LlmError {}

/// 业务侧 trait — 每个 provider 实现这个
///
/// 所有 provider 走 OpenAI 兼容协议 (`/chat/completions` + JSON 模式), 这样:
/// - DeepSeek / MiniMax / OpenAI / 阿里通义 / Moonshot 全兼容
/// - Gemini 通过其 OpenAI 兼容端点 (`/v1beta/openai/`) 兼容
/// - Claude 通过代理 (e.g. anyrouter) 也兼容
///
/// 唯一例外: 原生 Anthropic 协议需要单独 impl, 留到真用 Claude 时再加
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider 名称 (用于日志 / 调试)
    fn name(&self) -> &'static str;

    /// 模型 ID
    fn model(&self) -> &str;

    /// 调用 chat completion, 要求模型返回 JSON.
    ///
    /// `system`: 系统 prompt (含角色定义 + JSON schema 要求)
    /// `user`: 用户 prompt (含具体任务)
    /// 返回: 解析后的 JSON Value
    async fn chat_json(&self, system: &str, user: &str) -> Result<Value, LlmError>;
}

/// 通用 OpenAI 兼容调用 — 90% provider 走这个
pub(crate) async fn openai_compatible_chat_json(
    client: &Client<OpenAIConfig>,
    model: &str,
    system: &str,
    user: &str,
) -> Result<Value, LlmError> {
    use async_openai::types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, ChatCompletionResponseFormat,
        ChatCompletionResponseFormatType, CreateChatCompletionRequestArgs,
    };

    let req = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(vec![
            ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system)
                    .build()
                    .map_err(|e| LlmError::Http(format!("system msg build: {}", e)))?,
            ),
            ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(user)
                    .build()
                    .map_err(|e| LlmError::Http(format!("user msg build: {}", e)))?,
            ),
        ])
        .response_format(ChatCompletionResponseFormat {
            r#type: ChatCompletionResponseFormatType::JsonObject,
        })
        .temperature(0.1)
        .max_tokens(2048u16)
        .build()
        .map_err(|e| LlmError::Http(format!("req build: {}", e)))?;

    let resp = client.chat().create(req).await.map_err(|e| match &e {
        async_openai::error::OpenAIError::ApiError(api) => LlmError::Api {
            status: 0, // async_openai 0.19 不暴露 status code
            body: format!("{} | {:?}", api.message, api.code),
        },
        other => LlmError::Http(format!("{:#}", other)),
    })?;

    let content = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| LlmError::Parse("响应无 content".into()))?;

    serde_json::from_str::<Value>(&content).map_err(|e| {
        LlmError::Parse(format!(
            "JSON 解析失败: {} | content={}",
            e,
            content.chars().take(200).collect::<String>()
        ))
    })
}
