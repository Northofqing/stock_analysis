//! v12 PR2-2.1: 数据模式三态判定 (DataHealth ∈ {Full, Degraded, Unsafe}).
//!
//! 设计: 与 `risk::account_mode` 对齐 — 纯函数 + 数据入参, 不直接读行情 DB.
//!       Capability 各自维护真实成功时间；从未成功的能力保持 Missing.
//!
//! 状态机:
//!   Full --(任一关键 Capability staleness > 120s)--> Degraded
//!   Full/Degraded --(Quote staleness > 120s)--> Unsafe
//!   Degraded --(全部 Capability 恢复)--> Full
//!
//! 关键设计: **OrderBook 恒缺不拖累全局模式** (PR2-2.1 专项要求)
//!   - OrderBook Missing → 计入 `missing_capabilities`, 但 DataMode 仍可 Full
//!   - 只有缺盘口时, 推送横幅显示 "[⚠️ 缺盘口深度: 本条不含承接判断]"
//!   - 业务侧 (T-07 候选触发) 缺盘口时 EvidenceQuality=Missing, 但不阻塞触发

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

/// v12 §2.4 数据能力枚举
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// 实时行情
    Quote,
    /// K线 (日/分钟)
    Kline,
    /// 资金流 (主力/北向)
    MoneyFlow,
    /// 新闻快讯
    News,
    /// 盘口深度 (十档买卖)
    OrderBook,
}

impl Capability {
    pub const ALL: [Capability; 5] = [
        Capability::Quote,
        Capability::Kline,
        Capability::MoneyFlow,
        Capability::News,
        Capability::OrderBook,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Capability::Quote => "Quote",
            Capability::Kline => "Kline",
            Capability::MoneyFlow => "MoneyFlow",
            Capability::News => "News",
            Capability::OrderBook => "OrderBook",
        }
    }

    /// 该 capability 缺失是否影响"价格型建议" (PR2-2.1 关键)
    ///
    /// true = 关键能力, 缺失降级 DataMode
    /// false = 辅助能力 (盘口), 缺失只挂横幅, 不降级
    pub fn is_critical(self) -> bool {
        !matches!(self, Capability::OrderBook)
    }
}

/// 三态数据模式
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DataMode {
    Full,
    Degraded,
    Unsafe,
}

impl DataMode {
    pub fn label(self) -> &'static str {
        match self {
            DataMode::Full => "Full",
            DataMode::Degraded => "Degraded",
            DataMode::Unsafe => "Unsafe",
        }
    }
}

/// BR-135: a confirmed persistent-Unsafe reminder remains quiet for 30 minutes.
pub const PERSISTENT_UNSAFE_REMINDER_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Tracks only authoritative DataMode delivery confirmation.
///
/// The caller supplies the monotonic clock and records a result only after the
/// real sink and every mandatory audit have succeeded.
#[derive(Debug, Default)]
pub struct PersistentUnsafeReminder {
    last_confirmed_at: Option<Instant>,
}

impl PersistentUnsafeReminder {
    /// Clears the prior outage interval as soon as real health has recovered.
    /// This observation is independent of whether the recovery notification is delivered.
    pub fn observe_mode(&mut self, mode: DataMode) -> bool {
        let cleared = mode != DataMode::Unsafe && self.last_confirmed_at.is_some();
        if mode != DataMode::Unsafe {
            self.last_confirmed_at = None;
        }
        cleared
    }

    pub fn should_dispatch(&self, mode: DataMode, now: Instant) -> Result<bool, String> {
        if mode != DataMode::Unsafe {
            return Ok(false);
        }
        let Some(last_confirmed_at) = self.last_confirmed_at else {
            return Ok(true);
        };
        let elapsed = now
            .checked_duration_since(last_confirmed_at)
            .ok_or_else(|| {
                "BR-135 monotonic reminder clock moved backwards; reminder state unchanged"
                    .to_string()
            })?;
        Ok(elapsed >= PERSISTENT_UNSAFE_REMINDER_INTERVAL)
    }

