//! BR-193 durable acquisition evidence vocabulary.
//!
//! This module validates the closed response/error matrix before a response
//! seal can enter persistence. It performs no provider I/O and accepts no raw
//! provider diagnostic text.

use crate::news::aggregator::raw_v2::{
    registered_global_news_feeds, RegisteredGlobalNewsFeed, REGISTERED_GLOBAL_NEWS_LIMIT,
};
use crate::selection::schema_v2::sha256_bytes;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

pub const NEWS_PER_FEED_LIMIT: usize = 20;
pub const NEWS_FETCH_PERIOD_SECS: i64 = 120;
const CADENCE_RECEIPT_DOMAIN: &str =
    "stock_analysis.selection_v2_generation_acquisition_cadence_receipt.v1";

/// Opaque BR-193 acquisition namespace.
///
/// The public constructor can mint only physically isolated `TEST_CODE_`
/// namespaces. Production construction remains crate-private so command and
/// provider code cannot choose or forge a production namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionModeNamespace(String);

impl AcquisitionModeNamespace {
    pub fn for_test_code(value: impl Into<String>) -> Result<Self, AcquisitionV2Error> {
        let value = value.into();
        if !is_trim_stable_nonempty(&value) || !value.starts_with("TEST_CODE_") {
            return Err(AcquisitionV2Error::ambiguous(
                "test acquisition namespace must be trim-stable and start with TEST_CODE_",
            ));
        }
        Ok(Self(value))
    }

    #[allow(
        dead_code,
        reason = "BR-193 production namespace is minted only by the unreleased owner/lease factory"
    )]
    pub(crate) fn production() -> Self {
        Self("production".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationAcquisitionCadenceReceiptV1 {
    domain: String,
    schema_version: u64,
    cadence_receipt_id: String,
    mode_namespace: String,
    activation_run_id: String,
    activation_receipt_hash: String,
    scheduler_cycle_id: String,
    acquisition_started_at: String,
    next_acquisition_eligible_at: String,
    prior_cadence_receipt_hash: Option<String>,
    boot_instance_id: String,
    committed_at: String,
}

/// Strict canonical cadence receipt read back from durable storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGenerationAcquisitionCadenceReceipt {
    wire: GenerationAcquisitionCadenceReceiptV1,
    canonical_bytes: Vec<u8>,
    content_hash: String,
}

impl VerifiedGenerationAcquisitionCadenceReceipt {
    pub fn cadence_receipt_id(&self) -> &str {
        &self.wire.cadence_receipt_id
    }

    pub fn mode_namespace(&self) -> &str {
        &self.wire.mode_namespace
    }

    pub fn scheduler_cycle_id(&self) -> &str {
        &self.wire.scheduler_cycle_id
    }

    pub fn acquisition_started_at(&self) -> &str {
        &self.wire.acquisition_started_at
    }

    pub fn next_acquisition_eligible_at(&self) -> &str {
        &self.wire.next_acquisition_eligible_at
    }

    pub fn prior_cadence_receipt_hash(&self) -> Option<&str> {
        self.wire.prior_cadence_receipt_hash.as_deref()
    }

    pub fn boot_instance_id(&self) -> &str {
        &self.wire.boot_instance_id
    }

    pub fn committed_at(&self) -> &str {
        &self.wire.committed_at
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_generation_acquisition_cadence_receipt(
    cadence_receipt_id: impl Into<String>,
    mode_namespace: &AcquisitionModeNamespace,
    activation_run_id: impl Into<String>,
    activation_receipt_hash: impl Into<String>,
    scheduler_cycle_id: impl Into<String>,
    acquisition_started_at: impl Into<String>,
    prior_cadence_receipt_hash: Option<String>,
    boot_instance_id: impl Into<String>,
    committed_at: impl Into<String>,
) -> Result<VerifiedGenerationAcquisitionCadenceReceipt, AcquisitionV2Error> {
    let acquisition_started_at = acquisition_started_at.into();
    let started = parse_rfc3339_nanos_utc(&acquisition_started_at, "acquisition_started_at")?;
    let next = started
        .checked_add_signed(Duration::seconds(NEWS_FETCH_PERIOD_SECS))
        .ok_or_else(|| AcquisitionV2Error::ambiguous("next acquisition eligibility time overflow"))?
        .to_rfc3339_opts(SecondsFormat::Nanos, true);
    let wire = GenerationAcquisitionCadenceReceiptV1 {
        domain: CADENCE_RECEIPT_DOMAIN.to_owned(),
        schema_version: 1,
        cadence_receipt_id: cadence_receipt_id.into(),
        mode_namespace: mode_namespace.as_str().to_owned(),
        activation_run_id: activation_run_id.into(),
        activation_receipt_hash: activation_receipt_hash.into(),
        scheduler_cycle_id: scheduler_cycle_id.into(),
        acquisition_started_at,
        next_acquisition_eligible_at: next,
        prior_cadence_receipt_hash,
        boot_instance_id: boot_instance_id.into(),
        committed_at: committed_at.into(),
    };
    verified_cadence_from_wire(wire)
}

pub fn parse_generation_acquisition_cadence_receipt(
    canonical_bytes: &[u8],
    expected_content_hash: &str,
) -> Result<VerifiedGenerationAcquisitionCadenceReceipt, AcquisitionV2Error> {
    verify_evidence_hash(expected_content_hash, "cadence_receipt_content_hash")?;
    let wire: GenerationAcquisitionCadenceReceiptV1 = serde_json::from_slice(canonical_bytes)
        .map_err(|error| {
            AcquisitionV2Error::ambiguous(format!("cadence receipt decode failed: {error}"))
        })?;
    let verified = verified_cadence_from_wire(wire)?;
    if verified.canonical_bytes != canonical_bytes || verified.content_hash != expected_content_hash
    {
        return Err(AcquisitionV2Error::ambiguous(
            "cadence receipt bytes/hash readback mismatch",
        ));
    }
    Ok(verified)
}

fn verified_cadence_from_wire(
    wire: GenerationAcquisitionCadenceReceiptV1,
) -> Result<VerifiedGenerationAcquisitionCadenceReceipt, AcquisitionV2Error> {
    verify_cadence_wire(&wire)?;
    let canonical_bytes = serde_json::to_vec(&wire).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("cadence receipt encode failed: {error}"))
    })?;
    let mut hash_preimage = CADENCE_RECEIPT_DOMAIN.as_bytes().to_vec();
    hash_preimage.push(0);
    hash_preimage.extend_from_slice(&canonical_bytes);
    let content_hash = sha256_bytes(&hash_preimage);
    Ok(VerifiedGenerationAcquisitionCadenceReceipt {
        wire,
        canonical_bytes,
        content_hash,
    })
}

