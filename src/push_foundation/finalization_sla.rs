//! Read-only persisted finalization facts under an explicit observation clock.
//! This is not source authentication, route registration, a cross-database
//! atomic snapshot, a trusted clock, or permission to promote a production root.

#![cfg_attr(not(test), allow(dead_code))]

use std::time::Duration;

use serde::Deserialize;

use crate::durable_delivery::{DurableDeliveryCoordinator, TypedReceipt};
use crate::event::{AuditDispatcher, EventEnvelope, NewsFlashWindow, PushRecord};
use crate::monitor::push_job::{
    raw_digest, AuthorityClass, ChannelId, CompletionPolicy, DecisionId, IntentId, Namespace,
    ReasonCode, Sha256Digest, TerminalDisposition, UnitId, UtcMicros,
};

use super::dedicated_transport::{
    inspect_n02_dedicated, inspect_p01_dedicated, DedicatedConformanceError,
    DedicatedConformanceRoute,
};
use super::generic_transport::GenericTerminalAuthorityAdapter;
use super::intent_store::PersistedIntentRead;
use super::terminal_authority::{
    verify_terminal, AuthorityDescriptor, AuthorityQuery, AuthorityQueryFailure,
    AuthorityTerminalRecord, TerminalAuthorityPort, TerminalTemplateBinding,
};
use super::{BusinessIntentStore, InitialDecisionKind, IntentSnapshot, IntentState};

pub(crate) enum FinalizationSlaRoute<'a> {
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
        window: NewsFlashWindow,
    },
}

impl FinalizationSlaRoute<'_> {
    fn class(&self) -> AuthorityClass {
        match self {
            Self::Generic { .. } => AuthorityClass::GenericCounted,
            Self::P01 { .. } => AuthorityClass::P01Dedicated,
            Self::N02 { .. } => AuthorityClass::N02Dedicated,
        }
    }

    fn read(&self, snapshot: &IntentSnapshot) -> Result<AuthorityQuery, FinalizationSlaError> {
        match self {
            Self::Generic { source, .. } => {
                let adapter = GenericTerminalAuthorityAdapter::try_new(source)
                    .map_err(|_| FinalizationSlaError::SourceInvalid)?;
                let decision = snapshot
                    .attested_ready_binding()
                    .map_err(|_| FinalizationSlaError::BusinessInvalid)?
                    .decision_id;
                adapter
                    .requery_terminal(&decision)
                    .map_err(|_| FinalizationSlaError::SourceInvalid)
            }
            Self::P01 { source, route } => {
                dedicated_query(inspect_p01_dedicated(snapshot, route, *source))
            }
            Self::N02 {
                source,
                route,
                window,
            } => dedicated_query(inspect_n02_dedicated(snapshot, *window, route, *source)),
        }
    }
}

fn dedicated_query(
    result: Result<AuthorityTerminalRecord, DedicatedConformanceError>,
) -> Result<AuthorityQuery, FinalizationSlaError> {
    match result {
        Ok(record) => Ok(AuthorityQuery::Terminal(Box::new(record))),
        Err(DedicatedConformanceError::TerminalMissing) => Ok(AuthorityQuery::Missing),
        Err(DedicatedConformanceError::TerminalPendingSeal) => Ok(AuthorityQuery::PendingSeal),
        Err(DedicatedConformanceError::InvalidRoute) => Err(FinalizationSlaError::RouteMismatch),
        Err(_) => Err(FinalizationSlaError::SourceInvalid),
    }
}

pub(crate) struct FinalizationSlaQuery<'a> {
    pub(crate) namespace: &'a Namespace,
    pub(crate) unit: &'a UnitId,
    pub(crate) intent: &'a IntentId,
    pub(crate) template: &'a TerminalTemplateBinding,
    pub(crate) policy: &'a CompletionPolicy,
    pub(crate) route: FinalizationSlaRoute<'a>,
    pub(crate) observed_at: UtcMicros,
    pub(crate) reconcile_cycle: Duration,
}

