//! push_l7/sqlite_store.rs — SQLiteStore (W7.2)
//! Registered business rule: BR-005.
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
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::push_l1::Severity;
use crate::push_l2::DataMode;
use crate::push_l5::GovernanceDecision;
use crate::push_l7::{AnalyticsStore, PushAnalytics, ValidationStatus};

/// SQLiteStore — 持久化 PushAnalytics 到 push_analytics 表
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

/// 全局 SqliteStore 单例 (v14_stack 初始化时 set, governance 查询时 get)
static GLOBAL_STORE: once_cell::sync::OnceCell<SqliteStore> = once_cell::sync::OnceCell::new();

impl SqliteStore {
    /// 注册全局 SqliteStore (V14Stack init 时调用一次)
    pub fn set_global(store: SqliteStore) {
        if GLOBAL_STORE.set(store).is_err() {
            log::warn!("[SqliteStore] set_global called twice, ignoring");
        }
    }

    /// 获取全局 SqliteStore 引用 (governance 检查时调用)
    pub fn global() -> Option<&'static SqliteStore> {
        GLOBAL_STORE.get()
    }

    /// 统计今天 (本地时区) 该 user 推送数量 — governance daily_limit 检查用
    pub fn count_today_for_user(
        &self,
        user_id: &str,
        today: chrono::NaiveDate,
    ) -> rusqlite::Result<i64> {
        let conn = crate::util::recover_lock_or_warn(
            "sqlite_store::count_today_for_user",
            self.conn.lock(),
        );
        conn.query_row(
            "SELECT COUNT(*) FROM push_analytics
             WHERE user_id = ?1 AND substr(ts, 1, 10) = ?2",
            params![user_id, today.to_string()],
            |row| row.get(0),
        )
    }

    /// BR-005: 统计本地日期内指定用户、指定模板的真实成功投递数。
    ///
    /// 治理拒绝、去重和 sink 失败记录都带 `pushed=0`，不得消耗日配额。
    pub fn count_today_pushed_for_user_and_template(
        &self,
        user_id: &str,
        template_id: &str,
        today: chrono::NaiveDate,
    ) -> rusqlite::Result<i64> {
        let conn = crate::util::recover_lock_or_warn(
            "sqlite_store::count_today_pushed_for_user_and_template",
            self.conn.lock(),
        );
        conn.query_row(
            "SELECT COUNT(*) FROM push_analytics
             WHERE user_id = ?1 AND template_id = ?2 AND pushed = 1
             AND substr(ts, 1, 10) = ?3",
            params![user_id, template_id, today.to_string()],
            |row| row.get(0),
        )
    }
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
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
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
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
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

