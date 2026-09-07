//! W12 generic durable transport and exact W09 authority adapter.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::durable_delivery::{
    compiled_policy_catalog, AuthoritativeDeliveryRequest, AuthoritativeSink,
    AuthoritativeSinkPort, AuthoritativeSinkResult, DeliveryEnvelope, DeliverySubKind,
    DurableDeliveryCoordinator, FoundationDeliveryBinding, FoundationTerminalDisposition,
    FoundationTerminalQuery, ImmutableAppendPort, PushKind, TypedUncertainty,
};
use crate::monitor::push_job::{
    canonical_digest, raw_digest, subject_value, AttemptId, AudienceId, AuthorityClass,
    BusinessDate, ChannelId, CompletionEligibility, CompletionPolicy, DecisionId, DeliveryResult,
    DeliveryResultView, DurableSchemaVersion, IntentId, Namespace, OccurrenceId, RunId,
    Sha256Digest, SubjectId, TemplateId, TemplateVersion, TerminalDisposition, TerminalRefId,
    UnitId, UtcMicros,
};

use super::terminal_authority::{
    terminal_binding_sha256, verify_terminal, AuthorityAttemptBinding, AuthorityDescriptor,
    AuthorityQuery, AuthorityQueryFailure, AuthorityTerminalRecord, TerminalAuthorityPort,
    TerminalTemplateBinding,
};
use super::{IntentSnapshot, IntentState, LeaseOwnerId};

const REQUIRED_CHANNEL_MISMATCH: &str = "foundation_required_channel_mismatch";

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum GenericTransportError {
    #[error("generic transport route is invalid")]
    InvalidRoute,
    #[error("business intent is not an attested claimed Ready intent")]
    InvalidBusinessIntent,
    #[error("business dispatch lease does not match the claimed intent")]
    BusinessLeaseMismatch,
    #[error("transport sink descriptor does not match the required channel")]
    SinkDescriptorMismatch,
    #[error("generic transport timestamp is invalid")]
    InvalidTimestamp,
    #[error("durable transport operation failed")]
    DurableFailure,
    #[error("terminal authority verification failed")]
    TerminalVerificationFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GenericTransportRoute {
    push_kind: PushKind,
    sub_kind: DeliverySubKind,
    scope_key: String,
    required_channel: ChannelId,
    template: TerminalTemplateBinding,
}

impl GenericTransportRoute {
    pub(crate) fn try_new(
        push_kind: PushKind,
        sub_kind: DeliverySubKind,
        scope_key: String,
        required_channel: ChannelId,
        template: TerminalTemplateBinding,
    ) -> Result<Self, GenericTransportError> {
        let policy = compiled_policy_catalog()
            .into_iter()
            .find(|row| row.push_kind == push_kind && row.sub_kind == sub_kind)
            .ok_or(GenericTransportError::InvalidRoute)?;
        let scope_valid = match policy.cooldown_scope {
            crate::durable_delivery::CooldownScope::Global => scope_key == "GLOBAL",
            crate::durable_delivery::CooldownScope::PerTicket => {
                !scope_key.is_empty() && scope_key.trim() == scope_key
            }
        };
        if !scope_valid {
            return Err(GenericTransportError::InvalidRoute);
        }
        Ok(Self {
            push_kind,
            sub_kind,
            scope_key,
            required_channel,
            template,
        })
    }

    pub(crate) fn required_channel(&self) -> &ChannelId {
        &self.required_channel
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GenericDispatchFence {
    owner: LeaseOwnerId,
    generation: u64,
    until: UtcMicros,
}

impl GenericDispatchFence {
    pub(crate) fn try_new(
        owner: LeaseOwnerId,
        generation: u64,
        until: UtcMicros,
    ) -> Result<Self, GenericTransportError> {
        if generation == 0 {
            return Err(GenericTransportError::BusinessLeaseMismatch);
        }
        Ok(Self {
            owner,
            generation,
            until,
        })
    }

    fn matches(&self, snapshot: &IntentSnapshot, dispatched_at: UtcMicros) -> bool {
        snapshot.lease_owner() == Some(self.owner.as_str())
            && snapshot.lease_generation() == self.generation
            && snapshot.lease_until() == Some(self.until)
            && self.until > dispatched_at
    }
}

pub(crate) struct GenericDispatchRequest<'a> {
    snapshot: &'a IntentSnapshot,
    route: &'a GenericTransportRoute,
    fence: &'a GenericDispatchFence,
    completion_policy: &'a CompletionPolicy,
    sink: AuthoritativeSink,
    append_port: &'a dyn ImmutableAppendPort,
    dispatched_at: UtcMicros,
    verified_at: UtcMicros,
}

impl<'a> GenericDispatchRequest<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        snapshot: &'a IntentSnapshot,
        route: &'a GenericTransportRoute,
        fence: &'a GenericDispatchFence,
        completion_policy: &'a CompletionPolicy,
        sink: AuthoritativeSink,
        append_port: &'a dyn ImmutableAppendPort,
        dispatched_at: UtcMicros,
        verified_at: UtcMicros,
    ) -> Self {
        Self {
            snapshot,
            route,
            fence,
            completion_policy,
            sink,
            append_port,
            dispatched_at,
            verified_at,
        }
    }
}

