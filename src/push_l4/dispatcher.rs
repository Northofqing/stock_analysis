//! push_l4/dispatcher.rs — L4 Dispatcher (v14.2 §3.4, W4.2 dedup 闭环)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.4 + b-009 R-4 落地.
//! (master plan 周编号对应: Phase 2 W11-W12 骨架 → b011 P0-2 本次补完 dedup)
//!
//! dedup 语义 (b011 P0-2 修复后):
//!   - 键 = (event.kind, event.code): 业务键, 不再用 event_id (event_id 含 1~10s
//!     时间桶, 同一业务事件跨桶会得到不同 id, 按 id 去重形同虚设 — b011 实证)
//!   - 窗口 = 调用方传入 (来自 PushKind::cooldown_secs() 的 §14.3 治理表),
//!     None = 无冷却 (紧急/状态变更类, 或冷却由 sm 状态机/模板层 memo 专管)
//!   - 语义是"速率限制"(同键窗口内只放行一次), 不是内容去重
//!
//! 红线约束:
//!   - b-009 R-4: v13 入口强制走 v14.2 dispatcher
//!   - AGENTS.md §2.1 / §2.2: dedup 命中必须显式返回 Deduped + 原因, 不静默吞

use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::push_l1::SignalEvent;

/// dedup 表容量阈值: 超过则先清理过期项 (防常驻进程无界增长)
const DEDUP_TABLE_SOFT_CAP: usize = 4096;

/// Dispatcher — v14.2 L4 仲裁层 (单实例, 全局共享)
pub struct Dispatcher {
    /// dedup 表: (kind, code) → (上次放行时间, 该键冷却窗口)
    dedup_table: DashMap<(String, String), (Instant, Duration)>,
    /// 派发计数 (调试/analytics)
    pub stats: DispatcherStats,
}

/// Dispatcher 统计 (b013 P2-13: 改 AtomicU64, 让 dispatch(&self, ...) 能 &self 修改)
#[derive(Debug, Default)]
pub struct DispatcherStats {
    pub dispatched: std::sync::atomic::AtomicU64,
    pub deduped: std::sync::atomic::AtomicU64,
}

impl Clone for DispatcherStats {
    fn clone(&self) -> Self {
        Self {
            dispatched: self.dispatched.load(std::sync::atomic::Ordering::Relaxed).into(),
            deduped: self.deduped.load(std::sync::atomic::Ordering::Relaxed).into(),
        }
    }
}