    pub fn record_confirmed(&mut self, mode: DataMode, now: Instant) {
        self.observe_mode(mode);
        if mode == DataMode::Unsafe {
            self.last_confirmed_at = Some(now);
        }
    }
}

/// 单个 capability 的新鲜度快照 (由主循环填入)
#[derive(Copy, Clone, Debug)]
pub struct CapabilityStatus {
    pub cap: Capability,
    /// 自上次成功刷新起的秒数. None 表示从未刷新过 (Missing).
    pub staleness_secs: Option<u64>,
}

impl CapabilityStatus {
    pub fn missing(cap: Capability) -> Self {
        Self {
            cap,
            staleness_secs: None,
        }
    }

    pub fn fresh(cap: Capability, secs: u64) -> Self {
        Self {
            cap,
            staleness_secs: Some(secs),
        }
    }

    /// 是否可用: 有数据且 staleness ≤ max_age_secs
    pub fn is_ok(&self, max_age_secs: u64) -> bool {
        match self.staleness_secs {
            Some(s) => s <= max_age_secs,
            None => false,
        }
    }
}

/// 入参: 5 个 capability 的状态 + 配置
#[derive(Clone, Debug)]
pub struct DataHealthInput {
    pub capabilities: Vec<CapabilityStatus>,
    /// 关键 capability 的 staleness 阈值 (默认 120s, 复用 data_quality.rs:296-309)
    pub critical_max_age_secs: u64,
    /// OrderBook 专用阈值 (默认 600s, 因为盘口刷新频率低)
    pub orderbook_max_age_secs: u64,
}

impl Default for DataHealthInput {
    fn default() -> Self {
        Self {
            capabilities: Capability::ALL
                .iter()
                .map(|c| CapabilityStatus::missing(*c))
                .collect(),
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        }
    }
}

/// 评估结果
#[derive(Clone, Debug)]
pub struct DataHealth {
    pub mode: DataMode,
    pub missing: Vec<Capability>,
    /// prev 模式 (None 表示首次评估)
    pub prev_mode: Option<DataMode>,
    /// ETA 恢复预计 (供 T-02 推送文案), 简单写 "N/A" or "{capability} 刷新后"
    pub eta: Option<String>,
}

impl DataHealth {
    pub fn is_changed(&self) -> bool {
        match self.prev_mode {
            Some(p) => p != self.mode,
            None => false, // 首次评估不算变更, 不触发 T-02
        }
    }
}

/// PR2-2.1 主评估函数
///
/// 规则:
///   1. 任一**关键** capability 缺失或 staleness > critical_max_age_secs → Degraded
///   2. Quote staleness > critical_max_age_secs (即行情断流) → Unsafe
///   3. OrderBook 缺失只计入 missing, 不触发 Degraded (专项要求)
///   4. 全 Full 且全 fresh → Full
///
/// `prev` 由调用方从 history 表恢复, 首次评估传 None.
pub fn evaluate(input: &DataHealthInput, prev: Option<DataMode>) -> DataHealth {
    let mut missing: Vec<Capability> = Vec::new();
    let mut critical_stale: Vec<Capability> = Vec::new();
    let mut quote_stale = false;

    for cs in &input.capabilities {
        let max_age = if cs.cap.is_critical() {
            input.critical_max_age_secs
        } else {
            input.orderbook_max_age_secs
        };

        if cs.is_ok(max_age) {
            continue;
        }

        missing.push(cs.cap);

        if cs.cap.is_critical() {
            critical_stale.push(cs.cap);
            if matches!(cs.cap, Capability::Quote) {
                quote_stale = true;
            }
        }
    }

    // 1. Quote 断流 → Unsafe
    if quote_stale {
        return DataHealth {
            mode: DataMode::Unsafe,
            missing,
            prev_mode: prev,
            eta: Some("Quote 恢复后".to_string()),
        };
    }

    // 2. 关键能力降级 → Degraded
    if !critical_stale.is_empty() {
        let caps: Vec<String> = critical_stale
            .iter()
            .map(|c| c.label().to_string())
            .collect();
        let eta = format!("{} 刷新后", caps.join("/"));
        return DataHealth {
            mode: DataMode::Degraded,
            missing,
            prev_mode: prev,
            eta: Some(eta),
        };
    }

    // 3. 仅辅助能力缺失 (OrderBook) → Full, 横幅提示
    DataHealth {
        mode: DataMode::Full,
        missing,
        prev_mode: prev,
        eta: None,
    }
}

