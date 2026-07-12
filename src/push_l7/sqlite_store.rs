//! push_l7/sqlite_store.rs — SQLiteStore (W7.2)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.7 落地.
//!
//! W7.2 范围:
//!   - SQLiteStore (持久化 PushAnalytics 到 push_analytics 表)
//!   - 表 schema: 复用现有 diesel schema 或新建 (W7.2 用新建, 不破坏 v10 governance_log)
//!   - CRUD: record / get / query_by_time_range / count_total / count_by_governance / push_rate
//!
//! 注: W7.2 用 rusqlite (已存在依赖) 直接操作, 不引 diesel schema. v10 governance_log 兼容由外层
//!     schema migration 处理 (不在本模块范围).

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local};
use rusqlite::{params, Connection};

use crate::push_l1::Severity;
use crate::push_l2::DataMode;
use crate::push_l5::GovernanceDecision;
use crate::push_l7::{AnalyticsStore, PushAnalytics, ValidationStatus};

/// SQLiteStore — 持久化 PushAnalytics 到 push_analytics 表
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// 打开 / 创建 SQLite 数据库 + 表
    pub fn open<P: AsRef<Path>>(path: P) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS push_analytics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                template_id TEXT NOT NULL,
                template_version INTEGER NOT NULL,
                ts TEXT NOT NULL,
                severity TEXT NOT NULL,
                data_mode TEXT NOT NULL,
                validation_status TEXT NOT NULL,
                governance_decision TEXT NOT NULL,
                pushed INTEGER NOT NULL,
                rendered_len INTEGER NOT NULL,
                sink_name TEXT NOT NULL,
                user_id TEXT NOT NULL,
                validation_errors TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_push_analytics_event_id ON push_analytics(event_id);
            CREATE INDEX IF NOT EXISTS idx_push_analytics_ts ON push_analytics(ts);
            CREATE INDEX IF NOT EXISTS idx_push_analytics_template_id ON push_analytics(template_id);
            "#,
        )?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// 打开内存数据库 (测试用)
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS push_analytics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                template_id TEXT NOT NULL,
                template_version INTEGER NOT NULL,
                ts TEXT NOT NULL,
                severity TEXT NOT NULL,
                data_mode TEXT NOT NULL,
                validation_status TEXT NOT NULL,
                governance_decision TEXT NOT NULL,
                pushed INTEGER NOT NULL,
                rendered_len INTEGER NOT NULL,
                sink_name TEXT NOT NULL,
                user_id TEXT NOT NULL,
                validation_errors TEXT NOT NULL
            );
            "#,
        )?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }
}

fn severity_to_str(s: Severity) -> &'static str {
    match s {
        Severity::Emergency => "emergency",
        Severity::High => "high",
        Severity::Normal => "normal",
        Severity::Info => "info",
    }
}

fn severity_from_str(s: &str) -> Severity {
    match s {
        "emergency" => Severity::Emergency,
        "high" => Severity::High,
        "normal" => Severity::Normal,
        "info" => Severity::Info,
        _ => Severity::Normal,
    }
}

fn data_mode_to_str(m: DataMode) -> &'static str {
    match m {
        DataMode::Full => "full",
        DataMode::Degraded => "degraded",
        DataMode::Unsafe => "unsafe",
        DataMode::Down => "down",
    }
}

fn data_mode_from_str(s: &str) -> DataMode {
    match s {
        "full" => DataMode::Full,
        "degraded" => DataMode::Degraded,
        "unsafe" => DataMode::Unsafe,
        "down" => DataMode::Down,
        _ => DataMode::Full,
    }
}

fn governance_to_str(d: &GovernanceDecision) -> String {
    match d {
        GovernanceDecision::Approve => "Approve".to_string(),
        GovernanceDecision::Deny(r) => format!("Deny:{}", r),
    }
}

fn governance_from_str(s: &str) -> GovernanceDecision {
    if s == "Approve" {
        GovernanceDecision::Approve
    } else if let Some(reason) = s.strip_prefix("Deny:") {
        GovernanceDecision::Deny(reason.to_string())
    } else {
        GovernanceDecision::Approve
    }
}

fn validation_status_to_str(v: ValidationStatus) -> &'static str {
    v.as_str()
}

fn validation_status_from_str(s: &str) -> ValidationStatus {
    match s {
        "passed" => ValidationStatus::Passed,
        "degraded" => ValidationStatus::Degraded,
        "retried" => ValidationStatus::Retried,
        "dropped" => ValidationStatus::Dropped,
        _ => ValidationStatus::Passed,
    }
}

