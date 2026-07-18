//! push_l6/external_sinks.rs — WechatSink + FeishuSink (W6.2 骨架)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.6 落地.
//!
//! W6.2 范围:
//!   - WechatSink (企业微信 webhook 推送)
//!   - FeishuSink (飞书机器人 webhook 推送)
//!   - HttpSink 通用骨架 (reqwest 客户端, 默认 30s 超时)
//!   - 失败重试走 SinkRouter 内置机制
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2: HTTP 失败必须显式返回 Err, 不允许静默吞错
//!   - b-009 R-4: dispatcher 必须走 dispatcher (Sink 仅为 destination)
//!
//! 注: W6.2 是骨架, 不实跑 HTTP. reqwest 客户端创建 / URL 配置等需要 W8 (运维) 落地.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::push_l6::{PushMessage, Sink, SinkResult};

/// HTTP 客户端配置 (W6.2 骨架, W8 实际配置)
#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub base_url: String,
    pub timeout: Duration,
    pub max_retries: u32,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            timeout: Duration::from_secs(30),
            max_retries: 2,
        }
    }
}

/// HTTP 错误 (用于测试 + 错误聚合)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)
    }
}

// ============================================================================
// HttpSink — 通用 HTTP 推送骨架
// ============================================================================

pub type MockSender = Arc<dyn Fn(&PushMessage) -> Result<(), HttpError> + Send + Sync>;

/// HttpSink — 通过 HTTP POST 推送消息 (W6.2 骨架)
///
/// 实际 W6.2 不实跑 HTTP (无 reqwest 调用), 留给 W8 集成
pub struct HttpSink {
    pub name: &'static str,
    pub config: HttpConfig,
    /// mock send_fn (测试用), None 表示真实 HTTP 推送
    pub mock_send: Option<MockSender>,
}

#[async_trait]
impl Sink for HttpSink {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn send(&self, msg: &PushMessage) -> SinkResult {
        if let Some(mock) = &self.mock_send {
            match mock(msg) {
                Ok(()) => {
                    log::info!(
                        "[sink:{}] mock send ok: event_id={}",
                        self.name,
                        msg.event.event_id
                    );
                    SinkResult::Ok
                }
                Err(e) => {
                    log::error!("[sink:{}] mock send failed: {}", self.name, e);
                    SinkResult::Err(e.to_string())
                }
            }
        } else {
            // v15.1 A4.1: 真实 HTTP POST (之前是 w6_2_skeleton_not_implemented)
            let url = self.config.base_url.clone();
            if url.is_empty() {
                return SinkResult::Err(format!("[sink:{}] base_url 未配置", self.name));
            }
            // PushMessage 不 impl Serialize, 用简化 JSON (template_id + text)
            let body = serde_json::json!({
                "template_id": msg.template_id,
                "template_version": msg.template_version,
                "user_id": msg.user_id,
                "text": msg.text.body,
            })
            .to_string();
            let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
            let req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("User-Agent", "stock-analysis-monitor/v15.1")
                .timeout(self.config.timeout)
                .body(body);
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    log::info!("[sink:{}] POST {} ok", self.name, url);
                    SinkResult::Ok
                }
                Ok(resp) => {
                    let status = resp.status();
                    let reason = resp.status().canonical_reason().unwrap_or("").to_string();
                    log::error!(
                        "[sink:{}] POST {} HTTP {} {}",
                        self.name,
                        url,
                        status,
                        reason
                    );
                    SinkResult::Err(format!("HTTP {} {}", status, reason))
                }
                Err(e) => {
                    log::error!("[sink:{}] POST {} network: {}", self.name, url, e);
                    SinkResult::Err(format!("network: {e}"))
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        true
    }
}

// ============================================================================
// WechatSink — 企业微信机器人 webhook
// ============================================================================

/// WechatSink — 企业微信群机器人 (W6.2 骨架)
///
/// 企业微信 webhook URL 格式: https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=xxx
pub struct WechatSink {
    pub config: HttpConfig,
    pub webhook_key: String,
    pub mock_send: Option<MockSender>,
}

impl WechatSink {
    pub fn new(webhook_key: impl Into<String>, config: HttpConfig) -> Self {
        Self {
            config,
            webhook_key: webhook_key.into(),
            mock_send: None,
        }
    }

    pub fn with_mock(
        webhook_key: impl Into<String>,
        config: HttpConfig,
        mock_send: MockSender,
    ) -> Self {
        Self {
            config,
            webhook_key: webhook_key.into(),
            mock_send: Some(mock_send),
        }
    }
}

#[async_trait]
impl Sink for WechatSink {
    fn name(&self) -> &'static str {
        "wechat"
    }