/// 便利: 构造 DataHealthInput from `(cap, last_update_secs_ago)` pairs
pub fn input_from_pairs(
    critical_max_age_secs: u64,
    pairs: &[(Capability, Option<u64>)],
) -> DataHealthInput {
    DataHealthInput {
        capabilities: pairs
            .iter()
            .map(|(cap, s)| match s {
                Some(secs) => CapabilityStatus::fresh(*cap, *secs),
                None => CapabilityStatus::missing(*cap),
            })
            .collect(),
        critical_max_age_secs,
        orderbook_max_age_secs: 600,
    }
}

static LAST_CAPABILITY_SUCCESS: OnceLock<RwLock<HashMap<Capability, Instant>>> = OnceLock::new();

fn capability_successes() -> &'static RwLock<HashMap<Capability, Instant>> {
    LAST_CAPABILITY_SUCCESS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Record a capability only after its production source and quality checks succeed.
pub fn mark_capability_success(capability: Capability) -> Result<(), String> {
    capability_successes()
        .write()
        .map_err(|_| "capability success tracker write lock poisoned".to_string())?
        .insert(capability, Instant::now());
    Ok(())
}

fn input_from_successes_at(
    successes: &HashMap<Capability, Instant>,
    now: Instant,
    critical_max_age_secs: u64,
    orderbook_max_age_secs: u64,
) -> DataHealthInput {
    DataHealthInput {
        capabilities: Capability::ALL
            .iter()
            .map(|capability| CapabilityStatus {
                cap: *capability,
                staleness_secs: successes
                    .get(capability)
                    .map(|last_success| now.saturating_duration_since(*last_success).as_secs()),
            })
            .collect(),
        critical_max_age_secs,
        orderbook_max_age_secs,
    }
}