pub(crate) struct GenericTransportAuthorityAdapter<'a> {
    coordinator: &'a DurableDeliveryCoordinator,
}

impl<'a> GenericTransportAuthorityAdapter<'a> {
    pub(crate) fn new(coordinator: &'a DurableDeliveryCoordinator) -> Self {
        Self { coordinator }
    }

    pub(crate) fn dispatch(
        &self,
        request: GenericDispatchRequest<'_>,
    ) -> Result<DeliveryResult, GenericTransportError> {
        let attested = request
            .snapshot
            .attested_ready_binding()
            .map_err(|_| GenericTransportError::InvalidBusinessIntent)?;
        if request.verified_at < request.dispatched_at {
            return Err(GenericTransportError::InvalidTimestamp);
        }
        if request.snapshot.state() != IntentState::AwaitingAuthority
            || !request
                .fence
                .matches(request.snapshot, request.dispatched_at)
            || request.fence.until <= request.verified_at
        {
            return Err(GenericTransportError::BusinessLeaseMismatch);
        }
        if request.route.template.sha256() != &attested.template_sha256 {
            return Err(GenericTransportError::InvalidRoute);
        }
        if request.sink.sink_identity() != request.route.required_channel().as_str() {
            return Err(GenericTransportError::SinkDescriptorMismatch);
        }
        let dispatched_at = DateTime::<Utc>::from_timestamp_micros(request.dispatched_at.get())
            .ok_or(GenericTransportError::InvalidTimestamp)?;
        let envelope = build_foundation_envelope(request.snapshot, &attested, request.route)?;
        let decision_identity = envelope.decision_identity.clone();
        let required_sink: AuthoritativeSink = Arc::new(RequiredChannelSink {
            required_channel: request.route.required_channel().as_str().to_owned(),
            inner: request.sink,
        });
        self.coordinator
            .prepare(&envelope, 1, dispatched_at)
            .map_err(|_| GenericTransportError::DurableFailure)?;
        let already_terminal = match self
            .coordinator
            .inspect_foundation_terminal(&decision_identity)
            .map_err(|_| GenericTransportError::DurableFailure)?
        {
            FoundationTerminalQuery::Terminal(_) => true,
            FoundationTerminalQuery::PendingSeal { .. } => false,
            FoundationTerminalQuery::Missing => return Err(GenericTransportError::DurableFailure),
        };
        if !already_terminal {
            self.coordinator
                .resume_deliverable(&decision_identity, &[required_sink], dispatched_at)
                .map_err(|_| GenericTransportError::DurableFailure)?;
        }
        self.coordinator
            .reconcile_all_pending(request.append_port, dispatched_at)
            .map_err(|_| GenericTransportError::DurableFailure)?;

        let authority = GenericTerminalAuthorityAdapter::try_new(self.coordinator)?;
        let verified = verify_terminal(
            request.snapshot,
            &request.route.template,
            request.completion_policy,
            &authority,
            request.verified_at,
        )
        .map_err(|_| GenericTransportError::TerminalVerificationFailed)?;
        Ok(verified.into_delivery_result())
    }

