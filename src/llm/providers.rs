//! 各 Provider 实现 — 都走 OpenAI 兼容协议
//!
//! 新增 provider: 写一个 struct + impl LlmProvider, 然后在 [`super::LlmRegistry::from_env`]
//! 里加 env 读取即可.

use super::{openai_compatible_chat_json, LlmError, LlmProvider};
use async_openai::{config::OpenAIConfig, Client};

// ============================================================================
// DeepSeek
// ============================================================================

/// DeepSeek (OpenAI 兼容) — `.env`: `DEEPSEEK_API_KEY` / `DEEPSEEK_BASE_URL` / `DEEPSEEK_MODEL`
pub struct DeepSeekProvider {
    client: Client<OpenAIConfig>,
    model: String,
}

impl DeepSeekProvider {
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("DEEPSEEK_API_KEY").ok().filter(|k| !k.is_empty())?;
        let base = std::env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string());
        let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());
        let cfg = OpenAIConfig::new().with_api_key(key).with_api_base(base);
        Some(Self { client: Client::with_config(cfg), model })
    }
}

#[async_trait::async_trait]
impl LlmProvider for DeepSeekProvider {
    fn name(&self) -> &'static str { "deepseek" }
    fn model(&self) -> &str { &self.model }
    async fn chat_json(&self, system: &str, user: &str) -> Result<serde_json::Value, LlmError> {
        openai_compatible_chat_json(&self.client, &self.model, system, user).await
    }
}

// ============================================================================
// MiniMax
// ============================================================================

/// MiniMax (OpenAI 兼容) — `.env`: `MiniMax_API_KEY` / `MiniMax_BASE_URL` / `MiniMax_MODEL`
///
/// 用户后续会接入做分析. 默认 base 留空, 由 env 强制要求.
pub struct MiniMaxProvider {
    client: Client<OpenAIConfig>,
    model: String,
}

impl MiniMaxProvider {
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("MiniMax_API_KEY").ok().filter(|k| !k.is_empty())?;
        let base = std::env::var("MiniMax_BASE_URL")
            .unwrap_or_else(|_| "https://api.minimaxi.com/v1".to_string());
        let model = std::env::var("MiniMax_MODEL").unwrap_or_else(|_| "MiniMax-Text-01".to_string());
        let cfg = OpenAIConfig::new().with_api_key(key).with_api_base(base);
        Some(Self { client: Client::with_config(cfg), model })
    }
}

#[async_trait::async_trait]
impl LlmProvider for MiniMaxProvider {
    fn name(&self) -> &'static str { "minimax" }
    fn model(&self) -> &str { &self.model }
    async fn chat_json(&self, system: &str, user: &str) -> Result<serde_json::Value, LlmError> {
        openai_compatible_chat_json(&self.client, &self.model, system, user).await
    }
}

// ============================================================================
// OpenAI 通用兼容 (Doubao / OpenAI / Gemini OpenAI 端点 / 任意代理)
// ============================================================================

/// 任意 OpenAI 兼容 provider — 通过 env 配置, 用于 DeepSeek/MiniMax 之外的接入
///
/// `.env`:
/// - `OPENAI_COMPAT_API_KEY` / `OPENAI_COMPAT_BASE_URL` / `OPENAI_COMPAT_MODEL`
/// - `OPENAI_COMPAT_NAME` (默认 "openai_compat")
pub struct OpenAiCompatProvider {
    provider_name: &'static str,
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAiCompatProvider {
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("OPENAI_COMPAT_API_KEY").ok().filter(|k| !k.is_empty())?;
        let base = std::env::var("OPENAI_COMPAT_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("OPENAI_COMPAT_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());
        let provider_name = std::env::var("OPENAI_COMPAT_NAME").unwrap_or_else(|_| "openai_compat".to_string());
        // 注意: 静态生命周期限制, 这里只把常见名字放常量池
        let name_static: &'static str = match provider_name.as_str() {
            "openai" => "openai",
            "doubao" => "doubao",
            "gemini" => "gemini",
            "moonshot" => "moonshot",
            _ => "openai_compat",
        };
        let cfg = OpenAIConfig::new().with_api_key(key).with_api_base(base);
        Some(Self { provider_name: name_static, client: Client::with_config(cfg), model })
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &'static str { self.provider_name }
    fn model(&self) -> &str { &self.model }
    async fn chat_json(&self, system: &str, user: &str) -> Result<serde_json::Value, LlmError> {
        openai_compatible_chat_json(&self.client, &self.model, system, user).await
    }
}