fn severity_from_str(s: &str) -> Result<Severity, String> {
    match s {
        "emergency" => Ok(Severity::Emergency),
        "high" => Ok(Severity::High),
        "normal" => Ok(Severity::Normal),
        "info" => Ok(Severity::Info),
        _ => Err(format!("unknown severity {s:?}")),
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

fn data_mode_from_str(s: &str) -> Result<DataMode, String> {
    match s {
        "full" => Ok(DataMode::Full),
        "degraded" => Ok(DataMode::Degraded),
        "unsafe" => Ok(DataMode::Unsafe),
        "down" => Ok(DataMode::Down),
        _ => Err(format!("unknown data_mode {s:?}")),
    }
}

fn governance_to_str(d: &GovernanceDecision) -> String {
    match d {
        GovernanceDecision::Approve => "Approve".to_string(),
        GovernanceDecision::Deny(r) => format!("Deny:{}", r),
    }
}

fn governance_from_str(s: &str) -> Result<GovernanceDecision, String> {
    if s == "Approve" {
        Ok(GovernanceDecision::Approve)
    } else if let Some(reason) = s.strip_prefix("Deny:").filter(|reason| !reason.is_empty()) {
        Ok(GovernanceDecision::Deny(reason.to_string()))
    } else {
        Err(format!("unknown governance_decision {s:?}"))
    }
}

fn validation_status_to_str(v: ValidationStatus) -> &'static str {
    v.as_str()
}

fn validation_status_from_str(s: &str) -> Result<ValidationStatus, String> {
    match s {
        "passed" => Ok(ValidationStatus::Passed),
        "degraded" => Ok(ValidationStatus::Degraded),
        "retried" => Ok(ValidationStatus::Retried),
        "dropped" => Ok(ValidationStatus::Dropped),
        _ => Err(format!("unknown validation_status {s:?}")),
    }
}

struct RawAnalytics {
    event_id: String,
    template_id: String,
    template_version: i64,
    ts: String,
    severity: String,
    data_mode: String,
    validation_status: String,
    governance_decision: String,
    pushed: i64,
    rendered_len: i64,
    sink_name: String,
    user_id: String,
    validation_errors: String,
}

fn read_raw_analytics(row: &Row<'_>) -> rusqlite::Result<RawAnalytics> {
    Ok(RawAnalytics {
        event_id: row.get(0)?,
        template_id: row.get(1)?,
        template_version: row.get(2)?,
        ts: row.get(3)?,
        severity: row.get(4)?,
        data_mode: row.get(5)?,
        validation_status: row.get(6)?,
        governance_decision: row.get(7)?,
        pushed: row.get(8)?,
        rendered_len: row.get(9)?,
        sink_name: row.get(10)?,
        user_id: row.get(11)?,
        validation_errors: row.get(12)?,
    })
}

fn decode_analytics(raw: RawAnalytics) -> Result<PushAnalytics, String> {
    if raw.event_id.trim().is_empty()
        || raw.template_id.trim().is_empty()
        || raw.sink_name.trim().is_empty()
        || raw.user_id.trim().is_empty()
    {
        return Err("analytics row contains an empty identity field".to_string());
    }
    let template_version = u32::try_from(raw.template_version)
        .map_err(|error| format!("invalid template_version: {error}"))?;
    let rendered_len = usize::try_from(raw.rendered_len)
        .map_err(|error| format!("invalid rendered_len: {error}"))?;
    let pushed = match raw.pushed {
        0 => false,
        1 => true,
        value => return Err(format!("invalid pushed flag: {value}")),
    };
    let ts = parse_rfc3339(&raw.ts)?;
    let validation_errors = serde_json::from_str::<Vec<String>>(&raw.validation_errors)
        .map_err(|error| format!("invalid validation_errors JSON: {error}"))?;
    Ok(PushAnalytics {
        event_id: raw.event_id,
        template_id: raw.template_id,
        template_version,
        ts,
        severity: severity_from_str(&raw.severity)?,
        data_mode: data_mode_from_str(&raw.data_mode)?,
        validation_status: validation_status_from_str(&raw.validation_status)?,
        governance_decision: governance_from_str(&raw.governance_decision)?,
        pushed,
        rendered_len,
        sink_name: raw.sink_name,
        user_id: raw.user_id,
        validation_errors,
    })
}

impl AnalyticsStore for SqliteStore {
    fn record(&self, analytics: &PushAnalytics) -> Result<(), String> {
        let conn = crate::util::recover_lock_or_warn("sqlite_store::record", self.conn.lock());
        let validation_errors_json = serde_json::to_string(&analytics.validation_errors)
            .map_err(|error| format!("serialize analytics validation errors: {error}"))?;
        conn.execute(
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
        )
        .map_err(|error| format!("record push_analytics: {error}"))?;
        Ok(())
    }

    fn get_by_event_id(&self, event_id: &str) -> Result<Option<PushAnalytics>, String> {
        let conn =
            crate::util::recover_lock_or_warn("sqlite_store::get_by_event_id", self.conn.lock());
        let mut stmt = conn
            .prepare(
                "SELECT event_id, template_id, template_version, ts, severity, data_mode,
                    validation_status, governance_decision, pushed, rendered_len,
                    sink_name, user_id, validation_errors
             FROM push_analytics WHERE event_id = ?1 LIMIT 1",
            )
            .map_err(|error| format!("prepare get push_analytics: {error}"))?;
        let raw = stmt
            .query_row(params![event_id], read_raw_analytics)
            .optional()
            .map_err(|error| format!("query push_analytics by event_id: {error}"))?;
        raw.map(decode_analytics).transpose()
    }

    fn query_by_time_range(
        &self,
        from: DateTime<Local>,
        to: DateTime<Local>,
    ) -> Result<Vec<PushAnalytics>, String> {
        let conn = crate::util::recover_lock_or_warn(
            "sqlite_store::query_by_time_range",
            self.conn.lock(),
        );
        let mut stmt = conn
            .prepare(
                "SELECT event_id, template_id, template_version, ts, severity, data_mode,
                    validation_status, governance_decision, pushed, rendered_len,
                    sink_name, user_id, validation_errors
             FROM push_analytics WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts",
            )
            .map_err(|error| format!("prepare push_analytics range query: {error}"))?;
        let rows = stmt
            .query_map(
                params![from.to_rfc3339(), to.to_rfc3339()],
                read_raw_analytics,
            )
            .map_err(|error| format!("query push_analytics range: {error}"))?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("read push_analytics row: {error}"))?;
            out.push(decode_analytics(raw)?);
        }
        Ok(out)
    }

    fn count_total(&self) -> Result<u64, String> {
        let conn = crate::util::recover_lock_or_warn("sqlite_store::count_total", self.conn.lock());
        conn.query_row::<u64, _, _>("SELECT COUNT(*) FROM push_analytics", [], |row| row.get(0))
            .map_err(|error| format!("count push_analytics: {error}"))
    }

    fn count_by_governance(&self, decision: &GovernanceDecision) -> Result<u64, String> {
        let conn = crate::util::recover_lock_or_warn(
            "sqlite_store::count_by_governance",
            self.conn.lock(),
        );
        let pattern = match decision {
            GovernanceDecision::Approve => "Approve".to_string(),
            GovernanceDecision::Deny(r) => format!("Deny:{}", r),
        };
        conn.query_row::<u64, _, _>(
            "SELECT COUNT(*) FROM push_analytics WHERE governance_decision = ?1",
            params![pattern],
            |row| row.get(0),
        )
        .map_err(|error| format!("count push_analytics by governance: {error}"))
    }

    fn push_rate(&self) -> Result<Option<f64>, String> {
        let conn = crate::util::recover_lock_or_warn("sqlite_store::push_rate", self.conn.lock());
        let total: u64 = conn
            .query_row("SELECT COUNT(*) FROM push_analytics", [], |row| row.get(0))
            .map_err(|error| format!("count push_analytics for rate: {error}"))?;
        if total == 0 {
            return Ok(None);
        }
        let pushed: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM push_analytics WHERE pushed = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("count pushed analytics for rate: {error}"))?;
        Ok(Some(pushed as f64 / total as f64))
    }
}

