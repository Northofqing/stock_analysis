//! W09 exact terminal-authority verification. Concrete durable adapters are added in W12.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;

#[cfg(test)]
use crate::monitor::push_job::canonical_preimage;
use crate::monitor::push_job::{
    canonical_digest, namespace_value, raw_digest, subject_value, AttemptId, AudienceId,
    AuthorityClass, BusinessDate, CanonicalValue, CompletionPolicy, DecisionId,
    DurableSchemaVersion, IntentId, Namespace, OccurrenceId, Sha256Digest, SubjectId, TemplateId,
    TemplateVersion, TerminalDisposition, TerminalRefId, UnitId, UtcMicros, VerifiedTerminalParts,
    VerifiedTerminalRef,
};

use super::IntentSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalTemplateBinding {
    template_id: TemplateId,
    template_version: TemplateVersion,
    sha256: Sha256Digest,
}

impl TerminalTemplateBinding {
    pub fn new(template_id: TemplateId, template_version: TemplateVersion) -> Self {
        let sha256 = canonical_digest(
            "TemplateBinding/v1",
            &BTreeMap::from([
                (
                    "template_id",
                    CanonicalValue::String(template_id.as_str().to_owned()),
                ),
                (
                    "template_version",
                    CanonicalValue::String(template_version.as_str().to_owned()),
                ),
            ]),
        );
        Self {
            template_id,
            template_version,
            sha256,
        }
    }

    pub fn template_id(&self) -> &TemplateId {
        &self.template_id
    }

