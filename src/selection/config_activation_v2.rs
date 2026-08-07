//! BR-174/BR-177 deterministic, storage-free config-activation preparation.
//!
//! This module reads exact checked-in bytes and prepares the immutable schema-v2
//! activation payload. It never writes configuration, selects a last-known-good
//! snapshot, or manufactures board evidence.
#![allow(
    dead_code,
    reason = "BR-183 keeps the unreleased selection-v2 activation capability disabled until its release evidence closes"
)]

use crate::data_gateway::board::{
    load_verified_board_artifact_from_root, BoardSelectionError, BOARD_BINDINGS_PATH,
    BOARD_BINDING_PROPOSAL_PATH,
};
use crate::news::aggregator::raw_v2::{registered_global_news_feeds, MAGIC_MARKET_DATA_REVISION};
use crate::selection::admission::ADMISSION_VERSION;
use crate::selection::features::FEATURE_VERSION;
use crate::selection::schema_v2::{
    canonical_json, sha256_bytes, sha256_json, ArtifactHashPreimage,
    ChainRuleSnapshotEntryPreimage, ChainRulesSnapshotPreimage, ConfigActivationContentPreimage,
    ConfigActivationStageInputPreimage, ExecutableInputFilePreimage, ExecutableRevisionPreimage,
    LegacyCutoverSnapshotPreimage, ProviderBoardKind, RegisteredFeedConfigurationPreimage,
    RegisteredFeedEntryPreimage, RegisteredFeedIdentityPreimage, RegisteredFeedSnapshotPreimage,
    RunLogicalSubjectPreimage, RunPayloadPreimage, SelectionConfigSnapshotPreimage,
    SelectionRecoveryEnvelopeRowContentPreimage, SubjectKind, BOARD_DIRECTORY_PROVIDER,
    DOMAIN_CHAIN_RULES_SNAPSHOT, DOMAIN_CONFIG_ACTIVATION, DOMAIN_CONFIG_ACTIVATION_PAYLOAD,
    DOMAIN_CONFIG_ACTIVATION_STAGE, DOMAIN_EXECUTABLE_REVISION, DOMAIN_RECOVERY_ENVELOPE_ROW,
    DOMAIN_REGISTERED_FEED_CONFIG, DOMAIN_REGISTERED_FEED_IDENTITY,
    DOMAIN_REGISTERED_FEED_SNAPSHOT, DOMAIN_RUN_LOGICAL_SUBJECT, DOMAIN_SELECTION_CONFIG_SNAPSHOT,
    UPSTREAM_REVISION,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const CHAIN_CONFIG_RELATIVE_PATH: &str = "config/chain.toml";
pub const BOARD_ARTIFACT_RELATIVE_PATH: &str = BOARD_BINDINGS_PATH;
pub const ACTIVATION_FILE_RELATIVE_PATH: &str = "config/selection/selection_activation.v1.json";
pub const CONFIG_SNAPSHOT_SCHEMA_VERSION: &str = "selection-config-snapshot-v1";
pub const ACTIVATION_FILE_SCHEMA_VERSION: &str = "selection-config-activation-v1";
pub const EXECUTABLE_INPUT_MANIFEST_VERSION: &str = "selection-executable-inputs-v1";
pub const RELATION_SCHEMA_VERSION: &str = "event-relation-v2";
pub const INGRESS_GATE_VERSION: &str = "br137-ingress-v1";
pub const INGRESS_FRESHNESS_MAX_AGE_SECS: u64 = 86_400;
pub const INGRESS_FUTURE_TOLERANCE_SECS: u64 = 0;

const CONFIG_ACTIVATION_PAYLOAD_SCHEMA: &str = "config-activation-stage-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigActivationGateContract {
    ingress_gate_version: String,
    freshness_max_age_secs: u64,
    future_tolerance_secs: u64,
    relation_schema_version: String,
    feature_version: String,
    admission_version: String,
}

impl ConfigActivationGateContract {
    fn checked_in() -> Self {
        Self {
            ingress_gate_version: INGRESS_GATE_VERSION.to_owned(),
            freshness_max_age_secs: INGRESS_FRESHNESS_MAX_AGE_SECS,
            future_tolerance_secs: INGRESS_FUTURE_TOLERANCE_SECS,
            relation_schema_version: RELATION_SCHEMA_VERSION.to_owned(),
            feature_version: FEATURE_VERSION.to_owned(),
            admission_version: ADMISSION_VERSION.to_owned(),
        }
    }