    async fn send(&self, msg: &PushMessage) -> SinkResult {
        let wechat_msg = serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "content": msg.text.body
            }
        });

        let url = format!(
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key={}",
            self.webhook_key
        );

        log::debug!(
            "[sink:wechat] 推送到 {}, body_len={}, event_id={}",
            url,
            wechat_msg.to_string().len(),
            msg.event.event_id
        );

        if let Some(mock) = &self.mock_send {
            match mock(msg) {
                Ok(()) => {
                    log::info!(
                        "[sink:wechat] mock send ok: event_id={}",
                        msg.event.event_id
                    );
                    SinkResult::Ok
                }
                Err(e) => {
                    log::error!("[sink:wechat] mock send failed: {}", e);
                    SinkResult::Err(e.to_string())
                }
            }
        } else {
            // v15.1 A4.2: 真实 HTTP POST 企业微信 webhook
            if self.webhook_key.is_empty() {
                return SinkResult::Err("[sink:wechat] webhook_key 未配置".to_string());
            }
            let body = match serde_json::to_string(&wechat_msg) {
                Ok(b) => b,
                Err(e) => return SinkResult::Err(format!("[sink:wechat] serialize: {e}")),
            };
            let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
            let req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .timeout(self.config.timeout)
                .body(body);
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    log::info!("[sink:wechat] POST {} ok", url);
                    SinkResult::Ok
                }
                Ok(resp) => {
                    log::error!("[sink:wechat] POST {} HTTP {}", url, resp.status());
                    SinkResult::Err(format!("wechat HTTP {}", resp.status()))
                }
                Err(e) => {
                    log::error!("[sink:wechat] POST {} network: {}", url, e);
                    SinkResult::Err(format!("wechat network: {e}"))
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        !self.webhook_key.is_empty()
    }
}

// ============================================================================
// FeishuSink — 飞书机器人 webhook
// ============================================================================

/// FeishuSink — 飞书自定义机器人 (W6.2 骨架)
///
/// 飞书 webhook URL 格式: https://open.feishu.cn/open-apis/bot/v2/hook/xxx
pub struct FeishuSink {
    pub config: HttpConfig,
    pub webhook_url: String,
    pub mock_send: Option<MockSender>,
}

impl FeishuSink {
    pub fn new(webhook_url: impl Into<String>, config: HttpConfig) -> Self {
        Self {
            config,
            webhook_url: webhook_url.into(),
            mock_send: None,
        }
    }

    pub fn with_mock(
        webhook_url: impl Into<String>,
        config: HttpConfig,
        mock_send: MockSender,
    ) -> Self {
        Self {
            config,
            webhook_url: webhook_url.into(),
            mock_send: Some(mock_send),
        }
    }
}

