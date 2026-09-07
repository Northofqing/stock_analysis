//! Raw activation-authorization observations and claim comparisons.
//!
//! This module deliberately issues no authenticated or approved production value. Unix peer
//! credentials identify the kernel peer of one live connection, while approval claims remain
//! untrusted input. A production trust root, canonical operator identity issuer, durable approval
//! replay store, and mandatory PAM path are still required before an execution can be authorized.

#![cfg_attr(not(test), allow(dead_code))]

use std::convert::Infallible;
use std::fmt;
#[cfg(unix)]
use std::marker::PhantomData;

use crate::monitor::push_job::{CommandId, Namespace, Sha256Digest, UnitId};

use super::activation::PromotionAction;

#[cfg(unix)]
use tokio::net::UnixStream;

const MAX_TEXT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum AuthorizationRefusal {
    #[error("Unix peer credentials are unavailable")]
    PeerCredentialsUnavailable,
    #[cfg(not(unix))]
    #[error("Unix peer credential observation is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("invalid authorization text: {0}")]
    InvalidText(&'static str),
    #[error("authorization time must be non-negative")]
    NegativeTime,
    #[error("approval window must be a non-empty half-open interval")]
    InvalidApprovalWindow,
    #[error("expected generation exceeds signed persistence range")]
    ExpectedGenerationOutOfRange,
    #[error("the claimed Unix peer UID does not match the observed connection")]
    ClaimedUidMismatch,
    #[error("the claimed Unix peer GID does not match the observed connection")]
    ClaimedGidMismatch,
    #[error("the observed Unix peer is not in the fixed Test allowlist")]
    PeerNotAllowed,
    #[error("Test authorization policy cannot be used for Production")]
    TestPolicyProductionForbidden,
    #[error("approval claim does not exactly bind {0}")]
    ExactBindingMismatch(&'static str),
    #[error("approval issuer is not trusted by the selected policy")]
    IssuerMismatch,
    #[error("approval subject is not permitted by the selected policy")]
    SubjectNotAllowed,
    #[error("approval role is not permitted by the selected policy")]
    RoleNotAllowed,
    #[error("approval namespace is outside the selected policy scope")]
    NamespaceNotAllowed,
    #[error("approval Unit is outside the selected policy scope")]
    UnitNotAllowed,
    #[error("promotion action is outside the selected policy scope")]
    ActionNotAllowed,
    #[error("approval has been revoked")]
    ApprovalRevoked,
    #[error("approval revocation version is stale or from another policy state")]
    RevocationVersionMismatch,
    #[error("approval is not yet effective")]
    ApprovalNotYetEffective,
    #[error("approval has expired")]
    ApprovalExpired,
    #[error("Rollback requires its dedicated permission")]
    RollbackPermissionRequired,
    #[error("DualControl cannot use the same Unix identity twice")]
    DualControlSameUnixIdentity,
    #[error("DualControl canonical identity proof is unavailable")]
    DualControlIdentityRootUnavailable,
    #[error("production authorization trust root is unavailable")]
    ProductionTrustRootUnavailable,
}

/// Kernel credentials for the peer of one borrowed live Unix connection.
///
/// UID, GID, and PID are transport observations only. In particular, they do not prove a
/// supervisor instance or a canonical human operator identity. This value cannot outlive the
/// connection from which it was observed and intentionally is neither cloneable nor serializable.
#[cfg(unix)]
pub(super) struct ObservedUnixPeer<'connection> {
    uid: u32,
    gid: u32,
    pid: Option<i32>,
    _connection: PhantomData<&'connection UnixStream>,
}

#[cfg(unix)]
impl fmt::Debug for ObservedUnixPeer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservedUnixPeer")
            .field("identity", &"[redacted]")
            .field("connection_bound", &true)
            .finish()
    }
}

#[cfg(unix)]
impl ObservedUnixPeer<'_> {
    pub(super) fn uid(&self) -> u32 {
        self.uid
    }

    pub(super) fn gid(&self) -> u32 {
        self.gid
    }

    pub(super) fn pid(&self) -> Option<i32> {
        self.pid
    }
}

