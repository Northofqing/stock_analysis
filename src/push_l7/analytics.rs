//! push_l7/analytics.rs — L7 Analytics (v14.2 §3.7)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.7 落地.
//!
//! W7.1 范围:
//!   - PushAnalytics struct (记录每次推送: event_id, template_id, version, dedup 标记, governance 结果)
//!   - AnalyticsStore trait (持久化抽象)
//!   - InMemoryStore (默认实现, 用于单进程 + 测试)
//!   - ValidationStatus 枚举 (passed / degraded / retried / dropped)
//!   - 12+ 单测
//!
//! 后续 W7.2 会加: SQLiteStore (持久化到 push_analytics 表, 与 v10 governance_log 兼容).
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2: analytics 不静默填补, 验证失败必须显式记录 (ValidationStatus::Dropped)
//!   - b-009 R-1: validation_status 字段必填, 不允许 None (除可选错误详情)

use std::sync::Arc;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::push_l1::{Severity, SignalEvent};
use crate::push_l2::{DataMode, RenderedText};
use crate::push_l5::GovernanceDecision;

/// 推送分析记录 (每次推送/尝试 1 条)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushAnalytics {
    /// 关联的 event_id (用于去重 + 跨层关联)
    pub event_id: String,
    /// 模板 ID (例: "limit_up_v2")
    pub template_id: String,
    /// 模板版本号 (用于灰度分析)
    pub template_version: u32,
    /// 推送时间戳
    pub ts: DateTime<Local>,
    /// 严重度
    pub severity: Severity,
    /// 当前数据质量
    pub data_mode: DataMode,
    /// 验证状态
    pub validation_status: ValidationStatus,
    /// 治理决策
    pub governance_decision: GovernanceDecision,
    /// 推送是否真正发出 (vs 被 dedup / governance 拦截)
    pub pushed: bool,
    /// 渲染文本长度 (用于 analytics: 文本大小)
    pub rendered_len: usize,
    /// Sink 名称 (多 Sink 时记录是哪个实际推送)
    pub sink_name: String,
    /// 用户 ID
    pub user_id: String,
    /// 验证错误详情 (仅 failed 时填, AGENTS.md §2.7 audit trail)
    pub validation_errors: Vec<String>,
}

/// 数据契约验证状态 (b-009 R-1 修订后仅 4 种)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidationStatus {
    /// 验证通过, 正常推送
    Passed,
    /// 用 fallback 降级推送 (b-009 R-1 后已删除, 仅作历史兼容, 不再产生)
    Degraded,
    /// 重试后通过
    Retried,
    /// 验证失败且丢弃
    Dropped,
}

impl ValidationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Degraded => "degraded",
            Self::Retried => "retried",
            Self::Dropped => "dropped",
        }
    }
}

/// AnalyticsStore trait — 持久化抽象
///
/// v14.2 §3.7: AnalyticsStore 是 trait, 可独立实现 (InMemory / SQLite / 远程)
pub trait AnalyticsStore: Send + Sync {
    /// 记录一次推送
    fn record(&self, analytics: &PushAnalytics) -> Result<(), String>;

    /// 按 event_id 查询
    fn get_by_event_id(&self, event_id: &str) -> Result<Option<PushAnalytics>, String>;

    /// 按时间范围查询
    fn query_by_time_range(
        &self,
        from: DateTime<Local>,
        to: DateTime<Local>,
    ) -> Result<Vec<PushAnalytics>, String>;

    /// 统计总数
    fn count_total(&self) -> Result<u64, String>;

    /// 按 governance_decision 统计
    fn count_by_governance(&self, decision: &GovernanceDecision) -> Result<u64, String>;

    /// 统计推送率 (pushed / total)；无记录时为 None。
    fn push_rate(&self) -> Result<Option<f64>, String>;
}

// ============================================================================
// InMemoryStore — 默认实现
// ============================================================================

/// InMemoryStore — 内存实现, 用于测试 + 单进程 (重启数据丢失)
pub struct InMemoryStore {
    records: Arc<std::sync::Mutex<Vec<PushAnalytics>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            records: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// 当前记录数 (调试用)
    pub fn len(&self) -> usize {
        crate::util::recover_lock_or_warn("InMemoryStore::len", self.records.lock()).len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        crate::util::recover_lock_or_warn("InMemoryStore::is_empty", self.records.lock()).is_empty()
    }

