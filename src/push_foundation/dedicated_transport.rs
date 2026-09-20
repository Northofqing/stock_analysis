//! W13 conformance adapters for dedicated legacy delivery authorities.

#![cfg_attr(not(test), allow(dead_code))]

use serde::Deserialize;

use crate::durable_delivery::{
    CooldownScope, DeliveryEnvelope, DeliverySubKind, DurableDeliveryCoordinator,
    FoundationTerminalDisposition, P01DedicatedTerminalQuery, P01DedicatedTerminalRecord, PushKind,
    TypedReceipt,
};
use crate::event::dispatcher::AuditAuthorityResourceBinding;
use crate::event::envelope::{
    news_flash_evidence_sha256, NewsFlashTransactionStage, NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION,
};
use crate::event::{
    requery_news_flash_window_terminal_bound_with, requery_news_flash_window_terminal_with,
    AuditDispatcher, EventEnvelope, NewsFlashWindow, NewsFlashWindowTerminalQuery,
    NewsFlashWindowTerminalRecord, PushRecord,
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
const N02_UNIT_ID: &str = "MU-news-flash-aggregate";
const N02_TEMPLATE_ID: &str = "news_flash_aggregated_v1";
const N02_DURABLE_SCHEMA_VERSION: &str = "news-flash-authority-v5";

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
    #[error("N02 dedicated authority does not match field {field}")]
    N02BindingMismatch { field: &'static str },
    #[error("N02 dedicated terminal disposition is invalid")]
    InvalidN02Disposition,
    #[error("N02 dedicated accepted receipt channel is invalid")]
    N02ChannelMismatch,
    #[error("W09 terminal authority verification failed")]
    TerminalVerificationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DedicatedSpecialty {
    P01,
    N02,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DedicatedConformanceRoute {
    specialty: DedicatedSpecialty,
    template: TerminalTemplateBinding,
    required_channel: ChannelId,
}

impl DedicatedConformanceRoute {
    pub(super) fn authority_class(&self) -> AuthorityClass {
        match self.specialty {
            DedicatedSpecialty::P01 => AuthorityClass::P01Dedicated,
            DedicatedSpecialty::N02 => AuthorityClass::N02Dedicated,
        }
    }

    pub(super) fn authority_descriptor(
        &self,
    ) -> Result<AuthorityDescriptor, DedicatedConformanceError> {
        let version = match self.specialty {
            DedicatedSpecialty::P01 => format!(
                "p01-durable-v{}",
                crate::durable_delivery::DURABLE_SCHEMA_VERSION
            ),
            DedicatedSpecialty::N02 => N02_DURABLE_SCHEMA_VERSION.to_owned(),
        };
        Ok(AuthorityDescriptor {
            authority_class: self.authority_class(),
            durable_schema_version: DurableSchemaVersion::try_new(version)
                .map_err(|_| DedicatedConformanceError::InvalidRoute)?,
        })
    }

    #[cfg(test)]
    pub(super) fn template(&self) -> &TerminalTemplateBinding {
        &self.template
    }

    pub(super) fn required_channel(&self) -> &ChannelId {
        &self.required_channel
    }

    pub(crate) fn matches_sla_route(
        &self,
        class: AuthorityClass,
        template: &TerminalTemplateBinding,
    ) -> bool {
        self.template == *template
            && matches!(
                (self.specialty, class),
                (DedicatedSpecialty::P01, AuthorityClass::P01Dedicated)
                    | (DedicatedSpecialty::N02, AuthorityClass::N02Dedicated)
            )
    }

    pub(crate) fn try_new(
        template: TerminalTemplateBinding,
        required_channel: ChannelId,
    ) -> Result<Self, DedicatedConformanceError> {
        let specialty = match template.template_id().as_str() {
            P01_TEMPLATE_ID => DedicatedSpecialty::P01,
            N02_TEMPLATE_ID => DedicatedSpecialty::N02,
            _ => return Err(DedicatedConformanceError::InvalidRoute),
        };
        Ok(Self {
            specialty,
            template,
            required_channel,
        })
    }
}

pub(super) fn requery_p01_dedicated_authority(
    snapshot: &IntentSnapshot,
    route: &DedicatedConformanceRoute,
    source: &DurableDeliveryCoordinator,
) -> Result<AuthorityQuery, AuthorityQueryFailure> {
    match inspect_p01_dedicated(snapshot, route, source) {
        Ok(record) => Ok(AuthorityQuery::Terminal(Box::new(record))),
        Err(DedicatedConformanceError::TerminalMissing) => Ok(AuthorityQuery::Missing),
        Err(DedicatedConformanceError::TerminalPendingSeal) => Ok(AuthorityQuery::PendingSeal),
        Err(_) => Err(AuthorityQueryFailure),
    }
}

pub(super) fn requery_n02_dedicated_authority(
    snapshot: &IntentSnapshot,
    window: NewsFlashWindow,
    route: &DedicatedConformanceRoute,
    source: &AuditDispatcher,
    binding: &AuditAuthorityResourceBinding,
) -> Result<AuthorityQuery, AuthorityQueryFailure> {
    let bound_source = BoundN02AuthoritySource {
        dispatcher: source,
        binding,
    };
    match inspect_n02_dedicated(snapshot, window, route, &bound_source) {
        Ok(record) => Ok(AuthorityQuery::Terminal(Box::new(record))),
        Err(DedicatedConformanceError::TerminalMissing) => Ok(AuthorityQuery::Missing),
        Err(DedicatedConformanceError::TerminalPendingSeal) => Ok(AuthorityQuery::PendingSeal),
        Err(_) => Err(AuthorityQueryFailure),
    }
}

struct BoundN02AuthoritySource<'a> {
    dispatcher: &'a AuditDispatcher,
    binding: &'a AuditAuthorityResourceBinding,
}

impl N02DedicatedTerminalSource for BoundN02AuthoritySource<'_> {
    fn requery_n02(
        &self,
        business_date: &BusinessDate,
        window: NewsFlashWindow,
    ) -> Result<NewsFlashWindowTerminalQuery, DedicatedSourceFailure> {
        let business_date = chrono::NaiveDate::parse_from_str(business_date.as_str(), "%Y-%m-%d")
            .map_err(|_| DedicatedSourceFailure)?;
        requery_news_flash_window_terminal_bound_with(
            self.dispatcher,
            self.binding,
            business_date,
            window,
        )
        .map_err(|_| DedicatedSourceFailure)
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

pub(crate) trait N02DedicatedTerminalSource {
    fn requery_n02(
        &self,
        business_date: &BusinessDate,
        window: NewsFlashWindow,
    ) -> Result<NewsFlashWindowTerminalQuery, DedicatedSourceFailure>;
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

impl N02DedicatedTerminalSource for AuditDispatcher {
    fn requery_n02(
        &self,
        business_date: &BusinessDate,
        window: NewsFlashWindow,
    ) -> Result<NewsFlashWindowTerminalQuery, DedicatedSourceFailure> {
        let business_date = chrono::NaiveDate::parse_from_str(business_date.as_str(), "%Y-%m-%d")
            .map_err(|_| DedicatedSourceFailure)?;
        requery_news_flash_window_terminal_with(self, business_date, window)
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
    let mapped = inspect_p01_dedicated(snapshot, route, source)?;
    let authority = FixedDedicatedAuthority::new(mapped);
    let verified = verify_terminal(snapshot, &route.template, policy, &authority, verified_at)
        .map_err(|_| DedicatedConformanceError::TerminalVerificationFailed)?;
    Ok(verified.into_delivery_result())
}

pub(crate) fn inspect_p01_dedicated(
    snapshot: &IntentSnapshot,
    route: &DedicatedConformanceRoute,
    source: &dyn P01DedicatedTerminalSource,
) -> Result<AuthorityTerminalRecord, DedicatedConformanceError> {
    let attested = snapshot
        .attested_ready_binding()
        .map_err(|_| DedicatedConformanceError::InvalidBusinessIntent)?;
    if attested.unit_id.as_str() != P01_UNIT_ID {
        return Err(DedicatedConformanceError::P01BindingMismatch { field: "unit_id" });
    }
    if attested.subject != SubjectId::Global {
        return Err(DedicatedConformanceError::P01BindingMismatch { field: "subject" });
    }
    if route.specialty != DedicatedSpecialty::P01
        || route.template.template_id().as_str() != P01_TEMPLATE_ID
    {
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
    map_p01_terminal(&attested, route, source_record)
}

pub(crate) fn verify_n02_dedicated(
    snapshot: &IntentSnapshot,
    window: NewsFlashWindow,
    route: &DedicatedConformanceRoute,
    policy: &CompletionPolicy,
    source: &dyn N02DedicatedTerminalSource,
    verified_at: UtcMicros,
) -> Result<DeliveryResult, DedicatedConformanceError> {
    let mapped = inspect_n02_dedicated(snapshot, window, route, source)?;
    let authority = FixedDedicatedAuthority::new(mapped);
    let verified = verify_terminal(snapshot, &route.template, policy, &authority, verified_at)
        .map_err(|_| DedicatedConformanceError::TerminalVerificationFailed)?;
    Ok(verified.into_delivery_result())
}

pub(crate) fn inspect_n02_dedicated(
    snapshot: &IntentSnapshot,
    window: NewsFlashWindow,
    route: &DedicatedConformanceRoute,
    source: &dyn N02DedicatedTerminalSource,
) -> Result<AuthorityTerminalRecord, DedicatedConformanceError> {
    let attested = snapshot
        .attested_ready_binding()
        .map_err(|_| DedicatedConformanceError::InvalidBusinessIntent)?;
    if attested.unit_id.as_str() != N02_UNIT_ID {
        return Err(DedicatedConformanceError::N02BindingMismatch { field: "unit_id" });
    }
    if attested.subject != SubjectId::Global {
        return Err(DedicatedConformanceError::N02BindingMismatch { field: "subject" });
    }
    if route.specialty != DedicatedSpecialty::N02
        || route.template.template_id().as_str() != N02_TEMPLATE_ID
    {
        return Err(DedicatedConformanceError::InvalidRoute);
    }
    let rendered_len = snapshot
        .rendered_bytes()
        .ok_or(DedicatedConformanceError::InvalidBusinessIntent)?
        .len();

    let source_record = match source
        .requery_n02(&attested.business_date, window)
        .map_err(|_| DedicatedConformanceError::SourceUnavailable)?
    {
        NewsFlashWindowTerminalQuery::Missing => {
            return Err(DedicatedConformanceError::TerminalMissing)
        }
        NewsFlashWindowTerminalQuery::PendingSeal => {
            return Err(DedicatedConformanceError::TerminalPendingSeal)
        }
        NewsFlashWindowTerminalQuery::Terminal(record) => *record,
    };
    map_n02_terminal(&attested, rendered_len, window, route, source_record)
}

fn map_n02_terminal(
    attested: &super::intent_store::AttestedReadyIntent,
    rendered_len: usize,
    window: NewsFlashWindow,
    route: &DedicatedConformanceRoute,
    source: NewsFlashWindowTerminalRecord,
) -> Result<AuthorityTerminalRecord, DedicatedConformanceError> {
    let attempt_bytes = canonical_n02_envelope(&source.attempt)?;
    let terminal_bytes = canonical_n02_envelope(&source.terminal)?;
    let attempt = PushRecord::try_from_authoritative(&source.attempt).map_err(|_| {
        DedicatedConformanceError::N02BindingMismatch {
            field: "attempt_envelope",
        }
    })?;
    let terminal = PushRecord::try_from_authoritative(&source.terminal).map_err(|_| {
        DedicatedConformanceError::N02BindingMismatch {
            field: "terminal_envelope",
        }
    })?;
    check_n02(
        "audit_schema_version",
        attempt.audit_schema_version == Some(NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION)
            && terminal.audit_schema_version == Some(NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION),
    )?;
    check_n02(
        "kind",
        attempt.kind == N02_TEMPLATE_ID && terminal.kind == N02_TEMPLATE_ID,
    )?;
    let expected_date =
        chrono::NaiveDate::parse_from_str(attested.business_date.as_str(), "%Y-%m-%d")
            .map_err(|_| DedicatedConformanceError::InvalidBusinessIntent)?;
    let expected_key = window.decision_key();
    check_n02(
        "business_date",
        attempt.news_flash_business_date == Some(expected_date)
            && terminal.news_flash_business_date == Some(expected_date),
    )?;
    check_n02(
        "decision_key",
        attempt.news_flash_decision_key.as_deref() == Some(expected_key.as_str())
            && terminal.news_flash_decision_key.as_deref() == Some(expected_key.as_str()),
    )?;
    check_n02(
        "attempt_stage",
        attempt.news_flash_transaction_stage.as_deref()
            == Some(NewsFlashTransactionStage::SinkAttempt.as_str()),
    )?;
    let terminal_stage = terminal
        .news_flash_transaction_stage
        .as_deref()
        .and_then(NewsFlashTransactionStage::parse)
        .ok_or(DedicatedConformanceError::InvalidN02Disposition)?;
    if terminal_stage == NewsFlashTransactionStage::SinkAttempt {
        return Err(DedicatedConformanceError::InvalidN02Disposition);
    }
    check_n02(
        "reservation_sha256",
        attempt.news_flash_reservation_sha256 == terminal.news_flash_reservation_sha256,
    )?;
    check_n02(
        "attempt_ordinal",
        attempt.news_flash_attempt_ordinal == terminal.news_flash_attempt_ordinal,
    )?;
    check_n02(
        "attempt_observed_at",
        attempt.news_flash_attempt_observed_at == terminal.news_flash_attempt_observed_at,
    )?;
    check_n02(
        "sink_attempt_identity",
        attempt.news_flash_sink_attempt_identity == terminal.news_flash_sink_attempt_identity,
    )?;
    check_n02(
        "sink_attempt_sha256",
        attempt.news_flash_sink_attempt_sha256 == terminal.news_flash_sink_attempt_sha256,
    )?;
    check_n02(
        "attempt_envelope_id",
        terminal.news_flash_attempt_envelope_id.as_deref() == Some(source.attempt.id.as_str()),
    )?;
    check_n02(
        "ordered_sources",
        attempt.news_flash_sources == terminal.news_flash_sources,
    )?;
    let sources = terminal.news_flash_sources.as_deref().ok_or(
        DedicatedConformanceError::N02BindingMismatch {
            field: "ordered_sources",
        },
    )?;
    check_n02(
        "source_evidence_sha256",
        terminal.news_flash_evidence_sha256.as_deref()
            == Some(news_flash_evidence_sha256(sources).as_str())
            && attempt.news_flash_evidence_sha256 == terminal.news_flash_evidence_sha256,
    )?;
    check_n02(
        "render_sha256",
        attempt.news_flash_render_sha256 == terminal.news_flash_render_sha256
            && terminal.news_flash_render_sha256.as_deref()
                == Some(attested.rendered_sha256.as_str()),
    )?;
    check_n02(
        "rendered_len",
        attempt.rendered_len == rendered_len && terminal.rendered_len == rendered_len,
    )?;
    check_n02(
        "channel",
        attempt.channel == terminal.channel && terminal.channel == route.required_channel.as_str(),
    )?;

    let terminal_disposition = match terminal_stage {
        NewsFlashTransactionStage::Accepted => {
            let receipt = terminal
                .news_flash_remote_receipt
                .as_ref()
                .ok_or(DedicatedConformanceError::InvalidN02Disposition)?;
            if receipt.channel != route.required_channel.as_str()
                || receipt.channel != attempt.channel
            {
                return Err(DedicatedConformanceError::N02ChannelMismatch);
            }
            TerminalDisposition::Accepted
        }
        NewsFlashTransactionStage::DefinitivelyRejected => TerminalDisposition::Rejected,
        NewsFlashTransactionStage::Uncertain => TerminalDisposition::Uncertain,
        NewsFlashTransactionStage::SinkAttempt => {
            return Err(DedicatedConformanceError::InvalidN02Disposition)
        }
    };
    let evidence_sha256 = raw_digest(&terminal_bytes);
    let durable_schema_version =
        DurableSchemaVersion::try_new(N02_DURABLE_SCHEMA_VERSION.to_owned()).map_err(|_| {
            DedicatedConformanceError::N02BindingMismatch {
                field: "durable_schema_version",
            }
        })?;
    let mut mapped = AuthorityTerminalRecord {
        ref_id: TerminalRefId::try_new(source.terminal.id)
            .map_err(|_| DedicatedConformanceError::N02BindingMismatch { field: "ref_id" })?,
        authority_class: AuthorityClass::N02Dedicated,
        namespace: attested.namespace.clone(),
        decision_id: attested.decision_id.clone(),
        attempt_binding: AuthorityAttemptBinding::Attempt(
            AttemptId::try_new(source.attempt.id).map_err(|_| {
                DedicatedConformanceError::N02BindingMismatch {
                    field: "attempt_envelope_id",
                }
            })?,
        ),
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
        evidence_bytes: terminal_bytes,
        evidence_sha256,
        durable_schema_version,
        binding_sha256: raw_digest(b"N02 terminal binding pending"),
    };
    mapped.binding_sha256 = terminal_binding_sha256(&mapped);
    drop(attempt_bytes);
    Ok(mapped)
}

fn canonical_n02_envelope(envelope: &EventEnvelope) -> Result<Vec<u8>, DedicatedConformanceError> {
    let bytes = serde_json::to_vec(envelope).map_err(|_| {
        DedicatedConformanceError::N02BindingMismatch {
            field: "envelope_canonical",
        }
    })?;
    let decoded: EventEnvelope = serde_json::from_slice(&bytes).map_err(|_| {
        DedicatedConformanceError::N02BindingMismatch {
            field: "envelope_canonical",
        }
    })?;
    let canonical = serde_json::to_vec(&decoded).map_err(|_| {
        DedicatedConformanceError::N02BindingMismatch {
            field: "envelope_canonical",
        }
    })?;
    if canonical != bytes {
        return Err(DedicatedConformanceError::N02BindingMismatch {
            field: "envelope_canonical",
        });
    }
    Ok(bytes)
}

fn check_n02(field: &'static str, valid: bool) -> Result<(), DedicatedConformanceError> {
    if !valid {
        return Err(DedicatedConformanceError::N02BindingMismatch { field });
    }
    Ok(())
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
                authority_class: record.authority_class,
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
