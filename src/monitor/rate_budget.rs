//! 请求预算与退避熔断。
//!
//! 功能：
//! - RateBudget: 滑动窗口请求计数，防止超频
//! - BackoffStrategy: 多级退避（切换Host/降级频率/熔断）
//! - CircuitBreaker: 熔断器（连续失败N次→断开→冷却后恢复）
//!
//! 设计原则：
//! - 多源数据优先，单源失败自动降级补偿
//! - 熔断后自动尝试恢复，避免永久静默
//! - 所有状态可观测

use log::{info, warn};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

// ============================================================================
// 滑动窗口请求预算
// ============================================================================

/// 滑动窗口请求预算器。
///
/// 线程安全：使用 AtomicU32 进行计数，Mutex 保护窗口重置。
pub struct RateBudget {
    /// 窗口大小（秒）
    window_secs: u64,
    /// 窗口内最大请求数
    max_requests: u32,
    /// 当前窗口起始时间（秒级精度即可）
    window_start: Mutex<Instant>,
    /// 当前窗口内的请求计数
    count: AtomicU32,
}

impl RateBudget {
    pub fn new(max_requests_per_minute: u32) -> Self {
        Self {
            window_secs: 60,
            max_requests: max_requests_per_minute,
            window_start: Mutex::new(Instant::now()),
            count: AtomicU32::new(0),
        }
    }

    pub fn with_window(max_requests: u32, window_secs: u64) -> Self {
        Self {
            window_secs,
            max_requests,
            window_start: Mutex::new(Instant::now()),
            count: AtomicU32::new(0),
        }
    }

    /// 尝试消耗一次请求配额。返回 true 表示允许，false 表示超限。
    pub fn try_acquire(&self) -> bool {
        let now = Instant::now();
        // 检查是否需要重置窗口
        if let Ok(mut start) = self.window_start.lock() {
            if now.duration_since(*start).as_secs() >= self.window_secs {
                *start = now;
                self.count.store(0, Ordering::Relaxed);
            }
            // 旧窗口已过，重置
        }

        let current = self.count.load(Ordering::Relaxed);
        if current >= self.max_requests {
            return false;
        }

        self.count.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// 当前窗口已用配额数
    pub fn used(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }

    /// 当前窗口剩余配额
    pub fn remaining(&self) -> u32 {
        self.max_requests.saturating_sub(self.used())
    }

    /// 窗口上限
    pub fn limit(&self) -> u32 {
        self.max_requests
    }
}

// ============================================================================
// 退避策略
// ============================================================================

/// 请求失败后的退避级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BackoffLevel {
    /// 正常，无退避
    Normal,
    /// 切换备用 Host
    SwitchHost,
    /// 增加轮询间隔（原间隔 × 2）
    SlowDown,
    /// 降级为低频轮询（如 5 分钟一次）
    Degrade,
    /// 熔断（暂停 N 秒）
    CircuitBreak(u64),
}

impl BackoffLevel {
    pub fn label(&self) -> &'static str {
        match self {
            BackoffLevel::Normal => "正常",
            BackoffLevel::SwitchHost => "切换Host",
            BackoffLevel::SlowDown => "降速",
            BackoffLevel::Degrade => "降级",
            BackoffLevel::CircuitBreak(_) => "熔断",
        }
    }
}

/// 退避状态机：记录连续失败次数，自动升级退避级别。
pub struct BackoffState {
    /// 当前退避级别
    pub level: BackoffLevel,
    /// 连续失败次数
    consecutive_failures: AtomicU32,
    /// 连续成功次数
    consecutive_successes: AtomicU32,
    /// 进入当前级别的时间
    since: Mutex<Instant>,
    /// 熔断恢复所需的连续成功次数
    recovery_threshold: u32,
}

impl BackoffState {
    pub fn new() -> Self {
        Self {
            level: BackoffLevel::Normal,
            consecutive_failures: AtomicU32::new(0),
            consecutive_successes: AtomicU32::new(0),
            since: Mutex::new(Instant::now()),
            recovery_threshold: 3,
        }
    }

