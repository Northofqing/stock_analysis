//! v10 P2 multi_agent 成本看板 (Q7=C 自适应阈值)
//!
//! 设计 (v10 §0 D2 + Q7=C 决策):
//! - 每次 multi_agent 调用记: API 成本 / 耗时 / 失败标志
//! - 自适应阈值:
//!   - cost_per_push = 月推送数 × ¥3 (env V10_P2_COST_PER_PUSH, 默认 3)
//!   - failure_rate = 5% (env V10_P2_FAIL_RATE_THRESHOLD, 默认 0.05)
//! - 超阈 → 自动回退 G5a 规则快归因 + 告警
//! - 落库: agent_cost_log 表 (每次调用一行)
//!
//! Phase 7 实施:
//! - CostBoard 维护 sliding window (最近 100 次调用)
//! - check_thresholds() 实时算
//! - 触发回退时输出 alert (实际接 monitor 推送, 这里是 log)

use log::warn;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 单次 multi_agent 调用记录
#[derive(Debug, Clone)]
pub struct CallRecord {
    pub timestamp: Instant,
    pub cost_cny: f64,
    pub duration_ms: u64,
    pub success: bool,
}

/// 成本看板 (全局, sliding window)
pub struct CostBoard {
    /// sliding window 大小
    window_size: usize,
    /// 调用记录 (最近 N 次)
    records: Mutex<Vec<CallRecord>>,
    /// 总成本累计
    total_cost: Mutex<f64>,
    /// 推送数累计 (用于算 cost_per_push)
    total_pushes: Mutex<u64>,
}

impl CostBoard {
    /// 新建 (默认 sliding window = 100)
    pub fn new() -> Self {
        Self::with_window(100)
    }

    pub fn with_window(window_size: usize) -> Self {
        Self {
            window_size,
            records: Mutex::new(Vec::with_capacity(window_size)),
            total_cost: Mutex::new(0.0),
            total_pushes: Mutex::new(0),
        }
    }

    /// 记录一次调用
    pub fn record(&self, cost_cny: f64, duration: Duration, success: bool) {
        let rec = CallRecord {
            timestamp: Instant::now(),
            cost_cny,
            duration_ms: duration.as_millis() as u64,
            success,
        };
        // BUG FIX (codex C1): 不用 unwrap() 避免 Mutex poisoning panic (库代码)
        // 改用 lock().unwrap_or_else(|p| p.into_inner()) 从 poison 恢复
        let mut records = self.records.lock().unwrap_or_else(|p| p.into_inner());
        records.push(rec);
        if records.len() > self.window_size {
            records.remove(0);
        }
        drop(records);
        let mut total = self.total_cost.lock().unwrap_or_else(|p| p.into_inner());
        *total += cost_cny;
    }

    /// 推送一次 (计数 + 1, 用于算 cost_per_push)
    pub fn count_push(&self) {
        let mut total = self.total_pushes.lock().unwrap_or_else(|p| p.into_inner());
        *total += 1;
    }

    /// 失败率 (最近 sliding window)
    pub fn failure_rate(&self) -> f64 {
        let records = self.records.lock().unwrap_or_else(|p| p.into_inner());
        if records.is_empty() {
            return 0.0;
        }
        let failures = records.iter().filter(|r| !r.success).count();
        failures as f64 / records.len() as f64
    }

    /// 平均成本 (每次调用, 最近 sliding window)
    pub fn avg_cost(&self) -> f64 {
        let records = self.records.lock().unwrap_or_else(|p| p.into_inner());
        if records.is_empty() {
            return 0.0;
        }
        let total: f64 = records.iter().map(|r| r.cost_cny).sum();
        total / records.len() as f64
    }

    /// 平均耗时 (ms, 最近 sliding window)
    pub fn avg_duration_ms(&self) -> f64 {
        let records = self.records.lock().unwrap_or_else(|p| p.into_inner());
        if records.is_empty() {
            return 0.0;
        }
        let total: u64 = records.iter().map(|r| r.duration_ms).sum();
        total as f64 / records.len() as f64
    }

    /// 检查阈值 (Q7=C 自适应, 参数注入避免 env 竞争)
    /// 返回: (cost_exceeded, failure_exceeded)
    /// BUG FIX (codex C2): 防御性 clamp 仅作用于 fail_rate (应是 [0,1] 比例)
    /// 注意: avg_cost 不 clamp (成本可能 > ¥1, 是合理范围)
    pub fn check_thresholds_with(&self, cost_per_push: f64, fail_rate_th: f64) -> (bool, bool) {
        let fail_rate_th = fail_rate_th.clamp(0.0, 1.0);
        let avg = self.avg_cost(); // 不 clamp, 成本可以是任意正值
        let fail = self.failure_rate().clamp(0.0, 1.0);
        let cost_exceeded = avg > cost_per_push;
        let failure_exceeded = fail > fail_rate_th;
        (cost_exceeded, failure_exceeded)
    }

