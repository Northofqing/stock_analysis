//! Audited candidate inputs; no runtime/response expected-identity factory.
use super::{policy_sha256, BuildIdentityError, BundleMetadata, ExpectedBuildIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const PROFILE: &str = "windows-b7-20261002.17-diagnostic-v1";
const PURPOSE: &str = "CandidateAcceptanceObservationOnly";
const POLICY: &str = "002567fb6006202984f6597a141db21f84de74d46adad7678d990e706e241597";
const SOURCE: &str = "b7d206668753dc762e776b0ff893c63d53166afe";
const BINARY: &str = "f23a7bf05f68b7f4fabc7feee27b405f1ae6a69ec281272203e5f3437114479a";
const SERVER_DESCRIPTOR: &str = "abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf";
const METADATA: &[u8] =
    include_bytes!("probe_profiles/windows-b7-20261002.17/bundle-metadata.json");
const PROTO: &[u8] = include_bytes!("probe_profiles/windows-b7-20261002.17/market.proto");
const METADATA_SHA: &str = "605e856735047f68a7cc5c1bbc3f7e65472da59c41f0c7cd395d0bf3eb0338a8";
const PROTO_SHA: &str = "801c2033e72c520b34ca110eda18029699c619b740b71e50f99308a3088689c1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCompiledInputs {
    version: u32,
    profile: String,
    purpose: String,
    bundle_version: String,
    formal_deployment_identity_available: bool,
    raw_metadata_bytes: usize,
    raw_metadata_sha256: String,
    raw_proto_bytes: usize,
    raw_proto_sha256: String,
    expected_service_version: String,
    expected_source_revision: String,
    expected_server_binary_sha256: String,
    expected_server_descriptor_sha256: String,
    compiled_client_descriptor_sha256: String,
    expected_policy_sha256: String,
}

impl CandidateCompiledInputs {
    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn policy_sha256(&self) -> &str {
        &self.expected_policy_sha256
    }
    pub fn client_descriptor_sha256(&self) -> &str {
        &self.compiled_client_descriptor_sha256
    }
}

pub(super) fn expected() -> ExpectedBuildIdentity {
    ExpectedBuildIdentity {
        service_version: "0.2.0".into(),
        source_revision: SOURCE.into(),
        contract_sha256: SERVER_DESCRIPTOR.into(),
        binary_sha256: BINARY.into(),
        contract_sha256_scope: super::CONTRACT_HASH_SCOPE.into(),
        binary_sha256_scope: super::BINARY_HASH_SCOPE.into(),
    }
}

pub(super) fn compiled_inputs() -> Result<CandidateCompiledInputs, BuildIdentityError> {
    let metadata: BundleMetadata = serde_json::from_slice(METADATA)
        .map_err(|_| BuildIdentityError::ExpectedIdentityUnavailable)?;
    let descriptor = crate::grpc_client::external_query_transport::compiled_descriptor_sha256();
    if METADATA.len() != 779
        || PROTO.len() != 12707
        || hex::encode(Sha256::digest(METADATA)) != METADATA_SHA
        || hex::encode(Sha256::digest(PROTO)) != PROTO_SHA
        || metadata.bundle_version != "2026-10-02.3"
        || metadata.deployment_build_identity.is_some()
        || descriptor
            != crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
        || policy_sha256(&expected(), &descriptor) != POLICY
    {
        return Err(BuildIdentityError::ExpectedIdentityUnavailable);
    }
    Ok(CandidateCompiledInputs {
        version: 1,
        profile: PROFILE.into(),
        purpose: PURPOSE.into(),
        bundle_version: metadata.bundle_version,
        formal_deployment_identity_available: false,
        raw_metadata_bytes: METADATA.len(),
        raw_metadata_sha256: METADATA_SHA.into(),
        raw_proto_bytes: PROTO.len(),
        raw_proto_sha256: PROTO_SHA.into(),
        expected_service_version: "0.2.0".into(),
        expected_source_revision: SOURCE.into(),
        expected_server_binary_sha256: BINARY.into(),
        expected_server_descriptor_sha256: SERVER_DESCRIPTOR.into(),
        compiled_client_descriptor_sha256: descriptor,
        expected_policy_sha256: POLICY.into(),
    })
}
