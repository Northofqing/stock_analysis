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

#[derive(Debug, PartialEq, Eq)]
struct DeepSeekSettings {
    key: String,
    base: String,
    model: String,
}

impl DeepSeekProvider {
    fn settings_from_lookup<F>(get: F) -> Option<DeepSeekSettings>
    where
        F: Fn(&str) -> Option<String>,
    {
        let key = get("DEEPSEEK_API_KEY").filter(|value| !value.is_empty())?;
        let base =
            get("DEEPSEEK_BASE_URL").unwrap_or_else(|| "https://api.deepseek.com/v1".to_string());
        let model = get("DEEPSEEK_MODEL").unwrap_or_else(|| "deepseek-chat".to_string());
        Some(DeepSeekSettings { key, base, model })
    }

    /// 从 key-value lookup 构造. 用于 from_env 和单测.
    fn from_lookup<F>(get: F) -> Option<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let settings = Self::settings_from_lookup(get)?;
        let cfg = OpenAIConfig::new()
            .with_api_key(settings.key)
            .with_api_base(settings.base);
        Some(Self {
            client: Client::with_config(cfg),
            model: settings.model,
        })
    }

    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }
}

#[async_trait::async_trait]
impl LlmProvider for DeepSeekProvider {
    fn name(&self) -> &'static str {
        "deepseek"
    }
    fn model(&self) -> &str {
        &self.model
    }
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
        let key = std::env::var("MiniMax_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())?;
        let base = std::env::var("MiniMax_BASE_URL")
            .unwrap_or_else(|_| "https://api.minimaxi.com/v1".to_string());
        let model =
            std::env::var("MiniMax_MODEL").unwrap_or_else(|_| "MiniMax-Text-01".to_string());
        let cfg = OpenAIConfig::new().with_api_key(key).with_api_base(base);
        Some(Self {
            client: Client::with_config(cfg),
            model,
        })
    }
}

#[async_trait::async_trait]
impl LlmProvider for MiniMaxProvider {
    fn name(&self) -> &'static str {
        "minimax"
    }
    fn model(&self) -> &str {
        &self.model
    }
    async fn chat_json(&self, system: &str, user: &str) -> Result<serde_json::Value, LlmError> {
        openai_compatible_chat_json(&self.client, &self.model, system, user).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_provider_reads_canonical_names_only() {
        let settings = DeepSeekProvider::settings_from_lookup(|name| match name {
            "DEEPSEEK_API_KEY" => Some("test-key".into()),
            "DEEPSEEK_BASE_URL" => Some("https://example.invalid/v1".into()),
            "DEEPSEEK_MODEL" => Some("deepseek-reasoner".into()),
            _ => None,
        })
        .expect("canonical DeepSeek key should create settings");

        assert_eq!(settings.key, "test-key");
        assert_eq!(settings.base, "https://example.invalid/v1");
        assert_eq!(settings.model, "deepseek-reasoner");
    }

    #[test]
    fn deepseek_provider_does_not_use_legacy_openai_names() {
        assert!(DeepSeekProvider::from_lookup(|name| match name {
            "OPENAI_API_KEY" => Some("stale-key".into()),
            "OPENAI_BASE_URL" => Some("https://api.deepseek.com/v1".into()),
            "OPENAI_MODEL" => Some("deepseek-chat".into()),
            _ => None,
        })
        .is_none());
    }
}
