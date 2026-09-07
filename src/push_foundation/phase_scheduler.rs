//! W14 deterministic phase-schedule evaluation. This module has no timer or I/O wiring.

#![cfg_attr(not(test), allow(dead_code))]

use crate::monitor::push_job::{
    derive_schedule_occurrence_id, BusinessDate, MachineCatalog, PhaseEpic, ReasonCode,
    ScheduleOccurrenceId, ScheduleOccurrenceIdentityMaterial, UtcMicros,
};

use super::reconciler::StartupRecoveryBarrier;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CatchUpPolicy {
    ExpireWithoutCatchUp,
    SameBusinessDayBeforeDeadline,
    DeferToNextEligibleSession,
    RecoverPersistedOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScheduleStatus {
    Expected,
    Eligible,
    Prepared,
    Closed,
    Missed,
    Deferred,
    BlockedOnInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowPosition {
    Before,
    Open,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScheduleWindow {
    start: UtcMicros,
    end: UtcMicros,
}

impl ScheduleWindow {
    pub(crate) fn try_new(start: UtcMicros, end: UtcMicros) -> Result<Self, PhaseSchedulerError> {
        if end <= start {
            return Err(PhaseSchedulerError::InvalidWindow);
        }
        Ok(Self { start, end })
    }

    pub(crate) fn start(self) -> UtcMicros {
        self.start
    }

    pub(crate) fn end(self) -> UtcMicros {
        self.end
    }

    pub(crate) fn position(self, observed_at: UtcMicros) -> WindowPosition {
        if observed_at < self.start {
            WindowPosition::Before
        } else if observed_at < self.end {
            WindowPosition::Open
        } else {
            WindowPosition::Expired
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PhaseSchedule {
    identity: ScheduleOccurrenceIdentityMaterial,
    occurrence_id: ScheduleOccurrenceId,
    phase: PhaseEpic,
    window: ScheduleWindow,
    catch_up_policy: CatchUpPolicy,
}

impl PhaseSchedule {
    pub(crate) fn try_bind(
        catalog: &MachineCatalog,
        identity: ScheduleOccurrenceIdentityMaterial,
        phase: PhaseEpic,
        window: ScheduleWindow,
        catch_up_policy: CatchUpPolicy,
    ) -> Result<Self, PhaseSchedulerError> {
        let producer = catalog
            .producer(identity.producer_id())
            .ok_or(PhaseSchedulerError::CatalogProducerMissing)?;
        if producer.unit_id() != identity.unit_id() {
            return Err(PhaseSchedulerError::CatalogMismatch { field: "unit_id" });
        }
        if producer.completion_owner() != identity.completion_owner() {
            return Err(PhaseSchedulerError::CatalogMismatch {
                field: "completion_owner",
            });
        }
        if producer.occurrence_family() != identity.occurrence().occurrence_family() {
            return Err(PhaseSchedulerError::CatalogMismatch {
                field: "occurrence_family",
            });
        }
        if !producer.phase_epics().contains(&phase) {
            return Err(PhaseSchedulerError::CatalogMismatch { field: "phase" });
        }
        let occurrence_id = derive_schedule_occurrence_id(&identity);
        Ok(Self {
            identity,
            occurrence_id,
            phase,
            window,
            catch_up_policy,
        })
    }

    pub(crate) fn occurrence_id(&self) -> &ScheduleOccurrenceId {
        &self.occurrence_id
    }

    pub(crate) fn business_date(&self) -> &BusinessDate {
        self.identity.occurrence().business_date()
    }

    pub(crate) fn phase(&self) -> PhaseEpic {
        self.phase
    }

    pub(crate) fn window(&self) -> ScheduleWindow {
        self.window
    }

    pub(crate) fn catch_up_policy(&self) -> CatchUpPolicy {
        self.catch_up_policy
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarketObservation {
    business_date: BusinessDate,
    observed_at: UtcMicros,
    trading_day: bool,
}

impl MarketObservation {
    pub(crate) fn trading_day(business_date: BusinessDate, observed_at: UtcMicros) -> Self {
        Self {
            business_date,
            observed_at,
            trading_day: true,
        }
    }

    pub(crate) fn non_trading_day(business_date: BusinessDate, observed_at: UtcMicros) -> Self {
        Self {
            business_date,
            observed_at,
            trading_day: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScheduleOccurrenceSnapshot {
    schedule: PhaseSchedule,
    status: ScheduleStatus,
    version: u64,
    reason: ReasonCode,
    created_at: UtcMicros,
    updated_at: UtcMicros,
}

impl ScheduleOccurrenceSnapshot {
    fn expected(schedule: &PhaseSchedule, reason: ReasonCode, observed_at: UtcMicros) -> Self {
        Self {
            schedule: schedule.clone(),
            status: ScheduleStatus::Expected,
            version: 0,
            reason,
            created_at: observed_at,
            updated_at: observed_at,
        }
    }

    pub(crate) fn occurrence_id(&self) -> &ScheduleOccurrenceId {
        &self.schedule.occurrence_id
    }

    pub(crate) fn business_date(&self) -> &BusinessDate {
        self.schedule.business_date()
    }

    pub(crate) fn status(&self) -> ScheduleStatus {
        self.status
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn reason(&self) -> ReasonCode {
        self.reason
    }

    pub(crate) fn created_at(&self) -> UtcMicros {
        self.created_at
    }

    pub(crate) fn updated_at(&self) -> UtcMicros {
        self.updated_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScheduleTransitionProposal {
    occurrence_id: ScheduleOccurrenceId,
    from_status: ScheduleStatus,
    to_status: ScheduleStatus,
    expected_version: u64,
    result_version: u64,
    reason: ReasonCode,
    observed_at: UtcMicros,
}

impl ScheduleTransitionProposal {
    fn try_new(
        current: &ScheduleOccurrenceSnapshot,
        to_status: ScheduleStatus,
        reason: ReasonCode,
        observed_at: UtcMicros,
    ) -> Result<Self, PhaseSchedulerError> {
        let result_version = current
            .version
            .checked_add(1)
            .ok_or(PhaseSchedulerError::VersionOverflow)?;
        Ok(Self {
            occurrence_id: current.schedule.occurrence_id.clone(),
            from_status: current.status,
            to_status,
            expected_version: current.version,
            result_version,
            reason,
            observed_at,
        })
    }

    pub(crate) fn occurrence_id(&self) -> &ScheduleOccurrenceId {
        &self.occurrence_id
    }

    pub(crate) fn from_status(&self) -> ScheduleStatus {
        self.from_status
    }

    pub(crate) fn to_status(&self) -> ScheduleStatus {
        self.to_status
    }

    pub(crate) fn expected_version(&self) -> u64 {
        self.expected_version
    }

    pub(crate) fn result_version(&self) -> u64 {
        self.result_version
    }

    pub(crate) fn reason(&self) -> ReasonCode {
        self.reason
    }

    pub(crate) fn observed_at(&self) -> UtcMicros {
        self.observed_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScheduleStep {
    NoOccurrence {
        reason: ReasonCode,
    },
    CreateExpected(ScheduleOccurrenceSnapshot),
    RecoveryOnly {
        occurrence_id: ScheduleOccurrenceId,
    },
    TransitionProposal(ScheduleTransitionProposal),
    NoChange {
        occurrence_id: ScheduleOccurrenceId,
        status: ScheduleStatus,
        version: u64,
        reason: ReasonCode,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PhaseSchedulerError {
    #[error("schedule window must be a non-empty half-open interval")]
    InvalidWindow,
    #[error("schedule producer is not present in the machine catalog")]
    CatalogProducerMissing,
    #[error("schedule binding does not match machine catalog field {field}")]
    CatalogMismatch { field: &'static str },
    #[error("market observation business date does not match the schedule")]
    BusinessDateMismatch,
    #[error("current occurrence does not match the registered schedule")]
    OccurrenceBindingMismatch,
    #[error("schedule occurrence version overflow")]
    VersionOverflow,
}

pub(crate) struct PhaseScheduler;

impl PhaseScheduler {
    pub(crate) fn tick(
        schedule: &PhaseSchedule,
        current: Option<&ScheduleOccurrenceSnapshot>,
        observation: &MarketObservation,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        Self::evaluate(schedule, current, observation)
    }

    pub(crate) fn startup_catch_up(
        _recovery_barrier: &StartupRecoveryBarrier,
        schedule: &PhaseSchedule,
        current: Option<&ScheduleOccurrenceSnapshot>,
        observation: &MarketObservation,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        Self::evaluate(schedule, current, observation)
    }

    fn evaluate(
        schedule: &PhaseSchedule,
        current: Option<&ScheduleOccurrenceSnapshot>,
        observation: &MarketObservation,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        if &observation.business_date != schedule.business_date() {
            return Err(PhaseSchedulerError::BusinessDateMismatch);
        }
        if let Some(current) = current {
            if &current.schedule != schedule {
                return Err(PhaseSchedulerError::OccurrenceBindingMismatch);
            }
            return Self::evaluate_current(current, observation);
        }
        if !observation.trading_day {
            return Ok(ScheduleStep::NoOccurrence {
                reason: ReasonCode::ScheduleNotTradingDay,
            });
        }
        if schedule.catch_up_policy == CatchUpPolicy::RecoverPersistedOnly {
            return Ok(ScheduleStep::RecoveryOnly {
                occurrence_id: schedule.occurrence_id.clone(),
            });
        }
        let reason = match schedule.window.position(observation.observed_at) {
            WindowPosition::Before => ReasonCode::ScheduleWindowNotOpen,
            WindowPosition::Open => ReasonCode::ScheduleWindowOpen,
            WindowPosition::Expired => ReasonCode::ScheduleWindowExpired,
        };
        Ok(ScheduleStep::CreateExpected(
            ScheduleOccurrenceSnapshot::expected(schedule, reason, observation.observed_at),
        ))
    }

    fn evaluate_current(
        current: &ScheduleOccurrenceSnapshot,
        observation: &MarketObservation,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        let position = current.schedule.window.position(observation.observed_at);
        match (current.status, position) {
            (ScheduleStatus::Expected, WindowPosition::Open) => Self::propose(
                current,
                ScheduleStatus::Eligible,
                ReasonCode::ScheduleWindowOpen,
                observation.observed_at,
            ),
            (ScheduleStatus::Expected, WindowPosition::Expired)
                if matches!(
                    current.schedule.catch_up_policy,
                    CatchUpPolicy::ExpireWithoutCatchUp
                        | CatchUpPolicy::SameBusinessDayBeforeDeadline
                ) =>
            {
                Self::propose(
                    current,
                    ScheduleStatus::Missed,
                    ReasonCode::ScheduleWindowExpired,
                    observation.observed_at,
                )
            }
            _ => Ok(Self::no_change(
                current,
                match position {
                    WindowPosition::Before => ReasonCode::ScheduleWindowNotOpen,
                    WindowPosition::Open => current.reason,
                    WindowPosition::Expired => current.reason,
                },
            )),
        }
    }

    fn propose(
        current: &ScheduleOccurrenceSnapshot,
        to_status: ScheduleStatus,
        reason: ReasonCode,
        observed_at: UtcMicros,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        ScheduleTransitionProposal::try_new(current, to_status, reason, observed_at)
            .map(ScheduleStep::TransitionProposal)
    }

    fn no_change(current: &ScheduleOccurrenceSnapshot, reason: ReasonCode) -> ScheduleStep {
        ScheduleStep::NoChange {
            occurrence_id: current.schedule.occurrence_id.clone(),
            status: current.status,
            version: current.version,
            reason,
        }
    }
}
