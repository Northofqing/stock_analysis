//! Strict, side-effect-free decoding for unverified operator requests.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;

use serde::de::{value::MapAccessDeserializer, MapAccess, Visitor};
use serde::Deserialize;

use crate::monitor::push_job::{
    canonical_preimage, namespace_value, raw_digest, CanonicalValue, Namespace, ProtectedRef,
    ReasonCode, RunId, Sha256Digest, UnitId, UtcMicros,
};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_EVIDENCE_REFS: usize = 64;
const REQUEST_DOMAIN: &str = "PushOperatorRequest/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum OperatorRequestError {
    #[error("operator request exceeds the input limit")]
    InputTooLarge,
    #[error("invalid operator request wire structure")]
    WireStructure,
    #[error("invalid operator request field: {0}")]
    InvalidField(&'static str),
    #[error("operator request has too many evidence references")]
    TooManyEvidenceRefs,
    #[error("operator command does not allow this target kind")]
    TargetCommandMismatch,
    #[error("operator request contains duplicate evidence")]
    DuplicateEvidence,
    #[error("operator command identity does not match its request")]
    CommandIdMismatch,
}

pub type Result<T> = std::result::Result<T, OperatorRequestError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorCommand {
    Inspect,
    Reconcile,
    ResolveUncertain,
    Promote,
    Rollback,
}

impl OperatorCommand {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Reconcile => "reconcile",
            Self::ResolveUncertain => "resolve-uncertain",
            Self::Promote => "promote",
            Self::Rollback => "rollback",
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum OperatorTarget {
    Unit {
        id: UnitId,
        namespace: Namespace,
    },
    Intent {
        id: Sha256Digest,
        namespace: Namespace,
    },
    Decision {
        id: Sha256Digest,
        namespace: Namespace,
    },
}

impl fmt::Debug for OperatorTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let namespace_kind = match self.namespace() {
            Namespace::Production => "Production",
            Namespace::Test { .. } => "Test",
        };
        formatter
            .debug_struct("OperatorTarget")
            .field("kind", &self.kind())
            .field("namespace_kind", &namespace_kind)
            .finish()
    }
}

impl OperatorTarget {
    pub fn namespace(&self) -> &Namespace {
        match self {
            Self::Unit { namespace, .. }
            | Self::Intent { namespace, .. }
            | Self::Decision { namespace, .. } => namespace,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Unit { .. } => "unit",
            Self::Intent { .. } => "intent",
            Self::Decision { .. } => "decision",
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Unit { id, .. } => id.as_str(),
            Self::Intent { id, .. } | Self::Decision { id, .. } => id.as_str(),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct OperatorEvidenceRef {
    kind: String,
    version: String,
    protected_uri: ProtectedRef,
    sha256: Sha256Digest,
}

impl OperatorEvidenceRef {
    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn protected_uri(&self) -> &ProtectedRef {
        &self.protected_uri
    }

    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
}

impl fmt::Debug for OperatorEvidenceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperatorEvidenceRef")
            .field("sha256", &self.sha256)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct UnverifiedOperatorRequest {
    command: OperatorCommand,
    target: OperatorTarget,
    expected_version: u64,
    expected_generation: u64,
    dry_run: bool,
    claimed_operator_ref: String,
    reason: ReasonCode,
    evidence_refs: Vec<OperatorEvidenceRef>,
    requested_at: UtcMicros,
    canonical_bytes: Vec<u8>,
    request_digest: Sha256Digest,
}

impl UnverifiedOperatorRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(OperatorRequestError::InputTooLarge);
        }

        let MapOnly(wire): MapOnly<WireRequest> =
            serde_json::from_slice(bytes).map_err(|_| OperatorRequestError::WireStructure)?;
        if wire.evidence_refs.len() > MAX_EVIDENCE_REFS {
            return Err(OperatorRequestError::TooManyEvidenceRefs);
        }

        let command = parse_command(&wire.command)?;
        let target = parse_target(wire.target)?;
        if !command_allows_target(command, &target) {
            return Err(OperatorRequestError::TargetCommandMismatch);
        }

        let claimed_operator_ref = validate_text(
            "authenticated_operator_ref",
            wire.authenticated_operator_ref,
        )?;
        let reason = ReasonCode::try_from(wire.reason.as_str())
            .map_err(|_| OperatorRequestError::InvalidField("reason"))?;
        let requested_at = UtcMicros::try_new(wire.requested_at)
            .map_err(|_| OperatorRequestError::InvalidField("requested_at"))?;
        let evidence_refs = parse_evidence(wire.evidence_refs)?;

        let canonical_bytes = request_canonical_bytes(
            command,
            &target,
            wire.expected_version,
            wire.expected_generation,
            wire.dry_run,
            &claimed_operator_ref,
            reason,
            &evidence_refs,
            requested_at,
        );
        let request_digest = raw_digest(&canonical_bytes);
        let claimed_command_id = Sha256Digest::parse("command_id", &wire.command_id)
            .map_err(|_| OperatorRequestError::InvalidField("command_id"))?;
        if claimed_command_id != request_digest {
            return Err(OperatorRequestError::CommandIdMismatch);
        }

        Ok(Self {
            command,
            target,
            expected_version: wire.expected_version,
            expected_generation: wire.expected_generation,
            dry_run: wire.dry_run,
            claimed_operator_ref,
            reason,
            evidence_refs,
            requested_at,
            canonical_bytes,
            request_digest,
        })
    }