/// Observes the peer through the kernel-owned credentials on this exact connection.
#[cfg(unix)]
pub(super) fn observe_unix_peer(
    connection: &UnixStream,
) -> Result<ObservedUnixPeer<'_>, AuthorizationRefusal> {
    let credentials = connection
        .peer_cred()
        .map_err(|_| AuthorizationRefusal::PeerCredentialsUnavailable)?;
    Ok(ObservedUnixPeer {
        uid: credentials.uid(),
        gid: credentials.gid(),
        pid: credentials.pid(),
        _connection: PhantomData,
    })
}

/// Makes the non-Unix refusal explicit without inventing a portable credential substitute.
#[cfg(not(unix))]
pub(super) fn observe_unix_peer() -> Result<Infallible, AuthorizationRefusal> {
    Err(AuthorizationRefusal::UnsupportedPlatform)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ApprovalRole {
    DeploymentApprover,
    Observer,
}

/// Exact command material presented for comparison. This is a claim, not trusted authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApprovalBinding {
    pub(super) command_id: CommandId,
    pub(super) namespace: Namespace,
    pub(super) unit_id: UnitId,
    pub(super) action: PromotionAction,
    pub(super) expected_generation: u64,
    pub(super) target_manifest_sha256: Sha256Digest,
    pub(super) evidence_sha256: Sha256Digest,
    pub(super) window_start_utc_micros: i64,
    pub(super) window_end_utc_micros: i64,
    pub(super) approval_id: String,
    pub(super) policy_version: String,
}

/// Unauthenticated approval record fields supplied by the eventual approval-record adapter.
///
/// Even an exact match is only a raw comparison. The issuer and canonical subject still need a
/// configured production identity root, and `approval_id` alone does not prevent replay across
/// process restarts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RawApprovalClaims {
    pub(super) issuer: String,
    pub(super) canonical_subject: String,
    pub(super) role: ApprovalRole,
    pub(super) claimed_peer_uid: u32,
    pub(super) claimed_peer_gid: u32,
    pub(super) binding: ApprovalBinding,
    pub(super) revoked: bool,
    pub(super) revocation_version: u64,
    pub(super) rollback_permitted: bool,
}

/// The result of comparing raw claims only. It is not authentication or execution approval.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct RawClaimComparison {
    _raw_only: (),
}

struct ClaimComparisonPolicy {
    namespace: Namespace,
    unit_id: UnitId,
    allowed_actions: [PromotionAction; 2],
    issuer: String,
    subject: String,
    role: ApprovalRole,
    rollback_allowed: bool,
    policy_version: String,
    revocation_version: u64,
    allowed_uid: u32,
    allowed_gid: u32,
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && !value.contains('\0')
        && value.trim() == value
}

fn validate_binding(binding: &ApprovalBinding) -> Result<(), AuthorizationRefusal> {
    if !valid_text(&binding.approval_id) {
        return Err(AuthorizationRefusal::InvalidText("approval_id"));
    }
    if !valid_text(&binding.policy_version) {
        return Err(AuthorizationRefusal::InvalidText("policy_version"));
    }
    if binding.window_start_utc_micros < 0 || binding.window_end_utc_micros < 0 {
        return Err(AuthorizationRefusal::NegativeTime);
    }
    if binding.window_start_utc_micros >= binding.window_end_utc_micros {
        return Err(AuthorizationRefusal::InvalidApprovalWindow);
    }
    if binding.expected_generation > i64::MAX as u64 {
        return Err(AuthorizationRefusal::ExpectedGenerationOutOfRange);
    }
    Ok(())
}

