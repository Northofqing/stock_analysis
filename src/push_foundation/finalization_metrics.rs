//! Bounded, read-only SLA aggregation over one persisted namespace snapshot.
//! The result is an inventory fact, not production source registration,
//! external-clock attestation, a cross-source atomic view, or promotion authority.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::time::Duration;

use crate::durable_delivery::DurableDeliveryCoordinator;
use crate::event::AuditDispatcher;
use crate::monitor::push_job::{
    AuthorityClass, ChannelId, CompletionPolicy, Namespace, TerminalDisposition, UnitId, UtcMicros,
};

use super::dedicated_transport::DedicatedConformanceRoute;
use super::finalization_sla::{
    checked_target, inspect_persisted_finalization_sla, persisted_n02_window, FinalizationSlaError,
    FinalizationSlaReport, FinalizationSlaRoute, FinalizationSlaStatus,
    PersistedFinalizationSlaQuery,
};
use super::intent_store::{InventoryItemFailure, NamespaceInventoryItem, PersistedIntentRead};
use super::{BusinessIntentStore, IntentState, TerminalTemplateBinding};

macro_rules! counts_getters {
    ($($name:ident),* $(,)?) => {
        $(pub(crate) fn $name(&self) -> u64 { self.$name })*
    };
}

pub(crate) enum FinalizationMetricsSource<'a> {
    Generic {
        source: &'a DurableDeliveryCoordinator,
        required_channel: &'a ChannelId,
    },
    P01 {
        source: &'a DurableDeliveryCoordinator,
        route: &'a DedicatedConformanceRoute,
    },
    N02 {
        source: &'a AuditDispatcher,
        route: &'a DedicatedConformanceRoute,
    },
}

impl FinalizationMetricsSource<'_> {
    fn class(&self) -> AuthorityClass {
        match self {
            Self::Generic { .. } => AuthorityClass::GenericCounted,
            Self::P01 { .. } => AuthorityClass::P01Dedicated,
            Self::N02 { .. } => AuthorityClass::N02Dedicated,
        }
    }
}

pub(crate) struct FinalizationMetricsBinding<'a> {
    pub(crate) unit: &'a UnitId,
    pub(crate) template: &'a TerminalTemplateBinding,
    pub(crate) policy: &'a CompletionPolicy,
    pub(crate) source: FinalizationMetricsSource<'a>,
}