    pub(crate) fn dispatch_required_channel(
        &self,
        request: GenericDispatchRequest<'_>,
    ) -> Result<RequiredChannelObservation, GenericTransportError> {
        let channel = request.route.required_channel().clone();
        let result = self.dispatch(request)?;
        Ok(RequiredChannelObservation { channel, result })
    }
}

fn build_foundation_envelope(
    snapshot: &IntentSnapshot,
    attested: &super::intent_store::AttestedReadyIntent,
    route: &GenericTransportRoute,
) -> Result<DeliveryEnvelope, GenericTransportError> {
    let rendered = snapshot
        .rendered_bytes()
        .ok_or(GenericTransportError::InvalidBusinessIntent)?
        .to_vec();
    let prepared_snapshot = snapshot
        .prepared_push_bytes()
        .ok_or(GenericTransportError::InvalidBusinessIntent)?
        .to_vec();
    let delivery_subject_hash = canonical_digest(
        "FoundationDeliverySubject/v1",
        &BTreeMap::from([("subject", subject_value(&attested.subject))]),
    );
    let mut envelope = DeliveryEnvelope::new(
        attested.business_date.as_str(),
        route.push_kind,
        route.sub_kind,
        route.scope_key.clone(),
        attested.occurrence.as_str(),
        attested.source_evidence_fingerprint.as_str(),
        prepared_snapshot,
        delivery_subject_hash.as_str(),
        rendered,
        false,
        None,
    )
    .map_err(|_| GenericTransportError::InvalidRoute)?;
    let binding = FoundationDeliveryBinding::try_new(
        namespace_storage(&attested.namespace),
        attested.decision_id.as_str().to_owned(),
        attested.intent_id.as_str().to_owned(),
        attested.unit_id.as_str().to_owned(),
        attested.occurrence.as_str().to_owned(),
        attested.business_date.as_str().to_owned(),
        subject_storage(&attested.subject),
        delivery_subject_hash.as_str().to_owned(),
        attested.audience.as_str().to_owned(),
        route.template.template_id().as_str().to_owned(),
        route.template.template_version().as_str().to_owned(),
        attested.rendered_sha256.as_str().to_owned(),
        attested.source_evidence_fingerprint.as_str().to_owned(),
        route.required_channel().as_str().to_owned(),
    )
    .map_err(|_| GenericTransportError::InvalidBusinessIntent)?;
    envelope = envelope
        .with_foundation_binding(binding)
        .map_err(|_| GenericTransportError::InvalidBusinessIntent)?;
    Ok(envelope)
}

fn namespace_storage(namespace: &Namespace) -> String {
    match namespace {
        Namespace::Production => "Production".to_owned(),
        Namespace::Test { run_id } => format!("Test:{}", run_id.as_str()),
    }
}

fn subject_storage(subject: &SubjectId) -> String {
    match subject {
        SubjectId::Global => "Global".to_owned(),
        SubjectId::Entity(value) => format!("Entity:{}", value.as_str()),
    }
}

struct RequiredChannelSink {
    required_channel: String,
    inner: AuthoritativeSink,
}

impl AuthoritativeSinkPort for RequiredChannelSink {
    fn sink_identity(&self) -> &str {
        &self.required_channel
    }

    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        match self.inner.deliver(request) {
            AuthoritativeSinkResult::Accepted(receipt)
                if receipt.channel != self.required_channel =>
            {
                let evidence = match serde_json::to_vec(&receipt) {
                    Ok(bytes) => bytes,
                    Err(_) => b"required-channel receipt serialization failed".to_vec(),
                };
                AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                    reason_code: REQUIRED_CHANNEL_MISMATCH.to_owned(),
                    evidence,
                    observed_at: receipt.accepted_at,
                })
            }
            result => result,
        }
    }
}

