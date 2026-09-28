//! Read-only policy comparison for the legacy scheduled chain reports.
//!
//! The legacy schedule store remains the only production owner. This module
//! does not persist a Foundation occurrence or invoke a producer/finalizer.

use std::io::ErrorKind;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone};

use crate::app::chain_schedule::{ChainPhase, ChainScheduleStatus, ChainScheduleStore};
use crate::monitor::push_job::{
    BusinessDate, CalendarId, MachineCatalog, Namespace, OccurrenceIdentityMaterial, OccurrenceKey,
    PhaseEpic, ProducerId, ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId,
    SourceContractId, UtcMicros,
};

use super::phase_scheduler::{
    CatchUpPolicy, MarketObservation, PhaseSchedule, PhaseScheduler, ScheduleStatus, ScheduleStep,
    ScheduleWindow, WindowPosition,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainPreopenShadowReport {
    pub observed_at: DateTime<FixedOffset>,
    pub calendar_date: NaiveDate,
    pub legacy_occurrence_key: String,
    pub foundation_occurrence_id: Option<String>,
    pub legacy_status: ChainScheduleStatus,
    pub legacy_miss_recorded: bool,
    pub foundation_status: &'static str,
    pub foundation_reason: &'static str,
    pub legacy_window_open: bool,
    pub foundation_window_open: bool,
    pub legacy_due: bool,
    pub foundation_due: bool,
    pub legacy_miss_due: bool,
    pub foundation_miss_due: bool,
    pub legacy_closed: bool,
    pub foundation_closed: bool,
}

impl ChainPreopenShadowReport {
    pub fn has_decision_diff(&self) -> bool {
        self.legacy_window_open != self.foundation_window_open
            || self.legacy_due != self.foundation_due
            || self.legacy_miss_due != self.foundation_miss_due
            || self.legacy_closed != self.foundation_closed
    }

    /// Compare only the send gate. The caller may have captured the legacy
    /// state before the physical sender changed it.
    pub fn has_send_gate_diff(&self) -> bool {
        self.legacy_due != self.foundation_due
    }
}

/// Reads the old store and evaluates the new scheduler against the same clock.
/// A missing old database means the legacy policy has no attempt or miss yet.
/// Existing databases are opened read-only; any other inspection error is
/// returned without affecting the legacy producer.
pub fn observe_chain_preopen_shadow(
    store: &ChainScheduleStore,
    observed_at: DateTime<FixedOffset>,
    trading_day: bool,
) -> Result<ChainPreopenShadowReport> {
    observe_chain_schedule_shadow(store, observed_at, trading_day, ChainPhase::Preopen)
}

pub(super) fn observe_chain_schedule_shadow(
    store: &ChainScheduleStore,
    observed_at: DateTime<FixedOffset>,
    trading_day: bool,
    phase: ChainPhase,
) -> Result<ChainPreopenShadowReport> {
    let calendar_date = observed_at.date_naive();
    let (legacy_status, legacy_miss_recorded) = match std::fs::metadata(store.path()) {
        Ok(_) => {
            let (status, _) = store.inspect(phase, calendar_date)?;
            let miss = store.inspect_miss(phase, calendar_date)?.is_some();
            (status, miss)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => (ChainScheduleStatus::Ready, false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("inspect chain schedule path {}", store.path().display())
            });
        }
    };

    project_chain_schedule_snapshot(
        phase,
        calendar_date,
        observed_at,
        trading_day,
        legacy_status,
        legacy_miss_recorded,
    )
}

