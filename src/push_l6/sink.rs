//! push_l6/sink.rs — L6 Delivery (v14.2 §3.6)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.6 落地.
//!
//! W6.1 范围:
//!   - Sink trait (异步推送通道抽象)
//!   - ConsoleSink (默认实现, 输出到 stdout / 日志)
//!   - SinkRouter (多 Sink 路由 + 失败隔离)
//!   - PushMessage (Sink 接收的统一消息结构)
//!   - 12+ 单测
//!
//! 后续 W6.2 会加: WechatSink / FeishuSink (调外部 API).
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2: Sink 不静默填补, 推送失败要显式 log error
//!   - b-009 R-4: dispatch 必须走 dispatcher (Sink 仅为 destination, 不参与调度)

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::push_l1::SignalEvent;
use crate::push_l2::RenderedText;

/// 推送消息 (Sink 接收的统一结构)
///
/// L3 Render → PushMessage → Sink.send()
#[derive(Debug, Clone)]
pub struct PushMessage {
    /// 关联的 SignalEvent (用于 analytics / 调试)
    pub event: SignalEvent,
    /// 渲染后的文本
    pub text: RenderedText,
    /// 模板 ID
    pub template_id: String,
    /// 模板版本
    pub template_version: u32,
    /// 目标用户 (单用户场景: "default"; 多用户: user_id)
    pub user_id: String,
}

/// Sink 推送结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkResult {
    /// 推送成功
    Ok,
    /// 推送失败 + 原因
    Err(String),
}

/// Sink trait — 异步推送通道
///
/// v14.2 §3.6: Sink 是 trait, 可独立开发测试
/// 实现方需保证 Send + Sync (多线程 dispatcher 可能并发调用)
#[async_trait]
pub trait Sink: Send + Sync {
    /// Sink 名称 (用于日志 + 路由)
    fn name(&self) -> &'static str;

    /// 推送一条消息
    async fn send(&self, msg: &PushMessage) -> SinkResult;

    /// 健康检查 (可选实现)
    async fn health_check(&self) -> bool {
        true  // 默认健康
    }
}

// ============================================================================
// ConsoleSink — 默认实现
// ============================================================================

/// ConsoleSink — 输出到 stdout + 日志 (开发/调试用)
pub struct ConsoleSink {
    /// 是否同时打印到 stdout (生产环境通常 false)
    pub print_to_stdout: bool,
}

impl Default for ConsoleSink {
    fn default() -> Self {
        Self { print_to_stdout: false }
    }
}

impl ConsoleSink {
    pub fn new(print_to_stdout: bool) -> Self {
        Self { print_to_stdout }
    }
}

#[async_trait]
impl Sink for ConsoleSink {
    fn name(&self) -> &'static str {
        "console"
    }

    async fn send(&self, msg: &PushMessage) -> SinkResult {
        log::info!(
            "[sink:console] template={} v{} user={} event_id={} body_len={}",
            msg.template_id, msg.template_version, msg.user_id, msg.event.event_id,
            msg.text.body.len()
        );
        if self.print_to_stdout {
            println!("{}", msg.text.body);
        }
        SinkResult::Ok
    }

    async fn health_check(&self) -> bool {
        true  // stdout 始终可用
    }
}

// ============================================================================
// SinkRouter — 多 Sink 路由
// ============================================================================

/// SinkRouter — 维护多个 Sink, 按 SinkResult 决定重试/失败
pub struct SinkRouter {
    sinks: Vec<Arc<dyn Sink>>,
    /// 单次推送重试次数
    max_retries: u32,
    /// 重试间隔
    retry_interval: Duration,
}

impl SinkRouter {
    pub fn new() -> Self {
        Self {
            sinks: Vec::new(),
            max_retries: 0,
            retry_interval: Duration::from_millis(100),
        }
    }

    /// 注册一个 Sink
    pub fn register(&mut self, sink: Arc<dyn Sink>) {
        self.sinks.push(sink);
    }

    /// 注册所有内置 Sink (Console 永远在)
    pub fn register_defaults(&mut self) {
        self.register(Arc::new(ConsoleSink::default()));
    }

    /// 推送到所有 Sink (失败隔离: 一个 sink 失败不影响其他)
    ///
    /// 返回整体结果: 全部成功 → Ok, 任一失败 → Err + 失败原因列表
    pub async fn route(&self, msg: &PushMessage) -> SinkResult {
        if self.sinks.is_empty() {
            return SinkResult::Err("no_sinks_registered".to_string());
        }

        let mut errors = Vec::new();
        for sink in &self.sinks {
            let mut attempt = 0;
            loop {
                match sink.send(msg).await {
                    SinkResult::Ok => break,
                    SinkResult::Err(e) => {
                        if attempt < self.max_retries {
                            attempt += 1;
                            log::warn!(
                                "[sink:{}] send 失败 (尝试 {}/{}): {}, 重试中",
                                sink.name(), attempt, self.max_retries, e
                            );
                            tokio::time::sleep(self.retry_interval).await;
                        } else {
                            log::error!(
                                "[sink:{}] send 失败 (放弃): {}",
                                sink.name(), e
                            );
                            errors.push(format!("{}: {}", sink.name(), e));
                            break;
                        }
                    }
                }
            }
        }

        if errors.is_empty() {
            SinkResult::Ok
        } else {
            SinkResult::Err(errors.join("; "))
        }
    }

