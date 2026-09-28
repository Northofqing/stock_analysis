//! Cross-restart admission for the two scheduled chain reports.
//!
//! The legacy notification channels return only weak acceptance. A recorded
//! send attempt therefore blocks automatic replay until an operator resolves
//! it; a crash or an error after the send boundary cannot be treated as a
//! definite rejection.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::chain_shadow_input::{self, ChainReportInputObservation};
use super::modes::{run_chain_analysis_mode_with_observation, ChainDeliveryEnvelope};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainPhase {
    Preopen,
    Postclose,
}

impl ChainPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preopen => "preopen",
            Self::Postclose => "postclose",
        }
    }

    fn window(self) -> (NaiveTime, NaiveTime) {
        match self {
            Self::Preopen => (
                NaiveTime::from_hms_opt(9, 5, 0).unwrap(),
                NaiveTime::from_hms_opt(9, 15, 0).unwrap(),
            ),
            Self::Postclose => (
                NaiveTime::from_hms_opt(15, 30, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 35, 0).unwrap(),
            ),
        }
    }

    pub fn starts_in_window(self, date: NaiveDate, observed_at: NaiveDateTime) -> bool {
        let (start, end) = self.window();
        observed_at.date() == date && observed_at.time() >= start && observed_at.time() < end
    }

    /// Read-only policy query for comparing the legacy missed-window decision.
    pub fn is_overdue(self, date: NaiveDate, observed_at: NaiveDateTime) -> bool {
        observed_at.date() == date && observed_at.time() >= self.window().1
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainScheduleStatus {
    Ready,
    Uncertain,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainScheduleOutcome {
    WeakAccepted,
    AlreadyClosed,
    NeedsReview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManualResolution {
    Delivered,
    Retry,
}

impl ManualResolution {
    fn state(self) -> &'static str {
        match self {
            Self::Delivered => "manual_delivered",
            Self::Retry => "manual_retry",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChainScheduleStore {
    path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainAttemptEvidence {
    pub attempt_no: i64,
    pub state: String,
    pub report_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub resolution_note: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainMissEvidence {
    pub detected_at: String,
    pub latest_attempt_no: Option<i64>,
    pub latest_state: Option<String>,
}

impl ChainScheduleStore {
    pub fn production() -> Self {
        Self::new(
            stock_analysis::production_root::production_root().join("data/chain_schedule.sqlite3"),
        )
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建产业链调度状态目录 {}", parent.display()))?;
        }
        let connection = Connection::open(&self.path)
            .with_context(|| format!("打开产业链调度状态库 {}", self.path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS chain_schedule_attempt_v1 (
                 phase TEXT NOT NULL CHECK (phase IN ('preopen', 'postclose')),
                 schedule_date TEXT NOT NULL,
                 attempt_no INTEGER NOT NULL CHECK (attempt_no > 0),
                 state TEXT NOT NULL CHECK (state IN
                     ('sending', 'weak_accepted', 'manual_delivered', 'manual_retry')),
                 report_path TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 resolution_note TEXT,
                 PRIMARY KEY (phase, schedule_date, attempt_no)
             );
             CREATE TABLE IF NOT EXISTS chain_schedule_miss_v1 (
                 phase TEXT NOT NULL CHECK (phase IN ('preopen', 'postclose')),
                 schedule_date TEXT NOT NULL,
                 detected_at TEXT NOT NULL,
                 latest_attempt_no INTEGER,
                 latest_state TEXT,
                 PRIMARY KEY (phase, schedule_date),
                 CHECK ((latest_attempt_no IS NULL) = (latest_state IS NULL))
             );
             CREATE TRIGGER IF NOT EXISTS chain_schedule_miss_no_update
             BEFORE UPDATE ON chain_schedule_miss_v1
             BEGIN SELECT RAISE(ABORT, 'chain schedule miss is immutable'); END;
             CREATE TRIGGER IF NOT EXISTS chain_schedule_miss_no_delete
             BEFORE DELETE ON chain_schedule_miss_v1
             BEGIN SELECT RAISE(ABORT, 'chain schedule miss is immutable'); END;",
        )?;
        Ok(connection)
    }

    fn latest_state(
        connection: &Connection,
        phase: ChainPhase,
        date: NaiveDate,
    ) -> Result<Option<(i64, String)>> {
        connection
            .query_row(
                "SELECT attempt_no, state FROM chain_schedule_attempt_v1
                 WHERE phase = ?1 AND schedule_date = ?2
                 ORDER BY attempt_no DESC LIMIT 1",
                params![phase.as_str(), date.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn status(&self, phase: ChainPhase, date: NaiveDate) -> Result<ChainScheduleStatus> {
        let connection = self.connect()?;
        status_from_state(
            Self::latest_state(&connection, phase, date)?
                .as_ref()
                .map(|(_, s)| s.as_str()),
        )
    }

    /// Read-only operator inspection. A wrong or absent database path is an
    /// error instead of silently creating an empty history.
    pub fn inspect(
        &self,
        phase: ChainPhase,
        date: NaiveDate,
    ) -> Result<(ChainScheduleStatus, Option<ChainAttemptEvidence>)> {
        let connection = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("只读打开产业链调度状态库 {}", self.path.display()))?;
        let attempt = connection
            .query_row(
                "SELECT attempt_no, state, report_path, created_at, updated_at, resolution_note
                 FROM chain_schedule_attempt_v1 WHERE phase = ?1 AND schedule_date = ?2
                 ORDER BY attempt_no DESC LIMIT 1",
                params![phase.as_str(), date.to_string()],
                |row| {
                    Ok(ChainAttemptEvidence {
                        attempt_no: row.get(0)?,
                        state: row.get(1)?,
                        report_path: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                        resolution_note: row.get(5)?,
                    })
                },
            )
            .optional()?;
        let status = status_from_state(attempt.as_ref().map(|row| row.state.as_str()))?;
        Ok((status, attempt))
    }

    /// A missed window records absence of a confirmed weak acceptance. It is
    /// observational only: no analysis, notification, or retry is triggered.
    pub fn record_missed_window(
        &self,
        phase: ChainPhase,
        date: NaiveDate,
        observed_at: DateTime<FixedOffset>,
    ) -> Result<Option<ChainMissEvidence>> {
        if !phase.is_overdue(date, observed_at.naive_local()) {
            return Ok(None);
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let latest = Self::latest_state(&transaction, phase, date)?;
        if status_from_state(latest.as_ref().map(|(_, state)| state.as_str()))?
            != ChainScheduleStatus::Ready
        {
            return Ok(None);
        }
        let evidence = ChainMissEvidence {
            detected_at: observed_at.to_rfc3339(),
            latest_attempt_no: latest.as_ref().map(|(number, _)| *number),
            latest_state: latest.map(|(_, state)| state),
        };
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO chain_schedule_miss_v1
             (phase, schedule_date, detected_at, latest_attempt_no, latest_state)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                phase.as_str(),
                date.to_string(),
                evidence.detected_at,
                evidence.latest_attempt_no,
                evidence.latest_state,
            ],
        )?;
        transaction.commit()?;
        Ok((inserted == 1).then_some(evidence))
    }

    pub fn inspect_miss(
        &self,
        phase: ChainPhase,
        date: NaiveDate,
    ) -> Result<Option<ChainMissEvidence>> {
        let connection = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("只读打开产业链调度状态库 {}", self.path.display()))?;
        let installed: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'chain_schedule_miss_v1')",
            [],
            |row| row.get(0),
        )?;
        if !installed {
            return Ok(None);
        }
        connection
            .query_row(
                "SELECT detected_at, latest_attempt_no, latest_state
                 FROM chain_schedule_miss_v1 WHERE phase = ?1 AND schedule_date = ?2",
                params![phase.as_str(), date.to_string()],
                |row| {
                    Ok(ChainMissEvidence {
                        detected_at: row.get(0)?,
                        latest_attempt_no: row.get(1)?,
                        latest_state: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn begin_send(&self, phase: ChainPhase, date: NaiveDate, report_path: &str) -> Result<i64> {
        anyhow::ensure!(!report_path.is_empty(), "产业链报告路径不能为空");
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let latest = Self::latest_state(&transaction, phase, date)?;
        if status_from_state(latest.as_ref().map(|(_, s)| s.as_str()))?
            != ChainScheduleStatus::Ready
        {
            bail!(
                "产业链 {} {} 已有发送记录，拒绝重复发送",
                phase.as_str(),
                date
            );
        }
        let attempt_no = latest.map_or(1, |(number, _)| number + 1);
        let now = Utc::now().to_rfc3339();
        let changed = transaction.execute(
            "INSERT INTO chain_schedule_attempt_v1
             (phase, schedule_date, attempt_no, state, report_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'sending', ?4, ?5, ?5)",
            params![
                phase.as_str(),
                date.to_string(),
                attempt_no,
                report_path,
                now
            ],
        )?;
        anyhow::ensure!(changed == 1, "产业链发送尝试写入失败");
        transaction.commit()?;
        Ok(attempt_no)
    }

    pub fn mark_weak_accepted(
        &self,
        phase: ChainPhase,
        date: NaiveDate,
        attempt_no: i64,
    ) -> Result<()> {
        let connection = self.connect()?;
        let changed = connection.execute(
            "UPDATE chain_schedule_attempt_v1
             SET state = 'weak_accepted', updated_at = ?4
             WHERE phase = ?1 AND schedule_date = ?2 AND attempt_no = ?3 AND state = 'sending'",
            params![
                phase.as_str(),
                date.to_string(),
                attempt_no,
                Utc::now().to_rfc3339()
            ],
        )?;
        anyhow::ensure!(changed == 1, "产业链弱接受状态写入失败，发送状态需人工检查");
        Ok(())
    }

    pub fn resolve_uncertain(
        &self,
        phase: ChainPhase,
        date: NaiveDate,
        resolution: ManualResolution,
        note: &str,
    ) -> Result<()> {
        anyhow::ensure!(!note.trim().is_empty(), "人工裁定必须记录依据");
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((attempt_no, state)) = Self::latest_state(&transaction, phase, date)? else {
            bail!("产业链 {} {} 没有待裁定发送", phase.as_str(), date);
        };
        anyhow::ensure!(state == "sending", "只有发送状态不明的产业链报告可人工裁定");
        let changed = transaction.execute(
            "UPDATE chain_schedule_attempt_v1
             SET state = ?4, updated_at = ?5, resolution_note = ?6
             WHERE phase = ?1 AND schedule_date = ?2 AND attempt_no = ?3 AND state = 'sending'",
            params![
                phase.as_str(),
                date.to_string(),
                attempt_no,
                resolution.state(),
                Utc::now().to_rfc3339(),
                note.trim()
            ],
        )?;
        anyhow::ensure!(changed == 1, "产业链人工裁定写入失败");
        transaction.commit()?;
        Ok(())
    }
}

fn status_from_state(state: Option<&str>) -> Result<ChainScheduleStatus> {
    match state {
        None | Some("manual_retry") => Ok(ChainScheduleStatus::Ready),
        Some("sending") => Ok(ChainScheduleStatus::Uncertain),
        Some("weak_accepted" | "manual_delivered") => Ok(ChainScheduleStatus::Closed),
        Some(other) => bail!("未知产业链发送状态 {other}，停止自动重试"),
    }
}

fn scheduled_report_filename(
    phase: ChainPhase,
    date: NaiveDate,
    generated_at: DateTime<Utc>,
) -> String {
    format!(
        "chain_analysis_schedule_{}_{}_{}.md",
        date.format("%Y%m%d"),
        phase.as_str(),
        generated_at.format("%Y%m%dT%H%M%S%fZ")
    )
}

pub async fn run_scheduled_chain_analysis(
    store: &ChainScheduleStore,
    phase: ChainPhase,
    date: NaiveDate,
) -> Result<ChainScheduleOutcome> {
    match store.status(phase, date)? {
        ChainScheduleStatus::Closed => {
            log::info!(
                "[chain_shadow_suppression] phase={} schedule_date={} reason=already_closed coverage=incomplete",
                phase.as_str(), date
            );
            return Ok(ChainScheduleOutcome::AlreadyClosed);
        }
        ChainScheduleStatus::Uncertain => {
            log::info!(
                "[chain_shadow_suppression] phase={} schedule_date={} reason=uncertain_needs_review coverage=incomplete",
                phase.as_str(), date
            );
            return Ok(ChainScheduleOutcome::NeedsReview);
        }
        ChainScheduleStatus::Ready => {}
    }

    let observed_now = Local::now().naive_local();
    if !phase.starts_in_window(date, observed_now) {
        log::info!(
            "[chain_shadow_suppression] phase={} schedule_date={} reason=outside_send_window coverage=incomplete",
            phase.as_str(), date
        );
    }
    anyhow::ensure!(
        phase.starts_in_window(date, observed_now),
        "产业链 {} {} 已不在新报告发送窗口，禁止窗口外重新采集并发送",
        phase.as_str(),
        date
    );

    let filename = scheduled_report_filename(phase, date, Utc::now());
    let mut attempt_no = None;
    let envelope = run_chain_analysis_mode_with_observation(true, Some(&filename), |report_path| {
        attempt_no = Some(store.begin_send(phase, date, report_path)?);
        Ok(())
    })
    .await?;
    finish_scheduled_delivery(
        envelope,
        phase,
        date,
        || {
            let attempt_no = attempt_no.context("产业链发送守卫未执行")?;
            store.mark_weak_accepted(phase, date, attempt_no)
        },
        chain_shadow_input::observe,
    )
}

pub(super) fn finish_scheduled_delivery<M, O>(
    envelope: ChainDeliveryEnvelope,
    phase: ChainPhase,
    date: NaiveDate,
    mark: M,
    observer: O,
) -> Result<ChainScheduleOutcome>
where
    M: FnOnce() -> Result<()>,
    O: FnOnce(
        ChainPhase,
        NaiveDate,
        &stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis,
        &[u8],
        Option<&super::chain_acquisition::ChainAcquisitionEvidence>,
    ) -> Result<ChainReportInputObservation>,
{
    let legacy_result = envelope.legacy_result.and_then(|()| {
        mark()?;
        Ok(ChainScheduleOutcome::WeakAccepted)
    });
    if let Some(reason) = envelope.suppression {
        log::info!(
            "[chain_shadow_suppression] phase={} schedule_date={} reason={} coverage=incomplete",
            phase.as_str(),
            date,
            reason.as_str()
        );
    }
    if let Some(report) = envelope.notification_report.as_ref() {
        let attempted_targets = report.attempts().len();
        let visible_targets = report
            .attempts()
            .iter()
            .take(16)
            .map(|attempt| {
                format!(
                    "{}:{}:{:?}",
                    attempt.channel().name(),
                    attempt.target_index(),
                    attempt.outcome()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        log::info!(
            "[chain_shadow_channel_attempts] phase={} schedule_date={} report_input_sha256={:x} targets={} accepted={} unknown={} completion={:?} first_targets={} omitted_targets={} channel_wire_bytes=unobserved coverage=incomplete",
            phase.as_str(), date, Sha256::digest(&envelope.report_input), attempted_targets,
            report.accepted_count(), report.unknown_count(),
            report.completion(), visible_targets, attempted_targets.saturating_sub(16)
        );
        if report.attempts().iter().any(|attempt| {
            attempt.channel() == stock_analysis::notification::NotificationChannel::Wechat
        }) {
            match envelope.wechat_http_body.as_ref() {
                Some(body) => log::info!(
                    "[chain_shadow_wechat_body] phase={} schedule_date={} report_input_sha256={:x} scope=wechat_http_entity_body built_requests={} built_body_bytes={} sequence_sha256={} full_http_wire=unobserved non_wechat_feishu_channels=unobserved coverage=incomplete",
                    phase.as_str(), date, Sha256::digest(&envelope.report_input),
                    body.request_count(), body.total_body_bytes(), body.sequence_sha256()
                ),
                None => log::warn!(
                    "[chain_shadow_wechat_body] phase={} schedule_date={} scope=wechat_http_entity_body status=unobserved reason=no_built_or_readable_request_body coverage=incomplete",
                    phase.as_str(), date
                ),
            }
        }
        if report.attempts().iter().any(|attempt| {
            attempt.channel() == stock_analysis::notification::NotificationChannel::Feishu
        }) {
            match envelope.feishu_http_body.as_ref() {
                Some(body) => log::info!(
                    "[chain_shadow_feishu_body] phase={} schedule_date={} report_input_sha256={:x} scope=feishu_http_entity_body built_requests={} built_body_bytes={} sequence_sha256={} headers=unobserved framing=unobserved tls=unobserved non_wechat_feishu_channels=unobserved coverage=incomplete",
                    phase.as_str(), date, Sha256::digest(&envelope.report_input),
                    body.request_count(), body.total_body_bytes(), body.sequence_sha256()
                ),
                None => log::warn!(
                    "[chain_shadow_feishu_body] phase={} schedule_date={} scope=feishu_http_entity_body status=unobserved reason=no_built_or_readable_request_body coverage=incomplete",
                    phase.as_str(), date
                ),
            }
        }
    }
    if envelope.send_attempted {
        match observer(
            phase,
            date,
            &envelope.prepared,
            &envelope.report_input,
            envelope.acquisition.as_ref(),
        ) {
            Ok(observation) => log::info!(
                "[chain_shadow_input] phase={} schedule_date={} prepared_business_date={} artifact_sha256={} artifact_bytes={} report_input_sha256={} report_input_bytes={} prepared_report_equals_input={} acquisition_sha256={} acquisition_report_binding_sha256={} coverage={} covered_inputs={} foundation_persisted=false",
                observation.phase.as_str(), observation.schedule_date, observation.prepared_business_date,
                observation.artifact_sha256, observation.artifact_bytes, observation.report_input_sha256,
                observation.report_input_bytes, observation.prepared_report_equals_input,
                observation.acquisition_sha256.as_deref().unwrap_or("absent"),
                observation.acquisition_report_binding_sha256.as_deref().unwrap_or("absent"),
                observation.coverage, observation.covered_inputs,
            ),
            Err(_error) => log::warn!(
                "[chain_shadow_input] phase={} schedule_date={} coverage={} covered_inputs=unknown foundation_persisted=false observer_status=incomplete reason=source_or_artifact_observation_failed",
                phase.as_str(), date, chain_shadow_input::COVERAGE,
            ),
        }
    }
    legacy_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_window_is_recorded_once_without_authorizing_a_late_send() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chain.sqlite3");
        let store = ChainScheduleStore::new(&path);
        let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        let in_window = date.and_hms_opt(15, 34, 59).unwrap();
        let overdue = DateTime::parse_from_rfc3339("2026-09-24T15:35:00+08:00").unwrap();
        assert!(ChainPhase::Postclose.starts_in_window(date, in_window));
        assert!(!ChainPhase::Postclose.starts_in_window(date, overdue.naive_local()));
        assert_eq!(
            store
                .record_missed_window(
                    ChainPhase::Postclose,
                    date,
                    DateTime::parse_from_rfc3339("2026-09-24T15:34:59+08:00").unwrap(),
                )
                .unwrap(),
            None
        );
        assert!(!path.exists());

        let first = store
            .record_missed_window(ChainPhase::Postclose, date, overdue)
            .unwrap()
            .unwrap();
        assert_eq!(first.detected_at, "2026-09-24T15:35:00+08:00");
        assert_eq!(first.latest_attempt_no, None);
        assert_eq!(
            ChainScheduleStore::new(&path)
                .record_missed_window(
                    ChainPhase::Postclose,
                    date,
                    DateTime::parse_from_rfc3339("2026-09-24T22:25:00+08:00").unwrap(),
                )
                .unwrap(),
            None
        );
        assert_eq!(
            store.inspect_miss(ChainPhase::Postclose, date).unwrap(),
            Some(first)
        );
        assert_eq!(
            store.status(ChainPhase::Postclose, date).unwrap(),
            ChainScheduleStatus::Ready
        );
        assert!(!ChainPhase::Postclose.starts_in_window(date, date.and_hms_opt(22, 25, 0).unwrap()));
        assert_eq!(
            store
                .record_missed_window(
                    ChainPhase::Postclose,
                    date,
                    DateTime::parse_from_rfc3339("2026-09-25T00:00:00+08:00").unwrap(),
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn uncertain_or_closed_send_is_not_misclassified_as_missed() {
        let directory = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(directory.path().join("chain.sqlite3"));
        let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        let late = DateTime::parse_from_rfc3339("2026-09-24T16:00:00+08:00").unwrap();
        store
            .begin_send(ChainPhase::Postclose, date, "reports/postclose.md")
            .unwrap();
        assert_eq!(
            store
                .record_missed_window(ChainPhase::Postclose, date, late)
                .unwrap(),
            None
        );
        store
            .resolve_uncertain(
                ChainPhase::Postclose,
                date,
                ManualResolution::Delivered,
                "渠道日志已核对",
            )
            .unwrap();
        assert_eq!(
            store
                .record_missed_window(ChainPhase::Postclose, date, late)
                .unwrap(),
            None
        );
        assert_eq!(
            store.inspect_miss(ChainPhase::Postclose, date).unwrap(),
            None
        );

        let preopen_end = DateTime::parse_from_rfc3339("2026-09-24T09:15:00+08:00").unwrap();
        assert!(!ChainPhase::Preopen.starts_in_window(date, preopen_end.naive_local()));
        let preopen_miss = store
            .record_missed_window(ChainPhase::Preopen, date, preopen_end)
            .unwrap()
            .unwrap();
        assert_eq!(preopen_miss.latest_attempt_no, None);
    }

    #[test]
    fn sending_attempt_survives_restart_and_requires_manual_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chain.sqlite3");
        let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        let first = ChainScheduleStore::new(&path);
        assert!(first.inspect(ChainPhase::Preopen, date).is_err());
        assert!(!path.exists());
        assert_eq!(
            first.status(ChainPhase::Preopen, date).unwrap(),
            ChainScheduleStatus::Ready
        );
        assert_eq!(
            first
                .begin_send(ChainPhase::Preopen, date, "reports/preopen.md")
                .unwrap(),
            1
        );
        drop(first);

        let restarted = ChainScheduleStore::new(&path);
        let (status, evidence) = restarted.inspect(ChainPhase::Preopen, date).unwrap();
        assert_eq!(status, ChainScheduleStatus::Uncertain);
        assert_eq!(evidence.unwrap().report_path, "reports/preopen.md");
        assert_eq!(
            restarted.status(ChainPhase::Preopen, date).unwrap(),
            ChainScheduleStatus::Uncertain
        );
        assert!(restarted
            .begin_send(ChainPhase::Preopen, date, "reports/preopen.md")
            .is_err());
        assert_eq!(
            restarted.status(ChainPhase::Postclose, date).unwrap(),
            ChainScheduleStatus::Ready
        );
        restarted
            .resolve_uncertain(
                ChainPhase::Preopen,
                date,
                ManualResolution::Retry,
                "核对渠道日志：未接收",
            )
            .unwrap();
        assert_eq!(
            restarted
                .begin_send(ChainPhase::Preopen, date, "reports/preopen.md")
                .unwrap(),
            2
        );
        restarted
            .mark_weak_accepted(ChainPhase::Preopen, date, 2)
            .unwrap();
        assert_eq!(
            ChainScheduleStore::new(&path)
                .status(ChainPhase::Preopen, date)
                .unwrap(),
            ChainScheduleStatus::Closed
        );
        assert!(restarted
            .begin_send(ChainPhase::Preopen, date, "reports/preopen.md")
            .is_err());
    }

    #[test]
    fn manual_delivered_closes_only_the_selected_phase() {
        let directory = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(directory.path().join("chain.sqlite3"));
        let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        store
            .begin_send(ChainPhase::Postclose, date, "reports/postclose.md")
            .unwrap();
        assert!(store
            .resolve_uncertain(
                ChainPhase::Postclose,
                date,
                ManualResolution::Delivered,
                " "
            )
            .is_err());
        store
            .resolve_uncertain(
                ChainPhase::Postclose,
                date,
                ManualResolution::Delivered,
                "渠道回执已核对",
            )
            .unwrap();
        assert_eq!(
            store.status(ChainPhase::Postclose, date).unwrap(),
            ChainScheduleStatus::Closed
        );
        assert_eq!(
            store.status(ChainPhase::Preopen, date).unwrap(),
            ChainScheduleStatus::Ready
        );
    }

    #[test]
    fn manual_retry_keeps_the_first_attempt_report() {
        let directory = tempfile::tempdir().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        let first_at = DateTime::parse_from_rfc3339("2026-09-24T07:30:00.000000001Z")
            .unwrap()
            .with_timezone(&Utc);
        let retry_at = DateTime::parse_from_rfc3339("2026-09-24T07:30:00.000000002Z")
            .unwrap()
            .with_timezone(&Utc);
        let first = directory.path().join(scheduled_report_filename(
            ChainPhase::Postclose,
            date,
            first_at,
        ));
        let retry = directory.path().join(scheduled_report_filename(
            ChainPhase::Postclose,
            date,
            retry_at,
        ));
        std::fs::write(&first, "first attempt").unwrap();
        std::fs::write(&retry, "manual retry").unwrap();
        assert_ne!(first, retry);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "first attempt");
    }
}
