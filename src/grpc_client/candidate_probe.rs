//! Fixed candidate acceptance diagnostics. Recorded bytes never recreate live authority.
use super::{DataTransport, ExternalSystemCall, GrpcMarketClient, PreparedExternalEndpoint};
use crate::grpc_client::build_identity::{BuildIdentityTrust, CandidateCompiledInputs};
use crate::grpc_client::connection_qualification::ConnectionIdentity;
use crate::grpc_client::errors::{GrpcError, StatusErrorContext};
use crate::grpc_client::external_decoder::ExternalDecoder;
use crate::grpc_client::external_pb::magic::market::v1 as pb;
use crate::grpc_client::external_query_transport::{
    admit_external_payload, ExternalFrameFailureV1, ExternalQueryCall, ExternalQueryMethod,
    ExternalWireEvidenceV1, ExternalWireMaterialV1, EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
    EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES, EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
};
use crate::grpc_contract::methods::{ContractProfile, ExternalMethod, MethodIdentity};
use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::path::Path;

pub const CANDIDATE_PROFILE: &str = "windows-b7-20261002.17-diagnostic-v1";
pub(super) const CANDIDATE_ENDPOINT: &str = "https://10.211.55.3:50051";
pub(super) const CANDIDATE_TLS_NAME: &str = "magic-market.local";
pub const MAX_CANDIDATE_PLAN_BYTES: usize = 65_536;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_RPC: usize = 36;
const MAX_PARTS: usize = 96;
// At most 33 successful responses precede a terminal positive-call status.
// Add both expected negatives' six status parts, the schema3 captured body,
// and the terminal status' three parts. This preserves failed-prefix evidence.
const MAX_RAW_BYTES: usize = 43 * EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES;
pub const MAX_CANDIDATE_RECEIPT_BYTES: usize = MAX_RAW_BYTES + MAX_MANIFEST_BYTES + 64;
const MAGIC: &[u8; 16] = b"CANDIDATE-B7-V1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CandidateProbeError {
    #[error("candidate compiled inputs are invalid")]
    CompiledInputs,
    #[error("candidate plan is invalid")]
    PlanInput,
    #[error("candidate receipt is invalid")]
    ReceiptInput,
    #[error("candidate material exceeds its bound")]
    MaterialBound,
    #[error("candidate bundle or fixed endpoint is invalid")]
    BundleInput,
}

pub fn compiled_candidate_b7_inputs() -> Result<CandidateCompiledInputs, CandidateProbeError> {
    crate::grpc_client::build_identity::compiled_candidate_b7_inputs()
        .map_err(|_| CandidateProbeError::CompiledInputs)
}