/// Dispatch 结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// 放行 (允许进入 governance → 投递)
    Pushed,
    /// 被 dedup 拦下 (同 (kind, code) 在冷却窗口内重复), 携带原因描述
    Deduped(String),
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            dedup_table: DashMap::new(),
            stats: DispatcherStats::default(),
        }
    }

    /// dispatch 入口 — W4.2: 按 (kind, code) + 冷却窗口做速率限制
    ///
    /// b013 P2-13: 改 &self (DashMap 内部 shard lock), v14_adapter 的外部
    /// Mutex<Dispatcher> 可放宽 → 推送热路径减少序列化.
    /// b013 P2-14: entry API 一次操作, 消除 len()/retain()/insert() TOCTOU.
    ///
    /// `cooldown = None` → 不做冷却直接放行 (调用方声明该 kind 无冷却或冷却归其他层)
    pub fn dispatch(&self, event: &SignalEvent, cooldown: Option<Duration>) -> DispatchOutcome {
        let Some(window) = cooldown.filter(|w| !w.is_zero()) else {
            self.stats.dispatched.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return DispatchOutcome::Pushed;
        };

        let key = (
            event.kind.clone(),
            event.code.clone().unwrap_or_default(),
        );
        // b013 N-7 (P0): entry().or_insert() 让首次插入立即触发 elapsed=0 < window,
        // 导致首次调用被自己的冷却期拦截. 正确: 区分"刚插入"与"已存在".
        let now = Instant::now();
        use dashmap::mapref::entry::Entry;
        match self.dedup_table.entry(key) {
            Entry::Vacant(v) => {
                v.insert((now, window));
                self.stats.dispatched.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return DispatchOutcome::Pushed;
            }
            Entry::Occupied(mut o) => {
                let elapsed = o.get().0.elapsed();
                if elapsed < o.get().1 {
                    self.stats.deduped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let k = o.key().clone();
                    let detail = format!(
                        "cooldown {}s/{}s kind={} code={}",
                        elapsed.as_secs(),
                        o.get().1.as_secs(),
                        k.0, k.1
                    );
                    return DispatchOutcome::Deduped(detail);
                }
                // 冷却已过 → 刷新时间戳
                o.get_mut().0 = now;
            }
        }
        self.stats.dispatched.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // 软上限: 仅当接近上限时才 retain (避免每次都遍历全表)
        if self.dedup_table.len() >= DEDUP_TABLE_SOFT_CAP {
            self.dedup_table.retain(|_, (t, w)| t.elapsed() < *w);
        }
        DispatchOutcome::Pushed
    }

    /// 清理 dedup 表 (测试 / 手动重置)
    pub fn clear_dedup(&self) {
        self.dedup_table.clear();
    }

    /// 当前 dedup 表大小 (调试)
    pub fn dedup_size(&self) -> usize {
        self.dedup_table.len()
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_l1::{LimitUpPayload, Severity, SignalPayload, SignalSource};
    use chrono::Local;

    fn make_event(code: &str) -> SignalEvent {
        SignalEvent::new(
            SignalSource::LimitUp,
            "limit_up",
            Some(code.to_string()),
            Local::now(),
            SignalPayload::LimitUp(LimitUpPayload {
                code: Some(code.to_string()),
                ..Default::default()
            }),
            Severity::High,
        )
    }

    const WIN: Option<Duration> = Some(Duration::from_secs(60));

    #[test]
    fn new_dispatcher_has_empty_dedup() {
        let d = Dispatcher::new();
        assert_eq!(d.dedup_size(), 0);
        assert_eq!(d.stats.dispatched.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn first_dispatch_succeeds() {
        let d = Dispatcher::new();
        let e = make_event("600519");
        assert_eq!(d.dispatch(&e, WIN), DispatchOutcome::Pushed);
        assert_eq!(d.stats.dispatched.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn same_kind_code_deduped_within_window() {
        let d = Dispatcher::new();
        let _ = d.dispatch(&make_event("600519"), WIN);
        // 同 (kind, code), 即使 event_id/时间不同也要被冷却挡住 (b011 P0-2)
        let outcome = d.dispatch(&make_event("600519"), WIN);
        assert!(matches!(outcome, DispatchOutcome::Deduped(_)));
        assert_eq!(d.stats.dispatched.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn different_codes_not_deduped() {
        let d = Dispatcher::new();
        let _ = d.dispatch(&make_event("600519"), WIN);
        assert_eq!(d.dispatch(&make_event("000001"), WIN), DispatchOutcome::Pushed);
        assert_eq!(d.stats.dispatched.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn none_cooldown_never_dedups() {
        let d = Dispatcher::new();
        assert_eq!(d.dispatch(&make_event("600519"), None), DispatchOutcome::Pushed);
        assert_eq!(d.dispatch(&make_event("600519"), None), DispatchOutcome::Pushed);
        assert_eq!(d.stats.dispatched.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(d.dedup_size(), 0, "无冷却不登记 dedup 表");
    }

    #[test]
    fn zero_cooldown_treated_as_none() {
        let d = Dispatcher::new();
        let zero = Some(Duration::ZERO);
        assert_eq!(d.dispatch(&make_event("600519"), zero), DispatchOutcome::Pushed);
        assert_eq!(d.dispatch(&make_event("600519"), zero), DispatchOutcome::Pushed);
    }

    #[test]
    fn window_expiry_allows_re_dispatch() {
        let d = Dispatcher::new();
        let win = Some(Duration::from_millis(50));
        let _ = d.dispatch(&make_event("600519"), win);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(d.dispatch(&make_event("600519"), win), DispatchOutcome::Pushed);
    }

    #[test]
    fn clear_dedup_resets_table() {
        let d = Dispatcher::new();
        let _ = d.dispatch(&make_event("600519"), WIN);
        assert_eq!(d.dedup_size(), 1);
        d.clear_dedup();
        assert_eq!(d.dedup_size(), 0);
        assert_eq!(d.dispatch(&make_event("600519"), WIN), DispatchOutcome::Pushed);
    }

    #[test]
    fn dispatcher_is_thread_safe() {
        use std::sync::{Arc, Mutex};

        let dispatcher = Arc::new(Mutex::new(Dispatcher::new()));
        let mut handles = vec![];

        for i in 0..10 {
            let d = Arc::clone(&dispatcher);
            let handle = std::thread::spawn(move || {
                let code = format!("60000{}", i);
                let d = d.lock().unwrap();
                let _ = d.dispatch(&make_event(&code), WIN);
            });
            handles.push(handle);
        }

        for h in handles {
            h.join().unwrap();
        }

        let d = dispatcher.lock().unwrap();
        assert_eq!(d.stats.dispatched.load(std::sync::atomic::Ordering::Relaxed), 10);
        assert_eq!(d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}
