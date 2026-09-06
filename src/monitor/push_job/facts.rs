//! W04 source-reference values. Prepared facts and the capture state are added in the next slice.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use super::canonical::{canonical_digest, raw_digest, CanonicalValue};
use super::context::RunContext;
use super::identity::validate_text;
use super::policy::{ReasonCode, VerifiedEmptyEvidenceRef};
use super::{
    PushJobError, Result, Sha256Digest, SourceContractId, SourceContractVersion, UtcMicros,
};

macro_rules! fact_text {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn try_new(value: String) -> Result<Self> {
                validate_text($field, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

fact_text!(SourceRefId, "source_ref_id");
fact_text!(SourceProvider, "source_provider");
fact_text!(ExternalId, "external_id");
fact_text!(ModelId, "model_id");
fact_text!(ModelVersion, "model_version");
fact_text!(ProtectedRef, "protected_ref");

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SourceRef {
    source_ref_id: SourceRefId,
    provider: SourceProvider,
    external_id: ExternalId,
    source_contract_id: SourceContractId,
    content_sha256: Sha256Digest,
}

impl SourceRef {
    pub fn new(
        source_ref_id: SourceRefId,
        provider: SourceProvider,
        external_id: ExternalId,
        source_contract_id: SourceContractId,
        content_sha256: Sha256Digest,
    ) -> Self {
        Self {
            source_ref_id,
            provider,
            external_id,
            source_contract_id,
            content_sha256,
        }
    }

    pub fn source_ref_id(&self) -> &SourceRefId {
        &self.source_ref_id
    }

    pub fn provider(&self) -> &SourceProvider {
        &self.provider
    }

    pub fn external_id(&self) -> &ExternalId {
        &self.external_id
    }

    pub fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }

    pub fn content_sha256(&self) -> &Sha256Digest {
        &self.content_sha256
    }
}

pub(super) fn source_ref_value(source_ref: &SourceRef) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "content_sha256",
            CanonicalValue::String(source_ref.content_sha256.as_str().to_owned()),
        ),
        (
            "external_id",
            CanonicalValue::String(source_ref.external_id.as_str().to_owned()),
        ),
        (
            "provider",
            CanonicalValue::String(source_ref.provider.as_str().to_owned()),
        ),
        (
            "source_contract_id",
            CanonicalValue::String(source_ref.source_contract_id.as_str().to_owned()),
        ),
        (
            "source_ref_id",
            CanonicalValue::String(source_ref.source_ref_id.as_str().to_owned()),
        ),
    ]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum SourceTimeKind {
    ObservedAt,
    AsOf,
}

impl SourceTimeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ObservedAt => "ObservedAt",
            Self::AsOf => "AsOf",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SourceTime {
    source_ref_id: SourceRefId,
    kind: SourceTimeKind,
    value: Option<UtcMicros>,
}

impl SourceTime {
    pub fn observed_at(source_ref_id: SourceRefId, value: Option<UtcMicros>) -> Self {
        Self {
            source_ref_id,
            kind: SourceTimeKind::ObservedAt,
            value,
        }
    }

    pub fn as_of(source_ref_id: SourceRefId, value: Option<UtcMicros>) -> Self {
        Self {
            source_ref_id,
            kind: SourceTimeKind::AsOf,
            value,
        }
    }

    pub fn source_ref_id(&self) -> &SourceRefId {
        &self.source_ref_id
    }

    pub fn kind(&self) -> SourceTimeKind {
        self.kind
    }