pub(crate) struct GenericTerminalAuthorityAdapter<'a> {
    coordinator: &'a DurableDeliveryCoordinator,
    descriptor: AuthorityDescriptor,
}

impl<'a> GenericTerminalAuthorityAdapter<'a> {
    pub(crate) fn try_new(
        coordinator: &'a DurableDeliveryCoordinator,
    ) -> Result<Self, GenericTransportError> {
        let durable_schema_version = DurableSchemaVersion::try_new(format!(
            "durable-delivery-v{}",
            crate::durable_delivery::DURABLE_SCHEMA_VERSION
        ))
        .map_err(|_| GenericTransportError::DurableFailure)?;
        Ok(Self {
            coordinator,
            descriptor: AuthorityDescriptor {
                authority_class: AuthorityClass::GenericCounted,
                durable_schema_version,
            },
        })
    }
}

impl TerminalAuthorityPort for GenericTerminalAuthorityAdapter<'_> {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }

    fn requery_terminal(
        &self,
        decision_id: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure> {
        match self
            .coordinator
            .inspect_foundation_terminal(decision_id.as_str())
            .map_err(|_| AuthorityQueryFailure)?
        {
            FoundationTerminalQuery::Missing => Ok(AuthorityQuery::Missing),
            FoundationTerminalQuery::PendingSeal { .. } => Ok(AuthorityQuery::PendingSeal),
            FoundationTerminalQuery::Terminal(record) => {
                map_terminal_record(*record).map(AuthorityQuery::Terminal)
            }
        }
    }
}

fn map_terminal_record(
    record: crate::durable_delivery::FoundationTerminalRecord,
) -> Result<Box<AuthorityTerminalRecord>, AuthorityQueryFailure> {
    let terminal_disposition_source = record.disposition();
    let attempt_id = record.attempt_id().map(str::to_owned);
    let evidence_bytes = record.evidence_bytes().to_vec();
    let evidence_sha256_source = record.evidence_sha256().to_owned();
    let durable_schema_version_source = record.durable_schema_version();
    let ref_id = record.ref_id.clone();
    let binding = record.binding;
    let decision_id = DecisionId::try_new(binding.application_decision_id().to_owned())
        .map_err(|_| AuthorityQueryFailure)?;
    let intent_digest = Sha256Digest::parse("foundation intent_id", binding.intent_id())
        .map_err(|_| AuthorityQueryFailure)?;
    let occurrence_digest = Sha256Digest::parse("foundation occurrence", binding.occurrence_id())
        .map_err(|_| AuthorityQueryFailure)?;
    let namespace = parse_namespace(binding.namespace())?;
    let subject = parse_subject(binding.subject())?;
    let attempt_binding = match (attempt_id.as_deref(), terminal_disposition_source) {
        (Some(attempt_id), _) => AuthorityAttemptBinding::Attempt(
            AttemptId::try_new(attempt_id.to_owned()).map_err(|_| AuthorityQueryFailure)?,
        ),
        (None, FoundationTerminalDisposition::Rejected) => {
            AuthorityAttemptBinding::ValidatedPreAttemptRejection
        }
        (None, FoundationTerminalDisposition::ManualAccepted)
        | (None, FoundationTerminalDisposition::ManualNotDelivered) => {
            AuthorityAttemptBinding::ValidatedManualWithoutAttempt
        }
        _ => return Err(AuthorityQueryFailure),
    };
    let terminal_disposition = match terminal_disposition_source {
        FoundationTerminalDisposition::Accepted => TerminalDisposition::Accepted,
        FoundationTerminalDisposition::Rejected => TerminalDisposition::Rejected,
        FoundationTerminalDisposition::Uncertain => TerminalDisposition::Uncertain,
        FoundationTerminalDisposition::ManualAccepted => {
            TerminalDisposition::ManualConfirmedAccepted
        }
        FoundationTerminalDisposition::ManualNotDelivered => {
            TerminalDisposition::ManualConfirmedNotDelivered
        }
    };
    let evidence_sha256 = Sha256Digest::parse("foundation evidence", &evidence_sha256_source)
        .map_err(|_| AuthorityQueryFailure)?;
    let durable_schema_version = DurableSchemaVersion::try_new(format!(
        "durable-delivery-v{}",
        durable_schema_version_source
    ))
    .map_err(|_| AuthorityQueryFailure)?;
    let mut mapped = AuthorityTerminalRecord {
        ref_id: TerminalRefId::try_new(ref_id).map_err(|_| AuthorityQueryFailure)?,
        authority_class: AuthorityClass::GenericCounted,
        namespace,
        decision_id,
        attempt_binding,
        intent_id: IntentId::from_digest(&intent_digest),
        unit_id: UnitId::try_new(binding.unit_id().to_owned())
            .map_err(|_| AuthorityQueryFailure)?,
        occurrence: OccurrenceId::from_digest(&occurrence_digest),
        business_date: BusinessDate::parse(binding.business_date())
            .map_err(|_| AuthorityQueryFailure)?,
        subject,
        audience: AudienceId::try_new(binding.audience().to_owned())
            .map_err(|_| AuthorityQueryFailure)?,
        template_id: TemplateId::try_new(binding.template_id().to_owned())
            .map_err(|_| AuthorityQueryFailure)?,
        template_version: TemplateVersion::try_new(binding.template_version().to_owned())
            .map_err(|_| AuthorityQueryFailure)?,
        rendered_sha256: Sha256Digest::parse("foundation rendered", binding.rendered_sha256())
            .map_err(|_| AuthorityQueryFailure)?,
        terminal_disposition,
        evidence_bytes,
        evidence_sha256,
        durable_schema_version,
        binding_sha256: raw_digest(b"foundation terminal binding pending"),
    };
    mapped.binding_sha256 = terminal_binding_sha256(&mapped);
    Ok(Box::new(mapped))
}

