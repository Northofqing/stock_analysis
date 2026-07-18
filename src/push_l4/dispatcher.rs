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

/// v17.6 §5.1: 拼出 (kind, code, sub_kind) dedup 三元组的字符串表示, 给
/// dispatch_row / 启动 audit log 看. 实际 dedup key 由 Dispatcher::reserve/commit
/// 内部用 `(String, String, String)` 三元组, 本函数仅做字符串格式化输出.
///
/// 用法: `sub_kind_dedup_key("daily_report", Some("FactorIC"), "2026-07-16")` →
///   `"daily_report|sub_kind=FactorIC|date=2026-07-16"`.
pub fn sub_kind_dedup_key(kind: &str, sub_kind: Option<&str>, date: &str) -> String {
    match sub_kind {
        Some(s) => format!("{}|sub_kind={}|date={}", kind, s, date),
        None => kind.to_string(),
    }
}

/// Dispatcher — v14.2 L4 仲裁层 (单实例, 全局共享)
pub struct Dispatcher {
    /// dedup 表: (kind, code, sub_kind) → (上次放行时间, 该键冷却窗口).
    /// sub_kind="" (空字符串) 表示无 sub_kind (向后兼容原 `(kind, code)` 键空间).
    /// v17.6 §5.1 引入第三元组让 DailyReport 下 3 个 sub_kind 独立 dedup 窗口.
    dedup_table: DashMap<(String, String, String), (Instant, Duration)>,
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
            dispatched: self
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed)
                .into(),
            deduped: self
                .deduped
                .load(std::sync::atomic::Ordering::Relaxed)
                .into(),
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