    pub fn command(&self) -> OperatorCommand {
        self.command
    }

    pub fn target(&self) -> &OperatorTarget {
        &self.target
    }

    pub fn expected_version(&self) -> u64 {
        self.expected_version
    }

    pub fn expected_generation(&self) -> u64 {
        self.expected_generation
    }

    pub fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// Returns a claimed reference only; successful parsing grants no authority.
    pub fn claimed_operator_ref(&self) -> &str {
        &self.claimed_operator_ref
    }

    pub fn reason(&self) -> ReasonCode {
        self.reason
    }

    pub fn evidence_refs(&self) -> &[OperatorEvidenceRef] {
        &self.evidence_refs
    }

    pub fn requested_at(&self) -> UtcMicros {
        self.requested_at
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub fn request_digest(&self) -> &Sha256Digest {
        &self.request_digest
    }
}

impl fmt::Debug for UnverifiedOperatorRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnverifiedOperatorRequest")
            .field("command", &self.command)
            .field("target_kind", &self.target.kind())
            .field("evidence_count", &self.evidence_refs.len())
            .field("dry_run", &self.dry_run)
            .field("request_digest", &self.request_digest)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    command_id: String,
    command: String,
    target: MapOnly<WireTarget>,
    expected_version: u64,
    expected_generation: u64,
    dry_run: bool,
    authenticated_operator_ref: String,
    reason: String,
    evidence_refs: Vec<MapOnly<WireEvidenceRef>>,
    requested_at: i64,
}

struct MapOnly<T>(T);