    /// 健康检查所有 Sink
    pub async fn health_check_all(&self) -> Vec<(&'static str, bool)> {
        let mut results = Vec::new();
        for sink in &self.sinks {
            let ok = sink.health_check().await;
            results.push((sink.name(), ok));
        }
        results
    }

    /// 已注册 Sink 数
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// 是否无 Sink
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

impl Default for SinkRouter {
    fn default() -> Self {
        let mut r = Self::new();
        r.register_defaults();
        r
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_l1::{LimitUpPayload, Severity, SignalPayload};
    use crate::push_l2::{DataMode, TemplateMetadata};
    use chrono::Local;

    fn make_msg() -> PushMessage {
        let event = SignalEvent::new(
            crate::push_l1::SignalSource::LimitUp,
            "limit_up",
            Some("600519".to_string()),
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

    /// 测试用 Sink (可注入失败)
    struct MockSink {
        name: &'static str,
        failures_remaining: std::sync::Mutex<u32>,
        healthy: bool,
    }

    impl MockSink {
        fn new(name: &'static str, failures: u32, healthy: bool) -> Self {
            Self {
                name,
                failures_remaining: std::sync::Mutex::new(failures),
                healthy,
            }
        }
    }

    #[async_trait]
    impl Sink for MockSink {
        fn name(&self) -> &'static str { self.name }

        async fn send(&self, _msg: &PushMessage) -> SinkResult {
            let mut f = self.failures_remaining.lock().unwrap();
            if *f > 0 {
                *f -= 1;
                SinkResult::Err(format!("mock failure, remaining={}", *f))
            } else {
                SinkResult::Ok
            }
        }

        async fn health_check(&self) -> bool {
            self.healthy
        }
    }

    #[test]
    fn console_sink_default_does_not_print_to_stdout() {
        let s = ConsoleSink::default();
        assert!(!s.print_to_stdout);
        assert_eq!(s.name(), "console");
    }

    #[test]
    fn console_sink_with_stdout() {
        let s = ConsoleSink::new(true);
        assert!(s.print_to_stdout);
    }

    #[tokio::test]
    async fn console_sink_send_ok() {
        let s = ConsoleSink::default();
        let r = s.send(&make_msg()).await;
        assert_eq!(r, SinkResult::Ok);
    }

    #[tokio::test]
    async fn console_sink_health_check() {
        let s = ConsoleSink::default();
        assert!(s.health_check().await);
    }

    #[test]
    fn sink_router_default_has_console() {
        let r = SinkRouter::default();
        assert!(!r.is_empty());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn sink_router_register_multiple() {
        let mut r = SinkRouter::new();
        r.register(Arc::new(ConsoleSink::default()));
        r.register(Arc::new(MockSink::new("mock1", 0, true)));
        r.register(Arc::new(MockSink::new("mock2", 0, true)));
        assert_eq!(r.len(), 3);
    }

    #[tokio::test]
    async fn sink_router_route_to_all() {
        let mut r = SinkRouter::new();
        r.register(Arc::new(ConsoleSink::default()));
        r.register(Arc::new(MockSink::new("mock1", 0, true)));
        let result = r.route(&make_msg()).await;
        assert_eq!(result, SinkResult::Ok);
    }

    #[tokio::test]
    async fn sink_router_failure_isolation() {
        // 一个 sink 失败不应影响另一个
        let mut r = SinkRouter::new();
        r.register(Arc::new(MockSink::new("failing", 100, false)));  // 永远失败
        r.register(Arc::new(MockSink::new("ok", 0, true)));
        let result = r.route(&make_msg()).await;
        assert!(matches!(result, SinkResult::Err(_)));
        // 失败信息应包含 "failing"
        if let SinkResult::Err(e) = result {
            assert!(e.contains("failing"));
        }
    }

    #[tokio::test]
    async fn sink_router_retry_then_success() {
        let mut r = SinkRouter::new();
        r.max_retries = 2;
        r.register(Arc::new(MockSink::new("flaky", 2, true)));  // 前 2 次失败, 第 3 次成功
        let result = r.route(&make_msg()).await;
        assert_eq!(result, SinkResult::Ok);
    }

    #[tokio::test]
    async fn sink_router_retry_exhausted() {
        let mut r = SinkRouter::new();
        r.max_retries = 1;
        r.register(Arc::new(MockSink::new("always_fails", 100, false)));
        let result = r.route(&make_msg()).await;
        assert!(matches!(result, SinkResult::Err(_)));
    }

    #[tokio::test]
    async fn sink_router_empty_returns_error() {
        let r = SinkRouter::new();
        let result = r.route(&make_msg()).await;
        assert!(matches!(result, SinkResult::Err(_)));
    }

    #[tokio::test]
    async fn sink_router_health_check_all() {
        let mut r = SinkRouter::new();
        r.register(Arc::new(MockSink::new("healthy", 0, true)));
        r.register(Arc::new(MockSink::new("unhealthy", 0, false)));
        let results = r.health_check_all().await;
        assert_eq!(results.len(), 2);
        let healthy = results.iter().find(|(n, _)| *n == "healthy").unwrap();
        let unhealthy = results.iter().find(|(n, _)| *n == "unhealthy").unwrap();
        assert!(healthy.1);
        assert!(!unhealthy.1);
    }

    #[test]
    fn push_message_constructor() {
        let m = make_msg();
        assert_eq!(m.template_id, "limit_up_v1");
        assert_eq!(m.template_version, 1);
        assert_eq!(m.user_id, "default");
        assert_eq!(m.text.body, "test body");
    }
}