    pub fn template_version(&self) -> &TemplateVersion {
        &self.template_version
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorityDescriptor {
    pub(crate) authority_class: AuthorityClass,
    pub(crate) durable_schema_version: DurableSchemaVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorityTerminalRecord {
    pub(crate) ref_id: TerminalRefId,
    pub(crate) authority_class: AuthorityClass,
    pub(crate) namespace: Namespace,
    pub(crate) decision_id: DecisionId,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) intent_id: IntentId,
    pub(crate) unit_id: UnitId,
    pub(crate) occurrence: OccurrenceId,
    pub(crate) business_date: BusinessDate,
    pub(crate) subject: SubjectId,
    pub(crate) audience: AudienceId,
    pub(crate) template_id: TemplateId,
    pub(crate) template_version: TemplateVersion,
    pub(crate) rendered_sha256: Sha256Digest,
    pub(crate) terminal_disposition: TerminalDisposition,
    pub(crate) evidence_bytes: Vec<u8>,
    pub(crate) evidence_sha256: Sha256Digest,
    pub(crate) durable_schema_version: DurableSchemaVersion,
    pub(crate) binding_sha256: Sha256Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityQuery {
    Missing,
    PendingSeal,
    Terminal(AuthorityTerminalRecord),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthorityQueryFailure;

pub(crate) trait TerminalAuthorityPort {
    fn descriptor(&self) -> &AuthorityDescriptor;
    fn requery_terminal(
        &self,
        decision_id: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure>;
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum TerminalAuthorityError {
    #[error("business intent cannot provide an attested Ready terminal binding")]
    IntentSnapshotInvalid,
    #[error("terminal template does not match the immutable business intent")]
    TemplateBindingMismatch,
    #[error("completion policy does not match the immutable business intent field {field}")]
    CompletionPolicyMismatch { field: &'static str },
    #[error("terminal authority is not allowed by the registered completion policy")]
    AuthorityNotAllowed,
    #[error("terminal authority query is unavailable")]
    AuthorityUnavailable,
    #[error("terminal authority has no matching decision")]
    TerminalMissing,
    #[error("terminal authority decision is not durably sealed")]
    TerminalPendingSeal,
    #[error("terminal authority descriptor does not match field {field}")]
    AuthorityDescriptorMismatch { field: &'static str },
    #[error("terminal authority record does not match field {field}")]
    BindingMismatch { field: &'static str },
    #[error("terminal authority evidence bytes do not match their SHA-256")]
    EvidenceHashMismatch,
    #[error("terminal authority binding does not match its canonical SHA-256")]
    TerminalBindingHashMismatch,
    #[error("transport terminal disposition requires an attempt identity")]
    AttemptRequired,
}

pub(crate) fn verify_terminal(
    snapshot: &IntentSnapshot,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
) -> Result<VerifiedTerminalRef, TerminalAuthorityError> {
    let expected = snapshot
        .attested_ready_binding()
        .map_err(|_| TerminalAuthorityError::IntentSnapshotInvalid)?;
    if template.sha256 != expected.template_sha256 {
        return Err(TerminalAuthorityError::TemplateBindingMismatch);
    }
    if policy.completion_owner().unit_id() != &expected.unit_id {
        return Err(TerminalAuthorityError::CompletionPolicyMismatch { field: "unit_id" });
    }
    if policy.completion_owner().completion_owner() != &expected.completion_owner {
        return Err(TerminalAuthorityError::CompletionPolicyMismatch {
            field: "completion_owner",
        });
    }

    let descriptor = authority.descriptor();
    if !policy.allows_authority(descriptor.authority_class) {
        return Err(TerminalAuthorityError::AuthorityNotAllowed);
    }
    let record = match authority
        .requery_terminal(&expected.decision_id)
        .map_err(|_| TerminalAuthorityError::AuthorityUnavailable)?
    {
        AuthorityQuery::Missing => return Err(TerminalAuthorityError::TerminalMissing),
        AuthorityQuery::PendingSeal => return Err(TerminalAuthorityError::TerminalPendingSeal),
        AuthorityQuery::Terminal(record) => record,
    };

    if record.authority_class != descriptor.authority_class {
        return Err(TerminalAuthorityError::AuthorityDescriptorMismatch {
            field: "authority_class",
        });
    }
    if record.durable_schema_version != descriptor.durable_schema_version {
        return Err(TerminalAuthorityError::AuthorityDescriptorMismatch {
            field: "durable_schema_version",
        });
    }

    check_binding("namespace", &record.namespace, &expected.namespace)?;
    check_binding("decision_id", &record.decision_id, &expected.decision_id)?;
    check_binding("intent_id", &record.intent_id, &expected.intent_id)?;
    check_binding("unit_id", &record.unit_id, &expected.unit_id)?;
    check_binding("occurrence", &record.occurrence, &expected.occurrence)?;
    check_binding(
        "business_date",
        &record.business_date,
        &expected.business_date,
    )?;
    check_binding("subject", &record.subject, &expected.subject)?;
    check_binding("audience", &record.audience, &expected.audience)?;
    check_binding("template_id", &record.template_id, &template.template_id)?;
    check_binding(
        "template_version",
        &record.template_version,
        &template.template_version,
    )?;
    check_binding(
        "rendered_sha256",
        &record.rendered_sha256,
        &expected.rendered_sha256,
    )?;

    if raw_digest(&record.evidence_bytes) != record.evidence_sha256 {
        return Err(TerminalAuthorityError::EvidenceHashMismatch);
    }
    if matches!(
        record.terminal_disposition,
        TerminalDisposition::Accepted
            | TerminalDisposition::Rejected
            | TerminalDisposition::Uncertain
    ) && record.attempt_id.is_none()
    {
        return Err(TerminalAuthorityError::AttemptRequired);
    }
    let computed_binding = terminal_binding_sha256(&record);
    if computed_binding != record.binding_sha256 {
        return Err(TerminalAuthorityError::TerminalBindingHashMismatch);
    }

    Ok(VerifiedTerminalRef::from_verified_parts(
        VerifiedTerminalParts {
            ref_id: record.ref_id,
            authority_class: record.authority_class,
            namespace: record.namespace,
            decision_id: record.decision_id,
            attempt_id: record.attempt_id,
            intent_id: record.intent_id,
            unit_id: record.unit_id,
            occurrence: record.occurrence,
            business_date: record.business_date,
            subject: record.subject,
            audience: record.audience,
            template_id: record.template_id,
            template_version: record.template_version,
            rendered_sha256: record.rendered_sha256,
            terminal_disposition: record.terminal_disposition,
            evidence_sha256: record.evidence_sha256,
            durable_schema_version: record.durable_schema_version,
            verified_at,
            binding_sha256: computed_binding,
        },
    ))
}

fn check_binding<T: Eq>(
    field: &'static str,
    actual: &T,
    expected: &T,
) -> Result<(), TerminalAuthorityError> {
    if actual != expected {
        return Err(TerminalAuthorityError::BindingMismatch { field });
    }
    Ok(())
}

pub(crate) fn terminal_binding_sha256(record: &AuthorityTerminalRecord) -> Sha256Digest {
    canonical_digest("TerminalBinding/v1", &terminal_binding_fields(record))
}

#[cfg(test)]
pub(crate) fn terminal_binding_preimage_for_test(record: &AuthorityTerminalRecord) -> Vec<u8> {
    canonical_preimage("TerminalBinding/v1", &terminal_binding_fields(record))
}

fn terminal_binding_fields(
    record: &AuthorityTerminalRecord,
) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "ref_id",
            CanonicalValue::String(record.ref_id.as_str().to_owned()),
        ),
        (
            "authority_class",
            CanonicalValue::String(authority_class_name(record.authority_class).to_owned()),
        ),
        ("namespace", namespace_value(&record.namespace)),
        (
            "decision_id",
            CanonicalValue::String(record.decision_id.as_str().to_owned()),
        ),
        (
            "attempt_id",
            record
                .attempt_id
                .as_ref()
                .map_or(CanonicalValue::Null, |id| {
                    CanonicalValue::String(id.as_str().to_owned())
                }),
        ),
        (
            "intent_id",
            CanonicalValue::String(record.intent_id.as_str().to_owned()),
        ),
        (
            "unit_id",
            CanonicalValue::String(record.unit_id.as_str().to_owned()),
        ),
        (
            "occurrence",
            CanonicalValue::String(record.occurrence.as_str().to_owned()),
        ),
        (
            "business_date",
            CanonicalValue::String(record.business_date.as_str().to_owned()),
        ),
        ("subject", subject_value(&record.subject)),
        (
            "audience",
            CanonicalValue::String(record.audience.as_str().to_owned()),
        ),
        (
            "template_id",
            CanonicalValue::String(record.template_id.as_str().to_owned()),
        ),
        (
            "template_version",
            CanonicalValue::String(record.template_version.as_str().to_owned()),
        ),
        (
            "rendered_sha256",
            CanonicalValue::String(record.rendered_sha256.as_str().to_owned()),
        ),
        (
            "terminal_disposition",
            CanonicalValue::String(
                terminal_disposition_name(record.terminal_disposition).to_owned(),
            ),
        ),
        (
            "evidence_sha256",
            CanonicalValue::String(record.evidence_sha256.as_str().to_owned()),
        ),
        (
            "durable_schema_version",
            CanonicalValue::String(record.durable_schema_version.as_str().to_owned()),
        ),
    ])
}

fn authority_class_name(authority: AuthorityClass) -> &'static str {
    match authority {
        AuthorityClass::GenericCounted => "GenericCounted",
        AuthorityClass::P01Dedicated => "P01Dedicated",
        AuthorityClass::N02Dedicated => "N02Dedicated",
    }
}

fn terminal_disposition_name(disposition: TerminalDisposition) -> &'static str {
    match disposition {
        TerminalDisposition::Accepted => "Accepted",
        TerminalDisposition::Rejected => "Rejected",
        TerminalDisposition::Uncertain => "Uncertain",
        TerminalDisposition::ManualConfirmedAccepted => "ManualConfirmedAccepted",
        TerminalDisposition::ManualConfirmedNotDelivered => "ManualConfirmedNotDelivered",
    }
}
