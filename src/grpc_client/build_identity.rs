//! Qualification of an ExternalV1 server against the deployment identity
//! shipped with the public client contract. A live process is not by itself a
//! qualified source of market data.

use super::external_pb::magic::market::v1::{BuildIdentity, HealthResponse};
use serde::{Deserialize, Serialize};

const PUBLIC_BUNDLE_METADATA: &str = include_str!("../../client-bundle/bundle-metadata.json");
const ARCHIVED_20260928_METADATA: &str =
    include_str!("../../contracts/external_v1_history/20260928.2/bundle-metadata.json");
// V1-V3 recorded no expected-policy receipt. Their explicit legacy policy is
// this frozen public release, never the current bundle or a response's claim.
const HISTORICAL_V3_METADATA: &str =
    include_str!("../../contracts/external_v1_history/bundle-20260917.1.json");
const CONTRACT_HASH_SCOPE: &str =
    "SHA-256 of the raw compiled FileDescriptorSet bytes returned by magic_market_grpc_contracts::v1::FILE_DESCRIPTOR_SET";
const BINARY_HASH_SCOPE: &str =
    "SHA-256 of the exact deployed magic-market-grpc-server executable bytes";

#[derive(Debug, Deserialize)]
struct BundleMetadata {
    deployment_build_identity: Option<ExpectedBuildIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ExpectedBuildIdentity {
    service_version: String,
    source_revision: String,
    contract_sha256: String,
    binary_sha256: String,
    contract_sha256_scope: String,
    binary_sha256_scope: String,
}

/// Trust inputs are compiled release policy, never learned from a response.
/// Kept separate so replay can be tested under a different current client pin.
#[derive(Clone)]
pub(crate) struct BuildIdentityTrust {
    current: ExpectedBuildIdentity,
    historical_v3: ExpectedBuildIdentity,
    archived_20260928: ExpectedBuildIdentity,
    current_descriptor: &'static str,
}

impl BuildIdentityTrust {
    pub(crate) fn current_policy_sha256(&self) -> String {
        policy_sha256(&self.current, self.current_descriptor)
    }

    pub(crate) fn current_descriptor(&self) -> &str {
        self.current_descriptor
    }

    pub(crate) fn accepts_recorded_policy(&self, digest: &str, descriptor: &str) -> bool {
        self.recorded_identity(digest, descriptor).is_some()
    }

    pub(crate) fn recorded_health(
        &self,
        digest: &str,
        descriptor: &str,
        response: &HealthResponse,
    ) -> Result<(), BuildIdentityError> {
        let expected = self
            .recorded_identity(digest, descriptor)
            .ok_or(BuildIdentityError::ExpectedIdentityUnavailable)?;
        if !response.live || !response.ready {
            return Err(BuildIdentityError::NotReady);
        }
        qualify_identity(response.build_identity.as_ref(), &expected)
    }

    fn recorded_identity(&self, digest: &str, descriptor: &str) -> Option<ExpectedBuildIdentity> {
        super::external_decoder::ExternalDecoder::for_descriptor(descriptor).ok()?;
        if descriptor == self.current_descriptor && digest == self.current_policy_sha256() {
            return Some(self.current.clone());
        } else if super::historical_external::accepts_descriptor(descriptor)
            && digest == policy_sha256(&self.historical_v3, descriptor)
        {
            return Some(self.historical_v3.clone());
        } else if super::archived_external_20260928::accepts_descriptor(descriptor)
            && digest == policy_sha256(&self.archived_20260928, descriptor)
        {
            return Some(self.archived_20260928.clone());
        }
        // Explicit compiled test release, never learned from response/env. This
        // only verifies a recorded receipt; it does not change live A's pin.
        #[cfg(test)]
        {
            for b in [Self::test_client_b(), Self::test_client_b_with_descriptor()] {
                if descriptor == b.current_descriptor && digest == b.current_policy_sha256() {
                    return Some(b.current);
                }
            }
        }
        None
    }

    pub(crate) fn bundled() -> Result<Self, BuildIdentityError> {
        Ok(Self {
            current: expected_identity()?,
            historical_v3: parse_expected_identity(HISTORICAL_V3_METADATA)?,
            archived_20260928: parse_expected_identity(ARCHIVED_20260928_METADATA)?,
            current_descriptor:
                super::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
        })
    }

    pub(crate) fn historical_identity(
        &self,
        identity: &BuildIdentity,
    ) -> Result<(), BuildIdentityError> {
        qualify_identity(Some(identity), &self.historical_v3)
    }

