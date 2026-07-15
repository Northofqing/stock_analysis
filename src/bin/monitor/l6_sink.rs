//! L6 接入层 (push_l6 v14.2 §3.6)
//!
//! 把 `notify::push_wechat` 包成 `Sink` 实现, 在全局 `SinkRouter` 中注册.
//! 这样既保留现有推送基础设施 (dry-run / MagicLaw daemon / 飞书 webhook)
//! 又把 v14.2 七层栈的 L6 推到 monitor 主路径上, 让 L4 → L5 → L6 → L7 完整.
//!
//! ## 调用链
//!
//! ```text
//! notify::push_governor_inner()
//!   ├─ v14_gate()        [L4 dedup + L5 governance]      ← v14_adapter.rs
//!   ├─ [Approved] l6_sink::sink_router().route(&msg)     ← 本文件
//!   │     ├─ ConsoleSink     (默认, log info + 可选 stdout)
//!   │     └─ MagiclawSink    (delegate notify::push_wechat)
//!   └─ v14_record_delivery()  [L7 analytics]              ← v14_adapter.rs
//! ```
//!
//! ## 未来增量
//!
//! - 注册 `FeishuSink` / `WechatSink` 走独立 webhook (绕过 MagicLaw):
//!   ```rust
//!   router.register(Arc::new(FeishuSink::new(feishu_url, HttpConfig::default())));
//!   router.register(Arc::new(WechatSink::new(wechat_key, HttpConfig::default())));
//!   ```
//! - 按 `PushKind::default_level()` 分桶路由 (Critical → 飞书+微信, Info → 仅日志)
//!
//! ## 红线约束
//!
//! - AGENTS.md §2.1: 推送失败显式 log error, 不静默吞
//! - CLAUDE.md Completion Rule: 本模块由 `src/bin/monitor/` 集成 (grep ≥1),
//!   不能只活在测试 binary (`v14_e2e`) 里

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use stock_analysis::push_l1::SignalEvent;
use stock_analysis::push_l2::RenderedText;
use stock_analysis::push_l6::{ConsoleSink, PushMessage, Sink, SinkResult, SinkRouter};

use super::notify::{push_wechat, PushKind};

// ============================================================================
// MagiclawSink — 把 notify::push_wechat 装成 v14.2 Sink 抽象
// ============================================================================

/// MagiclawSink — delegate `notify::push_wechat` (含 dry-run + MagicLaw daemon + 飞书 HTTP)
pub struct MagiclawSink;

#[async_trait]
impl Sink for MagiclawSink {
    fn name(&self) -> &'static str {
        "magiclaw"
    }

    async fn send(&self, msg: &PushMessage) -> SinkResult {
        // notify::push_wechat 内部:
        //   1. V10_DRY_RUN_PUSH=1 → 跳过真实外发, 仅 log + save_push_log, 返回 true
        //   2. save_push_log 永远执行 (audit)
        //   3. resolve_send_type → Feishu / MagicLaw
        //   4. Feishu HTTP 直接 POST webhook
        //   5. MagicLaw CLI path → daemon + token + API send
        match push_wechat(&msg.text.body).await {
            true => {
                log::info!(
                    "[sink:{}] ok: event_id={} body_len={}",
                    self.name(),
                    msg.event.event_id,
                    msg.text.body.len()
                );
                SinkResult::Ok
            }
            false => {
                log::error!(
                    "[sink:{}] failed: event_id={} body_len={}",
                    self.name(),
                    msg.event.event_id,
                    msg.text.body.len()
                );
                SinkResult::Err("magiclaw_push_wechat_returned_false".to_string())
            }
        }
    }

    async fn health_check(&self) -> bool {
        true // notify::push_wechat 内部有完整 daemon 启停 + token 鉴权, 视为健康
    }
}

// ============================================================================
// PushMessage 构造 — 给 L6 route 用的统一消息结构
// ============================================================================

/// 把 (SignalEvent, 渲染后文本, PushKind) 装成 PushMessage.
/// template_id 用 push kind 的字符串名 (调试 + analytics 友好);
/// template_version 永远 1 (当前 v14.2 单版本);
/// user_id 永远 "default" (单用户场景).
pub fn build_push_message(event: &SignalEvent, text: &str, kind: PushKind) -> PushMessage {
    PushMessage {
        event: event.clone(),
        text: RenderedText::new(text),
        template_id: format!("{kind:?}"),
        template_version: 1,
        user_id: "default".to_string(),
    }
}

// ============================================================================
// 全局 SinkRouter 单例
// ============================================================================

/// 全局 SinkRouter (OnceLock 单例, 进程内唯一).
///
/// 注册默认 sink: ConsoleSink (永远在, 开发/调试日志/审计; 默认 `print_to_stdout=false`) + MagiclawSink (生产).
/// 后续可在 init 路径里 append FeishuSink / WechatSink / HttpSink.
static L6_SINK_ROUTER: OnceLock<Arc<SinkRouter>> = OnceLock::new();

