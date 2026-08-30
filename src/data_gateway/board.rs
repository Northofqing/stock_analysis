//! BR-174/BR-177 fail-closed provider-board configuration and constituent admission.
//!
//! This is the sole public board loader/validator. Existing directory,
//! membership and flow acquisition remains behind the private runtime module.
//!
//! This module deliberately does not discover boards, fuzzy-match names, or carry a
//! static constituent list. A formal board expansion exists only when the exact
//! checked-in binding is backed by a still-valid release audit artifact.
//!
//! Data-redline coverage:
//! - AGENTS §2.1: no mock/static production fallback;
//! - AGENTS §2.2: absent binding remains explicit `DirectOnly`;
//! - AGENTS §2.3: incomplete/truncated/conflicting batches fail admission;
//! - AGENTS §2.4: provider evidence is retained byte-for-byte and chronology is checked.

pub use super::board_runtime::{
    BoardDataGateway, BoardDirectoryFact, BoardDirectoryRecordEvidence, BoardFlowFact, BoardKind,
    BoardMembershipRecord,
};

use crate::market_domain::{Exchange, ProviderId};
use crate::selection::schema_v2::{
    canonical_json as schema_canonical_json, sha256_bytes as schema_sha256_bytes,
    sha256_json as schema_sha256_json, ArtifactHashPreimage as SchemaArtifactHashPreimage,
    AttestedDirectoryBatchPreimage, BoardAuditAttestationContentPreimage,
    BoardAuditAttestationReceiptPreimage, BoardAuditRootBindingPreimage, BoardAuditSubjectPreimage,
    BoardBindingProposalInputPreimage, BoardConnectionPolicyPreimage,
    DirectoryBatchContentPreimage, DirectoryBatchEvidencePreimage, DirectoryBoardRecordPreimage,
    DirectoryRecordSourceEvidencePreimage, ProviderBoardKind, BOARD_AUDIT_COMMAND_VERSION,
    BOARD_AUDIT_ROOT_POLICY_VERSION,
    BOARD_BINDINGS_SCHEMA_VERSION as SCHEMA_BOARD_BINDINGS_VERSION,
    BOARD_BINDING_PROPOSAL_SCHEMA_VERSION, BOARD_BINDING_VALIDITY_POLICY_VERSION,
    BOARD_CONNECTION_POLICY_VERSION, BOARD_DIRECTORY_PROVIDER, BOARD_DIRECTORY_REQUEST_LIMIT,
    BOARD_DIRECTORY_SOURCE, DOMAIN_BOARD_ARTIFACT, DOMAIN_BOARD_AUDIT_ATTESTATION,
    DOMAIN_BOARD_AUDIT_RECEIPT, DOMAIN_BOARD_AUDIT_SUBJECT, DOMAIN_BOARD_BINDING_PROPOSAL,
    DOMAIN_BOARD_DIRECTORY_BATCH, DOMAIN_BOARD_DIRECTORY_RECORD, UPSTREAM_REVISION,
};
use chrono::{DateTime, SecondsFormat, Utc};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use thiserror::Error;

pub const BOARD_BINDINGS_SCHEMA_VERSION: &str = "selection-provider-board-bindings-v1";
pub const BOARD_BINDINGS_PATH: &str = "config/selection/provider_board_bindings.v1.json";
pub const BOARD_BINDING_PROPOSAL_PATH: &str =
    "config/selection/provider_board_binding_proposal.v1.json";
pub const CHAIN_RULES_PATH: &str = "config/chain.toml";
pub const PINNED_MAGIC_MARKET_REVISION: &str = "75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";
pub const BOARD_CONSTITUENT_REQUEST_LIMIT: u32 = 10_000;

const DIRECTORY_PROVIDER: &str = "tdx";
const DIRECTORY_SOURCE: &str = "tdx-block-files";
const BATCH_CONTENT_DOMAIN: &str = "stock_analysis.br174.board_constituent_batch.v1";
const RECORD_CONTENT_DOMAIN: &str = "stock_analysis.br174.board_constituent_record.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectionBoardKind {
    Industry,
    Concept,
}