    fn validate(&self) -> Result<(), ConfigActivationPreparationError> {
        let expected = Self::checked_in();
        if self != &expected {
            return Err(ConfigActivationPreparationError::new(
                "gate_contract_mismatch",
                "gate versions/thresholds differ from the checked-in contract",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConfigActivationPreparationContext {
    stage_run_id: String,
    activated_at: DateTime<Utc>,
    enveloped_at: DateTime<Utc>,
    gate_contract: ConfigActivationGateContract,
    legacy_cutover_snapshot: LegacyCutoverSnapshotPreimage,
}

impl ConfigActivationPreparationContext {
    fn checked_in(
        stage_run_id: String,
        activated_at: DateTime<Utc>,
        enveloped_at: DateTime<Utc>,
        legacy_cutover_snapshot: LegacyCutoverSnapshotPreimage,
    ) -> Self {
        Self {
            stage_run_id,
            activated_at,
            enveloped_at,
            gate_contract: ConfigActivationGateContract::checked_in(),
            legacy_cutover_snapshot,
        }
    }
}

/// Opaque config-activation preparation capability.
///
/// External crates may only move this value into the persistence owner; they
/// cannot inspect or replace its canonical staging preimages:
///
/// ```compile_fail
/// use stock_analysis::selection::config_activation_v2::PreparedConfigActivation;
///
/// fn leak_stage(value: PreparedConfigActivation) {
///     let PreparedConfigActivation { stage_input, .. } = value;
///     drop(stage_input);
/// }
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct PreparedConfigActivation {
    stage_input: ConfigActivationStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl PreparedConfigActivation {
    pub(crate) fn stage_input(&self) -> &ConfigActivationStageInputPreimage {
        &self.stage_input
    }

    pub(crate) fn run_payload(&self) -> &RunPayloadPreimage {
        &self.run_payload
    }

    pub(crate) fn recovery_envelope(&self) -> &SelectionRecoveryEnvelopeRowContentPreimage {
        &self.recovery_envelope
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableRevisionSnapshot {
    pub preimage: ExecutableRevisionPreimage,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigActivationPreparationError {
    pub code: &'static str,
    pub detail: String,
}

impl ConfigActivationPreparationError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ConfigActivationPreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for ConfigActivationPreparationError {}

impl From<crate::selection::schema_v2::SchemaV2Error> for ConfigActivationPreparationError {
    fn from(error: crate::selection::schema_v2::SchemaV2Error) -> Self {
        Self::new("schema_v2_preimage_invalid", error.to_string())
    }
}

pub(crate) fn prepare_checked_in_config_activation(
    context: ConfigActivationPreparationContext,
) -> Result<PreparedConfigActivation, ConfigActivationPreparationError> {
    prepare_config_activation_from_root(Path::new(env!("CARGO_MANIFEST_DIR")), context)
}

/// BR-183/193 release-gate materials: stages 1-4 of the full activation
/// preparation (chain snapshot + board artifact + config hash), without the
/// legacy-cutover snapshot and envelope persistence that remain gated behind
/// BR-180. Consumed by `activation_gate` to decide the production
/// selection-v2 capability verdict.
#[derive(Debug)]
pub struct PreparedActivationMaterials {
    pub config_hash: String,
    pub config_snapshot_json_hash: String,
    pub board_artifact_valid_from: String,
    pub board_artifact_expires_at: String,
}

/// Release-gate materials preparation, public for the
/// `selection_activation_prepare` binary (read-only; never persists).
///
/// Stages 1-3 only: gate contract + chain/board snapshot → config hash and
/// board release window, without requiring an activation file. Used to
/// *produce* the activation file (a file cannot require itself to exist).
pub fn prepare_activation_config_hash(
    repository_root: impl AsRef<Path>,
    activated_at: DateTime<Utc>,
) -> Result<PreparedActivationMaterials, ConfigActivationPreparationError> {
    let contract = ConfigActivationGateContract::checked_in();
    contract.validate()?;
    let repository_root = validate_repository_root(repository_root.as_ref())?;
    let snapshot = prepare_snapshot(&repository_root, activated_at, &contract)?;
    Ok(PreparedActivationMaterials {
        config_hash: snapshot.config_hash,
        config_snapshot_json_hash: snapshot.config_snapshot_json_hash,
        board_artifact_valid_from: snapshot.board_artifact_valid_from,
        board_artifact_expires_at: snapshot.board_artifact_expires_at,
    })
}

/// Full gate materials: stages 1-5 including the activation file's existence,
/// config-hash match and chronology (used by the production activation gate).
pub fn prepare_activation_materials(
    repository_root: impl AsRef<Path>,
    activated_at: DateTime<Utc>,
) -> Result<PreparedActivationMaterials, ConfigActivationPreparationError> {
    let contract = ConfigActivationGateContract::checked_in();
    contract.validate()?;
    let repository_root = validate_repository_root(repository_root.as_ref())?;
    let snapshot = prepare_snapshot(&repository_root, activated_at, &contract)?;

    // Stage 4-5: activation file must exist, match the exact checked-in
    // config hash, and satisfy the frozen chronology against the board
    // artifact release window.
    let activation_path = repository_root.join(ACTIVATION_FILE_RELATIVE_PATH);
    let activation_bytes = read_required_file(&activation_path, "activation_file")?;
    let activation_file = parse_activation_file(&activation_bytes)?;
    if activation_file.expected_config_hash != snapshot.config_hash {
        return Err(ConfigActivationPreparationError::new(
            "activation_expected_config_hash_mismatch",
            "activation file expected_config_hash does not match exact checked-in inputs",
        ));
    }
    let effective_from =
        parse_canonical_timestamp(&activation_file.effective_from, "effective_from")?;
    let reviewed_at = parse_canonical_timestamp(&activation_file.reviewed_at, "reviewed_at")?;
    let artifact_valid_from =
        parse_canonical_timestamp(&snapshot.board_artifact_valid_from, "artifact_valid_from")?;
    let artifact_expires_at =
        parse_canonical_timestamp(&snapshot.board_artifact_expires_at, "artifact_expires_at")?;
    if reviewed_at > activated_at
        || effective_from < activated_at
        || effective_from < reviewed_at
        || effective_from < artifact_valid_from
        || effective_from >= artifact_expires_at
    {
        return Err(ConfigActivationPreparationError::new(
            "activation_chronology_invalid",
            "reviewed_at <= activated_at <= effective_from and artifact validity at effective_from are required",
        ));
    }

    Ok(PreparedActivationMaterials {
        config_hash: snapshot.config_hash,
        config_snapshot_json_hash: snapshot.config_snapshot_json_hash,
        board_artifact_valid_from: snapshot.board_artifact_valid_from,
        board_artifact_expires_at: snapshot.board_artifact_expires_at,
    })
}

fn prepare_config_activation_from_root(
    repository_root: impl AsRef<Path>,
    context: ConfigActivationPreparationContext,
) -> Result<PreparedConfigActivation, ConfigActivationPreparationError> {
    validate_context(&context)?;
    let repository_root = validate_repository_root(repository_root.as_ref())?;
    let prepared_snapshot = prepare_snapshot(
        &repository_root,
        context.activated_at,
        &context.gate_contract,
    )?;

    let activation_path = repository_root.join(ACTIVATION_FILE_RELATIVE_PATH);
    let activation_bytes = read_required_file(&activation_path, "activation_file")?;
    let activation_file = parse_activation_file(&activation_bytes)?;
    if activation_file.expected_config_hash != prepared_snapshot.config_hash {
        return Err(ConfigActivationPreparationError::new(
            "activation_expected_config_hash_mismatch",
            "activation file expected_config_hash does not match exact checked-in inputs",
        ));
    }

    let activated_at = rfc3339_nanos(context.activated_at);
    let enveloped_at = rfc3339_nanos(context.enveloped_at);
    let effective_from =
        parse_canonical_timestamp(&activation_file.effective_from, "effective_from")?;
    let reviewed_at = parse_canonical_timestamp(&activation_file.reviewed_at, "reviewed_at")?;
    let artifact_valid_from = parse_canonical_timestamp(
        &prepared_snapshot.board_artifact_valid_from,
        "artifact_valid_from",
    )?;
    let artifact_expires_at = parse_canonical_timestamp(
        &prepared_snapshot.board_artifact_expires_at,
        "artifact_expires_at",
    )?;
    if reviewed_at > context.activated_at
        || effective_from < context.activated_at
        || effective_from < reviewed_at
        || effective_from < artifact_valid_from
        || effective_from >= artifact_expires_at
    {
        return Err(ConfigActivationPreparationError::new(
            "activation_chronology_invalid",
            "reviewed_at <= activated_at <= effective_from and artifact validity at effective_from are required",
        ));
    }

    let activation_file_content_hash = sha256_bytes(&activation_bytes);
    let activation = ConfigActivationContentPreimage {
        domain: DOMAIN_CONFIG_ACTIVATION.to_owned(),
        config_hash: prepared_snapshot.config_hash.clone(),
        activated_at_rfc3339_nanos_utc: activated_at,
        effective_from_rfc3339_nanos_utc: activation_file.effective_from.clone(),
        activation_file_content_hash: activation_file_content_hash.clone(),
        reviewed_by: activation_file.reviewed_by,
        reviewed_at_rfc3339_nanos_utc: activation_file.reviewed_at,
        artifact_valid_from: prepared_snapshot.board_artifact_valid_from.clone(),
        artifact_expires_at: prepared_snapshot.board_artifact_expires_at.clone(),
        executable_revision: prepared_snapshot.executable_revision.hash.clone(),
    };
    let activation_content_hash = sha256_json(&activation)?;

    validate_legacy_cutover_snapshot(&context.legacy_cutover_snapshot, context.activated_at)?;
    let legacy_cutover_snapshot_hash = sha256_json(&context.legacy_cutover_snapshot)?;

    let logical_subject = RunLogicalSubjectPreimage {
        domain: DOMAIN_RUN_LOGICAL_SUBJECT.to_owned(),
        subject_kind: SubjectKind::ConfigActivation,
        source_fact_key: None,
        config_hash: Some(prepared_snapshot.config_hash.clone()),
        sample_key: None,
        outcome_phase: None,
        stored_due_date: None,
        ingress_source_batch_hash: None,
    };
    logical_subject.validate()?;
    let logical_subject_key = sha256_json(&logical_subject)?;

    let stage_input = ConfigActivationStageInputPreimage {
        domain: DOMAIN_CONFIG_ACTIVATION_STAGE.to_owned(),
        stage_run_id: context.stage_run_id.clone(),
        logical_subject_key: logical_subject_key.clone(),
        config_snapshot: prepared_snapshot.config_snapshot.clone(),
        config_snapshot_json_hash: prepared_snapshot.config_snapshot_json_hash.clone(),
        config_hash: prepared_snapshot.config_hash.clone(),
        activation: activation.clone(),
        activation_content_hash: activation_content_hash.clone(),
        legacy_cutover_snapshot: context.legacy_cutover_snapshot,
        legacy_cutover_snapshot_hash: legacy_cutover_snapshot_hash.clone(),
    };
    let payload_json = canonical_json(&stage_input)?;
    let payload_json_hash = sha256_bytes(payload_json.as_bytes());

    let run_payload = RunPayloadPreimage {
        domain: DOMAIN_CONFIG_ACTIVATION_PAYLOAD.to_owned(),
        subject_kind: SubjectKind::ConfigActivation,
        subject_id: context.stage_run_id.clone(),
        logical_subject_key: logical_subject_key.clone(),
        source_fact_key: None,
        config_activation_run_id: context.stage_run_id.clone(),
        config_hash: prepared_snapshot.config_hash.clone(),
        config_snapshot_json_hash: Some(prepared_snapshot.config_snapshot_json_hash.clone()),
        config_activation_content_hash: Some(activation_content_hash.clone()),
        config_activation_file_content_hash: Some(activation_file_content_hash.clone()),
        config_effective_from_rfc3339_nanos_utc: Some(
            activation.effective_from_rfc3339_nanos_utc.clone(),
        ),
        artifact_valid_from: Some(prepared_snapshot.board_artifact_valid_from.clone()),
        artifact_expires_at: Some(prepared_snapshot.board_artifact_expires_at.clone()),
        executable_revision: Some(prepared_snapshot.executable_revision.hash.clone()),
        legacy_cutover_snapshot_hash: Some(legacy_cutover_snapshot_hash),
        generation_market_date: None,
        aggregator_observed_at_rfc3339_nanos_utc: None,
        ingress_source_batch_content_hash: None,
        outcome_phase: None,
        stored_due_date: None,
        outcome_claim_id: None,
        planned_outcome_run_id: None,
        outcome_claim_receipt_content_hash: None,
        outcome_claim_due_binding_hash: None,
        outcome_claim_provider_request_hash: None,
        rows: Vec::new(),
    };
    let in_memory_payload_hash = sha256_json(&run_payload)?;

    let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
        domain: DOMAIN_RECOVERY_ENVELOPE_ROW.to_owned(),
        stage_run_id: context.stage_run_id.clone(),
        subject_kind: SubjectKind::ConfigActivation,
        logical_subject_key: logical_subject_key.clone(),
        payload_schema: CONFIG_ACTIVATION_PAYLOAD_SCHEMA.to_owned(),
        payload_json: payload_json.clone(),
        payload_json_hash: payload_json_hash.clone(),
        in_memory_payload_hash: in_memory_payload_hash.clone(),
        config_activation_run_id: context.stage_run_id,
        config_hash: prepared_snapshot.config_hash.clone(),
        enveloped_at,
    };
    drop(sha256_json(&recovery_envelope)?);

    Ok(PreparedConfigActivation {
        stage_input,
        run_payload,
        recovery_envelope,
    })
}

pub(crate) fn compute_executable_revision(
    repository_root: impl AsRef<Path>,
) -> Result<ExecutableRevisionSnapshot, ConfigActivationPreparationError> {
    let root = validate_repository_root(repository_root.as_ref())?;
    let relative_paths = enumerate_executable_inputs(&root)?;
    let mut files = Vec::with_capacity(relative_paths.len());
    let mut first_read_hashes = BTreeMap::new();
    for relative_path in &relative_paths {
        let absolute = root.join(relative_path);
        let bytes = read_stable_regular_file(&absolute, relative_path)?;
        let content_sha256 = sha256_bytes(&bytes);
        first_read_hashes.insert(relative_path.clone(), content_sha256.clone());
        files.push(ExecutableInputFilePreimage {
            relative_path: path_to_slash_string(relative_path)?,
            byte_len: u64::try_from(bytes.len()).map_err(|_| {
                ConfigActivationPreparationError::new(
                    "executable_input_length_overflow",
                    "input byte length does not fit u64",
                )
            })?,
            content_sha256,
        });
    }

    for relative_path in &relative_paths {
        let bytes = read_stable_regular_file(&root.join(relative_path), relative_path)?;
        let expected = first_read_hashes.get(relative_path).ok_or_else(|| {
            ConfigActivationPreparationError::new(
                "executable_input_manifest_internal_mismatch",
                "second-pass input has no first-pass hash",
            )
        })?;
        if &sha256_bytes(&bytes) != expected {
            return Err(ConfigActivationPreparationError::new(
                "executable_input_mutated_during_activation",
                format!(
                    "{} changed between activation reads",
                    relative_path.display()
                ),
            ));
        }
    }

    let preimage = ExecutableRevisionPreimage {
        domain: DOMAIN_EXECUTABLE_REVISION.to_owned(),
        input_manifest_version: EXECUTABLE_INPUT_MANIFEST_VERSION.to_owned(),
        files_sorted: files,
    };
    let hash = sha256_json(&preimage)?;
    Ok(ExecutableRevisionSnapshot { preimage, hash })
}

#[derive(Debug)]
struct PreparedSnapshot {
    config_snapshot: SelectionConfigSnapshotPreimage,
    config_snapshot_json_hash: String,
    config_hash: String,
    executable_revision: ExecutableRevisionSnapshot,
    board_artifact_valid_from: String,
    board_artifact_expires_at: String,
}

fn prepare_snapshot(
    repository_root: &Path,
    activated_at: DateTime<Utc>,
    gate_contract: &ConfigActivationGateContract,
) -> Result<PreparedSnapshot, ConfigActivationPreparationError> {
    gate_contract.validate()?;
    let chain_bytes = read_required_file(
        &repository_root.join(CHAIN_CONFIG_RELATIVE_PATH),
        "chain_config",
    )?;
    let parsed_chain = parse_chain_rules(&chain_bytes)?;
    let artifact = load_verified_board_artifact_from_root(repository_root, activated_at)
        .map_err(config_activation_board_error)?;
    cross_validate_chain_and_artifact(&parsed_chain, artifact.preimage())?;

    let executable_revision = compute_executable_revision(repository_root)?;
    validate_board_file_content_hashes(
        &executable_revision,
        artifact.proposal_file_content_hash(),
        artifact.artifact_file_content_hash(),
    )?;
    drop(prepare_registered_feed_snapshot()?);
    let chain_rules_sorted_content_hash = sha256_json(&parsed_chain.snapshot)?;
    let chain_config_bytes_hash = sha256_bytes(&chain_bytes);
    let binding_audit_hashes_sorted = artifact
        .preimage()
        .bindings_sorted
        .iter()
        .map(|binding| binding.binding_audit_hash.clone())
        .collect::<Vec<_>>();
    let board_artifact_content_hash = artifact.artifact_content_hash().to_owned();
    let board_artifact_valid_from = artifact.preimage().valid_from_rfc3339_nanos_utc.clone();
    let board_artifact_expires_at = artifact.preimage().expires_at_rfc3339_nanos_utc.clone();

    let config_snapshot = SelectionConfigSnapshotPreimage {
        domain: DOMAIN_SELECTION_CONFIG_SNAPSHOT.to_owned(),
        schema_version: CONFIG_SNAPSHOT_SCHEMA_VERSION.to_owned(),
        chain_config_bytes_hash,
        chain_rules_snapshot: parsed_chain.snapshot,
        chain_rules_sorted_content_hash,
        board_artifact: artifact.into_preimage(),
        board_artifact_content_hash,
        binding_audit_hashes_sorted,
        relation_schema_version: gate_contract.relation_schema_version.clone(),
        feature_version: gate_contract.feature_version.clone(),
        admission_version: gate_contract.admission_version.clone(),
        upstream_revision: UPSTREAM_REVISION.to_owned(),
        executable_revision: executable_revision.hash.clone(),
    };
    config_snapshot.validate()?;
    let config_snapshot_json = canonical_json(&config_snapshot)?;
    let config_snapshot_json_hash = sha256_bytes(config_snapshot_json.as_bytes());
    let config_hash = sha256_json(&config_snapshot)?;
    if config_snapshot_json_hash != config_hash {
        return Err(ConfigActivationPreparationError::new(
            "config_snapshot_hash_disagreement",
            "canonical JSON byte hash and typed config hash differ",
        ));
    }

    Ok(PreparedSnapshot {
        config_snapshot,
        config_snapshot_json_hash,
        config_hash,
        executable_revision,
        board_artifact_valid_from,
        board_artifact_expires_at,
    })
}

fn config_activation_board_error(error: BoardSelectionError) -> ConfigActivationPreparationError {
    ConfigActivationPreparationError::new(
        "board_artifact_verification_failed",
        format!("reason_code={}: {error}", error.reason_code()),
    )
}

fn validate_board_file_content_hashes(
    executable_revision: &ExecutableRevisionSnapshot,
    proposal_file_content_hash: &str,
    artifact_file_content_hash: &str,
) -> Result<(), ConfigActivationPreparationError> {
    for (relative_path, expected_hash) in [
        (BOARD_BINDING_PROPOSAL_PATH, proposal_file_content_hash),
        (BOARD_BINDINGS_PATH, artifact_file_content_hash),
    ] {
        let actual_hash = executable_revision
            .preimage
            .files_sorted
            .iter()
            .find(|file| file.relative_path == relative_path)
            .map(|file| file.content_sha256.as_str())
            .ok_or_else(|| {
                ConfigActivationPreparationError::new(
                    "verified_board_input_missing_from_revision",
                    format!("{relative_path} is absent from executable inputs"),
                )
            })?;
        if actual_hash != expected_hash {
            return Err(ConfigActivationPreparationError::new(
                "verified_board_input_mutated_during_activation",
                format!("{relative_path} changed after Board API verification"),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ChainRulesFileWire {
    rules: Vec<ChainRuleWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChainRuleWire {
    chain: String,
    logic: String,
    board_keyword: String,
    keywords: Vec<String>,
    #[serde(default)]
    priority: u32,
    #[serde(default)]
    category: String,
    #[serde(default)]
    generic: bool,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    provider_board: Option<ProviderBoardWire>,
}

const fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderBoardWire {
    provider: String,
    code: String,
    name: String,
    kind: ProviderBoardKind,
    binding_audit_hash: String,
}

#[derive(Debug)]
struct ParsedChainRules {
    snapshot: ChainRulesSnapshotPreimage,
    provider_boards: BTreeMap<String, ProviderBoardWire>,
}

fn parse_chain_rules(bytes: &[u8]) -> Result<ParsedChainRules, ConfigActivationPreparationError> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        ConfigActivationPreparationError::new("chain_config_not_utf8", error.to_string())
    })?;
    let file: ChainRulesFileWire = toml::from_str(text).map_err(|error| {
        ConfigActivationPreparationError::new("chain_config_invalid", error.to_string())
    })?;
    if file.rules.is_empty() {
        return Err(ConfigActivationPreparationError::new(
            "chain_rules_empty",
            "chain config must contain at least one rule",
        ));
    }

    let mut seen_chains = BTreeSet::new();
    let mut provider_boards = BTreeMap::new();
    let mut rules = Vec::with_capacity(file.rules.len());
    for rule in file.rules {
        require_text(&rule.chain, "rules.chain")?;
        require_text(&rule.logic, "rules.logic")?;
        require_text(&rule.board_keyword, "rules.board_keyword")?;
        if !seen_chains.insert(rule.chain.clone()) {
            return Err(ConfigActivationPreparationError::new(
                "duplicate_chain_rule",
                format!("duplicate chain ID {:?}", rule.chain),
            ));
        }
        if rule.priority > 100 {
            return Err(ConfigActivationPreparationError::new(
                "chain_priority_out_of_range",
                format!("priority exceeds 100 for {:?}", rule.chain),
            ));
        }
        let mut seen_keywords = BTreeSet::new();
        if rule.keywords.is_empty() {
            return Err(ConfigActivationPreparationError::new(
                "chain_keywords_empty",
                format!("rule {:?} has no keywords", rule.chain),
            ));
        }
        for keyword in &rule.keywords {
            require_text(keyword, "rules.keywords")?;
            if !seen_keywords.insert(keyword.clone()) {
                return Err(ConfigActivationPreparationError::new(
                    "duplicate_chain_keyword",
                    format!("duplicate keyword in {:?}", rule.chain),
                ));
            }
        }
        let binding_hash = if let Some(binding) = rule.provider_board {
            validate_provider_board(&rule.chain, &binding)?;
            let hash = binding.binding_audit_hash.clone();
            provider_boards.insert(rule.chain.clone(), binding);
            Some(hash)
        } else {
            None
        };
        rules.push(ChainRuleSnapshotEntryPreimage {
            chain_id: rule.chain,
            category: rule.category,
            priority: rule.priority,
            logic: rule.logic,
            board_keyword: rule.board_keyword,
            keywords_in_config_order: rule.keywords,
            generic: rule.generic,
            enabled: rule.enabled,
            provider_board_binding_audit_hash: binding_hash,
        });
    }
    rules.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.chain_id.as_bytes().cmp(right.chain_id.as_bytes()))
    });
    Ok(ParsedChainRules {
        snapshot: ChainRulesSnapshotPreimage {
            domain: DOMAIN_CHAIN_RULES_SNAPSHOT.to_owned(),
            rules_sorted: rules,
        },
        provider_boards,
    })
}

fn validate_provider_board(
    chain_id: &str,
    binding: &ProviderBoardWire,
) -> Result<(), ConfigActivationPreparationError> {
    require_text(chain_id, "provider_board.chain_id")?;
    require_text(&binding.provider, "provider_board.provider")?;
    require_text(&binding.code, "provider_board.code")?;
    require_text(&binding.name, "provider_board.name")?;
    require_hash(
        &binding.binding_audit_hash,
        "provider_board.binding_audit_hash",
    )?;
    if binding.provider != BOARD_DIRECTORY_PROVIDER {
        return Err(ConfigActivationPreparationError::new(
            "provider_board_provider_invalid",
            "formal board provider must be exactly tdx",
        ));
    }
    let expected_code = format!(
        "tdx:{}:{}",
        provider_board_kind_token(binding.kind),
        binding.name
    );
    if binding.code != expected_code {
        return Err(ConfigActivationPreparationError::new(
            "provider_board_code_invalid",
            format!("expected exact code {expected_code:?}"),
        ));
    }
    Ok(())
}

fn cross_validate_chain_and_artifact(
    chain: &ParsedChainRules,
    artifact: &ArtifactHashPreimage,
) -> Result<(), ConfigActivationPreparationError> {
    let artifact_bindings = artifact
        .bindings_sorted
        .iter()
        .map(|binding| (binding.chain_id.clone(), binding))
        .collect::<BTreeMap<_, _>>();
    if chain.provider_boards.len() != artifact_bindings.len() {
        return Err(ConfigActivationPreparationError::new(
            "board_binding_set_mismatch",
            "chain config and artifact binding counts differ",
        ));
    }
    for (chain_id, configured) in &chain.provider_boards {
        let artifact_binding = artifact_bindings.get(chain_id).ok_or_else(|| {
            ConfigActivationPreparationError::new(
                "board_binding_missing_from_artifact",
                format!("no artifact row for {chain_id:?}"),
            )
        })?;
        if configured.provider != artifact_binding.provider
            || configured.kind != artifact_binding.kind
            || configured.code != artifact_binding.code
            || configured.name != artifact_binding.name
            || configured.binding_audit_hash != artifact_binding.binding_audit_hash
        {
            return Err(ConfigActivationPreparationError::new(
                "board_binding_artifact_mismatch",
                format!("artifact/config differ for {chain_id:?}"),
            ));
        }
    }
    Ok(())
}

fn prepare_registered_feed_snapshot() -> Result<(String, String), ConfigActivationPreparationError>
{
    let mut entries = Vec::new();
    let mut seen_names = BTreeSet::new();
    for registration in registered_global_news_feeds() {
        if registration.upstream_revision != MAGIC_MARKET_DATA_REVISION
            || registration.upstream_revision != UPSTREAM_REVISION
            || !seen_names.insert(registration.feed_name)
        {
            return Err(ConfigActivationPreparationError::new(
                "registered_feed_registry_invalid",
                "feed names must be unique and use the frozen upstream revision",
            ));
        }
        let configuration = RegisteredFeedConfigurationPreimage {
            domain: DOMAIN_REGISTERED_FEED_CONFIG.to_owned(),
            gateway_provider: registration.gateway_provider.to_owned(),
            provider_id: registration.provider_id.to_owned(),
            source_contract: registration.source_contract.to_owned(),
            capability_name: registration.capability_name.to_owned(),
            max_limit: registration.max_limit,
            upstream_revision: registration.upstream_revision.to_owned(),
        };
        let configuration_hash = sha256_json(&configuration)?;
        let identity = RegisteredFeedIdentityPreimage {
            domain: DOMAIN_REGISTERED_FEED_IDENTITY.to_owned(),
            feed_name: registration.feed_name.to_owned(),
            gateway_provider: registration.gateway_provider.to_owned(),
            configuration_hash: configuration_hash.clone(),
        };
        entries.push((
            sha256_json(&identity)?,
            registration.gateway_provider.to_owned(),
            registration.capability_name.to_owned(),
            configuration_hash,
        ));
    }
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let feeds_sorted = entries
        .into_iter()
        .enumerate()
        .map(
            |(ordinal, (feed_identity, gateway_provider, capability_name, configuration_hash))| {
                Ok(RegisteredFeedEntryPreimage {
                    ordinal: u32::try_from(ordinal).map_err(|_| {
                        ConfigActivationPreparationError::new(
                            "registered_feed_ordinal_overflow",
                            "registered feed ordinal does not fit u32",
                        )
                    })?,
                    feed_identity,
                    gateway_provider,
                    capability_name,
                    configuration_hash,
                })
            },
        )
        .collect::<Result<Vec<_>, ConfigActivationPreparationError>>()?;
    let snapshot = RegisteredFeedSnapshotPreimage {
        domain: DOMAIN_REGISTERED_FEED_SNAPSHOT.to_owned(),
        feeds_sorted,
    };
    Ok((canonical_json(&snapshot)?, sha256_json(&snapshot)?))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationFileWire {
    schema_version: String,
    expected_config_hash: String,
    effective_from: String,
    reviewed_by: String,
    reviewed_at: String,
}

fn parse_activation_file(
    bytes: &[u8],
) -> Result<ActivationFileWire, ConfigActivationPreparationError> {
    let wire: ActivationFileWire = parse_canonical_json_file(bytes, "activation_file")?;
    if wire.schema_version != ACTIVATION_FILE_SCHEMA_VERSION {
        return Err(ConfigActivationPreparationError::new(
            "activation_schema_mismatch",
            "activation schema version differs from frozen value",
        ));
    }
    require_hash(&wire.expected_config_hash, "expected_config_hash")?;
    require_text(&wire.reviewed_by, "reviewed_by")?;
    let reviewer = wire.reviewed_by.to_ascii_lowercase();
    if matches!(reviewer.as_str(), "unreviewed" | "pending" | "todo") {
        return Err(ConfigActivationPreparationError::new(
            "activation_unreviewed",
            "activation file is not operator-reviewed",
        ));
    }
    parse_canonical_timestamp(&wire.effective_from, "effective_from")?;
    parse_canonical_timestamp(&wire.reviewed_at, "reviewed_at")?;
    Ok(wire)
}

fn validate_context(
    context: &ConfigActivationPreparationContext,
) -> Result<(), ConfigActivationPreparationError> {
    require_text(&context.stage_run_id, "stage_run_id")?;
    context.gate_contract.validate()?;
    if context.enveloped_at < context.activated_at {
        return Err(ConfigActivationPreparationError::new(
            "envelope_precedes_activation",
            "enveloped_at must not precede activated_at",
        ));
    }
    Ok(())
}

fn validate_legacy_cutover_snapshot(
    snapshot: &LegacyCutoverSnapshotPreimage,
    activated_at: DateTime<Utc>,
) -> Result<(), ConfigActivationPreparationError> {
    if snapshot.domain != crate::selection::schema_v2::DOMAIN_LEGACY_CUTOVER_SNAPSHOT {
        return Err(ConfigActivationPreparationError::new(
            "legacy_cutover_domain_mismatch",
            "legacy cutover snapshot domain is invalid",
        ));
    }
    require_hash(
        &snapshot.frozen_graph_trigger_set_hash,
        "frozen_graph_trigger_set_hash",
    )?;
    let captured_at = parse_canonical_timestamp(
        &snapshot.captured_at_rfc3339_nanos_utc,
        "cutover.captured_at",
    )?;
    if captured_at > activated_at {
        return Err(ConfigActivationPreparationError::new(
            "legacy_cutover_from_future",
            "legacy cutover snapshot was captured after activation",
        ));
    }
    if snapshot.tables_sorted.len() != crate::database::selection::LEGACY_SELECTION_TABLES.len() {
        return Err(ConfigActivationPreparationError::new(
            "legacy_cutover_table_set_mismatch",
            "legacy cutover snapshot must contain exactly the seven registered legacy tables",
        ));
    }

    let mut expected_tables =
        crate::database::selection::LEGACY_SELECTION_TABLES.map(str::to_owned);
    expected_tables.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for (table, expected_table_name) in snapshot.tables_sorted.iter().zip(expected_tables.iter()) {
        require_text(&table.table_name, "cutover.table_name")?;
        if &table.table_name != expected_table_name {
            return Err(ConfigActivationPreparationError::new(
                "legacy_cutover_table_set_mismatch",
                format!(
                    "legacy cutover table registry mismatch: expected {expected_table_name:?}, got {:?}",
                    table.table_name
                ),
            ));
        }
    }
    Ok(())
}

fn validate_repository_root(root: &Path) -> Result<PathBuf, ConfigActivationPreparationError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        ConfigActivationPreparationError::new("repository_root_unavailable", error.to_string())
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ConfigActivationPreparationError::new(
            "repository_root_invalid",
            "repository root must be a real directory, not a symlink",
        ));
    }
    fs::canonicalize(root).map_err(|error| {
        ConfigActivationPreparationError::new(
            "repository_root_canonicalization_failed",
            error.to_string(),
        )
    })
}

fn enumerate_executable_inputs(
    root: &Path,
) -> Result<Vec<PathBuf>, ConfigActivationPreparationError> {
    let src = root.join("src");
    let config = root.join("config");
    require_real_directory(&src, "src")?;
    require_real_directory(&config, "config")?;
    let mut paths = Vec::new();
    collect_regular_files(root, &src, &mut paths)?;
    collect_regular_files(root, &config, &mut paths)?;

    for entry in fs::read_dir(root).map_err(|error| {
        ConfigActivationPreparationError::new("repository_root_read_failed", error.to_string())
    })? {
        let entry = entry.map_err(|error| {
            ConfigActivationPreparationError::new("repository_entry_read_failed", error.to_string())
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ConfigActivationPreparationError::new("repository_entry_type_failed", error.to_string())
        })?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            ConfigActivationPreparationError::new(
                "non_utf8_executable_input",
                "root input filename is not UTF-8",
            )
        })?;
        let selected = (name.starts_with("Cargo") && name.ends_with(".toml"))
            || name == "Cargo.lock"
            || name == "build.rs";
        if selected {
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(ConfigActivationPreparationError::new(
                    "executable_input_not_regular",
                    format!("root input {name:?} is not a regular file"),
                ));
            }
            paths.push(PathBuf::from(name));
        }
    }
    if !paths.iter().any(|path| path == Path::new("Cargo.toml")) {
        return Err(ConfigActivationPreparationError::new(
            "cargo_manifest_missing",
            "root Cargo.toml is required",
        ));
    }
    paths.retain(|path| path != Path::new(ACTIVATION_FILE_RELATIVE_PATH));
    paths.sort_by(|left, right| {
        path_to_slash_string(left)
            .unwrap_or_default()
            .as_bytes()
            .cmp(path_to_slash_string(right).unwrap_or_default().as_bytes())
    });
    let mut unique = BTreeSet::new();
    for path in &paths {
        validate_relative_path(path)?;
        let key = path_to_slash_string(path)?;
        if !unique.insert(key) {
            return Err(ConfigActivationPreparationError::new(
                "duplicate_executable_input",
                "executable input path appears more than once",
            ));
        }
    }
    Ok(paths)
}

fn collect_regular_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), ConfigActivationPreparationError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            ConfigActivationPreparationError::new(
                "executable_input_directory_read_failed",
                error.to_string(),
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            ConfigActivationPreparationError::new(
                "executable_input_entry_read_failed",
                error.to_string(),
            )
        })?;
    entries.sort_by_key(|left| left.file_name());
    for entry in entries {
        let file_type = entry.file_type().map_err(|error| {
            ConfigActivationPreparationError::new(
                "executable_input_entry_type_failed",
                error.to_string(),
            )
        })?;
        if file_type.is_symlink() {
            return Err(ConfigActivationPreparationError::new(
                "executable_input_symlink_forbidden",
                format!("{} is a symlink", entry.path().display()),
            ));
        }
        if file_type.is_dir() {
            collect_regular_files(root, &entry.path(), output)?;
        } else if file_type.is_file() {
            let entry_path = entry.path();
            let relative = entry_path.strip_prefix(root).map_err(|error| {
                ConfigActivationPreparationError::new(
                    "executable_input_outside_root",
                    error.to_string(),
                )
            })?;
            if relative != Path::new(ACTIVATION_FILE_RELATIVE_PATH) {
                output.push(relative.to_owned());
            }
        } else {
            return Err(ConfigActivationPreparationError::new(
                "executable_input_not_regular",
                format!("{} is not a regular file", entry.path().display()),
            ));
        }
    }
    Ok(())
}

