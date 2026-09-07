#![cfg(unix)]

use crate::monitor::push_job::{CommandId, Namespace, RunId, Sha256Digest, UnitId};

use super::activation::PromotionAction;
use super::activation_authorization::{
    observe_unix_peer, refuse_unproven_dual_control, refuse_without_production_trust_root,
    ApprovalBinding, ApprovalRole, AuthorizationRefusal, ObservedUnixPeer, RawApprovalClaims,
    TestAuthorizationPolicy,
};

struct SocketFixture {
    _root: tempfile::TempDir,
    listener: tokio::net::UnixListener,
    _client: tokio::net::UnixStream,
    server: tokio::net::UnixStream,
    namespace: Namespace,
}

async fn socket_fixture(run: &str) -> SocketFixture {
    let root = tempfile::tempdir().expect("TEST_CODE create Unix socket root");
    let path = root.path().join("authorization.sock");
    let listener = tokio::net::UnixListener::bind(path).expect("TEST_CODE bind Unix listener");
    let address = listener.local_addr().expect("TEST_CODE address");
    let socket_path = address.as_pathname().expect("TEST_CODE pathname");
    let (client_result, accept_result) = tokio::join!(
        tokio::net::UnixStream::connect(socket_path),
        listener.accept()
    );
    let client = client_result.expect("TEST_CODE connect Unix client");
    let (server, _) = accept_result.expect("TEST_CODE accept Unix client");
    SocketFixture {
        _root: root,
        listener,
        _client: client,
        server,
        namespace: Namespace::test(RunId::try_new(run.to_owned()).expect("TEST_CODE valid RunId")),
    }
}

fn unit(value: &str) -> UnitId {
    UnitId::try_new(value.to_owned()).expect("TEST_CODE valid UnitId")
}

fn command(value: &str) -> CommandId {
    CommandId::try_new(value.to_owned()).expect("TEST_CODE valid CommandId")
}

fn digest(value: char) -> Sha256Digest {
    Sha256Digest::parse("TEST_CODE digest", &value.to_string().repeat(64))
        .expect("TEST_CODE valid digest")
}

fn binding(namespace: Namespace, action: PromotionAction) -> ApprovalBinding {
    ApprovalBinding {
        command_id: command("command-1"),
        namespace,
        unit_id: unit("w16-test-unit"),
        action,
        expected_generation: 11,
        target_manifest_sha256: digest('a'),
        evidence_sha256: digest('b'),
        window_start_utc_micros: 100,
        window_end_utc_micros: 200,
        approval_id: "approval-1".to_owned(),
        policy_version: "w16-test-policy-v1".to_owned(),
    }
}

fn claims(peer: &ObservedUnixPeer<'_>, binding: ApprovalBinding) -> RawApprovalClaims {
    RawApprovalClaims {
        issuer: "w16-test-issuer-v1".to_owned(),
        canonical_subject: "w16-test-subject".to_owned(),
        role: ApprovalRole::DeploymentApprover,
        claimed_peer_uid: peer.uid(),
        claimed_peer_gid: peer.gid(),
        binding,
        revoked: false,
        revocation_version: 7,
        rollback_permitted: false,
    }
}

fn compare(
    policy: &TestAuthorizationPolicy,
    peer: &ObservedUnixPeer<'_>,
    requested: &ApprovalBinding,
    claims: &RawApprovalClaims,
    now: i64,
) -> Result<super::activation_authorization::RawClaimComparison, AuthorizationRefusal> {
    policy.compare(peer, requested, claims, now)
}

#[tokio::test]
async fn real_unix_peer_observation_matches_socket_derived_test_allowlist() {
    let fixture = socket_fixture("peer-observation").await;
    let policy =
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, fixture.namespace.clone())
            .expect("TEST_CODE build fixed Test policy from bound listener");
    let peer =
        observe_unix_peer(&fixture.server).expect("TEST_CODE observe kernel peer credentials");
    let requested = binding(fixture.namespace.clone(), PromotionAction::Activate);
    let raw_claims = claims(&peer, requested.clone());

    assert!(compare(&policy, &peer, &requested, &raw_claims, 100).is_ok());
    assert_eq!(
        format!("{peer:?}"),
        "ObservedUnixPeer { identity: \"[redacted]\", connection_bound: true }"
    );
    let _pid_is_observation_only = peer.pid();
}