    pub(crate) fn current_health(
        &self,
        response: &HealthResponse,
    ) -> Result<(), BuildIdentityError> {
        if !response.live || !response.ready {
            return Err(BuildIdentityError::NotReady);
        }
        qualify_identity(response.build_identity.as_ref(), &self.current)
    }

    pub(crate) fn historical_health(
        &self,
        response: &HealthResponse,
    ) -> Result<(), BuildIdentityError> {
        if !response.live || !response.ready {
            return Err(BuildIdentityError::NotReady);
        }
        qualify_identity(response.build_identity.as_ref(), &self.historical_v3)
    }

    pub(crate) fn historical_descriptor(&self, recorded: &str) -> bool {
        super::historical_external::accepts_descriptor(recorded)
    }

    #[cfg(test)]
    pub(crate) fn test_client_b_with_descriptor() -> Self {
        let mut value = Self::test_client_b();
        value.current_descriptor = super::external_decoder::test_b::descriptor();
        value.current.contract_sha256 = value.current_descriptor.to_owned();
        value
    }

    #[cfg(test)]
    pub(crate) fn test_historical_a() -> Self {
        let mut value = Self::bundled().unwrap();
        value.current = value.historical_v3.clone();
        value.current_descriptor = super::historical_external::DESCRIPTOR_SHA256;
        value
    }

