//! Signal Context — 保留 market_event 子模块 (event_extractor 真在用).
//!
//! 修复 Top10#1 (2026-06-29 audit): 原 mod.rs 顶层定义的 Signal / SignalSource /
//! SignalDirection / SignalSet 结构体经 grep 验证 0 处外部使用 (真死代码).
//! audit §1.5 + 架构评审 F1 说"signal/ 上下文是死代码" 部分正确 —
//! market_event.rs 真活的 (event_extractor/{core,rule_filter,classifier}.rs 7 处用),
//! 顶层 Signal/SignalSource 等 4 个类型**真死的**, 已删除.
//!
//! 后续清理: 如果 event_extractor 完全迁出 src/opportunity/ (v10 重构),
//! market_event 可移到 src/event/mod.rs 顶层, 不再藏在 "signal/" 名下.

pub mod market_event;
pub mod push_recorder; // v16.3 Commit 2: pushed_stocks 入池