    /// 记录一次成功。如果处于退避状态且连续成功达标，则降级退避。
    pub fn record_success(&mut self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        let s = self.consecutive_successes.fetch_add(1, Ordering::Relaxed) + 1;

        if self.level > BackoffLevel::Normal {
            if let Ok(mut since) = self.since.lock() {
                *since = Instant::now();
            }
        }

        // 熔断恢复检查
        if matches!(self.level, BackoffLevel::CircuitBreak(_)) && s >= self.recovery_threshold {
            info!("[RateBudget] 连续 {} 次成功，熔断恢复 → 正常", s);
            self.level = BackoffLevel::Normal;
            self.consecutive_successes.store(0, Ordering::Relaxed);
        } else if self.level > BackoffLevel::Normal && s >= self.recovery_threshold {
            // 降级恢复
            let next = match self.level {
                BackoffLevel::CircuitBreak(_) => BackoffLevel::Degrade,
                BackoffLevel::Degrade => BackoffLevel::SlowDown,
                BackoffLevel::SlowDown => BackoffLevel::SwitchHost,
                BackoffLevel::SwitchHost => BackoffLevel::Normal,
                BackoffLevel::Normal => BackoffLevel::Normal,
            };
            info!("[RateBudget] 恢复: {:?} → {:?}", self.level, next);
            self.level = next;
            self.consecutive_successes.store(0, Ordering::Relaxed);
        }
    }

    /// 记录一次失败。达到阈值时自动升级退避级别。
    pub fn record_failure(&mut self) {
        self.consecutive_successes.store(0, Ordering::Relaxed);
        let f = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut since) = self.since.lock() {
            *since = Instant::now();
        }

        let threshold = match self.level {
            BackoffLevel::Normal => 3,
            BackoffLevel::SwitchHost => 5,
            BackoffLevel::SlowDown => 8,
            BackoffLevel::Degrade => 10,
            BackoffLevel::CircuitBreak(_) => u32::MAX, // 已熔断，不再升级
        };

        if f >= threshold {
            let next = match self.level {
                BackoffLevel::Normal => BackoffLevel::SwitchHost,
                BackoffLevel::SwitchHost => BackoffLevel::SlowDown,
                BackoffLevel::SlowDown => BackoffLevel::Degrade,
                BackoffLevel::Degrade => BackoffLevel::CircuitBreak(300), // 熔断5分钟
                BackoffLevel::CircuitBreak(s) => BackoffLevel::CircuitBreak(s * 2), // 翻倍
            };
            warn!(
                "[RateBudget] 连续 {} 次失败，退避: {:?} → {:?}",
                f, self.level, next
            );
            self.level = next;
            self.consecutive_failures.store(0, Ordering::Relaxed);
        }
    }

    /// 检查当前是否处于熔断状态
    pub fn is_circuit_broken(&self) -> bool {
        matches!(self.level, BackoffLevel::CircuitBreak(_))
    }

    /// 如果处于熔断状态且已超过冷却时间，尝试半开恢复
    pub fn try_half_open(&self) -> bool {
        if let BackoffLevel::CircuitBreak(cool_secs) = self.level {
            if let Ok(since) = self.since.lock() {
                if since.elapsed().as_secs() >= cool_secs {
                    return true; // 可以尝试
                }
            }
        }
        false
    }
}

impl Default for BackoffState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 多 Host 轮转管理器
// ============================================================================

/// 管理多个 API Host，在失败时自动轮转。
pub struct HostRotator {
    hosts: Vec<String>,
    current: AtomicU32,
    circuit_broken: Vec<AtomicBool>,
}