/// Build a health snapshot from actual process-local source successes.
///
/// A capability absent from the tracker has never succeeded in this process and
/// is therefore reported as Missing. OrderBook is intentionally never marked by
/// current production code because no real depth source is wired yet.
pub fn current_data_health_input(
    critical_max_age_secs: u64,
    orderbook_max_age_secs: u64,
) -> Result<DataHealthInput, String> {
    let successes = capability_successes()
        .read()
        .map_err(|_| "capability success tracker read lock poisoned".to_string())?;
    Ok(input_from_successes_at(
        &successes,
        Instant::now(),
        critical_max_age_secs,
        orderbook_max_age_secs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_all_fresh() -> DataHealthInput {
        DataHealthInput {
            capabilities: Capability::ALL
                .iter()
                .map(|c| CapabilityStatus::fresh(*c, 30))
                .collect(),
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        }
    }

    // ---- 全 Full 场景 ----

    #[test]
    fn full_when_all_fresh() {
        let h = evaluate(&input_all_fresh(), None);
        assert_eq!(h.mode, DataMode::Full);
        assert!(h.missing.is_empty());
        assert!(!h.is_changed());
    }

    #[test]
    fn full_when_only_orderbook_missing() {
        let mut input = input_all_fresh();
        input.capabilities[4] = CapabilityStatus::missing(Capability::OrderBook);
        let h = evaluate(&input, Some(DataMode::Full));
        // OrderBook 缺失 → Full, 但 missing 包含
        assert_eq!(h.mode, DataMode::Full, "OrderBook 缺失不降级");
        assert!(h.missing.contains(&Capability::OrderBook));
    }

    #[test]
    fn full_when_orderbook_stale() {
        let mut input = input_all_fresh();
        // OrderBook 5 分钟前, 阈值 600s → 仍 ok
        input.capabilities[4] = CapabilityStatus::fresh(Capability::OrderBook, 300);
        let h = evaluate(&input, None);
        assert_eq!(h.mode, DataMode::Full);
    }

    // ---- Degraded 场景 ----

    #[test]
    fn degraded_when_kline_stale() {
        let mut input = input_all_fresh();
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, 200);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Degraded);
        assert!(h.is_changed(), "Full → Degraded 应算变更");
        assert!(h.missing.contains(&Capability::Kline));
        assert!(h.eta.as_ref().unwrap().contains("Kline"));
    }

    #[test]
    fn degraded_when_news_missing() {
        let mut input = input_all_fresh();
        input.capabilities[3] = CapabilityStatus::missing(Capability::News);
        let h = evaluate(&input, None);
        assert_eq!(h.mode, DataMode::Degraded);
        assert!(h.missing.contains(&Capability::News));
    }

    #[test]
    fn degraded_when_moneyflow_stale() {
        let mut input = input_all_fresh();
        input.capabilities[2] = CapabilityStatus::fresh(Capability::MoneyFlow, 130);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Degraded);
    }

    // ---- Unsafe 场景 ----

    #[test]
    fn unsafe_when_quote_stale() {
        let mut input = input_all_fresh();
        input.capabilities[0] = CapabilityStatus::fresh(Capability::Quote, 150);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Unsafe);
        assert_eq!(h.missing[0], Capability::Quote);
    }

    #[test]
    fn unsafe_when_quote_missing() {
        let mut input = input_all_fresh();
        input.capabilities[0] = CapabilityStatus::missing(Capability::Quote);
        let h = evaluate(&input, None);
        assert_eq!(h.mode, DataMode::Unsafe);
    }

    #[test]
    fn unsafe_takes_priority_over_degraded() {
        let mut input = input_all_fresh();
        // Kline stale (Degraded) + Quote stale (Unsafe) → Unsafe
        input.capabilities[0] = CapabilityStatus::fresh(Capability::Quote, 200);
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, 200);
        let h = evaluate(&input, Some(DataMode::Full));
        assert_eq!(h.mode, DataMode::Unsafe);
    }

    // ---- 状态变更 ----

    #[test]
    fn first_eval_not_changed() {
        let h = evaluate(&input_all_fresh(), None);
        assert!(!h.is_changed(), "首次评估不算变更");
    }

    #[test]
    fn same_mode_not_changed() {
        let h = evaluate(&input_all_fresh(), Some(DataMode::Full));
        assert!(!h.is_changed());
    }

    #[test]
    fn full_to_degraded_changed() {
        let mut input = input_all_fresh();
        input.capabilities[1] = CapabilityStatus::fresh(Capability::Kline, 200);
        let h = evaluate(&input, Some(DataMode::Full));
        assert!(h.is_changed());
    }

    // ---- Capability 分类 ----

    #[test]
    fn capability_critical_classification() {
        assert!(Capability::Quote.is_critical());
        assert!(Capability::Kline.is_critical());
        assert!(Capability::MoneyFlow.is_critical());
        assert!(Capability::News.is_critical());
        assert!(
            !Capability::OrderBook.is_critical(),
            "OrderBook 辅助, 缺失不降级"
        );
    }

    #[test]
    fn capability_labels() {
        assert_eq!(Capability::Quote.label(), "Quote");
        assert_eq!(Capability::Kline.label(), "Kline");
        assert_eq!(Capability::MoneyFlow.label(), "MoneyFlow");
        assert_eq!(Capability::News.label(), "News");
        assert_eq!(Capability::OrderBook.label(), "OrderBook");
    }

    #[test]
    fn data_mode_labels() {
        assert_eq!(DataMode::Full.label(), "Full");
        assert_eq!(DataMode::Degraded.label(), "Degraded");
        assert_eq!(DataMode::Unsafe.label(), "Unsafe");
    }

    #[test]
    fn br135_persistent_unsafe_reminder_is_due_only_after_confirmed_interval() {
        let start = Instant::now();
        let mut state = PersistentUnsafeReminder::default();

        assert!(state.should_dispatch(DataMode::Unsafe, start).unwrap());
        state.record_confirmed(DataMode::Unsafe, start);

        assert!(!state
            .should_dispatch(
                DataMode::Unsafe,
                start + std::time::Duration::from_secs(1_799),
            )
            .unwrap());
        assert!(state
            .should_dispatch(
                DataMode::Unsafe,
                start + std::time::Duration::from_secs(1_800),
            )
            .unwrap());

        assert!(state.observe_mode(DataMode::Full));
        assert!(!state.observe_mode(DataMode::Degraded));
        assert!(!state
            .should_dispatch(
                DataMode::Full,
                start + std::time::Duration::from_secs(3_600),
            )
            .unwrap());
        assert!(state
            .should_dispatch(
                DataMode::Unsafe,
                start + std::time::Duration::from_secs(3_600),
            )
            .unwrap());
    }

    // ---- 输入辅助 ----

    #[test]
    fn input_from_pairs_basic() {
        let input = input_from_pairs(
            120,
            &[(Capability::Quote, Some(10)), (Capability::OrderBook, None)],
        );
        assert_eq!(input.capabilities.len(), 2);
        assert!(input.capabilities[0].is_ok(120));
        assert!(!input.capabilities[1].is_ok(600));
    }

    #[test]
    fn tracker_input_keeps_never_successful_capabilities_missing() {
        let now = Instant::now();
        let mut successes = HashMap::new();
        successes.insert(Capability::Quote, now);

        let input = input_from_successes_at(&successes, now, 120, 600);

        assert_eq!(input.capabilities.len(), Capability::ALL.len());
        assert_eq!(input.capabilities[0].staleness_secs, Some(0));
        assert!(input.capabilities[1..]
            .iter()
            .all(|status| status.staleness_secs.is_none()));
    }

    #[test]
    fn tracker_input_uses_elapsed_time_since_success() {
        let now = Instant::now();
        let mut successes = HashMap::new();
        successes.insert(
            Capability::Kline,
            now.checked_sub(std::time::Duration::from_secs(121))
                .expect("test instant must support a short subtraction"),
        );

        let input = input_from_successes_at(&successes, now, 120, 600);
        let kline = input
            .capabilities
            .iter()
            .find(|status| status.cap == Capability::Kline)
            .expect("Kline status must exist");

        assert_eq!(kline.staleness_secs, Some(121));
        assert!(!kline.is_ok(input.critical_max_age_secs));
    }

    // ---- 边界 ----

    #[test]
    fn staleness_at_threshold_is_ok() {
        // staleness = max_age → still ok
        let cs = CapabilityStatus::fresh(Capability::Quote, 120);
        assert!(cs.is_ok(120));
    }

    #[test]
    fn staleness_above_threshold_is_stale() {
        let cs = CapabilityStatus::fresh(Capability::Quote, 121);
        assert!(!cs.is_ok(120));
    }

    /// 异常: missing 字段空, prev=Some 任意状态 → is_changed 取决于新模式
    #[test]
    fn all_missing_falls_to_unsafe() {
        let input = DataHealthInput::default(); // 5 个全 missing
        let h = evaluate(&input, None);
        assert_eq!(
            h.mode,
            DataMode::Unsafe,
            "全 missing → Quote missing → Unsafe"
        );
        assert_eq!(h.missing.len(), 5);
    }
}
