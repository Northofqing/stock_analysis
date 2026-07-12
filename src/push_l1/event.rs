//! push_l1/event.rs — SignalEvent 与 EventBucket (v14.2 §3.1 + §3.1.1)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 (b-009 R-2 修订后) 实现:
//!   - SignalEvent 是 L1 → L4 dispatcher 的唯一信号载体
//!   - EventBucket 用 enum 精确匹配, 避免字符串 contains 的 ID 碰撞 (b-009 R-2)
//!   - make_event_id 用 sha256(source_kind + code + bucket_ts) 取前 8 字节 (16 hex)
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2 数据红线 — 本模块不静默填补任何字段
//!   - 所有字段缺失 → Option::None / FailureStrategy::Drop (在 L3 validate 层处理)

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// SignalEvent — 唯一信号载体
///
/// 用于 L1 Signal Source → L4 Dispatcher 的标准消息。
/// 每个 event_id 由 make_event_id 派生, 用于 dispatcher dedup 表。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEvent {
    /// 唯一 ID, 算法见 `make_event_id`
    pub event_id: String,
    /// 事件来源 (例如 LimitUp, SectorRotation, DataSourceDown)
    pub source: SignalSource,
    /// 事件类型 kind 字符串, 用于 EventBucket::for_kind 匹配
    pub kind: String,
    /// 涉及的股票/板块代码, None = 全局事件
    pub code: Option<String>,
    /// 事件产生的本地时间
    pub ts: DateTime<Local>,
    /// 强类型 payload (各 SignalSource 自己定义)
    pub payload: SignalPayload,
    /// 严重度, 影响 dispatcher 优先级
    pub severity: Severity,
}

/// 事件来源
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SignalSource {
    /// 涨跌停扫描 (LimitUp payload)
    LimitUp,
    /// 板块轮动 (SectorRotation payload)
    SectorRotation,
    /// 新闻催化 (NewsCatalyst payload)
    NewsCatalyst,
    /// 风控告警 (RiskViolation payload)
    RiskViolation,
    /// 持仓健康 (HoldingHealth payload)
    HoldingHealth,
    /// 仓位变化 (PositionChanged payload)
    PositionChanged,
    /// 数据源健康 (DataSourceDown payload, b-008 §4.1 补充)
    DataSourceDown,
    /// 盘后复盘 (PostSessionReview payload, b-008 §2.1 补充)
    PostSessionReview,
    /// 静默时段自身治理 (QuietHour payload, b-008 §3.0 补充)
    QuietHour,
}

/// 事件严重度
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Emergency,
    High,
    Normal,
    Info,
}

/// 事件 payload (强类型, 缺失字段必须显式为 None)
///
/// 注意: 这里**没有任何 "Degrade..." 前缀的变体** —
///// 这与 b-009 R-1 修订一致, v14.2 §3.3.2 仅允许 Drop + RetryWithBackoff。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignalPayload {
    LimitUp(LimitUpPayload),
    LimitUpTier(LimitUpTierPayload),
    SectorRotation(SectorRotationPayload),
    NewsCatalyst(NewsCatalystPayload),
    RiskViolation(RiskViolationPayload),
    HoldingHealth(HoldingHealthPayload),
    PositionChanged(PositionChangedPayload),
    DataSourceDown(DataSourceDownPayload),
    PostSessionReview(PostSessionReviewPayload),
    QuietHour(QuietHourPayload),
}