fn require_exact_binding(
    requested: &ApprovalBinding,
    claimed: &ApprovalBinding,
) -> Result<(), AuthorizationRefusal> {
    if claimed.command_id != requested.command_id {
        return Err(AuthorizationRefusal::ExactBindingMismatch("command_id"));
    }
    if claimed.namespace != requested.namespace {
        return Err(AuthorizationRefusal::ExactBindingMismatch("namespace"));
    }
    if claimed.unit_id != requested.unit_id {
        return Err(AuthorizationRefusal::ExactBindingMismatch("unit_id"));
    }
    if claimed.action != requested.action {
        return Err(AuthorizationRefusal::ExactBindingMismatch("action"));
    }
    if claimed.expected_generation != requested.expected_generation {
        return Err(AuthorizationRefusal::ExactBindingMismatch(
            "expected_generation",
        ));
    }
    if claimed.target_manifest_sha256 != requested.target_manifest_sha256 {
        return Err(AuthorizationRefusal::ExactBindingMismatch(
            "target_manifest_sha256",
        ));
    }
    if claimed.evidence_sha256 != requested.evidence_sha256 {
        return Err(AuthorizationRefusal::ExactBindingMismatch(
            "evidence_sha256",
        ));
    }
    if claimed.window_start_utc_micros != requested.window_start_utc_micros {
        return Err(AuthorizationRefusal::ExactBindingMismatch(
            "window_start_utc_micros",
        ));
    }
    if claimed.window_end_utc_micros != requested.window_end_utc_micros {
        return Err(AuthorizationRefusal::ExactBindingMismatch(
            "window_end_utc_micros",
        ));
    }
    if claimed.approval_id != requested.approval_id {
        return Err(AuthorizationRefusal::ExactBindingMismatch("approval_id"));
    }
    if claimed.policy_version != requested.policy_version {
        return Err(AuthorizationRefusal::ExactBindingMismatch("policy_version"));
    }
    Ok(())
}

#[cfg(unix)]
fn compare_raw_claims(
    policy: &ClaimComparisonPolicy,
    peer: &ObservedUnixPeer<'_>,
    requested: &ApprovalBinding,
    claims: &RawApprovalClaims,
    now_utc_micros: i64,
) -> Result<RawClaimComparison, AuthorizationRefusal> {
    validate_binding(requested)?;
    validate_binding(&claims.binding)?;
    if !valid_text(&claims.issuer) {
        return Err(AuthorizationRefusal::InvalidText("issuer"));
    }
    if !valid_text(&claims.canonical_subject) {
        return Err(AuthorizationRefusal::InvalidText("canonical_subject"));
    }
    if now_utc_micros < 0 {
        return Err(AuthorizationRefusal::NegativeTime);
    }

    if matches!(&requested.namespace, Namespace::Production)
        || matches!(&claims.binding.namespace, Namespace::Production)
    {
        return Err(AuthorizationRefusal::TestPolicyProductionForbidden);
    }
    if peer.uid != policy.allowed_uid || peer.gid != policy.allowed_gid {
        return Err(AuthorizationRefusal::PeerNotAllowed);
    }
    if claims.claimed_peer_uid != peer.uid {
        return Err(AuthorizationRefusal::ClaimedUidMismatch);
    }
    if claims.claimed_peer_gid != peer.gid {
        return Err(AuthorizationRefusal::ClaimedGidMismatch);
    }

    require_exact_binding(requested, &claims.binding)?;

    if claims.issuer != policy.issuer {
        return Err(AuthorizationRefusal::IssuerMismatch);
    }
    if claims.canonical_subject != policy.subject {
        return Err(AuthorizationRefusal::SubjectNotAllowed);
    }
    if claims.role != policy.role {
        return Err(AuthorizationRefusal::RoleNotAllowed);
    }
    if claims.binding.namespace != policy.namespace {
        return Err(AuthorizationRefusal::NamespaceNotAllowed);
    }
    if claims.binding.unit_id != policy.unit_id {
        return Err(AuthorizationRefusal::UnitNotAllowed);
    }
    if !policy.allowed_actions.contains(&claims.binding.action) {
        return Err(AuthorizationRefusal::ActionNotAllowed);
    }
    if claims.binding.policy_version != policy.policy_version {
        return Err(AuthorizationRefusal::ExactBindingMismatch("policy_version"));
    }
    if claims.revoked {
        return Err(AuthorizationRefusal::ApprovalRevoked);
    }
    if claims.revocation_version != policy.revocation_version {
        return Err(AuthorizationRefusal::RevocationVersionMismatch);
    }
    if claims.binding.action == PromotionAction::Rollback
        && (!policy.rollback_allowed || !claims.rollback_permitted)
    {
        return Err(AuthorizationRefusal::RollbackPermissionRequired);
    }
    if now_utc_micros < claims.binding.window_start_utc_micros {
        return Err(AuthorizationRefusal::ApprovalNotYetEffective);
    }
    if now_utc_micros >= claims.binding.window_end_utc_micros {
        return Err(AuthorizationRefusal::ApprovalExpired);
    }

    Ok(RawClaimComparison { _raw_only: () })
}

