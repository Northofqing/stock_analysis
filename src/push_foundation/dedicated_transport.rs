//! W13 conformance adapters for dedicated legacy delivery authorities.

#![cfg_attr(not(test), allow(dead_code))]

use serde::Deserialize;

use crate::durable_delivery::{
    CooldownScope, DeliveryEnvelope, DeliverySubKind, DurableDeliveryCoordinator,
    FoundationTerminalDisposition, P01DedicatedTerminalQuery, P01DedicatedTerminalRecord, PushKind,
    TypedReceipt,
};
use crate::monitor::push_job::{
    raw_digest, AttemptId, AuthorityClass, BusinessDate, ChannelId, CompletionPolicy, DecisionId,
    DeliveryResult, DurableSchemaVersion, Sha256Digest, SubjectId, TerminalDisposition,
    TerminalRefId, UtcMicros,
};

use super::terminal_authority::{
    terminal_binding_sha256, verify_terminal, AuthorityAttemptBinding, AuthorityDescriptor,
    AuthorityQuery, AuthorityQueryFailure, AuthorityTerminalRecord, TerminalAuthorityPort,
    TerminalTemplateBinding,
};
use super::IntentSnapshot;

const P01_UNIT_ID: &str = "MU-p01";
const P01_TEMPLATE_ID: &str = "preopen_news_hot_v1";
const P01_SOURCE_BINDING_SCHEMA: &str = "P01_SOURCE_BINDING_V1";

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DedicatedConformanceError {
    #[error("dedicated conformance route is invalid")]
    InvalidRoute,
    #[error("business intent is not an attested dedicated Ready intent")]
    InvalidBusinessIntent,
    #[error("dedicated terminal source is unavailable")]
    SourceUnavailable,
    #[error("dedicated terminal source has no matching authority")]
    TerminalMissing,
    #[error("dedicated terminal source is not durably sealed")]
    TerminalPendingSeal,
    #[error("P01 dedicated authority does not match field {field}")]
    P01BindingMismatch { field: &'static str },
    #[error("P01 dedicated source binding is invalid")]
    InvalidP01SourceBinding,
    #[error("P01 dedicated terminal disposition is invalid")]
    InvalidP01Disposition,
    #[error("P01 dedicated accepted receipt channel is invalid")]
    P01ChannelMismatch,
    #[error("W09 terminal authority verification failed")]
    TerminalVerificationFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DedicatedConformanceRoute {
    template: TerminalTemplateBinding,
    required_channel: ChannelId,
}

impl DedicatedConformanceRoute {
    pub(crate) fn try_new(
        template: TerminalTemplateBinding,
        required_channel: ChannelId,
    ) -> Result<Self, DedicatedConformanceError> {
        if template.template_id().as_str() != P01_TEMPLATE_ID {
            return Err(DedicatedConformanceError::InvalidRoute);
        }
        Ok(Self {
            template,
            required_channel,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DedicatedSourceFailure;

pub(crate) trait P01DedicatedTerminalSource {
    fn requery_p01(
        &self,
        business_date: &BusinessDate,
    ) -> Result<P01DedicatedTerminalQuery, DedicatedSourceFailure>;
}

impl P01DedicatedTerminalSource for DurableDeliveryCoordinator {
    fn requery_p01(
        &self,
        business_date: &BusinessDate,
    ) -> Result<P01DedicatedTerminalQuery, DedicatedSourceFailure> {
        self.inspect_p01_dedicated_terminal(business_date.as_str())
            .map_err(|_| DedicatedSourceFailure)
    }
}

pub(crate) fn verify_p01_dedicated(
    snapshot: &IntentSnapshot,
    route: &DedicatedConformanceRoute,
    policy: &CompletionPolicy,
    source: &dyn P01DedicatedTerminalSource,
    verified_at: UtcMicros,
) -> Result<DeliveryResult, DedicatedConformanceError> {
    let attested = snapshot
        .attested_ready_binding()
        .map_err(|_| DedicatedConformanceError::InvalidBusinessIntent)?;
    if attested.unit_id.as_str() != P01_UNIT_ID {
        return Err(DedicatedConformanceError::P01BindingMismatch { field: "unit_id" });
    }
    if attested.subject != SubjectId::Global {
        return Err(DedicatedConformanceError::P01BindingMismatch { field: "subject" });
    }
    if route.template.template_id().as_str() != P01_TEMPLATE_ID {
        return Err(DedicatedConformanceError::InvalidRoute);
    }

    let source_record = match source
        .requery_p01(&attested.business_date)
        .map_err(|_| DedicatedConformanceError::SourceUnavailable)?
    {
        P01DedicatedTerminalQuery::Missing => {
            return Err(DedicatedConformanceError::TerminalMissing)
        }
        P01DedicatedTerminalQuery::PendingSeal { .. } => {
            return Err(DedicatedConformanceError::TerminalPendingSeal)
        }
        P01DedicatedTerminalQuery::Terminal(record) => *record,
    };
    let mapped = map_p01_terminal(&attested, route, source_record)?;
    let authority = FixedDedicatedAuthority::new(mapped);
    let verified = verify_terminal(snapshot, &route.template, policy, &authority, verified_at)
        .map_err(|_| DedicatedConformanceError::TerminalVerificationFailed)?;
    Ok(verified.into_delivery_result())
}

fn map_p01_terminal(
    attested: &super::intent_store::AttestedReadyIntent,
    route: &DedicatedConformanceRoute,
    source: P01DedicatedTerminalRecord,
) -> Result<AuthorityTerminalRecord, DedicatedConformanceError> {
    if source.durable_schema_version != crate::durable_delivery::DURABLE_SCHEMA_VERSION {
        return Err(DedicatedConformanceError::P01BindingMismatch {
            field: "durable_schema_version",
        });
    }
    if raw_digest(&source.envelope_canonical).as_str() != source.envelope_sha256 {
        return Err(DedicatedConformanceError::P01BindingMismatch {
            field: "envelope_sha256",
        });
    }
    let envelope: DeliveryEnvelope =
        serde_json::from_slice(&source.envelope_canonical).map_err(|_| {
            DedicatedConformanceError::P01BindingMismatch {
                field: "envelope_canonical",
            }
        })?;
    if envelope
        .canonical_bytes()
        .map_err(|_| DedicatedConformanceError::P01BindingMismatch {
            field: "envelope_canonical",
        })?
        != source.envelope_canonical
    {
        return Err(DedicatedConformanceError::P01BindingMismatch {
            field: "envelope_canonical",
        });
    }
    check_p01(
        "foundation_binding",
        envelope.foundation_binding().is_none(),
    )?;
    check_p01(
        "legacy_decision_identity",
        envelope.decision_identity == source.legacy_decision_identity,
    )?;
    check_p01(
        "business_date",
        envelope.business_date == attested.business_date.as_str(),
    )?;
    check_p01("push_kind", envelope.push_kind == PushKind::PreopenNewsHot)?;
    check_p01("sub_kind", envelope.sub_kind == DeliverySubKind::None)?;
    check_p01(
        "cooldown_scope",
        envelope.cooldown_scope == CooldownScope::Global,
    )?;
    check_p01("scope_key", envelope.scope_key == "GLOBAL")?;
    check_p01(
        "occurrence",
        envelope.schedule_occurrence_identity == format!("p01:{}", attested.business_date.as_str()),
    )?;
    check_p01(
        "source_evidence_fingerprint",
        envelope.source_evidence_fingerprint == attested.source_evidence_fingerprint.as_str(),
    )?;
    check_p01(
        "rendered_sha256",
        envelope.rendered_content_sha256 == attested.rendered_sha256.as_str(),
    )?;
    validate_p01_source_binding(&envelope.source_binding_canonical)?;

    if raw_digest(&source.evidence_bytes).as_str() != source.evidence_sha256 {
        return Err(DedicatedConformanceError::P01BindingMismatch {
            field: "evidence_sha256",
        });
    }
    let attempt_binding = map_attempt_binding(source.attempt_id.as_deref(), source.disposition)?;
    match source.disposition {
        FoundationTerminalDisposition::Accepted => {
            if source.accepted_channel.as_deref() != Some(route.required_channel.as_str()) {
                return Err(DedicatedConformanceError::P01ChannelMismatch);
            }
            validate_accepted_channel(&source.evidence_bytes, &route.required_channel)?;
        }
        FoundationTerminalDisposition::ManualAccepted => {
            if source
                .accepted_channel
                .as_deref()
                .is_some_and(|channel| channel != route.required_channel.as_str())
            {
                return Err(DedicatedConformanceError::P01ChannelMismatch);
            }
        }
        FoundationTerminalDisposition::Rejected
        | FoundationTerminalDisposition::Uncertain
        | FoundationTerminalDisposition::ManualNotDelivered => {
            if source.accepted_channel.is_some() {
                return Err(DedicatedConformanceError::InvalidP01Disposition);
            }
        }
    }
    let terminal_disposition = map_disposition(source.disposition);
    let evidence_sha256 = Sha256Digest::parse("P01 terminal evidence", &source.evidence_sha256)
        .map_err(|_| DedicatedConformanceError::P01BindingMismatch {
            field: "evidence_sha256",
        })?;
    let durable_schema_version =
        DurableSchemaVersion::try_new(format!("p01-durable-v{}", source.durable_schema_version))
            .map_err(|_| DedicatedConformanceError::P01BindingMismatch {
                field: "durable_schema_version",
            })?;
    let mut mapped = AuthorityTerminalRecord {
        ref_id: TerminalRefId::try_new(source.ref_id)
            .map_err(|_| DedicatedConformanceError::P01BindingMismatch { field: "ref_id" })?,
        authority_class: AuthorityClass::P01Dedicated,
        namespace: attested.namespace.clone(),
        decision_id: attested.decision_id.clone(),
        attempt_binding,
        intent_id: attested.intent_id.clone(),
        unit_id: attested.unit_id.clone(),
        occurrence: attested.occurrence.clone(),
        business_date: attested.business_date.clone(),
        subject: attested.subject.clone(),
        audience: attested.audience.clone(),
        template_id: route.template.template_id().clone(),
        template_version: route.template.template_version().clone(),
        rendered_sha256: attested.rendered_sha256.clone(),
        terminal_disposition,
        evidence_bytes: source.evidence_bytes,
        evidence_sha256,
        durable_schema_version,
        binding_sha256: raw_digest(b"P01 terminal binding pending"),
    };
    mapped.binding_sha256 = terminal_binding_sha256(&mapped);
    Ok(mapped)
}

fn check_p01(field: &'static str, valid: bool) -> Result<(), DedicatedConformanceError> {
    if !valid {
        return Err(DedicatedConformanceError::P01BindingMismatch { field });
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct P01SourceBinding {
    schema_version: String,
    render_mode: P01RenderMode,
}

#[derive(Deserialize)]
enum P01RenderMode {
    Scheduled,
    Compensation,
}

fn validate_p01_source_binding(bytes: &[u8]) -> Result<(), DedicatedConformanceError> {
    let binding: P01SourceBinding = serde_json::from_slice(bytes)
        .map_err(|_| DedicatedConformanceError::InvalidP01SourceBinding)?;
    if binding.schema_version != P01_SOURCE_BINDING_SCHEMA {
        return Err(DedicatedConformanceError::InvalidP01SourceBinding);
    }
    match binding.render_mode {
        P01RenderMode::Scheduled | P01RenderMode::Compensation => Ok(()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedEvidence {
    kind: String,
    receipt: TypedReceipt,
}

fn validate_accepted_channel(
    bytes: &[u8],
    required_channel: &ChannelId,
) -> Result<(), DedicatedConformanceError> {
    let evidence: AcceptedEvidence =
        serde_json::from_slice(bytes).map_err(|_| DedicatedConformanceError::P01ChannelMismatch)?;
    if evidence.kind != "Accepted" || evidence.receipt.channel != required_channel.as_str() {
        return Err(DedicatedConformanceError::P01ChannelMismatch);
    }
    Ok(())
}

fn map_attempt_binding(
    attempt_id: Option<&str>,
    disposition: FoundationTerminalDisposition,
) -> Result<AuthorityAttemptBinding, DedicatedConformanceError> {
    match (attempt_id, disposition) {
        (Some(attempt_id), _) => Ok(AuthorityAttemptBinding::Attempt(
            AttemptId::try_new(attempt_id.to_owned())
                .map_err(|_| DedicatedConformanceError::InvalidP01Disposition)?,
        )),
        (None, FoundationTerminalDisposition::Rejected) => {
            Ok(AuthorityAttemptBinding::ValidatedPreAttemptRejection)
        }
        (None, FoundationTerminalDisposition::ManualAccepted)
        | (None, FoundationTerminalDisposition::ManualNotDelivered) => {
            Ok(AuthorityAttemptBinding::ValidatedManualWithoutAttempt)
        }
        _ => Err(DedicatedConformanceError::InvalidP01Disposition),
    }
}

fn map_disposition(disposition: FoundationTerminalDisposition) -> TerminalDisposition {
    match disposition {
        FoundationTerminalDisposition::Accepted => TerminalDisposition::Accepted,
        FoundationTerminalDisposition::Rejected => TerminalDisposition::Rejected,
        FoundationTerminalDisposition::Uncertain => TerminalDisposition::Uncertain,
        FoundationTerminalDisposition::ManualAccepted => {
            TerminalDisposition::ManualConfirmedAccepted
        }
        FoundationTerminalDisposition::ManualNotDelivered => {
            TerminalDisposition::ManualConfirmedNotDelivered
        }
    }
}

struct FixedDedicatedAuthority {
    descriptor: AuthorityDescriptor,
    record: AuthorityTerminalRecord,
}

impl FixedDedicatedAuthority {
    fn new(record: AuthorityTerminalRecord) -> Self {
        Self {
            descriptor: AuthorityDescriptor {
                authority_class: AuthorityClass::P01Dedicated,
                durable_schema_version: record.durable_schema_version.clone(),
            },
            record,
        }
    }
}

impl TerminalAuthorityPort for FixedDedicatedAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }

    fn requery_terminal(
        &self,
        decision_id: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure> {
        if decision_id != &self.record.decision_id {
            return Err(AuthorityQueryFailure);
        }
        Ok(AuthorityQuery::Terminal(Box::new(self.record.clone())))
    }
}