fn verify_cadence_wire(
    wire: &GenerationAcquisitionCadenceReceiptV1,
) -> Result<(), AcquisitionV2Error> {
    if wire.domain != CADENCE_RECEIPT_DOMAIN
        || wire.schema_version != 1
        || !is_canonical_uuid_v7(&wire.cadence_receipt_id)
        || !valid_mode_namespace(&wire.mode_namespace)
        || !is_trim_stable_nonempty(&wire.activation_run_id)
        || !is_lower_hash(&wire.activation_receipt_hash)
        || !is_canonical_uuid_v7(&wire.scheduler_cycle_id)
        || !is_canonical_uuid_v7(&wire.boot_instance_id)
        || wire
            .prior_cadence_receipt_hash
            .as_deref()
            .is_some_and(|hash| !is_lower_hash(hash))
    {
        return Err(AcquisitionV2Error::ambiguous(
            "cadence receipt violates its closed domain/schema/id/hash/namespace contract",
        ));
    }
    if wire.mode_namespace.starts_with("TEST_CODE_")
        && !wire.activation_run_id.starts_with("TEST_CODE_")
    {
        return Err(AcquisitionV2Error::ambiguous(
            "TEST_CODE cadence cannot bind a non-test activation run",
        ));
    }
    let started = parse_rfc3339_nanos_utc(&wire.acquisition_started_at, "acquisition_started_at")?;
    let next = parse_rfc3339_nanos_utc(
        &wire.next_acquisition_eligible_at,
        "next_acquisition_eligible_at",
    )?;
    let committed = parse_rfc3339_nanos_utc(&wire.committed_at, "committed_at")?;
    if next
        != started
            .checked_add_signed(Duration::seconds(NEWS_FETCH_PERIOD_SECS))
            .ok_or_else(|| {
                AcquisitionV2Error::ambiguous("next acquisition eligibility time overflow")
            })?
        || committed < started
    {
        return Err(AcquisitionV2Error::ambiguous(
            "cadence receipt timing window or commit observation is invalid",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedAcquisitionOutcomeKind {
    SuccessNonempty,
    VerifiedEmpty,
    TransportFailure,
    ProviderCancelled,
    FeedResponseLimitExceeded,
}

#[derive(Debug)]
pub struct CanonicalCarrier<'a> {
    bytes: Option<&'a [u8]>,
    sha256: Option<String>,
}

impl<'a> CanonicalCarrier<'a> {
    pub fn present(bytes: &'a [u8], sha256: impl Into<String>) -> Self {
        Self {
            bytes: Some(bytes),
            sha256: Some(sha256.into()),
        }
    }

    pub const fn absent() -> Self {
        Self {
            bytes: None,
            sha256: None,
        }
    }

    fn verified_bytes(self, label: &'static str) -> Result<Option<&'a [u8]>, AcquisitionV2Error> {
        match (self.bytes, self.sha256) {
            (None, None) => Ok(None),
            (Some(bytes), Some(expected_hash))
                if is_lower_hash(&expected_hash) && sha256_bytes(bytes) == expected_hash =>
            {
                Ok(Some(bytes))
            }
            _ => Err(AcquisitionV2Error::ambiguous(format!(
                "{label} bytes/hash presence or identity mismatch"
            ))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedFeedAcquisitionResponse {
    outcome: FeedAcquisitionOutcomeKind,
    sealed_response_record_count: usize,
}

impl VerifiedFeedAcquisitionResponse {
    pub const fn outcome(&self) -> FeedAcquisitionOutcomeKind {
        self.outcome
    }

    pub const fn sealed_response_record_count(&self) -> usize {
        self.sealed_response_record_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeedAcquisitionResolution {
    Sealed {
        intent_hash: String,
        seal_hash: String,
    },
    Uncertain {
        intent_hash: String,
        uncertainty_record_hash: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FeedAcquisitionResolutionWire {
    Sealed {
        intent_hash: String,
        seal_hash: String,
    },
    Uncertain {
        intent_hash: String,
        uncertainty_record_hash: String,
    },
}

impl<'de> Deserialize<'de> for FeedAcquisitionResolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        match FeedAcquisitionResolutionWire::deserialize(deserializer)? {
            FeedAcquisitionResolutionWire::Sealed {
                intent_hash,
                seal_hash,
            } if is_lower_hash(&intent_hash) && is_lower_hash(&seal_hash) => Ok(Self::Sealed {
                intent_hash,
                seal_hash,
            }),
            FeedAcquisitionResolutionWire::Uncertain {
                intent_hash,
                uncertainty_record_hash,
            } if is_lower_hash(&intent_hash) && is_lower_hash(&uncertainty_record_hash) => {
                Ok(Self::Uncertain {
                    intent_hash,
                    uncertainty_record_hash,
                })
            }
            _ => Err(D::Error::custom(
                "resolution hashes must be lowercase SHA-256 identities",
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedFeedEvidence {
    resolution: FeedAcquisitionResolution,
    response: Option<VerifiedFeedAcquisitionResponse>,
}

impl ResolvedFeedEvidence {
    pub fn sealed(
        intent_hash: impl Into<String>,
        seal_hash: impl Into<String>,
        response: VerifiedFeedAcquisitionResponse,
    ) -> Result<Self, AcquisitionV2Error> {
        let intent_hash = intent_hash.into();
        let seal_hash = seal_hash.into();
        verify_evidence_hash(&intent_hash, "intent_hash")?;
        verify_evidence_hash(&seal_hash, "seal_hash")?;
        Ok(Self {
            resolution: FeedAcquisitionResolution::Sealed {
                intent_hash,
                seal_hash,
            },
            response: Some(response),
        })
    }

    pub fn uncertain(
        intent_hash: impl Into<String>,
        uncertainty_record_hash: impl Into<String>,
    ) -> Result<Self, AcquisitionV2Error> {
        let intent_hash = intent_hash.into();
        let uncertainty_record_hash = uncertainty_record_hash.into();
        verify_evidence_hash(&intent_hash, "intent_hash")?;
        verify_evidence_hash(&uncertainty_record_hash, "uncertainty_record_hash")?;
        Ok(Self {
            resolution: FeedAcquisitionResolution::Uncertain {
                intent_hash,
                uncertainty_record_hash,
            },
            response: None,
        })
    }

    pub const fn resolution(&self) -> &FeedAcquisitionResolution {
        &self.resolution
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressAggregateOutcomeKind {
    SuccessNonempty,
    VerifiedEmpty,
    PendingDependency,
    FailedNonRetryable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressCycleTerminalKind {
    SourceIngressCommitted,
    VerifiedEmpty,
    PendingDependency,
    FailedNonRetryable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionPendingDependencyCode {
    FeedUnavailable,
    ProviderCancelled,
    AcquisitionOutcomeUncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFailedNonRetryableCode {
    FeedResponseLimitExceeded,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FeedPlanActivation {
    Active(FrozenFeedPlan),
    Disabled(crate::selection::activation_runtime::SelectionDisabledReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenFeedPlan {
    descriptor_hashes_in_registration_order: Vec<String>,
    plan_hash: String,
}

impl FrozenFeedPlan {
    pub fn feed_count(&self) -> usize {
        self.descriptor_hashes_in_registration_order.len()
    }

    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }

    pub fn descriptor_hashes_in_registration_order(&self) -> &[String] {
        &self.descriptor_hashes_in_registration_order
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCadenceFeedPlan {
    feed_count: usize,
    plan_hash: String,
}

impl VerifiedCadenceFeedPlan {
    pub const fn feed_count(&self) -> usize {
        self.feed_count
    }

    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedCanonicalCarrier {
    bytes: Option<Vec<u8>>,
    sha256: Option<String>,
}

impl OwnedCanonicalCarrier {
    pub fn present(bytes: Vec<u8>) -> Self {
        let sha256 = sha256_bytes(&bytes);
        Self {
            bytes: Some(bytes),
            sha256: Some(sha256),
        }
    }

    pub fn present_with_hash(bytes: Vec<u8>, sha256: impl Into<String>) -> Self {
        Self {
            bytes: Some(bytes),
            sha256: Some(sha256.into()),
        }
    }

    pub const fn absent() -> Self {
        Self {
            bytes: None,
            sha256: None,
        }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    pub fn sha256(&self) -> Option<&str> {
        self.sha256.as_deref()
    }

    fn borrowed(&self) -> CanonicalCarrier<'_> {
        CanonicalCarrier {
            bytes: self.bytes.as_deref(),
            sha256: self.sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFeedAcquisitionEvidence {
    outcome: FeedAcquisitionOutcomeKind,
    response: OwnedCanonicalCarrier,
    typed_error: OwnedCanonicalCarrier,
    ordered_attempt_evidence: OwnedCanonicalCarrier,
}

impl OwnedFeedAcquisitionEvidence {
    pub const fn new(
        outcome: FeedAcquisitionOutcomeKind,
        response: OwnedCanonicalCarrier,
        typed_error: OwnedCanonicalCarrier,
        ordered_attempt_evidence: OwnedCanonicalCarrier,
    ) -> Self {
        Self {
            outcome,
            response,
            typed_error,
            ordered_attempt_evidence,
        }
    }

    pub const fn outcome(&self) -> FeedAcquisitionOutcomeKind {
        self.outcome
    }

    pub const fn response(&self) -> &OwnedCanonicalCarrier {
        &self.response
    }

    pub const fn typed_error(&self) -> &OwnedCanonicalCarrier {
        &self.typed_error
    }

    pub const fn ordered_attempt_evidence(&self) -> &OwnedCanonicalCarrier {
        &self.ordered_attempt_evidence
    }

    fn verify(&self) -> Result<VerifiedFeedAcquisitionResponse, AcquisitionV2Error> {
        verify_feed_acquisition_response(
            self.outcome,
            self.response.borrowed(),
            self.typed_error.borrowed(),
            self.ordered_attempt_evidence.borrowed(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFeedIntent {
    intent_hash: String,
    feed_ordinal: usize,
    descriptor_hash: String,
}

impl VerifiedFeedIntent {
    pub fn intent_hash(&self) -> &str {
        &self.intent_hash
    }

    pub const fn feed_ordinal(&self) -> usize {
        self.feed_ordinal
    }

    pub fn descriptor_hash(&self) -> &str {
        &self.descriptor_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorBootUnsealedFeedIntent {
    intent_id: String,
    intent_hash: String,
    feed_ordinal: usize,
    descriptor_hash: String,
    prior_boot_instance_id: String,
}

impl PriorBootUnsealedFeedIntent {
    pub fn new(
        intent_id: impl Into<String>,
        intent_hash: impl Into<String>,
        feed_ordinal: usize,
        descriptor_hash: impl Into<String>,
        prior_boot_instance_id: impl Into<String>,
    ) -> Result<Self, AcquisitionV2Error> {
        let intent_id = intent_id.into();
        let intent_hash = intent_hash.into();
        let descriptor_hash = descriptor_hash.into();
        let prior_boot_instance_id = prior_boot_instance_id.into();
        if !is_canonical_uuid_v7(&intent_id) {
            return Err(AcquisitionV2Error::ambiguous(
                "prior_boot_intent_id must be a canonical UUIDv7",
            ));
        }
        verify_evidence_hash(&intent_hash, "prior_boot_intent_hash")?;
        verify_evidence_hash(&descriptor_hash, "prior_boot_feed_descriptor_hash")?;
        if !is_canonical_uuid_v7(&prior_boot_instance_id) {
            return Err(AcquisitionV2Error::ambiguous(
                "prior_boot_instance_id must be a canonical UUIDv7",
            ));
        }
        Ok(Self {
            intent_id,
            intent_hash,
            feed_ordinal,
            descriptor_hash,
            prior_boot_instance_id,
        })
    }

    pub fn intent_id(&self) -> &str {
        &self.intent_id
    }

    pub fn intent_hash(&self) -> &str {
        &self.intent_hash
    }

    pub const fn feed_ordinal(&self) -> usize {
        self.feed_ordinal
    }

    pub fn descriptor_hash(&self) -> &str {
        &self.descriptor_hash
    }

    pub fn prior_boot_instance_id(&self) -> &str {
        &self.prior_boot_instance_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AcquisitionUncertaintyReasonCode {
    AcquisitionOutcomeUncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationAcquisitionUncertainPreimageV1 {
    domain: String,
    schema_version: u64,
    uncertainty_id: String,
    intent_id: String,
    intent_sha256: String,
    prior_boot_instance_id: String,
    detection_boot_instance_id: String,
    detected_at: String,
    reason_code: AcquisitionUncertaintyReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationAcquisitionUncertainRecordV1 {
    preimage: GenerationAcquisitionUncertainPreimageV1,
    uncertainty_record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAcquisitionUncertaintyRecord {
    wire: GenerationAcquisitionUncertainRecordV1,
    canonical_bytes: Vec<u8>,
}

impl VerifiedAcquisitionUncertaintyRecord {
    pub fn uncertainty_record_hash(&self) -> &str {
        &self.wire.uncertainty_record_hash
    }

    pub fn intent_hash(&self) -> &str {
        &self.wire.preimage.intent_sha256
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
}

pub fn build_generation_acquisition_uncertainty_record(
    uncertainty_id: impl Into<String>,
    prior_boot_intent: &PriorBootUnsealedFeedIntent,
    detection_boot_instance_id: impl Into<String>,
    detected_at: impl Into<String>,
) -> Result<VerifiedAcquisitionUncertaintyRecord, AcquisitionV2Error> {
    let preimage = GenerationAcquisitionUncertainPreimageV1 {
        domain: "stock_analysis.selection_v2_generation_acquisition_uncertain.v1".into(),
        schema_version: 1,
        uncertainty_id: uncertainty_id.into(),
        intent_id: prior_boot_intent.intent_id.clone(),
        intent_sha256: prior_boot_intent.intent_hash.clone(),
        prior_boot_instance_id: prior_boot_intent.prior_boot_instance_id.clone(),
        detection_boot_instance_id: detection_boot_instance_id.into(),
        detected_at: detected_at.into(),
        reason_code: AcquisitionUncertaintyReasonCode::AcquisitionOutcomeUncertain,
    };
    verify_uncertainty_preimage(&preimage)?;
    let preimage_bytes = serde_json::to_vec(&preimage).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("uncertainty preimage cannot be encoded: {error}"))
    })?;
    let mut hash_preimage =
        b"stock_analysis.selection_v2_generation_acquisition_uncertain.v1\0".to_vec();
    hash_preimage.extend_from_slice(&preimage_bytes);
    let wire = GenerationAcquisitionUncertainRecordV1 {
        preimage,
        uncertainty_record_hash: sha256_bytes(&hash_preimage),
    };
    let canonical_bytes = serde_json::to_vec(&wire).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("uncertainty record cannot be encoded: {error}"))
    })?;
    Ok(VerifiedAcquisitionUncertaintyRecord {
        wire,
        canonical_bytes,
    })
}

pub fn parse_generation_acquisition_uncertainty_record(
    canonical_bytes: &[u8],
) -> Result<VerifiedAcquisitionUncertaintyRecord, AcquisitionV2Error> {
    let wire: GenerationAcquisitionUncertainRecordV1 = serde_json::from_slice(canonical_bytes)
        .map_err(|error| {
            AcquisitionV2Error::ambiguous(format!("uncertainty record decode failed: {error}"))
        })?;
    let encoded = serde_json::to_vec(&wire).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("uncertainty record encode failed: {error}"))
    })?;
    if encoded != canonical_bytes {
        return Err(AcquisitionV2Error::ambiguous(
            "uncertainty record is not canonical",
        ));
    }
    verify_uncertainty_preimage(&wire.preimage)?;
    let preimage_bytes = serde_json::to_vec(&wire.preimage).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!(
            "uncertainty preimage cannot be re-encoded: {error}"
        ))
    })?;
    let mut hash_preimage =
        b"stock_analysis.selection_v2_generation_acquisition_uncertain.v1\0".to_vec();
    hash_preimage.extend_from_slice(&preimage_bytes);
    if !is_lower_hash(&wire.uncertainty_record_hash)
        || sha256_bytes(&hash_preimage) != wire.uncertainty_record_hash
    {
        return Err(AcquisitionV2Error::ambiguous(
            "uncertainty record hash does not bind its strict preimage",
        ));
    }
    Ok(VerifiedAcquisitionUncertaintyRecord {
        wire,
        canonical_bytes: canonical_bytes.to_vec(),
    })
}

#[async_trait::async_trait]
pub trait SerialIngressJournal: Send {
    async fn append_sync_read_back_plan_intent(
        &mut self,
        plan: &VerifiedCadenceFeedPlan,
    ) -> Result<String, AcquisitionV2Error>;

    async fn append_sync_read_back_feed_intent(
        &mut self,
        plan_intent_hash: &str,
        ordinal: usize,
        descriptor_hash: &str,
    ) -> Result<String, AcquisitionV2Error>;

    async fn append_sync_read_back_feed_seal(
        &mut self,
        intent: &VerifiedFeedIntent,
        evidence: &OwnedFeedAcquisitionEvidence,
    ) -> Result<String, AcquisitionV2Error>;

    async fn append_sync_read_back_uncertainty(
        &mut self,
        intent: &PriorBootUnsealedFeedIntent,
    ) -> Result<VerifiedAcquisitionUncertaintyRecord, AcquisitionV2Error>;

    async fn append_sync_read_back_cycle_terminal(
        &mut self,
        plan_intent_hash: &str,
        prefix: &VerifiedIngressResolutionPrefix,
    ) -> Result<String, AcquisitionV2Error>;
}

#[async_trait::async_trait]
pub trait SerialIngressProvider: Send {
    async fn fetch_after_intent_read_back(
        &mut self,
        intent: &VerifiedFeedIntent,
    ) -> Result<OwnedFeedAcquisitionEvidence, AcquisitionV2Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub struct CompletedSerialIngressCycle {
    prefix: VerifiedIngressResolutionPrefix,
    terminal_receipt_hash: String,
}

impl CompletedSerialIngressCycle {
    pub const fn prefix(&self) -> &VerifiedIngressResolutionPrefix {
        &self.prefix
    }

    pub fn terminal_receipt_hash(&self) -> &str {
        &self.terminal_receipt_hash
    }
}

pub async fn run_serial_ingress_acquisition<J, P>(
    plan: &VerifiedCadenceFeedPlan,
    descriptor_hashes_in_registration_order: &[String],
    journal: &mut J,
    provider: &mut P,
) -> Result<CompletedSerialIngressCycle, AcquisitionV2Error>
where
    J: SerialIngressJournal,
    P: SerialIngressProvider,
{
    validate_serial_plan(plan, descriptor_hashes_in_registration_order)?;

    let plan_intent_hash = journal.append_sync_read_back_plan_intent(plan).await?;
    verify_evidence_hash(&plan_intent_hash, "ingress_plan_intent_hash")?;

    let mut resolutions = Vec::with_capacity(descriptor_hashes_in_registration_order.len());
    for (feed_ordinal, descriptor_hash) in
        descriptor_hashes_in_registration_order.iter().enumerate()
    {
        let intent_hash = journal
            .append_sync_read_back_feed_intent(&plan_intent_hash, feed_ordinal, descriptor_hash)
            .await?;
        verify_evidence_hash(&intent_hash, "ingress_feed_intent_hash")?;
        let intent = VerifiedFeedIntent {
            intent_hash,
            feed_ordinal,
            descriptor_hash: descriptor_hash.clone(),
        };

        let evidence = provider.fetch_after_intent_read_back(&intent).await?;
        let verified_response = evidence.verify()?;
        let outcome = verified_response.outcome();
        let seal_hash = journal
            .append_sync_read_back_feed_seal(&intent, &evidence)
            .await?;
        verify_evidence_hash(&seal_hash, "ingress_feed_seal_hash")?;
        resolutions.push(ResolvedFeedEvidence::sealed(
            intent.intent_hash,
            seal_hash,
            verified_response,
        )?);

        if !matches!(
            outcome,
            FeedAcquisitionOutcomeKind::SuccessNonempty | FeedAcquisitionOutcomeKind::VerifiedEmpty
        ) {
            break;
        }
    }

    let prefix = verify_ingress_resolution_prefix(
        descriptor_hashes_in_registration_order.len(),
        resolutions,
    )?;
    let terminal_receipt_hash = journal
        .append_sync_read_back_cycle_terminal(&plan_intent_hash, &prefix)
        .await?;
    verify_evidence_hash(
        &terminal_receipt_hash,
        "ingress_cycle_terminal_receipt_hash",
    )?;
    Ok(CompletedSerialIngressCycle {
        prefix,
        terminal_receipt_hash,
    })
}

pub async fn recover_prior_boot_unsealed_intent<J>(
    plan: &VerifiedCadenceFeedPlan,
    descriptor_hashes_in_registration_order: &[String],
    plan_intent_hash: impl Into<String>,
    mut sealed_prefix: Vec<ResolvedFeedEvidence>,
    prior_boot_intent: PriorBootUnsealedFeedIntent,
    journal: &mut J,
) -> Result<CompletedSerialIngressCycle, AcquisitionV2Error>
where
    J: SerialIngressJournal,
{
    validate_serial_plan(plan, descriptor_hashes_in_registration_order)?;
    let plan_intent_hash = plan_intent_hash.into();
    verify_evidence_hash(&plan_intent_hash, "ingress_plan_intent_hash")?;
    if prior_boot_intent.feed_ordinal != sealed_prefix.len()
        || descriptor_hashes_in_registration_order
            .get(prior_boot_intent.feed_ordinal)
            .is_none_or(|descriptor| descriptor != &prior_boot_intent.descriptor_hash)
    {
        return Err(AcquisitionV2Error::ambiguous(
            "prior-boot intent does not immediately follow the sealed registration prefix",
        ));
    }
    validate_reusable_normal_prefix(&sealed_prefix)?;

    let uncertainty_record = journal
        .append_sync_read_back_uncertainty(&prior_boot_intent)
        .await?;
    if uncertainty_record.intent_hash() != prior_boot_intent.intent_hash {
        return Err(AcquisitionV2Error::ambiguous(
            "read-back uncertainty record does not join the prior-boot intent",
        ));
    }
    let uncertainty_record_hash = uncertainty_record.uncertainty_record_hash().to_owned();
    verify_evidence_hash(
        &uncertainty_record_hash,
        "generation_acquisition_uncertainty_record_hash",
    )?;
    sealed_prefix.push(ResolvedFeedEvidence::uncertain(
        prior_boot_intent.intent_hash,
        uncertainty_record_hash,
    )?);
    let prefix = verify_ingress_resolution_prefix(
        descriptor_hashes_in_registration_order.len(),
        sealed_prefix,
    )?;
    let terminal_receipt_hash = journal
        .append_sync_read_back_cycle_terminal(&plan_intent_hash, &prefix)
        .await?;
    verify_evidence_hash(
        &terminal_receipt_hash,
        "ingress_cycle_terminal_receipt_hash",
    )?;
    Ok(CompletedSerialIngressCycle {
        prefix,
        terminal_receipt_hash,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct FeedPlanV2Error {
    code: &'static str,
    detail: String,
}

impl FeedPlanV2Error {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for FeedPlanV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for FeedPlanV2Error {}

#[derive(Serialize)]
struct FrozenFeedPlanPreimage<'a> {
    domain: &'static str,
    schema_version: u64,
    descriptor_hashes_in_registration_order: &'a [String],
    news_per_feed_limit: usize,
}

#[derive(Serialize)]
struct RegisteredFeedDescriptorPreimage<'a> {
    domain: &'static str,
    schema_version: u64,
    feed_name: &'a str,
    gateway_provider: &'a str,
    provider_id: &'a str,
    source_contract: &'a str,
    capability_name: &'a str,
    max_limit: u32,
    upstream_revision: &'a str,
}

pub fn registered_feed_descriptor_hash(
    registration: &RegisteredGlobalNewsFeed,
) -> Result<String, FeedPlanV2Error> {
    if registration.feed_name.is_empty()
        || registration.gateway_provider.is_empty()
        || registration.provider_id.is_empty()
        || registration.source_contract.is_empty()
        || registration.capability_name.is_empty()
        || registration.max_limit != REGISTERED_GLOBAL_NEWS_LIMIT
        || registration.max_limit as usize != NEWS_PER_FEED_LIMIT
        || !is_lower_hex_len(registration.upstream_revision, 40)
    {
        return Err(FeedPlanV2Error::new(
            "generation_state_ambiguous",
            "registered feed descriptor violates the frozen identity/limit/revision contract",
        ));
    }
    let preimage = RegisteredFeedDescriptorPreimage {
        domain: "stock_analysis.selection_v2_registered_feed_descriptor.v1",
        schema_version: 1,
        feed_name: registration.feed_name,
        gateway_provider: registration.gateway_provider,
        provider_id: registration.provider_id,
        source_contract: registration.source_contract,
        capability_name: registration.capability_name,
        max_limit: registration.max_limit,
        upstream_revision: registration.upstream_revision,
    };
    let bytes = serde_json::to_vec(&preimage).map_err(|error| {
        FeedPlanV2Error::new(
            "generation_state_ambiguous",
            format!("registered feed descriptor cannot be encoded: {error}"),
        )
    })?;
    let mut hash_preimage = b"stock_analysis.selection_v2_registered_feed_descriptor.v1\0".to_vec();
    hash_preimage.extend_from_slice(&bytes);
    Ok(sha256_bytes(&hash_preimage))
}

pub fn freeze_registered_global_news_feed_plan_at_activation(
) -> Result<FrozenFeedPlan, FeedPlanV2Error> {
    let descriptor_hashes = registered_global_news_feeds()
        .iter()
        .map(registered_feed_descriptor_hash)
        .collect::<Result<Vec<_>, _>>()?;
    match freeze_feed_plan_at_activation(descriptor_hashes)? {
        FeedPlanActivation::Active(plan) => Ok(plan),
        FeedPlanActivation::Disabled(_) => Err(FeedPlanV2Error::new(
            "generation_state_ambiguous",
            "checked-in registered global-news feed plan is unexpectedly empty",
        )),
    }
}

pub fn freeze_feed_plan_at_activation(
    descriptor_hashes_in_registration_order: Vec<String>,
) -> Result<FeedPlanActivation, FeedPlanV2Error> {
    use crate::selection::activation_runtime::SelectionDisabledReason;

    if descriptor_hashes_in_registration_order.is_empty() {
        return Ok(FeedPlanActivation::Disabled(
            SelectionDisabledReason::IngressContractUnavailable,
        ));
    }
    validate_feed_descriptor_hashes(
        &descriptor_hashes_in_registration_order,
        "generation_state_ambiguous",
    )?;
    let plan_hash = feed_plan_hash(&descriptor_hashes_in_registration_order)?;
    Ok(FeedPlanActivation::Active(FrozenFeedPlan {
        descriptor_hashes_in_registration_order,
        plan_hash,
    }))
}

pub fn verify_feed_plan_before_cadence(
    frozen: &FrozenFeedPlan,
    current_descriptor_hashes_in_registration_order: &[String],
) -> Result<VerifiedCadenceFeedPlan, FeedPlanV2Error> {
    validate_feed_descriptor_hashes(
        current_descriptor_hashes_in_registration_order,
        "config_snapshot_conflict",
    )?;
    let current_hash = feed_plan_hash(current_descriptor_hashes_in_registration_order)
        .map_err(|error| FeedPlanV2Error::new("config_snapshot_conflict", error.detail))?;
    if current_descriptor_hashes_in_registration_order
        != frozen.descriptor_hashes_in_registration_order
        || current_hash != frozen.plan_hash
    {
        return Err(FeedPlanV2Error::new(
            "config_snapshot_conflict",
            "feed descriptor count, order or identity drifted after activation",
        ));
    }
    Ok(VerifiedCadenceFeedPlan {
        feed_count: current_descriptor_hashes_in_registration_order.len(),
        plan_hash: current_hash,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedIngressResolutionPrefix {
    resolutions: Vec<ResolvedFeedEvidence>,
    aggregate_outcome_kind: IngressAggregateOutcomeKind,
    terminal_kind: IngressCycleTerminalKind,
    pending_dependency_code: Option<SelectionPendingDependencyCode>,
    failed_non_retryable_code: Option<SelectionFailedNonRetryableCode>,
    uncontacted_suffix_count: usize,
    stopped_after_feed_ordinal: Option<usize>,
    verified_empty_feed_count: usize,
    total_response_record_count: usize,
}

impl VerifiedIngressResolutionPrefix {
    pub const fn aggregate_outcome_kind(&self) -> IngressAggregateOutcomeKind {
        self.aggregate_outcome_kind
    }

    pub const fn terminal_kind(&self) -> IngressCycleTerminalKind {
        self.terminal_kind
    }

    pub const fn pending_dependency_code(&self) -> Option<SelectionPendingDependencyCode> {
        self.pending_dependency_code
    }

    pub const fn failed_non_retryable_code(&self) -> Option<SelectionFailedNonRetryableCode> {
        self.failed_non_retryable_code
    }

    pub fn resolved_feed_count(&self) -> usize {
        self.resolutions.len()
    }

    pub const fn uncontacted_suffix_count(&self) -> usize {
        self.uncontacted_suffix_count
    }

    pub const fn stopped_after_feed_ordinal(&self) -> Option<usize> {
        self.stopped_after_feed_ordinal
    }

    pub const fn verified_empty_feed_count(&self) -> usize {
        self.verified_empty_feed_count
    }

    pub const fn total_response_record_count(&self) -> usize {
        self.total_response_record_count
    }

    pub fn resolutions(&self) -> &[ResolvedFeedEvidence] {
        &self.resolutions
    }
}

pub fn verify_ingress_resolution_prefix(
    total_feed_count: usize,
    resolutions: Vec<ResolvedFeedEvidence>,
) -> Result<VerifiedIngressResolutionPrefix, AcquisitionV2Error> {
    if total_feed_count == 0 || resolutions.is_empty() || resolutions.len() > total_feed_count {
        return Err(AcquisitionV2Error::ambiguous(
            "ingress resolution prefix has invalid plan cardinality",
        ));
    }

    let mut intent_hashes = HashSet::with_capacity(resolutions.len());
    let mut verified_empty_feed_count = 0_usize;
    let mut total_response_record_count = 0_usize;
    let mut terminal = None;

    for (ordinal, evidence) in resolutions.iter().enumerate() {
        let (intent_hash, outcome, response_count) =
            classify_resolved_feed(evidence).map_err(|detail| {
                AcquisitionV2Error::ambiguous(format!(
                    "feed resolution {ordinal} is invalid: {detail}"
                ))
            })?;
        if !intent_hashes.insert(intent_hash) {
            return Err(AcquisitionV2Error::ambiguous(
                "ingress resolution prefix repeats an intent hash",
            ));
        }
        if terminal.is_some() {
            return Err(AcquisitionV2Error::ambiguous(
                "a terminal feed resolution must end the resolved prefix",
            ));
        }

        match outcome {
            ResolvedOutcome::SuccessNonempty => {
                total_response_record_count = total_response_record_count
                    .checked_add(response_count)
                    .ok_or_else(|| {
                        AcquisitionV2Error::ambiguous("response record counter overflow")
                    })?;
            }
            ResolvedOutcome::VerifiedEmpty => {
                verified_empty_feed_count =
                    verified_empty_feed_count.checked_add(1).ok_or_else(|| {
                        AcquisitionV2Error::ambiguous("verified-empty counter overflow")
                    })?;
            }
            ResolvedOutcome::Pending(code) => {
                terminal = Some(ResolutionTerminal::Pending(code, ordinal));
            }
            ResolvedOutcome::Failed(code) => {
                total_response_record_count = total_response_record_count
                    .checked_add(response_count)
                    .ok_or_else(|| {
                        AcquisitionV2Error::ambiguous("response record counter overflow")
                    })?;
                terminal = Some(ResolutionTerminal::Failed(code, ordinal));
            }
        }
    }

    let resolved_feed_count = resolutions.len();
    let uncontacted_suffix_count = total_feed_count - resolved_feed_count;
    let (
        aggregate_outcome_kind,
        terminal_kind,
        pending_dependency_code,
        failed_non_retryable_code,
        stopped_after_feed_ordinal,
    ) = match terminal {
        Some(ResolutionTerminal::Pending(code, ordinal)) => (
            IngressAggregateOutcomeKind::PendingDependency,
            IngressCycleTerminalKind::PendingDependency,
            Some(code),
            None,
            Some(ordinal),
        ),
        Some(ResolutionTerminal::Failed(code, ordinal)) => (
            IngressAggregateOutcomeKind::FailedNonRetryable,
            IngressCycleTerminalKind::FailedNonRetryable,
            None,
            Some(code),
            Some(ordinal),
        ),
        None if resolved_feed_count != total_feed_count => {
            return Err(AcquisitionV2Error::ambiguous(
                "a partial normal prefix has no durable terminal resolution",
            ));
        }
        None if verified_empty_feed_count == total_feed_count => (
            IngressAggregateOutcomeKind::VerifiedEmpty,
            IngressCycleTerminalKind::VerifiedEmpty,
            None,
            None,
            None,
        ),
        None => (
            IngressAggregateOutcomeKind::SuccessNonempty,
            IngressCycleTerminalKind::SourceIngressCommitted,
            None,
            None,
            None,
        ),
    };

    Ok(VerifiedIngressResolutionPrefix {
        resolutions,
        aggregate_outcome_kind,
        terminal_kind,
        pending_dependency_code,
        failed_non_retryable_code,
        uncontacted_suffix_count,
        stopped_after_feed_ordinal,
        verified_empty_feed_count,
        total_response_record_count,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct AcquisitionV2Error {
    code: &'static str,
    detail: String,
}

impl AcquisitionV2Error {
    fn ambiguous(detail: impl Into<String>) -> Self {
        Self {
            code: "generation_state_ambiguous",
            detail: detail.into(),
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for AcquisitionV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for AcquisitionV2Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GenerationFeedErrorCode {
    FeedUnavailable,
    ProviderCancelled,
    FeedResponseLimitExceeded,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationFeedTypedErrorV1 {
    domain: String,
    schema_version: u64,
    code: GenerationFeedErrorCode,
    redacted_detail_sha256_or_null: Option<String>,
    retryable: bool,
}

pub fn verify_feed_acquisition_response(
    outcome: FeedAcquisitionOutcomeKind,
    response: CanonicalCarrier<'_>,
    typed_error: CanonicalCarrier<'_>,
    ordered_attempt_evidence: CanonicalCarrier<'_>,
) -> Result<VerifiedFeedAcquisitionResponse, AcquisitionV2Error> {
    let response = response.verified_bytes("response")?;
    let typed_error = typed_error.verified_bytes("typed_error")?;
    let ordered_attempt_evidence =
        ordered_attempt_evidence.verified_bytes("ordered_attempt_evidence")?;
    let attempts = ordered_attempt_evidence.ok_or_else(|| {
        AcquisitionV2Error::ambiguous("ordered attempt evidence must always be present")
    })?;
    let attempt_values = decode_canonical_vector(attempts, "ordered_attempt_evidence")?;
    if attempt_values.is_empty() {
        return Err(AcquisitionV2Error::ambiguous(
            "ordered attempt evidence must be non-empty",
        ));
    }

    let response_count = match response {
        Some(bytes) => decode_canonical_vector(bytes, "response")?.len(),
        None => 0,
    };
    let decoded_error = typed_error.map(decode_typed_error).transpose()?;

    let valid = match outcome {
        FeedAcquisitionOutcomeKind::SuccessNonempty => {
            (1..=NEWS_PER_FEED_LIMIT).contains(&response_count)
                && response.is_some()
                && decoded_error.is_none()
        }
        FeedAcquisitionOutcomeKind::VerifiedEmpty => {
            response_count == 0 && response.is_some() && decoded_error.is_none()
        }
        FeedAcquisitionOutcomeKind::TransportFailure => {
            response.is_none()
                && matches!(
                    decoded_error,
                    Some(GenerationFeedTypedErrorV1 {
                        code: GenerationFeedErrorCode::FeedUnavailable,
                        retryable: true,
                        ..
                    })
                )
        }
        FeedAcquisitionOutcomeKind::ProviderCancelled => {
            response.is_none()
                && matches!(
                    decoded_error,
                    Some(GenerationFeedTypedErrorV1 {
                        code: GenerationFeedErrorCode::ProviderCancelled,
                        retryable: true,
                        ..
                    })
                )
        }
        FeedAcquisitionOutcomeKind::FeedResponseLimitExceeded => {
            response_count > NEWS_PER_FEED_LIMIT
                && response.is_some()
                && matches!(
                    decoded_error,
                    Some(GenerationFeedTypedErrorV1 {
                        code: GenerationFeedErrorCode::FeedResponseLimitExceeded,
                        retryable: false,
                        ..
                    })
                )
        }
    };
    if !valid {
        return Err(AcquisitionV2Error::ambiguous(
            "feed response/error/null/count matrix mismatch",
        ));
    }

    Ok(VerifiedFeedAcquisitionResponse {
        outcome,
        sealed_response_record_count: response_count,
    })
}

fn decode_canonical_vector(
    bytes: &[u8],
    label: &'static str,
) -> Result<Vec<serde_json::Value>, AcquisitionV2Error> {
    let values: Vec<serde_json::Value> = serde_json::from_slice(bytes).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("{label} is not a JSON vector: {error}"))
    })?;
    let canonical = serde_json::to_vec(&values).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("{label} cannot be serialized: {error}"))
    })?;
    if canonical == bytes {
        Ok(values)
    } else {
        Err(AcquisitionV2Error::ambiguous(format!(
            "{label} is not canonical"
        )))
    }
}

fn decode_typed_error(bytes: &[u8]) -> Result<GenerationFeedTypedErrorV1, AcquisitionV2Error> {
    let error: GenerationFeedTypedErrorV1 = serde_json::from_slice(bytes).map_err(|decode| {
        AcquisitionV2Error::ambiguous(format!("typed error decode failed: {decode}"))
    })?;
    let canonical = serde_json::to_vec(&error).map_err(|encode| {
        AcquisitionV2Error::ambiguous(format!("typed error encode failed: {encode}"))
    })?;
    if canonical != bytes
        || error.domain != "stock_analysis.selection_v2_generation_feed_error.v1"
        || error.schema_version != 1
        || error
            .redacted_detail_sha256_or_null
            .as_deref()
            .is_some_and(|hash| !is_lower_hash(hash))
    {
        return Err(AcquisitionV2Error::ambiguous(
            "typed error carrier is noncanonical or invalid",
        ));
    }
    Ok(error)
}

fn is_lower_hash(value: &str) -> bool {
    is_lower_hex_len(value, 64)
}

fn is_lower_hex_len(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn verify_evidence_hash(hash: &str, label: &'static str) -> Result<(), AcquisitionV2Error> {
    if is_lower_hash(hash) {
        Ok(())
    } else {
        Err(AcquisitionV2Error::ambiguous(format!(
            "{label} must be a lowercase SHA-256 identity"
        )))
    }
}

fn validate_feed_descriptor_hashes(
    hashes: &[String],
    error_code: &'static str,
) -> Result<(), FeedPlanV2Error> {
    if hashes.is_empty() {
        return Err(FeedPlanV2Error::new(
            error_code,
            "active feed registration plan must be nonempty",
        ));
    }
    let mut unique = HashSet::with_capacity(hashes.len());
    for hash in hashes {
        if !is_lower_hash(hash) || !unique.insert(hash) {
            return Err(FeedPlanV2Error::new(
                error_code,
                "feed descriptor identities must be unique lowercase SHA-256 values",
            ));
        }
    }
    Ok(())
}

fn feed_plan_hash(hashes: &[String]) -> Result<String, FeedPlanV2Error> {
    let preimage = FrozenFeedPlanPreimage {
        domain: "stock_analysis.selection_v2_frozen_feed_plan.v1",
        schema_version: 1,
        descriptor_hashes_in_registration_order: hashes,
        news_per_feed_limit: NEWS_PER_FEED_LIMIT,
    };
    let bytes = serde_json::to_vec(&preimage).map_err(|error| {
        FeedPlanV2Error::new(
            "generation_state_ambiguous",
            format!("feed plan cannot be canonically encoded: {error}"),
        )
    })?;
    let mut domain_separated = b"stock_analysis.selection_v2_frozen_feed_plan.v1\0".to_vec();
    domain_separated.extend_from_slice(&bytes);
    Ok(sha256_bytes(&domain_separated))
}

fn validate_serial_plan(
    plan: &VerifiedCadenceFeedPlan,
    hashes: &[String],
) -> Result<(), AcquisitionV2Error> {
    if hashes.is_empty() || hashes.len() != plan.feed_count {
        return Err(AcquisitionV2Error::ambiguous(
            "serial ingress plan count differs from the verified cadence plan",
        ));
    }
    let current_hash =
        feed_plan_hash(hashes).map_err(|error| AcquisitionV2Error::ambiguous(error.detail))?;
    if current_hash != plan.plan_hash {
        return Err(AcquisitionV2Error::ambiguous(
            "serial ingress plan identity differs from the verified cadence plan",
        ));
    }
    Ok(())
}

fn validate_reusable_normal_prefix(
    prefix: &[ResolvedFeedEvidence],
) -> Result<(), AcquisitionV2Error> {
    let mut intent_hashes = HashSet::with_capacity(prefix.len());
    for evidence in prefix {
        let (intent_hash, outcome, _) =
            classify_resolved_feed(evidence).map_err(AcquisitionV2Error::ambiguous)?;
        if !intent_hashes.insert(intent_hash)
            || !matches!(
                outcome,
                ResolvedOutcome::SuccessNonempty | ResolvedOutcome::VerifiedEmpty
            )
        {
            return Err(AcquisitionV2Error::ambiguous(
                "recovery may reuse only one unique contiguous normal sealed prefix",
            ));
        }
    }
    Ok(())
}

fn is_canonical_uuid_v7(value: &str) -> bool {
    if value.len() != 36
        || value.as_bytes()[8] != b'-'
        || value.as_bytes()[13] != b'-'
        || value.as_bytes()[18] != b'-'
        || value.as_bytes()[23] != b'-'
        || value.as_bytes()[14] != b'7'
        || !matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
    {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
    })
}

fn verify_uncertainty_preimage(
    preimage: &GenerationAcquisitionUncertainPreimageV1,
) -> Result<(), AcquisitionV2Error> {
    if preimage.domain != "stock_analysis.selection_v2_generation_acquisition_uncertain.v1"
        || preimage.schema_version != 1
        || !is_canonical_uuid_v7(&preimage.uncertainty_id)
        || !is_canonical_uuid_v7(&preimage.intent_id)
        || !is_lower_hash(&preimage.intent_sha256)
        || !is_canonical_uuid_v7(&preimage.prior_boot_instance_id)
        || !is_canonical_uuid_v7(&preimage.detection_boot_instance_id)
        || !is_rfc3339_nanos_utc(&preimage.detected_at)
        || preimage.reason_code != AcquisitionUncertaintyReasonCode::AcquisitionOutcomeUncertain
    {
        return Err(AcquisitionV2Error::ambiguous(
            "uncertainty preimage violates its closed domain/schema/id/time/reason contract",
        ));
    }
    Ok(())
}

fn is_rfc3339_nanos_utc(value: &str) -> bool {
    parse_rfc3339_nanos_utc(value, "timestamp").is_ok()
}

fn parse_rfc3339_nanos_utc(
    value: &str,
    label: &'static str,
) -> Result<DateTime<Utc>, AcquisitionV2Error> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|error| {
        AcquisitionV2Error::ambiguous(format!("{label} is not RFC3339: {error}"))
    })?;
    if parsed.offset().local_minus_utc() != 0
        || parsed.to_rfc3339_opts(SecondsFormat::Nanos, true) != value
    {
        return Err(AcquisitionV2Error::ambiguous(format!(
            "{label} must be fixed-width RFC3339 nanoseconds in UTC"
        )));
    }
    Ok(parsed.with_timezone(&Utc))
}

fn valid_mode_namespace(value: &str) -> bool {
    value == "production" || (is_trim_stable_nonempty(value) && value.starts_with("TEST_CODE_"))
}

fn is_trim_stable_nonempty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedOutcome {
    SuccessNonempty,
    VerifiedEmpty,
    Pending(SelectionPendingDependencyCode),
    Failed(SelectionFailedNonRetryableCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolutionTerminal {
    Pending(SelectionPendingDependencyCode, usize),
    Failed(SelectionFailedNonRetryableCode, usize),
}

fn classify_resolved_feed(
    evidence: &ResolvedFeedEvidence,
) -> Result<(&str, ResolvedOutcome, usize), &'static str> {
    match (&evidence.resolution, evidence.response.as_ref()) {
        (
            FeedAcquisitionResolution::Sealed { intent_hash, .. },
            Some(VerifiedFeedAcquisitionResponse {
                outcome: FeedAcquisitionOutcomeKind::SuccessNonempty,
                sealed_response_record_count,
            }),
        ) => Ok((
            intent_hash,
            ResolvedOutcome::SuccessNonempty,
            *sealed_response_record_count,
        )),
        (
            FeedAcquisitionResolution::Sealed { intent_hash, .. },
            Some(VerifiedFeedAcquisitionResponse {
                outcome: FeedAcquisitionOutcomeKind::VerifiedEmpty,
                ..
            }),
        ) => Ok((intent_hash, ResolvedOutcome::VerifiedEmpty, 0)),
        (
            FeedAcquisitionResolution::Sealed { intent_hash, .. },
            Some(VerifiedFeedAcquisitionResponse {
                outcome: FeedAcquisitionOutcomeKind::TransportFailure,
                ..
            }),
        ) => Ok((
            intent_hash,
            ResolvedOutcome::Pending(SelectionPendingDependencyCode::FeedUnavailable),
            0,
        )),
        (
            FeedAcquisitionResolution::Sealed { intent_hash, .. },
            Some(VerifiedFeedAcquisitionResponse {
                outcome: FeedAcquisitionOutcomeKind::ProviderCancelled,
                ..
            }),
        ) => Ok((
            intent_hash,
            ResolvedOutcome::Pending(SelectionPendingDependencyCode::ProviderCancelled),
            0,
        )),
        (
            FeedAcquisitionResolution::Sealed { intent_hash, .. },
            Some(VerifiedFeedAcquisitionResponse {
                outcome: FeedAcquisitionOutcomeKind::FeedResponseLimitExceeded,
                sealed_response_record_count,
            }),
        ) => Ok((
            intent_hash,
            ResolvedOutcome::Failed(SelectionFailedNonRetryableCode::FeedResponseLimitExceeded),
            *sealed_response_record_count,
        )),
        (FeedAcquisitionResolution::Uncertain { intent_hash, .. }, None) => Ok((
            intent_hash,
            ResolvedOutcome::Pending(SelectionPendingDependencyCode::AcquisitionOutcomeUncertain),
            0,
        )),
        (FeedAcquisitionResolution::Sealed { .. }, None)
        | (FeedAcquisitionResolution::Uncertain { .. }, Some(_)) => {
            Err("resolution and verified response presence mismatch")
        }
    }
}