    pub fn value(&self) -> Option<UtcMicros> {
        self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ModelOutputRef {
    model: ModelId,
    version: ModelVersion,
    input_sha256: Sha256Digest,
    output_sha256: Sha256Digest,
    protected_ref: ProtectedRef,
}

impl ModelOutputRef {
    pub fn new(
        model: ModelId,
        version: ModelVersion,
        input_sha256: Sha256Digest,
        output_sha256: Sha256Digest,
        protected_ref: ProtectedRef,
    ) -> Self {
        Self {
            model,
            version,
            input_sha256,
            output_sha256,
            protected_ref,
        }
    }

    pub fn model(&self) -> &ModelId {
        &self.model
    }

    pub fn version(&self) -> &ModelVersion {
        &self.version
    }

    pub fn input_sha256(&self) -> &Sha256Digest {
        &self.input_sha256
    }

    pub fn output_sha256(&self) -> &Sha256Digest {
        &self.output_sha256
    }

    pub fn protected_ref(&self) -> &ProtectedRef {
        &self.protected_ref
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ExactBytes {
    bytes: Vec<u8>,
    sha256: Sha256Digest,
}

impl fmt::Debug for ExactBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactBytes")
            .field("len", &self.bytes.len())
            .field("sha256", &self.sha256.as_str())
            .finish()
    }
}

impl ExactBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        let sha256 = raw_digest(&bytes);
        Self { bytes, sha256 }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FactsPresence {
    Present,
    VerifiedEmpty(VerifiedEmptyEvidenceRef),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedFacts {
    source_contract_id: SourceContractId,
    source_contract_version: SourceContractVersion,
    source_refs: Vec<SourceRef>,
    canonical_facts: ExactBytes,
    provider_observed_at: Vec<SourceTime>,
    facts_presence: FactsPresence,
    model_output_refs: Vec<ModelOutputRef>,
}

impl CapturedFacts {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        source_contract_id: SourceContractId,
        source_contract_version: SourceContractVersion,
        source_refs: Vec<SourceRef>,
        canonical_facts: ExactBytes,
        provider_observed_at: Vec<SourceTime>,
        facts_presence: FactsPresence,
        model_output_refs: Vec<ModelOutputRef>,
    ) -> Result<Self> {
        validate_source_refs(&source_contract_id, &source_refs, &provider_observed_at)?;
        validate_model_output_refs(&model_output_refs)?;
        if let FactsPresence::VerifiedEmpty(evidence) = &facts_presence {
            if evidence.source_contract_id() != &source_contract_id {
                return Err(PushJobError::InvalidVerifiedEmptyEvidence(
                    "source contract does not match captured facts",
                ));
            }
        }
        Ok(Self {
            source_contract_id,
            source_contract_version,
            source_refs,
            canonical_facts,
            provider_observed_at,
            facts_presence,
            model_output_refs,
        })
    }

    pub fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }

    pub fn source_contract_version(&self) -> &SourceContractVersion {
        &self.source_contract_version
    }

    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    pub fn canonical_facts(&self) -> &ExactBytes {
        &self.canonical_facts
    }

    pub fn provider_observed_at(&self) -> &[SourceTime] {
        &self.provider_observed_at
    }

    pub fn facts_presence(&self) -> &FactsPresence {
        &self.facts_presence
    }

    pub fn model_output_refs(&self) -> &[ModelOutputRef] {
        &self.model_output_refs
    }
}

fn validate_source_refs(
    source_contract_id: &SourceContractId,
    source_refs: &[SourceRef],
    provider_observed_at: &[SourceTime],
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for source_ref in source_refs {
        if source_ref.source_contract_id() != source_contract_id {
            return Err(PushJobError::InvalidSourceReferences(
                "source contract does not match captured facts",
            ));
        }
        if !seen.insert(source_ref.source_ref_id().clone()) {
            return Err(PushJobError::InvalidSourceReferences(
                "source_ref_id values must be unique",
            ));
        }
    }
    if source_refs.len() != provider_observed_at.len() {
        return Err(PushJobError::InvalidSourceReferences(
            "every source reference must have one source time",
        ));
    }
    if source_refs
        .iter()
        .zip(provider_observed_at)
        .any(|(source_ref, source_time)| source_ref.source_ref_id() != source_time.source_ref_id())
    {
        return Err(PushJobError::InvalidSourceReferences(
            "source times must match source reference order",
        ));
    }
    Ok(())
}

fn validate_model_output_refs(model_output_refs: &[ModelOutputRef]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for model_output_ref in model_output_refs {
        if !seen.insert(model_output_ref) {
            return Err(PushJobError::InvalidModelOutputReferences(
                "model output references must be unique",
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct PreparedFacts {
    run_context_sha256: Sha256Digest,
    source_contract_id: SourceContractId,
    source_contract_version: SourceContractVersion,
    source_refs: Vec<SourceRef>,
    canonical_facts: ExactBytes,
    facts_sha256: Sha256Digest,
    provider_observed_at: Vec<SourceTime>,
    verified_empty: bool,
    model_output_refs: Vec<ModelOutputRef>,
}

impl PreparedFacts {
    fn try_new(
        context: &RunContext,
        expected_source_contract_id: &SourceContractId,
        captured: CapturedFacts,
    ) -> Result<Self> {
        if &captured.source_contract_id != expected_source_contract_id {
            return Err(PushJobError::InvalidPreparedFacts(
                "source contract id does not match catalog binding",
            ));
        }
        if &captured.source_contract_version != context.source_contract_version() {
            return Err(PushJobError::InvalidPreparedFacts(
                "source contract version does not match run context",
            ));
        }
        if let FactsPresence::VerifiedEmpty(evidence) = &captured.facts_presence {
            if evidence.occurrence() != context.occurrence() {
                return Err(PushJobError::InvalidVerifiedEmptyEvidence(
                    "occurrence does not match run context",
                ));
            }
            if evidence.source_contract_id() != expected_source_contract_id {
                return Err(PushJobError::InvalidVerifiedEmptyEvidence(
                    "source contract does not match catalog binding",
                ));
            }
        }
        let facts_sha256 = captured.canonical_facts.sha256().clone();
        Ok(Self {
            run_context_sha256: context.canonical_sha256(),
            source_contract_id: captured.source_contract_id,
            source_contract_version: captured.source_contract_version,
            source_refs: captured.source_refs,
            canonical_facts: captured.canonical_facts,
            facts_sha256,
            provider_observed_at: captured.provider_observed_at,
            verified_empty: matches!(captured.facts_presence, FactsPresence::VerifiedEmpty(_)),
            model_output_refs: captured.model_output_refs,
        })
    }

    pub fn run_context_sha256(&self) -> &Sha256Digest {
        &self.run_context_sha256
    }

    pub fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }

    pub fn source_contract_version(&self) -> &SourceContractVersion {
        &self.source_contract_version
    }

    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    pub fn canonical_facts(&self) -> &ExactBytes {
        &self.canonical_facts
    }

    pub fn facts_sha256(&self) -> &Sha256Digest {
        &self.facts_sha256
    }

    pub fn provider_observed_at(&self) -> &[SourceTime] {
        &self.provider_observed_at
    }

    pub fn verified_empty(&self) -> bool {
        self.verified_empty
    }

    pub fn model_output_refs(&self) -> &[ModelOutputRef] {
        &self.model_output_refs
    }

    pub fn canonical_sha256(&self) -> Sha256Digest {
        canonical_digest("PreparedFacts/v1", &prepared_facts_fields(self))
    }
}

fn prepared_facts_fields(facts: &PreparedFacts) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "canonical_facts",
            CanonicalValue::Object(BTreeMap::from([
                (
                    "length",
                    CanonicalValue::Unsigned(facts.canonical_facts.len() as u64),
                ),
                (
                    "sha256",
                    CanonicalValue::String(facts.facts_sha256.as_str().to_owned()),
                ),
            ])),
        ),
        (
            "model_output_refs",
            CanonicalValue::Array(
                facts
                    .model_output_refs
                    .iter()
                    .map(model_output_ref_value)
                    .collect(),
            ),
        ),
        (
            "provider_observed_at",
            CanonicalValue::Array(
                facts
                    .provider_observed_at
                    .iter()
                    .map(source_time_value)
                    .collect(),
            ),
        ),
        (
            "run_context_sha256",
            CanonicalValue::String(facts.run_context_sha256.as_str().to_owned()),
        ),
        (
            "source_contract_id",
            CanonicalValue::String(facts.source_contract_id.as_str().to_owned()),
        ),
        (
            "source_contract_version",
            CanonicalValue::String(facts.source_contract_version.as_str().to_owned()),
        ),
        (
            "source_refs",
            CanonicalValue::Array(facts.source_refs.iter().map(source_ref_value).collect()),
        ),
        ("verified_empty", CanonicalValue::Bool(facts.verified_empty)),
    ])
}

fn source_time_value(source_time: &SourceTime) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "kind",
            CanonicalValue::String(source_time.kind.as_str().to_owned()),
        ),
        (
            "source_ref_id",
            CanonicalValue::String(source_time.source_ref_id.as_str().to_owned()),
        ),
        (
            "value",
            source_time.value.map_or(CanonicalValue::Null, |time| {
                CanonicalValue::Unsigned(time.get() as u64)
            }),
        ),
    ]))
}