impl AnalyticsStore for SqliteStore {
    fn record(&self, analytics: &PushAnalytics) {
        let conn = self.conn.lock().unwrap();
        let validation_errors_json = serde_json::to_string(&analytics.validation_errors).unwrap_or_else(|_| "[]".to_string());
        let result = conn.execute(
            "INSERT INTO push_analytics
             (event_id, template_id, template_version, ts, severity, data_mode,
              validation_status, governance_decision, pushed, rendered_len,
              sink_name, user_id, validation_errors)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                analytics.event_id,
                analytics.template_id,
                analytics.template_version,
                analytics.ts.to_rfc3339(),
                severity_to_str(analytics.severity),
                data_mode_to_str(analytics.data_mode),
                validation_status_to_str(analytics.validation_status),
                governance_to_str(&analytics.governance_decision),
                analytics.pushed as i32,
                analytics.rendered_len as i64,
                analytics.sink_name,
                analytics.user_id,
                validation_errors_json,
            ],
        );
        if let Err(e) = result {
            log::error!("[SqliteStore] record 失败: {}", e);
        }
    }

    fn get_by_event_id(&self, event_id: &str) -> Option<PushAnalytics> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT event_id, template_id, template_version, ts, severity, data_mode,
                    validation_status, governance_decision, pushed, rendered_len,
                    sink_name, user_id, validation_errors
             FROM push_analytics WHERE event_id = ?1 LIMIT 1"
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("[SqliteStore] prepare failed: {}", e);
                return None;
            }
        };
        let mut rows = match stmt.query(params![event_id]) {
            Ok(r) => r,
            Err(e) => {
                log::error!("[SqliteStore] query failed: {}", e);
                return None;
            }
        };
        let row = match rows.next() {
            Ok(Some(r)) => r,
            _ => return None,
        };
        Some(PushAnalytics {
            event_id: row.get(0).unwrap_or_default(),
            template_id: row.get(1).unwrap_or_default(),
            template_version: row.get::<_, i64>(2).unwrap_or(0) as u32,
            ts: parse_rfc3339(row.get::<_, String>(3).unwrap_or_default()),
            severity: severity_from_str(&row.get::<_, String>(4).unwrap_or_default()),
            data_mode: data_mode_from_str(&row.get::<_, String>(5).unwrap_or_default()),
            validation_status: validation_status_from_str(&row.get::<_, String>(6).unwrap_or_default()),
            governance_decision: governance_from_str(&row.get::<_, String>(7).unwrap_or_default()),
            pushed: row.get::<_, i64>(8).unwrap_or(0) != 0,
            rendered_len: row.get::<_, i64>(9).unwrap_or(0) as usize,
            sink_name: row.get(10).unwrap_or_default(),
            user_id: row.get(11).unwrap_or_default(),
            validation_errors: serde_json::from_str(&row.get::<_, String>(12).unwrap_or_default()).unwrap_or_default(),
        })
    }

    fn query_by_time_range(&self, from: DateTime<Local>, to: DateTime<Local>) -> Vec<PushAnalytics> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT event_id, template_id, template_version, ts, severity, data_mode,
                    validation_status, governance_decision, pushed, rendered_len,
                    sink_name, user_id, validation_errors
             FROM push_analytics WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map(params![from.to_rfc3339(), to.to_rfc3339()], |row| {
            Ok(PushAnalytics {
                event_id: row.get(0)?,
                template_id: row.get(1)?,
                template_version: row.get::<_, i64>(2)? as u32,
                ts: parse_rfc3339(row.get::<_, String>(3)?),
                severity: severity_from_str(&row.get::<_, String>(4)?),
                data_mode: data_mode_from_str(&row.get::<_, String>(5)?),
                validation_status: validation_status_from_str(&row.get::<_, String>(6)?),
                governance_decision: governance_from_str(&row.get::<_, String>(7)?),
                pushed: row.get::<_, i64>(8)? != 0,
                rendered_len: row.get::<_, i64>(9)? as usize,
                sink_name: row.get(10)?,
                user_id: row.get(11)?,
                validation_errors: serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default(),
            })
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    fn count_total(&self) -> u64 {
        let conn = self.conn.lock().unwrap();
        conn.query_row::<u64, _, _>("SELECT COUNT(*) FROM push_analytics", [], |row| row.get(0))
            .unwrap_or(0)
    }

    fn count_by_governance(&self, decision: &GovernanceDecision) -> u64 {
        let conn = self.conn.lock().unwrap();
        let pattern = match decision {
            GovernanceDecision::Approve => "Approve".to_string(),
            GovernanceDecision::Deny(r) => format!("Deny:{}", r),
        };
        conn.query_row::<u64, _, _>(
            "SELECT COUNT(*) FROM push_analytics WHERE governance_decision = ?1",
            params![pattern],
            |row| row.get(0),
        )
        .unwrap_or(0)
    }

    fn push_rate(&self) -> f64 {
        let total = self.count_total();
        if total == 0 {
            return 0.0;
        }
        let conn = self.conn.lock().unwrap();
        let pushed: u64 = conn.query_row(
            "SELECT COUNT(*) FROM push_analytics WHERE pushed = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
        pushed as f64 / total as f64
    }
}

fn parse_rfc3339(s: String) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(|_| Local::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_l1::{LimitUpPayload, Severity, SignalEvent, SignalPayload};
    use crate::push_l2::RenderedText;
    use crate::push_l7::build_analytics;
    use chrono::Local;

    fn make_event() -> SignalEvent {
        SignalEvent::new(
            crate::push_l1::SignalSource::LimitUp,
            "limit_up",
            Some("600519".to_string()),
            Local::now(),
            SignalPayload::LimitUp(LimitUpPayload::default()),
            Severity::High,
        )
    }

    fn make_analytics(pushed: bool, decision: GovernanceDecision, errors: Vec<String>) -> PushAnalytics {
        let event = make_event();
        build_analytics(
            &event,
            "limit_up_v1",
            1,
            DataMode::Full,
            decision,
            Some(&RenderedText::new("body")),
            pushed,
            "console",
            "default",
            errors,
        )
    }

    #[test]
    fn sqlite_store_open_in_memory() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.count_total(), 0);
    }

    #[test]
    fn sqlite_store_record_and_get() {
        let store = SqliteStore::open_in_memory().unwrap();
        let a = make_analytics(true, GovernanceDecision::Approve, vec![]);
        let event_id = a.event_id.clone();
        store.record(&a);
        assert_eq!(store.count_total(), 1);
        let got = store.get_by_event_id(&event_id).unwrap();
        assert_eq!(got.event_id, event_id);
        assert_eq!(got.template_id, "limit_up_v1");
        assert!(got.pushed);
    }

    #[test]
    fn sqlite_store_count_by_governance() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.record(&make_analytics(true, GovernanceDecision::Approve, vec![]));
        store.record(&make_analytics(false, GovernanceDecision::Deny("frozen".to_string()), vec![]));
        store.record(&make_analytics(false, GovernanceDecision::Deny("quiet_hour".to_string()), vec![]));
        assert_eq!(store.count_by_governance(&GovernanceDecision::Approve), 1);
        assert_eq!(store.count_by_governance(&GovernanceDecision::Deny("quiet_hour".to_string())), 1);
    }

    #[test]
    fn sqlite_store_push_rate() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.record(&make_analytics(true, GovernanceDecision::Approve, vec![]));
        store.record(&make_analytics(true, GovernanceDecision::Approve, vec![]));
        store.record(&make_analytics(false, GovernanceDecision::Deny("frozen".to_string()), vec![]));
        let rate = store.push_rate();
        assert!((rate - 0.6666).abs() < 0.01);
    }

    #[test]
    fn sqlite_store_query_by_time_range() {
        let store = SqliteStore::open_in_memory().unwrap();
        let now = Local::now();
        let mut a1 = make_analytics(true, GovernanceDecision::Approve, vec![]);
        a1.ts = now - chrono::Duration::hours(2);
        let mut a2 = make_analytics(true, GovernanceDecision::Approve, vec![]);
        a2.ts = now;
        store.record(&a1);
        store.record(&a2);
        let recent = store.query_by_time_range(now - chrono::Duration::hours(1), now);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].event_id, a2.event_id);
    }

    #[test]
    fn sqlite_store_severity_serialization() {
        assert_eq!(severity_to_str(Severity::Emergency), "emergency");
        assert_eq!(severity_to_str(Severity::High), "high");
        assert_eq!(severity_from_str("emergency"), Severity::Emergency);
        assert_eq!(severity_from_str("unknown"), Severity::Normal);
    }

    #[test]
    fn sqlite_store_data_mode_serialization() {
        assert_eq!(data_mode_to_str(DataMode::Down), "down");
        assert_eq!(data_mode_to_str(DataMode::Full), "full");
        assert_eq!(data_mode_from_str("unsafe"), DataMode::Unsafe);
    }

    #[test]
    fn sqlite_store_governance_serialization() {
        assert_eq!(governance_to_str(&GovernanceDecision::Approve), "Approve");
        assert_eq!(governance_to_str(&GovernanceDecision::Deny("quiet_hour".to_string())), "Deny:quiet_hour");
        assert_eq!(governance_from_str("Approve"), GovernanceDecision::Approve);
        assert_eq!(
            governance_from_str("Deny:data_quality"),
            GovernanceDecision::Deny("data_quality".to_string())
        );
    }

    #[test]
    fn sqlite_store_validation_status_serialization() {
        assert_eq!(validation_status_to_str(ValidationStatus::Passed), "passed");
        assert_eq!(validation_status_to_str(ValidationStatus::Dropped), "dropped");
        assert_eq!(validation_status_from_str("dropped"), ValidationStatus::Dropped);
    }
}