    /// 清空 (测试用)
    pub fn clear(&self) {
        crate::util::recover_lock_or_warn("InMemoryStore::clear", self.records.lock()).clear();
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalyticsStore for InMemoryStore {
    fn record(&self, analytics: &PushAnalytics) -> Result<(), String> {
        crate::util::recover_lock_or_warn("InMemoryStore::record", self.records.lock())
            .push(analytics.clone());
        Ok(())
    }

    fn get_by_event_id(&self, event_id: &str) -> Result<Option<PushAnalytics>, String> {
        Ok(
            crate::util::recover_lock_or_warn(
                "InMemoryStore::get_by_event_id",
                self.records.lock(),
            )
            .iter()
            .find(|a| a.event_id == event_id)
            .cloned(),
        )
    }

    fn query_by_time_range(
        &self,
        from: DateTime<Local>,
        to: DateTime<Local>,
    ) -> Result<Vec<PushAnalytics>, String> {
        Ok(crate::util::recover_lock_or_warn(
            "InMemoryStore::query_by_time_range",
            self.records.lock(),
        )
        .iter()
        .filter(|a| a.ts >= from && a.ts <= to)
        .cloned()
        .collect())
    }

    fn count_total(&self) -> Result<u64, String> {
        u64::try_from(
            crate::util::recover_lock_or_warn("InMemoryStore::count_total", self.records.lock())
                .len(),
        )
        .map_err(|error| format!("in-memory analytics count overflow: {error}"))
    }

    fn count_by_governance(&self, decision: &GovernanceDecision) -> Result<u64, String> {
        u64::try_from(
            crate::util::recover_lock_or_warn(
                "InMemoryStore::count_by_governance",
                self.records.lock(),
            )
            .iter()
            .filter(|a| &a.governance_decision == decision)
            .count(),
        )
        .map_err(|error| format!("in-memory governance count overflow: {error}"))
    }

    fn push_rate(&self) -> Result<Option<f64>, String> {
        let records =
            crate::util::recover_lock_or_warn("InMemoryStore::push_rate", self.records.lock());
        if records.is_empty() {
            return Ok(None);
        }
        let pushed = records.iter().filter(|a| a.pushed).count();
        Ok(Some(pushed as f64 / records.len() as f64))
    }
}

// ============================================================================
// 工厂: 从 SignalEvent + 治理结果构造 PushAnalytics
// ============================================================================

/// 从 SignalEvent + 治理结果构造 PushAnalytics
///
/// 这是 L4 dispatcher 在 Step 4 推送完成后调用的工厂
#[allow(
    clippy::too_many_arguments,
    reason = "audit-record factory keeps governance and real delivery outcomes distinct"
)]
pub fn build_analytics(
    event: &SignalEvent,
    template_id: &str,
    template_version: u32,
    data_mode: DataMode,
    governance_decision: GovernanceDecision,
    rendered: Option<&RenderedText>,
    // b011 P0-1: 真实投递结果由调用方显式传入.
    // 旧实现从 governance_decision.is_approve() 推导 — 批准≠送达, sink 失败也会记 pushed=1 (假数据).
    pushed: bool,
    sink_name: &str,
    user_id: &str,
    validation_errors: Vec<String>,
) -> PushAnalytics {
    let rendered_len = rendered.map(|r| r.body.len()).unwrap_or(0);

    // validation_status 规则 (W7.X MEDIUM-1 修订):
    // - 任何 validation_errors 非空 → Dropped (不论 governance 结果)
    // - validation_errors 为空 → Passed (W7.1 简化, 后续 L3 validate 阶段会显式传 Retried/Degraded)
    // 注: 这是简化, 实际 L3 validate 阶段会显式传 validation_status
    let validation_status = if !validation_errors.is_empty() {
        ValidationStatus::Dropped
    } else {
        ValidationStatus::Passed
    };

    PushAnalytics {
        event_id: event.event_id.clone(),
        template_id: template_id.to_string(),
        template_version,
        ts: event.ts,
        severity: event.severity,
        data_mode,
        validation_status,
        governance_decision,
        pushed,
        rendered_len,
        sink_name: sink_name.to_string(),
        user_id: user_id.to_string(),
        validation_errors,
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_l1::{LimitUpPayload, SignalPayload};

    fn make_event() -> SignalEvent {
        SignalEvent::new(
            crate::push_l1::SignalSource::LimitUp,
            "limit_up",
            Some("TEST_CODE_600519".to_string()),
            Local::now(),
            SignalPayload::LimitUp(LimitUpPayload::default()),
            Severity::High,
        )
    }

    fn make_analytics(pushed: bool, decision: GovernanceDecision) -> PushAnalytics {
        PushAnalytics {
            event_id: "abc123".to_string(),
            template_id: "limit_up_v1".to_string(),
            template_version: 1,
            ts: Local::now(),
            severity: Severity::High,
            data_mode: DataMode::Full,
            validation_status: ValidationStatus::Passed,
            governance_decision: decision,
            pushed,
            rendered_len: 100,
            sink_name: "console".to_string(),
            user_id: "default".to_string(),
            validation_errors: vec![],
        }
    }

    #[test]
    fn validation_status_as_str() {
        assert_eq!(ValidationStatus::Passed.as_str(), "passed");
        assert_eq!(ValidationStatus::Dropped.as_str(), "dropped");
        assert_eq!(ValidationStatus::Retried.as_str(), "retried");
        assert_eq!(ValidationStatus::Degraded.as_str(), "degraded");
    }

    #[test]
    fn in_memory_store_record_and_get() {
        let store = InMemoryStore::new();
        assert!(store.is_empty());

        let a = make_analytics(true, GovernanceDecision::Approve);
        store.record(&a).unwrap();

        assert_eq!(store.len(), 1);
        let got = store.get_by_event_id("abc123").unwrap().unwrap();
        assert_eq!(got.event_id, "abc123");
        assert_eq!(got.template_id, "limit_up_v1");
    }

    #[test]
    fn in_memory_store_get_by_event_id_missing() {
        let store = InMemoryStore::new();
        assert!(store.get_by_event_id("nonexistent").unwrap().is_none());
    }

    #[test]
    fn in_memory_store_count_total() {
        let store = InMemoryStore::new();
        for i in 0..5 {
            let mut a = make_analytics(true, GovernanceDecision::Approve);
            a.event_id = format!("e{}", i);
            store.record(&a).unwrap();
        }
        assert_eq!(store.count_total().unwrap(), 5);
    }

    #[test]
    fn in_memory_store_count_by_governance() {
        let store = InMemoryStore::new();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("frozen".to_string()),
            ))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("quiet_hour".to_string()),
            ))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("quiet_hour".to_string()),
            ))
            .unwrap();
        assert_eq!(
            store
                .count_by_governance(&GovernanceDecision::Approve)
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .count_by_governance(&GovernanceDecision::Deny("quiet_hour".to_string()))
                .unwrap(),
            2
        );
    }

    #[test]
    fn push_rate_calculation() {
        let store = InMemoryStore::new();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve))
            .unwrap();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("frozen".to_string()),
            ))
            .unwrap();
        assert!((store.push_rate().unwrap().unwrap() - 0.6666).abs() < 0.01);
    }

    #[test]
    fn push_rate_empty_store() {
        let store = InMemoryStore::new();
        assert_eq!(store.push_rate().unwrap(), None);
    }

    #[test]
    fn push_rate_all_pushed() {
        let store = InMemoryStore::new();
        for _ in 0..3 {
            store
                .record(&make_analytics(true, GovernanceDecision::Approve))
                .unwrap();
        }
        assert_eq!(store.push_rate().unwrap(), Some(1.0));
    }

    #[test]
    fn in_memory_store_query_by_time_range() {
        let store = InMemoryStore::new();
        let now = Local::now();
        let mut a1 = make_analytics(true, GovernanceDecision::Approve);
        a1.ts = now - chrono::Duration::hours(2);
        let mut a2 = make_analytics(true, GovernanceDecision::Approve);
        a2.ts = now;
        store.record(&a1).unwrap();
        store.record(&a2).unwrap();
        let recent = store
            .query_by_time_range(now - chrono::Duration::hours(1), now)
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].event_id, a2.event_id);
    }

    #[test]
    fn clear_resets_store() {
        let store = InMemoryStore::new();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve))
            .unwrap();
        assert_eq!(store.len(), 1);
        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn build_analytics_from_approved_event() {
        let event = make_event();
        let analytics = build_analytics(
            &event,
            "limit_up_v1",
            1,
            DataMode::Full,
            GovernanceDecision::Approve,
            Some(&RenderedText::new("hello")),
            true,
            "console",
            "default",
            vec![],
        );
        assert!(analytics.pushed);
        assert_eq!(analytics.validation_status, ValidationStatus::Passed);
        assert_eq!(analytics.rendered_len, 5);
        assert_eq!(analytics.event_id, event.event_id);
        assert_eq!(analytics.template_id, "limit_up_v1");
    }

    #[test]
    fn build_analytics_from_denied_event() {
        let event = make_event();
        let analytics = build_analytics(
            &event,
            "limit_up_v1",
            1,
            DataMode::Degraded,
            GovernanceDecision::Deny("data_quality".to_string()),
            None,
            false,
            "console",
            "default",
            vec![],
        );
        assert!(!analytics.pushed);
        assert_eq!(analytics.rendered_len, 0);
    }

    #[test]
    fn build_analytics_with_validation_errors_marks_dropped() {
        let event = make_event();
        let analytics = build_analytics(
            &event,
            "limit_up_v1",
            1,
            DataMode::Full,
            GovernanceDecision::Approve,
            None,
            false,
            "console",
            "default",
            vec!["missing fill_price".to_string()],
        );
        assert_eq!(analytics.validation_status, ValidationStatus::Dropped);
        assert_eq!(analytics.validation_errors.len(), 1);
    }
}