/// Pure diagnostic projection over the legacy status and clock supplied by a
/// caller's gate. Foundation starts from no occurrence; this is not admission,
/// a completion decision, or full shadow parity. No schedule-store read or
/// Foundation persistence occurs.
pub fn project_chain_schedule_snapshot(
    phase: ChainPhase,
    calendar_date: NaiveDate,
    observed_at: DateTime<FixedOffset>,
    trading_day: bool,
    legacy_status: ChainScheduleStatus,
    legacy_miss_recorded: bool,
) -> Result<ChainPreopenShadowReport> {
    let catalog = MachineCatalog::bundled()?;
    let (
        producer_name,
        schedule_name,
        occurrence_name,
        epic,
        start_hour,
        start_minute,
        end_hour,
        end_minute,
    ) = match phase {
        ChainPhase::Preopen => (
            "chain-preopen-timer",
            "chain-preopen-0905",
            "chain-preopen",
            PhaseEpic::Preopen,
            9,
            5,
            9,
            15,
        ),
        ChainPhase::Postclose => (
            "chain-post-close-timer",
            "chain-post-close-1530",
            "chain-post-close",
            PhaseEpic::Postclose,
            15,
            30,
            15,
            35,
        ),
    };
    let producer_id = ProducerId::try_new(producer_name.to_owned())?;
    let producer = catalog
        .producer(&producer_id)
        .with_context(|| format!("{producer_name} producer missing from bundled catalog"))?;
    let business_date = BusinessDate::parse(&calendar_date.to_string())?;
    let identity = ScheduleOccurrenceIdentityMaterial::new(
        Namespace::Production,
        producer.unit_id().clone(),
        producer_id,
        ScheduleOrTriggerId::try_new(schedule_name.to_owned())?,
        CalendarId::try_new("a-share-trading-calendar-v1".to_owned())?,
        OccurrenceIdentityMaterial::new(
            business_date.clone(),
            producer.occurrence_family().clone(),
            OccurrenceKey::try_new(format!("{occurrence_name}:{calendar_date}"))?,
        ),
        producer.completion_owner().clone(),
        SourceContractId::try_new("legacy-chain-report-v1".to_owned())?,
    );

    let offset = observed_at.offset();
    let start = offset
        .from_local_datetime(
            &calendar_date
                .and_hms_opt(start_hour, start_minute, 0)
                .context("chain window start time")?,
        )
        .single()
        .context("ambiguous chain window start")?;
    let end = offset
        .from_local_datetime(
            &calendar_date
                .and_hms_opt(end_hour, end_minute, 0)
                .context("chain window end time")?,
        )
        .single()
        .context("ambiguous chain window end")?;
    let window = ScheduleWindow::try_new(
        UtcMicros::try_new(start.timestamp_micros())?,
        UtcMicros::try_new(end.timestamp_micros())?,
    )?;
    let schedule = PhaseSchedule::try_bind(
        &catalog,
        identity,
        epic,
        window,
        CatchUpPolicy::ExpireWithoutCatchUp,
    )?;
    let at = UtcMicros::try_new(observed_at.timestamp_micros())?;
    let observation = if trading_day {
        MarketObservation::trading_day(business_date, at)
    } else {
        MarketObservation::non_trading_day(business_date, at)
    };
    let (foundation_status, foundation_reason, foundation_occurrence_id) =
        match PhaseScheduler::tick(&schedule, None, &observation)? {
            ScheduleStep::NoOccurrence { reason } => (None, reason.as_str(), None),
            ScheduleStep::CreateExpected(created) => {
                let next = match PhaseScheduler::tick(&schedule, Some(&created), &observation)? {
                    ScheduleStep::TransitionProposal(proposal) => {
                        created.apply_proposal(&proposal)?
                    }
                    ScheduleStep::NoChange { reason, .. } => {
                        return Ok(build_report(
                            observed_at,
                            phase,
                            calendar_date,
                            legacy_status,
                            legacy_miss_recorded,
                            trading_day,
                            window,
                            Some(created.status()),
                            reason.as_str(),
                            Some(schedule.occurrence_id().as_str().to_owned()),
                        ));
                    }
                    other => bail!("unexpected chain shadow scheduler step: {other:?}"),
                };
                (
                    Some(next.status()),
                    next.reason().as_str(),
                    Some(schedule.occurrence_id().as_str().to_owned()),
                )
            }
            other => bail!("unexpected chain shadow scheduler step: {other:?}"),
        };

    Ok(build_report(
        observed_at,
        phase,
        calendar_date,
        legacy_status,
        legacy_miss_recorded,
        trading_day,
        window,
        foundation_status,
        foundation_reason,
        foundation_occurrence_id,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    observed_at: DateTime<FixedOffset>,
    phase: ChainPhase,
    calendar_date: NaiveDate,
    legacy_status: ChainScheduleStatus,
    legacy_miss_recorded: bool,
    trading_day: bool,
    window: ScheduleWindow,
    foundation_status: Option<ScheduleStatus>,
    foundation_reason: &'static str,
    foundation_occurrence_id: Option<String>,
) -> ChainPreopenShadowReport {
    let local_time = observed_at.naive_local();
    let legacy_window_open = phase.starts_in_window(calendar_date, local_time);
    let foundation_window_open = window.position(
        UtcMicros::try_new(observed_at.timestamp_micros())
            .expect("validated observation timestamp"),
    ) == WindowPosition::Open;
    let legacy_ready = legacy_status == ChainScheduleStatus::Ready;
    ChainPreopenShadowReport {
        observed_at,
        calendar_date,
        legacy_occurrence_key: format!("{}:{calendar_date}", phase.as_str()),
        foundation_occurrence_id,
        legacy_status,
        legacy_miss_recorded,
        foundation_status: match foundation_status {
            None => "no_occurrence",
            Some(ScheduleStatus::Expected) => "expected",
            Some(ScheduleStatus::Eligible) => "eligible",
            Some(ScheduleStatus::Prepared) => "prepared",
            Some(ScheduleStatus::Closed) => "closed",
            Some(ScheduleStatus::Missed) => "missed",
            Some(ScheduleStatus::Deferred) => "deferred",
            Some(ScheduleStatus::BlockedOnInput) => "blocked_on_input",
        },
        foundation_reason,
        legacy_window_open,
        foundation_window_open,
        legacy_due: trading_day && legacy_ready && legacy_window_open,
        foundation_due: foundation_status == Some(ScheduleStatus::Eligible),
        legacy_miss_due: trading_day
            && legacy_ready
            && !legacy_miss_recorded
            && phase.is_overdue(calendar_date, local_time),
        foundation_miss_due: foundation_status == Some(ScheduleStatus::Missed),
        legacy_closed: legacy_status == ChainScheduleStatus::Closed,
        foundation_closed: foundation_status == Some(ScheduleStatus::Closed),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveDate, TimeZone};

    use crate::app::chain_schedule::{ChainPhase, ChainScheduleStore};

    use super::observe_chain_preopen_shadow;

    fn at(day: u32, hour: u32, minute: u32) -> chrono::DateTime<FixedOffset> {
        let offset = FixedOffset::east_opt(8 * 3600).unwrap();
        offset
            .with_ymd_and_hms(2026, 9, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn chain_preopen_shadow_boundary_and_identity_are_read_only() {
        let root = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(root.path().join("never-created.sqlite3"));
        let before = observe_chain_preopen_shadow(&store, at(28, 9, 4), true).unwrap();
        let open = observe_chain_preopen_shadow(&store, at(28, 9, 5), true).unwrap();
        let last = observe_chain_preopen_shadow(&store, at(28, 9, 14), true).unwrap();
        let expired = observe_chain_preopen_shadow(&store, at(28, 9, 15), true).unwrap();
        let next_day = observe_chain_preopen_shadow(&store, at(29, 9, 5), true).unwrap();
        assert!(!store.path().exists());
        assert_eq!(before.foundation_status, "expected");
        assert!(!before.legacy_window_open);
        assert!(!before.has_decision_diff());
        assert_eq!(open.foundation_status, "eligible");
        assert!(open.legacy_due && open.foundation_due);
        assert_eq!(open.foundation_occurrence_id, last.foundation_occurrence_id);
        assert_ne!(
            open.foundation_occurrence_id,
            next_day.foundation_occurrence_id
        );
        assert_eq!(open.legacy_occurrence_key, "preopen:2026-09-28");
        assert!(!last.has_decision_diff());
        assert_eq!(expired.foundation_status, "missed");
        assert!(expired.legacy_miss_due && expired.foundation_miss_due);
        assert!(!expired.has_decision_diff());
    }

    #[test]
    fn chain_preopen_shadow_exposes_legacy_closed_without_foundation_receipt() {
        let root = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(root.path().join("chain.sqlite3"));
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let attempt = store
            .begin_send(ChainPhase::Preopen, date, "reports/preopen.md")
            .unwrap();
        let uncertain = observe_chain_preopen_shadow(&store, at(28, 9, 6), true).unwrap();
        assert!(!uncertain.legacy_due && uncertain.foundation_due);
        assert!(!uncertain.legacy_closed && !uncertain.foundation_closed);
        assert!(uncertain.has_decision_diff());
        store
            .mark_weak_accepted(ChainPhase::Preopen, date, attempt)
            .unwrap();
        let report = observe_chain_preopen_shadow(&store, at(28, 9, 6), true).unwrap();
        assert!(report.legacy_closed);
        assert!(!report.foundation_closed);
        assert!(!report.legacy_due);
        assert!(report.foundation_due);
        assert!(report.has_decision_diff());
    }

    #[test]
    fn chain_preopen_shadow_nontrading_and_persisted_miss_are_visible() {
        let root = tempfile::tempdir().unwrap();
        let store = ChainScheduleStore::new(root.path().join("chain.sqlite3"));
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        store
            .record_missed_window(ChainPhase::Preopen, date, at(28, 9, 15))
            .unwrap();
        let missed = observe_chain_preopen_shadow(&store, at(28, 9, 15), true).unwrap();
        assert!(missed.legacy_miss_recorded);
        assert!(!missed.legacy_miss_due);
        assert!(missed.foundation_miss_due);
        assert!(missed.has_decision_diff());

        let closed_day = observe_chain_preopen_shadow(&store, at(27, 9, 5), false).unwrap();
        assert_eq!(closed_day.foundation_status, "no_occurrence");
        assert!(closed_day.foundation_occurrence_id.is_none());
        assert!(!closed_day.legacy_due && !closed_day.foundation_due);
        assert!(!closed_day.has_decision_diff());
    }
}