// Sequence bounds are enforced while deserializing, before growing the list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct BoundedVec<T, const N: usize>(Vec<T>);
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for BoundedVec<T, N> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
            type Value = BoundedVec<T, N>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "a sequence of at most {N} items")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<Self::Value, A::Error> {
                let hint = a.size_hint().unwrap_or(0);
                if hint > N {
                    return Err(serde::de::Error::custom("candidate list bound"));
                }
                let mut v = Vec::with_capacity(hint);
                loop {
                    if v.len() == N {
                        if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                            return Err(serde::de::Error::custom("candidate list bound"));
                        }
                        break;
                    }
                    match a.next_element()? {
                        Some(x) => v.push(x),
                        None => break,
                    }
                }
                Ok(BoundedVec(v))
            }
        }
        d.deserialize_seq(Visitor::<T, N>(PhantomData))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum StepKind {
    Health,
    Capabilities,
    Business,
    UnauthenticatedHealth,
    UnsupportedSchemaNegative,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedStep {
    ordinal: usize,
    kind: StepKind,
    review_id: String,
    request_id: String,
    request_wire_hex: String,
    request_wire_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanDto {
    version: u32,
    compiled_inputs: CandidateCompiledInputs,
    endpoint: String,
    tls_server_name: String,
    maximum_rpc: usize,
    outer_concurrency: u32,
    automatic_retries: u32,
    provider_fallback: bool,
    steps: BoundedVec<PlannedStep, MAX_RPC>,
}

/// Immutable requests only. Reading this does not load credentials or create a connection.
pub struct CandidateProbePlan {
    dto: PlanDto,
}
impl CandidateProbePlan {
    pub fn windows_b7() -> Result<Self, CandidateProbeError> {
        let mut steps = Vec::with_capacity(MAX_RPC);
        for ordinal in 0..MAX_RPC {
            let spec = step_spec(ordinal);
            let request_id = crate::grpc_client::envelope::new_request_id();
            let wire = request_wire(&spec, &request_id);
            steps.push(PlannedStep {
                ordinal,
                kind: spec.kind,
                review_id: spec.review_id,
                request_id,
                request_wire_hex: hex::encode(&wire),
                request_wire_sha256: digest(&wire),
            });
        }
        let dto = PlanDto {
            version: 1,
            compiled_inputs: compiled_candidate_b7_inputs()?,
            endpoint: CANDIDATE_ENDPOINT.into(),
            tls_server_name: CANDIDATE_TLS_NAME.into(),
            maximum_rpc: MAX_RPC,
            outer_concurrency: 1,
            automatic_retries: 0,
            provider_fallback: false,
            steps: BoundedVec(steps),
        };
        validate_plan(&dto)?;
        Ok(Self { dto })
    }
    pub fn read_checked(bytes: &[u8]) -> Result<Self, CandidateProbeError> {
        if bytes.len() > MAX_CANDIDATE_PLAN_BYTES {
            return Err(CandidateProbeError::MaterialBound);
        }
        let dto: PlanDto =
            serde_json::from_slice(bytes).map_err(|_| CandidateProbeError::PlanInput)?;
        validate_plan(&dto)?;
        Ok(Self { dto })
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CandidateProbeError> {
        validate_plan(&self.dto)?;
        let bytes = serde_json::to_vec(&self.dto).map_err(|_| CandidateProbeError::PlanInput)?;
        if bytes.len() > MAX_CANDIDATE_PLAN_BYTES {
            return Err(CandidateProbeError::MaterialBound);
        }
        Ok(bytes)
    }
    pub async fn execute_from_bundle(
        &self,
        path: &Path,
    ) -> Result<CandidateProbeRunReceipt, CandidateProbeError> {
        validate_plan(&self.dto)?;
        let prepared = GrpcMarketClient::prepare_candidate_b7_bundle(path)
            .map_err(|_| CandidateProbeError::BundleInput)?;
        self.execute_prepared(prepared).await
    }
}

struct StepSpec {
    kind: StepKind,
    review_id: String,
    business: Option<usize>,
}
fn step_spec(ordinal: usize) -> StepSpec {
    match ordinal {
        0 => StepSpec {
            kind: StepKind::Health,
            review_id: "initial-health".into(),
            business: None,
        },
        1 => StepSpec {
            kind: StepKind::Capabilities,
            review_id: "initial-capabilities".into(),
            business: None,
        },
        2 => StepSpec {
            kind: StepKind::UnauthenticatedHealth,
            review_id: "negative-unauthenticated-health".into(),
            business: None,
        },
        3 => StepSpec {
            kind: StepKind::UnsupportedSchemaNegative,
            review_id: "negative-cninfo-schema3-l1".into(),
            business: Some(8),
        },
        _ => {
            let business = (ordinal - 4) / 4;
            let (kind, suffix) = match (ordinal - 4) % 4 {
                0 => (StepKind::Health, "health-before"),
                1 => (StepKind::Capabilities, "capabilities"),
                2 => (StepKind::Business, "business"),
                _ => (StepKind::Health, "health-after"),
            };
            StepSpec {
                kind,
                review_id: format!("{}.{}", business_review_id(business), suffix),
                business: Some(business),
            }
        }
    }
}
fn business_review_id(index: usize) -> &'static str {
    [
        "hithink-v1-l1",
        "hithink-v1-l15",
        "hithink-v2-l1",
        "hithink-v2-l15",
        "cninfo-v1-l1",
        "cninfo-v1-l300",
        "cninfo-v2-l1",
        "cninfo-v2-l300",
    ][index]
}
fn business_recipe(index: usize) -> (pb::Operation, ExternalQueryMethod, &'static str, u32, u32) {
    if index < 4 {
        (
            pb::Operation::HistoricalBars,
            ExternalQueryMethod::HistoricalBars,
            "HithinkFinance",
            if index < 2 { 1 } else { 2 },
            if index % 2 == 0 { 1 } else { 15 },
        )
    } else {
        (
            pb::Operation::MarketAnnouncements,
            ExternalQueryMethod::MarketAnnouncements,
            "Cninfo",
            if index == 8 {
                3
            } else if index < 6 {
                1
            } else {
                2
            },
            if index % 2 == 0 { 1 } else { 300 },
        )
    }
}
fn request_wire(spec: &StepSpec, id: &str) -> Vec<u8> {
    let context = Some(pb::RequestContext {
        protocol_version: 1,
        request_id: id.into(),
    });
    match spec.kind {
        StepKind::Health | StepKind::UnauthenticatedHealth => {
            pb::HealthRequest { context }.encode_to_vec()
        }
        StepKind::Capabilities => pb::CapabilitiesRequest { context }.encode_to_vec(),
        StepKind::Business | StepKind::UnsupportedSchemaNegative => {
            let (_, _, provider, schema_version, limit) = business_recipe(spec.business.unwrap());
            let (schema, data) = if provider == "HithinkFinance" {
                ("magic.market.historical_bars.request", format!(concat!(
                    "{{\"instrument\":{{\"exchange\":\"Shanghai\",\"code\":\"688561\",\"asset_class\":\"Equity\"}},",
                    "\"interval\":\"Day\",\"start\":\"2026-07-16\",\"end\":\"2026-07-30\",\"limit\":{}}}"), limit))
            } else {
                (
                    "magic.market.market_announcements.request",
                    format!(
                        "{{\"start\":\"2026-07-24\",\"end\":\"2026-07-24\",\"limit\":{limit}}}"
                    ),
                )
            };
            pb::QueryRequest {
                context,
                preferred_provider: provider.into(),
                allow_unadmitted: false,
                payload: Some(pb::CanonicalPayload {
                    schema: schema.into(),
                    schema_version,
                    content_type: "application/json; charset=utf-8".into(),
                    data: data.into_bytes(),
                }),
            }
            .encode_to_vec()
        }
    }
}
fn validate_plan(dto: &PlanDto) -> Result<(), CandidateProbeError> {
    if dto.version != 1
        || dto.compiled_inputs != compiled_candidate_b7_inputs()?
        || dto.endpoint != CANDIDATE_ENDPOINT
        || dto.tls_server_name != CANDIDATE_TLS_NAME
        || dto.maximum_rpc != MAX_RPC
        || dto.outer_concurrency != 1
        || dto.automatic_retries != 0
        || dto.provider_fallback
        || dto.steps.0.len() != MAX_RPC
    {
        return Err(CandidateProbeError::PlanInput);
    }
    let mut ids = BTreeSet::new();
    for (ordinal, step) in dto.steps.0.iter().enumerate() {
        let spec = step_spec(ordinal);
        if step.ordinal != ordinal
            || step.kind != spec.kind
            || step.review_id != spec.review_id
            || step.request_id.is_empty()
            || step.request_id.len() > 128
            || !step
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(step.request_id.as_str())
        {
            return Err(CandidateProbeError::PlanInput);
        }
        let wire = request_wire(&spec, &step.request_id);
        if step.request_wire_hex != hex::encode(&wire) || step.request_wire_sha256 != digest(&wire)
        {
            return Err(CandidateProbeError::PlanInput);
        }
    }
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ErrorClass {
    InvalidArgument,
    Unauthenticated,
    PermissionDenied,
    Unimplemented,
    ResourceExhausted,
    DeadlineExceeded,
    Unavailable,
    FailedPrecondition,
    Internal,
    Unknown,
}
fn error_class(e: &GrpcError) -> ErrorClass {
    match e {
        GrpcError::InvalidArgument { .. } => ErrorClass::InvalidArgument,
        GrpcError::Unauthenticated { .. } => ErrorClass::Unauthenticated,
        GrpcError::PermissionDenied { .. } => ErrorClass::PermissionDenied,
        GrpcError::Unimplemented { .. } => ErrorClass::Unimplemented,
        GrpcError::ResourceExhausted { .. } => ErrorClass::ResourceExhausted,
        GrpcError::DeadlineExceeded { .. } => ErrorClass::DeadlineExceeded,
        GrpcError::Unavailable { .. } => ErrorClass::Unavailable,
        GrpcError::FailedPrecondition { .. } => ErrorClass::FailedPrecondition,
        GrpcError::Internal { .. } => ErrorClass::Internal,
        GrpcError::Unknown { .. } => ErrorClass::Unknown,
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum StepOutcome {
    ObservedSuccess,
    ExpectedUnauthenticatedStatus,
    ExpectedUnsupportedSchemaStatus,
    StoppedIdentity,
    StoppedCapabilities,
    StoppedEnvelope,
    StoppedStatus,
    StoppedLocalGate,
    StoppedLocalWire,
    StoppedUnexpectedNegativeResponse,
}
impl StepOutcome {
    fn continues(self) -> bool {
        matches!(
            self,
            Self::ObservedSuccess
                | Self::ExpectedUnauthenticatedStatus
                | Self::ExpectedUnsupportedSchemaStatus
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum RunOutcome {
    CompletedRecordedObservation,
    StoppedRecordedObservation,
}
impl RunOutcome {
    fn name(self) -> &'static str {
        match self {
            Self::CompletedRecordedObservation => "CompletedRecordedObservation",
            Self::StoppedRecordedObservation => "StoppedRecordedObservation",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartMeta {
    byte_length: usize,
    sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Trailer {
    Absent,
    Bytes { part: usize },
    Malformed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum WireBody {
    Payload {
        part: usize,
        payload_sha256: String,
        decode_limit_bytes: usize,
    },
    Missing {
        framed_body_limit_bytes: usize,
    },
    Overflow {
        framed_body_limit_bytes: usize,
        observed_framed_body_bytes_at_least: usize,
    },
    InvalidFrame {
        part: usize,
        body_sha256: String,
        failure: ExternalFrameFailureV1,
        framed_body_limit_bytes: usize,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMeta {
    material: String,
    profile: String,
    method: ExternalQueryMethod,
    client_descriptor_sha256: String,
    body: WireBody,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Material {
    ControlResponse {
        part: usize,
    },
    QueryResponse {
        wire: WireMeta,
    },
    TonicStatus {
        wire: Option<WireMeta>,
        code: i32,
        message: usize,
        details: usize,
        trailer: Trailer,
        error_class: ErrorClass,
        matched_request_id_correlation: Option<String>,
    },
    LocalWireFailure {
        wire: WireMeta,
        error_class: ErrorClass,
    },
    LocalGateFailure {
        error_class: ErrorClass,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedStep {
    ordinal: usize,
    request_id_correlation: String,
    material: Material,
    outcome: StepOutcome,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptDto {
    version: u32,
    purpose: String,
    plan: PlanDto,
    actual_endpoint: String,
    actual_acquisition_authority: String,
    // A planned generation is retained even when connect fails; it is not live proof.
    connection_identity: ConnectionIdentity,
    connect_invocations: u32,
    connect_error_class: Option<ErrorClass>,
    steps: BoundedVec<ObservedStep, MAX_RPC>,
    parts: BoundedVec<PartMeta, MAX_PARTS>,
    outcome: RunOutcome,
}
struct RawBuilder {
    parts: Vec<PartMeta>,
    arena: Vec<u8>,
}
impl RawBuilder {
    fn new() -> Self {
        Self {
            parts: Vec::new(),
            arena: Vec::new(),
        }
    }
    fn add(&mut self, bytes: &[u8]) -> Result<usize, CandidateProbeError> {
        if bytes.len() > EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES
            || self.parts.len() >= MAX_PARTS
            || self
                .arena
                .len()
                .checked_add(bytes.len())
                .filter(|n| *n <= MAX_RAW_BYTES)
                .is_none()
        {
            return Err(CandidateProbeError::MaterialBound);
        }
        let index = self.parts.len();
        self.parts.push(PartMeta {
            byte_length: bytes.len(),
            sha256: digest(bytes),
        });
        self.arena.extend_from_slice(bytes);
        Ok(index)
    }
    fn wire(&mut self, wire: ExternalWireEvidenceV1) -> Result<WireMeta, CandidateProbeError> {
        let body = match wire.evidence {
            ExternalWireMaterialV1::Payload {
                protobuf_payload,
                payload_sha256,
                decode_limit_bytes,
            } => WireBody::Payload {
                part: self.add(&protobuf_payload)?,
                payload_sha256,
                decode_limit_bytes,
            },
            ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes,
            } => WireBody::Missing {
                framed_body_limit_bytes,
            },
            ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes,
                observed_framed_body_bytes_at_least,
            } => WireBody::Overflow {
                framed_body_limit_bytes,
                observed_framed_body_bytes_at_least,
            },
            ExternalWireMaterialV1::InvalidFrame {
                failure,
                grpc_body_bytes,
                body_sha256,
                framed_body_limit_bytes,
            } => WireBody::InvalidFrame {
                part: self.add(&grpc_body_bytes)?,
                failure,
                body_sha256,
                framed_body_limit_bytes,
            },
        };
        Ok(WireMeta {
            material: wire.material,
            profile: wire.profile,
            method: wire.method,
            client_descriptor_sha256: wire.client_descriptor_sha256,
            body,
        })
    }
}

/// Bytes plus diagnostic outcomes only; this exposes no client/session/provider capability.
pub struct CandidateProbeRunReceipt {
    dto: ReceiptDto,
    arena: Vec<u8>,
}
impl CandidateProbeRunReceipt {
    pub fn outcome_name(&self) -> &'static str {
        self.dto.outcome.name()
    }
    pub fn rpc_count(&self) -> usize {
        rpc_count(&self.dto)
    }
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CandidateProbeError> {
        let manifest =
            serde_json::to_vec(&self.dto).map_err(|_| CandidateProbeError::ReceiptInput)?;
        if manifest.len() > MAX_MANIFEST_BYTES || self.arena.len() > MAX_RAW_BYTES {
            return Err(CandidateProbeError::MaterialBound);
        }
        let mut bytes = Vec::with_capacity(MAGIC.len() + 4 + manifest.len() + self.arena.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(manifest.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&manifest);
        bytes.extend_from_slice(&self.arena);
        Ok(bytes)
    }
}
fn rpc_count(dto: &ReceiptDto) -> usize {
    dto.steps
        .0
        .iter()
        .filter(|s| !matches!(s.material, Material::LocalGateFailure { .. }))
        .count()
}

/// A revision-free recorded observation. No conversion to any live evidence is provided.
pub struct RecordedCandidateProbeEvidence {
    outcome: RunOutcome,
    rpc_count: usize,
    artifact_sha256: String,
}
impl RecordedCandidateProbeEvidence {
    pub fn outcome_name(&self) -> &'static str {
        self.outcome.name()
    }
    pub fn rpc_count(&self) -> usize {
        self.rpc_count
    }
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }
}

fn required_capabilities(response: &pb::CapabilitiesResponse, business: Option<usize>) -> bool {
    let indices: &[usize] = match business {
        Some(i) if i < 4 => &[0],
        Some(_) => &[4],
        None => &[0, 4],
    };
    indices.iter().all(|i| {
        let (op, _, provider, _, _) = business_recipe(*i);
        let mut matching = response
            .capabilities
            .iter()
            .filter(|c| c.operation == op as i32 && c.provider == provider);
        let Some(cap) = matching.next() else {
            return false;
        };
        matching.next().is_none()
            && cap.repository_admission == pb::AdmissionState::Admitted as i32
            && cap.runtime_available
            && cap.blocker.is_empty()
            && !cap.exact_scope.is_empty()
    })
}

fn control_outcome(step: &PlannedStep, raw: &[u8]) -> StepOutcome {
    let Ok(decoder) = ExternalDecoder::for_descriptor(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256) else {
        return StepOutcome::StoppedLocalWire;
    };
    match step.kind {
        StepKind::Health => match decoder.health(raw) {
            Ok(response)
                if response.request_id == step.request_id
                    && BuildIdentityTrust::candidate_b7_probe()
                        .is_ok_and(|t| t.current_health(&response).is_ok()) =>
            {
                StepOutcome::ObservedSuccess
            }
            _ => StepOutcome::StoppedIdentity,
        },
        StepKind::UnauthenticatedHealth => StepOutcome::StoppedUnexpectedNegativeResponse,
        StepKind::Capabilities => match decoder.capabilities(raw) {
            Ok(response)
                if response.request_id == step.request_id
                    && super::external_control_attempt::validated_external_provider_catalog(
                        &step.request_id,
                        &response,
                    )
                    .is_ok()
                    && required_capabilities(&response, step_spec(step.ordinal).business) =>
            {
                StepOutcome::ObservedSuccess
            }
            _ => StepOutcome::StoppedCapabilities,
        },
        _ => StepOutcome::StoppedLocalWire,
    }
}
// Lower-trust parsing of recorded request/response bytes only. A partial
// provider quality flag remains false; this never issues a live capability.
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProbeInstrument {
    exchange: crate::market_domain::Exchange,
    code: String,
    asset_class: crate::market_domain::AssetClass,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProbeHistoricalRequest {
    instrument: ProbeInstrument,
    interval: crate::market_domain::BarInterval,
    start: String,
    end: String,
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeHistoricalRow {
    instrument: ProbeInstrument,
    interval: crate::market_domain::BarInterval,
    bar_start: String,
    bar_end: String,
    open: crate::market_domain::Price,
    high: crate::market_domain::Price,
    low: crate::market_domain::Price,
    close: crate::market_domain::Price,
    volume: crate::market_domain::Quantity,
    amount: crate::market_domain::Money,
    adjustment: crate::market_domain::Adjustment,
    source_at: String,
    observed_at: String,
    provider: crate::market_domain::ProviderId,
    batch_id: String,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementRequest {
    start: String,
    end: String,
    limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeSourceEvidence {
    provider: crate::market_domain::ProviderId,
    source_at: String,
    observed_at: String,
    batch_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementRow {
    announcement_id: String,
    instrument: ProbeInstrument,
    instrument_name: Option<String>,
    category: Option<String>,
    title: String,
    published_at: String,
    canonical_url: String,
    pdf_url: Option<String>,
    evidence: ProbeSourceEvidence,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementEnvelope {
    request_id: String,
    request_payload_sha256: String,
    request: ProbeAnnouncementRequest,
    coverage_scope: String,
    pit_guarantee: bool,
    exchange_event_universe_complete: bool,
    result: ProbeAnnouncementResult,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementResult {
    batch: ProbeAnnouncementBatch,
    coverage: ProbeAnnouncementCoverage,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementBatch {
    records: BoundedVec<ProbeAnnouncementRow, 300>,
    provenance: ProbeProvenance,
    quality: ProbeQuality,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeProvenance {
    source: String,
    source_at: Option<String>,
    fetched_at: String,
    batch_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeQuality {
    complete: bool,
    issues: BoundedVec<String, 3>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementCoverage {
    source_total: u64,
    expected_request_pages: u64,
    pages_read: u32,
    inspected_raw_rows: u64,
    unique_rows: u64,
    returned_rows: u64,
    equivalent_duplicate_rows: u64,
    terminal_has_more: bool,
    source_exhausted: bool,
    caller_limit_truncated: bool,
    verified_empty: bool,
    pages: BoundedVec<ProbeAnnouncementPage, 10>,
}
// Field order is the original b7 page-evidence serialization order. Its digest
// is a claim about recorded page receipts, not independent upstream body proof.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProbeAnnouncementPage {
    requested_page: u32,
    source_total: u64,
    source_total_pages: u64,
    has_more: bool,
    row_count: u64,
    request_body_sha256: String,
    response_body_sha256: String,
    response_bytes: u64,
}
#[derive(PartialEq, Eq)]
struct ProbeAnnouncementStamp {
    pages: u32,
    total: u64,
    raw: u64,
    unique: u64,
    returned: u64,
    pages_sha256: String,
}
fn probe_text(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}
fn probe_date(value: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|date| date.to_string() == value)
}
fn probe_observed(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let (seconds, fraction) = value.split_once('.')?;
    if seconds.is_empty()
        || fraction.is_empty()
        || fraction.len() > 9
        || !seconds.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let nanos = fraction
        .parse::<u32>()
        .ok()?
        .checked_mul(10_u32.pow(9 - fraction.len() as u32))?;
    chrono::DateTime::from_timestamp(seconds.parse().ok()?, nanos)
}
fn probe_record(record: &pb::CanonicalPayload, schema: &str, version: u32) -> bool {
    record.schema == schema
        && record.schema_version == version
        && record.content_type == JSON_CONTENT_TYPE
}
fn probe_instrument(value: &ProbeInstrument) -> Option<crate::market_domain::InstrumentId> {
    use crate::market_domain::{AssetClass, Exchange, InstrumentId};
    if value.asset_class != AssetClass::Equity
        || !matches!(
            value.exchange,
            Exchange::Shanghai | Exchange::Shenzhen | Exchange::Beijing
        )
        || value.code.len() != 6
        || !value.code.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    InstrumentId::new(value.exchange, &value.code, value.asset_class)
        .ok()
        .filter(|instrument| instrument.code() == value.code)
}
fn probe_historical_v1(request: &pb::QueryRequest, response: &pb::QueryResponse) -> bool {
    use crate::market_domain::{Adjustment, Bar, BarInterval, Exchange, ProviderId};
    let Some(payload) = request.payload.as_ref() else {
        return false;
    };
    let Ok(requested) = serde_json::from_slice::<ProbeHistoricalRequest>(&payload.data) else {
        return false;
    };
    let Some(instrument) = probe_instrument(&requested.instrument) else {
        return false;
    };
    let (Some(start), Some(end)) = (probe_date(&requested.start), probe_date(&requested.end))
    else {
        return false;
    };
    let Some(source_ms) = response
        .source_at
        .strip_prefix("unix-ms:")
        .and_then(|v| v.parse::<i64>().ok())
    else {
        return false;
    };
    let Some(source_at) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(source_ms) else {
        return false;
    };
    let Ok(day_capacity) = usize::try_from(end.signed_duration_since(start).num_days() + 1) else {
        return false;
    };
    if !matches!(
        instrument.exchange(),
        Exchange::Shanghai | Exchange::Shenzhen
    ) || requested.interval != BarInterval::Day
        || start > end
        || requested.limit == 0
        || response.records.len() > requested.limit
        || !response.complete
            && (response.records.len() != requested.limit || requested.limit >= day_capacity)
        || source_ms <= 0
        || response.source_at != format!("unix-ms:{source_ms}")
    {
        return false;
    }
    let mut previous = None;
    let mut first = None;
    for record in &response.records {
        if !probe_record(record, "magic.market.bar", 1) {
            return false;
        }
        let Ok(row) = serde_json::from_slice::<ProbeHistoricalRow>(&record.data) else {
            return false;
        };
        let Some(actual_instrument) = probe_instrument(&row.instrument) else {
            return false;
        };
        let Some(date) = probe_date(&row.bar_start) else {
            return false;
        };
        if actual_instrument != instrument
            || row.interval != BarInterval::Day
            || row.bar_start != row.bar_end
            || row.bar_start != row.source_at
            || date < start
            || date > end
            || previous.is_some_and(|p| p >= date)
            || row.adjustment != Adjustment::Unadjusted
            || row.provider != ProviderId::Tonghuashun
            || row.batch_id != response.batch_id
            || row.observed_at != response.observed_at
        {
            return false;
        }
        if Bar::new(
            actual_instrument,
            row.interval,
            &row.bar_start,
            &row.bar_end,
            row.open,
            row.high,
            row.low,
            row.close,
            row.volume,
            Some(row.amount),
            row.adjustment,
            row.provider,
            &row.batch_id,
        )
        .is_err()
        {
            return false;
        }
        first.get_or_insert(date);
        previous = Some(date);
    }
    let shanghai = chrono::FixedOffset::east_opt(8 * 60 * 60).unwrap();
    (response.complete || first.is_some_and(|date| date > start))
        && previous.is_none_or(|date| date == source_at.with_timezone(&shanghai).date_naive())
}
fn probe_announcement_stamp(
    request: &ProbeAnnouncementRequest,
    response: &pb::QueryResponse,
) -> Option<ProbeAnnouncementStamp> {
    let prefix = format!(
        "cninfo:{}:market-announcements:{}:{}:",
        response.observed_at, request.start, request.end
    );
    let suffix = response.batch_id.strip_prefix(&prefix)?;
    let mut parts = suffix.split(':');
    let values = [
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    ];
    if parts.next().is_some() {
        return None;
    }
    fn number(value: &str, key: &str) -> Option<u64> {
        let value = value.strip_prefix(key)?;
        let number = value.parse::<u64>().ok()?;
        (number.to_string() == value).then_some(number)
    }
    let pages = u32::try_from(number(values[0], "pages=")?).ok()?;
    let total = number(values[1], "total=")?;
    let limit = number(values[2], "limit=")?;
    let raw = number(values[3], "raw=")?;
    let unique = number(values[4], "unique=")?;
    let returned = number(values[5], "returned=")?;
    let pages_sha256 = values[6].strip_prefix("pages-sha256=")?;
    let expected_request_pages = (total / 30 + u64::from(total % 30 != 0)).max(1);
    if !(1..=10).contains(&pages)
        || u64::from(pages) > expected_request_pages
        || limit != request.limit as u64
        || limit == 1 && pages != 1
        || raw > 300
        || raw > total
        || unique > raw
        || returned != unique.min(limit)
        || !valid_digest(pages_sha256)
        || raw != total.min(u64::from(pages) * 30)
        || raw < total && unique < limit
        || (total == 0) != (raw == 0)
        || (raw == 0) != (unique == 0)
        || raw == 0 && pages != 1
    {
        return None;
    }
    Some(ProbeAnnouncementStamp {
        pages,
        total,
        raw,
        unique,
        returned,
        pages_sha256: pages_sha256.into(),
    })
}
fn probe_announcement_rows(
    request: &ProbeAnnouncementRequest,
    response: &pb::QueryResponse,
    rows: &[ProbeAnnouncementRow],
) -> bool {
    let (Some(start), Some(end)) = (probe_date(&request.start), probe_date(&request.end)) else {
        return false;
    };
    if start > end || request.limit == 0 || request.limit > 300 || rows.len() > request.limit {
        return false;
    }
    let offset = chrono::FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let mut previous = None;
    let mut ids = BTreeSet::new();
    for row in rows {
        let Some(instrument) = probe_instrument(&row.instrument) else {
            return false;
        };
        let Ok(published) = chrono::DateTime::parse_from_rfc3339(&row.published_at) else {
            return false;
        };
        if published.offset() != &offset
            || published.format("%Y-%m-%dT%H:%M:%S%:z").to_string() != row.published_at
            || published.date_naive() < start
            || published.date_naive() > end
            || previous.is_some_and(|p| p < published)
            || !ids.insert(&row.announcement_id)
            || !probe_text(&row.announcement_id)
            || !probe_text(&row.title)
            || row.instrument_name.as_ref().is_some_and(|v| !probe_text(v))
            || row.category.as_ref().is_some_and(|v| !probe_text(v))
            || row.evidence.provider != crate::market_domain::ProviderId::Cninfo
            || row.evidence.source_at != row.published_at
            || row.evidence.observed_at != response.observed_at
            || row.evidence.batch_id != response.batch_id
        {
            return false;
        }
        let Ok(url) = url::Url::parse(&row.canonical_url) else {
            return false;
        };
        let pairs: Vec<_> = url.query_pairs().collect();
        let one = |key: &str, value: &str| {
            pairs.iter().filter(|(k, v)| k == key && v == value).count() == 1
        };
        if url.scheme() != "https"
            || url.host_str() != Some("www.cninfo.com.cn")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.path() != "/new/disclosure/detail"
            || pairs.len() != 4
            || !one("stockCode", instrument.code())
            || !one("announcementId", &row.announcement_id)
            || !one("announcementTime", &row.published_at[..10])
            || pairs
                .iter()
                .filter(|(k, v)| k == "orgId" && probe_text(v))
                .count()
                != 1
        {
            return false;
        }
        if let Some(pdf) = &row.pdf_url {
            let Ok(url) = url::Url::parse(pdf) else {
                return false;
            };
            if url.scheme() != "https"
                || url.host_str() != Some("static.cninfo.com.cn")
                || url.port().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
                || pdf
                    .strip_prefix("https://static.cninfo.com.cn/")
                    .is_none_or(|relative| {
                        relative.is_empty()
                            || relative.contains("..")
                            || relative.contains('\\')
                            || relative.contains(':')
                    })
            {
                return false;
            }
        }
        previous = Some(published);
    }
    response.source_at
        == rows
            .first()
            .map(|row| row.published_at.as_str())
            .unwrap_or("")
}
fn probe_page_request_sha(request: &ProbeAnnouncementRequest, page: u32) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in [
        ("stock", String::new()),
        ("tabName", "fulltext".into()),
        ("pageSize", "30".into()),
        ("pageNum", page.to_string()),
        ("column", "szse".into()),
        ("category", String::new()),
        ("plate", String::new()),
        ("seDate", format!("{}~{}", request.start, request.end)),
        ("searchkey", String::new()),
        ("secid", String::new()),
        ("sortName", String::new()),
        ("sortType", String::new()),
        ("isHLtitle", "false".into()),
    ] {
        serializer.append_pair(key, &value);
    }
    digest(serializer.finish().as_bytes())
}
fn probe_announcements(request: &pb::QueryRequest, response: &pb::QueryResponse) -> bool {
    let Some(payload) = request.payload.as_ref() else {
        return false;
    };
    let Ok(requested) = serde_json::from_slice::<ProbeAnnouncementRequest>(&payload.data) else {
        return false;
    };
    let Some(stamp) = probe_announcement_stamp(&requested, response) else {
        return false;
    };
    if payload.schema_version == 1 {
        if response.records.len() > requested.limit {
            return false;
        }
        let mut rows = Vec::with_capacity(response.records.len());
        for record in &response.records {
            if !probe_record(record, "magic.market.announcement", 1) {
                return false;
            }
            let Ok(row) = serde_json::from_slice::<ProbeAnnouncementRow>(&record.data) else {
                return false;
            };
            rows.push(row);
        }
        // v1 has no independent coverage document. These original batch-id
        // counters constrain a prefix observation; they do not prove totality.
        return rows.len() as u64 == stamp.returned
            && response.complete
                == (stamp.raw == stamp.total
                    && stamp.unique == stamp.raw
                    && stamp.returned == stamp.unique)
            && probe_announcement_rows(&requested, response, &rows);
    }
    if payload.schema_version != 2 || response.records.len() != 1 {
        return false;
    }
    let record = &response.records[0];
    if !probe_record(record, "magic.market.market_announcements.coverage", 2) {
        return false;
    }
    let Ok(envelope) = serde_json::from_slice::<ProbeAnnouncementEnvelope>(&record.data) else {
        return false;
    };
    if envelope.request_id != request.context.as_ref().unwrap().request_id
        || envelope.request_payload_sha256 != digest(&payload.data)
        || envelope.request != requested
        || envelope.coverage_scope != "CninfoNativeDateRangeQuery"
        || envelope.pit_guarantee
        || envelope.exchange_event_universe_complete
    {
        return false;
    }
    let batch = &envelope.result.batch;
    let coverage = &envelope.result.coverage;
    let exhausted = stamp.raw == stamp.total;
    let truncated = stamp.returned < stamp.unique;
    let duplicates = stamp.raw - stamp.unique;
    let expected_pages = (stamp.total / 30 + u64::from(stamp.total % 30 != 0)).max(1);
    if coverage.source_total != stamp.total
        || coverage.expected_request_pages != expected_pages
        || coverage.pages_read != stamp.pages
        || coverage.inspected_raw_rows != stamp.raw
        || coverage.unique_rows != stamp.unique
        || coverage.returned_rows != stamp.returned
        || coverage.equivalent_duplicate_rows != duplicates
        || coverage.source_exhausted != exhausted
        || coverage.caller_limit_truncated != truncated
        || coverage.verified_empty != (stamp.total == 0)
        || coverage.terminal_has_more != !exhausted
        || coverage.pages.0.len() != stamp.pages as usize
        || batch.records.0.len() as u64 != stamp.returned
        || batch.provenance.source != "cninfo-market"
        || batch.provenance.source_at.as_deref().unwrap_or("") != response.source_at
        || batch.provenance.fetched_at != response.observed_at
        || batch.provenance.batch_id != response.batch_id
    {
        return false;
    }
    let mut inspected = 0_u64;
    for (index, page) in coverage.pages.0.iter().enumerate() {
        let expected_rows = (stamp.total - inspected).min(30);
        if page.requested_page != index as u32 + 1
            || page.source_total != stamp.total
            || page.source_total_pages != stamp.total / 30
            || page.row_count != expected_rows
            || page.has_more != (inspected + expected_rows < stamp.total)
            || page.request_body_sha256 != probe_page_request_sha(&requested, page.requested_page)
            || !valid_digest(&page.response_body_sha256)
            || page.response_bytes == 0
            || page.response_bytes > 8 * 1024 * 1024
            || index > 0 && inspected >= stamp.total
        {
            return false;
        }
        inspected += page.row_count;
    }
    if inspected != stamp.raw
        || digest(&serde_json::to_vec(&coverage.pages).expect("fixed page serialization"))
            != stamp.pages_sha256
        || !exhausted && stamp.unique < requested.limit as u64
    {
        return false;
    }
    let mut issues = Vec::new();
    if !exhausted {
        issues.push(format!(
            "source pagination incomplete: inspected {} of {} declared rows",
            stamp.raw, stamp.total
        ));
    }
    if truncated {
        issues.push(format!(
            "caller limit truncates {} inspected unique records to {}",
            stamp.unique, stamp.returned
        ));
    }
    if duplicates > 0 {
        issues.push(format!("source identity overlap: {duplicates} equivalent duplicate rows cannot prove complete unique coverage"));
    }
    batch.quality.issues.0 == issues
        && batch.quality.complete == issues.is_empty()
        && response.complete == batch.quality.complete
        && probe_announcement_rows(&requested, response, &batch.records.0)
}
fn query_outcome(
    step: &PlannedStep,
    wire: &ExternalWireEvidenceV1,
    authority: &str,
) -> StepOutcome {
    if step.kind == StepKind::UnsupportedSchemaNegative {
        return StepOutcome::StoppedUnexpectedNegativeResponse;
    }
    let (op, method, provider, schema_version, _) =
        business_recipe(step_spec(step.ordinal).business.unwrap());
    let valid = (|| {
        wire.validate_descriptor(method, EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
            .ok()?;
        let raw = wire.payload()?;
        admit_external_payload(raw).ok()?;
        let response = ExternalDecoder::for_descriptor(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
            .ok()?
            .query(raw)
            .ok()?;
        let outer = crate::grpc_client::envelope::parse_external_native_query_response(
            &step.request_id,
            op,
            authority,
            response.clone(),
        )
        .ok()?;
        if outer.admission != crate::grpc_client::envelope::QueryAdmission::Admitted
            || outer.selected_provider != provider
            || !outer.diagnostic_blocker.is_empty()
            || !probe_text(&outer.batch_id)
            || probe_observed(&outer.observed_at).is_none()
        {
            return None;
        }
        let original_request = hex::decode(&step.request_wire_hex).ok()?;
        let request = pb::QueryRequest::decode(original_request.as_slice()).ok()?;
        let typed = match (op, schema_version) {
            (pb::Operation::HistoricalBars, 1) => probe_historical_v1(&request, &response),
            (pb::Operation::HistoricalBars, 2) => {
                crate::data_gateway::parse_recorded_historical_coverage_v2(&original_request, raw)
                    .is_ok_and(|observation| {
                        usize::try_from(
                            observation
                                .end()
                                .signed_duration_since(observation.start())
                                .num_days()
                                + 1,
                        )
                        .is_ok_and(|days| {
                            let rows = observation.native_row_dates();
                            let source = observation.validated_source_rows_claim();
                            source <= days
                                && match rows.first() {
                                    None => source == 0,
                                    Some(first) => usize::try_from(
                                        first.signed_duration_since(observation.start()).num_days(),
                                    )
                                    .is_ok_and(
                                        |earlier_days| {
                                            source
                                                .checked_sub(rows.len())
                                                .is_some_and(|omitted| omitted <= earlier_days)
                                        },
                                    ),
                                }
                        })
                    })
            }
            (pb::Operation::MarketAnnouncements, _) => probe_announcements(&request, &response),
            _ => false,
        };
        typed.then_some(())
    })();
    if valid.is_some() {
        StepOutcome::ObservedSuccess
    } else {
        StepOutcome::StoppedEnvelope
    }
}
fn status_context<'a>(step: &'a PlannedStep) -> StatusErrorContext<'a> {
    if matches!(
        step.kind,
        StepKind::Business | StepKind::UnsupportedSchemaNegative
    ) {
        let (op, _, _, _, _) = business_recipe(step_spec(step.ordinal).business.unwrap());
        let method = ExternalMethod::try_from_operation(op).expect("fixed candidate operation");
        StatusErrorContext::data(MethodIdentity::External(method), &step.request_id)
    } else {
        StatusErrorContext::control(ContractProfile::ExternalV1, &step.request_id)
    }
}
fn status_outcome(
    step: &PlannedStep,
    code: i32,
    has_carrier: bool,
    correlation: Option<&str>,
    fixed_schema_rejection: bool,
) -> StepOutcome {
    let expected = crate::grpc_client::errors::request_id_correlation(&step.request_id);
    let carrier_matches = !has_carrier || correlation == expected.as_deref();
    match step.kind {
        StepKind::UnauthenticatedHealth if code == 16 && carrier_matches => {
            StepOutcome::ExpectedUnauthenticatedStatus
        }
        StepKind::UnsupportedSchemaNegative
            if code == 3 && carrier_matches && fixed_schema_rejection =>
        {
            StepOutcome::ExpectedUnsupportedSchemaStatus
        }
        _ => StepOutcome::StoppedStatus,
    }
}

// Source-reviewed b7 InvalidRequest emitted before provider I/O for schema !=1|2.
// This accepts no free-form message, inferred reason, provider failure or trace.
fn fixed_schema_rejection(step: &PlannedStep, status: &tonic::Status) -> bool {
    if step.kind != StepKind::UnsupportedSchemaNegative
        || status.code() != tonic::Code::InvalidArgument
    {
        return false;
    }
    let standard = (!status.details().is_empty()).then_some(status.details());
    let trailer = match status.metadata().get_bin("magic-error-detail-bin") {
        Some(value) => match value.to_bytes() {
            Ok(bytes) => Some(bytes),
            Err(_) => return false,
        },
        None => None,
    };
    let bytes = match (standard, trailer.as_deref()) {
        (Some(a), Some(b)) if a == b => a,
        (Some(_), Some(_)) => return false,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return false,
    };
    let Ok(decoder) = ExternalDecoder::for_descriptor(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256) else {
        return false;
    };
    let Some(detail) = decoder.error_detail(bytes) else {
        return false;
    };
    detail.encode_to_vec().as_slice() == bytes
        && detail.request_id == step.request_id
        && detail.operation == pb::Operation::MarketAnnouncements as i32
        && detail.reason_code == "invalid_request"
        && !detail.retryable
        && detail.admission == pb::AdmissionState::Unadmitted as i32
        && detail.provider.is_empty()
        && detail.evidence_code.is_empty()
        && detail.evidence_field.is_empty()
        && detail.record_index == 0
        && !detail.has_record_index
        && detail.provider_attempts.is_empty()
}

fn capture_status(
    builder: &mut RawBuilder,
    step: &PlannedStep,
    status: tonic::Status,
    wire: Option<ExternalWireEvidenceV1>,
) -> Result<(Material, StepOutcome), CandidateProbeError> {
    let wire = wire.map(|w| builder.wire(w)).transpose()?;
    let (code, details, raw_trailer) = super::unary_attempt::capture_status_material(&status);
    let message = builder.add(status.message().as_bytes())?;
    let details_part = builder.add(&details)?;
    let has_carrier = !details.is_empty()
        || !matches!(
            raw_trailer,
            super::unary_attempt::UnaryTrailerMaterial::Absent
        );
    let trailer = match raw_trailer {
        super::unary_attempt::UnaryTrailerMaterial::Absent => Trailer::Absent,
        super::unary_attempt::UnaryTrailerMaterial::Bytes(bytes) => Trailer::Bytes {
            part: builder.add(&bytes)?,
        },
        super::unary_attempt::UnaryTrailerMaterial::Malformed => Trailer::Malformed,
    };
    let fixed_schema_rejection = fixed_schema_rejection(step, &status);
    let error = GrpcError::from_status_with_decoder(
        status,
        status_context(step),
        ExternalDecoder::for_descriptor(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
            .map_err(|_| CandidateProbeError::CompiledInputs)?,
    );
    let correlation = error.details().request_id.clone();
    let outcome = status_outcome(
        step,
        code,
        has_carrier,
        correlation.as_deref(),
        fixed_schema_rejection,
    );
    Ok((
        Material::TonicStatus {
            wire,
            code,
            message,
            details: details_part,
            trailer,
            error_class: error_class(&error),
            matched_request_id_correlation: correlation,
        },
        outcome,
    ))
}

impl CandidateProbePlan {
    async fn execute_prepared(
        &self,
        prepared: PreparedExternalEndpoint,
    ) -> Result<CandidateProbeRunReceipt, CandidateProbeError> {
        validate_plan(&self.dto)?;
        let generation = prepared.plan_connection_generation();
        let mut dto = ReceiptDto {
            version: 1,
            purpose: "CandidateAcceptanceObservationOnly".into(),
            plan: self.dto.clone(),
            actual_endpoint: prepared.endpoint_uri.clone(),
            actual_acquisition_authority: prepared.acquisition_authority.clone(),
            connection_identity: generation.identity(),
            connect_invocations: 1,
            connect_error_class: None,
            steps: BoundedVec(Vec::with_capacity(MAX_RPC)),
            parts: BoundedVec(Vec::new()),
            outcome: RunOutcome::StoppedRecordedObservation,
        };
        let mut client = match prepared.connect_generation(generation).await {
            Ok(client) => client,
            Err(error) => {
                dto.connect_error_class = Some(error_class(&error));
                return Ok(CandidateProbeRunReceipt {
                    dto,
                    arena: Vec::new(),
                });
            }
        };
        let mut raw = RawBuilder::new();
        for step in &self.dto.steps.0 {
            let observation = execute_step(&mut client, step, &mut raw).await?;
            let continues = observation.outcome.continues();
            dto.steps.0.push(observation);
            if !continues {
                break;
            } // Never HealthAfter/retry after any failure.
        }
        if dto.steps.0.len() == MAX_RPC && dto.steps.0.iter().all(|s| s.outcome.continues()) {
            dto.outcome = RunOutcome::CompletedRecordedObservation;
        }
        dto.parts = BoundedVec(raw.parts);
        Ok(CandidateProbeRunReceipt {
            dto,
            arena: raw.arena,
        })
    }
}

async fn execute_step(
    client: &mut GrpcMarketClient,
    step: &PlannedStep,
    raw: &mut RawBuilder,
) -> Result<ObservedStep, CandidateProbeError> {
    let result = execute_step_inner(client, step, raw).await;
    if !result.as_ref().is_ok_and(|step| step.outcome.continues()) {
        if let Some(generation) = &client.connection_generation {
            generation.revoke();
        }
    }
    result
}

async fn execute_step_inner(
    client: &mut GrpcMarketClient,
    step: &PlannedStep,
    raw: &mut RawBuilder,
) -> Result<ObservedStep, CandidateProbeError> {
    let gate = || {
        (
            Material::LocalGateFailure {
                error_class: ErrorClass::FailedPrecondition,
            },
            StepOutcome::StoppedLocalGate,
        )
    };
    let wire = hex::decode(&step.request_wire_hex).map_err(|_| CandidateProbeError::PlanInput)?;
    let (material, outcome) = match step.kind {
        StepKind::Health | StepKind::UnauthenticatedHealth => {
            let generation = client
                .connection_generation
                .as_ref()
                .ok_or(CandidateProbeError::ReceiptInput)?;
            if generation.begin_health_observation().is_err() {
                gate()
            } else {
                let request = pb::HealthRequest::decode(wire.as_slice())
                    .map_err(|_| CandidateProbeError::PlanInput)?;
                let mut request = tonic::Request::new(request);
                if step.kind == StepKind::Health
                    && client.attach_request_auth(&mut request).is_err()
                {
                    gate()
                } else {
                    match client.execute_external_health(request).await {
                        ExternalSystemCall::Response(response, bytes) => {
                            let outcome = control_outcome(step, &bytes);
                            if step.kind == StepKind::Health {
                                // Original observe validates request ID/full identity/revocation and
                                // leaves None on every rejected response, even when raw bytes exist.
                                let actual =
                                    client.observe_external_health(&step.request_id, &response);
                                if actual.is_ok() != (outcome == StepOutcome::ObservedSuccess) {
                                    return Err(CandidateProbeError::ReceiptInput);
                                }
                            }
                            (
                                Material::ControlResponse {
                                    part: raw.add(&bytes)?,
                                },
                                outcome,
                            )
                        }
                        ExternalSystemCall::UnaryStatus(status) => {
                            capture_status(raw, step, status, None)?
                        }
                    }
                }
            }
        }
        StepKind::Capabilities => {
            if client.require_external_qualification().is_err() {
                gate()
            } else {
                let request = pb::CapabilitiesRequest::decode(wire.as_slice())
                    .map_err(|_| CandidateProbeError::PlanInput)?;
                let mut request = tonic::Request::new(request);
                if client.attach_request_auth(&mut request).is_err() {
                    gate()
                } else {
                    match client.execute_external_capabilities(request).await {
                        ExternalSystemCall::Response(response, bytes) => {
                            let outcome = control_outcome(step, &bytes);
                            if outcome == StepOutcome::ObservedSuccess
                                && client
                                    .accept_external_capabilities(&step.request_id, &response)
                                    .is_err()
                            {
                                return Err(CandidateProbeError::ReceiptInput);
                            }
                            (
                                Material::ControlResponse {
                                    part: raw.add(&bytes)?,
                                },
                                outcome,
                            )
                        }
                        ExternalSystemCall::UnaryStatus(status) => {
                            capture_status(raw, step, status, None)?
                        }
                    }
                }
            }
        }
        StepKind::Business | StepKind::UnsupportedSchemaNegative => {
            let unsupported_negative = step.kind == StepKind::UnsupportedSchemaNegative;
            let opening = if unsupported_negative {
                // Fixed schema3 negative only: keep Health absent and enforce original
                // revocation. This never authorizes a supported business operation.
                client
                    .connection_generation
                    .as_ref()
                    .ok_or(CandidateProbeError::ReceiptInput)?
                    .begin_health_observation()
            } else {
                client.require_external_qualification()
            };
            if opening.is_err() {
                gate()
            } else {
                let (_, method, _, _, _) =
                    business_recipe(step_spec(step.ordinal).business.unwrap());
                let request = pb::QueryRequest::decode(wire.as_slice())
                    .map_err(|_| CandidateProbeError::PlanInput)?;
                let mut request = tonic::Request::new(request);
                if client.attach_request_auth(&mut request).is_err() {
                    gate()
                } else {
                    let authority = client
                        .acquisition_authority
                        .as_deref()
                        .ok_or(CandidateProbeError::ReceiptInput)?
                        .to_owned();
                    let call = match &mut client.data {
                        DataTransport::External(data) => {
                            data.call_with_descriptor(
                                method,
                                request,
                                EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
                            )
                            .await
                        }
                        DataTransport::Local(_) => return Err(CandidateProbeError::ReceiptInput),
                    };
                    match call {
                        ExternalQueryCall::Response { evidence, .. } => {
                            let outcome = query_outcome(step, &evidence, &authority);
                            (
                                Material::QueryResponse {
                                    wire: raw.wire(evidence)?,
                                },
                                outcome,
                            )
                        }
                        ExternalQueryCall::UnaryStatus { status, evidence } => {
                            capture_status(raw, step, status, Some(evidence))?
                        }
                        ExternalQueryCall::LocalWireFailure { error, evidence } => (
                            Material::LocalWireFailure {
                                wire: raw.wire(evidence)?,
                                error_class: error_class(&error),
                            },
                            StepOutcome::StoppedLocalWire,
                        ),
                    }
                }
            }
        }
    };
    Ok(ObservedStep {
        ordinal: step.ordinal,
        request_id_correlation: crate::grpc_client::errors::request_id_correlation(
            &step.request_id,
        )
        .ok_or(CandidateProbeError::PlanInput)?,
        material,
        outcome,
    })
}

struct RawRead<'a> {
    arena: &'a [u8],
    ranges: Vec<std::ops::Range<usize>>,
    next: usize,
}
impl<'a> RawRead<'a> {
    fn new(arena: &'a [u8], parts: &[PartMeta]) -> Result<Self, CandidateProbeError> {
        if arena.len() > MAX_RAW_BYTES || parts.len() > MAX_PARTS {
            return Err(CandidateProbeError::MaterialBound);
        }
        let mut ranges = Vec::with_capacity(parts.len());
        let mut offset: usize = 0;
        for part in parts {
            if part.byte_length > EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES {
                return Err(CandidateProbeError::MaterialBound);
            }
            let end = offset
                .checked_add(part.byte_length)
                .ok_or(CandidateProbeError::ReceiptInput)?;
            let raw = arena
                .get(offset..end)
                .ok_or(CandidateProbeError::ReceiptInput)?;
            if digest(raw) != part.sha256 {
                return Err(CandidateProbeError::ReceiptInput);
            }
            ranges.push(offset..end);
            offset = end;
        }
        if offset != arena.len() {
            return Err(CandidateProbeError::ReceiptInput);
        }
        Ok(Self {
            arena,
            ranges,
            next: 0,
        })
    }
    fn part(&mut self, index: usize) -> Result<&'a [u8], CandidateProbeError> {
        // One canonical owner per byte part; no aliases, gaps, or unreferenced material.
        if index != self.next {
            return Err(CandidateProbeError::ReceiptInput);
        }
        let range = self
            .ranges
            .get(index)
            .ok_or(CandidateProbeError::ReceiptInput)?
            .clone();
        self.next += 1;
        Ok(&self.arena[range])
    }
    fn wire(
        &mut self,
        meta: &WireMeta,
        method: ExternalQueryMethod,
    ) -> Result<ExternalWireEvidenceV1, CandidateProbeError> {
        let evidence = match &meta.body {
            WireBody::Payload {
                part,
                payload_sha256,
                decode_limit_bytes,
            } => ExternalWireMaterialV1::Payload {
                protobuf_payload: self.part(*part)?.to_vec(),
                payload_sha256: payload_sha256.clone(),
                decode_limit_bytes: *decode_limit_bytes,
            },
            WireBody::Missing {
                framed_body_limit_bytes,
            } => ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: *framed_body_limit_bytes,
            },
            WireBody::Overflow {
                framed_body_limit_bytes,
                observed_framed_body_bytes_at_least,
            } => ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes: *framed_body_limit_bytes,
                observed_framed_body_bytes_at_least: *observed_framed_body_bytes_at_least,
            },
            WireBody::InvalidFrame {
                part,
                body_sha256,
                failure,
                framed_body_limit_bytes,
            } => ExternalWireMaterialV1::InvalidFrame {
                grpc_body_bytes: self.part(*part)?.to_vec(),
                body_sha256: body_sha256.clone(),
                failure: *failure,
                framed_body_limit_bytes: *framed_body_limit_bytes,
            },
        };
        let wire = ExternalWireEvidenceV1 {
            material: meta.material.clone(),
            profile: meta.profile.clone(),
            method: meta.method,
            client_descriptor_sha256: meta.client_descriptor_sha256.clone(),
            evidence,
        };
        wire.validate_descriptor(method, EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
            .map_err(|_| CandidateProbeError::ReceiptInput)?;
        Ok(wire)
    }
}

/// Verifies a separately sealed recorded artifact against this compiled profile.
/// Even a coherent caller-supplied artifact never creates a live session or source.
pub fn read_candidate_b7_receipt(
    bytes: &[u8],
    externally_sealed_artifact_sha256: &str,
) -> Result<RecordedCandidateProbeEvidence, CandidateProbeError> {
    read_receipt_on_target(
        bytes,
        externally_sealed_artifact_sha256,
        CANDIDATE_ENDPOINT,
        "grpc-mtls:magic-market.local",
    )
}

fn read_receipt_on_target(
    bytes: &[u8],
    expected_sha: &str,
    target: &str,
    authority: &str,
) -> Result<RecordedCandidateProbeEvidence, CandidateProbeError> {
    if bytes.len() > MAX_CANDIDATE_RECEIPT_BYTES || bytes.len() < MAGIC.len() + 4 {
        return Err(CandidateProbeError::MaterialBound);
    }
    if !valid_digest(expected_sha)
        || digest(bytes) != expected_sha
        || &bytes[..MAGIC.len()] != MAGIC
    {
        return Err(CandidateProbeError::ReceiptInput);
    }
    let prefix = MAGIC.len();
    let length = u32::from_be_bytes(
        bytes[prefix..prefix + 4]
            .try_into()
            .map_err(|_| CandidateProbeError::ReceiptInput)?,
    ) as usize;
    if length > MAX_MANIFEST_BYTES {
        return Err(CandidateProbeError::MaterialBound);
    }
    let end = (prefix + 4)
        .checked_add(length)
        .ok_or(CandidateProbeError::ReceiptInput)?;
    let manifest = bytes
        .get(prefix + 4..end)
        .ok_or(CandidateProbeError::ReceiptInput)?;
    let dto: ReceiptDto =
        serde_json::from_slice(manifest).map_err(|_| CandidateProbeError::ReceiptInput)?;
    if serde_json::to_vec(&dto).map_err(|_| CandidateProbeError::ReceiptInput)? != manifest {
        return Err(CandidateProbeError::ReceiptInput);
    }
    validate_plan(&dto.plan)?;
    if dto.version != 1
        || dto.purpose != "CandidateAcceptanceObservationOnly"
        || dto.actual_endpoint != target
        || dto.actual_acquisition_authority != authority
        || dto.connect_invocations != 1
        || dto.connection_identity.version != 1
        || dto.connection_identity.epoch.is_empty()
        || dto.connection_identity.epoch.len() > 512
        || !dto
            .connection_identity
            .epoch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || dto.connection_identity.policy_sha256 != dto.plan.compiled_inputs.policy_sha256()
        || dto.connection_identity.descriptor_sha256 != EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
    {
        return Err(CandidateProbeError::ReceiptInput);
    }
    let mut raw = RawRead::new(&bytes[end..], &dto.parts.0)?;
    if dto.connect_error_class.is_some() {
        if !dto.steps.0.is_empty()
            || !dto.parts.0.is_empty()
            || dto.outcome != RunOutcome::StoppedRecordedObservation
        {
            return Err(CandidateProbeError::ReceiptInput);
        }
    } else {
        if dto.steps.0.is_empty() {
            return Err(CandidateProbeError::ReceiptInput);
        }
        let mut qualified = false;
        let mut capability_business: Option<Option<usize>> = None;
        for (ordinal, observed) in dto.steps.0.iter().enumerate() {
            let step = &dto.plan.steps.0[ordinal];
            let expected_correlation =
                crate::grpc_client::errors::request_id_correlation(&step.request_id)
                    .ok_or(CandidateProbeError::ReceiptInput)?;
            if observed.ordinal != ordinal
                || observed.request_id_correlation != expected_correlation
                || (ordinal + 1 < dto.steps.0.len() && !observed.outcome.continues())
            {
                return Err(CandidateProbeError::ReceiptInput);
            }
            if matches!(
                step.kind,
                StepKind::Health | StepKind::UnauthenticatedHealth
            ) {
                qualified = false;
            }
            let opening = match step.kind {
                StepKind::Capabilities => qualified,
                StepKind::Business => {
                    qualified && capability_business == Some(step_spec(ordinal).business)
                }
                StepKind::UnsupportedSchemaNegative => {
                    ordinal == 3
                        && !qualified
                        && dto.steps.0[2].outcome == StepOutcome::ExpectedUnauthenticatedStatus
                }
                _ => true,
            };
            if !opening && !matches!(observed.material, Material::LocalGateFailure { .. }) {
                return Err(CandidateProbeError::ReceiptInput);
            }
            let actual = read_step_material(step, &observed.material, &mut raw, authority)?;
            if actual != observed.outcome {
                return Err(CandidateProbeError::ReceiptInput);
            }
            if actual == StepOutcome::ObservedSuccess {
                if step.kind == StepKind::Health {
                    qualified = true;
                }
                if step.kind == StepKind::Capabilities {
                    capability_business = Some(step_spec(ordinal).business);
                }
            }
        }
        let completed =
            dto.steps.0.len() == MAX_RPC && dto.steps.0.iter().all(|s| s.outcome.continues());
        if (dto.outcome == RunOutcome::CompletedRecordedObservation) != completed
            || (!completed && dto.steps.0.last().is_some_and(|s| s.outcome.continues()))
        {
            return Err(CandidateProbeError::ReceiptInput);
        }
    }
    if raw.next != raw.ranges.len() {
        return Err(CandidateProbeError::ReceiptInput);
    }
    Ok(RecordedCandidateProbeEvidence {
        outcome: dto.outcome,
        rpc_count: rpc_count(&dto),
        artifact_sha256: expected_sha.into(),
    })
}

fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn read_step_material(
    step: &PlannedStep,
    material: &Material,
    raw: &mut RawRead<'_>,
    authority: &str,
) -> Result<StepOutcome, CandidateProbeError> {
    match material {
        Material::ControlResponse { part }
            if matches!(
                step.kind,
                StepKind::Health | StepKind::UnauthenticatedHealth | StepKind::Capabilities
            ) =>
        {
            Ok(control_outcome(step, raw.part(*part)?))
        }
        Material::QueryResponse { wire }
            if matches!(
                step.kind,
                StepKind::Business | StepKind::UnsupportedSchemaNegative
            ) =>
        {
            let (_, method, _, _, _) = business_recipe(step_spec(step.ordinal).business.unwrap());
            let wire = raw.wire(wire, method)?;
            if wire.payload().is_none() {
                return Err(CandidateProbeError::ReceiptInput);
            }
            Ok(query_outcome(step, &wire, authority))
        }
        Material::TonicStatus {
            wire,
            code,
            message,
            details,
            trailer,
            error_class: class,
            matched_request_id_correlation,
        } => {
            let query = matches!(
                step.kind,
                StepKind::Business | StepKind::UnsupportedSchemaNegative
            );
            if query != wire.is_some() || !(1..=16).contains(code) {
                return Err(CandidateProbeError::ReceiptInput);
            }
            if let Some(wire) = wire {
                let (_, method, _, _, _) =
                    business_recipe(step_spec(step.ordinal).business.unwrap());
                raw.wire(wire, method)?;
            }
            let message = std::str::from_utf8(raw.part(*message)?)
                .map_err(|_| CandidateProbeError::ReceiptInput)?;
            let details = raw.part(*details)?;
            let mut status = tonic::Status::with_details(
                tonic::Code::from_i32(*code),
                message,
                prost::bytes::Bytes::copy_from_slice(details),
            );
            let has_carrier = !details.is_empty() || !matches!(trailer, Trailer::Absent);
            if let Trailer::Bytes { part } = trailer {
                status.metadata_mut().insert_bin(
                    "magic-error-detail-bin",
                    tonic::metadata::MetadataValue::from_bytes(raw.part(*part)?),
                );
            }
            let fixed_schema_rejection =
                !matches!(trailer, Trailer::Malformed) && fixed_schema_rejection(step, &status);
            let error = GrpcError::from_status_with_decoder(
                status,
                status_context(step),
                ExternalDecoder::for_descriptor(EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256)
                    .map_err(|_| CandidateProbeError::CompiledInputs)?,
            );
            let correlation = if matches!(trailer, Trailer::Malformed) {
                None
            } else {
                error.details().request_id.clone()
            };
            if *class != error_class(&error) || *matched_request_id_correlation != correlation {
                return Err(CandidateProbeError::ReceiptInput);
            }
            Ok(status_outcome(
                step,
                *code,
                has_carrier,
                correlation.as_deref(),
                fixed_schema_rejection,
            ))
        }
        Material::LocalWireFailure { wire, .. }
            if matches!(
                step.kind,
                StepKind::Business | StepKind::UnsupportedSchemaNegative
            ) =>
        {
            let (_, method, _, _, _) = business_recipe(step_spec(step.ordinal).business.unwrap());
            raw.wire(wire, method)?;
            Ok(StepOutcome::StoppedLocalWire)
        }
        Material::LocalGateFailure { .. } => Ok(StepOutcome::StoppedLocalGate),
        _ => Err(CandidateProbeError::ReceiptInput),
    }
}

#[cfg(test)]
#[path = "candidate_probe_tests.rs"]
mod tests;