/// Parse an RFC3339 timestamp without substituting the current time.
fn parse_rfc3339(s: &str) -> Result<DateTime<Local>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Local))
        .map_err(|error| format!("invalid analytics timestamp {s:?}: {error}"))
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
            Some("TEST_CODE_600519".to_string()),
            Local::now(),
            SignalPayload::LimitUp(LimitUpPayload::default()),
            Severity::High,
        )
    }

    fn make_analytics(
        pushed: bool,
        decision: GovernanceDecision,
        errors: Vec<String>,
    ) -> PushAnalytics {
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
        assert_eq!(store.count_total().unwrap(), 0);
    }

    #[test]
    fn sqlite_store_record_and_get() {
        let store = SqliteStore::open_in_memory().unwrap();
        let a = make_analytics(true, GovernanceDecision::Approve, vec![]);
        let event_id = a.event_id.clone();
        store.record(&a).unwrap();
        assert_eq!(store.count_total().unwrap(), 1);
        let got = store.get_by_event_id(&event_id).unwrap().unwrap();
        assert_eq!(got.event_id, event_id);
        assert_eq!(got.template_id, "limit_up_v1");
        assert!(got.pushed);
    }

    #[test]
    fn br113_record_failure_is_returned_to_caller() {
        let store = SqliteStore::open_in_memory().unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute("DROP TABLE push_analytics", []).unwrap();
        }
        let error = store
            .record(&make_analytics(true, GovernanceDecision::Approve, vec![]))
            .expect_err("missing audit table must reject the write");
        assert!(error.contains("record"));
    }

    #[test]
    fn sqlite_store_count_by_governance() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve, vec![]))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("frozen".to_string()),
                vec![],
            ))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("quiet_hour".to_string()),
                vec![],
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
            1
        );
    }

    #[test]
    fn sqlite_store_push_rate() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve, vec![]))
            .unwrap();
        store
            .record(&make_analytics(true, GovernanceDecision::Approve, vec![]))
            .unwrap();
        store
            .record(&make_analytics(
                false,
                GovernanceDecision::Deny("frozen".to_string()),
                vec![],
            ))
            .unwrap();
        let rate = store.push_rate().unwrap().unwrap();
        assert!((rate - 0.6666).abs() < 0.01);
    }

    #[test]
    fn br005_daily_template_count_only_includes_successful_deliveries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut successful = make_analytics(true, GovernanceDecision::Approve, vec![]);
        successful.template_id = "candidate_board".to_string();
        let mut failed = successful.clone();
        failed.event_id = "failed-event".to_string();
        failed.pushed = false;
        let mut other = successful.clone();
        other.event_id = "other-event".to_string();
        other.template_id = "holding_event".to_string();

        store.record(&successful).unwrap();
        store.record(&failed).unwrap();
        store.record(&other).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE push_analytics SET ts = '2026-07-19T00:30:00+08:00'",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            store
                .count_today_for_user(
                    "default",
                    chrono::NaiveDate::from_ymd_opt(2026, 7, 19).unwrap(),
                )
                .unwrap(),
            3
        );

        assert_eq!(
            store
                .count_today_pushed_for_user_and_template(
                    "default",
                    "candidate_board",
                    chrono::NaiveDate::from_ymd_opt(2026, 7, 19).unwrap(),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            store
                .count_today_pushed_for_user_and_template(
                    "default",
                    "candidate_board",
                    chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap(),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn sqlite_store_query_by_time_range() {
        let store = SqliteStore::open_in_memory().unwrap();
        let now = Local::now();
        let mut a1 = make_analytics(true, GovernanceDecision::Approve, vec![]);
        a1.ts = now - chrono::Duration::hours(2);
        let mut a2 = make_analytics(true, GovernanceDecision::Approve, vec![]);
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
    fn sqlite_store_severity_serialization() {
        assert_eq!(severity_to_str(Severity::Emergency), "emergency");
        assert_eq!(severity_to_str(Severity::High), "high");
        assert_eq!(severity_from_str("emergency").unwrap(), Severity::Emergency);
        assert!(severity_from_str("unknown").is_err());
    }

    #[test]
    fn sqlite_store_data_mode_serialization() {
        assert_eq!(data_mode_to_str(DataMode::Down), "down");
        assert_eq!(data_mode_to_str(DataMode::Full), "full");
        assert_eq!(data_mode_from_str("unsafe").unwrap(), DataMode::Unsafe);
    }

    #[test]
    fn sqlite_store_governance_serialization() {
        assert_eq!(governance_to_str(&GovernanceDecision::Approve), "Approve");
        assert_eq!(
            governance_to_str(&GovernanceDecision::Deny("quiet_hour".to_string())),
            "Deny:quiet_hour"
        );
        assert_eq!(
            governance_from_str("Approve").unwrap(),
            GovernanceDecision::Approve
        );
        assert_eq!(
            governance_from_str("Deny:data_quality").unwrap(),
            GovernanceDecision::Deny("data_quality".to_string())
        );
    }

    #[test]
    fn sqlite_store_validation_status_serialization() {
        assert_eq!(validation_status_to_str(ValidationStatus::Passed), "passed");
        assert_eq!(
            validation_status_to_str(ValidationStatus::Dropped),
            "dropped"
        );
        assert_eq!(
            validation_status_from_str("dropped").unwrap(),
            ValidationStatus::Dropped
        );
    }
}