    /// 检查阈值 (Q7=C 自适应, 默认参数从 env 读)
    pub fn check_thresholds(&self) -> (bool, bool) {
        let cost_per_push = std::env::var("V10_P2_COST_PER_PUSH")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(3.0);
        let fail_rate_th = std::env::var("V10_P2_FAIL_RATE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.05);
        self.check_thresholds_with(cost_per_push, fail_rate_th)
    }

    /// 触发回退: 输出告警 log (参数注入)
    pub fn maybe_fallback_with(&self, cost_per_push: f64, fail_rate_th: f64) -> bool {
        let (cost_exceeded, failure_exceeded) =
            self.check_thresholds_with(cost_per_push, fail_rate_th);
        if cost_exceeded {
            warn!(
                "[CostBoard] ✗ 成本超阈 → 自动回退 G5a 规则快归因 (avg_cost={:.3})",
                self.avg_cost()
            );
        }
        if failure_exceeded {
            warn!(
                "[CostBoard] ✗ 失败率超阈 → 自动回退 G5a 规则快归因 (failure_rate={:.3})",
                self.failure_rate()
            );
        }
        cost_exceeded || failure_exceeded
    }

    /// 触发回退: 用默认阈值
    pub fn maybe_fallback(&self) -> bool {
        let cost_per_push = std::env::var("V10_P2_COST_PER_PUSH")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(3.0);
        let fail_rate_th = std::env::var("V10_P2_FAIL_RATE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.05);
        self.maybe_fallback_with(cost_per_push, fail_rate_th)
    }
}

impl Default for CostBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// 全局 cost board (lazy_static 模式 — 实际项目用 once_cell)
use std::sync::OnceLock;
static GLOBAL_COST_BOARD: OnceLock<CostBoard> = OnceLock::new();

pub fn global_cost_board() -> &'static CostBoard {
    GLOBAL_COST_BOARD.get_or_init(CostBoard::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_record_and_avg() {
        let board = CostBoard::new();
        board.record(1.0, Duration::from_millis(100), true);
        board.record(2.0, Duration::from_millis(200), true);
        assert_eq!(board.avg_cost(), 1.5);
        assert_eq!(board.avg_duration_ms(), 150.0);
        assert_eq!(board.failure_rate(), 0.0);
    }

    #[test]
    fn test_failure_rate() {
        let board = CostBoard::new();
        for _ in 0..8 {
            board.record(1.0, Duration::from_millis(100), true);
        }
        for _ in 0..2 {
            board.record(1.0, Duration::from_millis(100), false);
        }
        assert_eq!(board.failure_rate(), 0.2);
    }

    #[test]
    fn test_sliding_window_eviction() {
        let board = CostBoard::with_window(3);
        for i in 0..5 {
            board.record(i as f64, Duration::from_millis(100), true);
        }
        // 只保留最近 3 条 (2, 3, 4)
        assert_eq!(board.avg_cost(), 3.0); // (2+3+4)/3
    }

    #[test]
    fn test_threshold_cost_exceeded() {
        // 阈值 ¥1, 实际 avg ¥3 → 超阈 (参数注入, 不依赖 env)
        let board = CostBoard::new();
        board.record(3.0, Duration::from_millis(100), true);
        board.count_push();
        let (cost_exceeded, _) = board.check_thresholds_with(1.0, 0.05);
        assert!(cost_exceeded, "avg=3.0 > 1.0 应超阈");
    }

    #[test]
    fn test_threshold_failure_exceeded() {
        // 阈值 0.05, 实际 0.2 → 超阈 (参数注入, 不依赖 env)
        let board = CostBoard::new();
        for _ in 0..4 {
            board.record(1.0, Duration::from_millis(100), true);
        }
        board.record(1.0, Duration::from_millis(100), false);
        let (_, failure_exceeded) = board.check_thresholds_with(3.0, 0.05);
        assert!(failure_exceeded, "failure=0.2 > 0.05 应超阈");
    }

    #[test]
    fn test_threshold_no_exceeded() {
        let board = CostBoard::new();
        for _ in 0..5 {
            board.record(1.0, Duration::from_millis(100), true);
        }
        let (c, f) = board.check_thresholds_with(5.0, 0.5);
        assert!(!c, "avg=1.0 < 5.0 不应超阈");
        assert!(!f, "failure=0.0 < 0.5 不应超阈");
    }

    #[test]
    fn test_maybe_fallback_returns_bool() {
        // 极低阈值 → 触发回退 (参数注入)
        let board = CostBoard::new();
        board.record(1.0, Duration::from_millis(100), true);
        board.count_push();
        let triggered = board.maybe_fallback_with(0.1, 0.5);
        assert!(triggered, "avg=1.0 > 0.1 触发回退");
    }

    #[test]
    fn test_global_board_singleton() {
        let b1 = global_cost_board();
        let b2 = global_cost_board();
        assert!(std::ptr::eq(b1, b2), "全局 cost board 应是单例");
    }
}