fn require_real_directory(
    path: &Path,
    label: &'static str,
) -> Result<(), ConfigActivationPreparationError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ConfigActivationPreparationError::new(
            "required_input_root_missing",
            format!("{label}: {error}"),
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ConfigActivationPreparationError::new(
            "required_input_root_invalid",
            format!("{label} must be a real directory"),
        ));
    }
    Ok(())
}

fn read_stable_regular_file(
    path: &Path,
    relative_path: &Path,
) -> Result<Vec<u8>, ConfigActivationPreparationError> {
    let before = fs::symlink_metadata(path).map_err(|error| {
        ConfigActivationPreparationError::new(
            "executable_input_unavailable",
            format!("{}: {error}", relative_path.display()),
        )
    })?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(ConfigActivationPreparationError::new(
            "executable_input_not_regular",
            relative_path.display().to_string(),
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        ConfigActivationPreparationError::new(
            "executable_input_read_failed",
            format!("{}: {error}", relative_path.display()),
        )
    })?;
    let after = fs::symlink_metadata(path).map_err(|error| {
        ConfigActivationPreparationError::new(
            "executable_input_post_read_failed",
            format!("{}: {error}", relative_path.display()),
        )
    })?;
    if !after.is_file()
        || after.file_type().is_symlink()
        || before.len() != after.len()
        || after.len() != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
        || before.modified().ok() != after.modified().ok()
    {
        return Err(ConfigActivationPreparationError::new(
            "executable_input_mutated_during_read",
            relative_path.display().to_string(),
        ));
    }
    Ok(bytes)
}