pub(super) fn model_output_ref_value(model_output_ref: &ModelOutputRef) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "input_sha256",
            CanonicalValue::String(model_output_ref.input_sha256.as_str().to_owned()),
        ),
        (
            "model",
            CanonicalValue::String(model_output_ref.model.as_str().to_owned()),
        ),
        (
            "output_sha256",
            CanonicalValue::String(model_output_ref.output_sha256.as_str().to_owned()),
        ),
        (
            "protected_ref",
            CanonicalValue::String(model_output_ref.protected_ref.as_str().to_owned()),
        ),
        (
            "version",
            CanonicalValue::String(model_output_ref.version.as_str().to_owned()),
        ),
    ]))
}

#[derive(Clone, Debug)]
pub struct PreparedFactsSnapshot(Arc<PreparedFacts>);

impl PartialEq for PreparedFactsSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.shares_instance_with(other)
    }
}

impl Eq for PreparedFactsSnapshot {}

impl PreparedFactsSnapshot {
    pub fn facts(&self) -> &PreparedFacts {
        &self.0
    }

    pub fn shares_instance_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureStateView {
    Open,
    Capturing,
    Sealed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PreparationError {
    #[error("fact acquisition failed: {reason:?}")]
    AcquisitionFailed { reason: ReasonCode },
    #[error("captured facts violated their contract: {0}")]
    InvalidCapturedFacts(PushJobError),
    #[error("fact acquisition was already attempted while capture was {state:?}")]
    AlreadyAttempted { state: CaptureStateView },
}

// W06 catalog wiring is the first non-test path that constructs an Open capability.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
enum CaptureState {
    Open,
    Capturing,
    Sealed,
    Failed,
}

/// A capture capability cannot be copied into a second provider/LLM call path.
///
/// ```compile_fail
/// use stock_analysis::monitor::push_job::PreparationCapture;
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<PreparationCapture>();
/// ```
#[derive(Debug)]
pub struct PreparationCapture {
    context: RunContext,
    expected_source_contract_id: SourceContractId,
    state: CaptureState,
    attempt_count: u64,
    rejected_count: u64,
}

impl PreparationCapture {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn new(context: RunContext, expected_source_contract_id: SourceContractId) -> Self {
        Self {
            context,
            expected_source_contract_id,
            state: CaptureState::Open,
            attempt_count: 0,
            rejected_count: 0,
        }
    }

    pub fn context(&self) -> &RunContext {
        &self.context
    }

    pub fn capture_once<F>(
        &mut self,
        acquire: F,
    ) -> std::result::Result<PreparedFactsSnapshot, PreparationError>
    where
        F: FnOnce(&RunContext) -> std::result::Result<CapturedFacts, PreparationError>,
    {
        let current = self.state();
        if current != CaptureStateView::Open {
            self.rejected_count = self.rejected_count.saturating_add(1);
            return Err(PreparationError::AlreadyAttempted { state: current });
        }

        self.state = CaptureState::Capturing;
        self.attempt_count = self.attempt_count.saturating_add(1);
        let captured = match acquire(&self.context) {
            Ok(captured) => captured,
            Err(error) => {
                self.state = CaptureState::Failed;
                return Err(error);
            }
        };
        let prepared = match PreparedFacts::try_new(
            &self.context,
            &self.expected_source_contract_id,
            captured,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.state = CaptureState::Failed;
                return Err(PreparationError::InvalidCapturedFacts(error));
            }
        };
        let snapshot = PreparedFactsSnapshot(Arc::new(prepared));
        self.state = CaptureState::Sealed;
        Ok(snapshot)
    }

    pub fn state(&self) -> CaptureStateView {
        match self.state {
            CaptureState::Open => CaptureStateView::Open,
            CaptureState::Capturing => CaptureStateView::Capturing,
            CaptureState::Sealed => CaptureStateView::Sealed,
            CaptureState::Failed => CaptureStateView::Failed,
        }
    }

    pub fn attempt_count(&self) -> u64 {
        self.attempt_count
    }

    pub fn rejected_count(&self) -> u64 {
        self.rejected_count
    }
}

#[cfg(test)]
pub(super) fn capture_fixture() -> Result<PreparationCapture> {
    super::context::capture_capability_fixture()
}
