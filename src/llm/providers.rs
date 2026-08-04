//! 各 Provider 实现 — 都走 OpenAI 兼容协议
//!
//! 新增 provider: 写一个 struct + impl LlmProvider, 然后在 [`super::LlmRegistry::from_env`]
//! 里加 env 读取即可.

use super::{
    openai_compatible_chat_json, openai_compatible_chat_json_with_receipt, LlmError, LlmProvider,
    ReceiptBearingJson,
};
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

    async fn chat_json_with_receipt(
        &self,
        system: &str,
        user: &str,
    ) -> Result<ReceiptBearingJson, LlmError> {
        openai_compatible_chat_json_with_receipt(
            &self.client,
            self.name(),
            &self.model,
            system,
            user,
        )
        .await
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

    async fn chat_json_with_receipt(
        &self,
        system: &str,
        user: &str,
    ) -> Result<ReceiptBearingJson, LlmError> {
        openai_compatible_chat_json_with_receipt(
            &self.client,
            self.name(),
            &self.model,
            system,
            user,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn openai_compatible_test_server(
        actual_model: &'static str,
        response_id: &'static str,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 4_096];
                let read = socket.read(&mut chunk).await.expect("read request");
                assert!(read > 0, "request closed before headers completed");
                request.extend_from_slice(&chunk[..read]);
                if let Some(index) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let header_text = String::from_utf8_lossy(&request[..header_end]);
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 4_096];
                let read = socket.read(&mut chunk).await.expect("read request body");
                assert!(read > 0, "request closed before body completed");
                request.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&request);
            assert!(
                request.starts_with("POST /v1/chat/completions "),
                "unexpected request line: {request}"
            );

            let body = serde_json::json!({
                "id": response_id,
                "choices": [{
                    "index": 0,
                    "message": {
                        "content": "{\"answer\":\"ok\"}",
                        "tool_calls": null,
                        "role": "assistant",
                        "function_call": null
                    },
                    "finish_reason": "stop",
                    "logprobs": null
                }],
                "created": 1,
                "model": actual_model,
                "system_fingerprint": null,
                "object": "chat.completion",
                "usage": null
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            socket.shutdown().await.expect("close response");
        });
        format!("http://{address}/v1")
    }

    fn disable_proxy(client: Client<OpenAIConfig>) -> Client<OpenAIConfig> {
        client.with_http_client(
            reqwest_011::Client::builder()
                .no_proxy()
                .build()
                .expect("no-proxy HTTP client"),
        )
    }

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

    #[tokio::test]
    async fn deepseek_receipt_binds_real_upstream_response() {
        let base =
            openai_compatible_test_server("deepseek-upstream-actual", "response-actual-123").await;
        let mut provider = DeepSeekProvider::from_lookup(|name| match name {
            "DEEPSEEK_API_KEY" => Some("test-key".into()),
            "DEEPSEEK_BASE_URL" => Some(base.clone()),
            "DEEPSEEK_MODEL" => Some("configured-model-must-not-be-receipt".into()),
            _ => None,
        })
        .expect("test provider");
        provider.client = disable_proxy(provider.client);

        let completed = provider
            .chat_json_with_receipt("return JSON", "test")
            .await
            .expect("real HTTP response should carry a receipt");

        assert_eq!(completed.value(), &serde_json::json!({"answer": "ok"}));
        assert_eq!(completed.raw_content(), "{\"answer\":\"ok\"}");
        assert_eq!(completed.receipt().provider(), "deepseek");
        assert_eq!(completed.receipt().model(), "deepseek-upstream-actual");
        assert_eq!(completed.receipt().upstream_request_id(), None);
        assert_eq!(
            completed.receipt().upstream_response_id(),
            Some("response-actual-123")
        );
        assert_eq!(
            completed.receipt().response_sha256(),
            "2b82c37965faf7db40dae134cc94675e0f6dac048a0173a1861b6ed1b8db7d5b"
        );
        assert_eq!(
            completed.receipt().system_sha256(),
            "fee3cccf40984b31e0e2fa1cbaed130cc5ac2f7cb2e08ce44c316c56fc5c51e5"
        );
        assert_eq!(
            completed.receipt().user_sha256(),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
        assert!(completed.receipt().completed_at() >= completed.receipt().started_at());
    }

    #[tokio::test]
    async fn minimax_receipt_binds_real_upstream_response() {
        let base =
            openai_compatible_test_server("minimax-upstream-actual", "response-actual-123").await;
        let config = OpenAIConfig::new()
            .with_api_key("test-key")
            .with_api_base(base);
        let provider = MiniMaxProvider {
            client: disable_proxy(Client::with_config(config)),
            model: "configured-model-must-not-be-receipt".into(),
        };

        let completed = provider
            .chat_json_with_receipt("return JSON", "test")
            .await
            .expect("real HTTP response should carry a receipt");

        assert_eq!(completed.value(), &serde_json::json!({"answer": "ok"}));
        assert_eq!(completed.receipt().provider(), "minimax");
        assert_eq!(completed.receipt().model(), "minimax-upstream-actual");
        assert_eq!(
            completed.receipt().upstream_response_id(),
            Some("response-actual-123")
        );
        assert_eq!(
            completed.receipt().response_sha256(),
            "2b82c37965faf7db40dae134cc94675e0f6dac048a0173a1861b6ed1b8db7d5b"
        );
        assert_eq!(
            completed.receipt().system_sha256(),
            "fee3cccf40984b31e0e2fa1cbaed130cc5ac2f7cb2e08ce44c316c56fc5c51e5"
        );
        assert_eq!(
            completed.receipt().user_sha256(),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
        assert!(completed.receipt().completed_at() >= completed.receipt().started_at());
    }

    #[tokio::test]
    async fn receipt_fails_when_openai_protocol_response_id_is_missing() {
        let base = openai_compatible_test_server("deepseek-upstream-actual", "").await;
        let mut provider = DeepSeekProvider::from_lookup(|name| match name {
            "DEEPSEEK_API_KEY" => Some("test-key".into()),
            "DEEPSEEK_BASE_URL" => Some(base.clone()),
            "DEEPSEEK_MODEL" => Some("configured-model".into()),
            _ => None,
        })
        .expect("test provider");
        provider.client = disable_proxy(provider.client);

        let error = provider
            .chat_json_with_receipt("return JSON", "test")
            .await
            .expect_err("OpenAI-compatible responses must expose their response ID");

        assert!(matches!(
            error,
            LlmError::ReceiptUnavailable {
                provider,
                model,
            } if provider == "deepseek" && model == "deepseek-upstream-actual"
        ));
    }

    #[tokio::test]
    async fn receipt_fails_when_upstream_model_identity_is_missing() {
        let base = openai_compatible_test_server("", "response-actual-123").await;
        let mut provider = DeepSeekProvider::from_lookup(|name| match name {
            "DEEPSEEK_API_KEY" => Some("test-key".into()),
            "DEEPSEEK_BASE_URL" => Some(base.clone()),
            "DEEPSEEK_MODEL" => Some("configured-model".into()),
            _ => None,
        })
        .expect("test provider");
        provider.client = disable_proxy(provider.client);

        let error = provider
            .chat_json_with_receipt("return JSON", "test")
            .await
            .expect_err("configured model must not replace missing upstream identity");

        assert!(matches!(
            error,
            LlmError::ReceiptUnavailable {
                provider,
                model,
            } if provider == "deepseek" && model == "configured-model"
        ));
    }

    #[tokio::test]
    async fn legacy_chat_json_does_not_require_receipt_metadata() {
        let base = openai_compatible_test_server("", "").await;
        let mut provider = DeepSeekProvider::from_lookup(|name| match name {
            "DEEPSEEK_API_KEY" => Some("test-key".into()),
            "DEEPSEEK_BASE_URL" => Some(base.clone()),
            "DEEPSEEK_MODEL" => Some("configured-model".into()),
            _ => None,
        })
        .expect("test provider");
        provider.client = disable_proxy(provider.client);

        let value = provider
            .chat_json("return JSON", "test")
            .await
            .expect("legacy API behavior must remain compatible");

        assert_eq!(value, serde_json::json!({"answer": "ok"}));
    }
}
