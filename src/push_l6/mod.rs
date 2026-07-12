//! push_l6 — v14.2 Layer 6: Delivery (投递层)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.6 落地.
//!
//! W6.1 状态: 落地 `sink.rs` (Sink trait + ConsoleSink + SinkRouter + 13 单测).
//! W6.2 状态: 落地 `external_sinks.rs` (WechatSink + FeishuSink + HttpSink 骨架 + 14 单测).
//! 后续 W8 集成: 实跑 reqwest HTTP 调用.

pub mod external_sinks;
pub mod sink;

// 重新导出
pub use external_sinks::{FeishuSink, HttpConfig, HttpError, HttpSink, WechatSink};
pub use sink::{ConsoleSink, PushMessage, Sink, SinkResult, SinkRouter};