impl HostRotator {
    pub fn new(hosts: Vec<String>) -> Self {
        let n = hosts.len();
        Self {
            hosts,
            current: AtomicU32::new(0),
            circuit_broken: (0..n).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// 获取当前 Host
    pub fn current(&self) -> &str {
        let idx = self.current.load(Ordering::Relaxed) as usize;
        &self.hosts[idx % self.hosts.len()]
    }

    /// 轮转到下一个可用的 Host
    pub fn rotate(&self) -> &str {
        let n = self.hosts.len() as u32;
        for _ in 0..n {
            let next = (self.current.load(Ordering::Relaxed) + 1) % n;
            self.current.store(next, Ordering::Relaxed);
            if !self.circuit_broken[next as usize].load(Ordering::Relaxed) {
                return &self.hosts[next as usize];
            }
        }
        // 全部被熔断，返回当前
        let idx = self.current.load(Ordering::Relaxed) as usize;
        &self.hosts[idx % self.hosts.len()]
    }

    /// 标记当前 Host 为熔断
    pub fn mark_broken(&self) {
        let idx = self.current.load(Ordering::Relaxed) as usize;
        if idx < self.circuit_broken.len() {
            self.circuit_broken[idx].store(true, Ordering::Relaxed);
            warn!("[RateBudget] Host {} 标记为熔断", self.hosts[idx]);
        }
        self.rotate();
    }

    /// 恢复所有 Host
    pub fn reset_all(&self) {
        for b in &self.circuit_broken {
            b.store(false, Ordering::Relaxed);
        }
    }

    /// 检查是否所有 Host 均已熔断
    pub fn all_broken(&self) -> bool {
        self.circuit_broken.iter().all(|b| b.load(Ordering::Relaxed))
    }
}

// ============================================================================
// 请求协调器（组合 RateBudget + BackoffState + HostRotator）
// ============================================================================

/// 数据源请求协调器：管理单个数据源的配额、退避和 Host 轮转。
pub struct RequestCoordinator {
    pub budget: RateBudget,
    pub backoff: BackoffState,
    pub rotator: Option<HostRotator>,
    /// 当前轮询间隔乘数（1.0 = 原始间隔，退避时递增）
    interval_multiplier: AtomicU32,
}

impl RequestCoordinator {
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            budget: RateBudget::new(max_per_minute),
            backoff: BackoffState::new(),
            rotator: None,
            interval_multiplier: AtomicU32::new(1),
        }
    }

    pub fn with_hosts(max_per_minute: u32, hosts: Vec<String>) -> Self {
        Self {
            budget: RateBudget::new(max_per_minute),
            backoff: BackoffState::new(),
            rotator: Some(HostRotator::new(hosts)),
            interval_multiplier: AtomicU32::new(1),
        }
    }

    /// 请求前检查：配额 + 熔断状态
    pub fn can_request(&self) -> bool {
        if self.backoff.is_circuit_broken() && !self.backoff.try_half_open() {
            return false;
        }
        self.budget.try_acquire()
    }

    /// 请求成功回调
    pub fn on_success(&mut self) {
        self.backoff.record_success();
    }

    /// 请求失败回调
    pub fn on_failure(&mut self) {
        let prev_level = self.backoff.level;
        self.backoff.record_failure();
        if let Some(ref rotator) = self.rotator {
            rotator.mark_broken();
        }
        // 仅在退避级别上升时才增加间隔乘数（避免每次失败都翻倍）
        if self.backoff.level > prev_level {
            let m = self.interval_multiplier.load(Ordering::Relaxed);
            let next = match m {
                1 => 2,
                2 => 4,
                4 => 8,
                _ => 8,
            };
            self.interval_multiplier.store(next, Ordering::Relaxed);
        }
    }

    /// 获取当前有效轮询间隔（基础间隔 × 退避乘数）
    pub fn effective_interval_secs(&self, base_secs: u64) -> u64 {
        let m = self.interval_multiplier.load(Ordering::Relaxed) as u64;
        base_secs * m
    }

    /// 尝试恢复：如果连续成功达标，降级退避并重置乘数
    pub fn try_recover(&mut self) {
        if self.backoff.level <= BackoffLevel::SwitchHost {
            self.interval_multiplier.store(1, Ordering::Relaxed);
            if let Some(ref rotator) = self.rotator {
                rotator.reset_all();
            }
        }
    }