pub(crate) struct PersistedFinalizationSlaQuery<'a> {
    pub(crate) namespace: &'a Namespace,
    pub(crate) unit: &'a UnitId,
    pub(crate) template: &'a TerminalTemplateBinding,
    pub(crate) policy: &'a CompletionPolicy,
    pub(crate) route: FinalizationSlaRoute<'a>,
    pub(crate) observed_at: UtcMicros,
    pub(crate) reconcile_cycle: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FinalizationSlaStatus {
    NotApplicable,
    Missing,
    PendingSeal,
    AwaitingFinalization,
    Completed,
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
    ResolutionRequired,
    ClockUncertain,
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum FinalizationSlaError {
    #[error("finalization SLA cycle must be nonzero exact microseconds without overflow")]
    InvalidCycle,
    #[error("finalization SLA business snapshot or transition chain is unavailable or invalid")]
    BusinessInvalid,
    #[error("finalization SLA query route does not match the business intent")]
    RouteMismatch,
    #[error("finalization SLA query does not support this N02 occurrence convention")]
    UnsupportedOccurrenceRoute,
    #[error("finalization SLA authority source is unavailable or invalid")]
    SourceInvalid,
    #[error("finalization SLA exact terminal verification failed")]
    TerminalInvalid,
}

impl FinalizationSlaError {
    pub(crate) fn reason(self) -> ReasonCode {
        ReasonCode::FinalizerTerminalRefInvalid
    }
}

/// All fields are computed from the attested snapshot and the exact sealed
/// evidence passed to W09. Debug deliberately excludes caller text and raw refs.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FinalizationSlaReport {
    namespace: String,
    unit: String,
    intent: String,
    decision: String,
    business_state: IntentState,
    version: u64,
    head: Option<Sha256Digest>,
    authority: AuthorityClass,
    disposition: Option<TerminalDisposition>,
    terminal_ref_sha256: Option<Sha256Digest>,
    evidence_sha256: Option<Sha256Digest>,
    binding_sha256: Option<Sha256Digest>,
    accepted_at: Option<UtcMicros>,
    completed_at: Option<UtcMicros>,
    observed_at: UtcMicros,
    elapsed: Option<Duration>,
    two_cycle_target: Duration,
    target_exceeded: Option<bool>,
    hard_limit_reached: Option<bool>,
    requires_block: bool,
    status: FinalizationSlaStatus,
}

impl std::fmt::Debug for FinalizationSlaReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FinalizationSlaReport")
            .field("status", &self.status)
            .field("business_state", &self.business_state)
            .field("version", &self.version)
            .field("authority", &self.authority)
            .field("elapsed", &self.elapsed)
            .field("requires_block", &self.requires_block)
            .finish_non_exhaustive()
    }
}

macro_rules! report_getters {
    ($($name:ident: $ty:ty),* $(,)?) => { $(pub(crate) fn $name(&self) -> $ty { self.$name })* };
}
impl FinalizationSlaReport {
    pub(crate) fn namespace(&self) -> &str {
        &self.namespace
    }
    pub(crate) fn unit(&self) -> &str {
        &self.unit
    }
    pub(crate) fn intent(&self) -> &str {
        &self.intent
    }
    pub(crate) fn decision(&self) -> &str {
        &self.decision
    }
    pub(crate) fn head(&self) -> Option<&Sha256Digest> {
        self.head.as_ref()
    }
    pub(crate) fn terminal_ref_sha256(&self) -> Option<&Sha256Digest> {
        self.terminal_ref_sha256.as_ref()
    }
    pub(crate) fn evidence_sha256(&self) -> Option<&Sha256Digest> {
        self.evidence_sha256.as_ref()
    }
    pub(crate) fn binding_sha256(&self) -> Option<&Sha256Digest> {
        self.binding_sha256.as_ref()
    }
    report_getters! {
        business_state: IntentState, version: u64, authority: AuthorityClass,
        disposition: Option<TerminalDisposition>, accepted_at: Option<UtcMicros>,
        completed_at: Option<UtcMicros>, observed_at: UtcMicros, elapsed: Option<Duration>,
        two_cycle_target: Duration, target_exceeded: Option<bool>, hard_limit_reached: Option<bool>,
        requires_block: bool, status: FinalizationSlaStatus,
    }
    pub(crate) fn reason(&self) -> Option<ReasonCode> {
        if matches!(
            self.status,
            FinalizationSlaStatus::Conflict | FinalizationSlaStatus::ClockUncertain
        ) {
            Some(ReasonCode::FinalizerTerminalRefInvalid)
        } else if self.target_exceeded == Some(true) || self.hard_limit_reached == Some(true) {
            Some(ReasonCode::FinalizerDeadlineExceeded)
        } else {
            None
        }
    }