/// There is deliberately no production-success branch until a protected trust root is selected.
pub(super) fn refuse_without_production_trust_root(
    namespace: &Namespace,
) -> Result<Infallible, AuthorizationRefusal> {
    match namespace {
        Namespace::Production => Err(AuthorizationRefusal::ProductionTrustRootUnavailable),
        Namespace::Test { .. } => Err(AuthorizationRefusal::TestPolicyProductionForbidden),
    }
}

/// DualControl cannot be proven from raw labels, UID, GID, or PID.
///
/// The same kernel UID is rejected explicitly. Even different UIDs remain refused until the
/// production identity issuer can prove two distinct canonical operator subjects.
#[cfg(unix)]
pub(super) fn refuse_unproven_dual_control(
    first: &ObservedUnixPeer<'_>,
    second: &ObservedUnixPeer<'_>,
) -> Result<Infallible, AuthorizationRefusal> {
    if first.uid == second.uid {
        return Err(AuthorizationRefusal::DualControlSameUnixIdentity);
    }
    Err(AuthorizationRefusal::DualControlIdentityRootUnavailable)
}

#[cfg(test)]
pub(super) struct TestAuthorizationPolicy {
    comparison: ClaimComparisonPolicy,
}

#[cfg(all(test, unix))]
impl TestAuthorizationPolicy {
    pub(super) fn from_bound_listener(
        listener: &tokio::net::UnixListener,
        namespace: Namespace,
    ) -> Result<Self, AuthorizationRefusal> {
        use std::os::unix::fs::MetadataExt;

        if matches!(&namespace, Namespace::Production) {
            return Err(AuthorizationRefusal::TestPolicyProductionForbidden);
        }
        let path = listener
            .local_addr()
            .map_err(|_| AuthorizationRefusal::PeerCredentialsUnavailable)?
            .as_pathname()
            .ok_or(AuthorizationRefusal::PeerCredentialsUnavailable)?
            .to_owned();
        let metadata = std::fs::metadata(path)
            .map_err(|_| AuthorizationRefusal::PeerCredentialsUnavailable)?;

        let unit_id = UnitId::try_new("w16-test-unit".to_owned())
            .map_err(|_| AuthorizationRefusal::InvalidText("unit_id"))?;
        Ok(Self {
            comparison: ClaimComparisonPolicy {
                namespace,
                unit_id,
                allowed_actions: [PromotionAction::Activate, PromotionAction::Rollback],
                issuer: "w16-test-issuer-v1".to_owned(),
                subject: "w16-test-subject".to_owned(),
                role: ApprovalRole::DeploymentApprover,
                rollback_allowed: true,
                policy_version: "w16-test-policy-v1".to_owned(),
                revocation_version: 7,
                allowed_uid: metadata.uid(),
                allowed_gid: metadata.gid(),
            },
        })
    }

    pub(super) fn compare(
        &self,
        peer: &ObservedUnixPeer<'_>,
        requested: &ApprovalBinding,
        claims: &RawApprovalClaims,
        now_utc_micros: i64,
    ) -> Result<RawClaimComparison, AuthorizationRefusal> {
        compare_raw_claims(&self.comparison, peer, requested, claims, now_utc_micros)
    }
}
