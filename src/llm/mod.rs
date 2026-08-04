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
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

/// Provider-verified evidence for one completed model call.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCallReceipt {
    /// Adapter that performed the HTTP request.
    provider: String,
    /// Model reported by the upstream response, not merely local configuration.
    model: String,
    /// Upstream request identifier when exposed by the protocol/client.
    upstream_request_id: Option<String>,
    /// Upstream response identifier when exposed by the protocol/client.
    upstream_response_id: Option<String>,
    /// SHA-256 of the exact system prompt sent to the provider.
    system_sha256: String,
    /// SHA-256 of the exact user prompt sent to the provider.
    user_sha256: String,
    /// SHA-256 of the exact response content returned by the model.
    response_sha256: String,
    /// UTC time immediately before the HTTP request.
    started_at: DateTime<Utc>,
    /// UTC time immediately after the HTTP response completed.
    completed_at: DateTime<Utc>,
}

impl ModelCallReceipt {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn upstream_request_id(&self) -> Option<&str> {
        self.upstream_request_id.as_deref()
    }

    pub fn upstream_response_id(&self) -> Option<&str> {
        self.upstream_response_id.as_deref()
    }

    pub fn system_sha256(&self) -> &str {
        &self.system_sha256
    }

    pub fn user_sha256(&self) -> &str {
        &self.user_sha256
    }

    pub fn response_sha256(&self) -> &str {
        &self.response_sha256
    }

    pub fn started_at(&self) -> &DateTime<Utc> {
        &self.started_at
    }

    pub fn completed_at(&self) -> &DateTime<Utc> {
        &self.completed_at
    }
}

/// Parsed JSON coupled to evidence for the real model call that produced it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReceiptBearingJson {
    value: Value,
    raw_content: String,
    receipt: ModelCallReceipt,
}

impl ReceiptBearingJson {
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn raw_content(&self) -> &str {
        &self.raw_content
    }

    pub fn receipt(&self) -> &ModelCallReceipt {
        &self.receipt
    }

    pub fn into_parts(self) -> (Value, String, ModelCallReceipt) {
        (self.value, self.raw_content, self.receipt)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn test_fixture(
        provider: &str,
        model: &str,
        upstream_request_id: Option<&str>,
        upstream_response_id: &str,
        system: &str,
        user: &str,
        raw_content: &str,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            value: serde_json::from_str(raw_content).expect("test response JSON"),
            raw_content: raw_content.to_owned(),
            receipt: ModelCallReceipt {
                provider: provider.to_owned(),
                model: model.to_owned(),
                upstream_request_id: upstream_request_id.map(str::to_owned),
                upstream_response_id: Some(upstream_response_id.to_owned()),
                system_sha256: sha256_hex(system),
                user_sha256: sha256_hex(user),
                response_sha256: sha256_hex(raw_content),
                started_at,
                completed_at,
            },
        }
    }
}

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
    /// Provider cannot prove a real upstream model-call receipt.
    ReceiptUnavailable { provider: String, model: String },
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
            LlmError::ReceiptUnavailable { provider, model } => write!(
                f,
                "[LLM] provider={} model={} 无法提供真实调用回执",
                provider, model
            ),
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

    /// Call the model and return parsed JSON plus a provider-verified receipt.
    ///
    /// Legacy, default and test-only providers fail closed. Implementations may
    /// override this only when they bind the result to a real upstream response.
    async fn chat_json_with_receipt(
        &self,
        _system: &str,
        _user: &str,
    ) -> Result<ReceiptBearingJson, LlmError> {
        Err(LlmError::ReceiptUnavailable {
            provider: self.name().to_string(),
            model: self.model().to_string(),
        })
    }
}

/// 通用 OpenAI 兼容调用 — 90% provider 走这个
pub(crate) async fn openai_compatible_chat_json(
    client: &Client<OpenAIConfig>,
    model: &str,
    system: &str,
    user: &str,
) -> Result<Value, LlmError> {
    Ok(openai_compatible_chat_json_raw(client, model, system, user)
        .await?
        .value)
}

/// OpenAI-compatible JSON call with an upstream-bound model-call receipt.
async fn openai_compatible_chat_json_with_receipt(
    client: &Client<OpenAIConfig>,
    provider: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<ReceiptBearingJson, LlmError> {
    let response = openai_compatible_chat_json_raw(client, model, system, user).await?;
    let actual_model =
        non_empty(response.upstream_model).ok_or_else(|| LlmError::ReceiptUnavailable {
            provider: provider.to_string(),
            model: model.to_string(),
        })?;
    let upstream_response_id =
        non_empty(response.upstream_response_id).ok_or_else(|| LlmError::ReceiptUnavailable {
            provider: provider.to_string(),
            model: actual_model.clone(),
        })?;
    let response_sha256 = sha256_hex(&response.content);

    Ok(ReceiptBearingJson {
        value: response.value,
        raw_content: response.content,
        receipt: ModelCallReceipt {
            provider: provider.to_string(),
            model: actual_model,
            upstream_request_id: None,
            upstream_response_id: Some(upstream_response_id),
            system_sha256: response.system_sha256,
            user_sha256: response.user_sha256,
            response_sha256,
            started_at: response.started_at,
            completed_at: response.completed_at,
        },
    })
}

struct OpenAiCompatibleJsonResponse {
    value: Value,
    content: String,
    upstream_model: String,
    upstream_response_id: String,
    system_sha256: String,
    user_sha256: String,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

async fn openai_compatible_chat_json_raw(
    client: &Client<OpenAIConfig>,
    model: &str,
    system: &str,
    user: &str,
) -> Result<OpenAiCompatibleJsonResponse, LlmError> {
    let system_sha256 = sha256_hex(system);
    let user_sha256 = sha256_hex(user);

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

    let started_at = Utc::now();
    let resp = client.chat().create(req).await.map_err(|e| match &e {
        async_openai::error::OpenAIError::ApiError(api) => LlmError::Api {
            status: 0, // async_openai 0.19 不暴露 status code
            body: format!("{} | {:?}", api.message, api.code),
        },
        other => LlmError::Http(format!("{:#}", other)),
    })?;
    let completed_at = Utc::now();

    let content = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| LlmError::Parse("响应无 content".into()))?;

    let value = serde_json::from_str::<Value>(&content).map_err(|e| {
        LlmError::Parse(format!(
            "JSON 解析失败: {} | content={}",
            e,
            content.chars().take(200).collect::<String>()
        ))
    })?;

    Ok(OpenAiCompatibleJsonResponse {
        value,
        content,
        upstream_model: resp.model,
        upstream_response_id: resp.id,
        system_sha256,
        user_sha256,
        started_at,
        completed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct LegacyOnlyProvider;

    #[async_trait::async_trait]
    impl LlmProvider for LegacyOnlyProvider {
        fn name(&self) -> &'static str {
            "test-only"
        }

        fn model(&self) -> &str {
            "test-model"
        }

        async fn chat_json(&self, _system: &str, _user: &str) -> Result<Value, LlmError> {
            Ok(serde_json::json!({"answer": "legacy"}))
        }
    }

    #[tokio::test]
    async fn provider_without_real_receipt_path_fails_explicitly() {
        let error = LegacyOnlyProvider
            .chat_json_with_receipt("system", "user")
            .await
            .expect_err("legacy-only providers must not fabricate model receipts");

        assert!(matches!(
            error,
            LlmError::ReceiptUnavailable {
                provider,
                model,
            } if provider == "test-only" && model == "test-model"
        ));
    }
}