fn read_required_file(
    path: &Path,
    label: &'static str,
) -> Result<Vec<u8>, ConfigActivationPreparationError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ConfigActivationPreparationError::new(
            "required_config_unavailable",
            format!("{label}: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ConfigActivationPreparationError::new(
            "required_config_not_regular",
            format!("{label} must be a real regular file"),
        ));
    }
    fs::read(path).map_err(|error| {
        ConfigActivationPreparationError::new(
            "required_config_read_failed",
            format!("{label}: {error}"),
        )
    })
}

fn parse_canonical_json_file<T>(
    bytes: &[u8],
    label: &'static str,
) -> Result<T, ConfigActivationPreparationError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let value = serde_json::from_slice::<T>(bytes).map_err(|error| {
        ConfigActivationPreparationError::new("strict_json_invalid", format!("{label}: {error}"))
    })?;
    let mut canonical = serde_json::to_vec(&value).map_err(|error| {
        ConfigActivationPreparationError::new(
            "strict_json_canonicalization_failed",
            format!("{label}: {error}"),
        )
    })?;
    canonical.push(b'\n');
    if canonical != bytes {
        return Err(ConfigActivationPreparationError::new(
            "strict_json_noncanonical",
            format!("{label} must be compact fixed-order JSON plus one LF"),
        ));
    }
    Ok(value)
}