    pub(crate) fn pending_accepted_age(&self) -> Option<Duration> {
        if matches!(
            self.status,
            FinalizationSlaStatus::Completed | FinalizationSlaStatus::ClockUncertain
        ) {
            return None;
        }
        let accepted_at = self.accepted_at?;
        let micros = self.observed_at.get().checked_sub(accepted_at.get())?;
        u64::try_from(micros).ok().map(Duration::from_micros)
    }
}

struct CapturedAuthority {
    descriptor: AuthorityDescriptor,
    record: AuthorityTerminalRecord,
}
impl TerminalAuthorityPort for CapturedAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }
    fn requery_terminal(
        &self,
        decision: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure> {
        if decision != &self.record.decision_id {
            return Err(AuthorityQueryFailure);
        }
        Ok(AuthorityQuery::Terminal(Box::new(self.record.clone())))
    }
}

pub(crate) fn inspect_finalization_sla(
    store: &BusinessIntentStore,
    query: FinalizationSlaQuery<'_>,
) -> Result<FinalizationSlaReport, FinalizationSlaError> {
    checked_target(query.reconcile_cycle)?;
    let persisted = store
        .inspect_with_transition_chain(query.intent)
        .map_err(|_| FinalizationSlaError::BusinessInvalid)?;
    if persisted.snapshot().intent_id() != query.intent.as_str() {
        return Err(FinalizationSlaError::RouteMismatch);
    }
    inspect_persisted_finalization_sla(
        &persisted,
        PersistedFinalizationSlaQuery {
            namespace: query.namespace,
            unit: query.unit,
            template: query.template,
            policy: query.policy,
            route: query.route,
            observed_at: query.observed_at,
            reconcile_cycle: query.reconcile_cycle,
        },
    )
}

pub(crate) fn persisted_n02_window(
    persisted: &PersistedIntentRead,
) -> Result<NewsFlashWindow, FinalizationSlaError> {
    if persisted.occurrence_family() != "news-flash-window" {
        return Err(FinalizationSlaError::UnsupportedOccurrenceRoute);
    }
    NewsFlashWindow::parse(persisted.occurrence_key())
        .map_err(|_| FinalizationSlaError::UnsupportedOccurrenceRoute)
}

