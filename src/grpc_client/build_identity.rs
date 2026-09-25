//! Qualification of an ExternalV1 server against the deployment identity
//! shipped with the public client contract. A live process is not by itself a
//! qualified source of market data.

use super::external_pb::magic::market::v1::{BuildIdentity, HealthResponse};
use serde::Deserialize;

const PUBLIC_BUNDLE_METADATA: &str = include_str!("../../client-bundle/bundle-metadata.json");
const CONTRACT_HASH_SCOPE: &str =
    "SHA-256 of the raw compiled FileDescriptorSet bytes returned by magic_market_grpc_contracts::v1::FILE_DESCRIPTOR_SET";
const BINARY_HASH_SCOPE: &str =
    "SHA-256 of the exact deployed magic-market-grpc-server executable bytes";

#[derive(Debug, Deserialize)]
struct BundleMetadata {
    deployment_build_identity: Option<ExpectedBuildIdentity>,
}

#[derive(Debug, Deserialize)]
struct ExpectedBuildIdentity {
    service_version: String,
    source_revision: String,
    contract_sha256: String,
    binary_sha256: String,
    contract_sha256_scope: String,
    binary_sha256_scope: String,
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
    let metadata: BundleMetadata = serde_json::from_str(PUBLIC_BUNDLE_METADATA)
        .map_err(|_| BuildIdentityError::ExpectedIdentityUnavailable)?;
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

/// The same rule is used before online queries, by the probe, and when
/// interpreting persisted Health bytes. It never learns an expected identity
/// from an untrusted first response.
pub fn qualify_public_health(response: &HealthResponse) -> Result<(), BuildIdentityError> {
    if !response.live || !response.ready {
        return Err(BuildIdentityError::NotReady);
    }
    qualify_identity(response.build_identity.as_ref(), &expected_identity()?)
}

pub(crate) fn qualify_public_build_identity(
    identity: &BuildIdentity,
) -> Result<(), BuildIdentityError> {
    qualify_identity(Some(identity), &expected_identity()?)
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
}