pub(crate) struct FinalizationMetricsQuery<'a> {
    pub(crate) namespace: &'a Namespace,
    pub(crate) observed_at: UtcMicros,
    pub(crate) reconcile_cycle: Duration,
    pub(crate) page_size: usize,
    pub(crate) max_intents: usize,
    pub(crate) bindings: &'a [FinalizationMetricsBinding<'a>],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum FinalizationMetricsError {
    #[error("finalization inventory page size must be between 1 and 1000")]
    InvalidPageSize,
    #[error("finalization inventory budget must be between 1 and 100000")]
    InvalidMaxIntents,
    #[error("finalization inventory SLA cycle is invalid")]
    InvalidCycle,
    #[error("finalization inventory contains a duplicate or ambiguous unit/template binding")]
    DuplicateBinding,
    #[error("finalization inventory binding does not match its typed policy or source")]
    InvalidBinding,
    #[error("finalization inventory persisted read failed")]
    InventoryReadFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InventoryCoverage {
    Complete,
    Limited,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BusinessStateCounts {
    pending_dispatch: u64,
    awaiting_authority: u64,
    awaiting_finalizer: u64,
    completed: u64,
    not_delivered: u64,
    no_data: u64,
    disabled: u64,
    resolution_required: u64,
}

impl BusinessStateCounts {
    counts_getters! {
        pending_dispatch, awaiting_authority, awaiting_finalizer, completed,
        not_delivered, no_data, disabled, resolution_required,
    }

    fn observe(&mut self, state: IntentState) {
        let count = match state {
            IntentState::PendingDispatch => &mut self.pending_dispatch,
            IntentState::AwaitingAuthority => &mut self.awaiting_authority,
            IntentState::AwaitingFinalizer => &mut self.awaiting_finalizer,
            IntentState::Completed => &mut self.completed,
            IntentState::NotDelivered => &mut self.not_delivered,
            IntentState::NoData => &mut self.no_data,
            IntentState::Disabled => &mut self.disabled,
            IntentState::ResolutionRequired => &mut self.resolution_required,
        };
        *count += 1;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SlaStatusCounts {
    not_applicable: u64,
    missing: u64,
    pending_seal: u64,
    awaiting_finalization: u64,
    completed: u64,
    manual_accepted: u64,
    rejected: u64,
    uncertain: u64,
    manual_not_delivered: u64,
    resolution_required: u64,
    clock_uncertain: u64,
    conflict: u64,
}

impl SlaStatusCounts {
    counts_getters! {
        not_applicable, missing, pending_seal, awaiting_finalization, completed,
        manual_accepted, rejected, uncertain, manual_not_delivered,
        resolution_required, clock_uncertain, conflict,
    }

    fn observe(&mut self, status: FinalizationSlaStatus) {
        let count = match status {
            FinalizationSlaStatus::NotApplicable => &mut self.not_applicable,
            FinalizationSlaStatus::Missing => &mut self.missing,
            FinalizationSlaStatus::PendingSeal => &mut self.pending_seal,
            FinalizationSlaStatus::AwaitingFinalization => &mut self.awaiting_finalization,
            FinalizationSlaStatus::Completed => &mut self.completed,
            FinalizationSlaStatus::ManualAccepted => &mut self.manual_accepted,
            FinalizationSlaStatus::Rejected => &mut self.rejected,
            FinalizationSlaStatus::Uncertain => &mut self.uncertain,
            FinalizationSlaStatus::ManualNotDelivered => &mut self.manual_not_delivered,
            FinalizationSlaStatus::ResolutionRequired => &mut self.resolution_required,
            FinalizationSlaStatus::ClockUncertain => &mut self.clock_uncertain,
            FinalizationSlaStatus::Conflict => &mut self.conflict,
        };
        *count += 1;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DispositionCounts {
    accepted: u64,
    rejected: u64,
    uncertain: u64,
    manual_accepted: u64,
    manual_not_delivered: u64,
}

impl DispositionCounts {
    counts_getters! {
        accepted, rejected, uncertain, manual_accepted, manual_not_delivered,
    }

    fn observe(&mut self, disposition: TerminalDisposition) {
        let count = match disposition {
            TerminalDisposition::Accepted => &mut self.accepted,
            TerminalDisposition::Rejected => &mut self.rejected,
            TerminalDisposition::Uncertain => &mut self.uncertain,
            TerminalDisposition::ManualConfirmedAccepted => &mut self.manual_accepted,
            TerminalDisposition::ManualConfirmedNotDelivered => &mut self.manual_not_delivered,
        };
        *count += 1;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InventoryErrorCounts {
    persisted_snapshot_invalid: u64,
    persisted_chain_invalid: u64,
    missing_binding: u64,
    business_invalid: u64,
    route_mismatch: u64,
    unsupported_occurrence_route: u64,
    source_invalid: u64,
    terminal_invalid: u64,
}

impl InventoryErrorCounts {
    counts_getters! {
        persisted_snapshot_invalid, persisted_chain_invalid, missing_binding,
        route_mismatch, unsupported_occurrence_route, source_invalid,
    }

    pub(crate) fn total(&self) -> u64 {
        self.persisted_snapshot_invalid
            + self.persisted_chain_invalid
            + self.missing_binding
            + self.business_invalid
            + self.route_mismatch
            + self.unsupported_occurrence_route
            + self.source_invalid
            + self.terminal_invalid
    }

    fn observe_read_failure(&mut self, failure: InventoryItemFailure) {
        match failure {
            InventoryItemFailure::SnapshotInvalid => self.persisted_snapshot_invalid += 1,
            InventoryItemFailure::TransitionChainInvalid => self.persisted_chain_invalid += 1,
        }
    }

    fn observe_sla_error(&mut self, error: FinalizationSlaError) {
        match error {
            FinalizationSlaError::InvalidCycle => unreachable!("cycle validated before reading"),
            FinalizationSlaError::BusinessInvalid => self.business_invalid += 1,
            FinalizationSlaError::RouteMismatch => self.route_mismatch += 1,
            FinalizationSlaError::UnsupportedOccurrenceRoute => {
                self.unsupported_occurrence_route += 1;
            }
            FinalizationSlaError::SourceInvalid => self.source_invalid += 1,
            FinalizationSlaError::TerminalInvalid => self.terminal_invalid += 1,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FinalizationMetricsReport {
    namespace: Namespace,
    observed_at: UtcMicros,
    reconcile_cycle: Duration,
    two_cycle_target: Duration,
    inventory_total: u64,
    checked: u64,
    unchecked: u64,
    coverage: InventoryCoverage,
    business_states: BusinessStateCounts,
    sla_statuses: SlaStatusCounts,
    dispositions: DispositionCounts,
    errors: InventoryErrorCounts,
    target_exceeded: u64,
    hard_limit_reached: u64,
    requires_block: u64,
    max_completed_latency: Option<Duration>,
    max_pending_accepted_age: Option<Duration>,
}

impl std::fmt::Debug for FinalizationMetricsReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FinalizationMetricsReport")
            .field("inventory_total", &self.inventory_total)
            .field("checked", &self.checked)
            .field("unchecked", &self.unchecked)
            .field("coverage", &self.coverage)
            .field("observed_at", &self.observed_at)
            .field("reconcile_cycle", &self.reconcile_cycle)
            .field("two_cycle_target", &self.two_cycle_target)
            .field("business_states", &self.business_states)
            .field("sla_statuses", &self.sla_statuses)
            .field("dispositions", &self.dispositions)
            .field("errors", &self.errors)
            .field("target_exceeded", &self.target_exceeded)
            .field("hard_limit_reached", &self.hard_limit_reached)
            .field("requires_block", &self.requires_block)
            .field("max_completed_latency", &self.max_completed_latency)
            .field("max_pending_accepted_age", &self.max_pending_accepted_age)
            .finish_non_exhaustive()
    }
}

impl FinalizationMetricsReport {
    pub(crate) fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    pub(crate) fn inventory_total(&self) -> u64 {
        self.inventory_total
    }

    pub(crate) fn observed_at(&self) -> UtcMicros {
        self.observed_at
    }

    pub(crate) fn reconcile_cycle(&self) -> Duration {
        self.reconcile_cycle
    }

    pub(crate) fn two_cycle_target(&self) -> Duration {
        self.two_cycle_target
    }

    pub(crate) fn checked(&self) -> u64 {
        self.checked
    }

    pub(crate) fn unchecked(&self) -> u64 {
        self.unchecked
    }

    pub(crate) fn coverage(&self) -> InventoryCoverage {
        self.coverage
    }

    pub(crate) fn business_states(&self) -> BusinessStateCounts {
        self.business_states
    }

    pub(crate) fn sla_statuses(&self) -> SlaStatusCounts {
        self.sla_statuses
    }

    pub(crate) fn dispositions(&self) -> DispositionCounts {
        self.dispositions
    }

    pub(crate) fn errors(&self) -> InventoryErrorCounts {
        self.errors
    }

    pub(crate) fn target_exceeded(&self) -> u64 {
        self.target_exceeded
    }

    pub(crate) fn hard_limit_reached(&self) -> u64 {
        self.hard_limit_reached
    }

    pub(crate) fn requires_block(&self) -> u64 {
        self.requires_block
    }

    pub(crate) fn max_completed_latency(&self) -> Option<Duration> {
        self.max_completed_latency
    }

    pub(crate) fn max_pending_accepted_age(&self) -> Option<Duration> {
        self.max_pending_accepted_age
    }
}

#[derive(Default)]
struct Accumulator {
    business_states: BusinessStateCounts,
    sla_statuses: SlaStatusCounts,
    dispositions: DispositionCounts,
    errors: InventoryErrorCounts,
    target_exceeded: u64,
    hard_limit_reached: u64,
    requires_block: u64,
    max_completed_latency: Option<Duration>,
    max_pending_accepted_age: Option<Duration>,
}

impl Accumulator {
    fn observe_report(&mut self, report: FinalizationSlaReport) {
        self.sla_statuses.observe(report.status());
        if let Some(disposition) = report.disposition() {
            self.dispositions.observe(disposition);
        }
        self.target_exceeded += u64::from(report.target_exceeded() == Some(true));
        self.hard_limit_reached += u64::from(report.hard_limit_reached() == Some(true));
        self.requires_block += u64::from(report.requires_block());
        if report.completed_at().is_some()
            && report.disposition() == Some(TerminalDisposition::Accepted)
        {
            maximize(&mut self.max_completed_latency, report.elapsed());
        }
        maximize(
            &mut self.max_pending_accepted_age,
            report.pending_accepted_age(),
        );
    }
}

pub(crate) fn inspect_finalization_metrics(
    store: &BusinessIntentStore,
    query: FinalizationMetricsQuery<'_>,
) -> Result<FinalizationMetricsReport, FinalizationMetricsError> {
    if !(1..=1_000).contains(&query.page_size) {
        return Err(FinalizationMetricsError::InvalidPageSize);
    }
    if !(1..=100_000).contains(&query.max_intents) {
        return Err(FinalizationMetricsError::InvalidMaxIntents);
    }
    let two_cycle_target = checked_target(query.reconcile_cycle)
        .map_err(|_| FinalizationMetricsError::InvalidCycle)?;

    let mut bindings = BTreeMap::new();
    for binding in query.bindings {
        if binding.policy.completion_owner().unit_id() != binding.unit
            || !binding.policy.allows_authority(binding.source.class())
            || !source_matches_template(&binding.source, binding.template)
        {
            return Err(FinalizationMetricsError::InvalidBinding);
        }
        let key = (binding.unit.as_str(), binding.template.sha256().as_str());
        if bindings.insert(key, binding).is_some() {
            return Err(FinalizationMetricsError::DuplicateBinding);
        }
    }

    let mut accumulator = Accumulator::default();
    let read = store
        .scan_namespace_inventory(
            query.namespace,
            query.page_size,
            query.max_intents,
            |item| match item {
                NamespaceInventoryItem::Invalid(failure) => {
                    accumulator.errors.observe_read_failure(failure);
                }
                NamespaceInventoryItem::Verified(persisted) => {
                    accumulator.business_states.observe(persisted.state());
                    let key = (persisted.unit_id(), persisted.template_sha256());
                    let Some(binding) = bindings.get(&key).copied() else {
                        if bindings
                            .keys()
                            .any(|(unit, _)| *unit == persisted.unit_id())
                        {
                            accumulator.errors.route_mismatch += 1;
                        } else {
                            accumulator.errors.missing_binding += 1;
                        }
                        return;
                    };
                    let route = match sla_route(&binding.source, persisted) {
                        Ok(route) => route,
                        Err(error) => {
                            accumulator.errors.observe_sla_error(error);
                            return;
                        }
                    };
                    let result = inspect_persisted_finalization_sla(
                        persisted,
                        PersistedFinalizationSlaQuery {
                            namespace: query.namespace,
                            unit: binding.unit,
                            template: binding.template,
                            policy: binding.policy,
                            route,
                            observed_at: query.observed_at,
                            reconcile_cycle: query.reconcile_cycle,
                        },
                    );
                    match result {
                        Ok(report) => accumulator.observe_report(report),
                        Err(error) => accumulator.errors.observe_sla_error(error),
                    }
                }
            },
        )
        .map_err(|_| FinalizationMetricsError::InventoryReadFailed)?;
    let unchecked = read.total() - read.checked();
    Ok(FinalizationMetricsReport {
        namespace: query.namespace.clone(),
        observed_at: query.observed_at,
        reconcile_cycle: query.reconcile_cycle,
        two_cycle_target,
        inventory_total: read.total(),
        checked: read.checked(),
        unchecked,
        coverage: if unchecked == 0 {
            InventoryCoverage::Complete
        } else {
            InventoryCoverage::Limited
        },
        business_states: accumulator.business_states,
        sla_statuses: accumulator.sla_statuses,
        dispositions: accumulator.dispositions,
        errors: accumulator.errors,
        target_exceeded: accumulator.target_exceeded,
        hard_limit_reached: accumulator.hard_limit_reached,
        requires_block: accumulator.requires_block,
        max_completed_latency: accumulator.max_completed_latency,
        max_pending_accepted_age: accumulator.max_pending_accepted_age,
    })
}

fn source_matches_template(
    source: &FinalizationMetricsSource<'_>,
    template: &TerminalTemplateBinding,
) -> bool {
    match source {
        FinalizationMetricsSource::Generic { .. } => true,
        FinalizationMetricsSource::P01 { route, .. }
        | FinalizationMetricsSource::N02 { route, .. } => {
            route.matches_sla_route(source.class(), template)
        }
    }
}

fn sla_route<'a, 'source>(
    source: &'a FinalizationMetricsSource<'source>,
    persisted: &PersistedIntentRead,
) -> Result<FinalizationSlaRoute<'a>, FinalizationSlaError>
where
    'source: 'a,
{
    match source {
        FinalizationMetricsSource::Generic {
            source,
            required_channel,
        } => Ok(FinalizationSlaRoute::Generic {
            source,
            required_channel,
        }),
        FinalizationMetricsSource::P01 { source, route } => {
            Ok(FinalizationSlaRoute::P01 { source, route })
        }
        FinalizationMetricsSource::N02 { source, route } => Ok(FinalizationSlaRoute::N02 {
            source,
            route,
            window: persisted_n02_window(persisted)?,
        }),
    }
}

fn maximize(target: &mut Option<Duration>, candidate: Option<Duration>) {
    if let Some(candidate) = candidate {
        *target = Some(target.map_or(candidate, |current| current.max(candidate)));
    }
}