pub(crate) fn inspect_persisted_finalization_sla(
    persisted: &PersistedIntentRead,
    query: PersistedFinalizationSlaQuery<'_>,
) -> Result<FinalizationSlaReport, FinalizationSlaError> {
    let two_cycle_target = checked_target(query.reconcile_cycle)?;
    let snapshot = persisted.snapshot();
    let chain = persisted.chain();
    let expected_namespace = match query.namespace {
        Namespace::Production => "Production".to_owned(),
        Namespace::Test { run_id } => format!("Test:{}", run_id.as_str()),
    };
    if snapshot.namespace() != expected_namespace
        || snapshot.unit_id() != query.unit.as_str()
        || query.policy.completion_owner().unit_id() != query.unit
        || !snapshot.sla_route_matches(
            query.template.sha256(),
            query.policy.completion_owner().completion_owner().as_str(),
        )
        || !query.policy.allows_authority(query.route.class())
    {
        return Err(FinalizationSlaError::RouteMismatch);
    }
    if let FinalizationSlaRoute::N02 { window, .. } = &query.route {
        // Local W19 support for the existing tested occurrence convention only.
        // This is not a registered production route. Producer migration must
        // establish the same contract before other occurrence forms are supported.
        if !snapshot.sla_n02_occurrence_supported() {
            return Err(FinalizationSlaError::UnsupportedOccurrenceRoute);
        }
        if !snapshot.sla_n02_window_matches(window.label()) {
            return Err(FinalizationSlaError::RouteMismatch);
        }
    }
    if let FinalizationSlaRoute::P01 { route, .. } | FinalizationSlaRoute::N02 { route, .. } =
        &query.route
    {
        if !route.matches_sla_route(query.route.class(), query.template) {
            return Err(FinalizationSlaError::RouteMismatch);
        }
    }
    let mut report = FinalizationSlaReport {
        namespace: snapshot.namespace().to_owned(),
        unit: snapshot.unit_id().to_owned(),
        intent: snapshot.intent_id().to_owned(),
        decision: snapshot.decision_id().to_owned(),
        business_state: snapshot.state(),
        version: snapshot.version(),
        head: chain.last().map(|event| event.canonical_sha256().clone()),
        authority: query.route.class(),
        disposition: None,
        terminal_ref_sha256: None,
        evidence_sha256: None,
        binding_sha256: None,
        accepted_at: None,
        completed_at: None,
        observed_at: query.observed_at,
        elapsed: None,
        two_cycle_target,
        target_exceeded: None,
        hard_limit_reached: None,
        requires_block: false,
        status: FinalizationSlaStatus::NotApplicable,
    };
    let clock_before_business = query.observed_at < snapshot.updated_at();
    if snapshot.decision_kind() != InitialDecisionKind::Ready {
        if snapshot.state() == IntentState::ResolutionRequired {
            report.status = FinalizationSlaStatus::ResolutionRequired;
        }
        if clock_before_business {
            report.status = FinalizationSlaStatus::ClockUncertain;
        }
        return Ok(report);
    }
    let record = match query.route.read(snapshot)? {
        AuthorityQuery::Missing => {
            report.status = FinalizationSlaStatus::Missing;
            return Ok(unavailable_report(report, chain, clock_before_business));
        }
        AuthorityQuery::PendingSeal => {
            report.status = FinalizationSlaStatus::PendingSeal;
            return Ok(unavailable_report(report, chain, clock_before_business));
        }
        AuthorityQuery::Terminal(record) => *record,
    };
    let captured = CapturedAuthority {
        descriptor: AuthorityDescriptor {
            authority_class: query.route.class(),
            durable_schema_version: record.durable_schema_version.clone(),
        },
        record,
    };
    let terminal = verify_terminal(
        snapshot,
        query.template,
        query.policy,
        &captured,
        query.observed_at,
    )
    .map_err(|_| FinalizationSlaError::TerminalInvalid)?;
    report.disposition = Some(terminal.terminal_disposition());
    report.terminal_ref_sha256 = Some(raw_digest(terminal.ref_id().as_str().as_bytes()));
    report.evidence_sha256 = Some(terminal.evidence_sha256().clone());
    report.binding_sha256 = Some(terminal.binding_sha256().clone());
    let mut conflict = false;
    for event in chain {
        // Both terminal business edges carry immutable authority material.
        // NotDelivered history must not silently accept a different terminal.
        if matches!(
            event.to_state(),
            IntentState::Completed | IntentState::NotDelivered
        ) {
            if event.terminal_ref_id() != Some(terminal.ref_id().as_str())
                || event.terminal_binding_sha256() != Some(terminal.binding_sha256())
                || event.terminal_disposition() != Some(terminal.terminal_disposition())
                || event
                    .terminal_decision_id()
                    .is_some_and(|decision| decision != terminal.decision_id().as_str())
            {
                conflict = true;
            } else if event.to_state() == IntentState::Completed {
                report.completed_at = report.completed_at.or(Some(event.occurred_at()));
            }
        }
        // The existing finalizer admits AwaitingFinalizer only after verifying
        // an accepted disposition. Later ResolutionRequired does not erase it.
        if event.to_state() == IntentState::AwaitingFinalizer
            && !matches!(
                terminal.terminal_disposition(),
                TerminalDisposition::Accepted | TerminalDisposition::ManualConfirmedAccepted
            )
        {
            conflict = true;
        }
    }
    report.status = match terminal.terminal_disposition() {
        TerminalDisposition::Accepted => {
            match original_accepted_at(&captured.record, &query.route) {
                Ok(accepted_at) => report.accepted_at = Some(accepted_at),
                Err(ClockReadError::Clock) => {
                    report.status = FinalizationSlaStatus::ClockUncertain;
                    return Ok(report);
                }
                Err(ClockReadError::Invalid) => return Err(FinalizationSlaError::SourceInvalid),
            }
            if matches!(
                snapshot.state(),
                IntentState::NoData | IntentState::Disabled | IntentState::NotDelivered
            ) {
                conflict = true;
            }
            if snapshot.state() == IntentState::Completed {
                FinalizationSlaStatus::Completed
            } else {
                FinalizationSlaStatus::AwaitingFinalization
            }
        }
        TerminalDisposition::ManualConfirmedAccepted => FinalizationSlaStatus::ManualAccepted,
        TerminalDisposition::Rejected => FinalizationSlaStatus::Rejected,
        TerminalDisposition::Uncertain => FinalizationSlaStatus::Uncertain,
        TerminalDisposition::ManualConfirmedNotDelivered => {
            FinalizationSlaStatus::ManualNotDelivered
        }
    };
    if snapshot.state() == IntentState::Completed && report.completed_at.is_none() {
        conflict = true;
    }
    if snapshot.state() == IntentState::ResolutionRequired {
        report.status = FinalizationSlaStatus::ResolutionRequired;
    }
    if conflict {
        report.status = FinalizationSlaStatus::Conflict;
    }
    if let Some(accepted) = report.accepted_at {
        let endpoint = report.completed_at.unwrap_or(query.observed_at);
        if clock_before_business || query.observed_at < accepted || endpoint < accepted {
            report.status = FinalizationSlaStatus::ClockUncertain;
        } else {
            let elapsed = checked_elapsed(accepted, endpoint)?;
            report.elapsed = Some(elapsed);
            report.target_exceeded = Some(elapsed > two_cycle_target);
            report.hard_limit_reached = Some(elapsed >= Duration::from_secs(300));
            report.requires_block = (snapshot.state() != IntentState::Completed || conflict)
                && checked_elapsed(accepted, query.observed_at)? >= Duration::from_secs(300);
        }
    } else if clock_before_business {
        report.status = FinalizationSlaStatus::ClockUncertain;
    }
    Ok(report)
}