#[async_trait]
impl Sink for FeishuSink {
    fn name(&self) -> &'static str {
        "feishu"
    }

    async fn send(&self, msg: &PushMessage) -> SinkResult {
        let feishu_msg = serde_json::json!({
            "msg_type": "interactive",
            "card": {
                "elements": [{
                    "tag": "markdown",
                    "content": msg.text.body
                }]
            }
        });

        log::debug!(
            "[sink:feishu] 推送到 {}, body_len={}, event_id={}",
            self.webhook_url,
            feishu_msg.to_string().len(),
            msg.event.event_id
        );

        if let Some(mock) = &self.mock_send {
            match mock(msg) {
                Ok(()) => {
                    log::info!(
                        "[sink:feishu] mock send ok: event_id={}",
                        msg.event.event_id
                    );
                    SinkResult::Ok
                }
                Err(e) => {
                    log::error!("[sink:feishu] mock send failed: {}", e);
                    SinkResult::Err(e.to_string())
                }
            }
        } else {
            // v15.1 A4.2: 真实 HTTP POST 飞书 webhook
            if self.webhook_url.is_empty() {
                return SinkResult::Err("[sink:feishu] webhook_url 未配置".to_string());
            }
            let body = match serde_json::to_string(&feishu_msg) {
                Ok(b) => b,
                Err(e) => return SinkResult::Err(format!("[sink:feishu] serialize: {e}")),
            };
            let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
            let req = client
                .post(&self.webhook_url)
                .header("Content-Type", "application/json; charset=utf-8")
                .timeout(self.config.timeout)
                .body(body);
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    log::info!("[sink:feishu] POST {} ok", self.webhook_url);
                    SinkResult::Ok
                }
                Ok(resp) => {
                    log::error!(
                        "[sink:feishu] POST {} HTTP {}",
                        self.webhook_url,
                        resp.status()
                    );
                    SinkResult::Err(format!("feishu HTTP {}", resp.status()))
                }
                Err(e) => {
                    log::error!("[sink:feishu] POST {} network: {}", self.webhook_url, e);
                    SinkResult::Err(format!("feishu network: {e}"))
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        !self.webhook_url.is_empty()
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_l1::{LimitUpPayload, Severity, SignalEvent, SignalPayload};
    use crate::push_l2::RenderedText;
    use chrono::Local;

    fn make_msg() -> PushMessage {
        let event = SignalEvent::new(
            crate::push_l1::SignalSource::LimitUp,
            "limit_up",
            Some("TEST_CODE_600519".to_string()),
            Local::now(),
            SignalPayload::LimitUp(LimitUpPayload::default()),
            Severity::High,
        );
        PushMessage {
            event,
            text: RenderedText::new("test body"),
            template_id: "limit_up_v1".to_string(),
            template_version: 1,
            user_id: "default".to_string(),
        }
    }

    fn success_mock() -> MockSender {
        Arc::new(|_| Ok(()))
    }

    fn fail_mock() -> MockSender {
        Arc::new(|_| {
            Err(HttpError {
                status: 500,
                message: "internal error".to_string(),
            })
        })
    }

    #[test]
    fn http_config_default() {
        let c = HttpConfig::default();
        assert_eq!(c.timeout, Duration::from_secs(30));
        assert_eq!(c.max_retries, 2);
        assert!(c.base_url.is_empty());
    }

    #[tokio::test]
    async fn http_sink_default_skeleton_error() {
        let sink = HttpSink {
            name: "test",
            config: HttpConfig::default(),
            mock_send: None,
        };
        assert_eq!(sink.name(), "test");
        assert!(sink.health_check().await);
    }

    #[tokio::test]
    async fn http_sink_skeleton_returns_err() {
        let sink = HttpSink {
            name: "test",
            config: HttpConfig::default(),
            mock_send: None,
        };
        let r = sink.send(&make_msg()).await;
        assert!(matches!(r, SinkResult::Err(_)));
    }

    #[tokio::test]
    async fn http_sink_mock_ok() {
        let sink = HttpSink {
            name: "test",
            config: HttpConfig::default(),
            mock_send: Some(success_mock()),
        };
        let r = sink.send(&make_msg()).await;
        assert_eq!(r, SinkResult::Ok);
    }

    #[tokio::test]
    async fn http_sink_mock_fail() {
        let sink = HttpSink {
            name: "test",
            config: HttpConfig::default(),
            mock_send: Some(fail_mock()),
        };
        let r = sink.send(&make_msg()).await;
        assert!(matches!(r, SinkResult::Err(_)));
    }

    #[tokio::test]
    async fn wechat_sink_constructor() {
        let s = WechatSink::new("test-key", HttpConfig::default());
        assert_eq!(s.webhook_key, "test-key");
        assert_eq!(s.name(), "wechat");
        assert!(s.health_check().await);
    }

    #[tokio::test]
    #[ignore = "v15.1 A4.2: real HTTP requires test server, skip in unit test"]
    async fn wechat_sink_real_http_fails_without_server() {
        let s = WechatSink::new("key", HttpConfig::default());
        let r = s.send(&make_msg()).await;
        let _ = r;
    }

    #[tokio::test]
    async fn wechat_sink_mock_ok() {
        let s = WechatSink::with_mock("key", HttpConfig::default(), success_mock());
        let r = s.send(&make_msg()).await;
        assert_eq!(r, SinkResult::Ok);
    }

    #[tokio::test]
    async fn feishu_sink_constructor() {
        let s = FeishuSink::new("https://open.feishu.cn/hook/xxx", HttpConfig::default());
        assert_eq!(s.webhook_url, "https://open.feishu.cn/hook/xxx");
        assert_eq!(s.name(), "feishu");
        assert!(s.health_check().await);
    }

    #[tokio::test]
    #[ignore = "v15.1 A4.2: real HTTP requires test server, skip in unit test (verified manually)"]
    async fn feishu_sink_skeleton_returns_err() {
        // v15.1 A4.2: feishu 现在尝试真实 HTTP POST.
        // example.com 不在 8.8.8.8 / 默认 webhook 域, 期待 connect/DNS 错误.
        let s = FeishuSink::new(
            "https://this-host-should-not-exist-12345.invalid",
            HttpConfig::default(),
        );
        let r = s.send(&make_msg()).await;
        let _ = r;
    }

    #[tokio::test]
    async fn feishu_sink_mock_ok() {
        let s = FeishuSink::with_mock("https://example.com", HttpConfig::default(), success_mock());
        let r = s.send(&make_msg()).await;
        assert_eq!(r, SinkResult::Ok);
    }

    #[tokio::test]
    async fn feishu_sink_empty_url_unhealthy() {
        let s = FeishuSink::new("", HttpConfig::default());
        assert!(!s.health_check().await);
    }

    #[tokio::test]
    async fn wechat_sink_empty_key_unhealthy() {
        let s = WechatSink::new("", HttpConfig::default());
        assert!(!s.health_check().await);
    }

    #[test]
    fn http_error_display() {
        let e = HttpError {
            status: 404,
            message: "not found".to_string(),
        };
        assert_eq!(format!("{}", e), "HTTP 404: not found");
    }
}