#[tokio::test]
async fn fake_peer_ids_and_raw_issuer_or_subject_are_rejected() {
    let fixture = socket_fixture("peer-claim-mismatch").await;
    let policy =
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, fixture.namespace.clone())
            .expect("TEST_CODE Test policy");
    let peer = observe_unix_peer(&fixture.server).expect("TEST_CODE observed peer");
    let requested = binding(fixture.namespace.clone(), PromotionAction::Activate);

    let mut wrong_uid = claims(&peer, requested.clone());
    wrong_uid.claimed_peer_uid = peer.uid().wrapping_add(1);
    assert_eq!(
        compare(&policy, &peer, &requested, &wrong_uid, 150),
        Err(AuthorizationRefusal::ClaimedUidMismatch)
    );

    let mut wrong_gid = claims(&peer, requested.clone());
    wrong_gid.claimed_peer_gid = peer.gid().wrapping_add(1);
    assert_eq!(
        compare(&policy, &peer, &requested, &wrong_gid, 150),
        Err(AuthorizationRefusal::ClaimedGidMismatch)
    );

    let mut wrong_issuer = claims(&peer, requested.clone());
    wrong_issuer.issuer = "another-issuer".to_owned();
    assert_eq!(
        compare(&policy, &peer, &requested, &wrong_issuer, 150),
        Err(AuthorizationRefusal::IssuerMismatch)
    );

    let mut wrong_subject = claims(&peer, requested.clone());
    wrong_subject.canonical_subject = "unlisted-subject".to_owned();
    assert_eq!(
        compare(&policy, &peer, &requested, &wrong_subject, 150),
        Err(AuthorizationRefusal::SubjectNotAllowed)
    );
}

#[tokio::test]
async fn role_unit_action_and_test_namespace_scope_are_fixed() {
    let fixture = socket_fixture("fixed-scope").await;
    let policy =
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, fixture.namespace.clone())
            .expect("TEST_CODE Test policy");
    let peer = observe_unix_peer(&fixture.server).expect("TEST_CODE observed peer");

    let requested = binding(fixture.namespace.clone(), PromotionAction::Activate);
    let mut wrong_role = claims(&peer, requested.clone());
    wrong_role.role = ApprovalRole::Observer;
    assert_eq!(
        compare(&policy, &peer, &requested, &wrong_role, 150),
        Err(AuthorizationRefusal::RoleNotAllowed)
    );

    let mut wrong_unit_request = requested.clone();
    wrong_unit_request.unit_id = unit("other-unit");
    let wrong_unit_claims = claims(&peer, wrong_unit_request.clone());
    assert_eq!(
        compare(&policy, &peer, &wrong_unit_request, &wrong_unit_claims, 150),
        Err(AuthorizationRefusal::UnitNotAllowed)
    );

    let mut wrong_action_request = requested.clone();
    wrong_action_request.action = PromotionAction::Disable;
    let wrong_action_claims = claims(&peer, wrong_action_request.clone());
    assert_eq!(
        compare(
            &policy,
            &peer,
            &wrong_action_request,
            &wrong_action_claims,
            150
        ),
        Err(AuthorizationRefusal::ActionNotAllowed)
    );

    let other_namespace =
        Namespace::test(RunId::try_new("other-test-run".to_owned()).expect("TEST_CODE run id"));
    let other_request = binding(other_namespace, PromotionAction::Activate);
    let other_claims = claims(&peer, other_request.clone());
    assert_eq!(
        compare(&policy, &peer, &other_request, &other_claims, 150),
        Err(AuthorizationRefusal::NamespaceNotAllowed)
    );

    let production_request = binding(Namespace::Production, PromotionAction::Activate);
    let production_claims = claims(&peer, production_request.clone());
    assert_eq!(
        compare(&policy, &peer, &production_request, &production_claims, 150),
        Err(AuthorizationRefusal::TestPolicyProductionForbidden)
    );
    assert_eq!(
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, Namespace::Production)
            .err(),
        Some(AuthorizationRefusal::TestPolicyProductionForbidden)
    );
}