fn require_text(value: &str, field: &'static str) -> Result<(), ConfigActivationPreparationError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(ConfigActivationPreparationError::new(
            "invalid_required_text",
            format!("{field} must be nonempty trim-stable text without controls"),
        ));
    }
    Ok(())
}

fn require_hash(value: &str, field: &'static str) -> Result<(), ConfigActivationPreparationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ConfigActivationPreparationError::new(
            "invalid_sha256",
            format!("{field} must be lowercase 64-hex"),
        ));
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), ConfigActivationPreparationError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ConfigActivationPreparationError::new(
            "invalid_executable_input_path",
            format!("invalid relative path {}", path.display()),
        ));
    }
    path_to_slash_string(path).map(|_| ())
}

fn path_to_slash_string(path: &Path) -> Result<String, ConfigActivationPreparationError> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().ok_or_else(|| {
                ConfigActivationPreparationError::new(
                    "non_utf8_executable_input",
                    "executable input path is not UTF-8",
                )
            }),
            _ => Err(ConfigActivationPreparationError::new(
                "invalid_executable_input_path",
                "executable input path contains a non-normal component",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(components.join("/"))
}

fn parse_canonical_timestamp(
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, ConfigActivationPreparationError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| {
            ConfigActivationPreparationError::new(
                "invalid_rfc3339_nanos_utc",
                format!("{field}: {error}"),
            )
        })?
        .with_timezone(&Utc);
    if rfc3339_nanos(parsed) != value {
        return Err(ConfigActivationPreparationError::new(
            "noncanonical_rfc3339_nanos_utc",
            format!("{field} must use UTC nanoseconds and Z"),
        ));
    }
    Ok(parsed)
}