fn parse_namespace(value: &str) -> Result<Namespace, AuthorityQueryFailure> {
    if value == "Production" {
        return Ok(Namespace::Production);
    }
    let run_id = value.strip_prefix("Test:").ok_or(AuthorityQueryFailure)?;
    Ok(Namespace::test(
        RunId::try_new(run_id.to_owned()).map_err(|_| AuthorityQueryFailure)?,
    ))
}

fn parse_subject(value: &str) -> Result<SubjectId, AuthorityQueryFailure> {
    if value == "Global" {
        return Ok(SubjectId::Global);
    }
    let entity = value.strip_prefix("Entity:").ok_or(AuthorityQueryFailure)?;
    SubjectId::entity(entity.to_owned()).map_err(|_| AuthorityQueryFailure)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequiredChannelClassification {
    AllRequiredAccepted,
    PartialRequiredChannels,
    RejectedRequiredChannels,
    UncertainRequiredChannels,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum RequiredChannelError {
    #[error("required channels must not be empty")]
    RequiredChannelsEmpty,
    #[error("required channels must be unique")]
    DuplicateRequiredChannel,
    #[error("one channel has more than one result")]
    DuplicateObservation,
    #[error("observed channels do not exactly match required channels")]
    ChannelSetMismatch,
    #[error("required-channel aggregation accepts only strong terminal authority")]
    StrongAuthorityRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequiredChannelObservation {
    channel: ChannelId,
    result: DeliveryResult,
}

impl RequiredChannelObservation {
    #[cfg(test)]
    pub(crate) fn new(channel: ChannelId, result: DeliveryResult) -> Self {
        Self { channel, result }
    }

    pub(crate) fn channel(&self) -> &ChannelId {
        &self.channel
    }

    pub(crate) fn result(&self) -> &DeliveryResult {
        &self.result
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequiredChannelResults {
    ordered_channels: Vec<ChannelId>,
    ordered_results: Vec<DeliveryResult>,
    classification: RequiredChannelClassification,
}

impl RequiredChannelResults {
    pub(crate) fn try_classify(
        required_channels: Vec<ChannelId>,
        observations: Vec<RequiredChannelObservation>,
    ) -> Result<Self, RequiredChannelError> {
        if required_channels.is_empty() {
            return Err(RequiredChannelError::RequiredChannelsEmpty);
        }
        let required_set = required_channels
            .iter()
            .map(ChannelId::as_str)
            .collect::<BTreeSet<_>>();
        if required_set.len() != required_channels.len() {
            return Err(RequiredChannelError::DuplicateRequiredChannel);
        }
        let mut observed = BTreeMap::new();
        for observation in observations {
            if observed
                .insert(observation.channel.as_str().to_owned(), observation.result)
                .is_some()
            {
                return Err(RequiredChannelError::DuplicateObservation);
            }
        }
        let observed_set = observed.keys().map(String::as_str).collect::<BTreeSet<_>>();
        if observed_set != required_set {
            return Err(RequiredChannelError::ChannelSetMismatch);
        }

        let mut accepted_count = 0usize;
        let mut has_uncertain = false;
        let mut ordered_results = Vec::with_capacity(required_channels.len());
        for required in &required_channels {
            let result = observed
                .remove(required.as_str())
                .ok_or(RequiredChannelError::ChannelSetMismatch)?;
            match classify_strong_channel_result(&result)? {
                StrongChannelStatus::Accepted => accepted_count = accepted_count.saturating_add(1),
                StrongChannelStatus::Rejected => {}
                StrongChannelStatus::Uncertain => has_uncertain = true,
            }
            ordered_results.push(result);
        }
        let classification = if has_uncertain {
            RequiredChannelClassification::UncertainRequiredChannels
        } else if accepted_count == required_channels.len() {
            RequiredChannelClassification::AllRequiredAccepted
        } else if accepted_count > 0 {
            RequiredChannelClassification::PartialRequiredChannels
        } else {
            RequiredChannelClassification::RejectedRequiredChannels
        };
        Ok(Self {
            ordered_channels: required_channels,
            ordered_results,
            classification,
        })
    }

    pub(crate) fn classification(&self) -> RequiredChannelClassification {
        self.classification
    }

    pub(crate) fn ordered_channels(&self) -> &[ChannelId] {
        &self.ordered_channels
    }

    pub(crate) fn ordered_results(&self) -> &[DeliveryResult] {
        &self.ordered_results
    }

    pub(crate) fn completion_eligibility(&self) -> CompletionEligibility {
        match self.classification {
            RequiredChannelClassification::AllRequiredAccepted => {
                CompletionEligibility::PolicyBound
            }
            RequiredChannelClassification::PartialRequiredChannels
            | RequiredChannelClassification::RejectedRequiredChannels
            | RequiredChannelClassification::UncertainRequiredChannels => {
                CompletionEligibility::Never
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StrongChannelStatus {
    Accepted,
    Rejected,
    Uncertain,
}

fn classify_strong_channel_result(
    result: &DeliveryResult,
) -> Result<StrongChannelStatus, RequiredChannelError> {
    match result.view() {
        DeliveryResultView::TransportAccepted(_) => Ok(StrongChannelStatus::Accepted),
        DeliveryResultView::TransportRejected(_) => Ok(StrongChannelStatus::Rejected),
        DeliveryResultView::TransportUncertain(_) => Ok(StrongChannelStatus::Uncertain),
        DeliveryResultView::AlreadyTerminal(terminal) => match terminal.terminal_disposition() {
            TerminalDisposition::ManualConfirmedAccepted => Ok(StrongChannelStatus::Accepted),
            TerminalDisposition::ManualConfirmedNotDelivered => Ok(StrongChannelStatus::Rejected),
            TerminalDisposition::Accepted
            | TerminalDisposition::Rejected
            | TerminalDisposition::Uncertain => Err(RequiredChannelError::StrongAuthorityRequired),
        },
        DeliveryResultView::BestEffortAccepted(_)
        | DeliveryResultView::PartiallyAccepted(_)
        | DeliveryResultView::NoChannelConfigured(_)
        | DeliveryResultView::AllChannelsFailed(_)
        | DeliveryResultView::Blocked(_) => Err(RequiredChannelError::StrongAuthorityRequired),
    }
}