#[tokio::test]
async fn every_approval_binding_field_is_compared_exactly() {
    let fixture = socket_fixture("exact-binding").await;
    let policy =
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, fixture.namespace.clone())
            .expect("TEST_CODE Test policy");
    let peer = observe_unix_peer(&fixture.server).expect("TEST_CODE observed peer");
    let requested = binding(fixture.namespace.clone(), PromotionAction::Activate);

    let cases: Vec<(&'static str, ApprovalBinding)> = vec![
        (
            "command_id",
            ApprovalBinding {
                command_id: command("command-2"),
                ..requested.clone()
            },
        ),
        (
            "namespace",
            ApprovalBinding {
                namespace: Namespace::test(
                    RunId::try_new("changed-run".to_owned()).expect("TEST_CODE run id"),
                ),
                ..requested.clone()
            },
        ),
        (
            "unit_id",
            ApprovalBinding {
                unit_id: unit("changed-unit"),
                ..requested.clone()
            },
        ),
        (
            "action",
            ApprovalBinding {
                action: PromotionAction::Rollback,
                ..requested.clone()
            },
        ),
        (
            "expected_generation",
            ApprovalBinding {
                expected_generation: 12,
                ..requested.clone()
            },
        ),
        (
            "target_manifest_sha256",
            ApprovalBinding {
                target_manifest_sha256: digest('c'),
                ..requested.clone()
            },
        ),
        (
            "evidence_sha256",
            ApprovalBinding {
                evidence_sha256: digest('d'),
                ..requested.clone()
            },
        ),
        (
            "window_start_utc_micros",
            ApprovalBinding {
                window_start_utc_micros: 101,
                ..requested.clone()
            },
        ),
        (
            "window_end_utc_micros",
            ApprovalBinding {
                window_end_utc_micros: 201,
                ..requested.clone()
            },
        ),
        (
            "approval_id",
            ApprovalBinding {
                approval_id: "approval-2".to_owned(),
                ..requested.clone()
            },
        ),
        (
            "policy_version",
            ApprovalBinding {
                policy_version: "w16-test-policy-v2".to_owned(),
                ..requested.clone()
            },
        ),
    ];

    for (field, changed_claim_binding) in cases {
        let changed_claims = claims(&peer, changed_claim_binding);
        assert_eq!(
            compare(&policy, &peer, &requested, &changed_claims, 150),
            Err(AuthorizationRefusal::ExactBindingMismatch(field)),
            "changed field {field} must refuse"
        );
    }
}

#[tokio::test]
async fn text_and_half_open_time_constraints_are_enforced() {
    let fixture = socket_fixture("claim-constraints").await;
    let policy =
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, fixture.namespace.clone())
            .expect("TEST_CODE Test policy");
    let peer = observe_unix_peer(&fixture.server).expect("TEST_CODE observed peer");
    let requested = binding(fixture.namespace.clone(), PromotionAction::Activate);

    let mut empty_issuer = claims(&peer, requested.clone());
    empty_issuer.issuer.clear();
    assert_eq!(
        compare(&policy, &peer, &requested, &empty_issuer, 150),
        Err(AuthorizationRefusal::InvalidText("issuer"))
    );

    let mut nul_subject = claims(&peer, requested.clone());
    nul_subject.canonical_subject = "subject\0suffix".to_owned();
    assert_eq!(
        compare(&policy, &peer, &requested, &nul_subject, 150),
        Err(AuthorizationRefusal::InvalidText("canonical_subject"))
    );

    let mut oversized_approval = requested.clone();
    oversized_approval.approval_id = "a".repeat(513);
    let oversized_claims = claims(&peer, oversized_approval.clone());
    assert_eq!(
        compare(&policy, &peer, &oversized_approval, &oversized_claims, 150),
        Err(AuthorizationRefusal::InvalidText("approval_id"))
    );

    let invalid_windows = [(-1, 200), (100, -1), (100, 100), (200, 100)];
    for (start, end) in invalid_windows {
        let invalid = ApprovalBinding {
            window_start_utc_micros: start,
            window_end_utc_micros: end,
            ..requested.clone()
        };
        let invalid_claims = claims(&peer, invalid.clone());
        let expected = if start < 0 || end < 0 {
            AuthorizationRefusal::NegativeTime
        } else {
            AuthorizationRefusal::InvalidApprovalWindow
        };
        assert_eq!(
            compare(&policy, &peer, &invalid, &invalid_claims, 150),
            Err(expected)
        );
    }

    let valid_claims = claims(&peer, requested.clone());
    assert_eq!(
        compare(&policy, &peer, &requested, &valid_claims, -1),
        Err(AuthorizationRefusal::NegativeTime)
    );
    assert!(compare(&policy, &peer, &requested, &valid_claims, 100).is_ok());
    assert_eq!(
        compare(&policy, &peer, &requested, &valid_claims, 99),
        Err(AuthorizationRefusal::ApprovalNotYetEffective)
    );
    assert_eq!(
        compare(&policy, &peer, &requested, &valid_claims, 200),
        Err(AuthorizationRefusal::ApprovalExpired)
    );

    let generation_overflow = ApprovalBinding {
        expected_generation: (i64::MAX as u64) + 1,
        ..requested
    };
    let overflow_claims = claims(&peer, generation_overflow.clone());
    assert_eq!(
        compare(&policy, &peer, &generation_overflow, &overflow_claims, 150),
        Err(AuthorizationRefusal::ExpectedGenerationOutOfRange)
    );
}