fn rfc3339_nanos(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

const fn provider_board_kind_token(kind: ProviderBoardKind) -> &'static str {
    match kind {
        ProviderBoardKind::Industry => "industry",
        ProviderBoardKind::Concept => "concept",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::schema_v2::{
        ArtifactBindingPreimage, AttestedDirectoryBatchPreimage, BindingAuditPreimage,
        BoardAuditAttestationContentPreimage, BoardAuditAttestationReceiptPreimage,
        BoardAuditSubjectPreimage, BoardBindingProposalInputPreimage,
        DirectoryBatchContentPreimage, DirectoryBatchEvidencePreimage,
        DirectoryBoardRecordPreimage, DirectoryRecordSourceEvidencePreimage,
        LegacyCutoverTableWatermarkPreimage, ProposalBindingPreimage, BOARD_AUDIT_COMMAND_VERSION,
        BOARD_AUDIT_ROOT_POLICY_VERSION, BOARD_BINDINGS_SCHEMA_VERSION,
        BOARD_BINDING_PROPOSAL_SCHEMA_VERSION, BOARD_BINDING_VALIDITY_POLICY_VERSION,
        BOARD_CONNECTION_POLICY_VERSION, BOARD_DIRECTORY_PROVIDER, BOARD_DIRECTORY_REQUEST_LIMIT,
        BOARD_DIRECTORY_SOURCE, DOMAIN_BOARD_ARTIFACT, DOMAIN_BOARD_AUDIT_ATTESTATION,
        DOMAIN_BOARD_AUDIT_RECEIPT, DOMAIN_BOARD_AUDIT_SUBJECT, DOMAIN_BOARD_BINDING,
        DOMAIN_BOARD_BINDING_PROPOSAL, DOMAIN_BOARD_DIRECTORY_BATCH, DOMAIN_BOARD_DIRECTORY_RECORD,
        DOMAIN_LEGACY_CUTOVER_SNAPSHOT,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TEST_PROPOSAL_HASH: &str =
        "e757ad42e250a8a5b29eb9b068ae970311774033d454f8e72a5ff5504656582b";
    const TEST_CONNECTION_POLICY_HASH: &str =
        "1afe40005c2fed9c4a4b12ad5ff2380cdbc6df43e5b751db41274f5d0b25534f";
    const TEST_AUDIT_ROOT_HASH: &str =
        "608e86c4520456ae8b10a6a8e2b1396a5014e1b91db5022cc4a6e21978432044";
    const TEST_CONCEPT_RECORD_HASH: &str =
        "1413cffc3e9a5906481208a328d0df1eb46e70f510e7f86152dedaebdf863eb5";
    const TEST_CONCEPT_BATCH_HASH: &str =
        "e03c9b21d60ebb59a77c8b5ee2d010797b2739cf0dc937d63274d787b3646240";
    const TEST_INDUSTRY_BATCH_HASH: &str =
        "3405317c3a7ecbc9f2200817a38377137dfd354a71ef285c4708f732220f9c4a";

    struct TestFixture {
        root: PathBuf,
    }

    impl TestFixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "TEST_CODE_config_activation_{}_{}",
                std::process::id(),
                id
            ));
            fs::create_dir_all(root.join("src")).expect("create src");
            fs::create_dir_all(root.join("config/selection")).expect("create config");
            fs::write(
                root.join("Cargo.toml"),
                b"[package]\nname=\"TEST_CODE_fixture\"\nversion=\"0.0.0\"\nedition=\"2021\"\n",
            )
            .expect("write Cargo.toml");
            fs::write(root.join("src/lib.rs"), b"pub const TEST_CODE: u8 = 1;\n")
                .expect("write source");
            Self { root }
        }

        fn install_verified_config(&self) {
            let artifact = verified_board_artifact();
            artifact.validate().expect("valid schema fixture");
            write_canonical(
                &self.root.join(BOARD_BINDING_PROPOSAL_PATH),
                &artifact.proposal_input,
            );
            let artifact_content_hash = artifact
                .artifact_content_hash()
                .expect("artifact content hash");
            write_canonical(
                &self.root.join(BOARD_BINDINGS_PATH),
                &VerifiedBoardArtifactFileFixture::from_preimage(&artifact, artifact_content_hash),
            );
            let chain = format!(
                "[[rules]]\nchain = \"TEST_CODE_CHAIN\"\ncategory = \"TEST_CODE\"\npriority = 100\nlogic = \"TEST_CODE logic\"\nboard_keyword = \"TEST_CODE_BOARD\"\nenabled = true\ngeneric = false\nkeywords = [\"TEST_CODE_keyword\"]\n\n[rules.provider_board]\nprovider = \"tdx\"\ncode = \"tdx:concept:TEST_CODE_BOARD\"\nname = \"TEST_CODE_BOARD\"\nkind = \"concept\"\nbinding_audit_hash = \"{}\"\n",
                artifact.bindings_sorted[0].binding_audit_hash,
            );
            fs::write(self.root.join(CHAIN_CONFIG_RELATIVE_PATH), chain.as_bytes())
                .expect("write chain");
        }

        fn install_proposal_only(&self) {
            let artifact = verified_board_artifact();
            write_canonical(
                &self.root.join(BOARD_BINDING_PROPOSAL_PATH),
                &artifact.proposal_input,
            );
        }

        fn install_activation(&self, expected_hash: &str, reviewed_by: &str) {
            let wire = ActivationFileWire {
                schema_version: ACTIVATION_FILE_SCHEMA_VERSION.to_owned(),
                expected_config_hash: expected_hash.to_owned(),
                effective_from: "2026-07-28T09:00:00.000000000Z".to_owned(),
                reviewed_by: reviewed_by.to_owned(),
                reviewed_at: "2026-07-28T07:00:00.000000000Z".to_owned(),
            };
            write_canonical(&self.root.join(ACTIVATION_FILE_RELATIVE_PATH), &wire);
        }

        fn context(&self) -> ConfigActivationPreparationContext {
            let mut tables_sorted = crate::database::selection::LEGACY_SELECTION_TABLES
                .into_iter()
                .map(|table_name| LegacyCutoverTableWatermarkPreimage {
                    table_name: table_name.to_owned(),
                    max_rowid: 0,
                    row_count: 0,
                })
                .collect::<Vec<_>>();
            tables_sorted
                .sort_by(|left, right| left.table_name.as_bytes().cmp(right.table_name.as_bytes()));
            ConfigActivationPreparationContext::checked_in(
                "TEST_CODE_STAGE_RUN".to_owned(),
                timestamp("2026-07-28T08:10:00.000000000Z"),
                timestamp("2026-07-28T08:10:01.000000000Z"),
                LegacyCutoverSnapshotPreimage {
                    domain: DOMAIN_LEGACY_CUTOVER_SNAPSHOT.to_owned(),
                    captured_at_rfc3339_nanos_utc: "2026-07-28T07:59:00.000000000Z".to_owned(),
                    tables_sorted,
                    pending_inbox_count: 0,
                    committed_legacy_candidate_count: 0,
                    legacy_outcome_row_count: 0,
                    frozen_graph_trigger_set_hash: HASH_A.to_owned(),
                },
            )
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn exact_checked_in_bytes_reuse_identical_hashes() {
        let fixture = TestFixture::new();
        fixture.install_verified_config();
        let first = prepare_snapshot(
            &fixture.root,
            fixture.context().activated_at,
            &ConfigActivationGateContract::checked_in(),
        )
        .expect("first snapshot");
        fixture.install_activation(&first.config_hash, "TEST_CODE_REVIEWER");

        let prepared_one = prepare_config_activation_from_root(&fixture.root, fixture.context())
            .expect("first activation");
        let prepared_two = prepare_config_activation_from_root(&fixture.root, fixture.context())
            .expect("replayed activation");

        assert_eq!(
            prepared_one.stage_input.config_hash,
            prepared_two.stage_input.config_hash
        );
        assert_eq!(
            prepared_one.recovery_envelope.in_memory_payload_hash,
            prepared_two.recovery_envelope.in_memory_payload_hash
        );
        assert_eq!(
            sha256_json(&prepared_one.recovery_envelope)
                .expect("TEST_CODE first recovery envelope hash"),
            sha256_json(&prepared_two.recovery_envelope)
                .expect("TEST_CODE second recovery envelope hash")
        );
    }

    #[test]
    fn one_source_byte_changes_executable_and_config_revision() {
        let fixture = TestFixture::new();
        fixture.install_verified_config();
        let first = prepare_snapshot(
            &fixture.root,
            fixture.context().activated_at,
            &ConfigActivationGateContract::checked_in(),
        )
        .expect("first snapshot");
        fs::write(
            fixture.root.join("src/lib.rs"),
            b"pub const TEST_CODE: u8 = 2;\n",
        )
        .expect("mutate source");
        let second = prepare_snapshot(
            &fixture.root,
            fixture.context().activated_at,
            &ConfigActivationGateContract::checked_in(),
        )
        .expect("second snapshot");

        assert_ne!(
            first.executable_revision.hash,
            second.executable_revision.hash
        );
        assert_ne!(first.config_hash, second.config_hash);
    }

    #[test]
    fn legacy_cutover_snapshot_requires_exact_registered_table_set() {
        let fixture = TestFixture::new();
        let context = fixture.context();
        validate_legacy_cutover_snapshot(&context.legacy_cutover_snapshot, context.activated_at)
            .expect("exact seven-table cutover snapshot");

        let mut missing = fixture.context();
        missing.legacy_cutover_snapshot.tables_sorted.pop();
        let error = validate_legacy_cutover_snapshot(
            &missing.legacy_cutover_snapshot,
            missing.activated_at,
        )
        .expect_err("missing legacy table must fail closed");
        assert_eq!(error.code, "legacy_cutover_table_set_mismatch");

        let mut substituted = fixture.context();
        substituted.legacy_cutover_snapshot.tables_sorted[0].table_name =
            "TEST_CODE_unknown_legacy_table".to_owned();
        substituted
            .legacy_cutover_snapshot
            .tables_sorted
            .sort_by(|left, right| left.table_name.as_bytes().cmp(right.table_name.as_bytes()));
        let error = validate_legacy_cutover_snapshot(
            &substituted.legacy_cutover_snapshot,
            substituted.activated_at,
        )
        .expect_err("substituted legacy table must fail closed");
        assert_eq!(error.code, "legacy_cutover_table_set_mismatch");
    }

    #[test]
    fn missing_board_artifact_fails_closed() {
        let fixture = TestFixture::new();
        fixture.install_proposal_only();
        fs::write(
            fixture.root.join(CHAIN_CONFIG_RELATIVE_PATH),
            b"[[rules]]\nchain=\"TEST_CODE_CHAIN\"\nlogic=\"x\"\nboard_keyword=\"x\"\nkeywords=[\"x\"]\n",
        )
        .expect("write chain");

        let error = prepare_snapshot(
            &fixture.root,
            fixture.context().activated_at,
            &ConfigActivationGateContract::checked_in(),
        )
        .expect_err("artifact is required");

        assert_eq!(error.code, "board_artifact_verification_failed");
        assert!(error
            .detail
            .contains("required_selection_config_unavailable"));
    }

    #[test]
    fn direct_only_registry_cannot_activate_or_synthesize_evidence() {
        let fixture = TestFixture::new();
        fixture.install_proposal_only();
        fs::write(
            fixture.root.join(CHAIN_CONFIG_RELATIVE_PATH),
            b"[[rules]]\nchain=\"TEST_CODE_CHAIN\"\nlogic=\"x\"\nboard_keyword=\"x\"\nkeywords=[\"x\"]\n",
        )
        .expect("write chain");
        fs::write(
            fixture.root.join(BOARD_BINDINGS_PATH),
            concat!(
                "{\"schema_version\":\"selection-provider-board-bindings-v1\",",
                "\"upstream_revision\":\"5f1ce93656a55854c844065390520cd4aecd9a14\",",
                "\"state\":\"direct_only_unverified\",\"bindings\":[]}\n"
            ),
        )
        .expect("write direct-only registry");

        let error = prepare_snapshot(
            &fixture.root,
            fixture.context().activated_at,
            &ConfigActivationGateContract::checked_in(),
        )
        .expect_err("unverified registry must not activate");

        assert_eq!(error.code, "board_artifact_verification_failed");
        assert!(error.detail.contains("board_artifact_invalid_json"));
    }

    #[test]
    fn activation_file_is_the_only_executable_revision_exclusion() {
        let fixture = TestFixture::new();
        fixture.install_verified_config();
        fixture.install_activation(HASH_A, "TEST_CODE_REVIEWER");
        let first = compute_executable_revision(&fixture.root).expect("first revision");
        fixture.install_activation(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "TEST_CODE_REVIEWER",
        );
        let second = compute_executable_revision(&fixture.root).expect("second revision");
        assert_eq!(first, second);

        fs::write(
            fixture.root.join("config/extra.json"),
            b"{\"TEST_CODE\":1}\n",
        )
        .expect("write extra config");
        let third = compute_executable_revision(&fixture.root).expect("third revision");
        assert_ne!(second.hash, third.hash);
    }

    #[derive(Serialize)]
    struct VerifiedBoardArtifactFileFixture {
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
        bindings: Vec<ArtifactBindingPreimage>,
    }

    impl VerifiedBoardArtifactFileFixture {
        fn from_preimage(artifact: &ArtifactHashPreimage, artifact_content_hash: String) -> Self {
            Self {
                schema_version: artifact.schema_version.clone(),
                artifact_content_hash,
                upstream_revision: artifact.upstream_revision.clone(),
                proposal_input: artifact.proposal_input.clone(),
                proposal_input_content_hash: artifact.proposal_input_content_hash.clone(),
                connection_policy_version: artifact.connection_policy_version.clone(),
                connection_policy_hash: artifact.connection_policy_hash.clone(),
                provider_endpoint_evidence: artifact.provider_endpoint_evidence.clone(),
                valid_from: artifact.valid_from_rfc3339_nanos_utc.clone(),
                expires_at: artifact.expires_at_rfc3339_nanos_utc.clone(),
                directory_batches_by_category: artifact.directory_batches_by_category.clone(),
                requested_limit: artifact.requested_limit,
                audit_command_version: artifact.audit_command_version.clone(),
                recorded_at: artifact.recorded_at_rfc3339_nanos_utc.clone(),
                audit_attestation_receipt: artifact.audit_attestation_receipt.clone(),
                audit_attestation_receipt_hash: artifact.audit_attestation_receipt_hash.clone(),
                bindings: artifact.bindings_sorted.clone(),
            }
        }
    }

    fn verified_board_artifact() -> ArtifactHashPreimage {
        let proposal = BoardBindingProposalInputPreimage {
            domain: DOMAIN_BOARD_BINDING_PROPOSAL.into(),
            schema_version: BOARD_BINDING_PROPOSAL_SCHEMA_VERSION.into(),
            validity_policy_version: BOARD_BINDING_VALIDITY_POLICY_VERSION.into(),
            valid_from_rfc3339_nanos_utc: "2026-07-28T08:10:00.000000000Z".into(),
            expires_at_rfc3339_nanos_utc: "2026-08-27T08:10:00.000000000Z".into(),
            reviewed_by: "TEST_CODE_REVIEWER".into(),
            reviewed_at_rfc3339_nanos_utc: "2026-07-28T08:00:00.000000000Z".into(),
            bindings_sorted: vec![ProposalBindingPreimage {
                chain_id: "TEST_CODE_CHAIN".into(),
                provider: BOARD_DIRECTORY_PROVIDER.into(),
                kind: ProviderBoardKind::Concept,
                code: "tdx:concept:TEST_CODE_BOARD".into(),
                name: "TEST_CODE_BOARD".into(),
            }],
        };
        seal_verified_board_artifact(ArtifactHashPreimage {
            domain: DOMAIN_BOARD_ARTIFACT.into(),
            schema_version: BOARD_BINDINGS_SCHEMA_VERSION.into(),
            upstream_revision: UPSTREAM_REVISION.into(),
            proposal_input: proposal,
            proposal_input_content_hash: TEST_PROPOSAL_HASH.into(),
            connection_policy_version: BOARD_CONNECTION_POLICY_VERSION.into(),
            connection_policy_hash: TEST_CONNECTION_POLICY_HASH.into(),
            provider_endpoint_evidence: None,
            valid_from_rfc3339_nanos_utc: "2026-07-28T08:10:00.000000000Z".into(),
            expires_at_rfc3339_nanos_utc: "2026-08-27T08:10:00.000000000Z".into(),
            directory_batches_by_category: vec![
                directory_batch(
                    ProviderBoardKind::Concept,
                    "TEST_CODE_BOARD",
                    42,
                    "unix-ms:1785225900000",
                    "TEST_CODE_CONCEPT_BATCH",
                    TEST_CONCEPT_BATCH_HASH,
                ),
                directory_batch(
                    ProviderBoardKind::Industry,
                    "TEST_CODE_INDUSTRY",
                    24,
                    "2026-07-28T08:05:00.000000000Z",
                    "TEST_CODE_INDUSTRY_BATCH",
                    TEST_INDUSTRY_BATCH_HASH,
                ),
            ],
            requested_limit: BOARD_DIRECTORY_REQUEST_LIMIT,
            audit_command_version: BOARD_AUDIT_COMMAND_VERSION.into(),
            recorded_at_rfc3339_nanos_utc: "2026-07-28T08:10:00.000000000Z".into(),
            audit_attestation_receipt: BoardAuditAttestationReceiptPreimage {
                domain: DOMAIN_BOARD_AUDIT_RECEIPT.into(),
                audit_subject_id: String::new(),
                audit_run_id: "01900000-0000-7000-8000-000000000001".into(),
                prepared_record_hash: HASH_A.into(),
                committed_record_hash:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                attestation_content_hash: String::new(),
                audit_root_policy_version: BOARD_AUDIT_ROOT_POLICY_VERSION.into(),
                audit_root_binding_hash: TEST_AUDIT_ROOT_HASH.into(),
            },
            audit_attestation_receipt_hash: String::new(),
            bindings_sorted: vec![ArtifactBindingPreimage {
                chain_id: "TEST_CODE_CHAIN".into(),
                provider: BOARD_DIRECTORY_PROVIDER.into(),
                kind: ProviderBoardKind::Concept,
                code: "tdx:concept:TEST_CODE_BOARD".into(),
                name: "TEST_CODE_BOARD".into(),
                binding_audit_hash: String::new(),
                directory_record_hash: TEST_CONCEPT_RECORD_HASH.into(),
                release_directory_member_count: 42,
            }],
        })
    }

    fn seal_verified_board_artifact(mut artifact: ArtifactHashPreimage) -> ArtifactHashPreimage {
        artifact.audit_attestation_receipt.audit_subject_id = BoardAuditSubjectPreimage {
            domain: DOMAIN_BOARD_AUDIT_SUBJECT.into(),
            proposal_input_content_hash: artifact.proposal_input_content_hash.clone(),
            audit_command_version: artifact.audit_command_version.clone(),
            connection_policy_hash: artifact.connection_policy_hash.clone(),
        }
        .audit_subject_id()
        .expect("derive TEST_CODE board audit subject");
        artifact.audit_attestation_receipt.attestation_content_hash =
            BoardAuditAttestationContentPreimage {
                domain: DOMAIN_BOARD_AUDIT_ATTESTATION.into(),
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
                directory_batches_by_category: artifact
                    .directory_batches_by_category
                    .iter()
                    .map(|batch| AttestedDirectoryBatchPreimage {
                        category: batch.content.category,
                        batch_content_hash: batch.batch_content_hash.clone(),
                        record_count: batch.record_count,
                        observed_at: batch.content.observed_at.clone(),
                    })
                    .collect(),
                recorded_at_rfc3339_nanos_utc: artifact.recorded_at_rfc3339_nanos_utc.clone(),
            }
            .attestation_content_hash()
            .expect("derive TEST_CODE board audit attestation");
        artifact.audit_attestation_receipt_hash = artifact
            .audit_attestation_receipt
            .audit_attestation_receipt_hash()
            .expect("derive TEST_CODE board audit receipt");
        for index in 0..artifact.bindings_sorted.len() {
            let binding = artifact.bindings_sorted[index].clone();
            let batch = artifact
                .directory_batches_by_category
                .iter()
                .find(|batch| batch.content.category == binding.kind)
                .expect("TEST_CODE binding category has a directory batch");
            artifact.bindings_sorted[index].binding_audit_hash = BindingAuditPreimage {
                domain: DOMAIN_BOARD_BINDING.into(),
                upstream_revision: artifact.upstream_revision.clone(),
                chain_id: binding.chain_id,
                provider: binding.provider,
                kind: binding.kind,
                code: binding.code,
                name: binding.name,
                directory_category: batch.content.category,
                directory_source: batch.content.source.clone(),
                directory_source_at: batch.content.source_at.clone(),
                directory_observed_at: batch.content.observed_at.clone(),
                directory_batch_id: batch.content.batch_id.clone(),
                directory_batch_content_hash: batch.batch_content_hash.clone(),
                directory_record_hash: binding.directory_record_hash,
                release_directory_member_count: binding.release_directory_member_count,
                proposal_input_content_hash: artifact.proposal_input_content_hash.clone(),
                proposal_reviewed_by: artifact.proposal_input.reviewed_by.clone(),
                proposal_reviewed_at_rfc3339_nanos_utc: artifact
                    .proposal_input
                    .reviewed_at_rfc3339_nanos_utc
                    .clone(),
                validity_policy_version: artifact.proposal_input.validity_policy_version.clone(),
                audit_command_version: artifact.audit_command_version.clone(),
                connection_policy_version: artifact.connection_policy_version.clone(),
                connection_policy_hash: artifact.connection_policy_hash.clone(),
                provider_endpoint_evidence: artifact.provider_endpoint_evidence.clone(),
                audit_attestation_receipt_hash: artifact.audit_attestation_receipt_hash.clone(),
                recorded_at_rfc3339_nanos_utc: artifact.recorded_at_rfc3339_nanos_utc.clone(),
                valid_from_rfc3339_nanos_utc: artifact.valid_from_rfc3339_nanos_utc.clone(),
                expires_at_rfc3339_nanos_utc: artifact.expires_at_rfc3339_nanos_utc.clone(),
            }
            .binding_audit_hash()
            .expect("derive TEST_CODE board binding audit hash");
        }
        artifact
    }

    fn directory_batch(
        kind: ProviderBoardKind,
        name: &str,
        member_count: u32,
        observed_at: &str,
        batch_id: &str,
        batch_content_hash: &str,
    ) -> DirectoryBatchEvidencePreimage {
        DirectoryBatchEvidencePreimage {
            content: DirectoryBatchContentPreimage {
                domain: DOMAIN_BOARD_DIRECTORY_BATCH.into(),
                category: kind,
                provider: BOARD_DIRECTORY_PROVIDER.into(),
                source: BOARD_DIRECTORY_SOURCE.into(),
                source_at: None,
                observed_at: observed_at.into(),
                batch_id: batch_id.into(),
                records_in_provider_order: vec![DirectoryBoardRecordPreimage {
                    domain: DOMAIN_BOARD_DIRECTORY_RECORD.into(),
                    provider_ordinal: 0,
                    code: format!("tdx:{}:{name}", kind.as_str()),
                    name: name.into(),
                    kind,
                    member_count,
                    evidence: DirectoryRecordSourceEvidencePreimage {
                        provider: BOARD_DIRECTORY_PROVIDER.into(),
                        source: BOARD_DIRECTORY_SOURCE.into(),
                        source_at: None,
                        observed_at: observed_at.into(),
                        batch_id: batch_id.into(),
                    },
                }],
            },
            batch_content_hash: batch_content_hash.into(),
            record_count: 1,
        }
    }

    fn write_canonical<T: Serialize>(path: &Path, value: &T) {
        let mut bytes = serde_json::to_vec(value).expect("serialize fixture");
        bytes.push(b'\n');
        fs::write(path, bytes).expect("write fixture");
    }

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("fixture timestamp")
            .with_timezone(&Utc)
    }
}