    #[cfg(test)]
    pub(crate) fn test_client_b() -> Self {
        let mut value = Self::bundled().unwrap();
        value.current.source_revision = "TEST_CODE_TRUSTED_RELEASE_B".into();
        value.current.binary_sha256 = "b".repeat(64);
        value
    }
}

fn policy_sha256(identity: &ExpectedBuildIdentity, descriptor: &str) -> String {
    // Ordered, versioned public trust inputs. Never hash or learn the response.
    let bytes = serde_json::to_vec(&(
        "stock_analysis.external_qualification_policy.v1",
        identity,
        descriptor,
    ))
    .expect("public identity contains only serializable strings");
    crate::monitor::push_job::raw_digest(&bytes)
        .as_str()
        .to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildIdentityError {
    NotReady,
    ExpectedIdentityUnavailable,
    MissingHealthIdentity,
    ServerIdentityError,
    MissingField(&'static str),
    Mismatch(&'static str),
}

impl std::fmt::Display for BuildIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady => formatter.write_str("ExternalV1 health is not live and ready"),
            Self::ExpectedIdentityUnavailable => {
                formatter.write_str("trusted deployment build identity is unavailable")
            }
            Self::MissingHealthIdentity => formatter.write_str("Health lacks build_identity"),
            Self::ServerIdentityError => formatter.write_str("Health reports identity_error"),
            Self::MissingField(field) => write!(formatter, "Health build_identity lacks {field}"),
            Self::Mismatch(field) => write!(
                formatter,
                "Health build_identity {field} mismatches deployment metadata"
            ),
        }
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn expected_identity() -> Result<ExpectedBuildIdentity, BuildIdentityError> {
    parse_expected_identity(PUBLIC_BUNDLE_METADATA)
}

fn parse_expected_identity(bytes: &str) -> Result<ExpectedBuildIdentity, BuildIdentityError> {
    let metadata: BundleMetadata =
        serde_json::from_str(bytes).map_err(|_| BuildIdentityError::ExpectedIdentityUnavailable)?;
    let expected = metadata
        .deployment_build_identity
        .ok_or(BuildIdentityError::ExpectedIdentityUnavailable)?;
    if expected.service_version.trim().is_empty()
        || expected.source_revision.trim().is_empty()
        || !valid_sha256(&expected.contract_sha256)
        || !valid_sha256(&expected.binary_sha256)
        || expected.contract_sha256_scope != CONTRACT_HASH_SCOPE
        || expected.binary_sha256_scope != BINARY_HASH_SCOPE
    {
        return Err(BuildIdentityError::ExpectedIdentityUnavailable);
    }
    Ok(expected)
}

fn qualify_identity(
    actual: Option<&BuildIdentity>,
    expected: &ExpectedBuildIdentity,
) -> Result<(), BuildIdentityError> {
    let actual = actual.ok_or(BuildIdentityError::MissingHealthIdentity)?;
    if !actual.identity_error.is_empty() {
        return Err(BuildIdentityError::ServerIdentityError);
    }
    for (field, observed, pinned) in [
        (
            "service_version",
            actual.service_version.as_str(),
            expected.service_version.as_str(),
        ),
        (
            "source_revision",
            actual.source_revision.as_str(),
            expected.source_revision.as_str(),
        ),
        (
            "contract_sha256",
            actual.contract_sha256.as_str(),
            expected.contract_sha256.as_str(),
        ),
        (
            "binary_sha256",
            actual.binary_sha256.as_str(),
            expected.binary_sha256.as_str(),
        ),
    ] {
        if observed.is_empty() {
            return Err(BuildIdentityError::MissingField(field));
        }
        if observed != pinned {
            return Err(BuildIdentityError::Mismatch(field));
        }
    }
    Ok(())
}

/// Current live qualification only; historical V1-V3 replay uses its frozen
/// release policy. Neither learns expected identity from a first response.
pub fn qualify_public_health(response: &HealthResponse) -> Result<(), BuildIdentityError> {
    if !response.live || !response.ready {
        return Err(BuildIdentityError::NotReady);
    }
    qualify_identity(response.build_identity.as_ref(), &expected_identity()?)
}

#[cfg(test)]
pub(crate) fn test_public_build_identity() -> BuildIdentity {
    let expected = expected_identity().expect("public deployment metadata is complete");
    BuildIdentity {
        service_version: expected.service_version,
        source_revision: expected.source_revision,
        contract_sha256: expected.contract_sha256,
        binary_sha256: expected.binary_sha256,
        identity_error: String::new(),
    }
}

#[cfg(test)]
pub(crate) fn test_historical_build_identity() -> BuildIdentity {
    let expected = parse_expected_identity(HISTORICAL_V3_METADATA)
        .expect("frozen public deployment metadata is complete");
    BuildIdentity {
        service_version: expected.service_version,
        source_revision: expected.source_revision,
        contract_sha256: expected.contract_sha256,
        binary_sha256: expected.binary_sha256,
        identity_error: String::new(),
    }
}

// Public old-production GetHealth receipt shape from the sealed SDK evidence.
// This fixture is historical evidence, never an expected current identity.
#[cfg(test)]
pub(crate) fn test_archived_20260928_health() -> HealthResponse {
    use super::external_pb::magic::market::v1::RuntimeObservability;
    HealthResponse {
        request_id: "sdk-date-release-preflight-20261006-GetHealth".into(),
        live: true,
        ready: true,
        state: "ready".into(),
        observability: Some(RuntimeObservability {
            process_started_at_unix_ms: 1791029431735,
            uptime_millis: 231721599,
            query_started: 20093,
            query_succeeded: 20028,
            query_failed: 65,
            query_cancelled: 1,
            query_in_flight: 0,
            query_rejected: 0,
            query_timed_out: 0,
            query_duration_micros_total: 2571960202,
            query_duration_micros_max: 10657523,
            unary_concurrency_limit: 16,
            unary_concurrency_available: 16,
            blocking_concurrency_limit: 8,
            blocking_concurrency_available: 8,
        }),
        build_identity: Some(BuildIdentity {
            service_version: "0.2.0".into(),
            source_revision: "4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c".into(),
            contract_sha256: "0c4485545dbfd0979a7d5ea206c840f39fd504ed62fb7eef92f1940bdc9c2f41"
                .into(),
            binary_sha256: "517e0b4c31bb42330f4bc2a0395e3af385a164212414a65775bed40c9eb87ae3"
                .into(),
            identity_error: String::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> ExpectedBuildIdentity {
        expected_identity().expect("public deployment metadata is complete")
    }

    fn actual(expected: &ExpectedBuildIdentity) -> BuildIdentity {
        BuildIdentity {
            service_version: expected.service_version.clone(),
            source_revision: expected.source_revision.clone(),
            contract_sha256: expected.contract_sha256.clone(),
            binary_sha256: expected.binary_sha256.clone(),
            identity_error: String::new(),
        }
    }

    #[test]
    fn public_identity_rejects_missing_and_changed_deployment_fields() {
        let expected = expected();
        let good = actual(&expected);
        assert_eq!(qualify_identity(Some(&good), &expected), Ok(()));
        assert_eq!(
            qualify_identity(None, &expected),
            Err(BuildIdentityError::MissingHealthIdentity)
        );
        for field in [
            "service_version",
            "source_revision",
            "contract_sha256",
            "binary_sha256",
        ] {
            let mut missing = good.clone();
            let mut changed = good.clone();
            match field {
                "service_version" => {
                    missing.service_version.clear();
                    changed.service_version.push('x');
                }
                "source_revision" => {
                    missing.source_revision.clear();
                    changed.source_revision.push('x');
                }
                "contract_sha256" => {
                    missing.contract_sha256.clear();
                    changed.contract_sha256.push('x');
                }
                "binary_sha256" => {
                    missing.binary_sha256.clear();
                    changed.binary_sha256.push('x');
                }
                _ => unreachable!(),
            }
            assert_eq!(
                qualify_identity(Some(&missing), &expected),
                Err(BuildIdentityError::MissingField(field))
            );
            assert_eq!(
                qualify_identity(Some(&changed), &expected),
                Err(BuildIdentityError::Mismatch(field))
            );
        }
        let mut reported_error = good;
        reported_error.identity_error = "TEST_CODE_HASH_READ_FAILED".to_owned();
        assert_eq!(
            qualify_identity(Some(&reported_error), &expected),
            Err(BuildIdentityError::ServerIdentityError)
        );
    }

    #[test]
    fn readiness_does_not_override_missing_or_wrong_identity() {
        let expected = expected();
        let mut response = HealthResponse {
            live: true,
            ready: true,
            build_identity: Some(actual(&expected)),
            ..Default::default()
        };
        assert_eq!(qualify_public_health(&response), Ok(()));
        response
            .build_identity
            .as_mut()
            .unwrap()
            .binary_sha256
            .push('x');
        assert_eq!(
            qualify_public_health(&response),
            Err(BuildIdentityError::Mismatch("binary_sha256"))
        );
        response.ready = false;
        assert_eq!(
            qualify_public_health(&response),
            Err(BuildIdentityError::NotReady)
        );
    }
    #[test]
    fn sep28_recorded_policy_binds_old_descriptor_and_observed_health_only() {
        let trust = BuildIdentityTrust::bundled().unwrap();
        let descriptor = super::super::archived_external_20260928::DESCRIPTOR_SHA256;
        let policy = "de9a897d7e35f35bdd475b4133f002b27e9dfb40cb029aeb81d590ac3e70f54a";
        assert_eq!(policy_sha256(&trust.archived_20260928, descriptor), policy);
        let old_health = test_archived_20260928_health();
        assert_eq!(
            trust.recorded_health(policy, descriptor, &old_health),
            Ok(())
        );
        assert!(trust.current_health(&old_health).is_err());
        assert!(qualify_public_health(&old_health).is_err());
        assert!(!trust.accepts_recorded_policy(policy, trust.current_descriptor()));
        assert!(!trust.accepts_recorded_policy(&trust.current_policy_sha256(), descriptor));
        assert!(trust
            .recorded_health(policy, trust.current_descriptor(), &old_health)
            .is_err());
        let candidate_health = HealthResponse {
            build_identity: Some(test_public_build_identity()),
            ..old_health.clone()
        };
        assert!(trust
            .recorded_health(policy, descriptor, &candidate_health)
            .is_err());
        let live = super::super::connection_qualification::ConnectionGeneration::new(trust);
        assert!(live
            .observe_health(&old_health.request_id, &old_health)
            .is_err());
        assert!(live.require_qualified().is_err());
    }

    #[test]
    fn declared_candidate_health_matches_compiled_expectation_in_test_only() {
        // These readiness fields are synthetic: the SDK candidate has not been probed.
        let health = HealthResponse {
            request_id: "TEST_CODE_DECLARED_CANDIDATE_NOT_OBSERVED".into(),
            live: true,
            ready: true,
            build_identity: Some(BuildIdentity {
                service_version: "0.2.0".into(),
                source_revision: "eea9cc6eea57725da1bc602dd66de68448c8d8ab".into(),
                contract_sha256: "abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf"
                    .into(),
                binary_sha256: "29a686802e80b9aa99a518b3adb68b664330b6572cc3ab4e3cb23fa51b6f29c1"
                    .into(),
                identity_error: String::new(),
            }),
            ..Default::default()
        };
        let metadata: serde_json::Value = serde_json::from_str(PUBLIC_BUNDLE_METADATA).unwrap();
        assert_eq!(
            metadata["identity_binding_state"],
            "ExpectedCandidateOnlyNotObserved"
        );
        assert_eq!(qualify_public_health(&health), Ok(()));
        let trust = BuildIdentityTrust::bundled().unwrap();
        assert_eq!(trust.current_health(&health), Ok(()));
        let live = super::super::connection_qualification::ConnectionGeneration::new(trust);
        live.observe_health(&health.request_id, &health).unwrap();
        live.require_qualified().unwrap();
    }
}