// 各 payload 结构体 — 所有数值字段都是 Option<T>, 显式缺失
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitUpPayload {
    pub code: Option<String>,
    pub name: Option<String>,
    pub change_pct: Option<f64>,
    pub seal_amount_wan: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitUpTierPayload {
    pub code: Option<String>,
    pub tier: LimitUpTier,
    pub consecutive_boards: Option<u32>,
}

/// 涨停分级 (b-008 §3.1 补充)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LimitUpTier {
    #[default]
    First,
    Second,
    ThirdPlus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SectorRotationPayload {
    pub sector_code: Option<String>,
    pub sector_name: Option<String>,
    pub change_pct: Option<f64>,
    pub main_net_yi: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NewsCatalystPayload {
    pub code: Option<String>,
    pub headline: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskViolationPayload {
    pub rule_id: Option<String>,
    pub metric: Option<String>,
    pub threshold: Option<f64>,
    pub observed: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HoldingHealthPayload {
    pub code: Option<String>,
    pub position_pct: Option<f64>,
    pub day_pnl_pct: Option<f64>,
    pub entry_price: Option<f64>,
    pub current_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PositionChangedPayload {
    pub code: Option<String>,
    pub action: Option<String>,  // buy/sell/hold
    pub qty: Option<u32>,
    pub price: Option<f64>,
}

/// 数据源健康事件 payload (b-008 §4.1 补充)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataSourceDownPayload {
    pub source_name: Option<String>,
    pub consecutive_failures: Option<u32>,
    pub last_error: Option<String>,
}

/// 盘后复盘事件 payload (b-008 §2.1 补充)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PostSessionReviewPayload {
    pub review_type: Option<String>,  // lhb/market/signal/failure
    pub summary: Option<String>,
}

/// 静默时段自身治理 payload (b-008 §3.0 补充)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuietHourPayload {
    pub hour: Option<u32>,
    pub expected_inhibit_count: Option<u32>,
    pub actual_inhibit_count: Option<u32>,
}

// ============================================================================
// §3.1.1 event_id 生成算法 (b-009 R-2 修订后)
// ============================================================================

/// SignalEvent 唯一 ID 生成算法
///
/// **目标**: 同一业务事件产生 1 次 event_id, 不同业务事件产生不同 event_id
///
/// **算法**: sha256(source_kind + ":" + code_or_global + ":" + bucket_ts) -> hex[0..16]
///
/// **时间桶策略 (EventBucket enum 精确匹配, 不用字符串 contains)**:
///   - OneSec: 涨跌停 / 委托 / 盘口
///   - TenSec: 板块轮动 / 资金流 / 新闻催化 (默认)
///   - FiveMin: 盘后 / 节假日 / 静默时段
///   - OneSec (紧急): data_source_down / risk_violation
pub fn make_event_id(source_kind: &str, code: Option<&str>, ts: DateTime<Local>) -> String {
    use sha2::{Digest, Sha256};
    let code_str = code.unwrap_or("_global");
    let bucket_ts = bucket_ts(source_kind, ts);
    let input = format!("{source_kind}:{code_str}:{bucket_ts}");
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(&hash[..8])  // 16 hex 字符
}

fn bucket_ts(source_kind: &str, ts: DateTime<Local>) -> i64 {
    let bucket_secs = match EventBucket::for_kind(source_kind) {
        EventBucket::OneSec => 1,
        EventBucket::TenSec => 10,
        EventBucket::FiveMin => 300,
    };
    ts.timestamp() / bucket_secs * bucket_secs
}

/// 事件分桶枚举 (b-009 R-2 修订)
///
/// 旧版用字符串 contains 匹配分桶, 存在 ID 碰撞风险:
///   - `source_kind = "news.drivers_for_orders"` 撞 "order" → 误判 1 秒桶
///   - `source_kind = "limit_up_suspended"` 撞 "limit_up" → 误判 1 秒桶 (实际 5min)
///
/// 新方案: SignalPayload 各变体在编译期绑定 EventBucket, 避免字符串误匹配
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventBucket {
    OneSec,    // 涨跌停 / 委托 / 盘口
    TenSec,    // 板块轮动 / 资金流 / 新闻催化
    FiveMin,   // 盘后 / 节假日 / 静默时段
}

impl EventBucket {
    /// 精确枚举匹配 (不是字符串 contains)
    pub fn for_kind(kind: &str) -> Self {
        match kind {
            // 高频: 涨跌停 / 委托 / 盘口
            "limit_up" | "limit_up_today" | "limit_up_tier"
            | "order" | "depth" => Self::OneSec,
            // 中频: 板块轮动 / 资金流 / 新闻催化
            "sector_rotation" | "money_flow" | "news_catalyst" => Self::TenSec,
            // 低频: 盘后 / 节假日 / 静默时段
            "post_session" | "event_calendar" | "quiet_hour" => Self::FiveMin,
            // 紧急 1 秒 (不能漏): 数据源 / 风控
            "data_source_down" | "risk_violation" => Self::OneSec,
            // 默认 10 秒
            _ => Self::TenSec,
        }
    }
}

/// SignalEvent 构造辅助 — 业务方用这个构造, 自动派生 event_id
impl SignalEvent {
    pub fn new(
        source: SignalSource,
        kind: impl Into<String>,
        code: Option<String>,
        ts: DateTime<Local>,
        payload: SignalPayload,
        severity: Severity,
    ) -> Self {
        let kind_str = kind.into();
        let event_id = make_event_id(&kind_str, code.as_deref(), ts);
        Self {
            event_id,
            source,
            kind: kind_str,
            code,
            ts,
            payload,
            severity,
        }
    }
}

// ============================================================================
// 单元测试 — 验证 (1) ID 唯一性 (2) EventBucket enum 匹配正确
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[test]
    fn same_event_same_id() {
        let ts = Local::now();
        let id1 = make_event_id("limit_up", Some("600519"), ts);
        let id2 = make_event_id("limit_up", Some("600519"), ts);
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_code_different_id() {
        let ts = Local::now();
        let id1 = make_event_id("limit_up", Some("600519"), ts);
        let id2 = make_event_id("limit_up", Some("000001"), ts);
        assert_ne!(id1, id2);
    }

    #[test]
    fn different_second_different_id_for_high_freq() {
        let ts1 = Local::now();
        let ts2 = ts1 + Duration::seconds(2);
        let id1 = make_event_id("order", Some("600519"), ts1);
        let id2 = make_event_id("order", Some("600519"), ts2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn same_bucket_dedup_for_low_freq() {
        // W7.2 修订: ts1 对齐到 5 分钟桶起点, ts2 +60s 必然同桶 (避免跨小时边界 flake)
        let now = Local::now().timestamp();
        let bucket_start = (now / 300) * 300;
        let ts1 = Local.timestamp_opt(bucket_start, 0).single().unwrap();
        let ts2 = ts1 + Duration::seconds(60);
        let id1 = make_event_id("post_session", None, ts1);
        let id2 = make_event_id("post_session", None, ts2);
        assert_eq!(id1, id2);
    }

    #[test]
    fn id_is_16_hex_chars() {
        let id = make_event_id("test", Some("000001"), Local::now());
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // b-009 R-2 核心验证: EventBucket 精确匹配, 避免 ID 碰撞
    #[test]
    fn event_bucket_no_id_collision_on_substring() {
        // 反例 1: "news.drivers_for_orders" 含 "order" 子串, 旧版会误判 1 秒桶
        //         新版精确匹配: 不在 kind 列表 → TenSec (默认)
        // 反例 2: "limit_up_suspended" 含 "limit_up" 子串, 旧版会误判 1 秒桶
        //         新版精确匹配: 不在 kind 列表 → TenSec (默认)
        assert_eq!(EventBucket::for_kind("news.drivers_for_orders"), EventBucket::TenSec);
        assert_eq!(EventBucket::for_kind("limit_up_suspended"), EventBucket::TenSec);
        assert_eq!(EventBucket::for_kind("drivers_for_orders"), EventBucket::TenSec);

        // 正例: 已注册的 kind 仍正确匹配
        assert_eq!(EventBucket::for_kind("limit_up"), EventBucket::OneSec);
        assert_eq!(EventBucket::for_kind("order"), EventBucket::OneSec);
        assert_eq!(EventBucket::for_kind("sector_rotation"), EventBucket::TenSec);
        assert_eq!(EventBucket::for_kind("post_session"), EventBucket::FiveMin);
        assert_eq!(EventBucket::for_kind("data_source_down"), EventBucket::OneSec);
        assert_eq!(EventBucket::for_kind("risk_violation"), EventBucket::OneSec);
    }

    #[test]
    fn signal_event_auto_derives_event_id() {
        let ts = Local::now();
        let payload = SignalPayload::LimitUp(LimitUpPayload {
            code: Some("600519".to_string()),
            name: Some("贵州茅台".to_string()),
            change_pct: Some(10.0),
            seal_amount_wan: None,
        });
        let event = SignalEvent::new(
            SignalSource::LimitUp,
            "limit_up",
            Some("600519".to_string()),
            ts,
            payload,
            Severity::High,
        );
        assert_eq!(event.event_id.len(), 16);
        assert!(event.event_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn payload_missing_fields_are_optional_not_zero() {
        // 验证: 所有 payload 数值字段都是 Option<T>, 不是 f64 默认 0.0
        let payload = HoldingHealthPayload::default();
        assert!(payload.entry_price.is_none(), "entry_price 必须为 None, 不能默认 0.0");
        assert!(payload.current_price.is_none());
        assert!(payload.position_pct.is_none());
        assert!(payload.day_pnl_pct.is_none());
    }
}