impl<'de, T> Deserialize<'de> for MapOnly<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct MapOnlyVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for MapOnlyVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = MapOnly<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A>(self, map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                T::deserialize(MapAccessDeserializer::new(map)).map(MapOnly)
            }
        }

        deserializer.deserialize_map(MapOnlyVisitor(PhantomData))
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum WireTarget {
    #[serde(rename = "unit")]
    Unit {
        id: String,
        namespace: MapOnly<WireNamespace>,
    },
    #[serde(rename = "intent")]
    Intent {
        id: String,
        namespace: MapOnly<WireNamespace>,
    },
    #[serde(rename = "decision")]
    Decision {
        id: String,
        namespace: MapOnly<WireNamespace>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum WireNamespace {
    Production { run_id: () },
    Test { run_id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvidenceRef {
    kind: String,
    version: String,
    protected_uri: String,
    sha256: String,
}

fn parse_target(MapOnly(target): MapOnly<WireTarget>) -> Result<OperatorTarget> {
    match target {
        WireTarget::Unit { id, namespace } => Ok(OperatorTarget::Unit {
            id: UnitId::try_new(id).map_err(|_| OperatorRequestError::InvalidField("target.id"))?,
            namespace: parse_namespace(namespace)?,
        }),
        WireTarget::Intent { id, namespace } => Ok(OperatorTarget::Intent {
            id: parse_digest("target.id", &id)?,
            namespace: parse_namespace(namespace)?,
        }),
        WireTarget::Decision { id, namespace } => Ok(OperatorTarget::Decision {
            id: parse_digest("target.id", &id)?,
            namespace: parse_namespace(namespace)?,
        }),
    }
}

fn parse_namespace(MapOnly(namespace): MapOnly<WireNamespace>) -> Result<Namespace> {
    match namespace {
        WireNamespace::Production { run_id: () } => Ok(Namespace::Production),
        WireNamespace::Test { run_id } => RunId::try_new(run_id)
            .map(Namespace::test)
            .map_err(|_| OperatorRequestError::InvalidField("target.namespace.run_id")),
    }
}

fn parse_evidence(entries: Vec<MapOnly<WireEvidenceRef>>) -> Result<Vec<OperatorEvidenceRef>> {
    let mut evidence = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for MapOnly(entry) in entries {
        let parsed = OperatorEvidenceRef {
            kind: validate_text("evidence_refs.kind", entry.kind)?,
            version: validate_text("evidence_refs.version", entry.version)?,
            protected_uri: ProtectedRef::try_new(entry.protected_uri)
                .map_err(|_| OperatorRequestError::InvalidField("evidence_refs.protected_uri"))?,
            sha256: parse_digest("evidence_refs.sha256", &entry.sha256)?,
        };
        let identity = (
            parsed.kind.clone(),
            parsed.version.clone(),
            parsed.protected_uri.as_str().to_owned(),
            parsed.sha256.as_str().to_owned(),
        );
        if !seen.insert(identity) {
            return Err(OperatorRequestError::DuplicateEvidence);
        }
        evidence.push(parsed);
    }
    Ok(evidence)
}

fn parse_command(value: &str) -> Result<OperatorCommand> {
    match value {
        "inspect" => Ok(OperatorCommand::Inspect),
        "reconcile" => Ok(OperatorCommand::Reconcile),
        "resolve-uncertain" => Ok(OperatorCommand::ResolveUncertain),
        "promote" => Ok(OperatorCommand::Promote),
        "rollback" => Ok(OperatorCommand::Rollback),
        _ => Err(OperatorRequestError::WireStructure),
    }
}

fn validate_text(field: &'static str, value: String) -> Result<String> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') || value.trim() != value {
        return Err(OperatorRequestError::InvalidField(field));
    }
    Ok(value)
}

fn parse_digest(field: &'static str, value: &str) -> Result<Sha256Digest> {
    Sha256Digest::parse(field, value).map_err(|_| OperatorRequestError::InvalidField(field))
}

fn command_allows_target(command: OperatorCommand, target: &OperatorTarget) -> bool {
    matches!(
        (command, target),
        (OperatorCommand::Inspect, _)
            | (
                OperatorCommand::Reconcile,
                OperatorTarget::Unit { .. } | OperatorTarget::Intent { .. }
            )
            | (
                OperatorCommand::ResolveUncertain,
                OperatorTarget::Decision { .. }
            )
            | (
                OperatorCommand::Promote | OperatorCommand::Rollback,
                OperatorTarget::Unit { .. }
            )
    )
}

#[allow(clippy::too_many_arguments)]
fn request_canonical_bytes(
    command: OperatorCommand,
    target: &OperatorTarget,
    expected_version: u64,
    expected_generation: u64,
    dry_run: bool,
    claimed_operator_ref: &str,
    reason: ReasonCode,
    evidence_refs: &[OperatorEvidenceRef],
    requested_at: UtcMicros,
) -> Vec<u8> {
    canonical_preimage(
        REQUEST_DOMAIN,
        &BTreeMap::from([
            (
                "authenticated_operator_ref",
                CanonicalValue::String(claimed_operator_ref.to_owned()),
            ),
            (
                "command",
                CanonicalValue::String(command.as_str().to_owned()),
            ),
            ("dry_run", CanonicalValue::Bool(dry_run)),
            (
                "evidence_refs",
                CanonicalValue::Array(evidence_refs.iter().map(evidence_value).collect()),
            ),
            (
                "expected_generation",
                CanonicalValue::Unsigned(expected_generation),
            ),
            (
                "expected_version",
                CanonicalValue::Unsigned(expected_version),
            ),
            ("reason", CanonicalValue::String(reason.as_str().to_owned())),
            (
                "requested_at",
                CanonicalValue::Unsigned(requested_at.get() as u64),
            ),
            ("target", target_value(target)),
        ]),
    )
}

fn target_value(target: &OperatorTarget) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("id", CanonicalValue::String(target.id().to_owned())),
        ("kind", CanonicalValue::String(target.kind().to_owned())),
        ("namespace", namespace_value(target.namespace())),
    ]))
}

fn evidence_value(evidence: &OperatorEvidenceRef) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("kind", CanonicalValue::String(evidence.kind().to_owned())),
        (
            "protected_uri",
            CanonicalValue::String(evidence.protected_uri().as_str().to_owned()),
        ),
        (
            "sha256",
            CanonicalValue::String(evidence.sha256().as_str().to_owned()),
        ),
        (
            "version",
            CanonicalValue::String(evidence.version().to_owned()),
        ),
    ]))
}