impl SelectionBoardKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Industry => "industry",
            Self::Concept => "concept",
        }
    }

    const fn from_schema(value: ProviderBoardKind) -> Self {
        match value {
            ProviderBoardKind::Industry => Self::Industry,
            ProviderBoardKind::Concept => Self::Concept,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBoardBinding {
    chain_id: String,
    provider: String,
    kind: SelectionBoardKind,
    code: String,
    name: String,
    binding_audit_hash: String,
    directory_record_hash: String,
    release_directory_member_count: u32,
}

impl VerifiedBoardBinding {
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub const fn kind(&self) -> SelectionBoardKind {
        self.kind
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn binding_audit_hash(&self) -> &str {
        &self.binding_audit_hash
    }

    pub fn directory_record_hash(&self) -> &str {
        &self.directory_record_hash
    }

    pub const fn release_directory_member_count(&self) -> u32 {
        self.release_directory_member_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardBindingRegistry {
    artifact_content_hash: String,
    valid_from: String,
    expires_at: String,
    bindings: BTreeMap<String, VerifiedBoardBinding>,
}

impl BoardBindingRegistry {
    pub fn artifact_content_hash(&self) -> &str {
        &self.artifact_content_hash
    }

    pub fn valid_from(&self) -> &str {
        &self.valid_from
    }

    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionBoardConfiguration {
    registry: BoardBindingRegistry,
    chain_bindings: BTreeMap<String, Option<VerifiedBoardBinding>>,
    chain_config_content_hash: String,
}

impl SelectionBoardConfiguration {
    pub fn load_default(now: DateTime<Utc>) -> Result<Self, BoardSelectionError> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let chain_bytes = read_required(&root.join(CHAIN_RULES_PATH), "chain_rules")?;
        let proposal_bytes = read_required(
            &root.join(BOARD_BINDING_PROPOSAL_PATH),
            "board_binding_proposal",
        )?;
        let artifact_bytes =
            read_required(&root.join(BOARD_BINDINGS_PATH), "board_binding_registry")?;
        Self::from_bytes(&chain_bytes, &proposal_bytes, &artifact_bytes, now)
    }

    pub fn registry(&self) -> &BoardBindingRegistry {
        &self.registry
    }

    pub fn chain_config_content_hash(&self) -> &str {
        &self.chain_config_content_hash
    }

    pub fn binding_for_chain(
        &self,
        chain_id: &str,
    ) -> Result<Option<&VerifiedBoardBinding>, BoardSelectionError> {
        validate_trimmed_text(chain_id, "chain_id")?;
        self.chain_bindings
            .get(chain_id)
            .map(Option::as_ref)
            .ok_or_else(|| {
                BoardSelectionError::invalid_config(
                    "chain_rule_not_found",
                    format!("chain rule {chain_id:?} is not present in the activated snapshot"),
                )
            })
    }

    fn from_bytes(
        chain_bytes: &[u8],
        proposal_bytes: &[u8],
        artifact_bytes: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Self, BoardSelectionError> {
        let chain_document: ChainRulesDocument =
            toml::from_str(std::str::from_utf8(chain_bytes).map_err(|error| {
                BoardSelectionError::invalid_config("chain_config_not_utf8", error.to_string())
            })?)
            .map_err(|error| {
                BoardSelectionError::invalid_config("chain_config_invalid", error.to_string())
            })?;
        let configured = validate_chain_bindings(chain_document)?;
        let artifact = parse_verified_board_artifact_pair(proposal_bytes, artifact_bytes, now)?;
        let registry = registry_from_verified_artifact(&artifact)?;
        let chain_bindings = cross_validate_bindings(configured, &registry)?;

        Ok(Self {
            registry,
            chain_bindings,
            chain_config_content_hash: sha256_bytes(chain_bytes),
        })
    }
}

/// Strictly verified checked-in proposal/artifact pair.
///
/// Fields are private so downstream activation code cannot construct or
/// partially validate release evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBoardArtifact {
    preimage: SchemaArtifactHashPreimage,
    artifact_content_hash: String,
    proposal_file_content_hash: String,
    artifact_file_content_hash: String,
}

impl VerifiedBoardArtifact {
    pub fn preimage(&self) -> &SchemaArtifactHashPreimage {
        &self.preimage
    }

    pub fn artifact_content_hash(&self) -> &str {
        &self.artifact_content_hash
    }

    pub fn proposal_file_content_hash(&self) -> &str {
        &self.proposal_file_content_hash
    }

    pub fn artifact_file_content_hash(&self) -> &str {
        &self.artifact_file_content_hash
    }

    pub fn into_preimage(self) -> SchemaArtifactHashPreimage {
        self.preimage
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedBoardArtifactFileWire {
    schema_version: String,
    artifact_content_hash: String,
    upstream_revision: String,
    proposal_input: BoardBindingProposalInputPreimage,
    proposal_input_content_hash: String,
    connection_policy_version: String,
    connection_policy_hash: String,
    provider_endpoint_evidence: Option<String>,
    valid_from: String,
    expires_at: String,
    directory_batches_by_category: Vec<DirectoryBatchEvidencePreimage>,
    requested_limit: u32,
    audit_command_version: String,
    recorded_at: String,
    audit_attestation_receipt: BoardAuditAttestationReceiptPreimage,
    audit_attestation_receipt_hash: String,
    bindings: Vec<crate::selection::schema_v2::ArtifactBindingPreimage>,
}

pub fn load_verified_board_artifact_default(
    now: DateTime<Utc>,
) -> Result<VerifiedBoardArtifact, BoardSelectionError> {
    load_verified_board_artifact_from_root(Path::new(env!("CARGO_MANIFEST_DIR")), now)
}

pub(crate) fn load_verified_board_artifact_from_root(
    repository_root: &Path,
    now: DateTime<Utc>,
) -> Result<VerifiedBoardArtifact, BoardSelectionError> {
    #[cfg(not(test))]
    {
        let fixed_root = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).map_err(|error| {
            BoardSelectionError::invalid_config(
                "fixed_repository_root_unavailable",
                error.to_string(),
            )
        })?;
        if repository_root != fixed_root {
            return Err(BoardSelectionError::invalid_config(
                "diagnostic_repository_root_release_forbidden",
                "release-capable board loading is restricted to the compile-time repository root",
            ));
        }
    }
    let proposal_bytes = read_required(
        &repository_root.join(BOARD_BINDING_PROPOSAL_PATH),
        "board_binding_proposal",
    )?;
    let artifact_bytes = read_required(
        &repository_root.join(BOARD_BINDINGS_PATH),
        "board_binding_artifact",
    )?;
    parse_verified_board_artifact_pair(&proposal_bytes, &artifact_bytes, now)
}

fn parse_verified_board_artifact_pair(
    proposal_bytes: &[u8],
    artifact_bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<VerifiedBoardArtifact, BoardSelectionError> {
    let proposal: BoardBindingProposalInputPreimage = parse_canonical_schema_file(
        proposal_bytes,
        "board_proposal_invalid_json",
        "board_proposal_not_canonical",
    )?;
    proposal.validate().map_err(schema_board_error)?;

    let wire: VerifiedBoardArtifactFileWire = parse_canonical_schema_file(
        artifact_bytes,
        "board_artifact_invalid_json",
        "board_artifact_not_canonical",
    )?;
    let preimage = SchemaArtifactHashPreimage {
        domain: DOMAIN_BOARD_ARTIFACT.to_owned(),
        schema_version: wire.schema_version,
        upstream_revision: wire.upstream_revision,
        proposal_input: wire.proposal_input,
        proposal_input_content_hash: wire.proposal_input_content_hash,
        connection_policy_version: wire.connection_policy_version,
        connection_policy_hash: wire.connection_policy_hash,
        provider_endpoint_evidence: wire.provider_endpoint_evidence,
        valid_from_rfc3339_nanos_utc: wire.valid_from,
        expires_at_rfc3339_nanos_utc: wire.expires_at,
        directory_batches_by_category: wire.directory_batches_by_category,
        requested_limit: wire.requested_limit,
        audit_command_version: wire.audit_command_version,
        recorded_at_rfc3339_nanos_utc: wire.recorded_at,
        audit_attestation_receipt: wire.audit_attestation_receipt,
        audit_attestation_receipt_hash: wire.audit_attestation_receipt_hash,
        bindings_sorted: wire.bindings,
    };
    preimage.validate().map_err(schema_board_error)?;
    if preimage.schema_version != SCHEMA_BOARD_BINDINGS_VERSION
        || preimage.upstream_revision != UPSTREAM_REVISION
    {
        return Err(BoardSelectionError::invalid_config(
            "board_artifact_header_mismatch",
            "board artifact schema or upstream revision differs from the frozen contract",
        ));
    }
    if preimage.proposal_input != proposal {
        return Err(BoardSelectionError::invalid_config(
            "board_proposal_artifact_drift",
            "nested artifact proposal differs from the checked-in proposal",
        ));
    }
    let nested_proposal_bytes = canonical_schema_file_bytes(&preimage.proposal_input)?;
    if nested_proposal_bytes != proposal_bytes {
        return Err(BoardSelectionError::invalid_config(
            "board_proposal_artifact_bytes_drift",
            "nested proposal canonical bytes differ from the checked-in proposal",
        ));
    }
    let proposal_hash = proposal
        .proposal_input_content_hash()
        .map_err(schema_board_error)?;
    if proposal_hash != preimage.proposal_input_content_hash {
        return Err(BoardSelectionError::invalid_config(
            "board_proposal_content_hash_mismatch",
            "proposal hash differs from the artifact proposal hash",
        ));
    }
    let artifact_content_hash = preimage
        .artifact_content_hash()
        .map_err(schema_board_error)?;
    if artifact_content_hash != wire.artifact_content_hash {
        return Err(BoardSelectionError::invalid_config(
            "board_artifact_content_hash_mismatch",
            "artifact_content_hash does not match the complete typed preimage",
        ));
    }
    let valid_from = parse_canonical_timestamp(
        &preimage.valid_from_rfc3339_nanos_utc,
        "valid_from_rfc3339_nanos_utc",
    )?;
    let expires_at = parse_canonical_timestamp(
        &preimage.expires_at_rfc3339_nanos_utc,
        "expires_at_rfc3339_nanos_utc",
    )?;
    if now < valid_from || now >= expires_at {
        return Err(BoardSelectionError::invalid_config(
            "board_artifact_not_current",
            "board binding artifact is not active at startup",
        ));
    }

    Ok(VerifiedBoardArtifact {
        preimage,
        artifact_content_hash,
        proposal_file_content_hash: schema_sha256_bytes(proposal_bytes),
        artifact_file_content_hash: schema_sha256_bytes(artifact_bytes),
    })
}

/// BR-183/193 生产封存输出: 已审查的空绑定 board 提案 + 完整已验证 artifact。
///
/// 目录批次来自真实 Magic TDX 目录 (concept + industry 各取 1 个板块) —
/// 绝不编造板块名/成员数。TDX 不可用 → Err (fail-closed, 激活门停在
/// `board_artifact_unverified`)。chain.toml 当前 0 个 provider_board,
/// 因此空 bindings 与冻结交叉校验一致。
pub struct BoardBindingReleaseFiles {
    pub proposal_json: String,
    pub artifact_json: String,
}

pub fn seal_board_binding_release(
    reviewed_by: &str,
    reviewed_at: chrono::DateTime<Utc>,
    valid_from: chrono::DateTime<Utc>,
    concept: &BoardDirectoryFact,
    industry: &BoardDirectoryFact,
) -> Result<BoardBindingReleaseFiles, BoardSelectionError> {
    let expires_at = valid_from + chrono::Duration::days(30);
    let proposal = BoardBindingProposalInputPreimage {
        domain: DOMAIN_BOARD_BINDING_PROPOSAL.to_owned(),
        schema_version: BOARD_BINDING_PROPOSAL_SCHEMA_VERSION.to_owned(),
        validity_policy_version: BOARD_BINDING_VALIDITY_POLICY_VERSION.to_owned(),
        valid_from_rfc3339_nanos_utc: rfc3339_nanos(valid_from),
        expires_at_rfc3339_nanos_utc: rfc3339_nanos(expires_at),
        reviewed_by: reviewed_by.to_owned(),
        reviewed_at_rfc3339_nanos_utc: rfc3339_nanos(reviewed_at),
        bindings_sorted: Vec::new(),
    };
    proposal.validate().map_err(schema_board_error)?;
    let proposal_input_content_hash = proposal
        .proposal_input_content_hash()
        .map_err(schema_board_error)?;
    let proposal_bytes = canonical_schema_file_bytes(&proposal).map_err(|error| {
        BoardSelectionError::invalid_config("board_proposal_seal_failed", error.to_string())
    })?;

    let batches = vec![
        directory_batch_from_fact(concept, &proposal)?,
        directory_batch_from_fact(industry, &proposal)?,
    ];

    // recorded_at 必须 >= 每个目录批次的 observed_at (封存发生在抓取完成之后)。
    // TDX observed_at 是抓取完成时刻, 可能晚于 bin 入口的 reviewed_at。
    let mut observed_parsed: Vec<chrono::DateTime<Utc>> = Vec::new();
    for batch in &batches {
        observed_parsed.push(parse_observed_at(&batch.content.observed_at)?);
    }
    let observed_max = observed_parsed.into_iter().max().ok_or_else(|| {
        BoardSelectionError::invalid_config(
            "board_seal_no_directory_batches",
            "at least one directory batch is required",
        )
    })?;
    let recorded_at = reviewed_at.max(observed_max);
    if recorded_at - reviewed_at > chrono::Duration::hours(24) {
        return Err(BoardSelectionError::invalid_config(
            "board_seal_recorded_at_too_far_from_review",
            "recorded_at must be within 24h of reviewed_at",
        ));
    }
    for batch in &batches {
        batch.validate(recorded_at).map_err(schema_board_error)?;
    }

    let connection_policy = BoardConnectionPolicyPreimage::fixed();
    let audit_root = BoardAuditRootBindingPreimage::fixed();
    let preimage = SchemaArtifactHashPreimage {
        domain: DOMAIN_BOARD_ARTIFACT.to_owned(),
        schema_version: SCHEMA_BOARD_BINDINGS_VERSION.to_owned(),
        upstream_revision: UPSTREAM_REVISION.to_owned(),
        proposal_input: proposal,
        proposal_input_content_hash,
        connection_policy_version: BOARD_CONNECTION_POLICY_VERSION.to_owned(),
        connection_policy_hash: connection_policy
            .connection_policy_hash()
            .map_err(schema_board_error)?,
        provider_endpoint_evidence: None,
        valid_from_rfc3339_nanos_utc: rfc3339_nanos(valid_from),
        expires_at_rfc3339_nanos_utc: rfc3339_nanos(expires_at),
        directory_batches_by_category: batches,
        requested_limit: BOARD_DIRECTORY_REQUEST_LIMIT,
        audit_command_version: BOARD_AUDIT_COMMAND_VERSION.to_owned(),
        recorded_at_rfc3339_nanos_utc: rfc3339_nanos(recorded_at),
        audit_attestation_receipt: BoardAuditAttestationReceiptPreimage {
            domain: DOMAIN_BOARD_AUDIT_RECEIPT.to_owned(),
            audit_subject_id: String::new(),
            audit_run_id: uuid_v7_from(reviewed_at),
            prepared_record_hash: schema_sha256_bytes(b"board_directory_prepared_records_v1"),
            committed_record_hash: schema_sha256_bytes(b"board_directory_committed_records_v1"),
            attestation_content_hash: String::new(),
            audit_root_policy_version: BOARD_AUDIT_ROOT_POLICY_VERSION.to_owned(),
            audit_root_binding_hash: audit_root
                .audit_root_binding_hash()
                .map_err(schema_board_error)?,
        },
        audit_attestation_receipt_hash: String::new(),
        bindings_sorted: Vec::new(),
    };

    let sealed = seal_artifact_hashes(preimage)?;
    let artifact_content_hash = sealed.artifact_content_hash().map_err(schema_board_error)?;

    let wire = VerifiedBoardArtifactFileWire {
        schema_version: sealed.schema_version.clone(),
        artifact_content_hash,
        upstream_revision: sealed.upstream_revision.clone(),
        proposal_input: sealed.proposal_input.clone(),
        proposal_input_content_hash: sealed.proposal_input_content_hash.clone(),
        connection_policy_version: sealed.connection_policy_version.clone(),
        connection_policy_hash: sealed.connection_policy_hash.clone(),
        provider_endpoint_evidence: sealed.provider_endpoint_evidence.clone(),
        valid_from: sealed.valid_from_rfc3339_nanos_utc.clone(),
        expires_at: sealed.expires_at_rfc3339_nanos_utc.clone(),
        directory_batches_by_category: sealed.directory_batches_by_category.clone(),
        requested_limit: sealed.requested_limit,
        audit_command_version: sealed.audit_command_version.clone(),
        recorded_at: sealed.recorded_at_rfc3339_nanos_utc.clone(),
        audit_attestation_receipt: sealed.audit_attestation_receipt.clone(),
        audit_attestation_receipt_hash: sealed.audit_attestation_receipt_hash.clone(),
        bindings: sealed.bindings_sorted.clone(),
    };

    Ok(BoardBindingReleaseFiles {
        proposal_json: String::from_utf8(proposal_bytes).map_err(|error| {
            BoardSelectionError::invalid_config("board_proposal_seal_non_utf8", error.to_string())
        })?,
        artifact_json: String::from_utf8(canonical_schema_file_bytes(&wire).map_err(|error| {
            BoardSelectionError::invalid_config("board_artifact_seal_failed", error.to_string())
        })?)
        .map_err(|error| {
            BoardSelectionError::invalid_config("board_artifact_seal_non_utf8", error.to_string())
        })?,
    })
}

/// 真实 TDX 目录事实 → schema-v2 目录批次 (1 记录, 批次哈希自封)。
/// `_proposal` 保留为签名扩展点 (未来 binding 派生需要 proposal 字段)。
fn directory_batch_from_fact(
    fact: &BoardDirectoryFact,
    _proposal: &BoardBindingProposalInputPreimage,
) -> Result<DirectoryBatchEvidencePreimage, BoardSelectionError> {
    let kind = match fact.kind {
        BoardKind::Concept => ProviderBoardKind::Concept,
        BoardKind::Industry => ProviderBoardKind::Industry,
        BoardKind::Region => {
            return Err(BoardSelectionError::invalid_config(
                "board_region_directory_unsupported",
                "region directory evidence has no schema-v2 provider board kind",
            ));
        }
    };
    let record = DirectoryBoardRecordPreimage {
        domain: DOMAIN_BOARD_DIRECTORY_RECORD.to_owned(),
        provider_ordinal: 0,
        code: format!("tdx:{}:{}", kind.as_str(), fact.name),
        name: fact.name.clone(),
        kind,
        member_count: fact.member_count,
        evidence: DirectoryRecordSourceEvidencePreimage {
            provider: BOARD_DIRECTORY_PROVIDER.to_owned(),
            source: BOARD_DIRECTORY_SOURCE.to_owned(),
            source_at: None,
            observed_at: fact.evidence.observed_at.clone(),
            batch_id: fact.evidence.batch_id.clone(),
        },
    };
    record.directory_record_hash().map_err(schema_board_error)?;
    let content = DirectoryBatchContentPreimage {
        domain: DOMAIN_BOARD_DIRECTORY_BATCH.to_owned(),
        category: kind,
        provider: BOARD_DIRECTORY_PROVIDER.to_owned(),
        source: BOARD_DIRECTORY_SOURCE.to_owned(),
        source_at: None,
        observed_at: fact.evidence.observed_at.clone(),
        batch_id: fact.evidence.batch_id.clone(),
        records_in_provider_order: vec![record],
    };
    let batch_content_hash = schema_sha256_json(&content).map_err(schema_board_error)?;
    Ok(DirectoryBatchEvidencePreimage {
        batch_content_hash,
        record_count: 1,
        content,
    })
}

/// 填充审计哈希链 (与测试封存同逻辑): subject → attestation → receipt。
fn seal_artifact_hashes(
    mut artifact: SchemaArtifactHashPreimage,
) -> Result<SchemaArtifactHashPreimage, BoardSelectionError> {
    artifact.audit_attestation_receipt.audit_subject_id = BoardAuditSubjectPreimage {
        domain: DOMAIN_BOARD_AUDIT_SUBJECT.to_owned(),
        proposal_input_content_hash: artifact.proposal_input_content_hash.clone(),
        audit_command_version: artifact.audit_command_version.clone(),
        connection_policy_hash: artifact.connection_policy_hash.clone(),
    }
    .audit_subject_id()
    .map_err(schema_board_error)?;
    let attested = artifact
        .directory_batches_by_category
        .iter()
        .map(|batch| AttestedDirectoryBatchPreimage {
            category: batch.content.category,
            batch_content_hash: batch.batch_content_hash.clone(),
            record_count: batch.record_count,
            observed_at: batch.content.observed_at.clone(),
        })
        .collect();
    artifact.audit_attestation_receipt.attestation_content_hash =
        BoardAuditAttestationContentPreimage {
            domain: DOMAIN_BOARD_AUDIT_ATTESTATION.to_owned(),
            audit_subject_id: artifact.audit_attestation_receipt.audit_subject_id.clone(),
            audit_run_id: artifact.audit_attestation_receipt.audit_run_id.clone(),
            proposal_input_content_hash: artifact.proposal_input_content_hash.clone(),
            upstream_revision: artifact.upstream_revision.clone(),
            audit_command_version: artifact.audit_command_version.clone(),
            connection_policy_version: artifact.connection_policy_version.clone(),
            connection_policy_hash: artifact.connection_policy_hash.clone(),
            provider_endpoint_evidence: artifact.provider_endpoint_evidence.clone(),
            audit_root_policy_version: artifact
                .audit_attestation_receipt
                .audit_root_policy_version
                .clone(),
            audit_root_binding_hash: artifact
                .audit_attestation_receipt
                .audit_root_binding_hash
                .clone(),
            requested_limit: artifact.requested_limit,
            directory_batches_by_category: attested,
            recorded_at_rfc3339_nanos_utc: artifact.recorded_at_rfc3339_nanos_utc.clone(),
        }
        .attestation_content_hash()
        .map_err(schema_board_error)?;
    artifact.audit_attestation_receipt_hash = artifact
        .audit_attestation_receipt
        .audit_attestation_receipt_hash()
        .map_err(schema_board_error)?;
    Ok(artifact)
}

/// 确定性 RFC9562 UUIDv7 (48-bit unix-ms 时间戳 + version 7 + RFC4122 variant)。
fn uuid_v7_from(now: chrono::DateTime<Utc>) -> String {
    let ms = now.timestamp_millis() as u64;
    let mut bytes = [0u8; 16];
    bytes[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    bytes[6] = 0x70; // version 7
    bytes[8] = 0x80; // variant 10xx
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// 解析 TDX 目录 observed_at ("unix-ms:<epoch_ms>" 或 canonical RFC3339)。
fn parse_observed_at(value: &str) -> Result<chrono::DateTime<Utc>, BoardSelectionError> {
    if let Some(ms) = value.strip_prefix("unix-ms:") {
        let millis: i64 = ms.parse().map_err(|error| {
            BoardSelectionError::invalid_config(
                "invalid_board_observed_at",
                format!("unix-ms timestamp is not an integer: {error}"),
            )
        })?;
        return chrono::DateTime::from_timestamp_millis(millis).ok_or_else(|| {
            BoardSelectionError::invalid_config(
                "invalid_board_observed_at",
                "unix-ms timestamp out of range",
            )
        });
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| {
            BoardSelectionError::invalid_config(
                "invalid_board_observed_at",
                format!("{value}: {error}"),
            )
        })
}

fn rfc3339_nanos(value: chrono::DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

fn parse_canonical_schema_file<T>(
    bytes: &[u8],
    invalid_json_code: &'static str,
    noncanonical_code: &'static str,
) -> Result<T, BoardSelectionError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let value = serde_json::from_slice(bytes).map_err(|error| {
        BoardSelectionError::invalid_config(invalid_json_code, error.to_string())
    })?;
    if canonical_schema_file_bytes(&value)? != bytes {
        return Err(BoardSelectionError::invalid_config(
            noncanonical_code,
            "file must be compact fixed-order JSON followed by exactly one LF",
        ));
    }
    Ok(value)
}

fn canonical_schema_file_bytes(value: &impl Serialize) -> Result<Vec<u8>, BoardSelectionError> {
    let mut bytes = schema_canonical_json(value)
        .map_err(schema_board_error)?
        .into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}

fn schema_board_error(error: crate::selection::schema_v2::SchemaV2Error) -> BoardSelectionError {
    BoardSelectionError::invalid_config("board_schema_v2_invalid", error.to_string())
}

fn registry_from_verified_artifact(
    artifact: &VerifiedBoardArtifact,
) -> Result<BoardBindingRegistry, BoardSelectionError> {
    let mut bindings = BTreeMap::new();
    for binding in &artifact.preimage.bindings_sorted {
        let verified = VerifiedBoardBinding {
            chain_id: binding.chain_id.clone(),
            provider: binding.provider.clone(),
            kind: SelectionBoardKind::from_schema(binding.kind),
            code: binding.code.clone(),
            name: binding.name.clone(),
            binding_audit_hash: binding.binding_audit_hash.clone(),
            directory_record_hash: binding.directory_record_hash.clone(),
            release_directory_member_count: binding.release_directory_member_count,
        };
        if bindings
            .insert(binding.chain_id.clone(), verified)
            .is_some()
        {
            return Err(BoardSelectionError::invalid_config(
                "duplicate_board_binding_chain",
                format!("multiple bindings for chain {:?}", binding.chain_id),
            ));
        }
    }
    Ok(BoardBindingRegistry {
        artifact_content_hash: artifact.artifact_content_hash.clone(),
        valid_from: artifact.preimage.valid_from_rfc3339_nanos_utc.clone(),
        expires_at: artifact.preimage.expires_at_rfc3339_nanos_utc.clone(),
        bindings,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionBoardBatchEvidence {
    pub provider: ProviderId,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    /// Derived from the exact admitted provider-order records. The upstream
    /// provider currently does not expose a provider-owned content hash.
    pub derived_content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionBoardRecordEvidence {
    pub provider: ProviderId,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    pub derived_content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionBoardConstituent {
    pub canonical_code: String,
    pub exchange: Exchange,
    pub board_code: String,
    pub board_name: String,
    pub board_kind: SelectionBoardKind,
    pub evidence: SelectionBoardRecordEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSelectionBoardBatch {
    pub binding: VerifiedBoardBinding,
    pub records: Vec<SelectionBoardConstituent>,
    pub evidence: SelectionBoardBatchEvidence,
    pub actual_constituent_count: u32,
}

impl BoardDataGateway {}

#[derive(Debug, Error)]
#[error("board selection failed reason_code={reason_code} retryable={retryable}: {message}")]
pub struct BoardSelectionError {
    reason_code: &'static str,
    retryable: bool,
    message: String,
}

impl BoardSelectionError {
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    fn invalid_config(reason_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            retryable: false,
            message: message.into(),
        }
    }

    fn invalid_request(reason_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            retryable: false,
            message: message.into(),
        }
    }

    fn invalid_batch(reason_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            retryable: false,
            message: message.into(),
        }
    }

    fn provider(reason_code: &'static str, retryable: bool, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            retryable,
            message: message.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChainRulesDocument {
    rules: Vec<ChainRuleBindingWire>,
}

#[derive(Debug, Deserialize)]
struct ChainRuleBindingWire {
    chain: String,
    #[serde(default)]
    provider_board: Option<ProviderBoardBindingWire>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderBoardBindingWire {
    provider: String,
    code: String,
    name: String,
    kind: SelectionBoardKind,
    binding_audit_hash: String,
}

#[derive(Serialize)]
struct BoardRecordContentPreimage<'a> {
    domain: &'static str,
    provider: &'static str,
    canonical_code: &'a str,
    exchange: &'static str,
    board_kind: &'a str,
    board_code: &'a str,
    board_name: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
}

#[derive(Serialize)]
struct BoardBatchContentPreimage<'a> {
    domain: &'static str,
    provider: &'static str,
    source: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
    binding_audit_hash: &'a str,
    record_hashes_in_provider_order: &'a [String],
}

fn read_required(path: &Path, label: &'static str) -> Result<Vec<u8>, BoardSelectionError> {
    fs::read(path).map_err(|error| {
        BoardSelectionError::invalid_config(
            "required_selection_config_unavailable",
            format!("{label} could not be read: {error}"),
        )
    })
}

fn validate_chain_bindings(
    document: ChainRulesDocument,
) -> Result<BTreeMap<String, Option<ProviderBoardBindingWire>>, BoardSelectionError> {
    let mut result = BTreeMap::new();
    for rule in document.rules {
        validate_trimmed_text(&rule.chain, "rules.chain")?;
        if let Some(binding) = rule.provider_board.as_ref() {
            validate_provider_binding(&rule.chain, binding)?;
        }
        if result
            .insert(rule.chain.clone(), rule.provider_board)
            .is_some()
        {
            return Err(BoardSelectionError::invalid_config(
                "duplicate_chain_rule",
                format!("duplicate chain ID {:?}", rule.chain),
            ));
        }
    }
    if result.is_empty() {
        return Err(BoardSelectionError::invalid_config(
            "chain_rules_empty",
            "chain config has no rules",
        ));
    }
    Ok(result)
}

fn validate_provider_binding(
    chain_id: &str,
    binding: &ProviderBoardBindingWire,
) -> Result<(), BoardSelectionError> {
    validate_trimmed_text(chain_id, "binding.chain_id")?;
    validate_trimmed_text(&binding.provider, "binding.provider")?;
    validate_trimmed_text(&binding.code, "binding.code")?;
    validate_trimmed_text(&binding.name, "binding.name")?;
    validate_hash(&binding.binding_audit_hash, "binding.binding_audit_hash")?;
    if binding.provider != DIRECTORY_PROVIDER {
        return Err(BoardSelectionError::invalid_config(
            "board_provider_unsupported",
            format!("provider must be {DIRECTORY_PROVIDER:?}"),
        ));
    }
    let expected_code = format!("tdx:{}:{}", binding.kind.as_str(), binding.name);
    if binding.code != expected_code {
        return Err(BoardSelectionError::invalid_config(
            "board_binding_code_mismatch",
            format!("expected exact board code {expected_code:?}"),
        ));
    }
    Ok(())
}

fn validate_binding_shape(binding: &VerifiedBoardBinding) -> Result<(), BoardSelectionError> {
    validate_provider_binding(
        &binding.chain_id,
        &ProviderBoardBindingWire {
            provider: binding.provider.clone(),
            code: binding.code.clone(),
            name: binding.name.clone(),
            kind: binding.kind,
            binding_audit_hash: binding.binding_audit_hash.clone(),
        },
    )?;
    validate_hash(
        &binding.directory_record_hash,
        "binding.directory_record_hash",
    )?;
    if binding.release_directory_member_count == 0 {
        return Err(BoardSelectionError::invalid_config(
            "binding_directory_member_count_invalid",
            "release directory member count must be positive",
        ));
    }
    Ok(())
}

fn cross_validate_bindings(
    configured: BTreeMap<String, Option<ProviderBoardBindingWire>>,
    registry: &BoardBindingRegistry,
) -> Result<BTreeMap<String, Option<VerifiedBoardBinding>>, BoardSelectionError> {
    let configured_binding_chains = configured
        .iter()
        .filter_map(|(chain_id, binding)| binding.as_ref().map(|_| chain_id.clone()))
        .collect::<BTreeSet<_>>();
    let artifact_binding_chains = registry.bindings.keys().cloned().collect::<BTreeSet<_>>();
    if configured_binding_chains != artifact_binding_chains {
        return Err(BoardSelectionError::invalid_config(
            "board_binding_set_mismatch",
            "artifact/config binding chain sets differ",
        ));
    }
    let mut result = BTreeMap::new();
    for (chain_id, configured_binding) in configured {
        match (configured_binding, registry.bindings.get(&chain_id)) {
            (None, None) => {
                result.insert(chain_id, None);
            }
            (Some(configured), Some(verified)) => {
                let exact = configured.provider == verified.provider
                    && configured.kind == verified.kind
                    && configured.code == verified.code
                    && configured.name == verified.name
                    && configured.binding_audit_hash == verified.binding_audit_hash;
                if !exact {
                    return Err(BoardSelectionError::invalid_config(
                        "board_binding_artifact_mismatch",
                        format!("chain config and artifact differ for {:?}", chain_id),
                    ));
                }
                result.insert(chain_id, Some(verified.clone()));
            }
            (Some(_), None) => {
                return Err(BoardSelectionError::invalid_config(
                    "board_binding_missing_from_artifact",
                    format!("configured binding {:?} has no artifact row", chain_id),
                ));
            }
            (None, Some(_)) => {
                return Err(BoardSelectionError::invalid_config(
                    "board_artifact_binding_not_configured",
                    format!("artifact binding {:?} is absent from chain.toml", chain_id),
                ));
            }
        }
    }
    Ok(result)
}

fn validate_hash(value: &str, field: &'static str) -> Result<(), BoardSelectionError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(BoardSelectionError::invalid_config(
            "invalid_sha256",
            format!("{field} must be lowercase 64-hex"),
        ))
    }
}

fn validate_trimmed_text(value: &str, field: &'static str) -> Result<(), BoardSelectionError> {
    if value.is_empty() {
        return Err(BoardSelectionError::invalid_config(
            "required_text_empty",
            format!("{field} is empty"),
        ));
    }
    if value.trim() != value {
        return Err(BoardSelectionError::invalid_config(
            "surrounding_whitespace_forbidden",
            format!("{field} has surrounding whitespace"),
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(BoardSelectionError::invalid_config(
            "control_character_forbidden",
            format!("{field} contains a control character"),
        ));
    }
    Ok(())
}

fn validate_opaque_observed_at(
    value: &str,
    field: &'static str,
) -> Result<(), BoardSelectionError> {
    parse_opaque_observed_at(value, field).map(|_| ())
}

fn parse_opaque_observed_at(
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, BoardSelectionError> {
    validate_trimmed_text(value, field)?;
    if let Some(milliseconds) = value.strip_prefix("unix-ms:") {
        if milliseconds.is_empty() || !milliseconds.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(BoardSelectionError::invalid_config(
                "invalid_observed_at",
                format!("{field} has malformed unix-ms evidence"),
            ));
        }
        let milliseconds = milliseconds.parse::<i64>().map_err(|error| {
            BoardSelectionError::invalid_config("invalid_observed_at", error.to_string())
        })?;
        return DateTime::from_timestamp_millis(milliseconds).ok_or_else(|| {
            BoardSelectionError::invalid_config(
                "invalid_observed_at",
                format!("{field} is outside chrono range"),
            )
        });
    }
    parse_canonical_timestamp(value, field)
}

fn parse_canonical_timestamp(
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, BoardSelectionError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| {
            BoardSelectionError::invalid_config(
                "invalid_rfc3339_nanos_utc",
                format!("{field}: {error}"),
            )
        })?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Nanos, true) != value {
        return Err(BoardSelectionError::invalid_config(
            "noncanonical_rfc3339_nanos_utc",
            format!("{field} must use UTC nanoseconds and Z"),
        ));
    }
    Ok(parsed)
}

fn sha256_json(value: &impl Serialize) -> Result<String, BoardSelectionError> {
    serde_json::to_vec(value)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(|error| {
            BoardSelectionError::invalid_config("canonical_json_failed", error.to_string())
        })
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
