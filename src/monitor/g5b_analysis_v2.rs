//! Claim-bound model observations. No counted admission, send or day-seal authority.

use super::alert_log::AlertRecord;
use super::attribution_deep::{
    deep_attribution_prompt, render_deep_attribution_summary, DeepAttributionRequest,
    DeepAttributionResult, DeepAttributionRow, G5B_SYSTEM_PROMPT_V1,
};
use super::g5b_selection_v2::G5bSelectionEvidence;
use crate::durable_delivery::{
    DeliveryEnvelope, DeliverySubKind, DurableDeliveryCoordinator, DurableDeliveryError,
    G5bConfiguredAnalysis, G5bDaySession, G5bSnapshotKind, PushKind, VerifiedStoredG5bCohort,
};
use crate::llm::{LlmProvider, ModelCallReceipt};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const ATTEMPT_SCHEMA: &str = "g5b-analysis-attempt-v2";
const CORE_SCHEMA: &str = "g5b-analysis-frozen-core-v2";
const SOURCE_SCHEMA: &str = "g5b-attribution-v2";
const HANDOFF_SCHEMA: &str = "g5b-frozen-handoff-v2";
const ATTEMPT_POLICY: &str = "g5b-one-original-model-attempt-v2";
const HANDOFF_POLICY: &str = "g5b-exact-frozen-handoff-v2";
const MAX_CONTENT_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum G5bAnalysisV2Error {
    Store(DurableDeliveryError),
    Codec(String),
    Model(String),
}
impl std::fmt::Display for G5bAnalysisV2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Codec(e) => write!(f, "G5b v2 evidence: {e}"),
            Self::Model(e) => write!(f, "G5b v2 model: {e}"),
        }
    }
}
impl std::error::Error for G5bAnalysisV2Error {}
impl From<DurableDeliveryError> for G5bAnalysisV2Error {
    fn from(e: DurableDeliveryError) -> Self {
        Self::Store(e)
    }
}
impl From<serde_json::Error> for G5bAnalysisV2Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Codec(e.to_string())
    }
}
type Result<T> = std::result::Result<T, G5bAnalysisV2Error>;
fn invalid(detail: &str) -> G5bAnalysisV2Error {
    G5bAnalysisV2Error::Codec(detail.to_owned())
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn canonical<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(v)?)
}
fn decode<T: serde::de::DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T> {
    let value = serde_json::from_slice(bytes)?;
    if canonical(&value)? != bytes {
        return Err(invalid("noncanonical evidence"));
    }
    Ok(value)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    business_date: NaiveDate,
    cohort_identity: String,
    selection_index: usize,
    occurrence_identity: String,
    line_ordinal: u64,
    start_offset: u64,
    end_offset: u64,
    raw_line_sha256: String,
    record_sha256: String,
}
fn member(evidence: &G5bSelectionEvidence, index: usize) -> Result<Member> {
    let line = evidence
        .encoded()
        .selected
        .get(index)
        .ok_or_else(|| invalid("member index absent"))?;
    Ok(Member {
        business_date: evidence.encoded().business_date,
        cohort_identity: evidence.cohort_identity(),
        selection_index: index,
        occurrence_identity: evidence.occurrences()[index].0.clone(),
        line_ordinal: line.line_ordinal,
        start_offset: line.start_offset,
        end_offset: line.end_offset,
        raw_line_sha256: line.raw_line_sha256.clone(),
        record_sha256: line.record_sha256.clone(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    schema: String,
    policy: String,
    member: Member,
    selection_canonical: Vec<u8>,
    selection_sha256: String,
    request_as_of: DateTime<Utc>,
    configured_provider: String,
    configured_model: String,
    system_prompt: String,
    system_sha256: String,
    user_prompt: String,
    user_sha256: String,
}
fn validate_attempt(attempt: &Attempt) -> Result<AlertRecord> {
    let evidence = G5bSelectionEvidence::decode(&attempt.selection_canonical)
        .map_err(|e| invalid(&e.to_string()))?;
    if attempt.schema != ATTEMPT_SCHEMA
        || attempt.policy != ATTEMPT_POLICY
        || attempt.member != member(&evidence, attempt.member.selection_index)?
        || hash(&attempt.selection_canonical) != attempt.selection_sha256
        || attempt.configured_provider.trim().is_empty()
        || attempt.configured_model.trim().is_empty()
        || attempt.system_prompt != G5B_SYSTEM_PROMPT_V1
        || hash(attempt.system_prompt.as_bytes()) != attempt.system_sha256
        || hash(attempt.user_prompt.as_bytes()) != attempt.user_sha256
    {
        return Err(invalid("attempt binding differs"));
    }
    let record: AlertRecord = serde_json::from_slice(
        &evidence.encoded().selected[attempt.member.selection_index].record_canonical,
    )?;
    let request = DeepAttributionRequest {
        record: record.clone(),
        as_of: attempt.request_as_of,
    };
    if deep_attribution_prompt(&request) != attempt.user_prompt {
        return Err(invalid("saved prompt differs from actual member"));
    }
    Ok(record)
}

/// A descriptor of the already existing real receipt, never a receipt constructor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    provider: String,
    model: String,
    upstream_request_id: Option<String>,
    upstream_response_id: Option<String>,
    system_sha256: String,
    user_sha256: String,
    response_sha256: String,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
}
impl From<&ModelCallReceipt> for Receipt {
    fn from(r: &ModelCallReceipt) -> Self {
        Self {
            provider: r.provider().to_owned(),
            model: r.model().to_owned(),
            upstream_request_id: r.upstream_request_id().map(str::to_owned),
            upstream_response_id: r.upstream_response_id().map(str::to_owned),
            system_sha256: r.system_sha256().to_owned(),
            user_sha256: r.user_sha256().to_owned(),
            response_sha256: r.response_sha256().to_owned(),
            started_at: *r.started_at(),
            completed_at: *r.completed_at(),
        }
    }
}
fn validate_receipt(attempt: &Attempt, receipt: &Receipt, content: &[u8]) -> Result<()> {
    if content.is_empty()
        || content.len() > MAX_CONTENT_BYTES
        || std::str::from_utf8(content).is_err()
        || receipt.provider != attempt.configured_provider
        || receipt.model.trim().is_empty()
        || receipt
            .upstream_response_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
        || receipt
            .upstream_request_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        || receipt.system_sha256 != attempt.system_sha256
        || receipt.user_sha256 != attempt.user_sha256
        || receipt.response_sha256 != hash(content)
        || receipt.started_at < attempt.request_as_of
        || receipt.completed_at < receipt.started_at
    {
        return Err(invalid("actual receipt/prompt/content binding differs"));
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParsedResult {
    main_reason: String,
    catalyst_chain: Vec<String>,
    capital_logic: String,
    confidence: String,
    risk_note: String,
}
impl ParsedResult {
    fn validate(&self) -> Result<()> {
        if !matches!(self.confidence.as_str(), "high" | "medium" | "low") {
            return Err(invalid("model confidence invalid"));
        }
        Ok(())
    }
    fn legacy(&self) -> DeepAttributionResult {
        DeepAttributionResult {
            main_reason: self.main_reason.clone(),
            catalyst_chain: self.catalyst_chain.clone(),
            capital_logic: self.capital_logic.clone(),
            confidence: self.confidence.clone(),
            risk_note: self.risk_note.clone(),
        }
    }
}
fn parse_content(content: &[u8]) -> Result<ParsedResult> {
    // Deserialize directly from actual content, retaining duplicate-field rejection.
    let result: ParsedResult = serde_json::from_slice(content)?;
    result.validate()?;
    Ok(result)
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Core {
    schema: String,
    member: Member,
    attempt_identity: String,
    attempt_canonical: Vec<u8>,
    attempt_sha256: String,
    model_content_utf8: Vec<u8>,
    model_content_sha256: String,
    receipt: Receipt,
    result_canonical: Vec<u8>,
    result_sha256: String,
    row_canonical: Vec<u8>,
    row_sha256: String,
    elapsed_ms: u64,
    summary_utf8: Vec<u8>,
    summary_sha256: String,
}
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    schema: String,
    member: Member,
    selection_sha256: String,
    attempt_identity: String,
    attempt_sha256: String,
    frozen_core_sha256: String,
    rendered_sha256: String,
}
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Handoff {
    schema: String,
    policy: String,
    core_canonical: Vec<u8>,
    core_sha256: String,
    source_canonical: Vec<u8>,
    source_sha256: String,
    envelope_canonical: Vec<u8>,
    envelope_sha256: String,
    decision_identity: String,
}
fn row_for(
    attempt: &Attempt,
    result: &ParsedResult,
    receipt: &Receipt,
    elapsed_ms: u64,
) -> Result<DeepAttributionRow> {
    Ok(DeepAttributionRow {
        record: validate_attempt(attempt)?,
        result: result.legacy(),
        analyzed_at: receipt.completed_at.to_rfc3339(),
        provider: receipt.provider.clone(),
        model: receipt.model.clone(),
        upstream_request_id: receipt.upstream_request_id.clone(),
        upstream_response_id: receipt.upstream_response_id.clone(),
        elapsed_ms,
    })
}
fn envelope_for(source: &Source, source_bytes: &[u8], summary: &[u8]) -> Result<DeliveryEnvelope> {
    Ok(DeliveryEnvelope::new(
        source.member.business_date.to_string(),
        PushKind::G5bAttribution,
        DeliverySubKind::None,
        "GLOBAL",
        &source.member.occurrence_identity,
        hash(source_bytes),
        source_bytes.to_vec(),
        hash(source_bytes),
        summary.to_vec(),
        true,
        None,
    )?)
}
fn build_handoff(core: Core) -> Result<Vec<u8>> {
    let attempt: Attempt = decode(&core.attempt_canonical)?;
    let core_bytes = canonical(&core)?;
    let source = Source {
        schema: SOURCE_SCHEMA.to_owned(),
        member: core.member.clone(),
        selection_sha256: attempt.selection_sha256,
        attempt_identity: core.attempt_identity.clone(),
        attempt_sha256: core.attempt_sha256.clone(),
        frozen_core_sha256: hash(&core_bytes),
        rendered_sha256: core.summary_sha256.clone(),
    };
    let source_bytes = canonical(&source)?;
    let envelope = envelope_for(&source, &source_bytes, &core.summary_utf8)?;
    let envelope_bytes = canonical(&envelope)?;
    canonical(&Handoff {
        schema: HANDOFF_SCHEMA.to_owned(),
        policy: HANDOFF_POLICY.to_owned(),
        core_sha256: hash(&core_bytes),
        core_canonical: core_bytes,
        source_sha256: hash(&source_bytes),
        source_canonical: source_bytes,
        envelope_sha256: hash(&envelope_bytes),
        envelope_canonical: envelope_bytes,
        decision_identity: envelope.decision_identity,
    })
}
fn validate_handoff(bytes: &[u8], actual_attempt: &[u8], actual_identity: &str) -> Result<Core> {
    let handoff: Handoff = decode(bytes)?;
    let core: Core = decode(&handoff.core_canonical)?;
    let attempt: Attempt = decode(&core.attempt_canonical)?;
    let record = validate_attempt(&attempt)?;
    validate_receipt(&attempt, &core.receipt, &core.model_content_utf8)?;
    let result = parse_content(&core.model_content_utf8)?;
    let expected_row = row_for(&attempt, &result, &core.receipt, core.elapsed_ms)?;
    let row_bytes = canonical(&expected_row)?;
    let summary = render_deep_attribution_summary(&expected_row).into_bytes();
    if core.schema != CORE_SCHEMA
        || core.member != attempt.member
        || core.attempt_canonical != actual_attempt
        || core.attempt_identity != actual_identity
        || hash(&core.attempt_canonical) != core.attempt_sha256
        || hash(&core.model_content_utf8) != core.model_content_sha256
        || canonical(&result)? != core.result_canonical
        || hash(&core.result_canonical) != core.result_sha256
        || core.row_canonical != row_bytes
        || hash(&row_bytes) != core.row_sha256
        || core.summary_utf8 != summary
        || hash(&summary) != core.summary_sha256
        || hash(&canonical(&record)?) != core.member.record_sha256
        || handoff.schema != HANDOFF_SCHEMA
        || handoff.policy != HANDOFF_POLICY
        || hash(&handoff.core_canonical) != handoff.core_sha256
        || hash(&handoff.source_canonical) != handoff.source_sha256
        || hash(&handoff.envelope_canonical) != handoff.envelope_sha256
    {
        return Err(invalid("frozen core/handoff bytes differ"));
    }
    let source: Source = decode(&handoff.source_canonical)?;
    let expected = Source {
        schema: SOURCE_SCHEMA.to_owned(),
        member: core.member.clone(),
        selection_sha256: attempt.selection_sha256,
        attempt_identity: actual_identity.to_owned(),
        attempt_sha256: hash(actual_attempt),
        frozen_core_sha256: handoff.core_sha256,
        rendered_sha256: core.summary_sha256.clone(),
    };
    let envelope: DeliveryEnvelope = decode(&handoff.envelope_canonical)?;
    if source != expected
        || envelope != envelope_for(&source, &handoff.source_canonical, &summary)?
        || envelope.decision_identity != handoff.decision_identity
    {
        return Err(invalid("closed source/envelope differs"));
    }
    Ok(core)
}

pub enum G5bAnalysisClaimV2 {
    Ready(G5bAnalysisWorkV2),
    Frozen(G5bFrozenAnalysisV2),
    CompletionUnproven { occurrence_identity: String },
}
/// Private fields, non-Clone and non-Deserialize; contains no date guard.
pub struct G5bAnalysisWorkV2 {
    coordinator: Arc<DurableDeliveryCoordinator>,
    provider: Arc<dyn LlmProvider>,
    attempt: Attempt,
    attempt_bytes: Vec<u8>,
    attempt_identity: String,
    #[cfg(test)]
    test_clock: Option<DateTime<Utc>>,
}
/// Real completed model observation, still without any sending authority.
pub struct G5bCompletedAnalysisV2 {
    coordinator: Arc<DurableDeliveryCoordinator>,
    core: Core,
}
/// Actual saved model observation. This is never a counted owner or day seal.
/// The individual local reads are not an atomic Attempt/Frozen/Archive bundle;
/// counted admission needs the later owner and final unified witness protocol.
pub struct G5bFrozenAnalysisV2 {
    core: Core,
    handoff_bytes: Vec<u8>,
}
impl G5bFrozenAnalysisV2 {
    pub fn occurrence_identity(&self) -> &str {
        &self.core.member.occurrence_identity
    }
    pub fn model_content_utf8(&self) -> &[u8] {
        &self.core.model_content_utf8
    }
    pub fn summary_utf8(&self) -> &[u8] {
        &self.core.summary_utf8
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.handoff_bytes
    }
}

fn existing(
    session: &G5bDaySession<'_>,
    cohort: &VerifiedStoredG5bCohort,
    index: usize,
) -> Result<Option<G5bAnalysisClaimV2>> {
    let evidence = G5bSelectionEvidence::decode(cohort.selection_bytes())
        .map_err(|e| invalid(&e.to_string()))?;
    let binding = member(&evidence, index)?;
    if session.has_member_snapshot(cohort, G5bSnapshotKind::Frozen, index)? {
        let attempt = session
            .read_committed_member_snapshot(cohort, G5bSnapshotKind::Attempt, index)?
            .ok_or_else(|| invalid("Frozen has no original Attempt"))?;
        let frozen = session
            .read_committed_member_snapshot(cohort, G5bSnapshotKind::Frozen, index)?
            .ok_or_else(|| invalid("Frozen snapshot absent"))?;
        let core = validate_handoff(
            frozen.desired_bytes(),
            attempt.desired_bytes(),
            attempt.identity(),
        )?;
        if core.member != binding {
            return Err(invalid("Frozen belongs to another actual member"));
        }
        return Ok(Some(G5bAnalysisClaimV2::Frozen(G5bFrozenAnalysisV2 {
            core,
            handoff_bytes: frozen.desired_bytes().to_vec(),
        })));
    }
    if session.has_member_snapshot(cohort, G5bSnapshotKind::Attempt, index)? {
        return Ok(Some(G5bAnalysisClaimV2::CompletionUnproven {
            occurrence_identity: binding.occurrence_identity,
        }));
    }
    Ok(None)
}

/// Synchronous, short local operation. Returns no held date lock.
pub fn claim_analysis_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
    index: usize,
) -> Result<G5bAnalysisClaimV2> {
    let session = coordinator.g5b_day_session(date)?;
    let cohort = session.read_cohort()?;
    if let Some(cohort) = &cohort {
        if let Some(state) = existing(&session, cohort, index)? {
            return Ok(state);
        }
    }
    let ready = session.configured_analysis()?;
    claim_with_ready(Arc::clone(&coordinator), &session, cohort, ready, index)
}
fn claim_with_ready(
    coordinator: Arc<DurableDeliveryCoordinator>,
    session: &G5bDaySession<'_>,
    cohort: Option<VerifiedStoredG5bCohort>,
    ready: G5bConfiguredAnalysis,
    index: usize,
) -> Result<G5bAnalysisClaimV2> {
    let cohort = match cohort {
        Some(cohort) => cohort,
        None => {
            let selection = session.prepare_cohort(&ready)?;
            session.publish_prepared_artifact(&selection)?;
            session.commit_prepared_artifact(&selection)?;
            session
                .read_cohort()?
                .ok_or_else(|| invalid("new cohort not published"))?
        }
    };
    if let Some(state) = existing(session, &cohort, index)? {
        return Ok(state);
    }
    let evidence = G5bSelectionEvidence::decode(cohort.selection_bytes())
        .map_err(|e| invalid(&e.to_string()))?;
    let binding = member(&evidence, index)?;
    let record: AlertRecord =
        serde_json::from_slice(&evidence.encoded().selected[index].record_canonical)?;
    let now = session.analysis_request_time(&ready)?;
    let request = DeepAttributionRequest { record, as_of: now };
    let provider = ready.provider();
    let user = deep_attribution_prompt(&request);
    let attempt = Attempt {
        schema: ATTEMPT_SCHEMA.to_owned(),
        policy: ATTEMPT_POLICY.to_owned(),
        member: binding,
        selection_canonical: cohort.selection_bytes().to_vec(),
        selection_sha256: hash(cohort.selection_bytes()),
        request_as_of: now,
        configured_provider: provider.name().to_owned(),
        configured_model: provider.model().to_owned(),
        system_prompt: G5B_SYSTEM_PROMPT_V1.to_owned(),
        system_sha256: hash(G5B_SYSTEM_PROMPT_V1.as_bytes()),
        user_sha256: hash(user.as_bytes()),
        user_prompt: user,
    };
    validate_attempt(&attempt)?;
    let bytes = canonical(&attempt)?;
    let original = session.prepare_new_analysis_attempt(&cohort, index, &bytes, &ready)?;
    session.publish_prepared_artifact(&original)?;
    session.commit_prepared_artifact(&original)?;
    let committed = session
        .read_committed_member_snapshot(&cohort, G5bSnapshotKind::Attempt, index)?
        .ok_or_else(|| invalid("new Attempt not committed"))?;
    if committed.identity() != original.identity() || committed.desired_bytes() != bytes {
        return Err(invalid("new original Attempt changed"));
    }
    // Crossing the closing boundary during local publication consumes the
    // original attempt but cannot return a new model-call capability.
    session.analysis_request_time(&ready)?;
    Ok(G5bAnalysisClaimV2::Ready(G5bAnalysisWorkV2 {
        coordinator,
        provider,
        attempt,
        attempt_bytes: bytes,
        attempt_identity: original.identity().to_owned(),
        #[cfg(test)]
        test_clock: ready.test_clock(),
    }))
}
impl G5bAnalysisWorkV2 {
    /// Consumes the only live work capability. A timeout/error never makes Frozen.
    pub async fn assess(self) -> Result<G5bCompletedAnalysisV2> {
        // Revalidate the original live claim and fresh invocation clock. This
        // short session is dropped before the provider future is constructed.
        {
            let session = self
                .coordinator
                .g5b_day_session(self.attempt.member.business_date)?;
            let cohort = session
                .read_cohort()?
                .ok_or_else(|| invalid("actual claimed cohort absent"))?;
            let evidence = G5bSelectionEvidence::decode(cohort.selection_bytes())
                .map_err(|e| invalid(&e.to_string()))?;
            if member(&evidence, self.attempt.member.selection_index)? != self.attempt.member {
                return Err(invalid("actual claimed member changed"));
            }
            let original = session
                .read_committed_member_snapshot(
                    &cohort,
                    G5bSnapshotKind::Attempt,
                    self.attempt.member.selection_index,
                )?
                .ok_or_else(|| invalid("actual original Attempt absent"))?;
            if original.identity() != self.attempt_identity
                || original.desired_bytes() != self.attempt_bytes
            {
                return Err(invalid("actual original Attempt changed"));
            }
            #[cfg(test)]
            if let Some(now) = self.test_clock {
                session.validate_analysis_call_time_for_test(now)?;
            } else {
                session.validate_analysis_call_time()?;
            }
            #[cfg(not(test))]
            session.validate_analysis_call_time()?;
        }
        let started = std::time::Instant::now();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.provider
                .chat_json_with_receipt(&self.attempt.system_prompt, &self.attempt.user_prompt),
        )
        .await
        .map_err(|_| {
            G5bAnalysisV2Error::Model(
                "model call exceeded 45s; original Attempt remains consumed".to_owned(),
            )
        })?
        .map_err(|e| G5bAnalysisV2Error::Model(e.to_string()))?;
        let (_, raw_content, actual_receipt) = response.into_parts();
        let raw = raw_content.into_bytes();
        let receipt = Receipt::from(&actual_receipt);
        validate_receipt(&self.attempt, &receipt, &raw)?;
        let result = parse_content(&raw)?;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis())
            .map_err(|_| invalid("elapsed milliseconds overflow"))?;
        let row = row_for(&self.attempt, &result, &receipt, elapsed_ms)?;
        let row_bytes = canonical(&row)?;
        let result_bytes = canonical(&result)?;
        let summary = render_deep_attribution_summary(&row).into_bytes();
        Ok(G5bCompletedAnalysisV2 {
            coordinator: self.coordinator,
            core: Core {
                schema: CORE_SCHEMA.to_owned(),
                member: self.attempt.member,
                attempt_identity: self.attempt_identity,
                attempt_sha256: hash(&self.attempt_bytes),
                attempt_canonical: self.attempt_bytes,
                model_content_sha256: hash(&raw),
                model_content_utf8: raw,
                receipt,
                result_sha256: hash(&result_bytes),
                result_canonical: result_bytes,
                row_sha256: hash(&row_bytes),
                row_canonical: row_bytes,
                elapsed_ms,
                summary_sha256: hash(&summary),
                summary_utf8: summary,
            },
        })
    }
}
#[cfg(test)]
impl G5bAnalysisWorkV2 {
    pub(crate) async fn assess_at_for_test(
        mut self,
        now: DateTime<Utc>,
    ) -> Result<G5bCompletedAnalysisV2> {
        {
            // Only the actual attested Test owner may advance this clock.
            // Do not validate its value here: the real assess control flow
            // must reject delayed invocations at its fresh clock boundary.
            let session = self
                .coordinator
                .g5b_day_session(self.attempt.member.business_date)?;
            session.validate_analysis_test_owner()?;
        }
        self.test_clock = Some(now);
        self.assess().await
    }
}
impl G5bCompletedAnalysisV2 {
    /// Short local publication, without durable decision admission or a sink.
    pub fn freeze(self) -> Result<G5bFrozenAnalysisV2> {
        let bytes = build_handoff(self.core)?;
        let handoff: Handoff = decode(&bytes)?;
        let core: Core = decode(&handoff.core_canonical)?;
        let session = self
            .coordinator
            .g5b_day_session(core.member.business_date)?;
        let cohort = session
            .read_cohort()?
            .ok_or_else(|| invalid("saved cohort absent"))?;
        let evidence = G5bSelectionEvidence::decode(cohort.selection_bytes())
            .map_err(|e| invalid(&e.to_string()))?;
        if member(&evidence, core.member.selection_index)? != core.member {
            return Err(invalid("result member differs from actual cohort"));
        }
        let attempt = session
            .read_committed_member_snapshot(
                &cohort,
                G5bSnapshotKind::Attempt,
                core.member.selection_index,
            )?
            .ok_or_else(|| invalid("actual original Attempt absent"))?;
        let validated = validate_handoff(&bytes, attempt.desired_bytes(), attempt.identity())?;
        if session.has_member_snapshot(
            &cohort,
            G5bSnapshotKind::Frozen,
            core.member.selection_index,
        )? {
            return Err(invalid("member already has a Frozen observation"));
        }
        let intent = session.prepare_opaque_snapshot(
            &cohort,
            G5bSnapshotKind::Frozen,
            Some(core.member.selection_index),
            &bytes,
        )?;
        session.publish_prepared_artifact(&intent)?;
        session.commit_prepared_artifact(&intent)?;
        let actual = session
            .read_committed_member_snapshot(
                &cohort,
                G5bSnapshotKind::Frozen,
                core.member.selection_index,
            )?
            .ok_or_else(|| invalid("Frozen not committed"))?;
        if actual.desired_bytes() != bytes {
            return Err(invalid("Frozen bytes changed"));
        }
        Ok(G5bFrozenAnalysisV2 {
            core: validated,
            handoff_bytes: bytes,
        })
    }
}

#[cfg(test)]
pub(crate) fn claim_for_test(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
    index: usize,
    provider: Arc<dyn LlmProvider>,
    now: DateTime<Utc>,
) -> Result<G5bAnalysisClaimV2> {
    let session = coordinator.g5b_day_session(date)?;
    let cohort = session.read_cohort()?;
    if let Some(cohort) = &cohort {
        if let Some(state) = existing(&session, cohort, index)? {
            return Ok(state);
        }
    }
    // This real owner helper rejects non-Test stores before accepting a fake.
    let ready = session.configured_analysis_for_test(provider, now)?;
    claim_with_ready(Arc::clone(&coordinator), &session, cohort, ready, index)
}

#[cfg(test)]
#[path = "g5b_analysis_v2_tests.rs"]
mod tests;