#[tokio::test]
async fn revocation_and_dedicated_rollback_permission_are_mandatory() {
    let fixture = socket_fixture("revocation-rollback").await;
    let policy =
        TestAuthorizationPolicy::from_bound_listener(&fixture.listener, fixture.namespace.clone())
            .expect("TEST_CODE Test policy");
    let peer = observe_unix_peer(&fixture.server).expect("TEST_CODE observed peer");
    let activate = binding(fixture.namespace.clone(), PromotionAction::Activate);

    let mut revoked = claims(&peer, activate.clone());
    revoked.revoked = true;
    assert_eq!(
        compare(&policy, &peer, &activate, &revoked, 150),
        Err(AuthorizationRefusal::ApprovalRevoked)
    );

    let mut stale = claims(&peer, activate.clone());
    stale.revocation_version = 6;
    assert_eq!(
        compare(&policy, &peer, &activate, &stale, 150),
        Err(AuthorizationRefusal::RevocationVersionMismatch)
    );

    let rollback = binding(fixture.namespace.clone(), PromotionAction::Rollback);
    let without_permission = claims(&peer, rollback.clone());
    assert_eq!(
        compare(&policy, &peer, &rollback, &without_permission, 150),
        Err(AuthorizationRefusal::RollbackPermissionRequired)
    );
    let mut with_permission = claims(&peer, rollback.clone());
    with_permission.rollback_permitted = true;
    assert!(compare(&policy, &peer, &rollback, &with_permission, 150).is_ok());
}

#[tokio::test]
async fn two_connections_from_the_same_unix_identity_cannot_satisfy_dual_control() {
    let fixture = socket_fixture("dual-control").await;
    let path = fixture
        .listener
        .local_addr()
        .expect("TEST_CODE address")
        .as_pathname()
        .expect("TEST_CODE pathname")
        .to_owned();
    let (second_client_result, second_accept_result) = tokio::join!(
        tokio::net::UnixStream::connect(path),
        fixture.listener.accept()
    );
    let _second_client = second_client_result.expect("TEST_CODE connect second peer");
    let (second_server, _) = second_accept_result.expect("TEST_CODE accept second peer");

    let first = observe_unix_peer(&fixture.server).expect("TEST_CODE first observed peer");
    let second = observe_unix_peer(&second_server).expect("TEST_CODE second observed peer");
    assert_eq!(
        refuse_unproven_dual_control(&first, &second),
        Err(AuthorizationRefusal::DualControlSameUnixIdentity)
    );
}

#[test]
fn missing_production_root_has_no_success_value() {
    assert_eq!(
        refuse_without_production_trust_root(&Namespace::Production),
        Err(AuthorizationRefusal::ProductionTrustRootUnavailable)
    );
}