    /// 状态摘要（用于系统自监控）
    pub fn status(&self) -> String {
        format!(
            "budget={}/{} backoff={:?} interval=x{}",
            self.budget.used(),
            self.budget.limit(),
            self.backoff.level,
            self.interval_multiplier.load(Ordering::Relaxed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_rate_budget_allows_within_limit() {
        let budget = RateBudget::new(5);
        for _ in 0..5 {
            assert!(budget.try_acquire());
        }
        assert!(!budget.try_acquire()); // 第6次拒绝
    }

    #[test]
    fn test_rate_budget_window_reset() {
        let budget = RateBudget::with_window(3, 1); // 1秒窗口，3次上限
        for _ in 0..3 {
            assert!(budget.try_acquire());
        }
        assert!(!budget.try_acquire());
        std::thread::sleep(std::time::Duration::from_secs(2));
        // 窗口过期后应该可重新获取
        assert!(budget.try_acquire());
    }

    #[test]
    fn test_rate_budget_remaining() {
        let budget = RateBudget::new(10);
        assert_eq!(budget.remaining(), 10);
        budget.try_acquire();
        budget.try_acquire();
        assert_eq!(budget.remaining(), 8);
    }

    #[test]
    fn test_backoff_escalates_on_failures() {
        let mut state = BackoffState::new();
        assert_eq!(state.level, BackoffLevel::Normal);

        // 3 consecutive failures → SwitchHost
        state.record_failure();
        state.record_failure();
        state.record_failure();
        assert_eq!(state.level, BackoffLevel::SwitchHost);
    }

    #[test]
    fn test_backoff_recovers_on_successes() {
        let mut state = BackoffState::new();
        // Force to SwitchHost
        for _ in 0..3 {
            state.record_failure();
        }
        assert_eq!(state.level, BackoffLevel::SwitchHost);

        // 3 consecutive successes → Normal
        for _ in 0..3 {
            state.record_success();
        }
        assert_eq!(state.level, BackoffLevel::Normal);
    }

    #[test]
    fn test_backoff_success_resets_failures() {
        let mut state = BackoffState::new();
        state.record_failure();
        state.record_failure();
        state.record_success(); // resets failure count
        state.record_failure();
        state.record_failure();
        // Should still be Normal (only 2 consecutive after reset)
        assert_eq!(state.level, BackoffLevel::Normal);
    }

    #[test]
    fn test_circuit_breaker() {
        let mut state = BackoffState::new();
        // Force to Degrade → CircuitBreak
        for _ in 0..3 {
            state.record_failure();
        }
        for _ in 0..5 {
            state.record_failure();
        }
        for _ in 0..8 {
            state.record_failure();
        }
        for _ in 0..10 {
            state.record_failure();
        }
        assert!(state.is_circuit_broken());
    }

    #[test]
    fn test_host_rotator_basic() {
        let rotator = HostRotator::new(vec![
            "host1.com".into(),
            "host2.com".into(),
            "host3.com".into(),
        ]);
        assert_eq!(rotator.current(), "host1.com");
        rotator.rotate();
        assert_eq!(rotator.current(), "host2.com");
    }

    #[test]
    fn test_host_rotator_skips_broken() {
        let rotator = HostRotator::new(vec!["a".into(), "b".into(), "c".into()]);
        // Rotate to b, mark it broken
        rotator.rotate();
        assert_eq!(rotator.current(), "b");
        rotator.mark_broken(); // marks b, rotates to c
        assert_eq!(rotator.current(), "c");
        // Rotate again → should skip b (broken) → go to a
        rotator.rotate();
        assert_eq!(rotator.current(), "a");
    }

    #[test]
    fn test_request_coordinator_quota() {
        let mut coord = RequestCoordinator::new(3);
        assert!(coord.can_request());
        assert!(coord.can_request());
        assert!(coord.can_request());
        assert!(!coord.can_request()); // quota exhausted
    }

    #[test]
    fn test_request_coordinator_interval_multiplier() {
        let mut coord = RequestCoordinator::new(10);
        assert_eq!(coord.effective_interval_secs(30), 30); // x1

        coord.on_failure();
        coord.on_failure();
        coord.on_failure(); // triggers backoff
        assert_eq!(coord.effective_interval_secs(30), 60); // x2 after failures

        // Recover
        for _ in 0..5 {
            coord.on_success();
        }
        coord.try_recover();
        assert_eq!(coord.effective_interval_secs(30), 30); // back to x1
    }

    #[test]
    fn test_backoff_labels() {
        assert_eq!(BackoffLevel::Normal.label(), "正常");
        assert_eq!(BackoffLevel::CircuitBreak(300).label(), "熔断");
    }
}