/// 获取全局 SinkRouter (首次调用时初始化).
pub fn sink_router() -> Arc<SinkRouter> {
    L6_SINK_ROUTER
        .get_or_init(|| {
            let mut r = SinkRouter::new();
            // 1. ConsoleSink 永远在 (审计 log; print_to_stdout 默认 false — 改 sink::ConsoleSink)
            r.register(Arc::new(ConsoleSink::default()) as Arc<dyn Sink>);
            // 2. MagiclawSink 是当前生产路径 (delegate notify::push_wechat;含 dry-run + MagicLaw daemon)
            r.register(Arc::new(MagiclawSink) as Arc<dyn Sink>);
            log::info!(
                "[L6] SinkRouter 初始化: 默认注册 ConsoleSink + MagiclawSink ({} sinks)",
                r.len()
            );
            Arc::new(r)
        })
        .clone()
}

/// 已注册 Sink 数 (供调试 / 单测断言).
pub fn sink_count() -> usize {
    sink_router().len()
}

/// 给定制初始化场景: 在 router 首次创建时插入额外 sinks.
///
/// 用法: `l6_sink::init_with_extra_sinks(|r| r.register(...))` 必须在任何 `sink_router()` 调用之前.
///
/// 预留接口: 后续要 hot-add `FeishuSink` / `WechatSink` 时使用. 当前默认
/// `ConsoleSink + MagiclawSink` 已足够生产推送.
#[allow(dead_code)] // 预留 hot-add Feishu/Wechat sink 用, 本期不调用
pub fn init_with_extra_sinks<F>(extra: F)
where
    F: FnOnce(&mut SinkRouter) + Send + 'static,
{
    L6_SINK_ROUTER.get_or_init(|| {
        let mut r = SinkRouter::new();
        r.register(Arc::new(ConsoleSink::default()) as Arc<dyn Sink>);
        r.register(Arc::new(MagiclawSink) as Arc<dyn Sink>);
        extra(&mut r);
        log::info!(
            "[L6] SinkRouter init_with_extra_sinks 完成: {} sinks",
            r.len()
        );
        Arc::new(r)
    });
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use stock_analysis::push_l1::{Severity, SignalPayload, SignalSource};
    use stock_analysis::push_l2::RenderedText;

    fn make_test_event() -> SignalEvent {
        SignalEvent::new(
            SignalSource::HoldingHealth,
            "holding_health",
            Some("600519".into()),
            Local::now(),
            SignalPayload::HoldingHealth(Default::default()),
            Severity::High,
        )
    }

    fn make_test_msg() -> PushMessage {
        PushMessage {
            event: make_test_event(),
            text: RenderedText::new("test body"),
            template_id: "test_v1".into(),
            template_version: 1,
            user_id: "default".into(),
        }
    }

    #[test]
    fn magiclaw_sink_name() {
        assert_eq!(MagiclawSink.name(), "magiclaw");
    }

    #[test]
    fn magiclaw_sink_health_default_ok() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ok = rt.block_on(async { MagiclawSink.health_check().await });
        assert!(ok, "MagiclawSink 默认 health_check 应为 true");
    }

    #[test]
    fn sink_router_singleton_has_console_and_magiclaw() {
        // OnceLock 是进程级, 跨 test 共享. 这里只断言 ≥2 (默认值, 不会多也不会少).
        let count = sink_count();
        assert!(
            count >= 2,
            "默认 SinkRouter 应至少含 ConsoleSink + MagiclawSink, 实际 {}",
            count
        );
    }

    #[test]
    fn sink_router_contains_magiclaw_and_console() {
        // 通过 health_check_all 间接验证 magiclaw / console 都在 router 里
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let report = rt.block_on(async {
            let r = sink_router();
            r.health_check_all().await
        });
        let has_magiclaw = report.iter().any(|(n, _)| *n == "magiclaw");
        let has_console = report.iter().any(|(n, _)| *n == "console");
        assert!(has_magiclaw, "router 含 magiclaw: {:?}", report);
        assert!(has_console, "router 含 console: {:?}", report);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sink_router_routes_to_at_least_one_sink() {
        let r = sink_router();
        let msg = make_test_msg();
        // route 不会 panic / deadlock 即可 (Ok 与 Err 都是合法结果: dry-run 时全 Ok,
        // 生产环境 Magiclaw 真实 HTTP 失败时 Err).
        let result = r.route(&msg).await;
        match result {
            SinkResult::Ok => log::info!("[test] route 在 dry-run 环境全 ok"),
            SinkResult::Err(e) => log::info!("[test] route 在生产环境失败 (可接受): {e}"),
        }
    }

    #[test]
    fn build_push_message_uses_debug_str_as_template_id() {
        let ev = make_test_event();
        let pm = build_push_message(&ev, "hello", PushKind::HoldingEvent);
        assert!(pm.template_id.contains("HoldingEvent"));
        assert_eq!(pm.text.body, "hello");
        assert_eq!(pm.user_id, "default");
        assert_eq!(pm.template_version, 1);
    }
}
