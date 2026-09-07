//! W14 deterministic phase-schedule evaluation. This module has no timer or I/O wiring.

#![cfg_attr(not(test), allow(dead_code))]

use crate::monitor::push_job::{
    derive_schedule_occurrence_id, BusinessDate, CompletionDirective, MachineCatalog, PhaseEpic,
    ReasonCode, ScheduleDirective, ScheduleOccurrenceId, ScheduleOccurrenceIdentityMaterial,
    UtcMicros,
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
pub(crate) struct NextEligibleSessionRef {
    occurrence_id: ScheduleOccurrenceId,
    business_date: BusinessDate,
    window: ScheduleWindow,
}

impl NextEligibleSessionRef {
    pub(crate) fn try_new(
        schedule: &PhaseSchedule,
        business_date: BusinessDate,
        window: ScheduleWindow,
    ) -> Result<Self, PhaseSchedulerError> {
        if &business_date <= schedule.business_date() || window.start < schedule.window.end {
            return Err(PhaseSchedulerError::InvalidNextEligibleSession);
        }
        Ok(Self {
            occurrence_id: schedule.occurrence_id.clone(),
            business_date,
            window,
        })
    }

    pub(crate) fn business_date(&self) -> &BusinessDate {
        &self.business_date
    }

    pub(crate) fn window(&self) -> ScheduleWindow {
        self.window
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
    next_eligible: Option<NextEligibleSessionRef>,
}

impl MarketObservation {
    pub(crate) fn trading_day(business_date: BusinessDate, observed_at: UtcMicros) -> Self {
        Self {
            business_date,
            observed_at,
            trading_day: true,
            next_eligible: None,
        }
    }

    pub(crate) fn non_trading_day(business_date: BusinessDate, observed_at: UtcMicros) -> Self {
        Self {
            business_date,
            observed_at,
            trading_day: false,
            next_eligible: None,
        }
    }

    pub(crate) fn with_next_eligible(mut self, next: NextEligibleSessionRef) -> Self {
        self.next_eligible = Some(next);
        self
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
    next_eligible: Option<NextEligibleSessionRef>,
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
            next_eligible: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_hydrate(
        schedule: PhaseSchedule,
        status: ScheduleStatus,
        version: u64,
        reason: ReasonCode,
        created_at: UtcMicros,
        updated_at: UtcMicros,
        next_eligible: Option<NextEligibleSessionRef>,
    ) -> Result<Self, PhaseSchedulerError> {
        if updated_at < created_at {
            return Err(PhaseSchedulerError::InvalidSnapshot {
                check: "updated_at_before_created_at",
            });
        }
        match (status, next_eligible.as_ref()) {
            (ScheduleStatus::Deferred, None) => {
                return Err(PhaseSchedulerError::NextEligibleSessionRequired);
            }
            (ScheduleStatus::Deferred, Some(next))
                if next.occurrence_id != schedule.occurrence_id =>
            {
                return Err(PhaseSchedulerError::InvalidSnapshot {
                    check: "next_eligible_occurrence_mismatch",
                });
            }
            (ScheduleStatus::Deferred, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return Err(PhaseSchedulerError::InvalidSnapshot {
                    check: "next_eligible_on_non_deferred",
                });
            }
        }
        Ok(Self {
            schedule,
            status,
            version,
            reason,
            created_at,
            updated_at,
            next_eligible,
        })
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

    pub(crate) fn next_eligible(&self) -> Option<&NextEligibleSessionRef> {
        self.next_eligible.as_ref()
    }

    pub(crate) fn apply_proposal(
        &self,
        proposal: &ScheduleTransitionProposal,
    ) -> Result<Self, PhaseSchedulerError> {
        let expected_result_version = self
            .version
            .checked_add(1)
            .ok_or(PhaseSchedulerError::VersionOverflow)?;
        if proposal.occurrence_id != self.schedule.occurrence_id
            || proposal.from_status != self.status
            || proposal.expected_version != self.version
            || proposal.result_version != expected_result_version
            || proposal.observed_at < self.updated_at
        {
            return Err(PhaseSchedulerError::TransitionBindingMismatch);
        }
        match (proposal.to_status, proposal.next_eligible.as_ref()) {
            (ScheduleStatus::Deferred, None) => {
                return Err(PhaseSchedulerError::NextEligibleSessionRequired);
            }
            (ScheduleStatus::Deferred, Some(next))
                if next.occurrence_id != self.schedule.occurrence_id =>
            {
                return Err(PhaseSchedulerError::TransitionBindingMismatch);
            }
            (ScheduleStatus::Deferred, Some(_)) | (_, None) => {}
            (_, Some(_)) => return Err(PhaseSchedulerError::TransitionBindingMismatch),
        }
        Ok(Self {
            schedule: self.schedule.clone(),
            status: proposal.to_status,
            version: proposal.result_version,
            reason: proposal.reason,
            created_at: self.created_at,
            updated_at: proposal.observed_at,
            next_eligible: proposal.next_eligible.clone(),
        })
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
    next_eligible: Option<NextEligibleSessionRef>,
}

impl ScheduleTransitionProposal {
    fn try_new(
        current: &ScheduleOccurrenceSnapshot,
        to_status: ScheduleStatus,
        reason: ReasonCode,
        observed_at: UtcMicros,
        next_eligible: Option<NextEligibleSessionRef>,
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
            next_eligible,
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

    pub(crate) fn next_eligible(&self) -> Option<&NextEligibleSessionRef> {
        self.next_eligible.as_ref()
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
    #[error("next eligible session must be later and bound to the same occurrence")]
    InvalidNextEligibleSession,
    #[error("deferred schedule occurrence requires a next eligible session")]
    NextEligibleSessionRequired,
    #[error("invalid schedule occurrence snapshot: {check}")]
    InvalidSnapshot { check: &'static str },
    #[error("schedule transition proposal does not match the current occurrence")]
    TransitionBindingMismatch,
    #[error("schedule lifecycle transition is not allowed")]
    InvalidLifecycleTransition,
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

    pub(crate) fn completion(
        current: &ScheduleOccurrenceSnapshot,
        directive: CompletionDirective,
        observed_at: UtcMicros,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        if current.status == ScheduleStatus::Closed {
            return Ok(Self::no_change(
                current,
                ReasonCode::ScheduleOccurrenceClosed,
            ));
        }
        if directive.schedule() == ScheduleDirective::KeepOpen {
            return Ok(Self::no_change(current, current.reason));
        }
        if current.status != ScheduleStatus::Prepared {
            return Err(PhaseSchedulerError::InvalidLifecycleTransition);
        }
        Self::propose(
            current,
            ScheduleStatus::Closed,
            ReasonCode::ScheduleOccurrenceClosed,
            observed_at,
            None,
        )
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
        if current.status == ScheduleStatus::Closed {
            return Ok(Self::no_change(
                current,
                ReasonCode::ScheduleOccurrenceClosed,
            ));
        }
        if current.status == ScheduleStatus::Missed {
            return Ok(Self::no_change(current, ReasonCode::ScheduleWindowExpired));
        }
        if current.status == ScheduleStatus::Prepared {
            return Ok(Self::no_change(current, current.reason));
        }
        if current.status == ScheduleStatus::Deferred {
            let next = current
                .next_eligible
                .as_ref()
                .ok_or(PhaseSchedulerError::NextEligibleSessionRequired)?;
            return match next.window.position(observation.observed_at) {
                WindowPosition::Before => {
                    Ok(Self::no_change(current, ReasonCode::ScheduleWindowNotOpen))
                }
                WindowPosition::Open => Self::propose(
                    current,
                    ScheduleStatus::Eligible,
                    ReasonCode::ScheduleWindowOpen,
                    observation.observed_at,
                    None,
                ),
                WindowPosition::Expired => {
                    Ok(Self::no_change(current, ReasonCode::ScheduleDeferred))
                }
            };
        }

        let position = current.schedule.window.position(observation.observed_at);
        match (current.status, position) {
            (ScheduleStatus::Expected, WindowPosition::Open) => Self::propose(
                current,
                ScheduleStatus::Eligible,
                ReasonCode::ScheduleWindowOpen,
                observation.observed_at,
                None,
            ),
            (
                ScheduleStatus::Expected
                | ScheduleStatus::Eligible
                | ScheduleStatus::BlockedOnInput,
                WindowPosition::Expired,
            ) => Self::propose_expired(current, observation),
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
        next_eligible: Option<NextEligibleSessionRef>,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        ScheduleTransitionProposal::try_new(current, to_status, reason, observed_at, next_eligible)
            .map(ScheduleStep::TransitionProposal)
    }

    fn propose_expired(
        current: &ScheduleOccurrenceSnapshot,
        observation: &MarketObservation,
    ) -> Result<ScheduleStep, PhaseSchedulerError> {
        match current.schedule.catch_up_policy {
            CatchUpPolicy::ExpireWithoutCatchUp | CatchUpPolicy::SameBusinessDayBeforeDeadline => {
                Self::propose(
                    current,
                    ScheduleStatus::Missed,
                    ReasonCode::ScheduleWindowExpired,
                    observation.observed_at,
                    None,
                )
            }
            CatchUpPolicy::DeferToNextEligibleSession => {
                let next = observation
                    .next_eligible
                    .clone()
                    .ok_or(PhaseSchedulerError::NextEligibleSessionRequired)?;
                if next.occurrence_id != current.schedule.occurrence_id {
                    return Err(PhaseSchedulerError::InvalidNextEligibleSession);
                }
                Self::propose(
                    current,
                    ScheduleStatus::Deferred,
                    ReasonCode::ScheduleDeferred,
                    observation.observed_at,
                    Some(next),
                )
            }
            CatchUpPolicy::RecoverPersistedOnly => Ok(ScheduleStep::RecoveryOnly {
                occurrence_id: current.schedule.occurrence_id.clone(),
            }),
        }
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