fn checked_elapsed(from: UtcMicros, to: UtcMicros) -> Result<Duration, FinalizationSlaError> {
    let micros = to
        .get()
        .checked_sub(from.get())
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(FinalizationSlaError::TerminalInvalid)?;
    Ok(Duration::from_micros(micros))
}

fn unavailable_report(
    mut report: FinalizationSlaReport,
    chain: &[super::TransitionReceipt],
    clock_before_business: bool,
) -> FinalizationSlaReport {
    if chain.iter().any(|event| {
        matches!(
            event.to_state(),
            IntentState::AwaitingFinalizer | IntentState::Completed | IntentState::NotDelivered
        )
    }) {
        report.status = FinalizationSlaStatus::Conflict;
    } else if report.business_state == IntentState::ResolutionRequired {
        report.status = FinalizationSlaStatus::ResolutionRequired;
    }
    if clock_before_business {
        report.status = FinalizationSlaStatus::ClockUncertain;
    }
    report
}

pub(super) fn checked_target(cycle: Duration) -> Result<Duration, FinalizationSlaError> {
    let micros =
        i64::try_from(cycle.as_micros()).map_err(|_| FinalizationSlaError::InvalidCycle)?;
    if cycle.is_zero() || Duration::from_micros(micros as u64) != cycle {
        return Err(FinalizationSlaError::InvalidCycle);
    }
    let target = micros
        .checked_mul(2)
        .ok_or(FinalizationSlaError::InvalidCycle)?;
    Ok(Duration::from_micros(target as u64))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedEvidence {
    kind: String,
    receipt: TypedReceipt,
}
enum ClockReadError {
    Clock,
    Invalid,
}

fn original_accepted_at(
    record: &AuthorityTerminalRecord,
    route: &FinalizationSlaRoute<'_>,
) -> Result<UtcMicros, ClockReadError> {
    let timestamp = match route {
        FinalizationSlaRoute::N02 { .. } => {
            let envelope: EventEnvelope = serde_json::from_slice(&record.evidence_bytes)
                .map_err(|_| ClockReadError::Invalid)?;
            let push = PushRecord::try_from_authoritative(&envelope)
                .map_err(|_| ClockReadError::Invalid)?;
            push.news_flash_remote_receipt
                .ok_or(ClockReadError::Clock)?
                .accepted_at
                .with_timezone(&chrono::Utc)
        }
        _ => {
            let evidence: AcceptedEvidence = serde_json::from_slice(&record.evidence_bytes)
                .map_err(|_| ClockReadError::Invalid)?;
            if evidence.kind != "Accepted" || evidence.receipt.validate().is_err() {
                return Err(ClockReadError::Invalid);
            }
            if let FinalizationSlaRoute::Generic {
                required_channel, ..
            } = route
            {
                if evidence.receipt.channel != required_channel.as_str() {
                    return Err(ClockReadError::Invalid);
                }
            }
            evidence.receipt.accepted_at
        }
    };
    // Persisted timestamps are projected to UTC microseconds (fractional
    // nanoseconds are floored); comparisons never round to seconds or clamp.
    let micros = timestamp
        .timestamp()
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(i64::from(timestamp.timestamp_subsec_micros())))
        .ok_or(ClockReadError::Clock)?;
    UtcMicros::try_new(micros).map_err(|_| ClockReadError::Clock)
}
