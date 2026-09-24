//! Cross-restart admission for the two scheduled chain reports.
//!
//! The legacy notification channels return only weak acceptance. A recorded
//! send attempt therefore blocks automatic replay until an operator resolves
//! it; a crash or an error after the send boundary cannot be treated as a
//! definite rejection.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use super::modes::run_chain_analysis_mode_with_send_guard;

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

impl ChainScheduleStore {
    pub fn production() -> Self {
        Self::new(crate::production_root::production_root().join("data/chain_schedule.sqlite3"))
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
             );",
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
        ChainScheduleStatus::Closed => return Ok(ChainScheduleOutcome::AlreadyClosed),
        ChainScheduleStatus::Uncertain => return Ok(ChainScheduleOutcome::NeedsReview),
        ChainScheduleStatus::Ready => {}
    }

    let filename = scheduled_report_filename(phase, date, Utc::now());
    let mut attempt_no = None;
    run_chain_analysis_mode_with_send_guard(true, Some(&filename), |report_path| {
        attempt_no = Some(store.begin_send(phase, date, report_path)?);
        Ok(())
    })
    .await?;
    let attempt_no = attempt_no.context("产业链发送守卫未执行")?;
    store.mark_weak_accepted(phase, date, attempt_no)?;
    Ok(ChainScheduleOutcome::WeakAccepted)
}

#[cfg(test)]
mod tests {
    use super::*;

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