/// v15.1 A3: 新的 reserve/commit/rollback 三阶段 dedup API
///
/// 解决 c1a9cfd 引入的 dedup-before-delivery bug:
/// 旧 dispatch() 在 governance 闸通过后立刻插入 dedup entry,
/// 但若 push_wechat 失败 (sink 错误/网络), 该 entry 仍占满 cooldown 窗口
/// (DailyReport 86400s = 24h 黑屏).
///
/// 新契约:
/// - reserve() 只检查 dedup 不插入, 返回 Reserved | Deduped
/// - 推送成功后 commit() 真正插入 entry
/// - 推送失败 rollback() 是 no-op (reserve 没占位)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveOutcome {
    /// 可以继续推送 (新事件或冷却窗口已过)
    Reserved,
    /// 在冷却窗口内, 不应推送
    Deduped,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {
            dedup_table: DashMap::new(),
            stats: DispatcherStats::default(),
        }
    }

    /// v15.1 A3: 检查 dedup 但不插入 (新契约). 调用方在 push 成功后调 commit().
    ///
    /// `sub_kind`: v17.6 §5.1 — 当 DailyReport 子段 (FactorIC/SectorTier/CapitalVerify)
    /// 推送时传入 Some("FactorIC") 让 dedup key 加上第三元组, 实现 per-sub_kind 隔离.
    /// None 时 sub_kind="" (向后兼容).
    pub fn reserve(
        &self,
        event: &SignalEvent,
        cooldown: Option<Duration>,
        sub_kind: Option<&str>,
    ) -> ReserveOutcome {
        let Some(_window) = cooldown.filter(|w| !w.is_zero()) else {
            return ReserveOutcome::Reserved;
        };
        let key = (
            event.kind.clone(),
            event.code.clone().unwrap_or_default(),
            sub_kind.unwrap_or("").to_string(),
        );
        match self.dedup_table.get(&key) {
            Some(entry) => {
                let (prev, w) = *entry.value();
                if prev.elapsed() < w {
                    self.stats
                        .deduped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    ReserveOutcome::Deduped
                } else {
                    ReserveOutcome::Reserved
                }
            }
            None => ReserveOutcome::Reserved,
        }
    }

    /// v15.1 A3: 实际插入 dedup entry (push 成功后调用).
    /// sub_kind 语义同 reserve().
    pub fn commit(&self, event: &SignalEvent, cooldown: Option<Duration>, sub_kind: Option<&str>) {
        let Some(window) = cooldown.filter(|w| !w.is_zero()) else {
            return;
        };
        let key = (
            event.kind.clone(),
            event.code.clone().unwrap_or_default(),
            sub_kind.unwrap_or("").to_string(),
        );
        let now = std::time::Instant::now();
        self.dedup_table.insert(key, (now, window));
        self.stats
            .dispatched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.dedup_table.len() >= DEDUP_TABLE_SOFT_CAP {
            self.dedup_table.retain(|_, (t, w)| t.elapsed() < *w);
        }
    }

    /// v15.1 A3: 失败回滚 — 因为 reserve() 不占位, 此函数为 no-op
    /// (保留 API 对称性, 让 push_governor_inner 显式调用)
    pub fn rollback(
        &self,
        _event: &SignalEvent,
        _cooldown: Option<Duration>,
        _sub_kind: Option<&str>,
    ) {
        // no-op: reserve 不留痕, 无需回滚
    }

    /// dispatch 入口 — W4.2: 按 (kind, code, sub_kind) + 冷却窗口做速率限制
    ///
    /// b013 P2-13: 改 &self (DashMap 内部 shard lock), v14_adapter 的外部
    /// Mutex<Dispatcher> 可放宽 → 推送热路径减少序列化.
    /// b013 P2-14: entry API 一次操作, 消除 len()/retain()/insert() TOCTOU.
    ///
    /// `cooldown = None` → 不做冷却直接放行 (调用方声明该 kind 无冷却或冷却归其他层)
    ///
    /// **v15.1 A3 DEPRECATED**: 用 reserve/commit 两阶段代替, 避免 delivery 失败
    /// 后 cooldown 仍占用窗口 (DailyReport 86400s 黑屏).
    #[deprecated(
        note = "v15.1 A3: use reserve() + commit() instead to allow rollback on delivery failure"
    )]
    pub fn dispatch(
        &self,
        event: &SignalEvent,
        cooldown: Option<Duration>,
        sub_kind: Option<&str>,
    ) -> DispatchOutcome {
        let Some(window) = cooldown.filter(|w| !w.is_zero()) else {
            self.stats
                .dispatched
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return DispatchOutcome::Pushed;
        };

        let key = (
            event.kind.clone(),
            event.code.clone().unwrap_or_default(),
            sub_kind.unwrap_or("").to_string(),
        );
        // b013 N-7 (P0): entry().or_insert() 让首次插入立即触发 elapsed=0 < window,
        // 导致首次调用被自己的冷却期拦截. 正确: 区分"刚插入"与"已存在".
        let now = Instant::now();
        use dashmap::mapref::entry::Entry;
        match self.dedup_table.entry(key) {
            Entry::Vacant(v) => {
                v.insert((now, window));
                self.stats
                    .dispatched
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return DispatchOutcome::Pushed;
            }
            Entry::Occupied(mut o) => {
                let elapsed = o.get().0.elapsed();
                if elapsed < o.get().1 {
                    self.stats
                        .deduped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let k = o.key().clone();
                    let detail = format!(
                        "cooldown {}s/{}s kind={} code={}",
                        elapsed.as_secs(),
                        o.get().1.as_secs(),
                        k.0,
                        k.1
                    );
                    return DispatchOutcome::Deduped(detail);
                }
                // 冷却已过 → 刷新时间戳
                o.get_mut().0 = now;
            }
        }
        self.stats
            .dispatched
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
#[allow(
    deprecated,
    reason = "this module verifies the retained legacy dispatch compatibility wrapper"
)]
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
        assert_eq!(
            d.stats
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn first_dispatch_succeeds() {
        let d = Dispatcher::new();
        let e = make_event("TEST_CODE_600519");
        assert_eq!(d.dispatch(&e, WIN, None), DispatchOutcome::Pushed);
        assert_eq!(
            d.stats
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn same_kind_code_deduped_within_window() {
        let d = Dispatcher::new();
        let _ = d.dispatch(&make_event("TEST_CODE_600519"), WIN, None);
        // 同 (kind, code), 即使 event_id/时间不同也要被冷却挡住 (b011 P0-2)
        let outcome = d.dispatch(&make_event("TEST_CODE_600519"), WIN, None);
        assert!(matches!(outcome, DispatchOutcome::Deduped(_)));
        assert_eq!(
            d.stats
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn different_codes_not_deduped() {
        let d = Dispatcher::new();
        let _ = d.dispatch(&make_event("TEST_CODE_600519"), WIN, None);
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_000001"), WIN, None),
            DispatchOutcome::Pushed
        );
        assert_eq!(
            d.stats
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        assert_eq!(
            d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn none_cooldown_never_dedups() {
        let d = Dispatcher::new();
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_600519"), None, None),
            DispatchOutcome::Pushed
        );
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_600519"), None, None),
            DispatchOutcome::Pushed
        );
        assert_eq!(
            d.stats
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        assert_eq!(d.dedup_size(), 0, "无冷却不登记 dedup 表");
    }

    #[test]
    fn zero_cooldown_treated_as_none() {
        let d = Dispatcher::new();
        let zero = Some(Duration::ZERO);
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_600519"), zero, None),
            DispatchOutcome::Pushed
        );
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_600519"), zero, None),
            DispatchOutcome::Pushed
        );
    }

    #[test]
    fn window_expiry_allows_re_dispatch() {
        let d = Dispatcher::new();
        let win = Some(Duration::from_millis(50));
        let _ = d.dispatch(&make_event("TEST_CODE_600519"), win, None);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_600519"), win, None),
            DispatchOutcome::Pushed
        );
    }

    #[test]
    fn clear_dedup_resets_table() {
        let d = Dispatcher::new();
        let _ = d.dispatch(&make_event("TEST_CODE_600519"), WIN, None);
        assert_eq!(d.dedup_size(), 1);
        d.clear_dedup();
        assert_eq!(d.dedup_size(), 0);
        assert_eq!(
            d.dispatch(&make_event("TEST_CODE_600519"), WIN, None),
            DispatchOutcome::Pushed
        );
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
                let _ = d.dispatch(&make_event(&code), WIN, None);
            });
            handles.push(handle);
        }

        for h in handles {
            h.join().unwrap();
        }

        let d = dispatcher.lock().unwrap();
        assert_eq!(
            d.stats
                .dispatched
                .load(std::sync::atomic::Ordering::Relaxed),
            10
        );
        assert_eq!(
            d.stats.deduped.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    // ============= v15.1 A3: reserve/commit/rollback 三阶段测试 =============

    #[test]
    fn reserve_first_call_succeeds() {
        let d = Dispatcher::new();
        let e = make_event("TEST_CODE_600519");
        // reserve 不插入
        assert_eq!(d.reserve(&e, WIN, None), ReserveOutcome::Reserved);
        assert_eq!(d.dedup_size(), 0, "reserve() 不应插入");
    }

    #[test]
    fn reserve_second_call_within_window_deduped() {
        let d = Dispatcher::new();
        let e = make_event("TEST_CODE_600519");
        let _ = d.reserve(&e, WIN, None);
        // reserve 没插, 再 reserve 还是 Reserved (这是 reserve 的特性)
        // 但 commit() 之后再 reserve 才 Deduped
        d.commit(&e, WIN, None);
        assert_eq!(d.reserve(&e, WIN, None), ReserveOutcome::Deduped);
    }

    #[test]
    fn delivery_failure_rolls_back_dedup() {
        let d = Dispatcher::new();
        let e = make_event("TEST_CODE_600519");

        // 1. reserve (不插入)
        assert_eq!(d.reserve(&e, WIN, None), ReserveOutcome::Reserved);
        assert_eq!(d.dedup_size(), 0);

        // 2. 模拟推送失败 → rollback (no-op, reserve 没占位)
        d.rollback(&e, WIN, None);
        assert_eq!(d.dedup_size(), 0, "rollback 后仍应没有 dedup entry");

        // 3. 立即 retry 应该 Reserved (不 Deduped — 关键修复)
        assert_eq!(d.reserve(&e, WIN, None), ReserveOutcome::Reserved);

        // 4. 模拟推送成功 → commit (真正插入)
        d.commit(&e, WIN, None);
        assert_eq!(d.dedup_size(), 1);

        // 5. 第三次再 reserve 应该 Deduped (因为 commit 过了)
        assert_eq!(d.reserve(&e, WIN, None), ReserveOutcome::Deduped);
    }

    #[test]
    fn commit_inserts_after_window_expiry() {
        let d = Dispatcher::new();
        let e = make_event("TEST_CODE_600519");
        let win = Some(Duration::from_millis(50));

        // 第一次 commit, 立即 reserve 应该是 Deduped
        d.commit(&e, win, None);
        assert_eq!(d.reserve(&e, win, None), ReserveOutcome::Deduped);

        // 等窗口过期
        std::thread::sleep(Duration::from_millis(80));

        // 窗口过期后 reserve 应是 Reserved
        assert_eq!(d.reserve(&e, win, None), ReserveOutcome::Reserved);
    }

    // ============== v17.6 §5.1: 3 sub_kind dedup key 独立 (修 FINDING #1) ==============

    #[test]
    fn sub_kind_three_paths_dedup_independent() {
        let d = Dispatcher::new();
        let e1 = make_event("TEST_CODE_600519");
        let e2 = make_event("TEST_CODE_600519");
        let e3 = make_event("TEST_CODE_600519");
        // 第一次 reserve: 都 Reserved (无 prior entry)
        assert_eq!(
            d.reserve(&e1, WIN, Some("FactorIC")),
            ReserveOutcome::Reserved
        );
        assert_eq!(
            d.reserve(&e2, WIN, Some("SectorTier")),
            ReserveOutcome::Reserved
        );
        assert_eq!(
            d.reserve(&e3, WIN, Some("CapitalVerify")),
            ReserveOutcome::Reserved
        );
        // commit 各自
        d.commit(&e1, WIN, Some("FactorIC"));
        d.commit(&e2, WIN, Some("SectorTier"));
        d.commit(&e3, WIN, Some("CapitalVerify"));
        // 第二次 reserve: 各自应被自己 sub_kind dedup (Deduped), 互不影响
        assert_eq!(
            d.reserve(&e1, WIN, Some("FactorIC")),
            ReserveOutcome::Deduped
        );
        assert_eq!(
            d.reserve(&e2, WIN, Some("SectorTier")),
            ReserveOutcome::Deduped
        );
        assert_eq!(
            d.reserve(&e3, WIN, Some("CapitalVerify")),
            ReserveOutcome::Deduped
        );
    }
}
