//! Canonical BR-174/BR-176/BR-177/BR-178/BR-182 schema-v2 preimages.
//!
//! This module is intentionally storage-agnostic. It owns the compact JSON field
//! order, domain-separated SHA-256 inputs, closed token sets, and the
//! required/NULL matrices that the later SQLite repository must enforce.
//!
//! Data-redline coverage:
//! - AGENTS §2.2: unavailable values remain `None` and serialize as JSON `null`;
//! - AGENTS §2.3: invalid state/evidence combinations fail explicitly;
//! - AGENTS §2.7: every identity and receipt has a fixed, auditable preimage.

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const UPSTREAM_REVISION: &str = "75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";
pub const AMENDMENT_DESIGN_SHA256: &str =
    "5c36c4a9d8b871de524186e9939717b8d888c15b06ac9543b9bec215796bc906";
pub const OUTCOME_PARENT_DESIGN_SHA256: &str =
    "d4a9dc3098541b15d0b7ee8e17381ae8531f6c2d452f5cdb95fba887b257df3a";
pub const OUTCOME_ADAPTIVE_POLICY_VERSION: &str = "magic-tdx-latest-n-v1";
pub const OUTCOME_TDX_HISTORICAL_PAGE_SIZE: u32 = 800;

pub const DOMAIN_SOURCE_FACT_KEY: &str = "stock_analysis.br174.source_fact_key.v1";
pub const DOMAIN_SOURCE_CONTENT: &str = "stock_analysis.br174.source_content.v1";
pub const DOMAIN_ACQUIRED_GLOBAL_NEWS_RECORD: &str =
    "stock_analysis.br174.acquired_global_news_record.v1";
pub const DOMAIN_SOURCE_FACT_CONFLICT: &str = "stock_analysis.br174.source_fact_conflict.v1";
pub const DOMAIN_SOURCE_ATTEMPT: &str = "stock_analysis.br174.source_attempt.v1";
pub const DOMAIN_FEED_ATTEMPT_KEY: &str = "stock_analysis.br174.feed_attempt_key.v1";
pub const DOMAIN_FEED_BATCH_EVIDENCE: &str = "stock_analysis.br174.feed_batch_evidence.v1";
pub const DOMAIN_FEED_AVAILABLE_EVIDENCE: &str = "stock_analysis.br174.feed_available_evidence.v1";
pub const DOMAIN_FEED_SOURCE_RECORD: &str = "stock_analysis.br174.feed_source_record.v1";
pub const DOMAIN_FEED_SOURCE_CONTENT: &str = "stock_analysis.br174.feed_source_content.v1";
pub const DOMAIN_FEED_ATTEMPT_CONTENT: &str = "stock_analysis.br174.feed_attempt_content.v2";
pub const DOMAIN_REGISTERED_FEED_CONFIG: &str = "stock_analysis.br174.registered_feed_config.v1";
pub const DOMAIN_REGISTERED_FEED_IDENTITY: &str =
    "stock_analysis.br174.registered_feed_identity.v1";
pub const DOMAIN_REGISTERED_FEED_SNAPSHOT: &str =
    "stock_analysis.br174.registered_feed_snapshot.v1";
pub const DOMAIN_SOURCE_BATCH_CONTENT: &str = "stock_analysis.br174.source_batch_content.v1";
pub const DOMAIN_INGRESS_GATE_INPUT: &str = "stock_analysis.br174.ingress_gate_input.v1";
pub const DOMAIN_INGRESS_GATE_RECEIPT: &str = "stock_analysis.br174.ingress_gate_receipt.v1";
pub const DOMAIN_BOARD_BINDING_PROPOSAL: &str = "stock_analysis.br174.board_binding_proposal.v1";
pub const DOMAIN_BOARD_CONNECTION_POLICY: &str = "stock_analysis.br174.board_connection_policy.v1";
pub const DOMAIN_BOARD_AUDIT_ROOT: &str = "stock_analysis.br174.board_audit_root.v1";
pub const DOMAIN_PRODUCTION_EVIDENCE_PATH: &str =
    "stock_analysis.br174.production_evidence_path.v1";
pub const DOMAIN_BOARD_DIRECTORY_RECORD: &str = "stock_analysis.br174.board_directory_record.v1";
pub const DOMAIN_BOARD_DIRECTORY_BATCH: &str = "stock_analysis.br174.board_directory_batch.v1";
pub const DOMAIN_BOARD_AUDIT_SUBJECT: &str = "stock_analysis.br174.board_audit_subject.v1";
pub const DOMAIN_BOARD_AUDIT_PREPARED: &str = "stock_analysis.br174.board_audit_prepared.v1";
pub const DOMAIN_BOARD_AUDIT_ATTESTATION: &str = "stock_analysis.br174.board_audit_attestation.v1";
pub const DOMAIN_BOARD_AUDIT_COMMITTED: &str = "stock_analysis.br174.board_audit_committed.v1";
pub const DOMAIN_BOARD_AUDIT_RECEIPT: &str = "stock_analysis.br174.board_audit_receipt.v1";
pub const DOMAIN_BOARD_ARTIFACT: &str = "stock_analysis.br174.board_artifact.v1";
pub const DOMAIN_BOARD_BINDING: &str = "stock_analysis.br174.board_binding.v1";
pub const DOMAIN_CHAIN_RULES_SNAPSHOT: &str = "stock_analysis.br174.chain_rules_snapshot.v1";
pub const DOMAIN_EXECUTABLE_REVISION: &str = "stock_analysis.br174.executable_revision.v1";
pub const DOMAIN_SELECTION_CONFIG_SNAPSHOT: &str =
    "stock_analysis.br174.selection_config_snapshot.v1";
pub const DOMAIN_CONFIG_ACTIVATION: &str = "stock_analysis.br174.config_activation.v1";
pub const DOMAIN_DIRECT_SOURCE: &str = "stock_analysis.br174.direct_source.v1";
pub const DOMAIN_BOARD_SOURCE: &str = "stock_analysis.br174.board_source.v1";
pub const DOMAIN_BOARD_SOURCE_NOT_CONFIGURED: &str =
    "stock_analysis.br174.board_source_not_configured.v1";
pub const DOMAIN_RELATION_EVIDENCE_SET: &str = "stock_analysis.br174.relation_evidence_set.v1";
pub const DOMAIN_BINDING_STATE: &str = "stock_analysis.br174.binding_state.v1";
pub const DOMAIN_RELATION_KEY: &str = "stock_analysis.br174.relation_key.v1";
pub const DOMAIN_RELATION_ATTEMPT: &str = "stock_analysis.br174.relation_attempt.v2";
pub const DOMAIN_SAMPLE_KEY: &str = "stock_analysis.br174.sample_key.v1";
pub const DOMAIN_EVALUATION_ATTEMPT: &str = "stock_analysis.br174.evaluation_attempt.v1";
pub const DOMAIN_OUTCOME_ATTEMPT: &str = "stock_analysis.br174.outcome_attempt.v3";
pub const DOMAIN_PROVIDER_CAPABILITY: &str = "stock_analysis.br174.provider_capability.v1";
pub const DOMAIN_REQUEST: &str = "stock_analysis.br174.request.v1";
pub const DOMAIN_REQUEST_EVIDENCE: &str = "stock_analysis.br174.request_evidence.v1";
pub const DOMAIN_GLOBAL_NEWS_REQUEST: &str = "stock_analysis.br174.global_news_request.v1";
pub const DOMAIN_BOARD_CONSTITUENT_REQUEST: &str = "stock_analysis.br174.board_request.v1";
pub const DOMAIN_T0_MARKET_REQUEST: &str = "stock_analysis.br174.t0_market_request.v1";
pub const DOMAIN_OUTCOME_MARKET_REQUEST: &str = "stock_analysis.br174.outcome_market_request.v2";
pub const DOMAIN_OUTCOME_TRADING_DATE_VECTOR: &str =
    "stock_analysis.br178.outcome_trading_dates.v1";
pub const DOMAIN_OUTCOME_DUE_DATABASE_OBJECT: &str =
    "stock_analysis.br178.outcome_due_database_object.v1";
pub const DOMAIN_OUTCOME_DUE_DATABASE_BINDING: &str =
    "stock_analysis.br178.outcome_due_database_binding.v1";
pub const DOMAIN_OUTCOME_AUDIT_PREFIX: &str = "stock_analysis.br178.selection_audit_prefix.v1";
pub const DOMAIN_OUTCOME_DUE_RECEIPT_TUPLE: &str =
    "stock_analysis.br178.outcome_due_receipt_tuple.v1";
pub const DOMAIN_VERIFIED_OUTCOME_DUE_SNAPSHOT: &str =
    "stock_analysis.br178.verified_outcome_due_snapshot.v1";
pub const DOMAIN_OUTCOME_CLAIM_DUE_BINDING: &str =
    "stock_analysis.br178.outcome_claim_due_binding.v1";
pub const DOMAIN_OUTCOME_CLAIM_STAGE: &str = "stock_analysis.br174.outcome_claim_stage.v2";
pub const DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE: &str =
    "stock_analysis.br178.outcome_provider_evidence.v1";
pub const DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS: &str =
    "stock_analysis.br174.outcome_transport_attempts.v1";
pub const DOMAIN_OUTCOME_PROVIDER_REQUEST: &str =
    "stock_analysis.br174.outcome_provider_request.v2";
pub const DOMAIN_RAW_SECURITY_IDENTITY: &str = "stock_analysis.br174.raw_security_identity.v1";
pub const DOMAIN_PROVIDER_AVAILABLE_EVIDENCE: &str =
    "stock_analysis.br174.provider_available_evidence.v1";
pub const DOMAIN_PROVIDER_ERROR_DETAIL: &str = "stock_analysis.br174.provider_error_detail.v1";
pub const DOMAIN_T0_FEATURE: &str = "stock_analysis.br174.t0_feature.v1";
pub const DOMAIN_ERROR_FINGERPRINT: &str = "stock_analysis.br174.error.v1";
pub const DOMAIN_RUN_LOGICAL_SUBJECT: &str = "stock_analysis.br174.run_logical_subject.v1";
pub const DOMAIN_RUN_ROW_LOGICAL_PK: &str = "stock_analysis.br174.run_row_logical_pk.v1";
pub const DOMAIN_RUN_ROW: &str = "stock_analysis.br174.run_row.v1";
pub const DOMAIN_STAGED_DB: &str = "stock_analysis.br174.staged_db.v1";
pub const DOMAIN_RUN_MANIFEST: &str = "stock_analysis.br174.run_manifest.v1";
pub const DOMAIN_COMMIT_RECEIPT: &str = "stock_analysis.br174.commit_receipt.v1";
pub const DOMAIN_PREPARED_AUDIT: &str = "stock_analysis.br174.prepared_audit_content.v1";
pub const DOMAIN_COMMITTED_AUDIT: &str = "stock_analysis.br174.committed_audit_content.v1";
pub const DOMAIN_SOURCE_BATCH_ATTEMPT_ROW: &str =
    "stock_analysis.br174.selection_source_batch_attempts_row.v2";
pub const DOMAIN_SOURCE_FACT_ROW: &str = "stock_analysis.br174.selection_source_fact_row.v1";
pub const DOMAIN_SOURCE_FACT_ATTEMPT_ROW: &str =
    "stock_analysis.br174.selection_source_fact_attempt_row.v1";
pub const DOMAIN_RELATION_ATTEMPT_ROW: &str =
    "stock_analysis.br174.selection_relation_attempts_row.v2";
pub const DOMAIN_EVALUATION_ATTEMPT_ROW: &str =
    "stock_analysis.br174.selection_evaluation_attempts_row.v2";
pub const DOMAIN_SAMPLE_ROW: &str = "stock_analysis.br174.selection_samples_row.v2";
pub const DOMAIN_REJECTION_ROW: &str = "stock_analysis.br174.selection_rejection_row.v1";
pub const DOMAIN_SAMPLE_OUTCOME_ROW: &str = "stock_analysis.br174.selection_sample_outcome_row.v1";
pub const DOMAIN_OUTCOME_ATTEMPT_ROW: &str =
    "stock_analysis.br174.selection_outcome_attempts_row.v3";
pub const DOMAIN_RECOVERY_ENVELOPE_ROW: &str =
    "stock_analysis.br174.selection_recovery_envelope_row.v1";
pub const DOMAIN_CONFIG_ACTIVATION_STAGE: &str = "stock_analysis.br174.config_activation_stage.v1";
pub const DOMAIN_SOURCE_INGRESS_STAGE: &str = "stock_analysis.br174.source_ingress_stage.v2";
pub const DOMAIN_GENERATION_STAGE: &str = "stock_analysis.br174.generation_stage.v3";
pub const DOMAIN_OUTCOME_STAGE: &str = "stock_analysis.br174.outcome_stage.v3";
pub const DOMAIN_CONFIG_ACTIVATION_PAYLOAD: &str =
    "stock_analysis.br174.config_activation_payload.v1";
pub const DOMAIN_INGRESS_PAYLOAD: &str = "stock_analysis.br174.ingress_payload.v1";
pub const DOMAIN_GENERATION_PAYLOAD: &str = "stock_analysis.br174.generation_payload.v1";
pub const DOMAIN_OUTCOME_PAYLOAD: &str = "stock_analysis.br174.outcome_payload.v2";
pub const DOMAIN_OUTCOME_CLAIM_PAYLOAD: &str = "stock_analysis.br174.outcome_claim_payload.v1";
pub const DOMAIN_LEGACY_TRIGGER_SET: &str = "stock_analysis.br174.legacy_trigger_set.v1";
pub const DOMAIN_LEGACY_CUTOVER_SNAPSHOT: &str = "stock_analysis.br174.legacy_cutover_snapshot.v1";

pub const GLOBAL_NEWS_REQUEST_PARAMETERS_SCHEMA: &str = "global-news-request-v1";
pub const GLOBAL_NEWS_SOURCE_FACT_SCHEMA: &str = "global-news-source-fact-v2";
pub const BOARD_CONSTITUENTS_REQUEST_PARAMETERS_SCHEMA: &str = "board-constituents-request-v1";
pub const T0_MARKET_REQUEST_PARAMETERS_SCHEMA: &str = "t0-market-request-v1";
pub const OUTCOME_MARKET_REQUEST_PARAMETERS_SCHEMA: &str = "outcome-market-request-v2";
pub const CONFIG_ACTIVATION_STAGE_PAYLOAD_SCHEMA: &str = "config-activation-stage-v1";
pub const SOURCE_INGRESS_STAGE_PAYLOAD_SCHEMA: &str = "source-ingress-stage-v2";
pub const GENERATION_STAGE_PAYLOAD_SCHEMA: &str = "generation-stage-v3";
pub const OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA: &str = "outcome-claim-stage-v2";
pub const OUTCOME_STAGE_PAYLOAD_SCHEMA: &str = "outcome-stage-v3";

pub const BOARD_BINDING_PROPOSAL_SCHEMA_VERSION: &str =
    "selection-provider-board-binding-proposal-v1";
pub const BOARD_BINDINGS_SCHEMA_VERSION: &str = "selection-provider-board-bindings-v1";
pub const BOARD_BINDING_VALIDITY_POLICY_VERSION: &str = "selection-board-binding-validity-v1";
pub const BOARD_CONNECTION_POLICY_VERSION: &str = "selection-board-tdx-production-v1";
pub const BOARD_AUDIT_ROOT_POLICY_VERSION: &str = "selection-board-audit-root-v1";
pub const BOARD_AUDIT_ROOT_RELATIVE_PATH: &str = "data/audit/production";
pub const BOARD_DIRECTORY_PROVIDER: &str = "tdx";
pub const BOARD_DIRECTORY_SOURCE: &str = "tdx-block-files";
pub const BOARD_GATEWAY_CONSTRUCTOR: &str = "BoardDataGateway::production_tdx";
pub const BOARD_RESOLVER_POLICY: &str = "magic_tdx_production_resolver_v1";
pub const BOARD_ENDPOINT_OVERRIDE_POLICY: &str = "forbidden";
pub const BOARD_AUDIT_COMMAND_VERSION: &str = "selection-board-binding-audit-v1";
pub const BOARD_AUDIT_CAPTURE_MAX_AGE_SECS: i64 = 300;
pub const BOARD_DIRECTORY_REQUEST_LIMIT: u32 = 10_000;

pub const TABLE_SOURCE_BATCH_ATTEMPT: u8 = 1;
pub const TABLE_SOURCE_FACT: u8 = 2;
pub const TABLE_SOURCE_FACT_ATTEMPT: u8 = 3;
pub const TABLE_RELATION_ATTEMPT: u8 = 4;
pub const TABLE_EVALUATION_ATTEMPT: u8 = 5;
pub const TABLE_SAMPLE: u8 = 6;
pub const TABLE_REJECTION: u8 = 7;
pub const TABLE_SAMPLE_OUTCOME: u8 = 8;
pub const TABLE_OUTCOME_ATTEMPT: u8 = 9;
pub const TABLE_RECOVERY_ENVELOPE: u8 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaV2Error {
    pub code: &'static str,
    pub detail: String,
}

impl SchemaV2Error {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for SchemaV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for SchemaV2Error {}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, SchemaV2Error> {
    serde_json::to_string(value)
        .map_err(|error| SchemaV2Error::new("canonical_json_failed", error.to_string()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sha256_json<T: Serialize>(value: &T) -> Result<String, SchemaV2Error> {
    Ok(sha256_bytes(canonical_json(value)?.as_bytes()))
}

pub fn sha256_domain_bytes(domain: &[u8], value: &impl Serialize) -> Result<String, SchemaV2Error> {
    let json = canonical_json(value)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(json.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

pub fn canonical_f64(value: f64) -> Result<String, SchemaV2Error> {
    if !value.is_finite() || (value == 0.0 && value.is_sign_negative()) {
        return Err(SchemaV2Error::new(
            "invalid_canonical_decimal",
            "decimal must be finite and must not be negative zero",
        ));
    }
    Ok(value.to_string())
}

fn parse_canonical_f64(field: &'static str, value: &str) -> Result<f64, SchemaV2Error> {
    let parsed = value.parse::<f64>().map_err(|error| {
        SchemaV2Error::new(
            "invalid_canonical_decimal",
            format!("{field} is not a canonical finite decimal: {error}"),
        )
    })?;
    let canonical = canonical_f64(parsed).map_err(|error| {
        SchemaV2Error::new(
            error.code,
            format!(
                "{field} is not a canonical finite decimal: {}",
                error.detail
            ),
        )
    })?;
    if canonical != value {
        return Err(SchemaV2Error::new(
            "invalid_canonical_decimal",
            format!("{field} must use canonical decimal {canonical}, got {value}"),
        ));
    }
    Ok(parsed)
}

fn require_domain(actual: &str, expected: &'static str) -> Result<(), SchemaV2Error> {
    if actual == expected {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            "domain_mismatch",
            format!("expected {expected}, got {actual}"),
        ))
    }
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), SchemaV2Error> {
    if value.is_empty() {
        Err(SchemaV2Error::new(
            "required_field_empty",
            format!("{field} is empty"),
        ))
    } else {
        Ok(())
    }
}

fn require_hash(value: &str, field: &'static str) -> Result<(), SchemaV2Error> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            "invalid_sha256",
            format!("{field} must be lowercase 64-hex"),
        ))
    }
}

fn require_trim_stable_non_empty(value: &str, field: &'static str) -> Result<(), SchemaV2Error> {
    require_non_empty(value, field)?;
    if value == value.trim() && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            "noncanonical_text",
            format!("{field} must be trim-stable and contain no control characters"),
        ))
    }
}

fn require_canonical_uuid_v7(value: &str, field: &'static str) -> Result<(), SchemaV2Error> {
    let bytes = value.as_bytes();
    let hyphens_are_canonical = bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-');
    let digits_are_canonical = bytes.iter().enumerate().all(|(index, byte)| {
        [8, 13, 18, 23].contains(&index) || byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
    });
    let version_is_v7 = bytes.get(14) == Some(&b'7');
    let variant_is_rfc4122 = bytes
        .get(19)
        .is_some_and(|byte| matches!(*byte, b'8' | b'9' | b'a' | b'b'));
    if hyphens_are_canonical && digits_are_canonical && version_is_v7 && variant_is_rfc4122 {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            "invalid_uuid_v7",
            format!("{field} must be a lowercase canonical RFC 9562 UUIDv7"),
        ))
    }
}

fn parse_canonical_nanos_utc(
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, SchemaV2Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| {
            SchemaV2Error::new("invalid_rfc3339_nanos_utc", format!("{field}: {error}"))
        })?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Nanos, true) == value {
        Ok(parsed)
    } else {
        Err(SchemaV2Error::new(
            "noncanonical_rfc3339_nanos_utc",
            format!("{field} must use exact UTC nanoseconds and Z"),
        ))
    }
}

fn parse_canonical_date(value: &str, field: &'static str) -> Result<NaiveDate, SchemaV2Error> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        SchemaV2Error::new("invalid_canonical_date", format!("{field}: {error}"))
    })?;
    if parsed.format("%Y-%m-%d").to_string() == value {
        Ok(parsed)
    } else {
        Err(SchemaV2Error::new(
            "noncanonical_date",
            format!("{field} must use exact YYYY-MM-DD"),
        ))
    }
}

fn parse_board_observed_at(
    value: &str,
    field: &'static str,
) -> Result<DateTime<Utc>, SchemaV2Error> {
    require_trim_stable_non_empty(value, field)?;
    if let Some(decimal) = value.strip_prefix("unix-ms:") {
        if decimal.is_empty()
            || !decimal.bytes().all(|byte| byte.is_ascii_digit())
            || (decimal.len() > 1 && decimal.starts_with('0'))
        {
            return Err(SchemaV2Error::new(
                "invalid_board_observed_at",
                format!("{field} is not canonical unix-ms unsigned decimal"),
            ));
        }
        let milliseconds = decimal.parse::<u64>().map_err(|error| {
            SchemaV2Error::new("invalid_board_observed_at", format!("{field}: {error}"))
        })?;
        let milliseconds = i64::try_from(milliseconds).map_err(|_| {
            SchemaV2Error::new(
                "invalid_board_observed_at",
                format!("{field} is outside supported UTC range"),
            )
        })?;
        return DateTime::from_timestamp_millis(milliseconds).ok_or_else(|| {
            SchemaV2Error::new(
                "invalid_board_observed_at",
                format!("{field} is outside supported UTC range"),
            )
        });
    }
    parse_canonical_nanos_utc(value, field)
}

fn validate_fixed_board_binding(
    chain_id: &str,
    provider: &str,
    kind: ProviderBoardKind,
    code: &str,
    name: &str,
) -> Result<(), SchemaV2Error> {
    require_trim_stable_non_empty(chain_id, "chain_id")?;
    require_trim_stable_non_empty(name, "board_name")?;
    if provider != BOARD_DIRECTORY_PROVIDER {
        return Err(SchemaV2Error::new(
            "board_provider_mismatch",
            format!("provider must be {BOARD_DIRECTORY_PROVIDER}"),
        ));
    }
    let expected_code = format!("tdx:{}:{name}", kind.as_str());
    if code != expected_code {
        return Err(SchemaV2Error::new(
            "board_code_mismatch",
            format!("code must equal {expected_code}"),
        ));
    }
    Ok(())
}

fn require_pair<T, U>(
    left: &Option<T>,
    right: &Option<U>,
    code: &'static str,
) -> Result<(), SchemaV2Error> {
    if left.is_some() == right.is_some() {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            code,
            "paired fields must both be NULL or non-NULL",
        ))
    }
}

fn validate_canonical_json_hash<T>(json: &str, expected_hash: &str) -> Result<T, SchemaV2Error>
where
    T: DeserializeOwned + Serialize,
{
    require_hash(expected_hash, "canonical_json_hash")?;
    let value: T = serde_json::from_str(json)
        .map_err(|error| SchemaV2Error::new("typed_json_invalid", error.to_string()))?;
    let canonical = canonical_json(&value)?;
    if canonical != json {
        return Err(SchemaV2Error::new(
            "typed_json_noncanonical",
            "stored JSON bytes differ from canonical typed serialization",
        ));
    }
    if sha256_bytes(json.as_bytes()) != expected_hash {
        return Err(SchemaV2Error::new(
            "typed_json_hash_mismatch",
            "stored JSON hash does not bind the canonical typed bytes",
        ));
    }
    Ok(value)
}

fn validate_canonical_json<T>(json: &str) -> Result<T, SchemaV2Error>
where
    T: DeserializeOwned + Serialize,
{
    let value: T = serde_json::from_str(json)
        .map_err(|error| SchemaV2Error::new("typed_json_invalid", error.to_string()))?;
    if canonical_json(&value)? != json {
        return Err(SchemaV2Error::new(
            "typed_json_noncanonical",
            "stored JSON bytes differ from canonical typed serialization",
        ));
    }
    Ok(value)
}

macro_rules! token_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => token_enum!(@snake $variant)),+
                }
            }
        }
    };
    (@snake Available) => { "available" };
    (@snake VerifiedEmpty) => { "verified_empty" };
    (@snake Unavailable) => { "unavailable" };
    (@snake Accepted) => { "accepted" };
    (@snake Replay) => { "replay" };
    (@snake Conflict) => { "conflict" };
    (@snake Admitted) => { "admitted" };
    (@snake Rejected) => { "rejected" };
    (@snake DirectNotApplicable) => { "direct_not_applicable" };
    (@snake NotConfigured) => { "not_configured" };
    (@snake Verified) => { "verified" };
    (@snake DirectMention) => { "direct_mention" };
    (@snake ProviderBoardConstituent) => { "provider_board_constituent" };
    (@snake ConfigActivation) => { "config_activation" };
    (@snake IngressRun) => { "ingress_run" };
    (@snake GenerationRun) => { "generation_run" };
    (@snake OutcomeClaim) => { "outcome_claim" };
    (@snake OutcomeRun) => { "outcome_run" };
    (@snake Activated) => { "activated" };
    (@snake Claimed) => { "claimed" };
    (@snake Completed) => { "completed" };
    (@snake VerifiedNoRelation) => { "verified_no_relation" };
    (@snake PendingDependency) => { "pending_dependency" };
    (@snake FailedNonRetryable) => { "failed_non_retryable" };
    (@snake Settled) => { "settled" };
    (@snake ExpectedWait) => { "expected_wait" };
    (@snake FailedRetryable) => { "failed_retryable" };
    (@snake Error) => { "error" };
    (@snake T0Close) => { "t0_close" };
    (@snake D1Settled) => { "d1_settled" };
    (@snake D3Settled) => { "d3_settled" };
    (@snake D5Settled) => { "d5_settled" };
    (@snake MarketSessionUnsettled) => { "market_session_unsettled" };
    (@snake SettledBarMissing) => { "settled_bar_missing" };
    (@snake ProviderUnavailable) => { "provider_unavailable" };
    (@snake ProviderInvalidData) => { "provider_invalid_data" };
    (@snake EvidenceIncomplete) => { "evidence_incomplete" };
    (@snake EvidenceStale) => { "evidence_stale" };
    (@snake EvidenceConflict) => { "evidence_conflict" };
    (@snake ManualConfirmationRequired) => { "manual_confirmation_required" };
    (@snake Transport) => { "transport" };
    (@snake Protocol) => { "protocol" };
    (@snake Timeout) => { "timeout" };
    (@snake InvalidData) => { "invalid_data" };
    (@snake Unsupported) => { "unsupported" };
    (@snake Integrity) => { "integrity" };
    (@snake BoardConstituents) => { "board_constituents" };
    (@snake T0MarketBundle) => { "t0_market_bundle" };
    (@snake OutcomeDailyBars) => { "outcome_daily_bars" };
    (@snake Complete) => { "complete" };
    (@snake Title) => { "title" };
    (@snake Summary) => { "summary" };
    (@snake Content) => { "content" };
    (@snake ExactCode) => { "exact_code" };
    (@snake ExactName) => { "exact_name" };
    (@snake Industry) => { "industry" };
    (@snake Concept) => { "concept" };
    (@snake Intraday) => { "intraday" };
    (@snake PostClose) => { "post_close" };
    (@snake GlobalNews) => { "global_news" };
    (@snake T0MarketEvidence) => { "t0_market_evidence" };
    (@snake OutcomeMarketEvidence) => { "outcome_market_evidence" };
    (@snake None) => { "none" };
    (@snake Day) => { "day" };
    (@snake HardRejected) => { "hard_rejected" };
    (@snake Insert) => { "insert" };
    (@snake Update) => { "update" };
    (@snake Delete) => { "delete" };
}

token_enum!(FeedStatusKind {
    Available,
    VerifiedEmpty,
    Unavailable
});
token_enum!(SourceFactAttemptResult {
    Accepted,
    Replay,
    Conflict
});
token_enum!(IngressDecision { Admitted, Rejected });
token_enum!(BindingStateKind {
    DirectNotApplicable,
    NotConfigured,
    Verified
});
token_enum!(RelationKind {
    DirectMention,
    ProviderBoardConstituent
});
token_enum!(SubjectKind {
    ConfigActivation,
    IngressRun,
    GenerationRun,
    OutcomeClaim,
    OutcomeRun
});
token_enum!(RunStatus {
    Activated,
    Claimed,
    Completed,
    VerifiedNoRelation,
    PendingDependency,
    FailedNonRetryable,
    Settled,
    ExpectedWait,
    FailedRetryable
});
token_enum!(OutcomeAttemptResult {
    Settled,
    ExpectedWait,
    Error
});
token_enum!(OutcomePhase {
    T0Close,
    D1Settled,
    D3Settled,
    D5Settled
});
token_enum!(OutcomeReasonCodeV1 {
    MarketSessionUnsettled,
    SettledBarMissing,
    ProviderUnavailable,
    ProviderInvalidData,
    EvidenceIncomplete,
    EvidenceStale,
    EvidenceConflict,
    ManualConfirmationRequired
});
token_enum!(ProviderErrorKind {
    Transport,
    Protocol,
    Timeout,
    InvalidData,
    Unsupported,
    Integrity
});
token_enum!(ProviderEvidenceKind {
    BoardConstituents,
    T0MarketBundle,
    OutcomeDailyBars
});
token_enum!(FeedBatchQuality { Complete });
token_enum!(DirectMentionField {
    Title,
    Summary,
    Content
});
token_enum!(MentionKind {
    ExactCode,
    ExactName
});
token_enum!(ProviderBoardKind { Industry, Concept });
token_enum!(EvaluationWindow {
    Intraday,
    PostClose
});
token_enum!(RequestKind {
    GlobalNews,
    BoardConstituents,
    T0MarketEvidence,
    OutcomeMarketEvidence
});
token_enum!(AdjustmentKind { None });
token_enum!(DailyIntervalKind { Day });
token_enum!(TerminalDecisionKind {
    Admitted,
    HardRejected
});
token_enum!(LegacyTriggerOperation {
    Insert,
    Update,
    Delete
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureCodeV1 {
    Transport,
    Protocol,
    Timeout,
    InvalidData,
    Unsupported,
    Integrity,
    SettledBarMissing,
    Unmapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderErrorMapping {
    pub error_kind: ProviderErrorKind,
    pub diagnostic_code: &'static str,
    pub retryable: bool,
    pub invariant_id: Option<&'static str>,
}

impl ProviderFailureCodeV1 {
    pub fn mapping(self) -> ProviderErrorMapping {
        match self {
            Self::Transport => ProviderErrorMapping {
                error_kind: ProviderErrorKind::Transport,
                diagnostic_code: "transport_failure",
                retryable: true,
                invariant_id: None,
            },
            Self::Protocol => ProviderErrorMapping {
                error_kind: ProviderErrorKind::Protocol,
                diagnostic_code: "protocol_failure",
                retryable: false,
                invariant_id: None,
            },
            Self::Timeout => ProviderErrorMapping {
                error_kind: ProviderErrorKind::Timeout,
                diagnostic_code: "provider_timeout",
                retryable: true,
                invariant_id: None,
            },
            Self::InvalidData => ProviderErrorMapping {
                error_kind: ProviderErrorKind::InvalidData,
                diagnostic_code: "provider_invalid_data",
                retryable: false,
                invariant_id: None,
            },
            Self::Unsupported => ProviderErrorMapping {
                error_kind: ProviderErrorKind::Unsupported,
                diagnostic_code: "provider_capability_unsupported",
                retryable: false,
                invariant_id: None,
            },
            Self::Integrity => ProviderErrorMapping {
                error_kind: ProviderErrorKind::Integrity,
                diagnostic_code: "provider_integrity_failure",
                retryable: false,
                invariant_id: Some("provider-error-codes-v1"),
            },
            Self::SettledBarMissing => ProviderErrorMapping {
                error_kind: ProviderErrorKind::InvalidData,
                diagnostic_code: "settled_bar_missing",
                retryable: true,
                invariant_id: None,
            },
            Self::Unmapped => ProviderErrorMapping {
                error_kind: ProviderErrorKind::Integrity,
                diagnostic_code: "provider_error_mapping_missing",
                retryable: false,
                invariant_id: Some("provider-error-codes-v1"),
            },
        }
    }
}

macro_rules! canonical_struct {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(pub $field: $ty),*
        }
    };
}

canonical_struct!(SourceFactKeyPreimage {
    domain: String,
    provider_source: String,
    item_id: String,
});

canonical_struct!(SourceFactContentPreimage {
    domain: String,
    provider_source: String,
    item_id: String,
    title: String,
    summary: Option<String>,
    content: Option<String>,
    publisher: String,
    canonical_url: String,
    published_at_rfc3339_nanos_utc: String,
    instruments_sorted: Vec<String>,
    topics_sorted: Vec<String>,
    language: String,
    record_source: String,
    record_source_at: Option<String>,
});

canonical_struct!(AcquiredGlobalNewsRecordPreimage {
    domain: String,
    source_fact_key: String,
    provider_content_hash: String,
    record: SourceFactContentPreimage,
    record_provider: String,
    record_source: String,
    record_source_at: Option<String>,
    record_observed_at: String,
    record_batch_id: String,
    record_batch_content_hash: String,
});

canonical_struct!(SourceFactConflictPreimage {
    domain: String,
    source_fact_key: String,
    authoritative_provider_content_hash: String,
    attempted_provider_content_hash: String,
});

canonical_struct!(SourceFactAttemptPreimage {
    domain: String,
    ingress_run_id: String,
    source_fact_key: String,
    source_batch_attempt_id: String,
    provider_ordinal: u32,
    source_batch_id: String,
    record_batch_id: String,
    observed_at: String,
    batch_evidence_hash: String,
});

canonical_struct!(FeedAttemptKeyPreimage {
    domain: String,
    ingress_run_id: String,
    feed_identity: String,
});

canonical_struct!(FeedBatchEvidencePreimage {
    domain: String,
    feed_identity: String,
    provider: String,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    batch_quality: FeedBatchQuality,
});

canonical_struct!(FeedAvailableEvidencePreimage {
    domain: String,
    feed_identity: String,
    provider: Option<String>,
    source: Option<String>,
    source_at: Option<String>,
    observed_at: Option<String>,
    batch_id: Option<String>,
    batch_content_hash: Option<String>,
});

canonical_struct!(FeedSourceRecordHashPreimage {
    domain: String,
    provider_ordinal: u32,
    source_fact_key: String,
    provider_content_hash: String,
});

canonical_struct!(FeedSourceContentPreimage {
    domain: String,
    feed_identity: String,
    evidence_hash: String,
    record_hashes_in_provider_order: Vec<String>,
});

canonical_struct!(FeedAttemptContentPreimage {
    domain: String,
    feed_identity: String,
    request_hash: String,
    request_evidence_hash: String,
    status_kind: FeedStatusKind,
    record_count: Option<u32>,
    evidence_hash: Option<String>,
    source_content_hash: Option<String>,
    available_evidence_hash: Option<String>,
    failed_stage: Option<String>,
    reason_code: Option<String>,
    retryable: Option<bool>,
    detail_hash: Option<String>,
    error_fingerprint: Option<String>,
});

impl FeedAttemptContentPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_FEED_ATTEMPT_CONTENT)?;
        require_hash(&self.request_hash, "request_hash")?;
        require_hash(&self.request_evidence_hash, "request_evidence_hash")?;
        match self.status_kind {
            FeedStatusKind::Available => {
                if self.record_count.is_none_or(|count| count == 0)
                    || self.evidence_hash.is_none()
                    || self.source_content_hash.is_none()
                    || self.available_evidence_hash != self.evidence_hash
                    || self.failed_stage.is_some()
                    || self.reason_code.is_some()
                    || self.retryable.is_some()
                    || self.detail_hash.is_some()
                    || self.error_fingerprint.is_some()
                {
                    return Err(SchemaV2Error::new(
                        "invalid_available_feed_matrix",
                        "Available requires positive count, complete evidence/content, and no failure fields",
                    ));
                }
                require_hash(
                    self.evidence_hash.as_deref().expect("checked above"),
                    "evidence_hash",
                )?;
                require_hash(
                    self.source_content_hash.as_deref().expect("checked above"),
                    "source_content_hash",
                )?;
            }
            FeedStatusKind::VerifiedEmpty => {
                if self.record_count != Some(0)
                    || self.evidence_hash.is_none()
                    || self.source_content_hash.is_none()
                    || self.available_evidence_hash != self.evidence_hash
                    || self.failed_stage.is_some()
                    || self.reason_code.is_some()
                    || self.retryable.is_some()
                    || self.detail_hash.is_some()
                    || self.error_fingerprint.is_some()
                {
                    return Err(SchemaV2Error::new(
                        "invalid_verified_empty_feed_matrix",
                        "VerifiedEmpty requires zero count, complete evidence/content, and no failure fields",
                    ));
                }
                require_hash(
                    self.evidence_hash.as_deref().expect("checked above"),
                    "evidence_hash",
                )?;
                require_hash(
                    self.source_content_hash.as_deref().expect("checked above"),
                    "source_content_hash",
                )?;
            }
            FeedStatusKind::Unavailable => {
                if self.record_count.is_some()
                    || self.evidence_hash.is_some()
                    || self.source_content_hash.is_some()
                    || self.failed_stage.as_deref().is_none_or(str::is_empty)
                    || self.reason_code.as_deref().is_none_or(str::is_empty)
                    || self.retryable.is_none()
                    || self.detail_hash.is_none()
                    || self.error_fingerprint.is_none()
                {
                    return Err(SchemaV2Error::new(
                        "invalid_unavailable_feed_matrix",
                        "Unavailable requires stage/reason/retryability/detail/fingerprint and no complete batch",
                    ));
                }
                require_hash(
                    self.detail_hash.as_deref().expect("checked above"),
                    "detail_hash",
                )?;
                require_hash(
                    self.error_fingerprint.as_deref().expect("checked above"),
                    "error_fingerprint",
                )?;
                if let Some(hash) = &self.available_evidence_hash {
                    require_hash(hash, "available_evidence_hash")?;
                }
            }
        }
        Ok(())
    }
}

canonical_struct!(RegisteredFeedConfigurationPreimage {
    domain: String,
    gateway_provider: String,
    provider_id: String,
    source_contract: String,
    capability_name: String,
    max_limit: u32,
    upstream_revision: String,
});

canonical_struct!(RegisteredFeedIdentityPreimage {
    domain: String,
    feed_name: String,
    gateway_provider: String,
    configuration_hash: String,
});

canonical_struct!(RegisteredFeedEntryPreimage {
    ordinal: u32,
    feed_identity: String,
    gateway_provider: String,
    capability_name: String,
    configuration_hash: String,
});

canonical_struct!(RegisteredFeedSnapshotPreimage {
    domain: String,
    feeds_sorted: Vec<RegisteredFeedEntryPreimage>,
});

canonical_struct!(SourceBatchContentPreimage {
    domain: String,
    registered_feed_snapshot_hash: String,
    feed_attempt_hashes_in_registered_feed_order: Vec<String>,
    source_record_hashes_in_feed_then_provider_order: Vec<String>,
    event_projection_ids_in_feed_then_provider_order: Vec<String>,
    aggregator_observed_at_rfc3339_nanos_utc: String,
});

canonical_struct!(IngressGateInputPreimage {
    domain: String,
    source_fact_key: String,
    config_activation_run_id: String,
    config_hash: String,
    provider_published_at_rfc3339_nanos_utc: String,
    record_observed_at: String,
    batch_observed_at: String,
    batch_content_hash: String,
    evaluated_at_rfc3339_nanos_utc: String,
    freshness_max_age_secs: u64,
    future_tolerance_secs: u64,
    gate_version: String,
});

canonical_struct!(IngressGateReceiptPreimage {
    domain: String,
    ingress_run_id: String,
    source_fact_key: String,
    ingress_gate_input_hash: String,
    decision: IngressDecision,
    reason_code: Option<String>,
    retryable: Option<bool>,
    evaluated_at_rfc3339_nanos_utc: String,
});

impl IngressGateReceiptPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_INGRESS_GATE_RECEIPT)?;
        match self.decision {
            IngressDecision::Admitted if self.reason_code.is_none() && self.retryable.is_none() => {
                Ok(())
            }
            IngressDecision::Rejected
                if self
                    .reason_code
                    .as_deref()
                    .is_some_and(|value| !value.is_empty())
                    && self.retryable == Some(false) =>
            {
                Ok(())
            }
            _ => Err(SchemaV2Error::new(
                "invalid_ingress_decision_matrix",
                "admitted has NULL reason/retryable; rejected has reason and retryable=false",
            )),
        }
    }
}

pub fn feed_source_content_hash(
    feed_identity: &str,
    evidence_hash: &str,
    records_in_provider_order: &[FeedSourceRecordHashPreimage],
) -> Result<String, SchemaV2Error> {
    require_hash(evidence_hash, "evidence_hash")?;
    for (expected, record) in records_in_provider_order.iter().enumerate() {
        if record.provider_ordinal != expected as u32 {
            return Err(SchemaV2Error::new(
                "non_contiguous_provider_ordinal",
                format!(
                    "expected ordinal {expected}, got {}",
                    record.provider_ordinal
                ),
            ));
        }
        require_domain(&record.domain, DOMAIN_FEED_SOURCE_RECORD)?;
    }
    let record_hashes = records_in_provider_order
        .iter()
        .map(sha256_json)
        .collect::<Result<Vec<_>, _>>()?;
    sha256_json(&FeedSourceContentPreimage {
        domain: DOMAIN_FEED_SOURCE_CONTENT.to_string(),
        feed_identity: feed_identity.to_string(),
        evidence_hash: evidence_hash.to_string(),
        record_hashes_in_provider_order: record_hashes,
    })
}

pub fn source_batch_content_hash(
    registered_feed_snapshot_hash: &str,
    feed_attempt_hashes_in_registered_feed_order: Vec<String>,
    source_record_hashes_in_feed_then_provider_order: Vec<String>,
    event_projection_ids_in_feed_then_provider_order: Vec<String>,
    aggregator_observed_at_rfc3339_nanos_utc: String,
) -> Result<String, SchemaV2Error> {
    require_hash(
        registered_feed_snapshot_hash,
        "registered_feed_snapshot_hash",
    )?;
    if source_record_hashes_in_feed_then_provider_order.len()
        != event_projection_ids_in_feed_then_provider_order.len()
    {
        return Err(SchemaV2Error::new(
            "source_batch_projection_count_mismatch",
            "every source record must have exactly one event projection identity",
        ));
    }
    sha256_json(&SourceBatchContentPreimage {
        domain: DOMAIN_SOURCE_BATCH_CONTENT.to_string(),
        registered_feed_snapshot_hash: registered_feed_snapshot_hash.to_string(),
        feed_attempt_hashes_in_registered_feed_order,
        source_record_hashes_in_feed_then_provider_order,
        event_projection_ids_in_feed_then_provider_order,
        aggregator_observed_at_rfc3339_nanos_utc,
    })
}

canonical_struct!(ProposalBindingPreimage {
    chain_id: String,
    provider: String,
    kind: ProviderBoardKind,
    code: String,
    name: String,
});

canonical_struct!(BoardBindingProposalInputPreimage {
    domain: String,
    schema_version: String,
    validity_policy_version: String,
    valid_from_rfc3339_nanos_utc: String,
    expires_at_rfc3339_nanos_utc: String,
    reviewed_by: String,
    reviewed_at_rfc3339_nanos_utc: String,
    bindings_sorted: Vec<ProposalBindingPreimage>,
});

impl BoardBindingProposalInputPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_BINDING_PROPOSAL)?;
        if self.schema_version != BOARD_BINDING_PROPOSAL_SCHEMA_VERSION
            || self.validity_policy_version != BOARD_BINDING_VALIDITY_POLICY_VERSION
        {
            return Err(SchemaV2Error::new(
                "board_proposal_header_mismatch",
                "proposal schema or validity policy version is not frozen BR-174 v1",
            ));
        }
        require_trim_stable_non_empty(&self.reviewed_by, "reviewed_by")?;
        let reviewed_at = parse_canonical_nanos_utc(
            &self.reviewed_at_rfc3339_nanos_utc,
            "reviewed_at_rfc3339_nanos_utc",
        )?;
        let valid_from = parse_canonical_nanos_utc(
            &self.valid_from_rfc3339_nanos_utc,
            "valid_from_rfc3339_nanos_utc",
        )?;
        let expires_at = parse_canonical_nanos_utc(
            &self.expires_at_rfc3339_nanos_utc,
            "expires_at_rfc3339_nanos_utc",
        )?;
        if reviewed_at > valid_from
            || valid_from >= expires_at
            || expires_at - valid_from > chrono::Duration::days(30)
        {
            return Err(SchemaV2Error::new(
                "board_proposal_validity_invalid",
                "reviewed_at <= valid_from < expires_at <= valid_from + 30 days is required",
            ));
        }
        let mut previous_key: Option<(String, String, String, String, String)> = None;
        for binding in &self.bindings_sorted {
            validate_fixed_board_binding(
                &binding.chain_id,
                &binding.provider,
                binding.kind,
                &binding.code,
                &binding.name,
            )?;
            let key = (
                binding.chain_id.clone(),
                binding.provider.clone(),
                binding.kind.as_str().to_owned(),
                binding.code.clone(),
                binding.name.clone(),
            );
            if previous_key
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                return Err(SchemaV2Error::new(
                    "board_proposal_bindings_not_sorted_unique",
                    "proposal bindings must be strictly sorted by the frozen tuple",
                ));
            }
            previous_key = Some(key);
        }
        Ok(())
    }

    pub fn proposal_input_content_hash(&self) -> Result<String, SchemaV2Error> {
        self.validate()?;
        sha256_json(self)
    }
}

canonical_struct!(BoardConnectionPolicyPreimage {
    domain: String,
    version: String,
    provider: String,
    gateway_constructor: String,
    resolver_policy: String,
    endpoint_override: String,
});

impl BoardConnectionPolicyPreimage {
    pub fn fixed() -> Self {
        Self {
            domain: DOMAIN_BOARD_CONNECTION_POLICY.into(),
            version: BOARD_CONNECTION_POLICY_VERSION.into(),
            provider: BOARD_DIRECTORY_PROVIDER.into(),
            gateway_constructor: BOARD_GATEWAY_CONSTRUCTOR.into(),
            resolver_policy: BOARD_RESOLVER_POLICY.into(),
            endpoint_override: BOARD_ENDPOINT_OVERRIDE_POLICY.into(),
        }
    }

    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        if self != &Self::fixed() {
            return Err(SchemaV2Error::new(
                "board_connection_policy_mismatch",
                "connection policy must equal the fixed BR-174 production TDX policy",
            ));
        }
        Ok(())
    }

    pub fn connection_policy_hash(&self) -> Result<String, SchemaV2Error> {
        self.validate()?;
        sha256_json(self)
    }
}

canonical_struct!(BoardAuditRootBindingPreimage {
    domain: String,
    version: String,
    repository_relative_path: String,
});

impl BoardAuditRootBindingPreimage {
    pub fn fixed() -> Self {
        Self {
            domain: DOMAIN_BOARD_AUDIT_ROOT.into(),
            version: BOARD_AUDIT_ROOT_POLICY_VERSION.into(),
            repository_relative_path: BOARD_AUDIT_ROOT_RELATIVE_PATH.into(),
        }
    }

    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        if self != &Self::fixed() {
            return Err(SchemaV2Error::new(
                "board_audit_root_mismatch",
                "audit root must equal the fixed BR-174 production root",
            ));
        }
        Ok(())
    }

    pub fn audit_root_binding_hash(&self) -> Result<String, SchemaV2Error> {
        self.validate()?;
        sha256_json(self)
    }
}

canonical_struct!(ProductionEvidencePathBindingPreimage {
    domain: String,
    kind: String,
    source: String,
    canonical_absolute_path: String,
});

impl ProductionEvidencePathBindingPreimage {
    pub fn production_evidence_path_hash(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_PRODUCTION_EVIDENCE_PATH)?;
        require_trim_stable_non_empty(&self.kind, "production_evidence_kind")?;
        if self.source != "fixed_cargo_manifest_dir" {
            return Err(SchemaV2Error::new(
                "production_evidence_path_source_mismatch",
                "production evidence paths must be anchored to fixed_cargo_manifest_dir",
            ));
        }
        require_trim_stable_non_empty(&self.canonical_absolute_path, "canonical_absolute_path")?;
        sha256_json(self)
    }
}

canonical_struct!(DirectoryRecordSourceEvidencePreimage {
    provider: String,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
});

canonical_struct!(DirectoryBoardRecordPreimage {
    domain: String,
    provider_ordinal: u32,
    code: String,
    name: String,
    kind: ProviderBoardKind,
    member_count: u32,
    evidence: DirectoryRecordSourceEvidencePreimage,
});

impl DirectoryBoardRecordPreimage {
    pub fn directory_record_hash(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_DIRECTORY_RECORD)?;
        require_trim_stable_non_empty(&self.name, "directory_record.name")?;
        if self.member_count == 0 {
            return Err(SchemaV2Error::new(
                "directory_member_count_invalid",
                "directory record member_count must be positive",
            ));
        }
        let expected_code = format!("tdx:{}:{}", self.kind.as_str(), self.name);
        if self.code != expected_code {
            return Err(SchemaV2Error::new(
                "directory_record_code_mismatch",
                format!("directory record code must equal {expected_code}"),
            ));
        }
        sha256_json(self)
    }
}

canonical_struct!(DirectoryBatchContentPreimage {
    domain: String,
    category: ProviderBoardKind,
    provider: String,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    records_in_provider_order: Vec<DirectoryBoardRecordPreimage>,
});

impl DirectoryBatchContentPreimage {
    pub fn validate(&self, recorded_at: DateTime<Utc>) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_DIRECTORY_BATCH)?;
        if self.provider != BOARD_DIRECTORY_PROVIDER
            || self.source != BOARD_DIRECTORY_SOURCE
            || self.source_at.is_some()
        {
            return Err(SchemaV2Error::new(
                "directory_batch_source_mismatch",
                "directory batch must retain exact tdx/tdx-block-files/NULL source evidence",
            ));
        }
        require_trim_stable_non_empty(&self.batch_id, "directory_batch.batch_id")?;
        let observed_at =
            parse_board_observed_at(&self.observed_at, "directory_batch.observed_at")?;
        let age = recorded_at.signed_duration_since(observed_at);
        if age < chrono::Duration::zero()
            || age > chrono::Duration::seconds(BOARD_AUDIT_CAPTURE_MAX_AGE_SECS)
        {
            return Err(SchemaV2Error::new(
                "directory_batch_freshness_invalid",
                "directory observed_at must be no later than and at most 300 seconds before recorded_at",
            ));
        }
        if self.records_in_provider_order.is_empty()
            || self.records_in_provider_order.len()
                >= usize::try_from(BOARD_DIRECTORY_REQUEST_LIMIT)
                    .expect("fixed board request limit fits usize")
        {
            return Err(SchemaV2Error::new(
                "directory_batch_cardinality_invalid",
                "directory batch must contain 1..9999 complete records",
            ));
        }

        let mut triples = BTreeSet::new();
        for (ordinal, record) in self.records_in_provider_order.iter().enumerate() {
            if record.provider_ordinal
                != u32::try_from(ordinal).map_err(|_| {
                    SchemaV2Error::new(
                        "directory_record_ordinal_overflow",
                        "provider ordinal does not fit u32",
                    )
                })?
            {
                return Err(SchemaV2Error::new(
                    "directory_record_ordinal_invalid",
                    "directory record ordinals must be contiguous provider order",
                ));
            }
            record.directory_record_hash()?;
            if record.kind != self.category {
                return Err(SchemaV2Error::new(
                    "directory_record_category_mismatch",
                    "record kind must equal its enclosing directory category",
                ));
            }
            if record.evidence.provider != self.provider
                || record.evidence.source != self.source
                || record.evidence.source_at != self.source_at
                || record.evidence.observed_at != self.observed_at
                || record.evidence.batch_id != self.batch_id
            {
                return Err(SchemaV2Error::new(
                    "directory_record_evidence_mismatch",
                    "record evidence must equal every enclosing batch evidence field",
                ));
            }
            if !triples.insert((
                record.code.clone(),
                record.name.clone(),
                record.kind.as_str().to_owned(),
            )) {
                return Err(SchemaV2Error::new(
                    "directory_record_duplicate",
                    "directory record code/name/kind triples must be unique per category",
                ));
            }
        }
        Ok(())
    }

    pub fn batch_content_hash(&self, recorded_at: DateTime<Utc>) -> Result<String, SchemaV2Error> {
        self.validate(recorded_at)?;
        sha256_json(self)
    }
}

canonical_struct!(DirectoryBatchEvidencePreimage {
    content: DirectoryBatchContentPreimage,
    batch_content_hash: String,
    record_count: u32,
});

impl DirectoryBatchEvidencePreimage {
    pub fn validate(&self, recorded_at: DateTime<Utc>) -> Result<(), SchemaV2Error> {
        self.content.validate(recorded_at)?;
        require_hash(&self.batch_content_hash, "directory_batch_content_hash")?;
        let count = u32::try_from(self.content.records_in_provider_order.len()).map_err(|_| {
            SchemaV2Error::new(
                "directory_batch_record_count_overflow",
                "directory record count does not fit u32",
            )
        })?;
        if self.record_count != count {
            return Err(SchemaV2Error::new(
                "directory_batch_record_count_mismatch",
                "record_count must equal the complete embedded provider record list",
            ));
        }
        if sha256_json(&self.content)? != self.batch_content_hash {
            return Err(SchemaV2Error::new(
                "directory_batch_content_hash_mismatch",
                "batch_content_hash must bind the complete embedded directory content",
            ));
        }
        Ok(())
    }
}

canonical_struct!(BoardAuditSubjectPreimage {
    domain: String,
    proposal_input_content_hash: String,
    audit_command_version: String,
    connection_policy_hash: String,
});

impl BoardAuditSubjectPreimage {
    pub fn audit_subject_id(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_AUDIT_SUBJECT)?;
        require_hash(
            &self.proposal_input_content_hash,
            "proposal_input_content_hash",
        )?;
        if self.audit_command_version != BOARD_AUDIT_COMMAND_VERSION {
            return Err(SchemaV2Error::new(
                "board_audit_command_mismatch",
                "board audit command version is not the frozen BR-174 command",
            ));
        }
        require_hash(&self.connection_policy_hash, "connection_policy_hash")?;
        sha256_json(self)
    }
}

canonical_struct!(BoardAuditPreparedContentPreimage {
    domain: String,
    audit_subject_id: String,
    audit_run_id: String,
    proposal_input_content_hash: String,
    audit_command_version: String,
    connection_policy_version: String,
    connection_policy_hash: String,
    provider_endpoint_evidence: Option<String>,
    audit_root_policy_version: String,
    audit_root_binding_hash: String,
    requested_categories_sorted: Vec<ProviderBoardKind>,
    requested_limit: u32,
    prepared_at_rfc3339_nanos_utc: String,
});

impl BoardAuditPreparedContentPreimage {
    pub fn prepared_content_hash(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_AUDIT_PREPARED)?;
        validate_board_audit_common(BoardAuditCommon {
            audit_subject_id: &self.audit_subject_id,
            audit_run_id: &self.audit_run_id,
            proposal_input_content_hash: &self.proposal_input_content_hash,
            audit_command_version: &self.audit_command_version,
            connection_policy_version: &self.connection_policy_version,
            connection_policy_hash: &self.connection_policy_hash,
            provider_endpoint_evidence: &self.provider_endpoint_evidence,
            audit_root_policy_version: &self.audit_root_policy_version,
            audit_root_binding_hash: &self.audit_root_binding_hash,
            requested_limit: self.requested_limit,
        })?;
        if self.requested_categories_sorted
            != vec![ProviderBoardKind::Concept, ProviderBoardKind::Industry]
        {
            return Err(SchemaV2Error::new(
                "board_audit_categories_invalid",
                "prepared categories must be exactly [concept, industry]",
            ));
        }
        parse_canonical_nanos_utc(
            &self.prepared_at_rfc3339_nanos_utc,
            "prepared_at_rfc3339_nanos_utc",
        )?;
        sha256_json(self)
    }
}

canonical_struct!(AttestedDirectoryBatchPreimage {
    category: ProviderBoardKind,
    batch_content_hash: String,
    record_count: u32,
    observed_at: String,
});

canonical_struct!(BoardAuditAttestationContentPreimage {
    domain: String,
    audit_subject_id: String,
    audit_run_id: String,
    proposal_input_content_hash: String,
    upstream_revision: String,
    audit_command_version: String,
    connection_policy_version: String,
    connection_policy_hash: String,
    provider_endpoint_evidence: Option<String>,
    audit_root_policy_version: String,
    audit_root_binding_hash: String,
    requested_limit: u32,
    directory_batches_by_category: Vec<AttestedDirectoryBatchPreimage>,
    recorded_at_rfc3339_nanos_utc: String,
});

impl BoardAuditAttestationContentPreimage {
    pub fn attestation_content_hash(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_AUDIT_ATTESTATION)?;
        if self.upstream_revision != UPSTREAM_REVISION {
            return Err(SchemaV2Error::new(
                "upstream_revision_mismatch",
                "board attestation must use the frozen upstream revision",
            ));
        }
        validate_board_audit_common(BoardAuditCommon {
            audit_subject_id: &self.audit_subject_id,
            audit_run_id: &self.audit_run_id,
            proposal_input_content_hash: &self.proposal_input_content_hash,
            audit_command_version: &self.audit_command_version,
            connection_policy_version: &self.connection_policy_version,
            connection_policy_hash: &self.connection_policy_hash,
            provider_endpoint_evidence: &self.provider_endpoint_evidence,
            audit_root_policy_version: &self.audit_root_policy_version,
            audit_root_binding_hash: &self.audit_root_binding_hash,
            requested_limit: self.requested_limit,
        })?;
        if self.directory_batches_by_category.len() != 2
            || self.directory_batches_by_category[0].category != ProviderBoardKind::Concept
            || self.directory_batches_by_category[1].category != ProviderBoardKind::Industry
        {
            return Err(SchemaV2Error::new(
                "attested_directory_batches_invalid",
                "attestation must contain exactly category-sorted concept and industry batches",
            ));
        }
        let recorded_at = parse_canonical_nanos_utc(
            &self.recorded_at_rfc3339_nanos_utc,
            "recorded_at_rfc3339_nanos_utc",
        )?;
        for batch in &self.directory_batches_by_category {
            require_hash(&batch.batch_content_hash, "attested_batch_content_hash")?;
            if batch.record_count == 0 || batch.record_count >= BOARD_DIRECTORY_REQUEST_LIMIT {
                return Err(SchemaV2Error::new(
                    "attested_batch_record_count_invalid",
                    "attested directory record count must be 1..9999",
                ));
            }
            let observed_at =
                parse_board_observed_at(&batch.observed_at, "attested_batch.observed_at")?;
            let age = recorded_at.signed_duration_since(observed_at);
            if age < chrono::Duration::zero()
                || age > chrono::Duration::seconds(BOARD_AUDIT_CAPTURE_MAX_AGE_SECS)
            {
                return Err(SchemaV2Error::new(
                    "attested_batch_freshness_invalid",
                    "each attested batch must be independently fresh within 300 seconds",
                ));
            }
        }
        sha256_json(self)
    }
}

canonical_struct!(BoardAuditCommittedContentPreimage {
    domain: String,
    audit_subject_id: String,
    audit_run_id: String,
    prepared_record_hash: String,
    attestation_content_hash: String,
    committed_at_rfc3339_nanos_utc: String,
});

impl BoardAuditCommittedContentPreimage {
    pub fn committed_content_hash(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_AUDIT_COMMITTED)?;
        require_hash(&self.audit_subject_id, "audit_subject_id")?;
        require_canonical_uuid_v7(&self.audit_run_id, "audit_run_id")?;
        require_hash(&self.prepared_record_hash, "prepared_record_hash")?;
        require_hash(&self.attestation_content_hash, "attestation_content_hash")?;
        parse_canonical_nanos_utc(
            &self.committed_at_rfc3339_nanos_utc,
            "committed_at_rfc3339_nanos_utc",
        )?;
        sha256_json(self)
    }
}

canonical_struct!(BoardAuditAttestationReceiptPreimage {
    domain: String,
    audit_subject_id: String,
    audit_run_id: String,
    prepared_record_hash: String,
    committed_record_hash: String,
    attestation_content_hash: String,
    audit_root_policy_version: String,
    audit_root_binding_hash: String,
});

impl BoardAuditAttestationReceiptPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_AUDIT_RECEIPT)?;
        require_hash(&self.audit_subject_id, "audit_subject_id")?;
        require_canonical_uuid_v7(&self.audit_run_id, "audit_run_id")?;
        require_hash(&self.prepared_record_hash, "prepared_record_hash")?;
        require_hash(&self.committed_record_hash, "committed_record_hash")?;
        require_hash(&self.attestation_content_hash, "attestation_content_hash")?;
        let root = BoardAuditRootBindingPreimage::fixed();
        if self.audit_root_policy_version != root.version.as_str()
            || self.audit_root_binding_hash != root.audit_root_binding_hash()?
        {
            return Err(SchemaV2Error::new(
                "board_audit_receipt_root_mismatch",
                "receipt must bind the fixed production board audit root",
            ));
        }
        Ok(())
    }

    pub fn audit_attestation_receipt_hash(&self) -> Result<String, SchemaV2Error> {
        self.validate()?;
        sha256_json(self)
    }
}

struct BoardAuditCommon<'a> {
    audit_subject_id: &'a str,
    audit_run_id: &'a str,
    proposal_input_content_hash: &'a str,
    audit_command_version: &'a str,
    connection_policy_version: &'a str,
    connection_policy_hash: &'a str,
    provider_endpoint_evidence: &'a Option<String>,
    audit_root_policy_version: &'a str,
    audit_root_binding_hash: &'a str,
    requested_limit: u32,
}

fn validate_board_audit_common(common: BoardAuditCommon<'_>) -> Result<(), SchemaV2Error> {
    require_hash(common.audit_subject_id, "audit_subject_id")?;
    require_canonical_uuid_v7(common.audit_run_id, "audit_run_id")?;
    require_hash(
        common.proposal_input_content_hash,
        "proposal_input_content_hash",
    )?;
    if common.audit_command_version != BOARD_AUDIT_COMMAND_VERSION
        || common.connection_policy_version != BOARD_CONNECTION_POLICY_VERSION
        || common.provider_endpoint_evidence.is_some()
        || common.requested_limit != BOARD_DIRECTORY_REQUEST_LIMIT
    {
        return Err(SchemaV2Error::new(
            "board_audit_fixed_policy_mismatch",
            "audit command, connection policy, endpoint NULL and request limit are fixed",
        ));
    }
    if common.connection_policy_hash
        != BoardConnectionPolicyPreimage::fixed()
            .connection_policy_hash()?
            .as_str()
    {
        return Err(SchemaV2Error::new(
            "board_connection_policy_hash_mismatch",
            "connection policy hash must bind the fixed production constructor",
        ));
    }
    let root = BoardAuditRootBindingPreimage::fixed();
    if common.audit_root_policy_version != root.version.as_str()
        || common.audit_root_binding_hash != root.audit_root_binding_hash()?.as_str()
    {
        return Err(SchemaV2Error::new(
            "board_audit_root_hash_mismatch",
            "audit root hash must bind the fixed production root",
        ));
    }
    Ok(())
}

canonical_struct!(ArtifactBindingPreimage {
    chain_id: String,
    provider: String,
    kind: ProviderBoardKind,
    code: String,
    name: String,
    binding_audit_hash: String,
    directory_record_hash: String,
    release_directory_member_count: u32,
});

canonical_struct!(ArtifactHashPreimage {
    domain: String,
    schema_version: String,
    upstream_revision: String,
    proposal_input: BoardBindingProposalInputPreimage,
    proposal_input_content_hash: String,
    connection_policy_version: String,
    connection_policy_hash: String,
    provider_endpoint_evidence: Option<String>,
    valid_from_rfc3339_nanos_utc: String,
    expires_at_rfc3339_nanos_utc: String,
    directory_batches_by_category: Vec<DirectoryBatchEvidencePreimage>,
    requested_limit: u32,
    audit_command_version: String,
    recorded_at_rfc3339_nanos_utc: String,
    audit_attestation_receipt: BoardAuditAttestationReceiptPreimage,
    audit_attestation_receipt_hash: String,
    bindings_sorted: Vec<ArtifactBindingPreimage>,
});

impl ArtifactHashPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_ARTIFACT)?;
        if self.schema_version != BOARD_BINDINGS_SCHEMA_VERSION
            || self.upstream_revision != UPSTREAM_REVISION
            || self.connection_policy_version != BOARD_CONNECTION_POLICY_VERSION
            || self.provider_endpoint_evidence.is_some()
            || self.requested_limit != BOARD_DIRECTORY_REQUEST_LIMIT
            || self.audit_command_version != BOARD_AUDIT_COMMAND_VERSION
        {
            return Err(SchemaV2Error::new(
                "board_artifact_fixed_contract_mismatch",
                "artifact schema, upstream, connection, endpoint, limit or audit command differs from Gate A",
            ));
        }
        self.proposal_input.validate()?;
        if self.proposal_input.proposal_input_content_hash()? != self.proposal_input_content_hash {
            return Err(SchemaV2Error::new(
                "board_proposal_content_hash_mismatch",
                "nested proposal does not match proposal_input_content_hash",
            ));
        }
        if self.connection_policy_hash
            != BoardConnectionPolicyPreimage::fixed().connection_policy_hash()?
        {
            return Err(SchemaV2Error::new(
                "board_connection_policy_hash_mismatch",
                "artifact connection policy hash is not the fixed production policy",
            ));
        }
        if self.valid_from_rfc3339_nanos_utc != self.proposal_input.valid_from_rfc3339_nanos_utc
            || self.expires_at_rfc3339_nanos_utc != self.proposal_input.expires_at_rfc3339_nanos_utc
        {
            return Err(SchemaV2Error::new(
                "board_artifact_proposal_window_mismatch",
                "artifact release window must equal the nested reviewed proposal",
            ));
        }
        let reviewed_at = parse_canonical_nanos_utc(
            &self.proposal_input.reviewed_at_rfc3339_nanos_utc,
            "proposal.reviewed_at",
        )?;
        let recorded_at = parse_canonical_nanos_utc(
            &self.recorded_at_rfc3339_nanos_utc,
            "recorded_at_rfc3339_nanos_utc",
        )?;
        let valid_from = parse_canonical_nanos_utc(
            &self.valid_from_rfc3339_nanos_utc,
            "valid_from_rfc3339_nanos_utc",
        )?;
        let expires_at = parse_canonical_nanos_utc(
            &self.expires_at_rfc3339_nanos_utc,
            "expires_at_rfc3339_nanos_utc",
        )?;
        if reviewed_at > recorded_at
            || recorded_at > valid_from
            || valid_from - recorded_at > chrono::Duration::hours(24)
            || valid_from >= expires_at
            || expires_at - valid_from > chrono::Duration::days(30)
        {
            return Err(SchemaV2Error::new(
                "board_artifact_validity_invalid",
                "reviewed_at <= recorded_at <= valid_from <= recorded_at + 24h and a <=30d window are required",
            ));
        }
        validate_directory_batches(&self.directory_batches_by_category, recorded_at)?;
        self.audit_attestation_receipt.validate()?;
        if self
            .audit_attestation_receipt
            .audit_attestation_receipt_hash()?
            != self.audit_attestation_receipt_hash
        {
            return Err(SchemaV2Error::new(
                "board_audit_receipt_hash_mismatch",
                "nested receipt does not match audit_attestation_receipt_hash",
            ));
        }
        let audit_subject = BoardAuditSubjectPreimage {
            domain: DOMAIN_BOARD_AUDIT_SUBJECT.into(),
            proposal_input_content_hash: self.proposal_input_content_hash.clone(),
            audit_command_version: self.audit_command_version.clone(),
            connection_policy_hash: self.connection_policy_hash.clone(),
        };
        if audit_subject.audit_subject_id()? != self.audit_attestation_receipt.audit_subject_id {
            return Err(SchemaV2Error::new(
                "board_audit_subject_mismatch",
                "receipt audit subject does not bind proposal, command and connection policy",
            ));
        }
        let attested_batches = self
            .directory_batches_by_category
            .iter()
            .map(|batch| AttestedDirectoryBatchPreimage {
                category: batch.content.category,
                batch_content_hash: batch.batch_content_hash.clone(),
                record_count: batch.record_count,
                observed_at: batch.content.observed_at.clone(),
            })
            .collect();
        let attestation = BoardAuditAttestationContentPreimage {
            domain: DOMAIN_BOARD_AUDIT_ATTESTATION.into(),
            audit_subject_id: self.audit_attestation_receipt.audit_subject_id.clone(),
            audit_run_id: self.audit_attestation_receipt.audit_run_id.clone(),
            proposal_input_content_hash: self.proposal_input_content_hash.clone(),
            upstream_revision: self.upstream_revision.clone(),
            audit_command_version: self.audit_command_version.clone(),
            connection_policy_version: self.connection_policy_version.clone(),
            connection_policy_hash: self.connection_policy_hash.clone(),
            provider_endpoint_evidence: self.provider_endpoint_evidence.clone(),
            audit_root_policy_version: self
                .audit_attestation_receipt
                .audit_root_policy_version
                .clone(),
            audit_root_binding_hash: self
                .audit_attestation_receipt
                .audit_root_binding_hash
                .clone(),
            requested_limit: self.requested_limit,
            directory_batches_by_category: attested_batches,
            recorded_at_rfc3339_nanos_utc: self.recorded_at_rfc3339_nanos_utc.clone(),
        };
        if attestation.attestation_content_hash()?
            != self.audit_attestation_receipt.attestation_content_hash
        {
            return Err(SchemaV2Error::new(
                "board_audit_attestation_hash_mismatch",
                "receipt attestation hash does not bind the complete artifact directory evidence",
            ));
        }
        validate_artifact_bindings(self)?;
        Ok(())
    }

    pub fn artifact_content_hash(&self) -> Result<String, SchemaV2Error> {
        self.validate()?;
        sha256_domain_bytes(b"SELECTION_PROVIDER_BOARD_BINDINGS_V1\0", self)
    }
}

canonical_struct!(BindingAuditPreimage {
    domain: String,
    upstream_revision: String,
    chain_id: String,
    provider: String,
    kind: ProviderBoardKind,
    code: String,
    name: String,
    directory_category: ProviderBoardKind,
    directory_source: String,
    directory_source_at: Option<String>,
    directory_observed_at: String,
    directory_batch_id: String,
    directory_batch_content_hash: String,
    directory_record_hash: String,
    release_directory_member_count: u32,
    proposal_input_content_hash: String,
    proposal_reviewed_by: String,
    proposal_reviewed_at_rfc3339_nanos_utc: String,
    validity_policy_version: String,
    audit_command_version: String,
    connection_policy_version: String,
    connection_policy_hash: String,
    provider_endpoint_evidence: Option<String>,
    audit_attestation_receipt_hash: String,
    recorded_at_rfc3339_nanos_utc: String,
    valid_from_rfc3339_nanos_utc: String,
    expires_at_rfc3339_nanos_utc: String,
});

impl BindingAuditPreimage {
    pub fn binding_audit_hash(&self) -> Result<String, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BOARD_BINDING)?;
        if self.upstream_revision != UPSTREAM_REVISION
            || self.directory_category != self.kind
            || self.directory_source != BOARD_DIRECTORY_SOURCE
            || self.directory_source_at.is_some()
            || self.validity_policy_version != BOARD_BINDING_VALIDITY_POLICY_VERSION
            || self.audit_command_version != BOARD_AUDIT_COMMAND_VERSION
            || self.connection_policy_version != BOARD_CONNECTION_POLICY_VERSION
            || self.provider_endpoint_evidence.is_some()
        {
            return Err(SchemaV2Error::new(
                "binding_audit_fixed_contract_mismatch",
                "binding audit does not retain the fixed source/policy/null contract",
            ));
        }
        validate_fixed_board_binding(
            &self.chain_id,
            &self.provider,
            self.kind,
            &self.code,
            &self.name,
        )?;
        require_trim_stable_non_empty(&self.directory_batch_id, "directory_batch_id")?;
        require_trim_stable_non_empty(&self.proposal_reviewed_by, "proposal_reviewed_by")?;
        for (value, field) in [
            (
                &self.directory_batch_content_hash,
                "directory_batch_content_hash",
            ),
            (&self.directory_record_hash, "directory_record_hash"),
            (
                &self.proposal_input_content_hash,
                "proposal_input_content_hash",
            ),
            (&self.connection_policy_hash, "connection_policy_hash"),
            (
                &self.audit_attestation_receipt_hash,
                "audit_attestation_receipt_hash",
            ),
        ] {
            require_hash(value, field)?;
        }
        if self.release_directory_member_count == 0 {
            return Err(SchemaV2Error::new(
                "binding_directory_member_count_invalid",
                "binding release directory member count must be positive",
            ));
        }
        parse_board_observed_at(&self.directory_observed_at, "directory_observed_at")?;
        parse_canonical_nanos_utc(
            &self.proposal_reviewed_at_rfc3339_nanos_utc,
            "proposal_reviewed_at_rfc3339_nanos_utc",
        )?;
        parse_canonical_nanos_utc(
            &self.recorded_at_rfc3339_nanos_utc,
            "recorded_at_rfc3339_nanos_utc",
        )?;
        parse_canonical_nanos_utc(
            &self.valid_from_rfc3339_nanos_utc,
            "valid_from_rfc3339_nanos_utc",
        )?;
        parse_canonical_nanos_utc(
            &self.expires_at_rfc3339_nanos_utc,
            "expires_at_rfc3339_nanos_utc",
        )?;
        sha256_json(self)
    }
}

fn validate_directory_batches(
    batches: &[DirectoryBatchEvidencePreimage],
    recorded_at: DateTime<Utc>,
) -> Result<(), SchemaV2Error> {
    if batches.len() != 2
        || batches[0].content.category != ProviderBoardKind::Concept
        || batches[1].content.category != ProviderBoardKind::Industry
    {
        return Err(SchemaV2Error::new(
            "directory_batches_invalid",
            "artifact must contain exactly category-sorted concept and industry batches",
        ));
    }
    batches[0].validate(recorded_at)?;
    batches[1].validate(recorded_at)?;
    if batches[0].content.batch_id == batches[1].content.batch_id {
        return Err(SchemaV2Error::new(
            "directory_batch_ids_not_distinct",
            "concept and industry provider batches must have distinct batch IDs",
        ));
    }
    Ok(())
}

fn validate_artifact_bindings(artifact: &ArtifactHashPreimage) -> Result<(), SchemaV2Error> {
    if artifact.bindings_sorted.len() != artifact.proposal_input.bindings_sorted.len() {
        return Err(SchemaV2Error::new(
            "artifact_proposal_binding_count_mismatch",
            "artifact must derive exactly one binding for every proposal binding",
        ));
    }
    let mut previous_key: Option<(String, String, String, String, String)> = None;
    for (binding, proposal) in artifact
        .bindings_sorted
        .iter()
        .zip(&artifact.proposal_input.bindings_sorted)
    {
        let key = (
            binding.chain_id.clone(),
            binding.provider.clone(),
            binding.kind.as_str().to_owned(),
            binding.code.clone(),
            binding.name.clone(),
        );
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(SchemaV2Error::new(
                "artifact_bindings_not_sorted_unique",
                "artifact bindings must be strictly sorted by the frozen tuple",
            ));
        }
        previous_key = Some(key);
        if binding.chain_id != proposal.chain_id
            || binding.provider != proposal.provider
            || binding.kind != proposal.kind
            || binding.code != proposal.code
            || binding.name != proposal.name
        {
            return Err(SchemaV2Error::new(
                "artifact_proposal_binding_mismatch",
                "artifact binding base fields must map one-to-one to proposal bindings",
            ));
        }
        require_hash(&binding.binding_audit_hash, "binding_audit_hash")?;
        require_hash(&binding.directory_record_hash, "directory_record_hash")?;
        let batch = artifact
            .directory_batches_by_category
            .iter()
            .find(|batch| batch.content.category == binding.kind)
            .ok_or_else(|| {
                SchemaV2Error::new(
                    "binding_directory_category_missing",
                    "binding category has no corresponding directory batch",
                )
            })?;
        let records = batch
            .content
            .records_in_provider_order
            .iter()
            .filter(|record| {
                record.code == binding.code
                    && record.name == binding.name
                    && record.kind == binding.kind
            })
            .collect::<Vec<_>>();
        if records.len() != 1 {
            return Err(SchemaV2Error::new(
                "binding_directory_exact_one_failed",
                "binding triple must exact-match one directory record in its category",
            ));
        }
        let record = records[0];
        if binding.provider != batch.content.provider
            || binding.directory_record_hash != record.directory_record_hash()?
            || binding.release_directory_member_count != record.member_count
        {
            return Err(SchemaV2Error::new(
                "binding_directory_evidence_mismatch",
                "binding provider, record hash and member count must equal exact directory evidence",
            ));
        }
        let audit = BindingAuditPreimage {
            domain: DOMAIN_BOARD_BINDING.into(),
            upstream_revision: artifact.upstream_revision.clone(),
            chain_id: binding.chain_id.clone(),
            provider: binding.provider.clone(),
            kind: binding.kind,
            code: binding.code.clone(),
            name: binding.name.clone(),
            directory_category: batch.content.category,
            directory_source: batch.content.source.clone(),
            directory_source_at: batch.content.source_at.clone(),
            directory_observed_at: batch.content.observed_at.clone(),
            directory_batch_id: batch.content.batch_id.clone(),
            directory_batch_content_hash: batch.batch_content_hash.clone(),
            directory_record_hash: binding.directory_record_hash.clone(),
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
        };
        if audit.binding_audit_hash()? != binding.binding_audit_hash {
            return Err(SchemaV2Error::new(
                "binding_audit_hash_mismatch",
                "binding_audit_hash must bind its exact proposal/directory/audit evidence",
            ));
        }
    }
    Ok(())
}

canonical_struct!(ChainRuleSnapshotEntryPreimage {
    chain_id: String,
    category: String,
    priority: u32,
    logic: String,
    board_keyword: String,
    keywords_in_config_order: Vec<String>,
    generic: bool,
    enabled: bool,
    provider_board_binding_audit_hash: Option<String>,
});

canonical_struct!(ChainRulesSnapshotPreimage {
    domain: String,
    rules_sorted: Vec<ChainRuleSnapshotEntryPreimage>,
});

canonical_struct!(ExecutableInputFilePreimage {
    relative_path: String,
    byte_len: u64,
    content_sha256: String,
});

canonical_struct!(ExecutableRevisionPreimage {
    domain: String,
    input_manifest_version: String,
    files_sorted: Vec<ExecutableInputFilePreimage>,
});

canonical_struct!(SelectionConfigSnapshotPreimage {
    domain: String,
    schema_version: String,
    chain_config_bytes_hash: String,
    chain_rules_snapshot: ChainRulesSnapshotPreimage,
    chain_rules_sorted_content_hash: String,
    board_artifact: ArtifactHashPreimage,
    board_artifact_content_hash: String,
    binding_audit_hashes_sorted: Vec<String>,
    relation_schema_version: String,
    feature_version: String,
    admission_version: String,
    upstream_revision: String,
    executable_revision: String,
});

impl SelectionConfigSnapshotPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_SELECTION_CONFIG_SNAPSHOT)?;
        require_domain(
            &self.chain_rules_snapshot.domain,
            DOMAIN_CHAIN_RULES_SNAPSHOT,
        )?;
        if sha256_json(&self.chain_rules_snapshot)? != self.chain_rules_sorted_content_hash {
            return Err(SchemaV2Error::new(
                "chain_rules_snapshot_hash_mismatch",
                "nested chain rules do not match chain_rules_sorted_content_hash",
            ));
        }
        if self.board_artifact.artifact_content_hash()? != self.board_artifact_content_hash {
            return Err(SchemaV2Error::new(
                "board_artifact_hash_mismatch",
                "nested board artifact does not match board_artifact_content_hash",
            ));
        }
        if self.upstream_revision != UPSTREAM_REVISION
            || self.board_artifact.upstream_revision != UPSTREAM_REVISION
        {
            return Err(SchemaV2Error::new(
                "upstream_revision_mismatch",
                "snapshot and artifact must use the frozen upstream revision",
            ));
        }
        Ok(())
    }
}

canonical_struct!(ConfigActivationContentPreimage {
    domain: String,
    config_hash: String,
    activated_at_rfc3339_nanos_utc: String,
    effective_from_rfc3339_nanos_utc: String,
    activation_file_content_hash: String,
    reviewed_by: String,
    reviewed_at_rfc3339_nanos_utc: String,
    artifact_valid_from: String,
    artifact_expires_at: String,
    executable_revision: String,
});

canonical_struct!(DirectMentionSourcePreimage {
    domain: String,
    source_fact_key: String,
    field: DirectMentionField,
    mention_kind: MentionKind,
    normalized_value: String,
    byte_start: u32,
    byte_end: u32,
});

canonical_struct!(BoardRelationSourcePreimage {
    domain: String,
    artifact_content_hash: String,
    binding_audit_hash: String,
    provider: String,
    kind: ProviderBoardKind,
    code: String,
    name: String,
});

canonical_struct!(BoardNotConfiguredSourcePreimage {
    domain: String,
    source_fact_key: String,
    chain_id: String,
    config_hash: String,
});

canonical_struct!(RelationEvidenceEntryPreimage {
    relation_rank: u8,
    relation_key: String,
    relation_kind: RelationKind,
    relation_attempt_id: String,
    relation_attempt_content_hash: String,
});

canonical_struct!(RelationEvidenceSetPreimage {
    domain: String,
    source_fact_key: String,
    event_id: String,
    chain_id: String,
    canonical_stock_code: String,
    entries_in_relation_order: Vec<RelationEvidenceEntryPreimage>,
});

canonical_struct!(BindingStatePreimage {
    domain: String,
    state: BindingStateKind,
    artifact_content_hash: Option<String>,
    binding_audit_hash: Option<String>,
    provider: Option<String>,
    kind: Option<ProviderBoardKind>,
    code: Option<String>,
    name: Option<String>,
    error_fingerprint: Option<String>,
});

impl BindingStatePreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_BINDING_STATE)?;
        let all_binding_fields_none = self.artifact_content_hash.is_none()
            && self.binding_audit_hash.is_none()
            && self.provider.is_none()
            && self.kind.is_none()
            && self.code.is_none()
            && self.name.is_none()
            && self.error_fingerprint.is_none();
        match self.state {
            BindingStateKind::DirectNotApplicable | BindingStateKind::NotConfigured
                if all_binding_fields_none =>
            {
                Ok(())
            }
            BindingStateKind::Verified
                if self.artifact_content_hash.is_some()
                    && self.binding_audit_hash.is_some()
                    && self.provider.is_some()
                    && self.kind.is_some()
                    && self.code.is_some()
                    && self.name.is_some()
                    && self.error_fingerprint.is_none() =>
            {
                Ok(())
            }
            _ => Err(SchemaV2Error::new(
                "invalid_binding_state_matrix",
                "binding state fields do not match direct/not-configured/verified contract",
            )),
        }
    }
}

canonical_struct!(RelationKeyPreimage {
    domain: String,
    event_id: String,
    chain_id: String,
    config_hash: String,
    relation_kind: RelationKind,
    relation_source_identity_hash: String,
    typed_binding_state_hash: String,
    relation_schema_version: String,
});

canonical_struct!(RelationAttemptPreimage {
    domain: String,
    stage_run_id: String,
    relation_key: String,
    request_hash: Option<String>,
    provider_batch_id: Option<String>,
    provider_observed_at: Option<String>,
    result_code: String,
    error_fingerprint: Option<String>,
});

canonical_struct!(SampleKeyPreimage {
    domain: String,
    event_id: String,
    chain_id: String,
    stock_code: String,
    relation_schema_version: String,
    feature_version: String,
    evaluation_market_date: String,
});

canonical_struct!(EvaluationAttemptPreimage {
    domain: String,
    stage_run_id: String,
    sample_key: String,
    market_request_hash: String,
    provider_batch_id: Option<String>,
    provider_observed_at: Option<String>,
    result_code: String,
    error_fingerprint: Option<String>,
});

canonical_struct!(OutcomeAttemptPreimage {
    domain: String,
    stage_run_id: String,
    sample_key: String,
    phase: OutcomePhase,
    stored_due_date: String,
    request_hash: Option<String>,
    transport_attempts_hash: Option<String>,
    provider_batch_id: Option<String>,
    provider_observed_at: Option<String>,
    result_code: OutcomeAttemptResult,
    error_fingerprint: Option<String>,
});

canonical_struct!(ProviderCapabilityHashPreimage {
    domain: String,
    provider: String,
    capability_name: String,
    contract_version: String,
    upstream_revision: String,
});

canonical_struct!(GlobalNewsRequestParametersPreimage {
    domain: String,
    feed_identity: String,
    limit: u32,
});

canonical_struct!(BoardConstituentRequestParametersPreimage {
    domain: String,
    artifact_content_hash: String,
    binding_audit_hash: String,
    provider: String,
    kind: ProviderBoardKind,
    code: String,
    name: String,
    limit: u32,
});

canonical_struct!(T0MarketRequestParametersPreimage {
    domain: String,
    canonical_stock_code: String,
    canonical_market: String,
    evaluation_market_date: String,
    quote_max_age_secs: u64,
    daily_interval: String,
    daily_limit: u32,
    intraday_interval: String,
    intraday_limit: u32,
    adjustment: AdjustmentKind,
});

canonical_struct!(OutcomeTradingDateVectorPreimage {
    domain: String,
    t0: String,
    d1: String,
    d2: String,
    d3: String,
    d4: String,
    d5: String,
});

impl OutcomeTradingDateVectorPreimage {
    pub fn validate(&self) -> Result<[NaiveDate; 6], SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_TRADING_DATE_VECTOR)?;
        let dates = [
            parse_canonical_date(&self.t0, "trading_date_vector.t0")?,
            parse_canonical_date(&self.d1, "trading_date_vector.d1")?,
            parse_canonical_date(&self.d2, "trading_date_vector.d2")?,
            parse_canonical_date(&self.d3, "trading_date_vector.d3")?,
            parse_canonical_date(&self.d4, "trading_date_vector.d4")?,
            parse_canonical_date(&self.d5, "trading_date_vector.d5")?,
        ];
        if dates.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(SchemaV2Error::new(
                "outcome_trading_date_vector_not_strict",
                "T0/D1/D2/D3/D4/D5 must be strictly increasing",
            ));
        }
        Ok(dates)
    }

    pub fn applicable_dates(&self, phase: OutcomePhase) -> Result<Vec<String>, SchemaV2Error> {
        self.validate()?;
        let count = match phase {
            OutcomePhase::T0Close => 1,
            OutcomePhase::D1Settled => 2,
            OutcomePhase::D3Settled => 4,
            OutcomePhase::D5Settled => 6,
        };
        Ok([
            self.t0.clone(),
            self.d1.clone(),
            self.d2.clone(),
            self.d3.clone(),
            self.d4.clone(),
            self.d5.clone(),
        ][..count]
            .to_vec())
    }
}

canonical_struct!(OutcomeMarketRequestParametersPreimage {
    domain: String,
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    phase: OutcomePhase,
    stored_due_date: String,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    applicable_trading_dates: Vec<String>,
    window_start: String,
    window_end: String,
    interval: DailyIntervalKind,
    adjustment: AdjustmentKind,
});

canonical_struct!(OutcomeProviderRequestPreimage {
    domain: String,
    design_sha256: String,
    amendment_design_sha256: String,
    semantic_request_hash: String,
    verified_due_binding_hash: String,
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    phase: OutcomePhase,
    stored_due_date: String,
    window_start: String,
    window_end: String,
    expected_bar_count: u16,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    expected_trading_dates: Vec<String>,
    receipted_t0_close: Option<String>,
    receipted_t0_volume_shares: Option<String>,
    request_local_date: String,
    post_close_cutoff: String,
    interval: String,
    adjustment: String,
    acquisition_strategy: String,
    adaptive_policy_version: String,
    maximum_latest_n: u16,
    volume_conversion_contract: String,
    volume_conversion_version: String,
    shares_per_board_lot: String,
});

canonical_struct!(OutcomeTransportBarFingerprint {
    market_date: String,
    open: String,
    high: String,
    low: String,
    close: String,
    core_volume_lots: String,
    amount: Option<String>,
    provider: String,
    batch_id: String,
});

canonical_struct!(OutcomeTransportBatchContentPreimage {
    provider: String,
    source: String,
    records: Vec<OutcomeTransportBarFingerprint>,
});

canonical_struct!(OutcomeTransportEvidencePreimage {
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    record_count: u32,
    batch_content: OutcomeTransportBatchContentPreimage,
    batch_content_hash: String,
});

canonical_struct!(OutcomeHistoricalBarCardinalityPreimage {
    offset: u32,
    actual: u64,
    expected_page: u16,
    requested_total: u16,
});

canonical_struct!(OutcomeProviderErrorPreimage {
    variant: String,
    coded_error: Option<u32>,
    io_kind: Option<String>,
    raw_os_error: Option<i32>,
    retry_attempts: Option<u32>,
    structured_detail_hash: Option<String>,
    historical_bar_cardinality: Option<OutcomeHistoricalBarCardinalityPreimage>,
});

canonical_struct!(OutcomeTransportRequestPreimage {
    provider: String,
    source: String,
    canonical_stock_code: String,
    canonical_market: String,
    interval: String,
    adjustment: String,
    latest_n: u16,
});

canonical_struct!(OutcomeTransportResultPreimage {
    terminal_state: String,
    requested_latest_n: u16,
    actual_count: Option<u16>,
    provider_evidence: Option<OutcomeTransportEvidencePreimage>,
    provider_evidence_hash: Option<String>,
    provider_error: Option<OutcomeProviderErrorPreimage>,
    provider_error_hash: Option<String>,
});

canonical_struct!(OutcomeTransportAttemptPreimage {
    request_ordinal: u32,
    request: OutcomeTransportRequestPreimage,
    request_hash: String,
    result: OutcomeTransportResultPreimage,
    result_hash: String,
});

canonical_struct!(OutcomeTransportAttemptsPreimage {
    domain: String,
    design_sha256: String,
    amendment_design_sha256: String,
    row_request_hash: String,
    request_evidence_hash: String,
    provider_capability_hash: String,
    provider_revision: String,
    request_parameters_hash: String,
    provider_request_hash: String,
    verified_due_binding_hash: String,
    adaptive_policy_version: String,
    expected_bar_count: u16,
    maximum_latest_n: u16,
    selected_transport_result_hash: Option<String>,
    attempts_in_request_order: Vec<OutcomeTransportAttemptPreimage>,
});

impl OutcomeProviderRequestPreimage {
    pub fn validate(
        &self,
        request: &RequestEvidencePreimage,
        parameters: &OutcomeMarketRequestParametersPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_PROVIDER_REQUEST)?;
        if self.design_sha256 != OUTCOME_PARENT_DESIGN_SHA256
            || self.amendment_design_sha256 != AMENDMENT_DESIGN_SHA256
        {
            return Err(SchemaV2Error::new(
                "outcome_provider_request_design_mismatch",
                "provider request must bind the frozen parent and amendment designs",
            ));
        }
        require_hash(&self.verified_due_binding_hash, "verified_due_binding_hash")?;
        let expected_dates = parameters
            .trading_date_vector
            .applicable_dates(parameters.phase)?;
        if self.semantic_request_hash != request.request_hash
            || self.sample_key != parameters.sample_key
            || self.canonical_stock_code != parameters.canonical_stock_code
            || self.canonical_market != parameters.canonical_market
            || self.phase != parameters.phase
            || self.stored_due_date != parameters.stored_due_date
            || self.window_start != parameters.window_start
            || self.window_end != parameters.window_end
            || self.calendar_version != parameters.calendar_version
            || self.calendar_hash != parameters.calendar_hash
            || self.trading_date_vector != parameters.trading_date_vector
            || self.trading_date_vector_hash != parameters.trading_date_vector_hash
            || self.expected_trading_dates != expected_dates
            || usize::from(self.expected_bar_count) != expected_dates.len()
            || self.interval != parameters.interval.as_str()
            || self.adjustment != parameters.adjustment.as_str()
        {
            return Err(SchemaV2Error::new(
                "outcome_provider_request_projection_mismatch",
                "provider request must equal the canonical semantic outcome request",
            ));
        }
        if self.adaptive_policy_version != OUTCOME_ADAPTIVE_POLICY_VERSION
            || self.maximum_latest_n < self.expected_bar_count
        {
            return Err(SchemaV2Error::new(
                "outcome_provider_request_adaptive_policy_mismatch",
                "provider request must bind the released latest-N policy and legal bounds",
            ));
        }
        parse_canonical_date(&self.request_local_date, "outcome_request_local_date")?;
        Ok(())
    }
}

impl OutcomeTransportAttemptsPreimage {
    pub fn validate(
        &self,
        row_request_hash: &str,
        row_request_evidence_hash: &str,
        request: &RequestEvidencePreimage,
        parameters: &OutcomeMarketRequestParametersPreimage,
        capability: &ProviderCapabilityHashPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS)?;
        if self.design_sha256 != OUTCOME_PARENT_DESIGN_SHA256
            || self.amendment_design_sha256 != AMENDMENT_DESIGN_SHA256
        {
            return Err(SchemaV2Error::new(
                "outcome_transport_design_mismatch",
                "transport attempts must bind the frozen parent and amendment designs",
            ));
        }
        require_hash(&self.provider_request_hash, "outcome_provider_request_hash")?;
        require_hash(
            &self.verified_due_binding_hash,
            "outcome_verified_due_binding_hash",
        )?;
        if self.row_request_hash != row_request_hash
            || self.row_request_hash != request.request_hash
            || self.request_evidence_hash != row_request_evidence_hash
            || self.provider_capability_hash != request.provider_capability_hash
            || self.provider_revision != UPSTREAM_REVISION
            || self.provider_revision != capability.upstream_revision
            || self.request_parameters_hash != request.parameters_json_hash
            || self.adaptive_policy_version != OUTCOME_ADAPTIVE_POLICY_VERSION
        {
            return Err(SchemaV2Error::new(
                "outcome_transport_request_cross_link_mismatch",
                "transport attempts must bind the exact row request, capability and revision",
            ));
        }
        let expected_bar_count =
            u16::try_from(parameters.applicable_trading_dates.len()).map_err(|_| {
                SchemaV2Error::new(
                    "outcome_transport_expected_count_overflow",
                    "applicable trading-date count exceeds u16",
                )
            })?;
        if self.expected_bar_count != expected_bar_count
            || self.expected_bar_count == 0
            || self.maximum_latest_n < self.expected_bar_count
        {
            return Err(SchemaV2Error::new(
                "outcome_transport_adaptive_bounds_mismatch",
                "transport adaptive bounds must derive from the semantic request",
            ));
        }
        if self.attempts_in_request_order.is_empty() {
            return Err(SchemaV2Error::new(
                "outcome_transport_attempts_empty",
                "a provider access must retain at least one typed transport attempt",
            ));
        }

        let window_start =
            parse_canonical_date(&parameters.window_start, "outcome_transport_window_start")?;
        let mut expected_latest_n = self.expected_bar_count;
        let mut last_success_latest_n = 0_u16;
        let mut binary_high: Option<u16> = None;
        for (position, attempt) in self.attempts_in_request_order.iter().enumerate() {
            let ordinal = u32::try_from(position).map_err(|_| {
                SchemaV2Error::new(
                    "outcome_transport_ordinal_overflow",
                    "transport attempt position exceeds u32",
                )
            })?;
            if attempt.request_ordinal != ordinal {
                return Err(SchemaV2Error::new(
                    "outcome_transport_ordinal_mismatch",
                    "transport ordinal must equal its zero-based array position",
                ));
            }
            if attempt.request.provider != capability.provider
                || attempt.request.source != "tdx-smart"
                || attempt.request.canonical_stock_code != parameters.canonical_stock_code
                || attempt.request.canonical_market != parameters.canonical_market
                || attempt.request.interval != parameters.interval.as_str()
                || attempt.request.adjustment != parameters.adjustment.as_str()
            {
                return Err(SchemaV2Error::new(
                    "outcome_transport_request_projection_mismatch",
                    "nested transport request must equal the semantic request and capability",
                ));
            }
            if attempt.request.latest_n != expected_latest_n {
                return Err(SchemaV2Error::new(
                    "outcome_transport_adaptive_sequence_mismatch",
                    format!(
                        "attempt {position} latest_n={} expected={expected_latest_n}",
                        attempt.request.latest_n
                    ),
                ));
            }
            if sha256_json(&attempt.request)? != attempt.request_hash
                || attempt.result.requested_latest_n != attempt.request.latest_n
                || sha256_json(&attempt.result)? != attempt.result_hash
            {
                return Err(SchemaV2Error::new(
                    "outcome_transport_attempt_hash_mismatch",
                    "nested request/result hashes must recompute exactly",
                ));
            }

            let is_last = position + 1 == self.attempts_in_request_order.len();
            match validate_outcome_transport_result(attempt, window_start)? {
                ValidatedOutcomeTransportResult::Available { covers_window } => {
                    last_success_latest_n = attempt.request.latest_n;
                    if covers_window {
                        if !is_last {
                            return Err(SchemaV2Error::new(
                                "outcome_transport_request_after_terminal",
                                "no request may follow a success covering the immutable window",
                            ));
                        }
                    } else if let Some(high) = binary_high {
                        if last_success_latest_n < high {
                            expected_latest_n =
                                last_success_latest_n + (high - last_success_latest_n).div_ceil(2);
                        } else if !is_last {
                            return Err(SchemaV2Error::new(
                                "outcome_transport_request_after_terminal",
                                "no request may follow the terminal highest-success decision",
                            ));
                        }
                    } else if last_success_latest_n < self.maximum_latest_n {
                        expected_latest_n = last_success_latest_n
                            .checked_mul(2)
                            .unwrap_or(self.maximum_latest_n)
                            .min(self.maximum_latest_n);
                    } else if !is_last {
                        return Err(SchemaV2Error::new(
                            "outcome_transport_request_after_terminal",
                            "no request may follow a maximum-bound success",
                        ));
                    }
                }
                ValidatedOutcomeTransportResult::CardinalityFailure { available_count } => {
                    let high = available_count
                        .min(attempt.request.latest_n.saturating_sub(1))
                        .min(self.maximum_latest_n)
                        .min(binary_high.unwrap_or(self.maximum_latest_n));
                    if high < last_success_latest_n {
                        return Err(SchemaV2Error::new(
                            "outcome_transport_cardinality_regression",
                            "available cardinality regressed below the last exact success",
                        ));
                    }
                    binary_high = Some(high);
                    if last_success_latest_n < high {
                        expected_latest_n =
                            last_success_latest_n + (high - last_success_latest_n).div_ceil(2);
                    } else if !is_last {
                        return Err(SchemaV2Error::new(
                            "outcome_transport_request_after_terminal",
                            "no request may follow the terminal cardinality decision",
                        ));
                    }
                }
                ValidatedOutcomeTransportResult::TerminalFailure => {
                    if !is_last {
                        return Err(SchemaV2Error::new(
                            "outcome_transport_request_after_terminal",
                            "no request may follow a terminal provider/validation failure",
                        ));
                    }
                }
            }
        }

        if let Some(selected_hash) = self.selected_transport_result_hash.as_deref() {
            require_hash(selected_hash, "selected_transport_result_hash")?;
            let matching = self
                .attempts_in_request_order
                .iter()
                .filter(|attempt| {
                    attempt.result_hash == selected_hash
                        && attempt.result.terminal_state == "available"
                })
                .count();
            if matching != 1 {
                return Err(SchemaV2Error::new(
                    "outcome_transport_selected_result_mismatch",
                    "selected result must identify exactly one successful transport attempt",
                ));
            }
        }
        Ok(())
    }

    pub fn selected_attempt(&self) -> Option<&OutcomeTransportAttemptPreimage> {
        let selected = self.selected_transport_result_hash.as_deref()?;
        self.attempts_in_request_order
            .iter()
            .find(|attempt| attempt.result_hash == selected)
    }
}

enum ValidatedOutcomeTransportResult {
    Available { covers_window: bool },
    CardinalityFailure { available_count: u16 },
    TerminalFailure,
}

fn validate_outcome_transport_result(
    attempt: &OutcomeTransportAttemptPreimage,
    window_start: NaiveDate,
) -> Result<ValidatedOutcomeTransportResult, SchemaV2Error> {
    require_pair(
        &attempt.result.provider_evidence,
        &attempt.result.provider_evidence_hash,
        "outcome_transport_provider_evidence_pair_mismatch",
    )?;
    require_pair(
        &attempt.result.provider_error,
        &attempt.result.provider_error_hash,
        "outcome_transport_provider_error_pair_mismatch",
    )?;
    match attempt.result.terminal_state.as_str() {
        "available" => {
            if attempt.result.provider_error.is_some()
                || attempt.result.actual_count != Some(attempt.request.latest_n)
            {
                return Err(SchemaV2Error::new(
                    "outcome_transport_available_matrix_mismatch",
                    "available requires exact cardinality/evidence and no provider error",
                ));
            }
            let evidence = validate_outcome_transport_evidence(attempt, attempt.request.latest_n)?;
            let first = evidence.batch_content.records.first().ok_or_else(|| {
                SchemaV2Error::new(
                    "outcome_transport_available_empty",
                    "available transport evidence cannot be empty",
                )
            })?;
            let first_date =
                parse_canonical_date(&first.market_date, "outcome_transport_first_market_date")?;
            Ok(ValidatedOutcomeTransportResult::Available {
                covers_window: first_date <= window_start,
            })
        }
        "provider_cardinality_violation" => {
            if attempt.result.provider_error.is_some()
                || attempt.result.actual_count == Some(attempt.request.latest_n)
            {
                return Err(SchemaV2Error::new(
                    "outcome_transport_validation_error_matrix_mismatch",
                    "post-transport cardinality violation requires non-exact evidence",
                ));
            }
            let actual = attempt.result.actual_count.ok_or_else(|| {
                SchemaV2Error::new(
                    "outcome_transport_validation_actual_missing",
                    "post-transport cardinality violation requires actual_count",
                )
            })?;
            validate_outcome_transport_evidence(attempt, actual)?;
            Ok(ValidatedOutcomeTransportResult::TerminalFailure)
        }
        "cardinality_mismatch" => {
            if attempt.result.provider_evidence.is_some() || attempt.result.actual_count.is_none() {
                return Err(SchemaV2Error::new(
                    "outcome_transport_cardinality_matrix_mismatch",
                    "cardinality mismatch requires only typed provider-error evidence",
                ));
            }
            let provider_error = validate_outcome_provider_error(attempt)?;
            let cardinality = provider_error
                .historical_bar_cardinality
                .as_ref()
                .ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_transport_cardinality_detail_missing",
                        "cardinality mismatch requires typed page geometry",
                    )
                })?;
            let available_count =
                validate_outcome_cardinality_geometry(cardinality, attempt.request.latest_n)?;
            if attempt.result.actual_count != Some(available_count) {
                return Err(SchemaV2Error::new(
                    "outcome_transport_cardinality_actual_mismatch",
                    "result actual_count must equal typed page-derived available count",
                ));
            }
            Ok(ValidatedOutcomeTransportResult::CardinalityFailure { available_count })
        }
        "provider_error" => {
            if attempt.result.provider_evidence.is_some() || attempt.result.actual_count.is_some() {
                return Err(SchemaV2Error::new(
                    "outcome_transport_provider_error_matrix_mismatch",
                    "provider error requires no batch evidence or actual_count",
                ));
            }
            let provider_error = validate_outcome_provider_error(attempt)?;
            if provider_error.historical_bar_cardinality.is_some() {
                return Err(SchemaV2Error::new(
                    "outcome_transport_provider_error_cardinality_mismatch",
                    "typed historical cardinality errors use cardinality_mismatch",
                ));
            }
            Ok(ValidatedOutcomeTransportResult::TerminalFailure)
        }
        _ => Err(SchemaV2Error::new(
            "outcome_transport_terminal_state_unknown",
            "transport terminal_state is not in the released closed set",
        )),
    }
}

fn validate_outcome_transport_evidence(
    attempt: &OutcomeTransportAttemptPreimage,
    expected_count: u16,
) -> Result<&OutcomeTransportEvidencePreimage, SchemaV2Error> {
    let evidence = attempt.result.provider_evidence.as_ref().ok_or_else(|| {
        SchemaV2Error::new(
            "outcome_transport_provider_evidence_missing",
            "transport result requires typed provider batch evidence",
        )
    })?;
    if sha256_json(evidence)?
        != *attempt
            .result
            .provider_evidence_hash
            .as_ref()
            .expect("pair validated")
        || sha256_json(&evidence.batch_content)? != evidence.batch_content_hash
        || evidence.record_count != u32::from(expected_count)
        || evidence.batch_content.records.len() != usize::from(expected_count)
        || evidence.source != attempt.request.source
        || evidence.batch_content.provider != attempt.request.provider
        || evidence.batch_content.source != attempt.request.source
    {
        return Err(SchemaV2Error::new(
            "outcome_transport_provider_evidence_mismatch",
            "provider evidence/hash/count/source must bind the exact transport response",
        ));
    }
    for record in &evidence.batch_content.records {
        parse_canonical_date(&record.market_date, "outcome_transport_bar_market_date")?;
        if record.provider != "Tdx" || record.batch_id != evidence.batch_id {
            return Err(SchemaV2Error::new(
                "outcome_transport_bar_batch_mismatch",
                "every transport bar must bind the raw TDX provider identity and attempt batch",
            ));
        }
        for (field, value) in [
            ("open", &record.open),
            ("high", &record.high),
            ("low", &record.low),
            ("close", &record.close),
        ] {
            if parse_canonical_f64(field, value)? <= 0.0 {
                return Err(SchemaV2Error::new(
                    "outcome_transport_price_invalid",
                    "transport bar prices must be positive",
                ));
            }
        }
        if parse_canonical_f64("core_volume_lots", &record.core_volume_lots)? < 0.0 {
            return Err(SchemaV2Error::new(
                "outcome_transport_volume_invalid",
                "transport bar volume cannot be negative",
            ));
        }
        if let Some(amount) = &record.amount {
            if parse_canonical_f64("amount", amount)? < 0.0 {
                return Err(SchemaV2Error::new(
                    "outcome_transport_amount_invalid",
                    "transport bar amount cannot be negative",
                ));
            }
        }
    }
    Ok(evidence)
}

fn validate_outcome_provider_error(
    attempt: &OutcomeTransportAttemptPreimage,
) -> Result<&OutcomeProviderErrorPreimage, SchemaV2Error> {
    let provider_error = attempt.result.provider_error.as_ref().ok_or_else(|| {
        SchemaV2Error::new(
            "outcome_transport_provider_error_missing",
            "provider failure requires typed provider error",
        )
    })?;
    if sha256_json(provider_error)?
        != *attempt
            .result
            .provider_error_hash
            .as_ref()
            .expect("pair validated")
    {
        return Err(SchemaV2Error::new(
            "outcome_transport_provider_error_hash_mismatch",
            "typed provider-error hash must recompute exactly",
        ));
    }
    require_trim_stable_non_empty(&provider_error.variant, "outcome_provider_error_variant")?;
    if let Some(hash) = provider_error.structured_detail_hash.as_deref() {
        require_hash(hash, "outcome_provider_error_structured_detail_hash")?;
    }
    if (provider_error.variant == "historical_bar_cardinality")
        != provider_error.historical_bar_cardinality.is_some()
    {
        return Err(SchemaV2Error::new(
            "outcome_transport_provider_error_detail_matrix_mismatch",
            "only historical_bar_cardinality may carry typed page geometry",
        ));
    }
    Ok(provider_error)
}

fn validate_outcome_cardinality_geometry(
    cardinality: &OutcomeHistoricalBarCardinalityPreimage,
    requested_latest_n: u16,
) -> Result<u16, SchemaV2Error> {
    if cardinality.requested_total != requested_latest_n
        || cardinality.expected_page == 0
        || !cardinality
            .offset
            .is_multiple_of(OUTCOME_TDX_HISTORICAL_PAGE_SIZE)
        || cardinality.offset >= u32::from(cardinality.requested_total)
    {
        return Err(SchemaV2Error::new(
            "outcome_transport_cardinality_geometry_invalid",
            "typed cardinality error does not bind the current 800-row request geometry",
        ));
    }
    let expected_page = (u32::from(cardinality.requested_total) - cardinality.offset)
        .min(OUTCOME_TDX_HISTORICAL_PAGE_SIZE);
    if u32::from(cardinality.expected_page) != expected_page
        || cardinality.actual >= u64::from(cardinality.expected_page)
    {
        return Err(SchemaV2Error::new(
            "outcome_transport_cardinality_page_invalid",
            "typed cardinality page actual/expected values are impossible",
        ));
    }
    let actual = u32::try_from(cardinality.actual).map_err(|_| {
        SchemaV2Error::new(
            "outcome_transport_cardinality_actual_overflow",
            "typed cardinality actual exceeds u32",
        )
    })?;
    let available = cardinality.offset.checked_add(actual).ok_or_else(|| {
        SchemaV2Error::new(
            "outcome_transport_cardinality_available_overflow",
            "typed available cardinality overflows u32",
        )
    })?;
    u16::try_from(available).map_err(|_| {
        SchemaV2Error::new(
            "outcome_transport_cardinality_available_overflow",
            "typed available cardinality exceeds u16",
        )
    })
}

canonical_struct!(RequestHashPreimage {
    domain: String,
    request_kind: RequestKind,
    canonical_subject: String,
    parameters_json_hash: String,
    provider_capability_hash: String,
});

canonical_struct!(RequestEvidencePreimage {
    domain: String,
    request_kind: RequestKind,
    canonical_subject: String,
    parameters_schema: String,
    parameters_json: String,
    parameters_json_hash: String,
    provider_capability_json: String,
    provider_capability_hash: String,
    request_hash: String,
});

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "the public canonical request-preimage API stays inline so BR-174/BR-182 hashes and cross-module constructors remain unchanged"
)]
pub enum RequestParametersPreimage {
    GlobalNews(GlobalNewsRequestParametersPreimage),
    BoardConstituents(BoardConstituentRequestParametersPreimage),
    T0MarketEvidence(T0MarketRequestParametersPreimage),
    OutcomeMarketEvidence(OutcomeMarketRequestParametersPreimage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestEvidenceColumns {
    pub request_hash: String,
    pub request_evidence_json: String,
    pub request_evidence_hash: String,
}

impl RequestParametersPreimage {
    fn request_kind(&self) -> RequestKind {
        match self {
            Self::GlobalNews(_) => RequestKind::GlobalNews,
            Self::BoardConstituents(_) => RequestKind::BoardConstituents,
            Self::T0MarketEvidence(_) => RequestKind::T0MarketEvidence,
            Self::OutcomeMarketEvidence(_) => RequestKind::OutcomeMarketEvidence,
        }
    }

    fn parameters_schema(&self) -> &'static str {
        match self {
            Self::GlobalNews(_) => GLOBAL_NEWS_REQUEST_PARAMETERS_SCHEMA,
            Self::BoardConstituents(_) => BOARD_CONSTITUENTS_REQUEST_PARAMETERS_SCHEMA,
            Self::T0MarketEvidence(_) => T0_MARKET_REQUEST_PARAMETERS_SCHEMA,
            Self::OutcomeMarketEvidence(_) => OUTCOME_MARKET_REQUEST_PARAMETERS_SCHEMA,
        }
    }

    fn canonical_subject(&self) -> String {
        match self {
            Self::GlobalNews(parameters) => parameters.feed_identity.clone(),
            Self::BoardConstituents(parameters) => parameters.binding_audit_hash.clone(),
            Self::T0MarketEvidence(parameters) => format!(
                "{}/{}",
                parameters.canonical_stock_code, parameters.evaluation_market_date
            ),
            Self::OutcomeMarketEvidence(parameters) => format!(
                "{}/{}/{}",
                parameters.sample_key,
                parameters.phase.as_str(),
                parameters.stored_due_date
            ),
        }
    }

    fn canonical_parameters_json(&self) -> Result<String, SchemaV2Error> {
        match self {
            Self::GlobalNews(parameters) => canonical_json(parameters),
            Self::BoardConstituents(parameters) => canonical_json(parameters),
            Self::T0MarketEvidence(parameters) => canonical_json(parameters),
            Self::OutcomeMarketEvidence(parameters) => canonical_json(parameters),
        }
    }

    fn validate(&self) -> Result<(), SchemaV2Error> {
        match self {
            Self::GlobalNews(parameters) => {
                require_domain(&parameters.domain, DOMAIN_GLOBAL_NEWS_REQUEST)?;
                require_hash(&parameters.feed_identity, "feed_identity")?;
                if parameters.limit != 20 {
                    return Err(SchemaV2Error::new(
                        "global_news_request_limit_mismatch",
                        "global-news requests must use the registered limit of 20",
                    ));
                }
            }
            Self::BoardConstituents(parameters) => {
                require_domain(&parameters.domain, DOMAIN_BOARD_CONSTITUENT_REQUEST)?;
                require_hash(&parameters.artifact_content_hash, "artifact_content_hash")?;
                require_hash(&parameters.binding_audit_hash, "binding_audit_hash")?;
                validate_fixed_board_binding(
                    "request-evidence",
                    &parameters.provider,
                    parameters.kind,
                    &parameters.code,
                    &parameters.name,
                )?;
                if parameters.limit != BOARD_DIRECTORY_REQUEST_LIMIT {
                    return Err(SchemaV2Error::new(
                        "board_request_limit_mismatch",
                        format!("board requests must use limit {BOARD_DIRECTORY_REQUEST_LIMIT}"),
                    ));
                }
            }
            Self::T0MarketEvidence(parameters) => {
                require_domain(&parameters.domain, DOMAIN_T0_MARKET_REQUEST)?;
                require_subject_component(
                    &parameters.canonical_stock_code,
                    "canonical_stock_code",
                )?;
                require_subject_component(&parameters.canonical_market, "canonical_market")?;
                parse_canonical_date(&parameters.evaluation_market_date, "evaluation_market_date")?;
                if parameters.quote_max_age_secs == 0
                    || parameters.daily_interval != DailyIntervalKind::Day.as_str()
                    || parameters.daily_limit == 0
                    || parameters.intraday_interval.is_empty()
                    || parameters.intraday_limit == 0
                    || parameters.adjustment != AdjustmentKind::None
                {
                    return Err(SchemaV2Error::new(
                        "t0_market_request_invalid",
                        "T0 request requires positive limits/age, day daily interval, a non-empty intraday interval, and no adjustment",
                    ));
                }
                require_trim_stable_non_empty(&parameters.intraday_interval, "intraday_interval")?;
            }
            Self::OutcomeMarketEvidence(parameters) => {
                require_domain(&parameters.domain, DOMAIN_OUTCOME_MARKET_REQUEST)?;
                require_hash(&parameters.sample_key, "sample_key")?;
                require_subject_component(
                    &parameters.canonical_stock_code,
                    "canonical_stock_code",
                )?;
                require_subject_component(&parameters.canonical_market, "canonical_market")?;
                require_trim_stable_non_empty(&parameters.calendar_version, "calendar_version")?;
                require_hash(&parameters.calendar_hash, "calendar_hash")?;
                require_hash(
                    &parameters.trading_date_vector_hash,
                    "trading_date_vector_hash",
                )?;
                parameters.trading_date_vector.validate()?;
                if sha256_json(&parameters.trading_date_vector)?
                    != parameters.trading_date_vector_hash
                {
                    return Err(SchemaV2Error::new(
                        "outcome_trading_date_vector_hash_mismatch",
                        "trading_date_vector_hash must bind the canonical full vector",
                    ));
                }
                let expected = parameters
                    .trading_date_vector
                    .applicable_dates(parameters.phase)?;
                if parameters.applicable_trading_dates != expected {
                    return Err(SchemaV2Error::new(
                        "outcome_applicable_trading_dates_mismatch",
                        "applicable_trading_dates must equal the exact phase prefix",
                    ));
                }
                let due = parse_canonical_date(&parameters.stored_due_date, "stored_due_date")?;
                let start = parse_canonical_date(&parameters.window_start, "window_start")?;
                let end = parse_canonical_date(&parameters.window_end, "window_end")?;
                let expected_start =
                    parse_canonical_date(&expected[0], "applicable_trading_dates[0]")?;
                let expected_end = parse_canonical_date(
                    expected.last().expect("phase prefix is non-empty"),
                    "applicable_trading_dates[last]",
                )?;
                if start != expected_start
                    || end != expected_end
                    || due != expected_end
                    || parameters.interval != DailyIntervalKind::Day
                    || parameters.adjustment != AdjustmentKind::None
                {
                    return Err(SchemaV2Error::new(
                        "outcome_market_request_invalid",
                        "outcome request endpoints/due must equal the exact phase prefix, use daily bars, and no adjustment",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn require_subject_component(value: &str, field: &'static str) -> Result<(), SchemaV2Error> {
    require_trim_stable_non_empty(value, field)?;
    if value.contains('/') {
        Err(SchemaV2Error::new(
            "request_subject_component_invalid",
            format!("{field} must not contain '/'"),
        ))
    } else {
        Ok(())
    }
}

fn validate_request_capability(
    request_kind: RequestKind,
    capability: &ProviderCapabilityHashPreimage,
) -> Result<(), SchemaV2Error> {
    require_domain(&capability.domain, DOMAIN_PROVIDER_CAPABILITY)?;
    if capability.upstream_revision != UPSTREAM_REVISION {
        return Err(SchemaV2Error::new(
            "request_capability_revision_mismatch",
            format!("upstream_revision must equal {UPSTREAM_REVISION}"),
        ));
    }
    let valid = match request_kind {
        RequestKind::GlobalNews => {
            capability.contract_version == "magic-market-core.NewsProvider.global_news.v0.2.0"
                && matches!(
                    (
                        capability.provider.as_str(),
                        capability.capability_name.as_str()
                    ),
                    ("eastmoney", "GlobalNews-Eastmoney")
                        | ("cailianpress", "GlobalNews-CLS")
                        | ("jin10", "GlobalNews-Jin10")
                        | ("thepaper", "GlobalNews-ThePaper")
                )
        }
        RequestKind::BoardConstituents => {
            capability.provider == "magic-tdx"
                && capability.capability_name == "MagicTdx-BoardConstituents"
                && capability.contract_version
                    == "magic-tdx-rs.BlockProvider.board_constituents.v0.2.0"
        }
        RequestKind::T0MarketEvidence => {
            capability.provider == "magic-tdx"
                && capability.capability_name == "MagicTdx-T0MarketBundle"
                && capability.contract_version
                    == "stock-analysis.MagicTdxSelectionGateway.t0_market_evidence.v1"
        }
        RequestKind::OutcomeMarketEvidence => {
            capability.provider == "magic-tdx"
                && capability.capability_name == "MagicTdx-UnadjustedDailyBars"
                && capability.contract_version == "magic-market-core.MarketDataProvider.bars.v0.2.0"
        }
    };
    if valid {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            "request_capability_mismatch",
            format!(
                "capability {}/{} does not implement {}",
                capability.provider,
                capability.capability_name,
                request_kind.as_str()
            ),
        ))
    }
}

fn registered_global_news_feed_identity(
    feed_name: &str,
    gateway_provider: &str,
    provider_id: &str,
    source_contract: &str,
    capability_name: &str,
) -> Result<String, SchemaV2Error> {
    let configuration = RegisteredFeedConfigurationPreimage {
        domain: DOMAIN_REGISTERED_FEED_CONFIG.into(),
        gateway_provider: gateway_provider.into(),
        provider_id: provider_id.into(),
        source_contract: source_contract.into(),
        capability_name: capability_name.into(),
        max_limit: 20,
        upstream_revision: UPSTREAM_REVISION.into(),
    };
    let identity = RegisteredFeedIdentityPreimage {
        domain: DOMAIN_REGISTERED_FEED_IDENTITY.into(),
        feed_name: feed_name.into(),
        gateway_provider: gateway_provider.into(),
        configuration_hash: sha256_json(&configuration)?,
    };
    sha256_json(&identity)
}

const REGISTERED_GLOBAL_NEWS_FEEDS: [(&str, &str, &str, &str, &str); 4] = [
    (
        "eastmoney_global_news",
        "eastmoney",
        "eastmoney",
        "eastmoney-web",
        "GlobalNews-Eastmoney",
    ),
    (
        "cls_global_news",
        "cailianpress",
        "cailianpress",
        "cls-v1",
        "GlobalNews-CLS",
    ),
    (
        "jin10_global_news",
        "jin10",
        "jin10",
        "jin10-flash-v1",
        "GlobalNews-Jin10",
    ),
    (
        "thepaper_global_news",
        "thepaper",
        "thepaper",
        "thepaper-finance-v1",
        "GlobalNews-ThePaper",
    ),
];

fn production_registered_global_news_entries(
) -> Result<Vec<RegisteredFeedEntryPreimage>, SchemaV2Error> {
    let mut entries = REGISTERED_GLOBAL_NEWS_FEEDS
        .iter()
        .map(
            |(feed_name, gateway_provider, provider_id, source_contract, capability_name)| {
                let configuration = RegisteredFeedConfigurationPreimage {
                    domain: DOMAIN_REGISTERED_FEED_CONFIG.into(),
                    gateway_provider: (*gateway_provider).into(),
                    provider_id: (*provider_id).into(),
                    source_contract: (*source_contract).into(),
                    capability_name: (*capability_name).into(),
                    max_limit: 20,
                    upstream_revision: UPSTREAM_REVISION.into(),
                };
                let configuration_hash = sha256_json(&configuration)?;
                let identity = RegisteredFeedIdentityPreimage {
                    domain: DOMAIN_REGISTERED_FEED_IDENTITY.into(),
                    feed_name: (*feed_name).into(),
                    gateway_provider: (*gateway_provider).into(),
                    configuration_hash: configuration_hash.clone(),
                };
                Ok(RegisteredFeedEntryPreimage {
                    ordinal: 0,
                    feed_identity: sha256_json(&identity)?,
                    gateway_provider: (*gateway_provider).into(),
                    capability_name: (*capability_name).into(),
                    configuration_hash,
                })
            },
        )
        .collect::<Result<Vec<_>, SchemaV2Error>>()?;
    entries.sort_by(|left, right| {
        left.feed_identity
            .as_bytes()
            .cmp(right.feed_identity.as_bytes())
    });
    for (ordinal, entry) in entries.iter_mut().enumerate() {
        entry.ordinal = u32::try_from(ordinal).map_err(|_| {
            SchemaV2Error::new(
                "registered_feed_ordinal_overflow",
                "registered feed ordinal does not fit u32",
            )
        })?;
    }
    Ok(entries)
}

fn validate_global_news_feed_capability(
    feed_identity: &str,
    capability: &ProviderCapabilityHashPreimage,
) -> Result<(), SchemaV2Error> {
    let mut matched = None;
    for (feed_name, gateway_provider, provider_id, source_contract, capability_name) in
        REGISTERED_GLOBAL_NEWS_FEEDS
    {
        if registered_global_news_feed_identity(
            feed_name,
            gateway_provider,
            provider_id,
            source_contract,
            capability_name,
        )? == feed_identity
        {
            matched = Some((gateway_provider, capability_name));
            break;
        }
    }
    let Some((expected_provider, expected_capability)) = matched else {
        return Err(SchemaV2Error::new(
            "global_news_feed_unregistered",
            "feed_identity is not one of the four checked-in global-news registrations",
        ));
    };
    if capability.provider != expected_provider
        || capability.capability_name != expected_capability
        || capability.contract_version != "magic-market-core.NewsProvider.global_news.v0.2.0"
        || capability.upstream_revision != UPSTREAM_REVISION
    {
        return Err(SchemaV2Error::new(
            "global_news_feed_capability_mismatch",
            "feed_identity must select its one exact checked-in provider capability tuple",
        ));
    }
    Ok(())
}

pub fn build_request_evidence(
    parameters: RequestParametersPreimage,
    capability: ProviderCapabilityHashPreimage,
) -> Result<RequestEvidenceColumns, SchemaV2Error> {
    parameters.validate()?;
    let request_kind = parameters.request_kind();
    validate_request_capability(request_kind, &capability)?;
    if let RequestParametersPreimage::GlobalNews(global_news) = &parameters {
        validate_global_news_feed_capability(&global_news.feed_identity, &capability)?;
    }
    let parameters_json = parameters.canonical_parameters_json()?;
    let parameters_json_hash = sha256_bytes(parameters_json.as_bytes());
    let provider_capability_json = canonical_json(&capability)?;
    let provider_capability_hash = sha256_bytes(provider_capability_json.as_bytes());
    let canonical_subject = parameters.canonical_subject();
    let request_hash = sha256_json(&RequestHashPreimage {
        domain: DOMAIN_REQUEST.into(),
        request_kind,
        canonical_subject: canonical_subject.clone(),
        parameters_json_hash: parameters_json_hash.clone(),
        provider_capability_hash: provider_capability_hash.clone(),
    })?;
    let evidence = RequestEvidencePreimage {
        domain: DOMAIN_REQUEST_EVIDENCE.into(),
        request_kind,
        canonical_subject,
        parameters_schema: parameters.parameters_schema().into(),
        parameters_json,
        parameters_json_hash,
        provider_capability_json,
        provider_capability_hash,
        request_hash: request_hash.clone(),
    };
    let request_evidence_json = canonical_json(&evidence)?;
    let request_evidence_hash = sha256_bytes(request_evidence_json.as_bytes());
    Ok(RequestEvidenceColumns {
        request_hash,
        request_evidence_json,
        request_evidence_hash,
    })
}

impl RequestEvidenceColumns {
    pub fn validate(
        &self,
        expected_kind: Option<RequestKind>,
    ) -> Result<RequestEvidencePreimage, SchemaV2Error> {
        require_hash(&self.request_hash, "request_hash")?;
        let evidence = validate_canonical_json_hash::<RequestEvidencePreimage>(
            &self.request_evidence_json,
            &self.request_evidence_hash,
        )?;
        evidence.validate(expected_kind)?;
        if evidence.request_hash != self.request_hash {
            return Err(SchemaV2Error::new(
                "request_evidence_projection_mismatch",
                "row request_hash must equal the typed request evidence",
            ));
        }
        Ok(evidence)
    }
}

impl RequestEvidencePreimage {
    pub fn validate(&self, expected_kind: Option<RequestKind>) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_REQUEST_EVIDENCE)?;
        if expected_kind.is_some_and(|kind| kind != self.request_kind) {
            return Err(SchemaV2Error::new(
                "request_kind_mismatch",
                "typed request evidence has the wrong request kind for this row",
            ));
        }
        let parameters = match self.request_kind {
            RequestKind::GlobalNews
                if self.parameters_schema == GLOBAL_NEWS_REQUEST_PARAMETERS_SCHEMA =>
            {
                RequestParametersPreimage::GlobalNews(validate_canonical_json_hash::<
                    GlobalNewsRequestParametersPreimage,
                >(
                    &self.parameters_json,
                    &self.parameters_json_hash,
                )?)
            }
            RequestKind::BoardConstituents
                if self.parameters_schema == BOARD_CONSTITUENTS_REQUEST_PARAMETERS_SCHEMA =>
            {
                RequestParametersPreimage::BoardConstituents(validate_canonical_json_hash::<
                    BoardConstituentRequestParametersPreimage,
                >(
                    &self.parameters_json,
                    &self.parameters_json_hash,
                )?)
            }
            RequestKind::T0MarketEvidence
                if self.parameters_schema == T0_MARKET_REQUEST_PARAMETERS_SCHEMA =>
            {
                RequestParametersPreimage::T0MarketEvidence(validate_canonical_json_hash::<
                    T0MarketRequestParametersPreimage,
                >(
                    &self.parameters_json,
                    &self.parameters_json_hash,
                )?)
            }
            RequestKind::OutcomeMarketEvidence
                if self.parameters_schema == OUTCOME_MARKET_REQUEST_PARAMETERS_SCHEMA =>
            {
                RequestParametersPreimage::OutcomeMarketEvidence(validate_canonical_json_hash::<
                    OutcomeMarketRequestParametersPreimage,
                >(
                    &self.parameters_json,
                    &self.parameters_json_hash,
                )?)
            }
            _ => {
                return Err(SchemaV2Error::new(
                    "request_parameters_schema_mismatch",
                    "request kind and typed parameters schema are not an exact pair",
                ));
            }
        };
        parameters.validate()?;
        if parameters.request_kind() != self.request_kind
            || parameters.canonical_subject() != self.canonical_subject
        {
            return Err(SchemaV2Error::new(
                "request_subject_mismatch",
                "canonical_subject must be reconstructed exactly from typed parameters",
            ));
        }
        let capability = validate_canonical_json_hash::<ProviderCapabilityHashPreimage>(
            &self.provider_capability_json,
            &self.provider_capability_hash,
        )?;
        validate_request_capability(self.request_kind, &capability)?;
        if let RequestParametersPreimage::GlobalNews(global_news) = &parameters {
            validate_global_news_feed_capability(&global_news.feed_identity, &capability)?;
        }
        let expected_hash = sha256_json(&RequestHashPreimage {
            domain: DOMAIN_REQUEST.into(),
            request_kind: self.request_kind,
            canonical_subject: self.canonical_subject.clone(),
            parameters_json_hash: self.parameters_json_hash.clone(),
            provider_capability_hash: self.provider_capability_hash.clone(),
        })?;
        if self.request_hash != expected_hash {
            return Err(SchemaV2Error::new(
                "request_hash_mismatch",
                "request_hash does not bind the typed parameters, subject, and capability",
            ));
        }
        Ok(())
    }
}

pub fn validate_request_evidence_columns(
    request_hash: Option<&str>,
    request_evidence_json: Option<&str>,
    request_evidence_hash: Option<&str>,
    expected_kind: Option<RequestKind>,
) -> Result<Option<RequestEvidencePreimage>, SchemaV2Error> {
    match (request_hash, request_evidence_json, request_evidence_hash) {
        (None, None, None) => Ok(None),
        (Some(request_hash), Some(request_evidence_json), Some(request_evidence_hash)) => {
            RequestEvidenceColumns {
                request_hash: request_hash.into(),
                request_evidence_json: request_evidence_json.into(),
                request_evidence_hash: request_evidence_hash.into(),
            }
            .validate(expected_kind)
            .map(Some)
        }
        _ => Err(SchemaV2Error::new(
            "request_evidence_column_matrix_mismatch",
            "request_hash, request_evidence_json, and request_evidence_hash must all be NULL or all present",
        )),
    }
}

canonical_struct!(RawSecurityIdentityPreimage {
    domain: String,
    provider: String,
    exchange: String,
    code: String,
    asset_class: String,
});

canonical_struct!(ProviderAvailableEvidencePreimage {
    domain: String,
    evidence_kind: ProviderEvidenceKind,
    provider: String,
    source: Option<String>,
    source_at: Option<String>,
    observed_at: Option<String>,
    batch_id: Option<String>,
    batch_content_hash: Option<String>,
});

impl ProviderAvailableEvidencePreimage {
    pub fn validate_partial(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_PROVIDER_AVAILABLE_EVIDENCE)?;
        require_non_empty(&self.provider, "provider")?;
        if self.source.is_none()
            && self.source_at.is_none()
            && self.observed_at.is_none()
            && self.batch_id.is_none()
            && self.batch_content_hash.is_none()
        {
            return Err(SchemaV2Error::new(
                "provider_evidence_all_null",
                "partial evidence must contain at least one observed field",
            ));
        }
        Ok(())
    }

    pub fn validate_complete(&self) -> Result<(), SchemaV2Error> {
        self.validate_partial()?;
        if self.source.is_none()
            || self.observed_at.is_none()
            || self.batch_id.is_none()
            || self.batch_content_hash.is_none()
        {
            return Err(SchemaV2Error::new(
                "provider_evidence_incomplete",
                "complete evidence requires source/observed_at/batch_id/batch_content_hash",
            ));
        }
        Ok(())
    }
}

canonical_struct!(OutcomeProviderAvailableEvidencePreimage {
    domain: String,
    request_hash: String,
    calendar_hash: String,
    trading_date_vector_hash: String,
    expected_trading_dates: Vec<String>,
    returned_trading_dates: Vec<String>,
    provider_evidence: ProviderAvailableEvidencePreimage,
});

impl OutcomeProviderAvailableEvidencePreimage {
    pub fn validate_partial(
        &self,
        request: &OutcomeMarketRequestParametersPreimage,
        request_hash: &str,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE)?;
        require_hash(&self.request_hash, "outcome_evidence.request_hash")?;
        require_hash(&self.calendar_hash, "outcome_evidence.calendar_hash")?;
        require_hash(
            &self.trading_date_vector_hash,
            "outcome_evidence.trading_date_vector_hash",
        )?;
        self.provider_evidence.validate_partial()?;
        let expected = request
            .trading_date_vector
            .applicable_dates(request.phase)?;
        if self.request_hash != request_hash
            || self.calendar_hash != request.calendar_hash
            || self.trading_date_vector_hash != request.trading_date_vector_hash
            || self.expected_trading_dates != expected
        {
            return Err(SchemaV2Error::new(
                "outcome_available_evidence_request_mismatch",
                "outcome evidence must bind the exact semantic request/vector/prefix",
            ));
        }
        if self.returned_trading_dates.is_empty() {
            return Err(SchemaV2Error::new(
                "outcome_returned_trading_dates_empty",
                "available outcome evidence must retain at least one real returned date",
            ));
        }
        let mut previous = None;
        for (index, value) in self.returned_trading_dates.iter().enumerate() {
            let date = parse_canonical_date(value, "returned_trading_dates")?;
            if previous.is_some_and(|prior| prior >= date) {
                return Err(SchemaV2Error::new(
                    "outcome_returned_trading_dates_not_strict",
                    format!("returned_trading_dates is not strictly ordered at index {index}"),
                ));
            }
            previous = Some(date);
        }
        Ok(())
    }

    pub fn validate_complete(
        &self,
        request: &OutcomeMarketRequestParametersPreimage,
        request_hash: &str,
    ) -> Result<(), SchemaV2Error> {
        self.validate_partial(request, request_hash)?;
        self.provider_evidence.validate_complete()?;
        if self.returned_trading_dates != self.expected_trading_dates {
            return Err(SchemaV2Error::new(
                "outcome_returned_trading_dates_mismatch",
                "complete outcome evidence requires exact element-for-element phase dates",
            ));
        }
        Ok(())
    }
}

canonical_struct!(ProviderErrorDetailPreimage {
    domain: String,
    error_kind: ProviderErrorKind,
    provider: String,
    operation: String,
    error_code: Option<String>,
    http_status: Option<u16>,
    timeout_ms: Option<u64>,
    invariant_id: Option<String>,
    diagnostic_code: String,
});

impl ProviderErrorDetailPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_PROVIDER_ERROR_DETAIL)?;
        require_non_empty(&self.provider, "provider")?;
        require_non_empty(&self.operation, "operation")?;
        if !is_snake_case_token(&self.diagnostic_code) {
            return Err(SchemaV2Error::new(
                "invalid_diagnostic_code",
                "diagnostic_code must be lowercase ASCII snake_case",
            ));
        }
        match self.error_kind {
            ProviderErrorKind::Timeout
                if self.timeout_ms.is_some()
                    && self.http_status.is_none()
                    && self.invariant_id.is_none() => {}
            ProviderErrorKind::Protocol
                if self.timeout_ms.is_none() && self.invariant_id.is_none() => {}
            ProviderErrorKind::Integrity
                if self.invariant_id.is_some()
                    && self.timeout_ms.is_none()
                    && self.http_status.is_none() => {}
            ProviderErrorKind::Transport
            | ProviderErrorKind::InvalidData
            | ProviderErrorKind::Unsupported
                if self.timeout_ms.is_none()
                    && self.http_status.is_none()
                    && self.invariant_id.is_none() => {}
            _ => {
                return Err(SchemaV2Error::new(
                    "invalid_provider_error_matrix",
                    "optional fields do not match provider error kind",
                ));
            }
        }
        Ok(())
    }
}

fn is_snake_case_token(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('_')
        && !value.ends_with('_')
        && !value.contains("__")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

canonical_struct!(T0FeaturePreimage {
    domain: String,
    feature_version: String,
    evaluation_window: EvaluationWindow,
    ma5: Option<String>,
    ma10: Option<String>,
    ma20: Option<String>,
    five_day_return: Option<String>,
    volume_vs_5d: Option<String>,
    volume_vs_20d: Option<String>,
    intraday_volume_pace: Option<String>,
    price_vs_ma5: Option<String>,
    price_vs_ma10: Option<String>,
    price_vs_ma20: Option<String>,
    evaluation_price: String,
    observed_volume: String,
    latest_settled_market_date: String,
    latest_settled_close: String,
    latest_settled_volume: String,
    prior_5d_average_volume: String,
    prior_20d_average_volume: String,
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdmissionStructuredDetailPreimage {
    MovingAverageNonpositive {
        ma5: String,
        ma10: String,
        ma20: String,
    },
    TrendAlignmentFailed {
        ma5: String,
        ma10: String,
        ma20: String,
    },
    PriceBelowMa5 {
        value: String,
        inclusive_min: String,
    },
    PriceMa20DistanceOutOfRange {
        value: String,
        inclusive_min: String,
        inclusive_max: String,
    },
    FiveDayReturnOutOfRange {
        value: String,
        inclusive_min: String,
        inclusive_max: String,
    },
    SettledVolumeConfirmationFailed {
        volume_vs_5d: String,
        volume_vs_20d: String,
        inclusive_min: String,
    },
    IntradayVolumeConfirmationFailed {
        intraday_volume_pace: String,
        inclusive_min: String,
    },
}

canonical_struct!(ErrorFingerprintPreimage {
    domain: String,
    failed_stage: String,
    reason_code: String,
    retryable: bool,
    available_evidence_hash: Option<String>,
    detail_hash: String,
});

canonical_struct!(OutcomeErrorFingerprintPreimageV2 {
    domain: String,
    failed_stage: String,
    reason_code: String,
    retryable: bool,
    available_evidence_hash: Option<String>,
    detail_hash: String,
    transport_attempts_hash: String,
});

// The twelve immutable row-content registries. Declaration order is serialization order.
canonical_struct!(SelectionSourceBatchAttemptRowContentPreimage {
    domain: String,
    source_batch_attempt_id: String,
    ingress_run_id: String,
    config_activation_run_id: String,
    config_hash: String,
    generation_market_date: String,
    registered_feed_identity: String,
    registered_feed_snapshot_hash: String,
    request_hash: String,
    request_evidence_json: String,
    request_evidence_hash: String,
    feed_attempt_content_hash: String,
    status_kind: FeedStatusKind,
    record_count: Option<u32>,
    provider: Option<String>,
    source: Option<String>,
    source_at: Option<String>,
    observed_at: Option<String>,
    batch_id: Option<String>,
    batch_content_hash: Option<String>,
    failed_stage: Option<String>,
    reason_code: Option<String>,
    retryable: Option<bool>,
    available_evidence_json: Option<String>,
    available_evidence_hash: Option<String>,
    error_detail_json: Option<String>,
    error_detail_hash: Option<String>,
    error_fingerprint: Option<String>,
    attempted_at: String,
});

impl SelectionSourceBatchAttemptRowContentPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_SOURCE_BATCH_ATTEMPT_ROW)?;
        require_hash(&self.source_batch_attempt_id, "source_batch_attempt_id")?;
        require_canonical_uuid_v7(&self.ingress_run_id, "ingress_run_id")?;
        require_canonical_uuid_v7(&self.config_activation_run_id, "config_activation_run_id")?;
        require_hash(&self.config_hash, "config_hash")?;
        parse_canonical_date(&self.generation_market_date, "generation_market_date")?;
        require_hash(&self.registered_feed_identity, "registered_feed_identity")?;
        require_hash(
            &self.registered_feed_snapshot_hash,
            "registered_feed_snapshot_hash",
        )?;
        let request = validate_request_evidence_columns(
            Some(&self.request_hash),
            Some(&self.request_evidence_json),
            Some(&self.request_evidence_hash),
            Some(RequestKind::GlobalNews),
        )?
        .expect("source attempt request evidence is required");
        let parameters = validate_canonical_json_hash::<GlobalNewsRequestParametersPreimage>(
            &request.parameters_json,
            &request.parameters_json_hash,
        )?;
        let request_capability = validate_canonical_json_hash::<ProviderCapabilityHashPreimage>(
            &request.provider_capability_json,
            &request.provider_capability_hash,
        )?;
        if parameters.feed_identity != self.registered_feed_identity {
            return Err(SchemaV2Error::new(
                "source_request_feed_projection_mismatch",
                "global-news request feed identity must equal the source attempt row",
            ));
        }
        require_pair(
            &self.available_evidence_json,
            &self.available_evidence_hash,
            "source_available_evidence_pair_mismatch",
        )?;
        require_pair(
            &self.error_detail_json,
            &self.error_detail_hash,
            "source_error_detail_pair_mismatch",
        )?;
        if let (Some(json), Some(hash)) = (
            self.available_evidence_json.as_deref(),
            self.available_evidence_hash.as_deref(),
        ) {
            match self.status_kind {
                FeedStatusKind::Available | FeedStatusKind::VerifiedEmpty => {
                    let evidence =
                        validate_canonical_json_hash::<FeedBatchEvidencePreimage>(json, hash)?;
                    require_domain(&evidence.domain, DOMAIN_FEED_BATCH_EVIDENCE)?;
                    if evidence.provider != request_capability.provider {
                        return Err(SchemaV2Error::new(
                            "source_provider_capability_projection_mismatch",
                            "complete source evidence provider must equal the requested provider capability",
                        ));
                    }
                    if evidence.feed_identity != self.registered_feed_identity
                        || self.provider.as_deref() != Some(&evidence.provider)
                        || self.source.as_deref() != Some(&evidence.source)
                        || self.source_at != evidence.source_at
                        || self.observed_at.as_deref() != Some(&evidence.observed_at)
                        || self.batch_id.as_deref() != Some(&evidence.batch_id)
                    {
                        return Err(SchemaV2Error::new(
                            "source_feed_evidence_projection_mismatch",
                            "source attempt provider fields must equal complete typed feed evidence",
                        ));
                    }
                }
                FeedStatusKind::Unavailable => {
                    let evidence =
                        validate_canonical_json_hash::<FeedAvailableEvidencePreimage>(json, hash)?;
                    require_domain(&evidence.domain, DOMAIN_FEED_AVAILABLE_EVIDENCE)?;
                    if evidence
                        .provider
                        .as_deref()
                        .is_some_and(|provider| provider != request_capability.provider)
                    {
                        return Err(SchemaV2Error::new(
                            "source_provider_capability_projection_mismatch",
                            "partial source evidence provider, when present, must equal the requested provider capability",
                        ));
                    }
                    if evidence.feed_identity != self.registered_feed_identity
                        || self.provider != evidence.provider
                        || self.source != evidence.source
                        || self.source_at != evidence.source_at
                        || self.observed_at != evidence.observed_at
                        || self.batch_id != evidence.batch_id
                        || self.batch_content_hash != evidence.batch_content_hash
                    {
                        return Err(SchemaV2Error::new(
                            "source_feed_evidence_projection_mismatch",
                            "source attempt provider fields must equal partial typed feed evidence",
                        ));
                    }
                }
            }
        }
        if let (Some(json), Some(hash)) = (
            self.error_detail_json.as_deref(),
            self.error_detail_hash.as_deref(),
        ) {
            let detail = validate_canonical_json_hash::<ProviderErrorDetailPreimage>(json, hash)?;
            detail.validate()?;
            if detail.provider != request_capability.provider {
                return Err(SchemaV2Error::new(
                    "source_error_provider_capability_projection_mismatch",
                    "source error provider must equal the requested provider capability",
                ));
            }
        }
        let content = FeedAttemptContentPreimage {
            domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
            feed_identity: self.registered_feed_identity.clone(),
            request_hash: self.request_hash.clone(),
            request_evidence_hash: self.request_evidence_hash.clone(),
            status_kind: self.status_kind,
            record_count: self.record_count,
            evidence_hash: match self.status_kind {
                FeedStatusKind::Available | FeedStatusKind::VerifiedEmpty => {
                    self.available_evidence_hash.clone()
                }
                FeedStatusKind::Unavailable => None,
            },
            source_content_hash: match self.status_kind {
                FeedStatusKind::Available | FeedStatusKind::VerifiedEmpty => {
                    self.batch_content_hash.clone()
                }
                FeedStatusKind::Unavailable => None,
            },
            available_evidence_hash: self.available_evidence_hash.clone(),
            failed_stage: self.failed_stage.clone(),
            reason_code: self.reason_code.clone(),
            retryable: self.retryable,
            detail_hash: self.error_detail_hash.clone(),
            error_fingerprint: self.error_fingerprint.clone(),
        };
        content.validate()?;
        if self.status_kind == FeedStatusKind::Unavailable {
            let fingerprint = ErrorFingerprintPreimage {
                domain: DOMAIN_ERROR_FINGERPRINT.into(),
                failed_stage: self
                    .failed_stage
                    .clone()
                    .expect("validated unavailable stage"),
                reason_code: self
                    .reason_code
                    .clone()
                    .expect("validated unavailable reason"),
                retryable: self.retryable.expect("validated unavailable retryability"),
                available_evidence_hash: self.available_evidence_hash.clone(),
                detail_hash: self
                    .error_detail_hash
                    .clone()
                    .expect("validated unavailable detail"),
            };
            if sha256_json(&fingerprint)?
                != *self
                    .error_fingerprint
                    .as_ref()
                    .expect("validated unavailable fingerprint")
            {
                return Err(SchemaV2Error::new(
                    "source_error_fingerprint_mismatch",
                    "source attempt error fingerprint must bind stage/reason/retry/evidence/detail",
                ));
            }
        }
        if sha256_json(&content)? != self.feed_attempt_content_hash {
            return Err(SchemaV2Error::new(
                "feed_attempt_content_hash_mismatch",
                "source attempt row must bind the exact request-bearing feed content",
            ));
        }
        parse_canonical_nanos_utc(&self.attempted_at, "attempted_at")?;
        Ok(())
    }
}

canonical_struct!(SelectionSourceFactRowContentPreimage {
    domain: String,
    source_fact_key: String,
    event_id: String,
    payload_schema: String,
    config_activation_run_id: String,
    config_hash: String,
    generation_market_date: String,
    provider_source: String,
    item_id: String,
    title: String,
    summary: Option<String>,
    content: Option<String>,
    publisher: String,
    canonical_url: String,
    published_at: String,
    instruments_json: String,
    topics_json: String,
    language: String,
    record_provider: String,
    record_source: String,
    record_source_at: Option<String>,
    record_observed_at: String,
    record_batch_id: String,
    record_batch_content_hash: String,
    provider_content_hash: String,
    first_ingress_run_id: String,
    ingress_gate_version: String,
    ingress_gate_input_json: String,
    ingress_gate_input_hash: String,
    ingress_decision: IngressDecision,
    ingress_reason_code: Option<String>,
    ingress_retryable: Option<bool>,
    ingress_gate_receipt_json: String,
    ingress_gate_receipt_hash: String,
});

canonical_struct!(SelectionSourceFactAttemptRowContentPreimage {
    domain: String,
    source_fact_attempt_id: String,
    ingress_run_id: String,
    source_batch_attempt_id: String,
    provider_ordinal: u32,
    source_fact_key: String,
    acquired_record_json: String,
    acquired_record_hash: String,
    batch_evidence_json: String,
    batch_evidence_hash: String,
    event_projection_id: String,
    attempt_result: SourceFactAttemptResult,
    conflict_hash: Option<String>,
    attempted_at: String,
});

fn source_event_projection_id(provider_source: &str, item_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"BR166_GLOBAL_NEWS_EVENT_V1\0");
    hasher.update(provider_source.as_bytes());
    hasher.update(b"\0");
    hasher.update(item_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn validate_sorted_source_values(
    json: &str,
    field: &'static str,
) -> Result<Vec<String>, SchemaV2Error> {
    let values = validate_canonical_json::<Vec<String>>(json)?;
    for value in &values {
        require_trim_stable_non_empty(value, field)?;
    }
    if values
        .windows(2)
        .any(|pair| pair[0].as_bytes() >= pair[1].as_bytes())
    {
        return Err(SchemaV2Error::new(
            "source_fact_list_not_sorted_unique",
            format!("{field} must be strictly UTF-8 byte sorted and unique"),
        ));
    }
    Ok(values)
}

impl SelectionSourceFactRowContentPreimage {
    pub fn validate(&self) -> Result<SourceFactContentPreimage, SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_SOURCE_FACT_ROW)?;
        require_hash(&self.source_fact_key, "source_fact_key")?;
        require_hash(&self.event_id, "event_id")?;
        if self.payload_schema != GLOBAL_NEWS_SOURCE_FACT_SCHEMA {
            return Err(SchemaV2Error::new(
                "source_fact_payload_schema_mismatch",
                "source facts must use the exact global-news-source-fact-v2 payload schema",
            ));
        }
        require_canonical_uuid_v7(
            &self.config_activation_run_id,
            "source_fact.config_activation_run_id",
        )?;
        require_hash(&self.config_hash, "source_fact.config_hash")?;
        parse_canonical_date(
            &self.generation_market_date,
            "source_fact.generation_market_date",
        )?;
        for (value, field) in [
            (&self.provider_source, "source_fact.provider_source"),
            (&self.item_id, "source_fact.item_id"),
            (&self.title, "source_fact.title"),
            (&self.publisher, "source_fact.publisher"),
            (&self.canonical_url, "source_fact.canonical_url"),
            (&self.language, "source_fact.language"),
            (&self.record_provider, "source_fact.record_provider"),
            (&self.record_source, "source_fact.record_source"),
            (&self.record_batch_id, "source_fact.record_batch_id"),
            (
                &self.ingress_gate_version,
                "source_fact.ingress_gate_version",
            ),
        ] {
            require_trim_stable_non_empty(value, field)?;
        }
        if !self.canonical_url.starts_with("https://") {
            return Err(SchemaV2Error::new(
                "source_fact_url_invalid",
                "source fact canonical_url must use HTTPS",
            ));
        }
        parse_canonical_nanos_utc(&self.published_at, "source_fact.published_at")?;
        parse_canonical_nanos_utc(&self.record_observed_at, "source_fact.record_observed_at")?;
        if let Some(source_at) = &self.record_source_at {
            parse_canonical_nanos_utc(source_at, "source_fact.record_source_at")?;
        }
        require_hash(
            &self.record_batch_content_hash,
            "source_fact.record_batch_content_hash",
        )?;
        require_hash(
            &self.provider_content_hash,
            "source_fact.provider_content_hash",
        )?;
        require_canonical_uuid_v7(
            &self.first_ingress_run_id,
            "source_fact.first_ingress_run_id",
        )?;
        let instruments =
            validate_sorted_source_values(&self.instruments_json, "source_fact.instrument")?;
        let topics = validate_sorted_source_values(&self.topics_json, "source_fact.topic")?;
        let expected_key = sha256_json(&SourceFactKeyPreimage {
            domain: DOMAIN_SOURCE_FACT_KEY.into(),
            provider_source: self.provider_source.clone(),
            item_id: self.item_id.clone(),
        })?;
        if expected_key != self.source_fact_key {
            return Err(SchemaV2Error::new(
                "source_fact_key_mismatch",
                "source_fact_key must bind provider_source and item_id",
            ));
        }
        if source_event_projection_id(&self.provider_source, &self.item_id) != self.event_id {
            return Err(SchemaV2Error::new(
                "source_fact_event_projection_mismatch",
                "event_id must use the frozen BR-166 provider/item projection",
            ));
        }
        let content = SourceFactContentPreimage {
            domain: DOMAIN_SOURCE_CONTENT.into(),
            provider_source: self.provider_source.clone(),
            item_id: self.item_id.clone(),
            title: self.title.clone(),
            summary: self.summary.clone(),
            content: self.content.clone(),
            publisher: self.publisher.clone(),
            canonical_url: self.canonical_url.clone(),
            published_at_rfc3339_nanos_utc: self.published_at.clone(),
            instruments_sorted: instruments,
            topics_sorted: topics,
            language: self.language.clone(),
            record_source: self.record_source.clone(),
            record_source_at: self.record_source_at.clone(),
        };
        if sha256_json(&content)? != self.provider_content_hash {
            return Err(SchemaV2Error::new(
                "source_fact_content_hash_mismatch",
                "provider_content_hash must bind the exact typed source content",
            ));
        }
        let gate_input = validate_canonical_json_hash::<IngressGateInputPreimage>(
            &self.ingress_gate_input_json,
            &self.ingress_gate_input_hash,
        )?;
        require_domain(&gate_input.domain, DOMAIN_INGRESS_GATE_INPUT)?;
        parse_canonical_nanos_utc(
            &gate_input.evaluated_at_rfc3339_nanos_utc,
            "source_fact.gate_input.evaluated_at",
        )?;
        if gate_input.source_fact_key != self.source_fact_key
            || gate_input.config_activation_run_id != self.config_activation_run_id
            || gate_input.config_hash != self.config_hash
            || gate_input.provider_published_at_rfc3339_nanos_utc != self.published_at
            || gate_input.record_observed_at != self.record_observed_at
            || gate_input.batch_observed_at != self.record_observed_at
            || gate_input.batch_content_hash != self.record_batch_content_hash
            || gate_input.gate_version != self.ingress_gate_version
            || gate_input.freshness_max_age_secs == 0
        {
            return Err(SchemaV2Error::new(
                "source_fact_gate_input_projection_mismatch",
                "ingress gate input must equal source/config/provider/batch projections",
            ));
        }
        let receipt = validate_canonical_json_hash::<IngressGateReceiptPreimage>(
            &self.ingress_gate_receipt_json,
            &self.ingress_gate_receipt_hash,
        )?;
        receipt.validate()?;
        if receipt.ingress_run_id != self.first_ingress_run_id
            || receipt.source_fact_key != self.source_fact_key
            || receipt.ingress_gate_input_hash != self.ingress_gate_input_hash
            || receipt.decision != self.ingress_decision
            || receipt.reason_code != self.ingress_reason_code
            || receipt.retryable != self.ingress_retryable
            || receipt.evaluated_at_rfc3339_nanos_utc != gate_input.evaluated_at_rfc3339_nanos_utc
        {
            return Err(SchemaV2Error::new(
                "source_fact_gate_receipt_projection_mismatch",
                "ingress gate receipt must equal the row decision and exact gate input",
            ));
        }
        Ok(content)
    }
}

impl SelectionSourceFactAttemptRowContentPreimage {
    pub fn validate(
        &self,
    ) -> Result<(AcquiredGlobalNewsRecordPreimage, FeedBatchEvidencePreimage), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_SOURCE_FACT_ATTEMPT_ROW)?;
        require_hash(&self.source_fact_attempt_id, "source_fact_attempt_id")?;
        require_canonical_uuid_v7(&self.ingress_run_id, "source_fact_attempt.ingress_run_id")?;
        require_hash(
            &self.source_batch_attempt_id,
            "source_fact_attempt.source_batch_attempt_id",
        )?;
        require_hash(&self.source_fact_key, "source_fact_attempt.source_fact_key")?;
        require_hash(
            &self.event_projection_id,
            "source_fact_attempt.event_projection_id",
        )?;
        let acquired = validate_canonical_json_hash::<AcquiredGlobalNewsRecordPreimage>(
            &self.acquired_record_json,
            &self.acquired_record_hash,
        )?;
        require_domain(&acquired.domain, DOMAIN_ACQUIRED_GLOBAL_NEWS_RECORD)?;
        require_domain(&acquired.record.domain, DOMAIN_SOURCE_CONTENT)?;
        let evidence = validate_canonical_json_hash::<FeedBatchEvidencePreimage>(
            &self.batch_evidence_json,
            &self.batch_evidence_hash,
        )?;
        require_domain(&evidence.domain, DOMAIN_FEED_BATCH_EVIDENCE)?;
        if evidence.batch_quality != FeedBatchQuality::Complete {
            return Err(SchemaV2Error::new(
                "source_fact_attempt_batch_quality_mismatch",
                "source fact attempts require complete provider batch evidence",
            ));
        }
        let expected_key = sha256_json(&SourceFactKeyPreimage {
            domain: DOMAIN_SOURCE_FACT_KEY.into(),
            provider_source: acquired.record.provider_source.clone(),
            item_id: acquired.record.item_id.clone(),
        })?;
        if acquired.source_fact_key != self.source_fact_key
            || expected_key != self.source_fact_key
            || sha256_json(&acquired.record)? != acquired.provider_content_hash
            || acquired.record_provider != evidence.provider
            || acquired.record_source != evidence.source
            || acquired.record_source_at != evidence.source_at
            || acquired.record_observed_at != evidence.observed_at
            || acquired.record_batch_id != evidence.batch_id
            || source_event_projection_id(
                &acquired.record.provider_source,
                &acquired.record.item_id,
            ) != self.event_projection_id
        {
            return Err(SchemaV2Error::new(
                "source_fact_attempt_projection_mismatch",
                "attempt row, acquired record, provider evidence and event projection must agree",
            ));
        }
        require_hash(
            &acquired.record_batch_content_hash,
            "acquired.record_batch_content_hash",
        )?;
        let attempt = SourceFactAttemptPreimage {
            domain: DOMAIN_SOURCE_ATTEMPT.into(),
            ingress_run_id: self.ingress_run_id.clone(),
            source_fact_key: self.source_fact_key.clone(),
            source_batch_attempt_id: self.source_batch_attempt_id.clone(),
            provider_ordinal: self.provider_ordinal,
            source_batch_id: evidence.batch_id.clone(),
            record_batch_id: acquired.record_batch_id.clone(),
            observed_at: acquired.record_observed_at.clone(),
            batch_evidence_hash: self.batch_evidence_hash.clone(),
        };
        if sha256_json(&attempt)? != self.source_fact_attempt_id {
            return Err(SchemaV2Error::new(
                "source_fact_attempt_id_mismatch",
                "source_fact_attempt_id must bind the exact acquisition attempt",
            ));
        }
        match self.attempt_result {
            SourceFactAttemptResult::Accepted | SourceFactAttemptResult::Replay
                if self.conflict_hash.is_none() => {}
            SourceFactAttemptResult::Conflict if self.conflict_hash.is_some() => {
                require_hash(
                    self.conflict_hash.as_deref().expect("checked above"),
                    "source_fact_attempt.conflict_hash",
                )?;
            }
            _ => {
                return Err(SchemaV2Error::new(
                    "source_fact_attempt_result_matrix_mismatch",
                    "accepted/replay require NULL conflict; conflict requires its hash",
                ));
            }
        }
        parse_canonical_nanos_utc(&self.attempted_at, "source_fact_attempt.attempted_at")?;
        Ok((acquired, evidence))
    }
}

canonical_struct!(SelectionRelationAttemptRowContentPreimage {
    domain: String,
    relation_attempt_id: String,
    relation_key: String,
    generation_run_id: String,
    source_fact_key: String,
    event_id: String,
    chain_id: String,
    config_activation_run_id: String,
    config_hash: String,
    relation_schema_version: String,
    relation_kind: RelationKind,
    relation_source_identity_json: String,
    relation_source_identity_hash: String,
    typed_binding_state_json: String,
    typed_binding_state_hash: String,
    request_hash: Option<String>,
    request_evidence_json: Option<String>,
    request_evidence_hash: Option<String>,
    result_code: String,
    failed_stage: Option<String>,
    retryable: Option<bool>,
    raw_identity_json: Option<String>,
    raw_identity_hash: Option<String>,
    canonical_stock_code: Option<String>,
    canonical_stock_name: Option<String>,
    canonical_market: Option<String>,
    artifact_content_hash: Option<String>,
    binding_audit_hash: Option<String>,
    provider_board_kind: Option<ProviderBoardKind>,
    provider_board_code: Option<String>,
    provider_board_name: Option<String>,
    provider_source: Option<String>,
    provider_source_at: Option<String>,
    provider_observed_at: Option<String>,
    provider_batch_id: Option<String>,
    provider_batch_content_hash: Option<String>,
    actual_constituent_count: Option<u32>,
    available_evidence_json: Option<String>,
    available_evidence_hash: Option<String>,
    error_detail_json: Option<String>,
    error_detail_hash: Option<String>,
    error_fingerprint: Option<String>,
    attempted_at: String,
});

canonical_struct!(SelectionEvaluationAttemptRowContentPreimage {
    domain: String,
    evaluation_attempt_id: String,
    sample_key: String,
    generation_run_id: String,
    source_fact_key: String,
    event_id: String,
    chain_id: String,
    canonical_stock_code: String,
    canonical_stock_name: String,
    canonical_market: String,
    relation_evidence_set_hash: String,
    market_request_hash: String,
    request_evidence_json: String,
    request_evidence_hash: String,
    result_code: String,
    failed_stage: Option<String>,
    retryable: Option<bool>,
    provider: Option<String>,
    source: Option<String>,
    source_at: Option<String>,
    observed_at: Option<String>,
    batch_id: Option<String>,
    batch_content_hash: Option<String>,
    available_evidence_json: Option<String>,
    available_evidence_hash: Option<String>,
    terminal_decision_hash: Option<String>,
    error_detail_json: Option<String>,
    error_detail_hash: Option<String>,
    error_fingerprint: Option<String>,
    attempted_at: String,
});

canonical_struct!(SelectionSampleRowContentPreimage {
    domain: String,
    sample_key: String,
    generation_run_id: String,
    source_fact_key: String,
    source_fact_content_hash: String,
    source_fact_attempt_id: String,
    source_batch_attempt_id: String,
    event_id: String,
    chain_id: String,
    config_activation_run_id: String,
    config_hash: String,
    matched_keyword: String,
    canonical_stock_code: String,
    canonical_stock_name: String,
    canonical_market: String,
    relation_schema_version: String,
    relation_evidence_json: String,
    relation_evidence_set_hash: String,
    feature_version: String,
    t0_feature_json: String,
    t0_feature_hash: String,
    market_provider: String,
    market_source: String,
    market_source_at: Option<String>,
    market_observed_at: String,
    market_batch_id: String,
    market_batch_content_hash: String,
    admission_version: String,
    decision_kind: TerminalDecisionKind,
    rejection_count: u32,
    rejection_row_hashes_in_ordinal_order: Vec<String>,
    evaluation_market_date: String,
    t0_due_date: String,
    d1_due_date: String,
    d2_due_date: String,
    d3_due_date: String,
    d4_due_date: String,
    d5_due_date: String,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector_json: String,
    trading_date_vector_hash: String,
    staged_at: String,
});

canonical_struct!(SelectionRejectionRowContentPreimage {
    domain: String,
    sample_key: String,
    ordinal: u32,
    generation_run_id: String,
    reason_code: String,
    rule_id: String,
    retryable: bool,
    structured_detail_json: String,
    structured_detail_hash: String,
    provider: Option<String>,
    source: Option<String>,
    source_at: Option<String>,
    observed_at: Option<String>,
    batch_id: Option<String>,
    batch_content_hash: Option<String>,
    created_at: String,
});

canonical_struct!(SelectionSampleOutcomeRowContentPreimage {
    domain: String,
    sample_key: String,
    phase: OutcomePhase,
    outcome_run_id: String,
    due_trading_date: String,
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
    amount: String,
    return_from_t0_close: String,
    cumulative_mfe: String,
    cumulative_mae: String,
    volume_ratio: String,
    provider: String,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    batch_content_hash: String,
    created_at: String,
});

canonical_struct!(SelectionOutcomeAttemptRowContentPreimageV3 {
    domain: String,
    outcome_attempt_id: String,
    sample_key: String,
    phase: OutcomePhase,
    stored_due_date: String,
    outcome_run_id: String,
    request_hash: Option<String>,
    request_evidence_json: Option<String>,
    request_evidence_hash: Option<String>,
    transport_attempts_json: Option<String>,
    transport_attempts_hash: Option<String>,
    result_code: OutcomeAttemptResult,
    reason_code: Option<OutcomeReasonCodeV1>,
    retryable: Option<bool>,
    provider: Option<String>,
    source: Option<String>,
    source_at: Option<String>,
    observed_at: Option<String>,
    batch_id: Option<String>,
    batch_content_hash: Option<String>,
    available_evidence_json: Option<String>,
    available_evidence_hash: Option<String>,
    error_detail_json: Option<String>,
    error_detail_hash: Option<String>,
    error_fingerprint: Option<String>,
    settled_outcome_content_hash: Option<String>,
    attempted_at: String,
});

pub type SelectionOutcomeAttemptRowContentPreimage = SelectionOutcomeAttemptRowContentPreimageV3;

impl SelectionOutcomeAttemptRowContentPreimageV3 {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_ATTEMPT_ROW)?;
        require_hash(&self.outcome_attempt_id, "outcome_attempt_id")?;
        require_hash(&self.sample_key, "sample_key")?;
        require_canonical_uuid_v7(&self.outcome_run_id, "outcome_run_id")?;
        parse_canonical_date(&self.stored_due_date, "stored_due_date")?;
        let request_evidence = validate_request_evidence_columns(
            self.request_hash.as_deref(),
            self.request_evidence_json.as_deref(),
            self.request_evidence_hash.as_deref(),
            Some(RequestKind::OutcomeMarketEvidence),
        )?;
        let request_capability = request_evidence
            .as_ref()
            .map(|request| {
                validate_canonical_json_hash::<ProviderCapabilityHashPreimage>(
                    &request.provider_capability_json,
                    &request.provider_capability_hash,
                )
            })
            .transpose()?;
        let request_parameters = request_evidence
            .as_ref()
            .map(|request| {
                validate_canonical_json_hash::<OutcomeMarketRequestParametersPreimage>(
                    &request.parameters_json,
                    &request.parameters_json_hash,
                )
            })
            .transpose()?;
        if let Some(parameters) = &request_parameters {
            if parameters.sample_key != self.sample_key
                || parameters.phase != self.phase
                || parameters.stored_due_date != self.stored_due_date
            {
                return Err(SchemaV2Error::new(
                    "outcome_request_projection_mismatch",
                    "typed outcome request must equal attempt sample/phase/due date",
                ));
            }
        }
        require_pair(
            &self.transport_attempts_json,
            &self.transport_attempts_hash,
            "outcome_transport_attempts_pair_mismatch",
        )?;
        let transport_attempts = match (
            self.transport_attempts_json.as_deref(),
            self.transport_attempts_hash.as_deref(),
        ) {
            (Some(json), Some(hash)) => {
                let attempts =
                    validate_canonical_json_hash::<OutcomeTransportAttemptsPreimage>(json, hash)?;
                let request = request_evidence.as_ref().ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_transport_request_missing",
                        "transport attempts require semantic request evidence",
                    )
                })?;
                attempts.validate(
                    self.request_hash.as_deref().expect("same optional request"),
                    self.request_evidence_hash
                        .as_deref()
                        .expect("same optional request"),
                    request,
                    request_parameters.as_ref().expect("same optional request"),
                    request_capability.as_ref().expect("same optional request"),
                )?;
                Some(attempts)
            }
            (None, None) => None,
            _ => unreachable!("require_pair rejected mismatched option state"),
        };
        require_pair(
            &self.available_evidence_json,
            &self.available_evidence_hash,
            "outcome_available_evidence_pair_mismatch",
        )?;
        require_pair(
            &self.error_detail_json,
            &self.error_detail_hash,
            "outcome_error_detail_pair_mismatch",
        )?;
        let available_evidence = match (
            self.available_evidence_json.as_deref(),
            self.available_evidence_hash.as_deref(),
        ) {
            (Some(json), Some(hash)) => {
                let evidence = validate_canonical_json_hash::<
                    OutcomeProviderAvailableEvidencePreimage,
                >(json, hash)?;
                let request = request_evidence.as_ref().ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_available_evidence_request_missing",
                        "outcome evidence requires semantic request evidence",
                    )
                })?;
                let parameters = request_parameters.as_ref().expect("same optional request");
                evidence.validate_partial(parameters, &request.request_hash)?;
                let provider_evidence = &evidence.provider_evidence;
                if request_capability
                    .as_ref()
                    .is_none_or(|capability| provider_evidence.provider != capability.provider)
                {
                    return Err(SchemaV2Error::new(
                        "outcome_provider_capability_projection_mismatch",
                        "outcome evidence provider must equal the requested provider capability",
                    ));
                }
                if provider_evidence.evidence_kind != ProviderEvidenceKind::OutcomeDailyBars
                    || self.provider.as_deref() != Some(&provider_evidence.provider)
                    || self.source != provider_evidence.source
                    || self.source_at != provider_evidence.source_at
                    || self.observed_at != provider_evidence.observed_at
                    || self.batch_id != provider_evidence.batch_id
                    || self.batch_content_hash != provider_evidence.batch_content_hash
                {
                    return Err(SchemaV2Error::new(
                        "outcome_evidence_projection_mismatch",
                        "outcome provider fields must equal the typed evidence preimage",
                    ));
                }
                Some(evidence)
            }
            (None, None) => None,
            _ => unreachable!("require_pair rejected mismatched option state"),
        };
        let error_detail = match (
            self.error_detail_json.as_deref(),
            self.error_detail_hash.as_deref(),
        ) {
            (Some(json), Some(hash)) => {
                let detail =
                    validate_canonical_json_hash::<ProviderErrorDetailPreimage>(json, hash)?;
                detail.validate()?;
                if request_capability
                    .as_ref()
                    .is_none_or(|capability| detail.provider != capability.provider)
                {
                    return Err(SchemaV2Error::new(
                        "outcome_error_provider_capability_projection_mismatch",
                        "outcome error provider must equal the requested provider capability",
                    ));
                }
                Some(detail)
            }
            (None, None) => None,
            _ => unreachable!("require_pair rejected mismatched option state"),
        };
        if let Some(evidence) = available_evidence.as_ref() {
            validate_outcome_available_evidence_transport_projection(
                evidence,
                transport_attempts.as_ref().ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_available_transport_attempts_missing",
                        "provider evidence cannot exist without typed transport attempts",
                    )
                })?,
            )?;
        }
        let no_provider_fields = self.provider.is_none()
            && self.source.is_none()
            && self.source_at.is_none()
            && self.observed_at.is_none()
            && self.batch_id.is_none()
            && self.batch_content_hash.is_none()
            && self.available_evidence_json.is_none()
            && self.available_evidence_hash.is_none();
        let no_error_fields = self.error_detail_json.is_none()
            && self.error_detail_hash.is_none()
            && self.error_fingerprint.is_none();
        match self.result_code {
            OutcomeAttemptResult::Settled => {
                if let Some(evidence) = &available_evidence {
                    let request = request_evidence.as_ref().expect("settled request required");
                    evidence.validate_complete(
                        request_parameters
                            .as_ref()
                            .expect("settled parameters required"),
                        &request.request_hash,
                    )?;
                }
                if self.reason_code.is_some()
                    || self.retryable.is_some()
                    || request_evidence.is_none()
                    || transport_attempts.is_none()
                    || transport_attempts
                        .as_ref()
                        .is_some_and(|attempts| attempts.selected_transport_result_hash.is_none())
                    || available_evidence.is_none()
                    || !no_error_fields
                    || self.settled_outcome_content_hash.is_none()
                {
                    return Err(SchemaV2Error::new(
                        "invalid_settled_outcome_matrix",
                        "settled requires complete evidence/hash and NULL reason/retry/error",
                    ));
                }
            }
            OutcomeAttemptResult::ExpectedWait => {
                if self.reason_code != Some(OutcomeReasonCodeV1::MarketSessionUnsettled)
                    || self.retryable.is_some()
                    || request_evidence.is_some()
                    || transport_attempts.is_some()
                    || !no_provider_fields
                    || !no_error_fields
                    || self.settled_outcome_content_hash.is_some()
                {
                    return Err(SchemaV2Error::new(
                        "invalid_expected_wait_outcome_matrix",
                        "ExpectedWait only permits market_session_unsettled and no provider/error",
                    ));
                }
            }
            OutcomeAttemptResult::Error => {
                if self.reason_code.is_none()
                    || self.reason_code == Some(OutcomeReasonCodeV1::MarketSessionUnsettled)
                    || self.retryable.is_none()
                    || request_evidence.is_none()
                    || transport_attempts.is_none()
                    || error_detail.is_none()
                    || self.error_fingerprint.is_none()
                    || self.settled_outcome_content_hash.is_some()
                {
                    return Err(SchemaV2Error::new(
                        "invalid_error_outcome_matrix",
                        "error requires typed reason/retryability/detail/fingerprint and no outcome",
                    ));
                }
                if available_evidence.is_none() && !no_provider_fields {
                    return Err(SchemaV2Error::new(
                        "outcome_partial_evidence_missing",
                        "provider evidence columns require their typed JSON/hash preimage",
                    ));
                }
                if available_evidence.is_none()
                    && transport_attempts
                        .as_ref()
                        .is_some_and(|attempts| attempts.selected_transport_result_hash.is_some())
                {
                    return Err(SchemaV2Error::new(
                        "outcome_error_selected_result_without_evidence",
                        "an error may select a retained success only when its evidence is persisted",
                    ));
                }
                if self.reason_code == Some(OutcomeReasonCodeV1::SettledBarMissing)
                    && available_evidence.is_none()
                {
                    return Err(SchemaV2Error::new(
                        "settled_bar_missing_evidence_required",
                        "settled_bar_missing requires the real partial evidence obtained by the read",
                    ));
                }
                let detail = error_detail.as_ref().expect("checked above");
                let fingerprint = OutcomeErrorFingerprintPreimageV2 {
                    domain: DOMAIN_ERROR_FINGERPRINT.into(),
                    failed_stage: detail.operation.clone(),
                    reason_code: self.reason_code.expect("checked above").as_str().into(),
                    retryable: self.retryable.expect("checked above"),
                    available_evidence_hash: self.available_evidence_hash.clone(),
                    detail_hash: self.error_detail_hash.clone().expect("checked above"),
                    transport_attempts_hash: self
                        .transport_attempts_hash
                        .clone()
                        .expect("error transport attempts required"),
                };
                if sha256_json(&fingerprint)? != *self.error_fingerprint.as_ref().expect("checked")
                {
                    return Err(SchemaV2Error::new(
                        "outcome_error_fingerprint_mismatch",
                        "error fingerprint must bind stage/reason/retry/evidence/detail/transport",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_outcome_available_evidence_transport_projection(
    available: &OutcomeProviderAvailableEvidencePreimage,
    attempts: &OutcomeTransportAttemptsPreimage,
) -> Result<(), SchemaV2Error> {
    let selected = attempts.selected_attempt().ok_or_else(|| {
        SchemaV2Error::new(
            "outcome_available_selected_result_missing",
            "available evidence requires one selected successful transport result",
        )
    })?;
    let selected_evidence = selected.result.provider_evidence.as_ref().ok_or_else(|| {
        SchemaV2Error::new(
            "outcome_available_selected_evidence_missing",
            "selected successful transport result has no provider evidence",
        )
    })?;
    let provider = &available.provider_evidence;
    if provider.provider != selected_evidence.batch_content.provider
        || provider.source.as_deref() != Some(selected_evidence.source.as_str())
        || provider.source_at != selected_evidence.source_at
        || provider.observed_at.as_deref() != Some(selected_evidence.observed_at.as_str())
        || provider.batch_id.as_deref() != Some(selected_evidence.batch_id.as_str())
    {
        return Err(SchemaV2Error::new(
            "outcome_available_transport_projection_mismatch",
            "available evidence provider/source/time/batch must equal the selected success",
        ));
    }
    let selected_dates = selected_evidence
        .batch_content
        .records
        .iter()
        .map(|record| record.market_date.as_str())
        .collect::<Vec<_>>();
    let mut cursor = 0_usize;
    for returned in &available.returned_trading_dates {
        let Some(relative) = selected_dates[cursor..]
            .iter()
            .position(|candidate| *candidate == returned)
        else {
            return Err(SchemaV2Error::new(
                "outcome_available_transport_subset_mismatch",
                "available trading dates must be an exact ordered subset of selected transport records",
            ));
        };
        cursor += relative + 1;
    }
    Ok(())
}

canonical_struct!(SelectionRecoveryEnvelopeRowContentPreimage {
    domain: String,
    stage_run_id: String,
    subject_kind: SubjectKind,
    logical_subject_key: String,
    payload_schema: String,
    payload_json: String,
    payload_json_hash: String,
    in_memory_payload_hash: String,
    config_activation_run_id: String,
    config_hash: String,
    enveloped_at: String,
});

canonical_struct!(LegacyCutoverTableWatermarkPreimage {
    table_name: String,
    max_rowid: i64,
    row_count: u64,
});

canonical_struct!(LegacyTriggerDefinitionPreimage {
    trigger_name: String,
    target_table: String,
    operation: LegacyTriggerOperation,
    canonical_sql: String,
});

canonical_struct!(LegacyTriggerSetPreimage {
    domain: String,
    triggers_sorted: Vec<LegacyTriggerDefinitionPreimage>,
});

canonical_struct!(LegacyCutoverSnapshotPreimage {
    domain: String,
    captured_at_rfc3339_nanos_utc: String,
    tables_sorted: Vec<LegacyCutoverTableWatermarkPreimage>,
    pending_inbox_count: u64,
    committed_legacy_candidate_count: u64,
    legacy_outcome_row_count: u64,
    frozen_graph_trigger_set_hash: String,
});

canonical_struct!(ConfigActivationStageInputPreimage {
    domain: String,
    stage_run_id: String,
    logical_subject_key: String,
    config_snapshot: SelectionConfigSnapshotPreimage,
    config_snapshot_json_hash: String,
    config_hash: String,
    activation: ConfigActivationContentPreimage,
    activation_content_hash: String,
    legacy_cutover_snapshot: LegacyCutoverSnapshotPreimage,
    legacy_cutover_snapshot_hash: String,
});

canonical_struct!(SourceIngressStageInputPreimage {
    domain: String,
    stage_run_id: String,
    logical_subject_key: String,
    config_activation_run_id: String,
    config_hash: String,
    generation_market_date: String,
    aggregator_observed_at_rfc3339_nanos_utc: String,
    source_batch_content_hash: String,
    registered_feed_snapshot_json: String,
    registered_feed_snapshot_hash: String,
    source_batch_attempt_rows: Vec<SelectionSourceBatchAttemptRowContentPreimage>,
    source_fact_rows: Vec<SelectionSourceFactRowContentPreimage>,
    source_fact_attempt_rows: Vec<SelectionSourceFactAttemptRowContentPreimage>,
    planned_run_status: RunStatus,
});

impl ConfigActivationStageInputPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_CONFIG_ACTIVATION_STAGE)?;
        require_canonical_uuid_v7(&self.stage_run_id, "stage_run_id")?;
        require_hash(&self.logical_subject_key, "logical_subject_key")?;
        require_hash(&self.config_snapshot_json_hash, "config_snapshot_json_hash")?;
        require_hash(&self.config_hash, "config_hash")?;
        validate_stage_logical_subject_key(
            &self.logical_subject_key,
            RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::ConfigActivation,
                source_fact_key: None,
                config_hash: Some(self.config_hash.clone()),
                sample_key: None,
                outcome_phase: None,
                stored_due_date: None,
                ingress_source_batch_hash: None,
            },
        )?;
        require_hash(&self.activation_content_hash, "activation_content_hash")?;
        require_hash(
            &self.legacy_cutover_snapshot_hash,
            "legacy_cutover_snapshot_hash",
        )?;
        if sha256_json(&self.config_snapshot)? != self.config_snapshot_json_hash
            || sha256_json(&self.activation)? != self.activation_content_hash
            || sha256_json(&self.legacy_cutover_snapshot)? != self.legacy_cutover_snapshot_hash
        {
            return Err(SchemaV2Error::new(
                "config_activation_stage_nested_hash_mismatch",
                "config stage nested typed content hashes must match the stored projections",
            ));
        }
        Ok(())
    }
}

impl SourceIngressStageInputPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_SOURCE_INGRESS_STAGE)?;
        require_canonical_uuid_v7(&self.stage_run_id, "stage_run_id")?;
        require_hash(&self.logical_subject_key, "logical_subject_key")?;
        require_canonical_uuid_v7(&self.config_activation_run_id, "config_activation_run_id")?;
        require_hash(&self.config_hash, "config_hash")?;
        parse_canonical_date(&self.generation_market_date, "generation_market_date")?;
        parse_canonical_nanos_utc(
            &self.aggregator_observed_at_rfc3339_nanos_utc,
            "aggregator_observed_at_rfc3339_nanos_utc",
        )?;
        require_hash(&self.source_batch_content_hash, "source_batch_content_hash")?;
        validate_stage_logical_subject_key(
            &self.logical_subject_key,
            RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::IngressRun,
                source_fact_key: None,
                config_hash: Some(self.config_hash.clone()),
                sample_key: None,
                outcome_phase: None,
                stored_due_date: None,
                ingress_source_batch_hash: Some(self.source_batch_content_hash.clone()),
            },
        )?;
        let snapshot = validate_canonical_json_hash::<RegisteredFeedSnapshotPreimage>(
            &self.registered_feed_snapshot_json,
            &self.registered_feed_snapshot_hash,
        )?;
        require_domain(&snapshot.domain, DOMAIN_REGISTERED_FEED_SNAPSHOT)?;
        if snapshot.feeds_sorted != production_registered_global_news_entries()? {
            return Err(SchemaV2Error::new(
                "registered_feed_registry_mismatch",
                "ingress snapshot must equal the exact four checked-in production registrations",
            ));
        }
        if snapshot.feeds_sorted.len() != self.source_batch_attempt_rows.len()
            || snapshot
                .feeds_sorted
                .iter()
                .enumerate()
                .any(|(ordinal, entry)| usize::try_from(entry.ordinal).ok() != Some(ordinal))
            || snapshot
                .feeds_sorted
                .windows(2)
                .any(|entries| entries[0].feed_identity >= entries[1].feed_identity)
        {
            return Err(SchemaV2Error::new(
                "registered_feed_snapshot_matrix_mismatch",
                "snapshot must be strictly identity-sorted with contiguous ordinals and one attempt per feed",
            ));
        }
        if self
            .source_batch_attempt_rows
            .windows(2)
            .any(|rows| rows[0].source_batch_attempt_id >= rows[1].source_batch_attempt_id)
        {
            return Err(SchemaV2Error::new(
                "source_attempt_rows_not_sorted_unique",
                "source attempt rows must be strictly sorted by attempt identity",
            ));
        }
        for row in &self.source_batch_attempt_rows {
            row.validate()?;
            let entry = snapshot
                .feeds_sorted
                .iter()
                .find(|entry| entry.feed_identity == row.registered_feed_identity)
                .ok_or_else(|| {
                    SchemaV2Error::new(
                        "source_attempt_feed_unregistered",
                        "source attempt feed is absent from the pinned registered snapshot",
                    )
                })?;
            let request = validate_request_evidence_columns(
                Some(&row.request_hash),
                Some(&row.request_evidence_json),
                Some(&row.request_evidence_hash),
                Some(RequestKind::GlobalNews),
            )?
            .expect("source request tuple is required");
            let capability = validate_canonical_json_hash::<ProviderCapabilityHashPreimage>(
                &request.provider_capability_json,
                &request.provider_capability_hash,
            )?;
            if row.ingress_run_id != self.stage_run_id
                || row.config_activation_run_id != self.config_activation_run_id
                || row.config_hash != self.config_hash
                || row.generation_market_date != self.generation_market_date
                || row.registered_feed_snapshot_hash != self.registered_feed_snapshot_hash
                || entry.gateway_provider != capability.provider
                || entry.capability_name != capability.capability_name
            {
                return Err(SchemaV2Error::new(
                    "source_attempt_stage_identity_mismatch",
                    "source attempt row must match stage run/config/date/feed snapshot lineage",
                ));
            }
        }
        if self
            .source_fact_rows
            .windows(2)
            .any(|rows| rows[0].source_fact_key >= rows[1].source_fact_key)
        {
            return Err(SchemaV2Error::new(
                "source_fact_rows_not_sorted_unique",
                "source fact rows must be strictly sorted by source_fact_key",
            ));
        }
        let mut validated_facts = Vec::with_capacity(self.source_fact_rows.len());
        for row in &self.source_fact_rows {
            let content = row.validate()?;
            if row.first_ingress_run_id != self.stage_run_id
                || row.config_activation_run_id != self.config_activation_run_id
                || row.config_hash != self.config_hash
                || row.generation_market_date != self.generation_market_date
            {
                return Err(SchemaV2Error::new(
                    "source_fact_stage_identity_mismatch",
                    "source fact row must match stage run/config/date lineage",
                ));
            }
            validated_facts.push((row, content));
        }
        if self
            .source_fact_attempt_rows
            .windows(2)
            .any(|rows| rows[0].source_fact_attempt_id >= rows[1].source_fact_attempt_id)
        {
            return Err(SchemaV2Error::new(
                "source_fact_attempt_rows_not_sorted_unique",
                "source fact attempt rows must be strictly sorted by attempt identity",
            ));
        }
        let mut validated_attempts = Vec::with_capacity(self.source_fact_attempt_rows.len());
        for row in &self.source_fact_attempt_rows {
            let (acquired, evidence) = row.validate()?;
            if row.ingress_run_id != self.stage_run_id {
                return Err(SchemaV2Error::new(
                    "source_fact_attempt_stage_identity_mismatch",
                    "source fact attempt must match the ingress stage run",
                ));
            }
            let feed_row = self
                .source_batch_attempt_rows
                .iter()
                .find(|feed| feed.source_batch_attempt_id == row.source_batch_attempt_id)
                .ok_or_else(|| {
                    SchemaV2Error::new(
                        "source_fact_attempt_feed_missing",
                        "source fact attempt must reference a staged feed attempt",
                    )
                })?;
            if feed_row.status_kind != FeedStatusKind::Available
                || evidence.feed_identity != feed_row.registered_feed_identity
                || Some(&row.batch_evidence_hash) != feed_row.available_evidence_hash.as_ref()
                || acquired.record_batch_content_hash.as_str()
                    != feed_row
                        .batch_content_hash
                        .as_deref()
                        .expect("available feed has a content hash")
            {
                return Err(SchemaV2Error::new(
                    "source_fact_attempt_feed_projection_mismatch",
                    "source fact attempt evidence must equal its available feed batch",
                ));
            }
            if row.attempt_result == SourceFactAttemptResult::Accepted {
                let (fact, content) = validated_facts
                    .iter()
                    .find(|(fact, _)| fact.source_fact_key == row.source_fact_key)
                    .ok_or_else(|| {
                        SchemaV2Error::new(
                            "accepted_source_fact_missing",
                            "accepted source attempt must stage its authoritative source fact",
                        )
                    })?;
                if content != &acquired.record
                    || fact.provider_content_hash != acquired.provider_content_hash
                    || fact.event_id != row.event_projection_id
                    || fact.record_provider != acquired.record_provider
                    || fact.record_observed_at != acquired.record_observed_at
                    || fact.record_batch_id != acquired.record_batch_id
                    || fact.record_batch_content_hash != acquired.record_batch_content_hash
                {
                    return Err(SchemaV2Error::new(
                        "accepted_source_fact_projection_mismatch",
                        "accepted attempt and staged authoritative source fact must be identical",
                    ));
                }
            }
            if row.attempt_result == SourceFactAttemptResult::Conflict {
                if let Some((fact, _)) = validated_facts
                    .iter()
                    .find(|(fact, _)| fact.source_fact_key == row.source_fact_key)
                {
                    let conflict = SourceFactConflictPreimage {
                        domain: DOMAIN_SOURCE_FACT_CONFLICT.into(),
                        source_fact_key: row.source_fact_key.clone(),
                        authoritative_provider_content_hash: fact.provider_content_hash.clone(),
                        attempted_provider_content_hash: acquired.provider_content_hash.clone(),
                    };
                    if sha256_json(&conflict)?.as_str()
                        != row
                            .conflict_hash
                            .as_deref()
                            .expect("validated conflict hash")
                    {
                        return Err(SchemaV2Error::new(
                            "source_fact_conflict_hash_mismatch",
                            "conflict hash must bind authoritative and attempted source content",
                        ));
                    }
                }
            }
            validated_attempts.push((row, acquired, evidence));
        }
        if validated_facts.iter().any(|(fact, _)| {
            !validated_attempts.iter().any(|(attempt, _, _)| {
                attempt.attempt_result == SourceFactAttemptResult::Accepted
                    && attempt.source_fact_key == fact.source_fact_key
            })
        }) {
            return Err(SchemaV2Error::new(
                "source_fact_without_accepted_attempt",
                "every newly staged source fact requires one accepted acquisition attempt",
            ));
        }
        let mut feed_attempt_hashes = Vec::with_capacity(snapshot.feeds_sorted.len());
        let mut source_record_hashes = Vec::new();
        let mut event_projection_ids = Vec::new();
        for entry in &snapshot.feeds_sorted {
            let feed_row = self
                .source_batch_attempt_rows
                .iter()
                .find(|row| row.registered_feed_identity == entry.feed_identity)
                .expect("snapshot/feed cardinality and membership validated");
            feed_attempt_hashes.push(feed_row.feed_attempt_content_hash.clone());
            let mut children = validated_attempts
                .iter()
                .filter(|(attempt, _, _)| {
                    attempt.source_batch_attempt_id == feed_row.source_batch_attempt_id
                })
                .collect::<Vec<_>>();
            children.sort_by_key(|(attempt, _, _)| attempt.provider_ordinal);
            if children
                .iter()
                .enumerate()
                .any(|(ordinal, (attempt, _, _))| attempt.provider_ordinal != ordinal as u32)
            {
                return Err(SchemaV2Error::new(
                    "source_fact_attempt_provider_order_invalid",
                    "per-feed provider ordinals must be contiguous from zero",
                ));
            }
            let expected_count = match feed_row.status_kind {
                FeedStatusKind::Available => feed_row.record_count,
                FeedStatusKind::VerifiedEmpty | FeedStatusKind::Unavailable => Some(0),
            };
            if expected_count != Some(children.len() as u32) {
                return Err(SchemaV2Error::new(
                    "source_fact_attempt_count_mismatch",
                    "per-feed child attempt count must equal its terminal feed matrix",
                ));
            }
            if matches!(
                feed_row.status_kind,
                FeedStatusKind::Available | FeedStatusKind::VerifiedEmpty
            ) {
                let records = children
                    .iter()
                    .map(|(attempt, acquired, _)| FeedSourceRecordHashPreimage {
                        domain: DOMAIN_FEED_SOURCE_RECORD.into(),
                        provider_ordinal: attempt.provider_ordinal,
                        source_fact_key: attempt.source_fact_key.clone(),
                        provider_content_hash: acquired.provider_content_hash.clone(),
                    })
                    .collect::<Vec<_>>();
                let evidence_hash = feed_row
                    .available_evidence_hash
                    .as_ref()
                    .expect("success feed has evidence");
                let expected_content =
                    feed_source_content_hash(&entry.feed_identity, evidence_hash, &records)?;
                if feed_row.batch_content_hash.as_deref() != Some(&expected_content) {
                    return Err(SchemaV2Error::new(
                        "feed_source_content_hash_mismatch",
                        "feed batch content must bind every child in provider order",
                    ));
                }
                for ((attempt, _, _), record) in children.iter().zip(records.iter()) {
                    source_record_hashes.push(sha256_json(record)?);
                    event_projection_ids.push(attempt.event_projection_id.clone());
                }
            }
        }
        let expected_source_batch_hash = source_batch_content_hash(
            &self.registered_feed_snapshot_hash,
            feed_attempt_hashes,
            source_record_hashes,
            event_projection_ids,
            self.aggregator_observed_at_rfc3339_nanos_utc.clone(),
        )?;
        if expected_source_batch_hash != self.source_batch_content_hash {
            return Err(SchemaV2Error::new(
                "source_batch_content_hash_mismatch",
                "source batch hash must bind the exact registry/feed/record/projection order",
            ));
        }
        if !matches!(
            self.planned_run_status,
            RunStatus::Completed | RunStatus::FailedNonRetryable
        ) {
            return Err(SchemaV2Error::new(
                "source_ingress_status_invalid",
                "source ingress permits only completed/failed_non_retryable",
            ));
        }
        Ok(())
    }
}

canonical_struct!(GenerationStageInputPreimage {
    domain: String,
    stage_run_id: String,
    logical_subject_key: String,
    source_fact_key: String,
    source_fact_content_hash: String,
    config_activation_run_id: String,
    config_hash: String,
    generation_market_date: String,
    relation_attempt_rows: Vec<SelectionRelationAttemptRowContentPreimage>,
    evaluation_attempt_rows: Vec<SelectionEvaluationAttemptRowContentPreimage>,
    sample_rows: Vec<SelectionSampleRowContentPreimage>,
    rejection_rows: Vec<SelectionRejectionRowContentPreimage>,
    planned_run_status: RunStatus,
});

impl GenerationStageInputPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_GENERATION_STAGE)?;
        if !matches!(
            self.planned_run_status,
            RunStatus::Completed
                | RunStatus::VerifiedNoRelation
                | RunStatus::PendingDependency
                | RunStatus::FailedNonRetryable
        ) {
            return Err(SchemaV2Error::new(
                "generation_status_invalid",
                "generation only permits completed/verified_no_relation/pending_dependency/failed_non_retryable",
            ));
        }
        require_canonical_uuid_v7(&self.stage_run_id, "stage_run_id")?;
        require_hash(&self.logical_subject_key, "logical_subject_key")?;
        require_hash(&self.source_fact_key, "source_fact_key")?;
        require_hash(&self.source_fact_content_hash, "source_fact_content_hash")?;
        require_canonical_uuid_v7(&self.config_activation_run_id, "config_activation_run_id")?;
        require_hash(&self.config_hash, "config_hash")?;
        validate_stage_logical_subject_key(
            &self.logical_subject_key,
            RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::GenerationRun,
                source_fact_key: Some(self.source_fact_key.clone()),
                config_hash: Some(self.config_hash.clone()),
                sample_key: None,
                outcome_phase: None,
                stored_due_date: None,
                ingress_source_batch_hash: None,
            },
        )?;
        parse_canonical_date(&self.generation_market_date, "generation_market_date")?;
        self.validate_sorted_unique_rows()?;

        for row in &self.relation_attempt_rows {
            self.validate_relation_row(row)?;
        }
        for row in &self.evaluation_attempt_rows {
            self.validate_evaluation_row(row)?;
        }
        for row in &self.sample_rows {
            self.validate_sample_row(row)?;
        }
        for row in &self.rejection_rows {
            self.validate_rejection_row(row)?;
        }
        self.validate_terminal_matrix()?;
        self.validate_status_matrix()
    }

    fn validate_sorted_unique_rows(&self) -> Result<(), SchemaV2Error> {
        if self
            .relation_attempt_rows
            .windows(2)
            .any(|rows| rows[0].relation_attempt_id >= rows[1].relation_attempt_id)
        {
            return Err(SchemaV2Error::new(
                "generation_relation_rows_not_sorted_unique",
                "relation rows must be strictly sorted by relation_attempt_id",
            ));
        }
        if self
            .evaluation_attempt_rows
            .windows(2)
            .any(|rows| rows[0].evaluation_attempt_id >= rows[1].evaluation_attempt_id)
        {
            return Err(SchemaV2Error::new(
                "generation_evaluation_rows_not_sorted_unique",
                "evaluation rows must be strictly sorted by evaluation_attempt_id",
            ));
        }
        if self
            .sample_rows
            .windows(2)
            .any(|rows| rows[0].sample_key >= rows[1].sample_key)
        {
            return Err(SchemaV2Error::new(
                "generation_sample_rows_not_sorted_unique",
                "sample rows must be strictly sorted by sample_key",
            ));
        }
        if self.rejection_rows.windows(2).any(|rows| {
            (&rows[0].sample_key, rows[0].ordinal) >= (&rows[1].sample_key, rows[1].ordinal)
        }) {
            return Err(SchemaV2Error::new(
                "generation_rejection_rows_not_sorted_unique",
                "rejection rows must be strictly sorted by (sample_key, ordinal)",
            ));
        }
        Ok(())
    }

    fn validate_relation_row(
        &self,
        row: &SelectionRelationAttemptRowContentPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&row.domain, DOMAIN_RELATION_ATTEMPT_ROW)?;
        if row.generation_run_id != self.stage_run_id
            || row.source_fact_key != self.source_fact_key
            || row.config_activation_run_id != self.config_activation_run_id
            || row.config_hash != self.config_hash
        {
            return Err(SchemaV2Error::new(
                "generation_relation_identity_mismatch",
                "relation row must match generation run/source/config lineage",
            ));
        }
        require_hash(&row.relation_attempt_id, "relation_attempt_id")?;
        require_hash(&row.relation_key, "relation_key")?;
        let request_evidence = validate_request_evidence_columns(
            row.request_hash.as_deref(),
            row.request_evidence_json.as_deref(),
            row.request_evidence_hash.as_deref(),
            Some(RequestKind::BoardConstituents),
        )?;
        let binding = validate_canonical_json_hash::<BindingStatePreimage>(
            &row.typed_binding_state_json,
            &row.typed_binding_state_hash,
        )?;
        binding.validate()?;
        require_pair(
            &row.raw_identity_json,
            &row.raw_identity_hash,
            "relation_raw_identity_pair_mismatch",
        )?;
        require_pair(
            &row.available_evidence_json,
            &row.available_evidence_hash,
            "relation_available_evidence_pair_mismatch",
        )?;
        require_pair(
            &row.error_detail_json,
            &row.error_detail_hash,
            "relation_error_detail_pair_mismatch",
        )?;
        let available_evidence = if let (Some(json), Some(hash)) = (
            row.available_evidence_json.as_deref(),
            row.available_evidence_hash.as_deref(),
        ) {
            let evidence =
                validate_canonical_json_hash::<ProviderAvailableEvidencePreimage>(json, hash)?;
            evidence.validate_partial()?;
            if evidence.evidence_kind != ProviderEvidenceKind::BoardConstituents
                || row.provider_source != evidence.source
                || row.provider_source_at != evidence.source_at
                || row.provider_observed_at != evidence.observed_at
                || row.provider_batch_id != evidence.batch_id
                || row.provider_batch_content_hash != evidence.batch_content_hash
            {
                return Err(SchemaV2Error::new(
                    "relation_evidence_projection_mismatch",
                    "relation provider fields must equal typed board evidence",
                ));
            }
            Some(evidence)
        } else {
            None
        };
        let error_detail = if let (Some(json), Some(hash)) = (
            row.error_detail_json.as_deref(),
            row.error_detail_hash.as_deref(),
        ) {
            let detail = validate_canonical_json_hash::<ProviderErrorDetailPreimage>(json, hash)?;
            detail.validate()?;
            Some(detail)
        } else {
            None
        };
        if let Some(fingerprint) = &row.error_fingerprint {
            require_hash(fingerprint, "relation_error_fingerprint")?;
        }

        match row.relation_kind {
            RelationKind::DirectMention => {
                let source = validate_canonical_json_hash::<DirectMentionSourcePreimage>(
                    &row.relation_source_identity_json,
                    &row.relation_source_identity_hash,
                )?;
                require_domain(&source.domain, DOMAIN_DIRECT_SOURCE)?;
                if source.source_fact_key != self.source_fact_key
                    || binding.state != BindingStateKind::DirectNotApplicable
                    || request_evidence.is_some()
                    || row.artifact_content_hash.is_some()
                    || row.binding_audit_hash.is_some()
                    || row.provider_board_kind.is_some()
                    || row.provider_board_code.is_some()
                    || row.provider_board_name.is_some()
                    || row.actual_constituent_count.is_some()
                {
                    return Err(SchemaV2Error::new(
                        "direct_relation_kind_matrix_mismatch",
                        "direct mention requires direct source, direct binding state, and no board/request fields",
                    ));
                }
            }
            RelationKind::ProviderBoardConstituent => match binding.state {
                BindingStateKind::NotConfigured => {
                    let source = validate_canonical_json_hash::<BoardNotConfiguredSourcePreimage>(
                        &row.relation_source_identity_json,
                        &row.relation_source_identity_hash,
                    )?;
                    require_domain(&source.domain, DOMAIN_BOARD_SOURCE_NOT_CONFIGURED)?;
                    if source.source_fact_key != self.source_fact_key
                        || source.chain_id != row.chain_id
                        || source.config_hash != self.config_hash
                        || request_evidence.is_some()
                        || !matches!(row.result_code.as_str(), "rejected" | "unsupported")
                        || row.artifact_content_hash.is_some()
                        || row.binding_audit_hash.is_some()
                        || row.provider_board_kind.is_some()
                        || row.provider_board_code.is_some()
                        || row.provider_board_name.is_some()
                    {
                        return Err(SchemaV2Error::new(
                            "board_not_configured_kind_matrix_mismatch",
                            "not-configured board relation must contain only the typed missing binding",
                        ));
                    }
                }
                BindingStateKind::Verified => {
                    let source = validate_canonical_json_hash::<BoardRelationSourcePreimage>(
                        &row.relation_source_identity_json,
                        &row.relation_source_identity_hash,
                    )?;
                    require_domain(&source.domain, DOMAIN_BOARD_SOURCE)?;
                    if row.artifact_content_hash.as_deref() != Some(&source.artifact_content_hash)
                        || row.binding_audit_hash.as_deref() != Some(&source.binding_audit_hash)
                        || row.provider_board_kind != Some(source.kind)
                        || row.provider_board_code.as_deref() != Some(&source.code)
                        || row.provider_board_name.as_deref() != Some(&source.name)
                        || binding.artifact_content_hash != row.artifact_content_hash
                        || binding.binding_audit_hash != row.binding_audit_hash
                        || binding.provider.as_deref() != Some(&source.provider)
                        || binding.kind != Some(source.kind)
                        || binding.code.as_deref() != Some(&source.code)
                        || binding.name.as_deref() != Some(&source.name)
                        || request_evidence.is_none()
                    {
                        return Err(SchemaV2Error::new(
                            "verified_board_kind_matrix_mismatch",
                            "verified board source, binding state, and projected columns must agree",
                        ));
                    }
                    let request = request_evidence
                        .as_ref()
                        .expect("verified board binding requires request evidence");
                    let parameters = validate_canonical_json_hash::<
                        BoardConstituentRequestParametersPreimage,
                    >(
                        &request.parameters_json, &request.parameters_json_hash
                    )?;
                    if parameters.artifact_content_hash != source.artifact_content_hash
                        || parameters.binding_audit_hash != source.binding_audit_hash
                        || parameters.provider != source.provider
                        || parameters.kind != source.kind
                        || parameters.code != source.code
                        || parameters.name != source.name
                    {
                        return Err(SchemaV2Error::new(
                            "board_request_projection_mismatch",
                            "typed board request parameters must equal the verified relation binding",
                        ));
                    }
                    if row.result_code == "resolved"
                        && (row.provider_source.is_none()
                            || row.provider_observed_at.is_none()
                            || row.provider_batch_id.is_none()
                            || row.provider_batch_content_hash.is_none()
                            || row.actual_constituent_count.is_none_or(|count| count == 0))
                    {
                        return Err(SchemaV2Error::new(
                            "resolved_board_evidence_incomplete",
                            "resolved board relation requires request and complete non-empty provider batch",
                        ));
                    }
                    if row.result_code == "resolved" {
                        available_evidence
                            .as_ref()
                            .expect("resolved board requires typed evidence")
                            .validate_complete()?;
                    }
                }
                BindingStateKind::DirectNotApplicable => {
                    return Err(SchemaV2Error::new(
                        "board_relation_binding_state_mismatch",
                        "provider board relation cannot use direct_not_applicable binding",
                    ));
                }
            },
        }

        match row.result_code.as_str() {
            "resolved" => {
                let raw = match (
                    row.raw_identity_json.as_deref(),
                    row.raw_identity_hash.as_deref(),
                ) {
                    (Some(json), Some(hash)) => Some(validate_canonical_json_hash::<
                        RawSecurityIdentityPreimage,
                    >(json, hash)?),
                    _ => None,
                };
                if row.failed_stage.is_some()
                    || row.retryable.is_some()
                    || raw.is_none()
                    || row.canonical_stock_code.is_none()
                    || row.canonical_stock_name.is_none()
                    || row.canonical_market.is_none()
                    || row.error_detail_json.is_some()
                    || row.error_fingerprint.is_some()
                {
                    return Err(SchemaV2Error::new(
                        "resolved_relation_result_matrix_mismatch",
                        "resolved relation requires canonical identity/evidence and NULL failure fields",
                    ));
                }
                let raw = raw.expect("checked above");
                match row.relation_kind {
                    RelationKind::DirectMention if available_evidence.is_none() => {}
                    RelationKind::ProviderBoardConstituent if available_evidence.is_some() => {}
                    _ => {
                        return Err(SchemaV2Error::new(
                            "resolved_relation_evidence_matrix_mismatch",
                            "direct relation has no provider batch; resolved board relation requires typed provider evidence",
                        ));
                    }
                }
                require_domain(&raw.domain, DOMAIN_RAW_SECURITY_IDENTITY)?;
                if row.canonical_stock_code.as_deref() != Some(&raw.code)
                    || row.canonical_market.as_deref() != Some(&raw.exchange)
                {
                    return Err(SchemaV2Error::new(
                        "relation_raw_identity_projection_mismatch",
                        "resolved canonical code/market must equal raw identity",
                    ));
                }
            }
            "rejected" | "unsupported" => {
                if row.failed_stage.as_deref().is_none_or(str::is_empty)
                    || row.retryable.is_none()
                    || error_detail.is_none()
                    || row.error_fingerprint.is_none()
                {
                    return Err(SchemaV2Error::new(
                        "failed_relation_result_matrix_mismatch",
                        "rejected/unsupported relation requires stage/retry/detail/fingerprint",
                    ));
                }
                if available_evidence.is_none()
                    && (row.provider_source.is_some()
                        || row.provider_source_at.is_some()
                        || row.provider_observed_at.is_some()
                        || row.provider_batch_id.is_some()
                        || row.provider_batch_content_hash.is_some())
                {
                    return Err(SchemaV2Error::new(
                        "relation_partial_evidence_missing",
                        "relation provider evidence columns require typed evidence JSON/hash",
                    ));
                }
                let fingerprint = ErrorFingerprintPreimage {
                    domain: DOMAIN_ERROR_FINGERPRINT.into(),
                    failed_stage: row.failed_stage.clone().expect("validated failure stage"),
                    reason_code: error_detail
                        .as_ref()
                        .expect("validated error detail")
                        .diagnostic_code
                        .clone(),
                    retryable: row.retryable.expect("validated failure retryability"),
                    available_evidence_hash: row.available_evidence_hash.clone(),
                    detail_hash: row
                        .error_detail_hash
                        .clone()
                        .expect("validated error detail hash"),
                };
                if sha256_json(&fingerprint)?
                    != *row
                        .error_fingerprint
                        .as_ref()
                        .expect("validated error fingerprint")
                {
                    return Err(SchemaV2Error::new(
                        "relation_error_fingerprint_mismatch",
                        "relation fingerprint must bind stage/diagnostic/retry/evidence/detail",
                    ));
                }
            }
            _ => {
                return Err(SchemaV2Error::new(
                    "relation_result_code_invalid",
                    "relation result must be resolved/rejected/unsupported",
                ));
            }
        }
        Ok(())
    }

    fn validate_evaluation_row(
        &self,
        row: &SelectionEvaluationAttemptRowContentPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&row.domain, DOMAIN_EVALUATION_ATTEMPT_ROW)?;
        if row.generation_run_id != self.stage_run_id || row.source_fact_key != self.source_fact_key
        {
            return Err(SchemaV2Error::new(
                "generation_evaluation_identity_mismatch",
                "evaluation row must match generation run/source",
            ));
        }
        require_hash(&row.evaluation_attempt_id, "evaluation_attempt_id")?;
        require_hash(&row.sample_key, "evaluation_sample_key")?;
        require_hash(
            &row.relation_evidence_set_hash,
            "relation_evidence_set_hash",
        )?;
        let request = validate_request_evidence_columns(
            Some(&row.market_request_hash),
            Some(&row.request_evidence_json),
            Some(&row.request_evidence_hash),
            Some(RequestKind::T0MarketEvidence),
        )?
        .expect("all required evaluation request fields were supplied");
        let parameters = validate_canonical_json_hash::<T0MarketRequestParametersPreimage>(
            &request.parameters_json,
            &request.parameters_json_hash,
        )?;
        let request_capability = validate_canonical_json_hash::<ProviderCapabilityHashPreimage>(
            &request.provider_capability_json,
            &request.provider_capability_hash,
        )?;
        if parameters.canonical_stock_code != row.canonical_stock_code
            || parameters.canonical_market != row.canonical_market
            || parameters.evaluation_market_date != self.generation_market_date
        {
            return Err(SchemaV2Error::new(
                "evaluation_request_projection_mismatch",
                "typed T0 request identity must equal evaluation row code/market/date",
            ));
        }
        require_pair(
            &row.available_evidence_json,
            &row.available_evidence_hash,
            "evaluation_available_evidence_pair_mismatch",
        )?;
        require_pair(
            &row.error_detail_json,
            &row.error_detail_hash,
            "evaluation_error_detail_pair_mismatch",
        )?;
        let available_evidence = if let (Some(json), Some(hash)) = (
            row.available_evidence_json.as_deref(),
            row.available_evidence_hash.as_deref(),
        ) {
            let evidence =
                validate_canonical_json_hash::<ProviderAvailableEvidencePreimage>(json, hash)?;
            evidence.validate_partial()?;
            if evidence.provider != request_capability.provider {
                return Err(SchemaV2Error::new(
                    "evaluation_provider_capability_projection_mismatch",
                    "evaluation evidence provider must equal the requested provider capability",
                ));
            }
            if evidence.evidence_kind != ProviderEvidenceKind::T0MarketBundle
                || row.provider.as_deref() != Some(&evidence.provider)
                || row.source != evidence.source
                || row.source_at != evidence.source_at
                || row.observed_at != evidence.observed_at
                || row.batch_id != evidence.batch_id
                || row.batch_content_hash != evidence.batch_content_hash
            {
                return Err(SchemaV2Error::new(
                    "evaluation_evidence_projection_mismatch",
                    "evaluation market fields must equal typed T0 evidence",
                ));
            }
            Some(evidence)
        } else {
            None
        };
        let error_detail = if let (Some(json), Some(hash)) = (
            row.error_detail_json.as_deref(),
            row.error_detail_hash.as_deref(),
        ) {
            let detail = validate_canonical_json_hash::<ProviderErrorDetailPreimage>(json, hash)?;
            detail.validate()?;
            if detail.provider != request_capability.provider {
                return Err(SchemaV2Error::new(
                    "evaluation_error_provider_capability_projection_mismatch",
                    "evaluation error provider must equal the requested provider capability",
                ));
            }
            Some(detail)
        } else {
            None
        };
        match row.result_code.as_str() {
            "completed"
                if row.failed_stage.is_none()
                    && row.retryable.is_none()
                    && row.provider.is_some()
                    && row.source.is_some()
                    && row.observed_at.is_some()
                    && row.batch_id.is_some()
                    && row.batch_content_hash.is_some()
                    && row.available_evidence_json.is_some()
                    && row.terminal_decision_hash.is_some()
                    && row.error_detail_json.is_none()
                    && row.error_fingerprint.is_none() =>
            {
                available_evidence
                    .as_ref()
                    .expect("completed evaluation requires typed evidence")
                    .validate_complete()?;
                require_hash(
                    row.terminal_decision_hash
                        .as_deref()
                        .expect("completed requires terminal hash"),
                    "terminal_decision_hash",
                )?;
            }
            "error"
                if row
                    .failed_stage
                    .as_deref()
                    .is_some_and(|stage| !stage.is_empty())
                    && row.retryable.is_some()
                    && row.terminal_decision_hash.is_none()
                    && error_detail.is_some()
                    && row.error_fingerprint.is_some() =>
            {
                if available_evidence.is_none()
                    && (row.provider.is_some()
                        || row.source.is_some()
                        || row.source_at.is_some()
                        || row.observed_at.is_some()
                        || row.batch_id.is_some()
                        || row.batch_content_hash.is_some())
                {
                    return Err(SchemaV2Error::new(
                        "evaluation_partial_evidence_missing",
                        "evaluation market columns require typed evidence JSON/hash",
                    ));
                }
                require_hash(
                    row.error_fingerprint
                        .as_deref()
                        .expect("error requires fingerprint"),
                    "evaluation_error_fingerprint",
                )?;
                let detail = error_detail.as_ref().expect("validated evaluation detail");
                let fingerprint = ErrorFingerprintPreimage {
                    domain: DOMAIN_ERROR_FINGERPRINT.into(),
                    failed_stage: row.failed_stage.clone().expect("validated failure stage"),
                    reason_code: detail.diagnostic_code.clone(),
                    retryable: row.retryable.expect("validated failure retryability"),
                    available_evidence_hash: row.available_evidence_hash.clone(),
                    detail_hash: row
                        .error_detail_hash
                        .clone()
                        .expect("validated detail hash"),
                };
                if sha256_json(&fingerprint)?
                    != *row
                        .error_fingerprint
                        .as_ref()
                        .expect("validated evaluation fingerprint")
                {
                    return Err(SchemaV2Error::new(
                        "evaluation_error_fingerprint_mismatch",
                        "evaluation fingerprint must bind stage/diagnostic/retry/evidence/detail",
                    ));
                }
            }
            "completed" | "error" => {
                return Err(SchemaV2Error::new(
                    "evaluation_result_matrix_mismatch",
                    "evaluation fields do not match completed/error result",
                ));
            }
            _ => {
                return Err(SchemaV2Error::new(
                    "evaluation_result_code_invalid",
                    "evaluation result must be completed/error",
                ));
            }
        }
        Ok(())
    }

    fn validate_sample_row(
        &self,
        row: &SelectionSampleRowContentPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&row.domain, DOMAIN_SAMPLE_ROW)?;
        if row.generation_run_id != self.stage_run_id
            || row.source_fact_key != self.source_fact_key
            || row.source_fact_content_hash != self.source_fact_content_hash
            || row.config_activation_run_id != self.config_activation_run_id
            || row.config_hash != self.config_hash
        {
            return Err(SchemaV2Error::new(
                "generation_sample_identity_mismatch",
                "sample row must match generation run/source/config lineage",
            ));
        }
        require_hash(&row.sample_key, "sample_key")?;
        let relation_evidence = validate_canonical_json_hash::<RelationEvidenceSetPreimage>(
            &row.relation_evidence_json,
            &row.relation_evidence_set_hash,
        )?;
        require_domain(&relation_evidence.domain, DOMAIN_RELATION_EVIDENCE_SET)?;
        if relation_evidence.source_fact_key != row.source_fact_key
            || relation_evidence.event_id != row.event_id
            || relation_evidence.chain_id != row.chain_id
            || relation_evidence.canonical_stock_code != row.canonical_stock_code
            || relation_evidence.entries_in_relation_order.is_empty()
            || relation_evidence
                .entries_in_relation_order
                .windows(2)
                .any(|entries| entries[0].relation_rank >= entries[1].relation_rank)
        {
            return Err(SchemaV2Error::new(
                "sample_relation_evidence_identity_mismatch",
                "sample relation evidence must be non-empty, ordered, and identity-equal",
            ));
        }
        for entry in &relation_evidence.entries_in_relation_order {
            let relation = self
                .relation_attempt_rows
                .iter()
                .find(|relation| relation.relation_attempt_id == entry.relation_attempt_id)
                .ok_or_else(|| {
                    SchemaV2Error::new(
                        "sample_relation_attempt_missing",
                        "relation evidence references an unstaged relation attempt",
                    )
                })?;
            if entry.relation_key != relation.relation_key
                || entry.relation_kind != relation.relation_kind
                || entry.relation_attempt_content_hash != sha256_json(relation)?
            {
                return Err(SchemaV2Error::new(
                    "sample_relation_attempt_hash_mismatch",
                    "relation evidence must bind the exact staged relation row",
                ));
            }
        }
        let feature = validate_canonical_json_hash::<T0FeaturePreimage>(
            &row.t0_feature_json,
            &row.t0_feature_hash,
        )?;
        require_domain(&feature.domain, DOMAIN_T0_FEATURE)?;
        if feature.feature_version != row.feature_version {
            return Err(SchemaV2Error::new(
                "sample_feature_version_mismatch",
                "sample feature version must equal typed feature preimage",
            ));
        }
        require_hash(&row.market_batch_content_hash, "market_batch_content_hash")?;
        require_trim_stable_non_empty(&row.calendar_version, "calendar_version")?;
        require_hash(&row.calendar_hash, "calendar_hash")?;
        let vector = validate_canonical_json_hash::<OutcomeTradingDateVectorPreimage>(
            &row.trading_date_vector_json,
            &row.trading_date_vector_hash,
        )?;
        vector.validate()?;
        if [
            row.t0_due_date.as_str(),
            row.d1_due_date.as_str(),
            row.d2_due_date.as_str(),
            row.d3_due_date.as_str(),
            row.d4_due_date.as_str(),
            row.d5_due_date.as_str(),
        ] != [
            vector.t0.as_str(),
            vector.d1.as_str(),
            vector.d2.as_str(),
            vector.d3.as_str(),
            vector.d4.as_str(),
            vector.d5.as_str(),
        ] || row.evaluation_market_date != vector.t0
        {
            return Err(SchemaV2Error::new(
                "sample_trading_date_vector_projection_mismatch",
                "sample schedule columns/evaluation date must equal the canonical full vector",
            ));
        }
        match row.decision_kind {
            TerminalDecisionKind::Admitted
                if row.rejection_count == 0
                    && row.rejection_row_hashes_in_ordinal_order.is_empty() => {}
            TerminalDecisionKind::HardRejected
                if row.rejection_count > 0
                    && row.rejection_row_hashes_in_ordinal_order.len()
                        == row.rejection_count as usize => {}
            _ => {
                return Err(SchemaV2Error::new(
                    "sample_decision_rejection_matrix_mismatch",
                    "admitted has zero rejections; hard_rejected has a non-empty exact hash list",
                ));
            }
        }
        for hash in &row.rejection_row_hashes_in_ordinal_order {
            require_hash(hash, "rejection_row_hash")?;
        }
        Ok(())
    }

    fn validate_rejection_row(
        &self,
        row: &SelectionRejectionRowContentPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&row.domain, DOMAIN_REJECTION_ROW)?;
        if row.generation_run_id != self.stage_run_id || row.retryable {
            return Err(SchemaV2Error::new(
                "generation_rejection_identity_mismatch",
                "rejection row must match generation run and be non-retryable",
            ));
        }
        let detail = validate_canonical_json_hash::<AdmissionStructuredDetailPreimage>(
            &row.structured_detail_json,
            &row.structured_detail_hash,
        )?;
        let detail_value = serde_json::to_value(detail)
            .map_err(|error| SchemaV2Error::new("typed_json_invalid", error.to_string()))?;
        if detail_value.get("kind").and_then(serde_json::Value::as_str)
            != Some(row.reason_code.as_str())
        {
            return Err(SchemaV2Error::new(
                "rejection_reason_detail_mismatch",
                "rejection reason_code must equal structured detail kind",
            ));
        }
        require_non_empty(&row.rule_id, "rejection_rule_id")
    }

    fn validate_terminal_matrix(&self) -> Result<(), SchemaV2Error> {
        for evaluation in &self.evaluation_attempt_rows {
            let samples: Vec<_> = self
                .sample_rows
                .iter()
                .filter(|sample| sample.sample_key == evaluation.sample_key)
                .collect();
            match evaluation.result_code.as_str() {
                "completed" if samples.len() == 1 => {
                    let sample = samples[0];
                    if sample.source_fact_key != evaluation.source_fact_key
                        || sample.event_id != evaluation.event_id
                        || sample.chain_id != evaluation.chain_id
                        || sample.canonical_stock_code != evaluation.canonical_stock_code
                        || sample.canonical_stock_name != evaluation.canonical_stock_name
                        || sample.canonical_market != evaluation.canonical_market
                        || sample.relation_evidence_set_hash
                            != evaluation.relation_evidence_set_hash
                        || evaluation.terminal_decision_hash.as_deref()
                            != Some(&sha256_json(sample)?)
                    {
                        return Err(SchemaV2Error::new(
                            "generation_terminal_decision_mismatch",
                            "completed evaluation must bind the exact staged sample",
                        ));
                    }
                }
                "completed" => {
                    return Err(SchemaV2Error::new(
                        "generation_completed_sample_cardinality",
                        "completed evaluation requires exactly one sample",
                    ));
                }
                "error" if samples.is_empty() => {}
                "error" => {
                    return Err(SchemaV2Error::new(
                        "generation_error_sample_present",
                        "error evaluation cannot stage a sample",
                    ));
                }
                _ => unreachable!("evaluation result validated earlier"),
            }
        }
        for sample in &self.sample_rows {
            if self
                .evaluation_attempt_rows
                .iter()
                .filter(|evaluation| {
                    evaluation.sample_key == sample.sample_key
                        && evaluation.result_code == "completed"
                })
                .count()
                != 1
            {
                return Err(SchemaV2Error::new(
                    "generation_sample_evaluation_missing",
                    "every sample requires exactly one completed evaluation",
                ));
            }
            let rejections: Vec<_> = self
                .rejection_rows
                .iter()
                .filter(|rejection| rejection.sample_key == sample.sample_key)
                .collect();
            match sample.decision_kind {
                TerminalDecisionKind::Admitted if rejections.is_empty() => {}
                TerminalDecisionKind::HardRejected
                    if rejections.len() == sample.rejection_count as usize =>
                {
                    for (ordinal, rejection) in rejections.iter().enumerate() {
                        if rejection.ordinal != ordinal as u32
                            || sample.rejection_row_hashes_in_ordinal_order[ordinal]
                                != sha256_json(rejection)?
                        {
                            return Err(SchemaV2Error::new(
                                "generation_rejection_sequence_mismatch",
                                "hard rejection ordinals and hashes must be contiguous and exact",
                            ));
                        }
                    }
                }
                _ => {
                    return Err(SchemaV2Error::new(
                        "generation_terminal_rejection_matrix_mismatch",
                        "sample decision does not match staged rejection rows",
                    ));
                }
            }
        }
        if self.rejection_rows.iter().any(|rejection| {
            !self
                .sample_rows
                .iter()
                .any(|sample| sample.sample_key == rejection.sample_key)
        }) {
            return Err(SchemaV2Error::new(
                "generation_orphan_rejection",
                "every rejection must belong to a staged sample",
            ));
        }
        Ok(())
    }

    fn validate_status_matrix(&self) -> Result<(), SchemaV2Error> {
        let retryable_failure = self
            .relation_attempt_rows
            .iter()
            .any(|row| row.result_code != "resolved" && row.retryable == Some(true))
            || self
                .evaluation_attempt_rows
                .iter()
                .any(|row| row.result_code == "error" && row.retryable == Some(true));
        let non_retryable_failure = self
            .relation_attempt_rows
            .iter()
            .any(|row| row.result_code != "resolved" && row.retryable == Some(false))
            || self
                .evaluation_attempt_rows
                .iter()
                .any(|row| row.result_code == "error" && row.retryable == Some(false));
        let valid = match self.planned_run_status {
            RunStatus::VerifiedNoRelation => {
                self.relation_attempt_rows.is_empty()
                    && self.evaluation_attempt_rows.is_empty()
                    && self.sample_rows.is_empty()
                    && self.rejection_rows.is_empty()
            }
            RunStatus::Completed => {
                !self.relation_attempt_rows.is_empty()
                    && !retryable_failure
                    && self
                        .evaluation_attempt_rows
                        .iter()
                        .all(|row| row.result_code == "completed")
            }
            RunStatus::PendingDependency => retryable_failure,
            RunStatus::FailedNonRetryable => !retryable_failure && non_retryable_failure,
            _ => unreachable!("generation status validated earlier"),
        };
        if valid {
            Ok(())
        } else {
            Err(SchemaV2Error::new(
                "generation_status_matrix_mismatch",
                "planned generation status does not match attempt terminals",
            ))
        }
    }

    pub fn expected_staged_row_count(&self) -> u32 {
        1 + self.relation_attempt_rows.len() as u32
            + self.evaluation_attempt_rows.len() as u32
            + self.sample_rows.len() as u32
            + self.rejection_rows.len() as u32
    }
}

canonical_struct!(VerifiedOutcomeDueDatabaseObjectBindingPreimage {
    domain: String,
    manifest_root_canonical_path: String,
    manifest_root_device: u64,
    manifest_root_inode: u64,
    manifest_root_mode: u32,
    database_relative_path: String,
    database_device: u64,
    database_inode: u64,
    database_mode: u32,
});

canonical_struct!(VerifiedOutcomeDueDatabaseBindingPreimage {
    domain: String,
    scope: String,
    object_binding: VerifiedOutcomeDueDatabaseObjectBindingPreimage,
    object_binding_hash: String,
    database_relative_path: String,
    sqlite_application_id: u32,
    sqlite_user_version: u32,
    sqlite_schema_hash: String,
    receipt_snapshot_high_water_rowid: i64,
    receipt_snapshot_high_water_content_hash: Option<String>,
});

canonical_struct!(VerifiedOutcomeAuditPrefixPreimage {
    domain: String,
    record_hashes_in_file_order: Vec<String>,
});

canonical_struct!(VerifiedOutcomeReceiptTuplePreimage {
    domain: String,
    receipt_role: String,
    outcome_phase: Option<OutcomePhase>,
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    run_status: RunStatus,
    committed_at_rfc3339_nanos_utc: String,
    receipt_content_hash: String,
    run_manifest_content_hash: String,
    committed_audit_record_hash: String,
});

canonical_struct!(VerifiedOutcomeDueSnapshotPreimage {
    domain: String,
    database_binding: VerifiedOutcomeDueDatabaseBindingPreimage,
    database_binding_hash: String,
    selection_audit_high_water_record_ordinal: u64,
    selection_audit_high_water_record_hash: String,
    selection_audit_prefix_hash: String,
    receipt_tuples_sorted: Vec<VerifiedOutcomeReceiptTuplePreimage>,
    sample_key_preimage: SampleKeyPreimage,
    sample_key: String,
    logical_subject_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    config_activation_run_id: String,
    config_hash: String,
    outcome_phase: OutcomePhase,
    stored_due_date: String,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    applicable_trading_dates: Vec<String>,
    expected_provider_bar_count: u32,
    provider_request_hash: String,
    t0_outcome_content_hash: Option<String>,
    t0_close: Option<String>,
    t0_volume: Option<String>,
});

canonical_struct!(OutcomeClaimDueBindingPreimage {
    domain: String,
    verified_due_snapshot: VerifiedOutcomeDueSnapshotPreimage,
    verified_due_snapshot_hash: String,
    same_subject_high_water_receipt_hash: Option<String>,
    outcome_attempt_ordinal: u32,
    previous_same_subject_attempt_receipt_hashes: Vec<String>,
    selection_audit_high_water_record_hash: String,
    sample_key_preimage: SampleKeyPreimage,
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    config_activation_run_id: String,
    config_hash: String,
    config_activation_receipt_hash: String,
    source_ingress_run_id: String,
    source_ingress_receipt_hash: String,
    generation_run_id: String,
    generation_receipt_hash: String,
    outcome_phase: OutcomePhase,
    t0_market_date: String,
    stored_due_date: String,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    applicable_trading_dates: Vec<String>,
    expected_provider_bar_count: u32,
    preceding_outcome_receipt_hashes: Vec<String>,
    t0_outcome_content_hash: Option<String>,
    t0_close: Option<String>,
    t0_volume: Option<String>,
});

canonical_struct!(OutcomeClaimStageInputPreimage {
    domain: String,
    stage_run_id: String,
    logical_subject_key: String,
    config_activation_run_id: String,
    config_hash: String,
    planned_outcome_run_id: String,
    due_binding: OutcomeClaimDueBindingPreimage,
    due_binding_hash: String,
    provider_request_evidence: RequestEvidencePreimage,
    provider_request_hash: String,
    provider_transport_request: OutcomeProviderRequestPreimage,
    provider_transport_request_hash: String,
    claim_lock_key: String,
    planned_run_status: RunStatus,
});

impl VerifiedOutcomeDueDatabaseBindingPreimage {
    fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_DUE_DATABASE_BINDING)?;
        require_domain(
            &self.object_binding.domain,
            DOMAIN_OUTCOME_DUE_DATABASE_OBJECT,
        )?;
        if !matches!(self.scope.as_str(), "production" | "test")
            || self.object_binding.manifest_root_canonical_path.is_empty()
            || self.database_relative_path.is_empty()
            || self.database_relative_path != self.object_binding.database_relative_path
            || self.object_binding.manifest_root_inode == 0
            || self.object_binding.database_inode == 0
            || self.object_binding.manifest_root_mode == 0
            || self.object_binding.database_mode == 0
        {
            return Err(SchemaV2Error::new(
                "outcome_due_database_binding_invalid",
                "database binding must retain the exact pinned root/database identity",
            ));
        }
        if sha256_json(&self.object_binding)? != self.object_binding_hash {
            return Err(SchemaV2Error::new(
                "outcome_due_database_object_hash_mismatch",
                "object_binding_hash must bind the exact typed object binding",
            ));
        }
        require_hash(&self.sqlite_schema_hash, "sqlite_schema_hash")?;
        match (
            self.receipt_snapshot_high_water_rowid,
            self.receipt_snapshot_high_water_content_hash.as_deref(),
        ) {
            (0, None) => {}
            (rowid, Some(hash)) if rowid > 0 => {
                require_hash(hash, "receipt_snapshot_high_water_content_hash")?
            }
            _ => {
                return Err(SchemaV2Error::new(
                    "outcome_due_receipt_high_water_matrix_invalid",
                    "zero receipt high-water has NULL hash; positive high-water has a hash",
                ));
            }
        }
        Ok(())
    }
}

impl VerifiedOutcomeReceiptTuplePreimage {
    fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_DUE_RECEIPT_TUPLE)?;
        require_canonical_uuid_v7(&self.subject_id, "receipt_tuple.subject_id")?;
        require_hash(
            &self.logical_subject_key,
            "receipt_tuple.logical_subject_key",
        )?;
        require_hash(
            &self.receipt_content_hash,
            "receipt_tuple.receipt_content_hash",
        )?;
        require_hash(
            &self.run_manifest_content_hash,
            "receipt_tuple.run_manifest_content_hash",
        )?;
        require_hash(
            &self.committed_audit_record_hash,
            "receipt_tuple.committed_audit_record_hash",
        )?;
        parse_canonical_nanos_utc(
            &self.committed_at_rfc3339_nanos_utc,
            "receipt_tuple.committed_at",
        )?;
        let valid = match self.receipt_role.as_str() {
            "config_activation" => {
                self.subject_kind == SubjectKind::ConfigActivation
                    && self.outcome_phase.is_none()
                    && self.run_status == RunStatus::Activated
            }
            "source_ingress" => {
                self.subject_kind == SubjectKind::IngressRun
                    && self.outcome_phase.is_none()
                    && matches!(
                        self.run_status,
                        RunStatus::Completed | RunStatus::FailedNonRetryable
                    )
            }
            "generation" => {
                self.subject_kind == SubjectKind::GenerationRun && self.outcome_phase.is_none()
            }
            "preceding_outcome" | "same_subject_attempt" => {
                self.subject_kind == SubjectKind::OutcomeRun && self.outcome_phase.is_some()
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(SchemaV2Error::new(
                "outcome_due_receipt_tuple_matrix_invalid",
                "receipt role/kind/phase/status matrix is not closed",
            ))
        }
    }

    fn sort_key(&self) -> (u8, u8, &str, &str, &str) {
        let role = match self.receipt_role.as_str() {
            "config_activation" => 0,
            "source_ingress" => 1,
            "generation" => 2,
            "preceding_outcome" => 3,
            "same_subject_attempt" => 4,
            _ => u8::MAX,
        };
        let phase = match self.outcome_phase {
            Some(OutcomePhase::T0Close) => 0,
            Some(OutcomePhase::D1Settled) => 1,
            Some(OutcomePhase::D3Settled) => 2,
            Some(OutcomePhase::D5Settled) => 3,
            None => u8::MAX,
        };
        (
            role,
            phase,
            &self.committed_at_rfc3339_nanos_utc,
            &self.subject_id,
            &self.receipt_content_hash,
        )
    }
}

impl VerifiedOutcomeDueSnapshotPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_VERIFIED_OUTCOME_DUE_SNAPSHOT)?;
        self.database_binding.validate()?;
        if sha256_json(&self.database_binding)? != self.database_binding_hash {
            return Err(SchemaV2Error::new(
                "outcome_due_database_binding_hash_mismatch",
                "database_binding_hash must bind the exact typed binding",
            ));
        }
        require_hash(
            &self.selection_audit_high_water_record_hash,
            "selection_audit_high_water_record_hash",
        )?;
        require_hash(
            &self.selection_audit_prefix_hash,
            "selection_audit_prefix_hash",
        )?;
        require_hash(&self.sample_key, "outcome_due.sample_key")?;
        if sha256_json(&self.sample_key_preimage)? != self.sample_key
            || self.sample_key_preimage.stock_code != self.canonical_stock_code
        {
            return Err(SchemaV2Error::new(
                "outcome_due_sample_projection_mismatch",
                "snapshot sample identity must bind the exact SampleKey preimage",
            ));
        }
        require_subject_component(
            &self.canonical_stock_code,
            "outcome_due.canonical_stock_code",
        )?;
        require_trim_stable_non_empty(&self.canonical_market, "outcome_due.canonical_market")?;
        require_canonical_uuid_v7(
            &self.config_activation_run_id,
            "outcome_due.config_activation_run_id",
        )?;
        require_hash(&self.config_hash, "outcome_due.config_hash")?;
        require_hash(&self.logical_subject_key, "outcome_due.logical_subject_key")?;
        require_trim_stable_non_empty(&self.calendar_version, "outcome_due.calendar_version")?;
        require_hash(&self.calendar_hash, "outcome_due.calendar_hash")?;
        if sha256_json(&self.trading_date_vector)? != self.trading_date_vector_hash
            || self.applicable_trading_dates
                != self
                    .trading_date_vector
                    .applicable_dates(self.outcome_phase)?
            || self.expected_provider_bar_count
                != u32::try_from(self.applicable_trading_dates.len()).map_err(|_| {
                    SchemaV2Error::new(
                        "outcome_due_bar_count_overflow",
                        "applicable date count exceeds u32",
                    )
                })?
            || self.applicable_trading_dates.last() != Some(&self.stored_due_date)
        {
            return Err(SchemaV2Error::new(
                "outcome_due_calendar_projection_mismatch",
                "snapshot phase/date/count must equal the immutable trading vector prefix",
            ));
        }
        require_hash(&self.provider_request_hash, "provider_request_hash")?;
        validate_t0_baseline_matrix(
            self.outcome_phase,
            &self.t0_outcome_content_hash,
            &self.t0_close,
            &self.t0_volume,
        )?;
        let mut previous = None;
        for tuple in &self.receipt_tuples_sorted {
            tuple.validate()?;
            if previous
                .as_ref()
                .is_some_and(|key| key >= &tuple.sort_key())
            {
                return Err(SchemaV2Error::new(
                    "outcome_due_receipts_not_sorted_unique",
                    "receipt tuples must be strictly sorted by the frozen role/phase/time/id/hash key",
                ));
            }
            previous = Some(tuple.sort_key());
        }
        Ok(())
    }
}

impl OutcomeClaimDueBindingPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_CLAIM_DUE_BINDING)?;
        self.verified_due_snapshot.validate()?;
        if sha256_json(&self.verified_due_snapshot)? != self.verified_due_snapshot_hash {
            return Err(SchemaV2Error::new(
                "outcome_claim_verified_due_hash_mismatch",
                "verified_due_snapshot_hash must bind the exact typed snapshot",
            ));
        }
        let snapshot = &self.verified_due_snapshot;
        if self.selection_audit_high_water_record_hash
            != snapshot.selection_audit_high_water_record_hash
            || self.sample_key_preimage != snapshot.sample_key_preimage
            || self.sample_key != snapshot.sample_key
            || self.canonical_stock_code != snapshot.canonical_stock_code
            || self.canonical_market != snapshot.canonical_market
            || self.config_activation_run_id != snapshot.config_activation_run_id
            || self.config_hash != snapshot.config_hash
            || self.outcome_phase != snapshot.outcome_phase
            || self.stored_due_date != snapshot.stored_due_date
            || self.calendar_version != snapshot.calendar_version
            || self.calendar_hash != snapshot.calendar_hash
            || self.trading_date_vector != snapshot.trading_date_vector
            || self.trading_date_vector_hash != snapshot.trading_date_vector_hash
            || self.applicable_trading_dates != snapshot.applicable_trading_dates
            || self.expected_provider_bar_count != snapshot.expected_provider_bar_count
            || self.t0_outcome_content_hash != snapshot.t0_outcome_content_hash
            || self.t0_close != snapshot.t0_close
            || self.t0_volume != snapshot.t0_volume
            || self.t0_market_date != self.trading_date_vector.t0
        {
            return Err(SchemaV2Error::new(
                "outcome_claim_due_projection_mismatch",
                "due binding duplicates must equal the verified snapshot byte-for-byte",
            ));
        }
        if self.outcome_attempt_ordinal
            != u32::try_from(self.previous_same_subject_attempt_receipt_hashes.len())
                .ok()
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_claim_attempt_ordinal_overflow",
                        "prior receipt count cannot produce a u32 ordinal",
                    )
                })?
            || self.same_subject_high_water_receipt_hash
                != self
                    .previous_same_subject_attempt_receipt_hashes
                    .last()
                    .cloned()
        {
            return Err(SchemaV2Error::new(
                "outcome_claim_attempt_lineage_mismatch",
                "ordinal/high-water must exactly bind prior same-subject receipts",
            ));
        }
        for hash in self
            .previous_same_subject_attempt_receipt_hashes
            .iter()
            .chain(self.preceding_outcome_receipt_hashes.iter())
            .chain([
                &self.config_activation_receipt_hash,
                &self.source_ingress_receipt_hash,
                &self.generation_receipt_hash,
            ])
        {
            require_hash(hash, "outcome_claim_receipt_hash")?;
        }
        require_canonical_uuid_v7(
            &self.source_ingress_run_id,
            "outcome_claim.source_ingress_run_id",
        )?;
        require_canonical_uuid_v7(&self.generation_run_id, "outcome_claim.generation_run_id")?;
        let expected_preceding = match self.outcome_phase {
            OutcomePhase::T0Close => 0,
            OutcomePhase::D1Settled => 1,
            OutcomePhase::D3Settled => 2,
            OutcomePhase::D5Settled => 3,
        };
        if self.preceding_outcome_receipt_hashes.len() != expected_preceding {
            return Err(SchemaV2Error::new(
                "outcome_claim_preceding_receipt_cardinality",
                "preceding outcome receipt count must equal the phase prefix",
            ));
        }
        Ok(())
    }
}

impl OutcomeClaimStageInputPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_CLAIM_STAGE)?;
        require_canonical_uuid_v7(&self.stage_run_id, "outcome_claim.stage_run_id")?;
        require_canonical_uuid_v7(
            &self.planned_outcome_run_id,
            "outcome_claim.planned_outcome_run_id",
        )?;
        if self.stage_run_id == self.planned_outcome_run_id {
            return Err(SchemaV2Error::new(
                "outcome_claim_run_identity_collision",
                "claim ID and planned outcome-run ID must differ",
            ));
        }
        require_canonical_uuid_v7(
            &self.config_activation_run_id,
            "outcome_claim.config_activation_run_id",
        )?;
        require_hash(&self.config_hash, "outcome_claim.config_hash")?;
        self.due_binding.validate()?;
        if sha256_json(&self.due_binding)? != self.due_binding_hash
            || self.config_activation_run_id != self.due_binding.config_activation_run_id
            || self.config_hash != self.due_binding.config_hash
            || self.logical_subject_key
                != self.due_binding.verified_due_snapshot.logical_subject_key
            || self.claim_lock_key != self.logical_subject_key
            || self.planned_run_status != RunStatus::Claimed
        {
            return Err(SchemaV2Error::new(
                "outcome_claim_projection_mismatch",
                "claim identity/status/due binding must be exact and lock-key bound",
            ));
        }
        validate_stage_logical_subject_key(
            &self.logical_subject_key,
            RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::OutcomeRun,
                source_fact_key: None,
                config_hash: Some(self.config_hash.clone()),
                sample_key: Some(self.due_binding.sample_key.clone()),
                outcome_phase: Some(self.due_binding.outcome_phase),
                stored_due_date: Some(self.due_binding.stored_due_date.clone()),
                ingress_source_batch_hash: None,
            },
        )?;
        self.provider_request_evidence
            .validate(Some(RequestKind::OutcomeMarketEvidence))?;
        if self.provider_request_evidence.request_hash != self.provider_request_hash
            || self.provider_request_hash
                != self.due_binding.verified_due_snapshot.provider_request_hash
        {
            return Err(SchemaV2Error::new(
                "outcome_claim_provider_request_mismatch",
                "claim request must equal the verified due request",
            ));
        }
        let provider_parameters =
            validate_canonical_json_hash::<OutcomeMarketRequestParametersPreimage>(
                &self.provider_request_evidence.parameters_json,
                &self.provider_request_evidence.parameters_json_hash,
            )?;
        self.provider_transport_request
            .validate(&self.provider_request_evidence, &provider_parameters)?;
        if sha256_json(&self.provider_transport_request)? != self.provider_transport_request_hash
            || self.provider_transport_request.semantic_request_hash != self.provider_request_hash
            || self.provider_transport_request.verified_due_binding_hash
                != self.due_binding.verified_due_snapshot_hash
        {
            return Err(SchemaV2Error::new(
                "outcome_claim_transport_request_mismatch",
                "claim must retain the exact canonical provider transport request",
            ));
        }
        Ok(())
    }

    pub const fn expected_staged_row_count(&self) -> u32 {
        1
    }
}

fn validate_t0_baseline_matrix(
    phase: OutcomePhase,
    content_hash: &Option<String>,
    close: &Option<String>,
    volume: &Option<String>,
) -> Result<(), SchemaV2Error> {
    match phase {
        OutcomePhase::T0Close if content_hash.is_none() && close.is_none() && volume.is_none() => {
            Ok(())
        }
        OutcomePhase::T0Close => Err(SchemaV2Error::new(
            "outcome_t0_baseline_matrix_invalid",
            "T0 claim must not carry a prior T0 baseline",
        )),
        _ => {
            require_hash(
                content_hash.as_deref().ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_t0_baseline_missing",
                        "post-T0 phase requires receipted T0 content",
                    )
                })?,
                "t0_outcome_content_hash",
            )?;
            let close = parse_canonical_f64(
                "t0_close",
                close.as_deref().ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_t0_baseline_missing",
                        "post-T0 phase requires T0 close",
                    )
                })?,
            )?;
            let volume = parse_canonical_f64(
                "t0_volume",
                volume.as_deref().ok_or_else(|| {
                    SchemaV2Error::new(
                        "outcome_t0_baseline_missing",
                        "post-T0 phase requires T0 volume",
                    )
                })?,
            )?;
            if close > 0.0 && volume > 0.0 {
                Ok(())
            } else {
                Err(SchemaV2Error::new(
                    "outcome_t0_baseline_invalid",
                    "T0 close and volume must be strictly positive",
                ))
            }
        }
    }
}

canonical_struct!(OutcomeStageInputPreimage {
    domain: String,
    stage_run_id: String,
    logical_subject_key: String,
    config_activation_run_id: String,
    config_hash: String,
    outcome_claim_id: String,
    outcome_claim_receipt_content_hash: String,
    outcome_claim_due_binding_hash: String,
    outcome_claim_provider_request_hash: String,
    sample_key_preimage: SampleKeyPreimage,
    sample_key: String,
    outcome_phase: OutcomePhase,
    stored_due_date: String,
    outcome_attempt_rows: Vec<SelectionOutcomeAttemptRowContentPreimage>,
    outcome_rows: Vec<SelectionSampleOutcomeRowContentPreimage>,
    planned_run_status: RunStatus,
});

impl OutcomeStageInputPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_OUTCOME_STAGE)?;
        require_canonical_uuid_v7(&self.stage_run_id, "stage_run_id")?;
        require_hash(&self.logical_subject_key, "logical_subject_key")?;
        require_canonical_uuid_v7(&self.config_activation_run_id, "config_activation_run_id")?;
        require_hash(&self.config_hash, "config_hash")?;
        require_canonical_uuid_v7(&self.outcome_claim_id, "outcome_claim_id")?;
        if self.outcome_claim_id == self.stage_run_id {
            return Err(SchemaV2Error::new(
                "outcome_claim_run_identity_collision",
                "outcome run must differ from its claim ID",
            ));
        }
        require_hash(
            &self.outcome_claim_receipt_content_hash,
            "outcome_claim_receipt_content_hash",
        )?;
        require_hash(
            &self.outcome_claim_due_binding_hash,
            "outcome_claim_due_binding_hash",
        )?;
        require_hash(
            &self.outcome_claim_provider_request_hash,
            "outcome_claim_provider_request_hash",
        )?;
        require_domain(&self.sample_key_preimage.domain, DOMAIN_SAMPLE_KEY)?;
        require_hash(&self.sample_key_preimage.event_id, "sample_event_id")?;
        require_trim_stable_non_empty(&self.sample_key_preimage.chain_id, "sample_chain_id")?;
        require_subject_component(&self.sample_key_preimage.stock_code, "sample_stock_code")?;
        require_trim_stable_non_empty(
            &self.sample_key_preimage.relation_schema_version,
            "sample_relation_schema_version",
        )?;
        require_trim_stable_non_empty(
            &self.sample_key_preimage.feature_version,
            "sample_feature_version",
        )?;
        parse_canonical_date(
            &self.sample_key_preimage.evaluation_market_date,
            "sample_evaluation_market_date",
        )?;
        if sha256_json(&self.sample_key_preimage)? != self.sample_key {
            return Err(SchemaV2Error::new(
                "outcome_sample_key_mismatch",
                "sample_key must bind the exact typed SampleKey preimage",
            ));
        }
        parse_canonical_date(&self.stored_due_date, "stored_due_date")?;
        validate_stage_logical_subject_key(
            &self.logical_subject_key,
            RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::OutcomeRun,
                source_fact_key: None,
                config_hash: Some(self.config_hash.clone()),
                sample_key: Some(self.sample_key.clone()),
                outcome_phase: Some(self.outcome_phase),
                stored_due_date: Some(self.stored_due_date.clone()),
                ingress_source_batch_hash: None,
            },
        )?;
        if self.planned_run_status == RunStatus::ExpectedWait {
            if !self.outcome_attempt_rows.is_empty() || !self.outcome_rows.is_empty() {
                return Err(SchemaV2Error::new(
                    "expected_wait_attempt_cardinality",
                    "ExpectedWait must not fabricate a provider attempt or outcome row",
                ));
            }
            return Ok(());
        }
        if self.outcome_attempt_rows.len() != 1 {
            return Err(SchemaV2Error::new(
                "outcome_attempt_cardinality",
                "provider-backed outcome runs must have exactly one attempt",
            ));
        }
        let attempt = &self.outcome_attempt_rows[0];
        attempt.validate()?;
        if let Some(request) = validate_request_evidence_columns(
            attempt.request_hash.as_deref(),
            attempt.request_evidence_json.as_deref(),
            attempt.request_evidence_hash.as_deref(),
            Some(RequestKind::OutcomeMarketEvidence),
        )? {
            let parameters = validate_canonical_json_hash::<OutcomeMarketRequestParametersPreimage>(
                &request.parameters_json,
                &request.parameters_json_hash,
            )?;
            if parameters.canonical_stock_code != self.sample_key_preimage.stock_code {
                return Err(SchemaV2Error::new(
                    "outcome_request_sample_projection_mismatch",
                    "outcome request stock code must equal the pinned SampleKey preimage",
                ));
            }
        }
        if attempt.outcome_run_id != self.stage_run_id
            || attempt.sample_key != self.sample_key
            || attempt.phase != self.outcome_phase
            || attempt.stored_due_date != self.stored_due_date
        {
            return Err(SchemaV2Error::new(
                "outcome_attempt_identity_mismatch",
                "attempt must match envelope run/sample/phase/due date",
            ));
        }
        let expected_status = match attempt.result_code {
            OutcomeAttemptResult::Settled => RunStatus::Settled,
            OutcomeAttemptResult::ExpectedWait => {
                return Err(SchemaV2Error::new(
                    "expected_wait_attempt_forbidden",
                    "ExpectedWait is represented by a zero-attempt receipted run",
                ));
            }
            OutcomeAttemptResult::Error if attempt.retryable == Some(true) => {
                RunStatus::FailedRetryable
            }
            OutcomeAttemptResult::Error => RunStatus::FailedNonRetryable,
        };
        if self.planned_run_status != expected_status {
            return Err(SchemaV2Error::new(
                "outcome_status_result_mismatch",
                "planned status does not match attempt result/retryability",
            ));
        }
        match attempt.result_code {
            OutcomeAttemptResult::Settled if self.outcome_rows.len() == 1 => {
                let outcome = &self.outcome_rows[0];
                if outcome.outcome_run_id != self.stage_run_id
                    || outcome.sample_key != self.sample_key
                    || outcome.phase != self.outcome_phase
                    || outcome.due_trading_date != self.stored_due_date
                {
                    return Err(SchemaV2Error::new(
                        "settled_outcome_identity_mismatch",
                        "settled outcome must match run/sample/phase/due date",
                    ));
                }
                Self::validate_outcome_row(outcome)?;
                if outcome.provider.as_str()
                    != attempt.provider.as_deref().expect("settled provider")
                    || outcome.source.as_str() != attempt.source.as_deref().expect("settled source")
                    || outcome.source_at != attempt.source_at
                    || Some(&outcome.observed_at) != attempt.observed_at.as_ref()
                    || Some(&outcome.batch_id) != attempt.batch_id.as_ref()
                    || Some(&outcome.batch_content_hash) != attempt.batch_content_hash.as_ref()
                {
                    return Err(SchemaV2Error::new(
                        "settled_outcome_evidence_projection_mismatch",
                        "settled outcome provider evidence must equal its complete attempt evidence",
                    ));
                }
                let outcome_hash = sha256_json(outcome)?;
                if attempt.settled_outcome_content_hash.as_deref() != Some(&outcome_hash) {
                    return Err(SchemaV2Error::new(
                        "settled_outcome_hash_mismatch",
                        "attempt must bind the exact outcome row content hash",
                    ));
                }
            }
            OutcomeAttemptResult::Settled => {
                return Err(SchemaV2Error::new(
                    "settled_outcome_cardinality",
                    "settled requires exactly one outcome row",
                ));
            }
            _ if !self.outcome_rows.is_empty() => {
                return Err(SchemaV2Error::new(
                    "non_settled_outcome_cardinality",
                    "non-settled runs must have zero outcome rows",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_outcome_row(
        outcome: &SelectionSampleOutcomeRowContentPreimage,
    ) -> Result<(), SchemaV2Error> {
        require_domain(&outcome.domain, DOMAIN_SAMPLE_OUTCOME_ROW)?;
        require_non_empty(&outcome.provider, "outcome_provider")?;
        require_non_empty(&outcome.source, "outcome_source")?;
        require_non_empty(&outcome.observed_at, "outcome_observed_at")?;
        require_non_empty(&outcome.batch_id, "outcome_batch_id")?;
        require_hash(&outcome.batch_content_hash, "outcome_batch_content_hash")?;
        let open = parse_canonical_f64("open", &outcome.open)?;
        let high = parse_canonical_f64("high", &outcome.high)?;
        let low = parse_canonical_f64("low", &outcome.low)?;
        let close = parse_canonical_f64("close", &outcome.close)?;
        if [open, high, low, close]
            .into_iter()
            .any(|price| price <= 0.0)
        {
            return Err(SchemaV2Error::new(
                "outcome_price_invalid",
                "outcome prices must be finite and strictly positive",
            ));
        }
        if high < open || high < close || low > open || low > close || high < low {
            return Err(SchemaV2Error::new(
                "outcome_ohlc_invalid",
                "outcome OHLC relationships are inconsistent",
            ));
        }
        let volume = parse_canonical_f64("volume", &outcome.volume)?;
        let amount = parse_canonical_f64("amount", &outcome.amount)?;
        parse_canonical_f64("return_from_t0_close", &outcome.return_from_t0_close)?;
        parse_canonical_f64("cumulative_mfe", &outcome.cumulative_mfe)?;
        parse_canonical_f64("cumulative_mae", &outcome.cumulative_mae)?;
        let volume_ratio = parse_canonical_f64("volume_ratio", &outcome.volume_ratio)?;
        if volume <= 0.0 || amount < 0.0 || volume_ratio <= 0.0 {
            return Err(SchemaV2Error::new(
                "outcome_volume_amount_invalid",
                "volume and volume_ratio must be strictly positive; amount must be non-negative",
            ));
        }
        if outcome.phase == OutcomePhase::T0Close
            && (outcome.return_from_t0_close != "0"
                || outcome.cumulative_mfe != "0"
                || outcome.cumulative_mae != "0"
                || outcome.volume_ratio != "1")
        {
            return Err(SchemaV2Error::new(
                "t0_baseline_not_fixed",
                "T0 return/MFE/MAE must be 0 and volume ratio must be 1",
            ));
        }
        Ok(())
    }

    pub fn expected_staged_row_count(&self) -> u32 {
        1 + self.outcome_attempt_rows.len() as u32 + self.outcome_rows.len() as u32
    }
}

canonical_struct!(RunLogicalSubjectPreimage {
    domain: String,
    subject_kind: SubjectKind,
    source_fact_key: Option<String>,
    config_hash: Option<String>,
    sample_key: Option<String>,
    outcome_phase: Option<OutcomePhase>,
    stored_due_date: Option<String>,
    ingress_source_batch_hash: Option<String>,
});

impl RunLogicalSubjectPreimage {
    pub fn validate(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_RUN_LOGICAL_SUBJECT)?;
        let valid = match self.subject_kind {
            SubjectKind::ConfigActivation => {
                self.source_fact_key.is_none()
                    && self.config_hash.is_some()
                    && self.sample_key.is_none()
                    && self.outcome_phase.is_none()
                    && self.stored_due_date.is_none()
                    && self.ingress_source_batch_hash.is_none()
            }
            SubjectKind::IngressRun => {
                self.source_fact_key.is_none()
                    && self.config_hash.is_some()
                    && self.sample_key.is_none()
                    && self.outcome_phase.is_none()
                    && self.stored_due_date.is_none()
                    && self.ingress_source_batch_hash.is_some()
            }
            SubjectKind::GenerationRun => {
                self.source_fact_key.is_some()
                    && self.config_hash.is_some()
                    && self.sample_key.is_none()
                    && self.outcome_phase.is_none()
                    && self.stored_due_date.is_none()
                    && self.ingress_source_batch_hash.is_none()
            }
            SubjectKind::OutcomeClaim | SubjectKind::OutcomeRun => {
                self.source_fact_key.is_none()
                    && self.config_hash.is_some()
                    && self.sample_key.is_some()
                    && self.outcome_phase.is_some()
                    && self.stored_due_date.is_some()
                    && self.ingress_source_batch_hash.is_none()
            }
        };
        if !valid {
            return Err(SchemaV2Error::new(
                "invalid_logical_subject_matrix",
                "logical subject required/NULL matrix does not match subject kind",
            ));
        }
        if let Some(source_fact_key) = &self.source_fact_key {
            require_hash(source_fact_key, "logical_subject.source_fact_key")?;
        }
        if let Some(config_hash) = &self.config_hash {
            require_hash(config_hash, "logical_subject.config_hash")?;
        }
        if let Some(sample_key) = &self.sample_key {
            require_hash(sample_key, "logical_subject.sample_key")?;
        }
        if let Some(stored_due_date) = &self.stored_due_date {
            parse_canonical_date(stored_due_date, "logical_subject.stored_due_date")?;
        }
        if let Some(source_batch_hash) = &self.ingress_source_batch_hash {
            require_hash(
                source_batch_hash,
                "logical_subject.ingress_source_batch_hash",
            )?;
        }
        Ok(())
    }
}

pub fn run_logical_subject_key(
    preimage: &RunLogicalSubjectPreimage,
) -> Result<String, SchemaV2Error> {
    preimage.validate()?;
    sha256_json(preimage)
}

fn validate_stage_logical_subject_key(
    actual: &str,
    preimage: RunLogicalSubjectPreimage,
) -> Result<(), SchemaV2Error> {
    require_hash(actual, "logical_subject_key")?;
    if run_logical_subject_key(&preimage)? == actual {
        Ok(())
    } else {
        Err(SchemaV2Error::new(
            "logical_subject_key_mismatch",
            "logical_subject_key must bind the exact authoritative stage subject fields",
        ))
    }
}

canonical_struct!(RunRowLogicalPrimaryKeyPreimage {
    domain: String,
    table_ordinal: u8,
    key_parts: Vec<String>,
});

canonical_struct!(RunRowHashPreimage {
    domain: String,
    table_ordinal: u8,
    table_name: String,
    logical_primary_key: String,
    row_content_hash: String,
});

canonical_struct!(RunPayloadPreimage {
    domain: String,
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    source_fact_key: Option<String>,
    config_activation_run_id: String,
    config_hash: String,
    config_snapshot_json_hash: Option<String>,
    config_activation_content_hash: Option<String>,
    config_activation_file_content_hash: Option<String>,
    config_effective_from_rfc3339_nanos_utc: Option<String>,
    artifact_valid_from: Option<String>,
    artifact_expires_at: Option<String>,
    executable_revision: Option<String>,
    legacy_cutover_snapshot_hash: Option<String>,
    generation_market_date: Option<String>,
    aggregator_observed_at_rfc3339_nanos_utc: Option<String>,
    ingress_source_batch_content_hash: Option<String>,
    outcome_phase: Option<OutcomePhase>,
    stored_due_date: Option<String>,
    outcome_claim_id: Option<String>,
    planned_outcome_run_id: Option<String>,
    outcome_claim_receipt_content_hash: Option<String>,
    outcome_claim_due_binding_hash: Option<String>,
    outcome_claim_provider_request_hash: Option<String>,
    rows: Vec<String>,
});

canonical_struct!(StagedDbPreimage {
    domain: String,
    subject_kind: SubjectKind,
    subject_id: String,
    expected_staged_row_count: u32,
    rows: Vec<String>,
});

canonical_struct!(RunManifestContentPreimage {
    domain: String,
    subject_kind: SubjectKind,
    subject_id: String,
    in_memory_payload_hash: String,
    prepared_record_hash: String,
    expected_staged_row_count: u32,
    staged_db_content_hash: String,
    recovery_envelope_content_hash: String,
    logical_subject_key: String,
    run_status: RunStatus,
    source_fact_key: Option<String>,
    config_activation_run_id: Option<String>,
    config_hash: Option<String>,
    config_snapshot_json_hash: Option<String>,
    config_activation_content_hash: Option<String>,
    config_activation_file_content_hash: Option<String>,
    config_effective_from_rfc3339_nanos_utc: Option<String>,
    artifact_valid_from: Option<String>,
    artifact_expires_at: Option<String>,
    executable_revision: Option<String>,
    legacy_cutover_snapshot_hash: Option<String>,
    generation_market_date: Option<String>,
    aggregator_observed_at_rfc3339_nanos_utc: Option<String>,
    ingress_source_batch_content_hash: Option<String>,
    outcome_phase: Option<OutcomePhase>,
    stored_due_date: Option<String>,
    outcome_claim_id: Option<String>,
    planned_outcome_run_id: Option<String>,
    outcome_claim_receipt_content_hash: Option<String>,
    outcome_claim_due_binding_hash: Option<String>,
    outcome_claim_provider_request_hash: Option<String>,
    staged_at_rfc3339_nanos_utc: String,
});

impl RunManifestContentPreimage {
    pub fn validate_kind_matrix(&self) -> Result<(), SchemaV2Error> {
        require_domain(&self.domain, DOMAIN_RUN_MANIFEST)?;
        let activation_only = self.config_snapshot_json_hash.is_some()
            && self.config_activation_content_hash.is_some()
            && self.config_activation_file_content_hash.is_some()
            && self.config_effective_from_rfc3339_nanos_utc.is_some()
            && self.artifact_valid_from.is_some()
            && self.artifact_expires_at.is_some()
            && self.executable_revision.is_some()
            && self.legacy_cutover_snapshot_hash.is_some();
        let activation_fields_null = self.config_snapshot_json_hash.is_none()
            && self.config_activation_content_hash.is_none()
            && self.config_activation_file_content_hash.is_none()
            && self.config_effective_from_rfc3339_nanos_utc.is_none()
            && self.artifact_valid_from.is_none()
            && self.artifact_expires_at.is_none()
            && self.executable_revision.is_none()
            && self.legacy_cutover_snapshot_hash.is_none();
        let claim_fields_null = self.outcome_claim_id.is_none()
            && self.planned_outcome_run_id.is_none()
            && self.outcome_claim_receipt_content_hash.is_none()
            && self.outcome_claim_due_binding_hash.is_none()
            && self.outcome_claim_provider_request_hash.is_none();
        let valid = match self.subject_kind {
            SubjectKind::ConfigActivation => {
                self.run_status == RunStatus::Activated
                    && self.config_activation_run_id.as_deref() == Some(&self.subject_id)
                    && self.config_hash.is_some()
                    && activation_only
                    && self.source_fact_key.is_none()
                    && self.generation_market_date.is_none()
                    && self.aggregator_observed_at_rfc3339_nanos_utc.is_none()
                    && self.ingress_source_batch_content_hash.is_none()
                    && self.outcome_phase.is_none()
                    && self.stored_due_date.is_none()
                    && claim_fields_null
            }
            SubjectKind::IngressRun => {
                matches!(
                    self.run_status,
                    RunStatus::Completed | RunStatus::FailedNonRetryable
                ) && self.config_activation_run_id.is_some()
                    && self.config_hash.is_some()
                    && activation_fields_null
                    && self.source_fact_key.is_none()
                    && self.generation_market_date.is_some()
                    && self.aggregator_observed_at_rfc3339_nanos_utc.is_some()
                    && self.ingress_source_batch_content_hash.is_some()
                    && self.outcome_phase.is_none()
                    && self.stored_due_date.is_none()
                    && claim_fields_null
            }
            SubjectKind::GenerationRun => {
                matches!(
                    self.run_status,
                    RunStatus::Completed
                        | RunStatus::VerifiedNoRelation
                        | RunStatus::PendingDependency
                        | RunStatus::FailedNonRetryable
                ) && self.config_activation_run_id.is_some()
                    && self.config_hash.is_some()
                    && activation_fields_null
                    && self.source_fact_key.is_some()
                    && self.generation_market_date.is_some()
                    && self.aggregator_observed_at_rfc3339_nanos_utc.is_none()
                    && self.ingress_source_batch_content_hash.is_none()
                    && self.outcome_phase.is_none()
                    && self.stored_due_date.is_none()
                    && claim_fields_null
            }
            SubjectKind::OutcomeClaim => {
                self.run_status == RunStatus::Claimed
                    && self.config_activation_run_id.is_some()
                    && self.config_hash.is_some()
                    && activation_fields_null
                    && self.source_fact_key.is_none()
                    && self.generation_market_date.is_none()
                    && self.aggregator_observed_at_rfc3339_nanos_utc.is_none()
                    && self.ingress_source_batch_content_hash.is_none()
                    && self.outcome_phase.is_some()
                    && self.stored_due_date.is_some()
                    && self.outcome_claim_id.as_deref() == Some(&self.subject_id)
                    && self.planned_outcome_run_id.as_ref().is_some_and(|planned| {
                        planned != &self.subject_id
                            && require_canonical_uuid_v7(planned, "planned_outcome_run_id").is_ok()
                    })
                    && self.outcome_claim_receipt_content_hash.is_none()
                    && self.outcome_claim_due_binding_hash.is_some()
                    && self.outcome_claim_provider_request_hash.is_some()
            }
            SubjectKind::OutcomeRun => {
                matches!(
                    self.run_status,
                    RunStatus::Settled
                        | RunStatus::ExpectedWait
                        | RunStatus::FailedRetryable
                        | RunStatus::FailedNonRetryable
                ) && self.config_activation_run_id.is_some()
                    && self.config_hash.is_some()
                    && activation_fields_null
                    && self.source_fact_key.is_none()
                    && self.generation_market_date.is_none()
                    && self.aggregator_observed_at_rfc3339_nanos_utc.is_none()
                    && self.ingress_source_batch_content_hash.is_none()
                    && self.outcome_phase.is_some()
                    && self.stored_due_date.is_some()
                    && self.outcome_claim_id.is_some()
                    && self.planned_outcome_run_id.is_none()
                    && self.outcome_claim_receipt_content_hash.is_some()
                    && self.outcome_claim_due_binding_hash.is_some()
                    && self.outcome_claim_provider_request_hash.is_some()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(SchemaV2Error::new(
                "invalid_manifest_kind_matrix",
                "manifest fields/status do not match subject kind",
            ))
        }
    }
}

pub type SelectionRunStageRowContentPreimage = RunManifestContentPreimage;

canonical_struct!(CommitReceiptContentPreimage {
    domain: String,
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    in_memory_payload_hash: String,
    recovery_envelope_content_hash: String,
    prepared_audit_hash: String,
    run_manifest_content_hash: String,
    staged_db_content_hash: String,
    committed_audit_hash: String,
    committed_at_rfc3339_nanos_utc: String,
});

pub type SelectionCommitReceiptRowContentPreimage = CommitReceiptContentPreimage;

canonical_struct!(PreparedAuditContentPreimage {
    domain: String,
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    recovery_envelope_content_hash: String,
    in_memory_payload_hash: String,
});

canonical_struct!(CommittedAuditContentPreimage {
    domain: String,
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    recovery_envelope_content_hash: String,
    prepared_record_hash: String,
    run_manifest_content_hash: String,
    staged_db_content_hash: String,
});

pub fn validate_stage_payload_json(
    subject_kind: SubjectKind,
    payload_schema: &str,
    payload_json: &str,
    payload_json_hash: &str,
) -> Result<(), SchemaV2Error> {
    match (subject_kind, payload_schema) {
        (SubjectKind::ConfigActivation, CONFIG_ACTIVATION_STAGE_PAYLOAD_SCHEMA) => {
            validate_canonical_json_hash::<ConfigActivationStageInputPreimage>(
                payload_json,
                payload_json_hash,
            )?
            .validate()
        }
        (SubjectKind::IngressRun, SOURCE_INGRESS_STAGE_PAYLOAD_SCHEMA) => {
            validate_canonical_json_hash::<SourceIngressStageInputPreimage>(
                payload_json,
                payload_json_hash,
            )?
            .validate()
        }
        (SubjectKind::GenerationRun, GENERATION_STAGE_PAYLOAD_SCHEMA) => {
            validate_canonical_json_hash::<GenerationStageInputPreimage>(
                payload_json,
                payload_json_hash,
            )?
            .validate()
        }
        (SubjectKind::OutcomeClaim, OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA) => {
            validate_canonical_json_hash::<OutcomeClaimStageInputPreimage>(
                payload_json,
                payload_json_hash,
            )?
            .validate()
        }
        (SubjectKind::OutcomeRun, OUTCOME_STAGE_PAYLOAD_SCHEMA) => {
            validate_canonical_json_hash::<OutcomeStageInputPreimage>(
                payload_json,
                payload_json_hash,
            )?
            .validate()
        }
        _ => Err(SchemaV2Error::new(
            "stage_payload_schema_mismatch",
            "subject kind and exact stage payload schema are not a registered pair",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn hash(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    #[test]
    fn generation_attempt_nullable_columns_deserialize_without_synthetic_defaults() {
        let relation_value = serde_json::to_value(direct_relation_row('1')).unwrap();
        let relation: SelectionRelationAttemptRowContentPreimage =
            serde_json::from_value(relation_value)
                .expect("DDL-null relation fields must remain NULL in the typed row");
        assert_eq!(relation.request_hash, None);
        assert_eq!(relation.request_evidence_json, None);
        assert_eq!(relation.retryable, None);

        let sample = admitted_sample_row(&relation);
        let evaluation_value = serde_json::to_value(completed_evaluation_row(&sample)).unwrap();
        let evaluation: SelectionEvaluationAttemptRowContentPreimage =
            serde_json::from_value(evaluation_value)
                .expect("DDL-null evaluation retryability must remain NULL in the typed row");
        assert_eq!(evaluation.retryable, None);
    }

    fn board_record(
        kind: ProviderBoardKind,
        name: &str,
        member_count: u32,
        observed_at: &str,
        batch_id: &str,
    ) -> DirectoryBoardRecordPreimage {
        DirectoryBoardRecordPreimage {
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
        }
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
                records_in_provider_order: vec![board_record(
                    kind,
                    name,
                    member_count,
                    observed_at,
                    batch_id,
                )],
            },
            batch_content_hash: batch_content_hash.into(),
            record_count: 1,
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
                prepared_record_hash: hash('a'),
                committed_record_hash: hash('b'),
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

    fn magic_tdx_capability(
        capability_name: &str,
        contract_version: &str,
    ) -> ProviderCapabilityHashPreimage {
        ProviderCapabilityHashPreimage {
            domain: DOMAIN_PROVIDER_CAPABILITY.into(),
            provider: "magic-tdx".into(),
            capability_name: capability_name.into(),
            contract_version: contract_version.into(),
            upstream_revision: UPSTREAM_REVISION.into(),
        }
    }

    fn outcome_sample_key_preimage() -> SampleKeyPreimage {
        SampleKeyPreimage {
            domain: DOMAIN_SAMPLE_KEY.into(),
            event_id: hash('4'),
            chain_id: "TEST_CODE_CHAIN".into(),
            stock_code: "TEST_CODE_000001".into(),
            relation_schema_version: "relation-v1".into(),
            feature_version: "feature-v1".into(),
            evaluation_market_date: "2026-07-28".into(),
        }
    }

    fn outcome_sample_key() -> String {
        sha256_json(&outcome_sample_key_preimage()).unwrap()
    }

    fn outcome_trading_date_vector() -> OutcomeTradingDateVectorPreimage {
        OutcomeTradingDateVectorPreimage {
            domain: DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
            t0: "2026-07-28".into(),
            d1: "2026-07-29".into(),
            d2: "2026-07-30".into(),
            d3: "2026-07-31".into(),
            d4: "2026-08-03".into(),
            d5: "2026-08-04".into(),
        }
    }

    fn outcome_request_columns() -> RequestEvidenceColumns {
        let vector = outcome_trading_date_vector();
        build_request_evidence(
            RequestParametersPreimage::OutcomeMarketEvidence(
                OutcomeMarketRequestParametersPreimage {
                    domain: DOMAIN_OUTCOME_MARKET_REQUEST.into(),
                    sample_key: outcome_sample_key(),
                    canonical_stock_code: "TEST_CODE_000001".into(),
                    canonical_market: "SZ".into(),
                    phase: OutcomePhase::D1Settled,
                    stored_due_date: "2026-07-29".into(),
                    calendar_version: "calendar-v1".into(),
                    calendar_hash: hash('a'),
                    trading_date_vector_hash: sha256_json(&vector).unwrap(),
                    applicable_trading_dates: vector
                        .applicable_dates(OutcomePhase::D1Settled)
                        .unwrap(),
                    trading_date_vector: vector,
                    window_start: "2026-07-28".into(),
                    window_end: "2026-07-29".into(),
                    interval: DailyIntervalKind::Day,
                    adjustment: AdjustmentKind::None,
                },
            ),
            magic_tdx_capability(
                "MagicTdx-UnadjustedDailyBars",
                "magic-market-core.MarketDataProvider.bars.v0.2.0",
            ),
        )
        .unwrap()
    }

    #[test]
    fn outcome_request_rejects_equal_endpoints_and_count_with_wrong_middle_date() {
        let vector = outcome_trading_date_vector();
        let mut applicable = vector
            .applicable_dates(OutcomePhase::D3Settled)
            .expect("canonical prefix");
        applicable[2] = "2026-07-29".into();
        let error = build_request_evidence(
            RequestParametersPreimage::OutcomeMarketEvidence(
                OutcomeMarketRequestParametersPreimage {
                    domain: DOMAIN_OUTCOME_MARKET_REQUEST.into(),
                    sample_key: outcome_sample_key(),
                    canonical_stock_code: "TEST_CODE_000001".into(),
                    canonical_market: "SZ".into(),
                    phase: OutcomePhase::D3Settled,
                    stored_due_date: "2026-07-31".into(),
                    calendar_version: "calendar-v1".into(),
                    calendar_hash: hash('a'),
                    trading_date_vector_hash: sha256_json(&vector).unwrap(),
                    applicable_trading_dates: applicable,
                    trading_date_vector: vector,
                    window_start: "2026-07-28".into(),
                    window_end: "2026-07-31".into(),
                    interval: DailyIntervalKind::Day,
                    adjustment: AdjustmentKind::None,
                },
            ),
            magic_tdx_capability(
                "MagicTdx-UnadjustedDailyBars",
                "magic-market-core.MarketDataProvider.bars.v0.2.0",
            ),
        )
        .expect_err("wrong middle date must fail despite equal endpoints/count");
        assert_eq!(error.code, "outcome_applicable_trading_dates_mismatch");
    }

    fn t0_request_columns(sample: &SelectionSampleRowContentPreimage) -> RequestEvidenceColumns {
        build_request_evidence(
            RequestParametersPreimage::T0MarketEvidence(T0MarketRequestParametersPreimage {
                domain: DOMAIN_T0_MARKET_REQUEST.into(),
                canonical_stock_code: sample.canonical_stock_code.clone(),
                canonical_market: sample.canonical_market.clone(),
                evaluation_market_date: sample.evaluation_market_date.clone(),
                quote_max_age_secs: 5,
                daily_interval: "day".into(),
                daily_limit: 60,
                intraday_interval: "five_minutes".into(),
                intraday_limit: 120,
                adjustment: AdjustmentKind::None,
            }),
            magic_tdx_capability(
                "MagicTdx-T0MarketBundle",
                "stock-analysis.MagicTdxSelectionGateway.t0_market_evidence.v1",
            ),
        )
        .unwrap()
    }

    fn outcome_row() -> SelectionSampleOutcomeRowContentPreimage {
        SelectionSampleOutcomeRowContentPreimage {
            domain: DOMAIN_SAMPLE_OUTCOME_ROW.into(),
            sample_key: outcome_sample_key(),
            phase: OutcomePhase::D1Settled,
            outcome_run_id: "01900000-0000-7000-8000-000000000001".into(),
            due_trading_date: "2026-07-29".into(),
            open: "10".into(),
            high: "11".into(),
            low: "9".into(),
            close: "10.5".into(),
            volume: "1000".into(),
            amount: "10500".into(),
            return_from_t0_close: "0.05".into(),
            cumulative_mfe: "0.1".into(),
            cumulative_mae: "-0.1".into(),
            volume_ratio: "1.2".into(),
            provider: "magic-tdx".into(),
            source: "tdx-smart".into(),
            source_at: Some("2026-07-29".into()),
            observed_at: "2026-07-28T07:01:00.000000000Z".into(),
            batch_id: "batch-1".into(),
            batch_content_hash: hash('2'),
            created_at: "2026-07-28T07:02:00.000000000Z".into(),
        }
    }

    fn outcome_transport_attempt_columns() -> (String, String) {
        let request_columns = outcome_request_columns();
        let request: RequestEvidencePreimage =
            serde_json::from_str(&request_columns.request_evidence_json).unwrap();
        let parameters: OutcomeMarketRequestParametersPreimage =
            serde_json::from_str(&request.parameters_json).unwrap();
        let batch_content = OutcomeTransportBatchContentPreimage {
            provider: "magic-tdx".into(),
            source: "tdx-smart".into(),
            records: vec![
                OutcomeTransportBarFingerprint {
                    market_date: "2026-07-28".into(),
                    open: "10".into(),
                    high: "10.5".into(),
                    low: "9.5".into(),
                    close: "10".into(),
                    core_volume_lots: "10".into(),
                    amount: Some("10000".into()),
                    provider: "Tdx".into(),
                    batch_id: "batch-1".into(),
                },
                OutcomeTransportBarFingerprint {
                    market_date: "2026-07-29".into(),
                    open: "10".into(),
                    high: "11".into(),
                    low: "9".into(),
                    close: "10.5".into(),
                    core_volume_lots: "10".into(),
                    amount: Some("10500".into()),
                    provider: "Tdx".into(),
                    batch_id: "batch-1".into(),
                },
            ],
        };
        let evidence = OutcomeTransportEvidencePreimage {
            source: "tdx-smart".into(),
            source_at: Some("2026-07-29".into()),
            observed_at: "2026-07-28T07:01:00.000000000Z".into(),
            batch_id: "batch-1".into(),
            record_count: 2,
            batch_content_hash: sha256_json(&batch_content).unwrap(),
            batch_content,
        };
        let transport_request = OutcomeTransportRequestPreimage {
            provider: "magic-tdx".into(),
            source: "tdx-smart".into(),
            canonical_stock_code: parameters.canonical_stock_code.clone(),
            canonical_market: parameters.canonical_market.clone(),
            interval: parameters.interval.as_str().into(),
            adjustment: parameters.adjustment.as_str().into(),
            latest_n: 2,
        };
        let result = OutcomeTransportResultPreimage {
            terminal_state: "available".into(),
            requested_latest_n: 2,
            actual_count: Some(2),
            provider_evidence_hash: Some(sha256_json(&evidence).unwrap()),
            provider_evidence: Some(evidence),
            provider_error: None,
            provider_error_hash: None,
        };
        let attempt = OutcomeTransportAttemptPreimage {
            request_ordinal: 0,
            request_hash: sha256_json(&transport_request).unwrap(),
            request: transport_request,
            result_hash: sha256_json(&result).unwrap(),
            result,
        };
        let attempts = OutcomeTransportAttemptsPreimage {
            domain: DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS.into(),
            design_sha256: OUTCOME_PARENT_DESIGN_SHA256.into(),
            amendment_design_sha256: AMENDMENT_DESIGN_SHA256.into(),
            row_request_hash: request_columns.request_hash,
            request_evidence_hash: request_columns.request_evidence_hash,
            provider_capability_hash: request.provider_capability_hash,
            provider_revision: UPSTREAM_REVISION.into(),
            request_parameters_hash: request.parameters_json_hash,
            provider_request_hash: hash('9'),
            verified_due_binding_hash: hash('8'),
            adaptive_policy_version: OUTCOME_ADAPTIVE_POLICY_VERSION.into(),
            expected_bar_count: 2,
            maximum_latest_n: 10,
            selected_transport_result_hash: Some(attempt.result_hash.clone()),
            attempts_in_request_order: vec![attempt],
        };
        (
            canonical_json(&attempts).unwrap(),
            sha256_json(&attempts).unwrap(),
        )
    }

    fn outcome_attempt(
        result_code: OutcomeAttemptResult,
    ) -> SelectionOutcomeAttemptRowContentPreimage {
        let request = outcome_request_columns();
        let mut attempt = SelectionOutcomeAttemptRowContentPreimage {
            domain: DOMAIN_OUTCOME_ATTEMPT_ROW.into(),
            outcome_attempt_id: hash('3'),
            sample_key: outcome_sample_key(),
            phase: OutcomePhase::D1Settled,
            stored_due_date: "2026-07-29".into(),
            outcome_run_id: "01900000-0000-7000-8000-000000000001".into(),
            request_hash: Some(request.request_hash),
            request_evidence_json: Some(request.request_evidence_json),
            request_evidence_hash: Some(request.request_evidence_hash),
            transport_attempts_json: None,
            transport_attempts_hash: None,
            result_code,
            reason_code: None,
            retryable: None,
            provider: None,
            source: None,
            source_at: None,
            observed_at: None,
            batch_id: None,
            batch_content_hash: None,
            available_evidence_json: None,
            available_evidence_hash: None,
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            settled_outcome_content_hash: None,
            attempted_at: "2026-07-28T07:02:00.000000000Z".into(),
        };
        match result_code {
            OutcomeAttemptResult::Settled => {
                let (json, transport_hash) = outcome_transport_attempt_columns();
                attempt.transport_attempts_json = Some(json);
                attempt.transport_attempts_hash = Some(transport_hash);
                let provider_evidence = ProviderAvailableEvidencePreimage {
                    domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
                    evidence_kind: ProviderEvidenceKind::OutcomeDailyBars,
                    provider: "magic-tdx".into(),
                    source: Some("tdx-smart".into()),
                    source_at: Some("2026-07-29".into()),
                    observed_at: Some("2026-07-28T07:01:00.000000000Z".into()),
                    batch_id: Some("batch-1".into()),
                    batch_content_hash: Some(hash('2')),
                };
                let request = outcome_request_columns();
                let vector = outcome_trading_date_vector();
                let evidence = OutcomeProviderAvailableEvidencePreimage {
                    domain: DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE.into(),
                    request_hash: request.request_hash,
                    calendar_hash: hash('a'),
                    trading_date_vector_hash: sha256_json(&vector).unwrap(),
                    expected_trading_dates: vector
                        .applicable_dates(OutcomePhase::D1Settled)
                        .unwrap(),
                    returned_trading_dates: vector
                        .applicable_dates(OutcomePhase::D1Settled)
                        .unwrap(),
                    provider_evidence,
                };
                attempt.provider = Some(evidence.provider_evidence.provider.clone());
                attempt.source = evidence.provider_evidence.source.clone();
                attempt.source_at = evidence.provider_evidence.source_at.clone();
                attempt.observed_at = evidence.provider_evidence.observed_at.clone();
                attempt.batch_id = evidence.provider_evidence.batch_id.clone();
                attempt.batch_content_hash = evidence.provider_evidence.batch_content_hash.clone();
                attempt.available_evidence_json = Some(canonical_json(&evidence).unwrap());
                attempt.available_evidence_hash = Some(sha256_json(&evidence).unwrap());
                attempt.settled_outcome_content_hash = Some(sha256_json(&outcome_row()).unwrap());
            }
            OutcomeAttemptResult::ExpectedWait => {
                attempt.request_hash = None;
                attempt.request_evidence_json = None;
                attempt.request_evidence_hash = None;
                attempt.reason_code = Some(OutcomeReasonCodeV1::MarketSessionUnsettled);
            }
            OutcomeAttemptResult::Error => {
                let (json, transport_hash) = outcome_transport_attempt_columns();
                attempt.transport_attempts_json = Some(json);
                attempt.transport_attempts_hash = Some(transport_hash);
                attempt.reason_code = Some(OutcomeReasonCodeV1::SettledBarMissing);
                attempt.retryable = Some(true);
                let provider_evidence = ProviderAvailableEvidencePreimage {
                    domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
                    evidence_kind: ProviderEvidenceKind::OutcomeDailyBars,
                    provider: "magic-tdx".into(),
                    source: Some("tdx-smart".into()),
                    source_at: Some("2026-07-29".into()),
                    observed_at: Some("2026-07-28T07:01:00.000000000Z".into()),
                    batch_id: Some("batch-1".into()),
                    batch_content_hash: Some(hash('2')),
                };
                let request = outcome_request_columns();
                let vector = outcome_trading_date_vector();
                let evidence = OutcomeProviderAvailableEvidencePreimage {
                    domain: DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE.into(),
                    request_hash: request.request_hash,
                    calendar_hash: hash('a'),
                    trading_date_vector_hash: sha256_json(&vector).unwrap(),
                    expected_trading_dates: vector
                        .applicable_dates(OutcomePhase::D1Settled)
                        .unwrap(),
                    returned_trading_dates: vec!["2026-07-28".into()],
                    provider_evidence,
                };
                attempt.provider = Some(evidence.provider_evidence.provider.clone());
                attempt.source = evidence.provider_evidence.source.clone();
                attempt.source_at = evidence.provider_evidence.source_at.clone();
                attempt.observed_at = evidence.provider_evidence.observed_at.clone();
                attempt.batch_id = evidence.provider_evidence.batch_id.clone();
                attempt.batch_content_hash = evidence.provider_evidence.batch_content_hash.clone();
                attempt.available_evidence_json = Some(canonical_json(&evidence).unwrap());
                attempt.available_evidence_hash = Some(sha256_json(&evidence).unwrap());
                let detail = ProviderErrorDetailPreimage {
                    domain: DOMAIN_PROVIDER_ERROR_DETAIL.into(),
                    error_kind: ProviderErrorKind::InvalidData,
                    provider: "magic-tdx".into(),
                    operation: "outcome_daily_bars".into(),
                    error_code: Some("settled_bar_missing".into()),
                    http_status: None,
                    timeout_ms: None,
                    invariant_id: None,
                    diagnostic_code: "settled_bar_missing".into(),
                };
                attempt.error_detail_json = Some(canonical_json(&detail).unwrap());
                attempt.error_detail_hash = Some(sha256_json(&detail).unwrap());
                let fingerprint = OutcomeErrorFingerprintPreimageV2 {
                    domain: DOMAIN_ERROR_FINGERPRINT.into(),
                    failed_stage: detail.operation,
                    reason_code: OutcomeReasonCodeV1::SettledBarMissing.as_str().into(),
                    retryable: true,
                    available_evidence_hash: attempt.available_evidence_hash.clone(),
                    detail_hash: attempt.error_detail_hash.clone().unwrap(),
                    transport_attempts_hash: attempt.transport_attempts_hash.clone().unwrap(),
                };
                attempt.error_fingerprint = Some(sha256_json(&fingerprint).unwrap());
            }
        }
        attempt
    }

    fn outcome_stage(
        attempt: SelectionOutcomeAttemptRowContentPreimage,
        outcome_rows: Vec<SelectionSampleOutcomeRowContentPreimage>,
        status: RunStatus,
    ) -> OutcomeStageInputPreimage {
        let config_hash = hash('5');
        let logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::OutcomeRun,
            source_fact_key: None,
            config_hash: Some(config_hash.clone()),
            sample_key: Some(attempt.sample_key.clone()),
            outcome_phase: Some(attempt.phase),
            stored_due_date: Some(attempt.stored_due_date.clone()),
            ingress_source_batch_hash: None,
        })
        .unwrap();
        OutcomeStageInputPreimage {
            domain: DOMAIN_OUTCOME_STAGE.into(),
            stage_run_id: attempt.outcome_run_id.clone(),
            logical_subject_key,
            config_activation_run_id: "01900000-0000-7000-8000-000000000002".into(),
            config_hash,
            outcome_claim_id: "01900000-0000-7000-8000-000000000003".into(),
            outcome_claim_receipt_content_hash: hash('6'),
            outcome_claim_due_binding_hash: hash('7'),
            outcome_claim_provider_request_hash: hash('8'),
            sample_key_preimage: outcome_sample_key_preimage(),
            sample_key: attempt.sample_key.clone(),
            outcome_phase: attempt.phase,
            stored_due_date: attempt.stored_due_date.clone(),
            outcome_attempt_rows: vec![attempt],
            outcome_rows,
            planned_run_status: status,
        }
    }

    fn expected_wait_stage() -> OutcomeStageInputPreimage {
        let mut stage = outcome_stage(
            outcome_attempt(OutcomeAttemptResult::ExpectedWait),
            Vec::new(),
            RunStatus::ExpectedWait,
        );
        stage.outcome_attempt_rows.clear();
        stage
    }

    fn direct_relation_row(
        relation_attempt_id: char,
    ) -> SelectionRelationAttemptRowContentPreimage {
        let source = DirectMentionSourcePreimage {
            domain: DOMAIN_DIRECT_SOURCE.into(),
            source_fact_key: hash('3'),
            field: DirectMentionField::Title,
            mention_kind: MentionKind::ExactCode,
            normalized_value: "TEST_CODE_000001".into(),
            byte_start: 0,
            byte_end: 16,
        };
        let binding = BindingStatePreimage {
            domain: DOMAIN_BINDING_STATE.into(),
            state: BindingStateKind::DirectNotApplicable,
            artifact_content_hash: None,
            binding_audit_hash: None,
            provider: None,
            kind: None,
            code: None,
            name: None,
            error_fingerprint: None,
        };
        let raw_identity = RawSecurityIdentityPreimage {
            domain: DOMAIN_RAW_SECURITY_IDENTITY.into(),
            provider: "source-event".into(),
            exchange: "SZ".into(),
            code: "TEST_CODE_000001".into(),
            asset_class: "equity".into(),
        };
        SelectionRelationAttemptRowContentPreimage {
            domain: DOMAIN_RELATION_ATTEMPT_ROW.into(),
            relation_attempt_id: hash(relation_attempt_id),
            relation_key: hash(relation_attempt_id),
            generation_run_id: "01900000-0000-7000-8000-000000000001".into(),
            source_fact_key: hash('3'),
            event_id: hash('4'),
            chain_id: "TEST_CODE_CHAIN".into(),
            config_activation_run_id: "01900000-0000-7000-8000-000000000002".into(),
            config_hash: hash('5'),
            relation_schema_version: "relation-v1".into(),
            relation_kind: RelationKind::DirectMention,
            relation_source_identity_json: canonical_json(&source).unwrap(),
            relation_source_identity_hash: sha256_json(&source).unwrap(),
            typed_binding_state_json: canonical_json(&binding).unwrap(),
            typed_binding_state_hash: sha256_json(&binding).unwrap(),
            request_hash: None,
            request_evidence_json: None,
            request_evidence_hash: None,
            result_code: "resolved".into(),
            failed_stage: None,
            retryable: None,
            raw_identity_json: Some(canonical_json(&raw_identity).unwrap()),
            raw_identity_hash: Some(sha256_json(&raw_identity).unwrap()),
            canonical_stock_code: Some("TEST_CODE_000001".into()),
            canonical_stock_name: Some("TEST_CODE_NAME".into()),
            canonical_market: Some("SZ".into()),
            artifact_content_hash: None,
            binding_audit_hash: None,
            provider_board_kind: None,
            provider_board_code: None,
            provider_board_name: None,
            provider_source: None,
            provider_source_at: None,
            provider_observed_at: None,
            provider_batch_id: None,
            provider_batch_content_hash: None,
            actual_constituent_count: None,
            available_evidence_json: None,
            available_evidence_hash: None,
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            attempted_at: "2026-07-28T07:00:00.000000000Z".into(),
        }
    }

    fn admitted_sample_row(
        relation: &SelectionRelationAttemptRowContentPreimage,
    ) -> SelectionSampleRowContentPreimage {
        let relation_evidence = RelationEvidenceSetPreimage {
            domain: DOMAIN_RELATION_EVIDENCE_SET.into(),
            source_fact_key: relation.source_fact_key.clone(),
            event_id: relation.event_id.clone(),
            chain_id: relation.chain_id.clone(),
            canonical_stock_code: relation.canonical_stock_code.clone().unwrap(),
            entries_in_relation_order: vec![RelationEvidenceEntryPreimage {
                relation_rank: 0,
                relation_key: relation.relation_key.clone(),
                relation_kind: relation.relation_kind,
                relation_attempt_id: relation.relation_attempt_id.clone(),
                relation_attempt_content_hash: sha256_json(relation).unwrap(),
            }],
        };
        let feature = T0FeaturePreimage {
            domain: DOMAIN_T0_FEATURE.into(),
            feature_version: "feature-v1".into(),
            evaluation_window: EvaluationWindow::PostClose,
            ma5: Some("10".into()),
            ma10: Some("9.8".into()),
            ma20: Some("9.5".into()),
            five_day_return: Some("0.05".into()),
            volume_vs_5d: Some("1.2".into()),
            volume_vs_20d: Some("1.1".into()),
            intraday_volume_pace: None,
            price_vs_ma5: Some("1.05".into()),
            price_vs_ma10: Some("1.07".into()),
            price_vs_ma20: Some("1.1".into()),
            evaluation_price: "10.5".into(),
            observed_volume: "1000".into(),
            latest_settled_market_date: "2026-07-28".into(),
            latest_settled_close: "10.5".into(),
            latest_settled_volume: "1000".into(),
            prior_5d_average_volume: "900".into(),
            prior_20d_average_volume: "850".into(),
        };
        SelectionSampleRowContentPreimage {
            domain: DOMAIN_SAMPLE_ROW.into(),
            sample_key: hash('b'),
            generation_run_id: relation.generation_run_id.clone(),
            source_fact_key: relation.source_fact_key.clone(),
            source_fact_content_hash: hash('6'),
            source_fact_attempt_id: hash('7'),
            source_batch_attempt_id: hash('8'),
            event_id: relation.event_id.clone(),
            chain_id: relation.chain_id.clone(),
            config_activation_run_id: relation.config_activation_run_id.clone(),
            config_hash: relation.config_hash.clone(),
            matched_keyword: "TEST_CODE".into(),
            canonical_stock_code: relation.canonical_stock_code.clone().unwrap(),
            canonical_stock_name: relation.canonical_stock_name.clone().unwrap(),
            canonical_market: relation.canonical_market.clone().unwrap(),
            relation_schema_version: relation.relation_schema_version.clone(),
            relation_evidence_json: canonical_json(&relation_evidence).unwrap(),
            relation_evidence_set_hash: sha256_json(&relation_evidence).unwrap(),
            feature_version: feature.feature_version.clone(),
            t0_feature_json: canonical_json(&feature).unwrap(),
            t0_feature_hash: sha256_json(&feature).unwrap(),
            market_provider: "magic-tdx".into(),
            market_source: "tdx".into(),
            market_source_at: Some("2026-07-28T06:59:00.000000000Z".into()),
            market_observed_at: "2026-07-28T07:00:00.000000000Z".into(),
            market_batch_id: "TEST_CODE_BATCH".into(),
            market_batch_content_hash: hash('9'),
            admission_version: "admission-v1".into(),
            decision_kind: TerminalDecisionKind::Admitted,
            rejection_count: 0,
            rejection_row_hashes_in_ordinal_order: vec![],
            evaluation_market_date: "2026-07-28".into(),
            t0_due_date: "2026-07-28".into(),
            d1_due_date: "2026-07-29".into(),
            d2_due_date: "2026-07-30".into(),
            d3_due_date: "2026-07-31".into(),
            d4_due_date: "2026-08-03".into(),
            d5_due_date: "2026-08-04".into(),
            calendar_version: "calendar-v1".into(),
            calendar_hash: hash('a'),
            trading_date_vector_json: canonical_json(&outcome_trading_date_vector()).unwrap(),
            trading_date_vector_hash: sha256_json(&outcome_trading_date_vector()).unwrap(),
            staged_at: "2026-07-28T07:00:00.000000000Z".into(),
        }
    }

    fn completed_evaluation_row(
        sample: &SelectionSampleRowContentPreimage,
    ) -> SelectionEvaluationAttemptRowContentPreimage {
        let request = t0_request_columns(sample);
        let evidence = ProviderAvailableEvidencePreimage {
            domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
            evidence_kind: ProviderEvidenceKind::T0MarketBundle,
            provider: "magic-tdx".into(),
            source: Some("tdx".into()),
            source_at: sample.market_source_at.clone(),
            observed_at: Some(sample.market_observed_at.clone()),
            batch_id: Some(sample.market_batch_id.clone()),
            batch_content_hash: Some(sample.market_batch_content_hash.clone()),
        };
        SelectionEvaluationAttemptRowContentPreimage {
            domain: DOMAIN_EVALUATION_ATTEMPT_ROW.into(),
            evaluation_attempt_id: hash('c'),
            sample_key: sample.sample_key.clone(),
            generation_run_id: sample.generation_run_id.clone(),
            source_fact_key: sample.source_fact_key.clone(),
            event_id: sample.event_id.clone(),
            chain_id: sample.chain_id.clone(),
            canonical_stock_code: sample.canonical_stock_code.clone(),
            canonical_stock_name: sample.canonical_stock_name.clone(),
            canonical_market: sample.canonical_market.clone(),
            relation_evidence_set_hash: sample.relation_evidence_set_hash.clone(),
            market_request_hash: request.request_hash,
            request_evidence_json: request.request_evidence_json,
            request_evidence_hash: request.request_evidence_hash,
            result_code: "completed".into(),
            failed_stage: None,
            retryable: None,
            provider: Some("magic-tdx".into()),
            source: Some("tdx".into()),
            source_at: sample.market_source_at.clone(),
            observed_at: Some(sample.market_observed_at.clone()),
            batch_id: Some(sample.market_batch_id.clone()),
            batch_content_hash: Some(sample.market_batch_content_hash.clone()),
            available_evidence_json: Some(canonical_json(&evidence).unwrap()),
            available_evidence_hash: Some(sha256_json(&evidence).unwrap()),
            terminal_decision_hash: Some(sha256_json(sample).unwrap()),
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            attempted_at: "2026-07-28T07:00:00.000000000Z".into(),
        }
    }

    fn valid_completed_generation_stage() -> GenerationStageInputPreimage {
        let relation = direct_relation_row('1');
        let sample = admitted_sample_row(&relation);
        let evaluation = completed_evaluation_row(&sample);
        let logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::GenerationRun,
            source_fact_key: Some(relation.source_fact_key.clone()),
            config_hash: Some(relation.config_hash.clone()),
            sample_key: None,
            outcome_phase: None,
            stored_due_date: None,
            ingress_source_batch_hash: None,
        })
        .unwrap();
        GenerationStageInputPreimage {
            domain: DOMAIN_GENERATION_STAGE.into(),
            stage_run_id: relation.generation_run_id.clone(),
            logical_subject_key,
            source_fact_key: relation.source_fact_key.clone(),
            source_fact_content_hash: sample.source_fact_content_hash.clone(),
            config_activation_run_id: relation.config_activation_run_id.clone(),
            config_hash: relation.config_hash.clone(),
            generation_market_date: "2026-07-28".into(),
            relation_attempt_rows: vec![relation],
            evaluation_attempt_rows: vec![evaluation],
            sample_rows: vec![sample],
            rejection_rows: vec![],
            planned_run_status: RunStatus::Completed,
        }
    }

    fn failed_direct_relation(retryable: bool) -> SelectionRelationAttemptRowContentPreimage {
        let mut relation = direct_relation_row('1');
        let detail = ProviderErrorDetailPreimage {
            domain: DOMAIN_PROVIDER_ERROR_DETAIL.into(),
            error_kind: ProviderErrorKind::InvalidData,
            provider: "source-event".into(),
            operation: "relation_resolution".into(),
            error_code: Some("TEST_CODE_FAILURE".into()),
            http_status: None,
            timeout_ms: None,
            invariant_id: None,
            diagnostic_code: "relation_failure".into(),
        };
        relation.result_code = "rejected".into();
        relation.failed_stage = Some("relation_resolution".into());
        relation.retryable = Some(retryable);
        relation.error_detail_json = Some(canonical_json(&detail).unwrap());
        relation.error_detail_hash = Some(sha256_json(&detail).unwrap());
        relation.error_fingerprint = Some(
            sha256_json(&ErrorFingerprintPreimage {
                domain: DOMAIN_ERROR_FINGERPRINT.into(),
                failed_stage: "relation_resolution".into(),
                reason_code: "relation_failure".into(),
                retryable,
                available_evidence_hash: None,
                detail_hash: relation.error_detail_hash.clone().unwrap(),
            })
            .unwrap(),
        );
        relation
    }

    fn hard_rejected_generation_stage() -> GenerationStageInputPreimage {
        let mut stage = valid_completed_generation_stage();
        let sample_key = stage.sample_rows[0].sample_key.clone();
        let detail = AdmissionStructuredDetailPreimage::PriceBelowMa5 {
            value: "0.9".into(),
            inclusive_min: "1".into(),
        };
        let rejection = SelectionRejectionRowContentPreimage {
            domain: DOMAIN_REJECTION_ROW.into(),
            sample_key,
            ordinal: 0,
            generation_run_id: stage.stage_run_id.clone(),
            reason_code: "price_below_ma5".into(),
            rule_id: "BR-178".into(),
            retryable: false,
            structured_detail_json: canonical_json(&detail).unwrap(),
            structured_detail_hash: sha256_json(&detail).unwrap(),
            provider: None,
            source: None,
            source_at: None,
            observed_at: None,
            batch_id: None,
            batch_content_hash: None,
            created_at: "2026-07-28T07:00:00.000000000Z".into(),
        };
        stage.sample_rows[0].decision_kind = TerminalDecisionKind::HardRejected;
        stage.sample_rows[0].rejection_count = 1;
        stage.sample_rows[0].rejection_row_hashes_in_ordinal_order =
            vec![sha256_json(&rejection).unwrap()];
        stage.evaluation_attempt_rows[0].terminal_decision_hash =
            Some(sha256_json(&stage.sample_rows[0]).unwrap());
        stage.rejection_rows = vec![rejection];
        stage
    }

    #[test]
    fn generation_stage_accepts_the_closed_completed_matrix() {
        valid_completed_generation_stage()
            .validate()
            .expect("closed generation stage");
    }

    #[test]
    fn generation_stage_rejects_domain_status_identity_and_ordering_matrix() {
        let mut cases = Vec::new();

        let mut stage = valid_completed_generation_stage();
        stage.domain = "wrong-domain".into();
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        stage.planned_run_status = RunStatus::Settled;
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        stage.generation_market_date = "2026-7-28".into();
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        stage.relation_attempt_rows[0].source_fact_key = hash('f');
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        let mut second = direct_relation_row('0');
        second.relation_key = hash('0');
        stage.relation_attempt_rows.push(second);
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        stage
            .relation_attempt_rows
            .push(stage.relation_attempt_rows[0].clone());
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        stage
            .evaluation_attempt_rows
            .push(stage.evaluation_attempt_rows[0].clone());
        cases.push(stage);

        let mut stage = valid_completed_generation_stage();
        stage.sample_rows.push(stage.sample_rows[0].clone());
        cases.push(stage);

        let mut stage = hard_rejected_generation_stage();
        stage.rejection_rows.push(stage.rejection_rows[0].clone());
        cases.push(stage);

        for stage in cases {
            stage.validate().expect_err("matrix violation must fail");
        }
    }

    #[test]
    fn generation_stage_accepts_all_four_closed_status_and_terminal_matrices() {
        let source_fact_key = hash('3');
        let config_hash = hash('5');
        GenerationStageInputPreimage {
            domain: DOMAIN_GENERATION_STAGE.into(),
            stage_run_id: "01900000-0000-7000-8000-000000000001".into(),
            logical_subject_key: run_logical_subject_key(&RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::GenerationRun,
                source_fact_key: Some(source_fact_key.clone()),
                config_hash: Some(config_hash.clone()),
                sample_key: None,
                outcome_phase: None,
                stored_due_date: None,
                ingress_source_batch_hash: None,
            })
            .unwrap(),
            source_fact_key,
            source_fact_content_hash: hash('6'),
            config_activation_run_id: "01900000-0000-7000-8000-000000000002".into(),
            config_hash,
            generation_market_date: "2026-07-28".into(),
            relation_attempt_rows: vec![],
            evaluation_attempt_rows: vec![],
            sample_rows: vec![],
            rejection_rows: vec![],
            planned_run_status: RunStatus::VerifiedNoRelation,
        }
        .validate()
        .expect("verified-no-relation is a closed empty generation terminal");

        GenerationStageInputPreimage {
            relation_attempt_rows: vec![failed_direct_relation(true)],
            evaluation_attempt_rows: vec![],
            sample_rows: vec![],
            rejection_rows: vec![],
            planned_run_status: RunStatus::PendingDependency,
            ..valid_completed_generation_stage()
        }
        .validate()
        .expect("retryable relation failure is pending dependency");

        GenerationStageInputPreimage {
            relation_attempt_rows: vec![failed_direct_relation(false)],
            evaluation_attempt_rows: vec![],
            sample_rows: vec![],
            rejection_rows: vec![],
            planned_run_status: RunStatus::FailedNonRetryable,
            ..valid_completed_generation_stage()
        }
        .validate()
        .expect("non-retryable relation failure is terminal");

        hard_rejected_generation_stage()
            .validate()
            .expect("hard-rejected sample binds a contiguous rejection hash sequence");
    }

    #[test]
    fn generation_stage_rejects_each_row_domain() {
        let mut relation = valid_completed_generation_stage();
        relation.relation_attempt_rows[0].domain = "wrong-domain".into();
        relation
            .validate()
            .expect_err("relation row domain must be exact");

        let mut evaluation = valid_completed_generation_stage();
        evaluation.evaluation_attempt_rows[0].domain = "wrong-domain".into();
        evaluation
            .validate()
            .expect_err("evaluation row domain must be exact");

        let mut sample = valid_completed_generation_stage();
        sample.sample_rows[0].domain = "wrong-domain".into();
        sample
            .validate()
            .expect_err("sample row domain must be exact");

        let mut rejection = hard_rejected_generation_stage();
        rejection.rejection_rows[0].domain = "wrong-domain".into();
        rejection
            .validate()
            .expect_err("rejection row domain must be exact");
    }

    #[test]
    fn generation_stage_rejects_relation_evaluation_and_terminal_kind_matrices() {
        let mut direct_with_request = valid_completed_generation_stage();
        direct_with_request.relation_attempt_rows[0].request_hash = Some(hash('f'));
        direct_with_request
            .validate()
            .expect_err("direct mention has no provider request");

        let mut resolved_with_retryability = valid_completed_generation_stage();
        resolved_with_retryability.relation_attempt_rows[0].retryable = Some(false);
        resolved_with_retryability
            .validate()
            .expect_err("resolved relation retryability is NULL");

        let mut completed_with_retryability = valid_completed_generation_stage();
        completed_with_retryability.evaluation_attempt_rows[0].retryable = Some(false);
        completed_with_retryability
            .validate()
            .expect_err("completed evaluation retryability is NULL");

        let mut admitted_with_rejection = hard_rejected_generation_stage();
        admitted_with_rejection.sample_rows[0].decision_kind = TerminalDecisionKind::Admitted;
        admitted_with_rejection.sample_rows[0].rejection_count = 0;
        admitted_with_rejection.sample_rows[0].rejection_row_hashes_in_ordinal_order = vec![];
        admitted_with_rejection.evaluation_attempt_rows[0].terminal_decision_hash =
            Some(sha256_json(&admitted_with_rejection.sample_rows[0]).unwrap());
        admitted_with_rejection
            .validate()
            .expect_err("admitted samples cannot carry rejection rows");
    }

    #[test]
    fn board_binding_proposal_has_fixed_canonical_json_and_hash() {
        let proposal = BoardBindingProposalInputPreimage {
            domain: DOMAIN_BOARD_BINDING_PROPOSAL.into(),
            schema_version: BOARD_BINDING_PROPOSAL_SCHEMA_VERSION.into(),
            validity_policy_version: BOARD_BINDING_VALIDITY_POLICY_VERSION.into(),
            valid_from_rfc3339_nanos_utc: "2026-07-28T08:05:00.000000000Z".into(),
            expires_at_rfc3339_nanos_utc: "2026-08-27T08:05:00.000000000Z".into(),
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
        let expected_json = concat!(
            "{\"domain\":\"stock_analysis.br174.board_binding_proposal.v1\",",
            "\"schema_version\":\"selection-provider-board-binding-proposal-v1\",",
            "\"validity_policy_version\":\"selection-board-binding-validity-v1\",",
            "\"valid_from_rfc3339_nanos_utc\":\"2026-07-28T08:05:00.000000000Z\",",
            "\"expires_at_rfc3339_nanos_utc\":\"2026-08-27T08:05:00.000000000Z\",",
            "\"reviewed_by\":\"TEST_CODE_REVIEWER\",",
            "\"reviewed_at_rfc3339_nanos_utc\":\"2026-07-28T08:00:00.000000000Z\",",
            "\"bindings_sorted\":[{\"chain_id\":\"TEST_CODE_CHAIN\",\"provider\":\"tdx\",",
            "\"kind\":\"concept\",\"code\":\"tdx:concept:TEST_CODE_BOARD\",",
            "\"name\":\"TEST_CODE_BOARD\"}]}"
        );
        assert_eq!(canonical_json(&proposal).unwrap(), expected_json);
        assert_eq!(
            proposal.proposal_input_content_hash().unwrap(),
            "34e99780bfd220516878de4cc84e9543dd15c4dff7becf6eb107e468647329b5"
        );
    }

    #[test]
    fn board_connection_and_directory_record_have_fixed_null_encoding_and_hashes() {
        let policy = BoardConnectionPolicyPreimage::fixed();
        assert_eq!(
            canonical_json(&policy).unwrap(),
            concat!(
                "{\"domain\":\"stock_analysis.br174.board_connection_policy.v1\",",
                "\"version\":\"selection-board-tdx-production-v1\",\"provider\":\"tdx\",",
                "\"gateway_constructor\":\"BoardDataGateway::production_tdx\",",
                "\"resolver_policy\":\"magic_tdx_production_resolver_v1\",",
                "\"endpoint_override\":\"forbidden\"}"
            )
        );
        assert_eq!(
            policy.connection_policy_hash().unwrap(),
            TEST_CONNECTION_POLICY_HASH
        );

        let record = board_record(
            ProviderBoardKind::Concept,
            "TEST_CODE_BOARD",
            42,
            "unix-ms:1785225900000",
            "TEST_CODE_CONCEPT_BATCH",
        );
        assert_eq!(
            canonical_json(&record).unwrap(),
            concat!(
                "{\"domain\":\"stock_analysis.br174.board_directory_record.v1\",",
                "\"provider_ordinal\":0,\"code\":\"tdx:concept:TEST_CODE_BOARD\",",
                "\"name\":\"TEST_CODE_BOARD\",\"kind\":\"concept\",\"member_count\":42,",
                "\"evidence\":{\"provider\":\"tdx\",\"source\":\"tdx-block-files\",",
                "\"source_at\":null,\"observed_at\":\"unix-ms:1785225900000\",",
                "\"batch_id\":\"TEST_CODE_CONCEPT_BATCH\"}}"
            )
        );
        assert_eq!(
            record.directory_record_hash().unwrap(),
            TEST_CONCEPT_RECORD_HASH
        );
    }

    #[test]
    fn complete_board_artifact_has_frozen_hash_and_exact_nested_evidence() {
        let artifact = verified_board_artifact();
        artifact.validate().unwrap();
        assert_eq!(
            artifact.artifact_content_hash().unwrap(),
            "f48cd7c51daeb0b15632e3dd977727203d7ef83ff6854b445d0229939edb608d"
        );
        let json = canonical_json(&artifact).unwrap();
        assert!(json.contains("\"provider_endpoint_evidence\":null"));
        assert!(json.contains("\"source_at\":null"));
        assert!(json.find("\"category\":\"concept\"") < json.find("\"category\":\"industry\""));
    }

    #[test]
    fn live_audited_empty_binding_artifact_is_valid_for_gate_b_and_c() {
        let mut artifact = verified_board_artifact();
        artifact.proposal_input.bindings_sorted.clear();
        artifact.bindings_sorted.clear();
        artifact.proposal_input_content_hash = artifact
            .proposal_input
            .proposal_input_content_hash()
            .unwrap();
        artifact.audit_attestation_receipt.audit_subject_id = BoardAuditSubjectPreimage {
            domain: DOMAIN_BOARD_AUDIT_SUBJECT.into(),
            proposal_input_content_hash: artifact.proposal_input_content_hash.clone(),
            audit_command_version: artifact.audit_command_version.clone(),
            connection_policy_hash: artifact.connection_policy_hash.clone(),
        }
        .audit_subject_id()
        .unwrap();
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
            .unwrap();
        artifact.audit_attestation_receipt_hash = artifact
            .audit_attestation_receipt
            .audit_attestation_receipt_hash()
            .unwrap();

        artifact.validate().unwrap();
    }

    #[test]
    fn artifact_rejects_record_provider_drift_and_noncanonical_observed_time() {
        let mut wrong_provider = verified_board_artifact();
        wrong_provider.directory_batches_by_category[0]
            .content
            .records_in_provider_order[0]
            .evidence
            .provider = "TEST_CODE_OTHER_PROVIDER".into();
        assert_eq!(
            wrong_provider.validate().unwrap_err().code,
            "directory_record_evidence_mismatch"
        );

        let mut leading_zero = verified_board_artifact();
        leading_zero.directory_batches_by_category[0]
            .content
            .observed_at = "unix-ms:01785225900000".into();
        assert_eq!(
            leading_zero.validate().unwrap_err().code,
            "invalid_board_observed_at"
        );
    }

    #[test]
    fn artifact_enforces_300_second_freshness_and_exact_one_binding() {
        verified_board_artifact().validate().unwrap();

        let mut stale = verified_board_artifact();
        stale.directory_batches_by_category[0].content.observed_at = "unix-ms:1785225899999".into();
        assert_eq!(
            stale.validate().unwrap_err().code,
            "directory_batch_freshness_invalid"
        );

        let mut record_hash_drift = verified_board_artifact();
        record_hash_drift.bindings_sorted[0].directory_record_hash = hash('c');
        assert_eq!(
            record_hash_drift.validate().unwrap_err().code,
            "binding_directory_evidence_mismatch"
        );
    }

    #[test]
    fn source_fact_key_has_fixed_canonical_json_and_hash() {
        let value = SourceFactKeyPreimage {
            domain: DOMAIN_SOURCE_FACT_KEY.into(),
            provider_source: "eastmoney".into(),
            item_id: "news-42".into(),
        };
        let expected_json = concat!(
            "{\"domain\":\"stock_analysis.br174.source_fact_key.v1\",",
            "\"provider_source\":\"eastmoney\",\"item_id\":\"news-42\"}"
        );
        assert_eq!(canonical_json(&value).unwrap(), expected_json);
        assert_eq!(
            sha256_json(&value).unwrap(),
            "e748126e294ad40a2eacc66b5c3afd616789077e48c3df540433cc2a4fb7790d"
        );
        let with_unknown = expected_json.replacen("\"item_id\"", "\"unknown\":true,\"item_id\"", 1);
        assert!(serde_json::from_str::<SourceFactKeyPreimage>(&with_unknown).is_err());
    }

    #[test]
    fn generation_subject_has_fixed_null_encoding_and_hash() {
        let subject = RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::GenerationRun,
            source_fact_key: Some(hash('1')),
            config_hash: Some(hash('2')),
            sample_key: None,
            outcome_phase: None,
            stored_due_date: None,
            ingress_source_batch_hash: None,
        };
        subject.validate().unwrap();
        let expected_json = concat!(
            "{\"domain\":\"stock_analysis.br174.run_logical_subject.v1\",",
            "\"subject_kind\":\"generation_run\",",
            "\"source_fact_key\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"config_hash\":\"2222222222222222222222222222222222222222222222222222222222222222\",",
            "\"sample_key\":null,\"outcome_phase\":null,\"stored_due_date\":null,",
            "\"ingress_source_batch_hash\":null}"
        );
        assert_eq!(canonical_json(&subject).unwrap(), expected_json);
        assert_eq!(
            sha256_json(&subject).unwrap(),
            "c899e73d79cb08e3b67cfbd9d05938e29652a5b7e7ebfd936901c9c33f74b538"
        );
    }

    #[test]
    fn acquisition_content_hashes_exclude_local_attempt_identity() {
        let record = FeedSourceRecordHashPreimage {
            domain: DOMAIN_FEED_SOURCE_RECORD.into(),
            provider_ordinal: 0,
            source_fact_key: hash('1'),
            provider_content_hash: hash('2'),
        };
        let source_content_hash =
            feed_source_content_hash(&hash('3'), &hash('4'), &[record]).unwrap();
        let feed_content = FeedAttemptContentPreimage {
            domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
            feed_identity: hash('3'),
            request_hash: hash('5'),
            request_evidence_hash: hash('6'),
            status_kind: FeedStatusKind::Available,
            record_count: Some(1),
            evidence_hash: Some(hash('4')),
            source_content_hash: Some(source_content_hash),
            available_evidence_hash: Some(hash('4')),
            failed_stage: None,
            reason_code: None,
            retryable: None,
            detail_hash: None,
            error_fingerprint: None,
        };
        feed_content.validate().unwrap();
        let stable_feed_hash = sha256_json(&feed_content).unwrap();
        let batch_one = source_batch_content_hash(
            &hash('6'),
            vec![stable_feed_hash.clone()],
            vec![hash('7')],
            vec![hash('8')],
            "2026-07-28T07:00:00.000000000Z".into(),
        )
        .unwrap();
        let batch_two = source_batch_content_hash(
            &hash('6'),
            vec![stable_feed_hash],
            vec![hash('7')],
            vec![hash('8')],
            "2026-07-28T07:00:00.000000000Z".into(),
        )
        .unwrap();
        assert_eq!(batch_one, batch_two);

        let key_one = FeedAttemptKeyPreimage {
            domain: DOMAIN_FEED_ATTEMPT_KEY.into(),
            ingress_run_id: "01900000-0000-7000-8000-000000000001".into(),
            feed_identity: hash('3'),
        };
        let key_two = FeedAttemptKeyPreimage {
            ingress_run_id: "01900000-0000-7000-8000-000000000002".into(),
            ..key_one.clone()
        };
        assert_ne!(
            sha256_json(&key_one).unwrap(),
            sha256_json(&key_two).unwrap()
        );
    }

    #[test]
    fn source_attempt_record_count_preserves_the_closed_null_matrix() {
        let available = FeedAttemptContentPreimage {
            domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
            feed_identity: hash('1'),
            request_hash: hash('2'),
            request_evidence_hash: hash('6'),
            status_kind: FeedStatusKind::Available,
            record_count: Some(1),
            evidence_hash: Some(hash('3')),
            source_content_hash: Some(hash('4')),
            available_evidence_hash: Some(hash('3')),
            failed_stage: None,
            reason_code: None,
            retryable: None,
            detail_hash: None,
            error_fingerprint: None,
        };
        available.validate().expect("available has Some(positive)");
        FeedAttemptContentPreimage {
            record_count: None,
            ..available.clone()
        }
        .validate()
        .expect_err("available cannot erase its observed count");
        FeedAttemptContentPreimage {
            record_count: Some(0),
            ..available
        }
        .validate()
        .expect_err("available cannot encode an empty batch");

        let verified_empty = FeedAttemptContentPreimage {
            domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
            feed_identity: hash('1'),
            request_hash: hash('2'),
            request_evidence_hash: hash('6'),
            status_kind: FeedStatusKind::VerifiedEmpty,
            record_count: Some(0),
            evidence_hash: Some(hash('3')),
            source_content_hash: Some(hash('4')),
            available_evidence_hash: Some(hash('3')),
            failed_stage: None,
            reason_code: None,
            retryable: None,
            detail_hash: None,
            error_fingerprint: None,
        };
        verified_empty
            .validate()
            .expect("verified-empty has explicit Some(0)");
        FeedAttemptContentPreimage {
            record_count: None,
            ..verified_empty
        }
        .validate()
        .expect_err("verified-empty cannot collapse into unavailable NULL");

        let unavailable = FeedAttemptContentPreimage {
            domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
            feed_identity: hash('1'),
            request_hash: hash('2'),
            request_evidence_hash: hash('6'),
            status_kind: FeedStatusKind::Unavailable,
            record_count: None,
            evidence_hash: None,
            source_content_hash: None,
            available_evidence_hash: None,
            failed_stage: Some("provider_fetch".into()),
            reason_code: Some("provider_unavailable".into()),
            retryable: Some(true),
            detail_hash: Some(hash('5')),
            error_fingerprint: Some(hash('6')),
        };
        unavailable
            .validate()
            .expect("unavailable preserves unknown count as NULL");
        FeedAttemptContentPreimage {
            record_count: Some(0),
            ..unavailable
        }
        .validate()
        .expect_err("unavailable must not synthesize a zero count");
    }

    #[test]
    fn binding_state_required_null_matrix_is_closed() {
        for state in [
            BindingStateKind::DirectNotApplicable,
            BindingStateKind::NotConfigured,
        ] {
            BindingStatePreimage {
                domain: DOMAIN_BINDING_STATE.into(),
                state,
                artifact_content_hash: None,
                binding_audit_hash: None,
                provider: None,
                kind: None,
                code: None,
                name: None,
                error_fingerprint: None,
            }
            .validate()
            .unwrap();
        }
        let verified = BindingStatePreimage {
            domain: DOMAIN_BINDING_STATE.into(),
            state: BindingStateKind::Verified,
            artifact_content_hash: Some(hash('1')),
            binding_audit_hash: Some(hash('2')),
            provider: Some("magic-tdx".into()),
            kind: Some(ProviderBoardKind::Concept),
            code: Some("BK0001".into()),
            name: Some("样例板块".into()),
            error_fingerprint: None,
        };
        verified.validate().unwrap();
        assert!(BindingStatePreimage {
            name: None,
            ..verified
        }
        .validate()
        .is_err());
    }

    #[test]
    fn outcome_four_state_matrix_and_cardinality_are_closed() {
        outcome_stage(
            outcome_attempt(OutcomeAttemptResult::Settled),
            vec![outcome_row()],
            RunStatus::Settled,
        )
        .validate()
        .unwrap();
        expected_wait_stage().validate().unwrap();
        outcome_stage(
            outcome_attempt(OutcomeAttemptResult::Error),
            vec![],
            RunStatus::FailedRetryable,
        )
        .validate()
        .unwrap();

        let mut non_retryable = outcome_attempt(OutcomeAttemptResult::Error);
        non_retryable.retryable = Some(false);
        let detail: ProviderErrorDetailPreimage =
            serde_json::from_str(non_retryable.error_detail_json.as_deref().unwrap()).unwrap();
        non_retryable.error_fingerprint = Some(
            sha256_json(&OutcomeErrorFingerprintPreimageV2 {
                domain: DOMAIN_ERROR_FINGERPRINT.into(),
                failed_stage: detail.operation,
                reason_code: OutcomeReasonCodeV1::SettledBarMissing.as_str().into(),
                retryable: false,
                available_evidence_hash: non_retryable.available_evidence_hash.clone(),
                detail_hash: non_retryable.error_detail_hash.clone().unwrap(),
                transport_attempts_hash: non_retryable.transport_attempts_hash.clone().unwrap(),
            })
            .unwrap(),
        );
        outcome_stage(non_retryable, vec![], RunStatus::FailedNonRetryable)
            .validate()
            .unwrap();

        let wrong_status = outcome_stage(
            outcome_attempt(OutcomeAttemptResult::ExpectedWait),
            vec![],
            RunStatus::FailedRetryable,
        );
        assert!(wrong_status.validate().is_err());
        let unexpected_row = outcome_stage(
            outcome_attempt(OutcomeAttemptResult::Error),
            vec![outcome_row()],
            RunStatus::FailedRetryable,
        );
        assert!(unexpected_row.validate().is_err());
    }

    #[test]
    fn expected_wait_has_no_provider_attempt_or_outcome_row() {
        let mut stage = expected_wait_stage();

        stage
            .validate()
            .expect("ExpectedWait is a receipted run with no provider attempt");
        assert_eq!(stage.expected_staged_row_count(), 1);

        stage.outcome_attempt_rows = vec![outcome_attempt(OutcomeAttemptResult::ExpectedWait)];
        stage
            .validate()
            .expect_err("ExpectedWait must not fabricate an attempt row");
    }

    #[test]
    fn outcome_claim_is_the_fifth_closed_subject_kind() {
        assert_eq!(
            canonical_json(&SubjectKind::OutcomeClaim).unwrap(),
            r#""outcome_claim""#
        );
        let decoded: SubjectKind = serde_json::from_str(r#""outcome_claim""#).unwrap();
        assert_eq!(decoded, SubjectKind::OutcomeClaim);
        serde_json::from_str::<SubjectKind>(r#""outcome_claim_v2""#)
            .expect_err("unknown claim subject token must fail closed");
    }

    #[test]
    fn outcome_claim_binding_validation_is_scoped_to_outcome_stage() {
        valid_empty_source_ingress_stage()
            .validate()
            .expect("ingress validation must remain claim-independent");

        let mut outcome = expected_wait_stage();
        outcome.outcome_claim_id = outcome.stage_run_id.clone();
        assert_eq!(
            outcome.validate().unwrap_err().code,
            "outcome_claim_run_identity_collision"
        );
    }

    #[test]
    fn outcome_stage_rejects_stage_row_domain_and_numeric_matrix() {
        let stage_for = |outcome: SelectionSampleOutcomeRowContentPreimage| {
            let mut attempt = outcome_attempt(OutcomeAttemptResult::Settled);
            attempt.settled_outcome_content_hash = Some(sha256_json(&outcome).unwrap());
            outcome_stage(attempt, vec![outcome], RunStatus::Settled)
        };

        let mut wrong_stage_domain = stage_for(outcome_row());
        wrong_stage_domain.domain = "wrong-domain".into();
        wrong_stage_domain
            .validate()
            .expect_err("stage domain must be exact");

        let mut wrong_row_domain = outcome_row();
        wrong_row_domain.domain = "wrong-domain".into();
        stage_for(wrong_row_domain)
            .validate()
            .expect_err("outcome row domain must be exact");

        let mut noncanonical_due_date = stage_for(outcome_row());
        noncanonical_due_date.stored_due_date = "2026-7-28".into();
        noncanonical_due_date.outcome_attempt_rows[0].stored_due_date = "2026-7-28".into();
        noncanonical_due_date.outcome_rows[0].due_trading_date = "2026-7-28".into();
        noncanonical_due_date.outcome_attempt_rows[0].settled_outcome_content_hash =
            Some(sha256_json(&noncanonical_due_date.outcome_rows[0]).unwrap());
        noncanonical_due_date
            .validate()
            .expect_err("stage due date must use canonical YYYY-MM-DD");

        let mut zero_volume = outcome_row();
        zero_volume.volume = "0".into();
        stage_for(zero_volume)
            .validate()
            .expect_err("DDL requires strictly positive volume");

        let mut zero_volume_ratio = outcome_row();
        zero_volume_ratio.volume_ratio = "0".into();
        stage_for(zero_volume_ratio)
            .validate()
            .expect_err("DDL requires strictly positive volume ratio");

        let mut invalid_ohlc = outcome_row();
        invalid_ohlc.high = "9".into();
        stage_for(invalid_ohlc)
            .validate()
            .expect_err("outcome OHLC relationships must be valid");

        let mut non_finite_return = outcome_row();
        non_finite_return.return_from_t0_close = "NaN".into();
        stage_for(non_finite_return)
            .validate()
            .expect_err("all outcome decimals must be finite");
    }

    #[test]
    fn outcome_stage_rejects_noncanonical_decimal_spellings() {
        let stage_for = |outcome: SelectionSampleOutcomeRowContentPreimage| {
            let mut attempt = outcome_attempt(OutcomeAttemptResult::Settled);
            attempt.settled_outcome_content_hash = Some(sha256_json(&outcome).unwrap());
            outcome_stage(attempt, vec![outcome], RunStatus::Settled)
        };

        macro_rules! assert_noncanonical {
            ($field:ident, $value:literal) => {{
                let mut outcome = outcome_row();
                outcome.$field = $value.into();
                let error = stage_for(outcome).validate().expect_err(concat!(
                    stringify!($field),
                    " must use canonical decimal bytes"
                ));
                assert_eq!(error.code, "invalid_canonical_decimal");
            }};
        }

        assert_noncanonical!(open, "10.0");
        assert_noncanonical!(high, "11.0");
        assert_noncanonical!(low, "9.0");
        assert_noncanonical!(close, "10.50");
        assert_noncanonical!(volume, "1e3");
        assert_noncanonical!(amount, "10500.0");
        assert_noncanonical!(return_from_t0_close, "0.10");
        assert_noncanonical!(cumulative_mfe, "1e-1");
        assert_noncanonical!(cumulative_mae, "-0");
        assert_noncanonical!(volume_ratio, "1.0");
        assert_noncanonical!(return_from_t0_close, "0.0");
    }

    #[test]
    fn outcome_stage_rejects_an_internally_consistent_non_uuid_run_identity() {
        let mut stage = expected_wait_stage();
        stage.stage_run_id = "TEST_CODE_NOT_UUID_V7".into();

        stage
            .validate()
            .expect_err("outcome run identity must be canonical UUIDv7");
    }

    fn global_news_capability() -> ProviderCapabilityHashPreimage {
        ProviderCapabilityHashPreimage {
            domain: DOMAIN_PROVIDER_CAPABILITY.into(),
            provider: "eastmoney".into(),
            capability_name: "GlobalNews-Eastmoney".into(),
            contract_version: "magic-market-core.NewsProvider.global_news.v0.2.0".into(),
            upstream_revision: UPSTREAM_REVISION.into(),
        }
    }

    fn global_news_request_columns() -> RequestEvidenceColumns {
        let feed_identity = registered_global_news_feed_identity(
            "eastmoney_global_news",
            "eastmoney",
            "eastmoney",
            "eastmoney-web",
            "GlobalNews-Eastmoney",
        )
        .unwrap();
        build_request_evidence(
            RequestParametersPreimage::GlobalNews(GlobalNewsRequestParametersPreimage {
                domain: DOMAIN_GLOBAL_NEWS_REQUEST.into(),
                feed_identity,
                limit: 20,
            }),
            global_news_capability(),
        )
        .expect("registered global-news request must build typed evidence")
    }

    fn available_source_attempt_row(
        response_provider: &str,
    ) -> SelectionSourceBatchAttemptRowContentPreimage {
        let request = global_news_request_columns();
        let request_preimage = request
            .validate(Some(RequestKind::GlobalNews))
            .expect("test request must be valid");
        let evidence = FeedBatchEvidencePreimage {
            domain: DOMAIN_FEED_BATCH_EVIDENCE.into(),
            feed_identity: request_preimage.canonical_subject.clone(),
            provider: response_provider.into(),
            source: "eastmoney-web".into(),
            source_at: Some("2026-07-28T07:00:00.000000000Z".into()),
            observed_at: "2026-07-28T07:00:01.000000000Z".into(),
            batch_id: "TEST_CODE_NEWS_BATCH".into(),
            batch_quality: FeedBatchQuality::Complete,
        };
        let evidence_hash = sha256_json(&evidence).unwrap();
        let batch_content_hash = hash('d');
        let feed_content = FeedAttemptContentPreimage {
            domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
            feed_identity: request_preimage.canonical_subject.clone(),
            request_hash: request.request_hash.clone(),
            request_evidence_hash: request.request_evidence_hash.clone(),
            status_kind: FeedStatusKind::Available,
            record_count: Some(1),
            evidence_hash: Some(evidence_hash.clone()),
            source_content_hash: Some(batch_content_hash.clone()),
            available_evidence_hash: Some(evidence_hash.clone()),
            failed_stage: None,
            reason_code: None,
            retryable: None,
            detail_hash: None,
            error_fingerprint: None,
        };
        SelectionSourceBatchAttemptRowContentPreimage {
            domain: DOMAIN_SOURCE_BATCH_ATTEMPT_ROW.into(),
            source_batch_attempt_id: hash('c'),
            ingress_run_id: "01900000-0000-7000-8000-000000000001".into(),
            config_activation_run_id: "01900000-0000-7000-8000-000000000002".into(),
            config_hash: hash('b'),
            generation_market_date: "2026-07-28".into(),
            registered_feed_identity: request_preimage.canonical_subject,
            registered_feed_snapshot_hash: hash('a'),
            request_hash: request.request_hash,
            request_evidence_json: request.request_evidence_json,
            request_evidence_hash: request.request_evidence_hash,
            feed_attempt_content_hash: sha256_json(&feed_content).unwrap(),
            status_kind: FeedStatusKind::Available,
            record_count: Some(1),
            provider: Some(response_provider.into()),
            source: Some(evidence.source.clone()),
            source_at: evidence.source_at.clone(),
            observed_at: Some(evidence.observed_at.clone()),
            batch_id: Some(evidence.batch_id.clone()),
            batch_content_hash: Some(batch_content_hash),
            failed_stage: None,
            reason_code: None,
            retryable: None,
            available_evidence_json: Some(canonical_json(&evidence).unwrap()),
            available_evidence_hash: Some(evidence_hash),
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            attempted_at: "2026-07-28T07:00:02.000000000Z".into(),
        }
    }

    fn valid_empty_source_ingress_stage() -> SourceIngressStageInputPreimage {
        const RUN_ID: &str = "01900000-0000-7000-8000-000000000001";
        const ACTIVATION_ID: &str = "01900000-0000-7000-8000-000000000002";
        let registrations = [
            (
                "eastmoney_global_news",
                "eastmoney",
                "eastmoney",
                "eastmoney-web",
                "GlobalNews-Eastmoney",
            ),
            (
                "cls_global_news",
                "cailianpress",
                "cailianpress",
                "cls-v1",
                "GlobalNews-CLS",
            ),
            (
                "jin10_global_news",
                "jin10",
                "jin10",
                "jin10-flash-v1",
                "GlobalNews-Jin10",
            ),
            (
                "thepaper_global_news",
                "thepaper",
                "thepaper",
                "thepaper-finance-v1",
                "GlobalNews-ThePaper",
            ),
        ];
        let mut registered = registrations
            .iter()
            .map(
                |(feed_name, gateway_provider, provider_id, source_contract, capability_name)| {
                    let configuration = RegisteredFeedConfigurationPreimage {
                        domain: DOMAIN_REGISTERED_FEED_CONFIG.into(),
                        gateway_provider: (*gateway_provider).into(),
                        provider_id: (*provider_id).into(),
                        source_contract: (*source_contract).into(),
                        capability_name: (*capability_name).into(),
                        max_limit: 20,
                        upstream_revision: UPSTREAM_REVISION.into(),
                    };
                    let configuration_hash = sha256_json(&configuration).unwrap();
                    let identity = RegisteredFeedIdentityPreimage {
                        domain: DOMAIN_REGISTERED_FEED_IDENTITY.into(),
                        feed_name: (*feed_name).into(),
                        gateway_provider: (*gateway_provider).into(),
                        configuration_hash: configuration_hash.clone(),
                    };
                    (
                        sha256_json(&identity).unwrap(),
                        configuration_hash,
                        *gateway_provider,
                        *source_contract,
                        *capability_name,
                        *feed_name,
                    )
                },
            )
            .collect::<Vec<_>>();
        registered.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
        let snapshot = RegisteredFeedSnapshotPreimage {
            domain: DOMAIN_REGISTERED_FEED_SNAPSHOT.into(),
            feeds_sorted: registered
                .iter()
                .enumerate()
                .map(
                    |(
                        ordinal,
                        (
                            feed_identity,
                            configuration_hash,
                            gateway_provider,
                            _,
                            capability_name,
                            _,
                        ),
                    )| RegisteredFeedEntryPreimage {
                        ordinal: ordinal as u32,
                        feed_identity: feed_identity.clone(),
                        gateway_provider: (*gateway_provider).into(),
                        capability_name: (*capability_name).into(),
                        configuration_hash: configuration_hash.clone(),
                    },
                )
                .collect(),
        };
        let snapshot_json = canonical_json(&snapshot).unwrap();
        let snapshot_hash = sha256_json(&snapshot).unwrap();
        let mut rows = registered
            .iter()
            .map(
                |(
                    feed_identity,
                    _,
                    gateway_provider,
                    source_contract,
                    capability_name,
                    feed_name,
                )| {
                    let request = build_request_evidence(
                        RequestParametersPreimage::GlobalNews(
                            GlobalNewsRequestParametersPreimage {
                                domain: DOMAIN_GLOBAL_NEWS_REQUEST.into(),
                                feed_identity: feed_identity.clone(),
                                limit: 20,
                            },
                        ),
                        ProviderCapabilityHashPreimage {
                            domain: DOMAIN_PROVIDER_CAPABILITY.into(),
                            provider: (*gateway_provider).into(),
                            capability_name: (*capability_name).into(),
                            contract_version: "magic-market-core.NewsProvider.global_news.v0.2.0"
                                .into(),
                            upstream_revision: UPSTREAM_REVISION.into(),
                        },
                    )
                    .unwrap();
                    let evidence = FeedBatchEvidencePreimage {
                        domain: DOMAIN_FEED_BATCH_EVIDENCE.into(),
                        feed_identity: feed_identity.clone(),
                        provider: (*gateway_provider).into(),
                        source: (*source_contract).into(),
                        source_at: Some("2026-07-28T07:00:00.000000000Z".into()),
                        observed_at: "2026-07-28T07:00:01.000000000Z".into(),
                        batch_id: format!("TEST_CODE_{feed_name}_EMPTY"),
                        batch_quality: FeedBatchQuality::Complete,
                    };
                    let evidence_hash = sha256_json(&evidence).unwrap();
                    let feed_source_hash =
                        feed_source_content_hash(feed_identity, &evidence_hash, &[]).unwrap();
                    let feed_content = FeedAttemptContentPreimage {
                        domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
                        feed_identity: feed_identity.clone(),
                        request_hash: request.request_hash.clone(),
                        request_evidence_hash: request.request_evidence_hash.clone(),
                        status_kind: FeedStatusKind::VerifiedEmpty,
                        record_count: Some(0),
                        evidence_hash: Some(evidence_hash.clone()),
                        source_content_hash: Some(feed_source_hash.clone()),
                        available_evidence_hash: Some(evidence_hash.clone()),
                        failed_stage: None,
                        reason_code: None,
                        retryable: None,
                        detail_hash: None,
                        error_fingerprint: None,
                    };
                    SelectionSourceBatchAttemptRowContentPreimage {
                        domain: DOMAIN_SOURCE_BATCH_ATTEMPT_ROW.into(),
                        source_batch_attempt_id: sha256_json(&FeedAttemptKeyPreimage {
                            domain: DOMAIN_FEED_ATTEMPT_KEY.into(),
                            ingress_run_id: RUN_ID.into(),
                            feed_identity: feed_identity.clone(),
                        })
                        .unwrap(),
                        ingress_run_id: RUN_ID.into(),
                        config_activation_run_id: ACTIVATION_ID.into(),
                        config_hash: hash('5'),
                        generation_market_date: "2026-07-28".into(),
                        registered_feed_identity: feed_identity.clone(),
                        registered_feed_snapshot_hash: snapshot_hash.clone(),
                        request_hash: request.request_hash,
                        request_evidence_json: request.request_evidence_json,
                        request_evidence_hash: request.request_evidence_hash,
                        feed_attempt_content_hash: sha256_json(&feed_content).unwrap(),
                        status_kind: FeedStatusKind::VerifiedEmpty,
                        record_count: Some(0),
                        provider: Some(evidence.provider.clone()),
                        source: Some(evidence.source.clone()),
                        source_at: evidence.source_at.clone(),
                        observed_at: Some(evidence.observed_at.clone()),
                        batch_id: Some(evidence.batch_id.clone()),
                        batch_content_hash: Some(feed_source_hash),
                        failed_stage: None,
                        reason_code: None,
                        retryable: None,
                        available_evidence_json: Some(canonical_json(&evidence).unwrap()),
                        available_evidence_hash: Some(evidence_hash),
                        error_detail_json: None,
                        error_detail_hash: None,
                        error_fingerprint: None,
                        attempted_at: "2026-07-28T07:00:02.000000000Z".into(),
                    }
                },
            )
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            left.source_batch_attempt_id
                .as_bytes()
                .cmp(right.source_batch_attempt_id.as_bytes())
        });
        let source_batch_content_hash = source_batch_content_hash(
            &snapshot_hash,
            snapshot
                .feeds_sorted
                .iter()
                .map(|entry| {
                    rows.iter()
                        .find(|row| row.registered_feed_identity == entry.feed_identity)
                        .unwrap()
                        .feed_attempt_content_hash
                        .clone()
                })
                .collect(),
            vec![],
            vec![],
            "2026-07-28T07:00:03.000000000Z".into(),
        )
        .unwrap();
        SourceIngressStageInputPreimage {
            domain: DOMAIN_SOURCE_INGRESS_STAGE.into(),
            stage_run_id: RUN_ID.into(),
            logical_subject_key: run_logical_subject_key(&RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::IngressRun,
                source_fact_key: None,
                config_hash: Some(hash('5')),
                sample_key: None,
                outcome_phase: None,
                stored_due_date: None,
                ingress_source_batch_hash: Some(source_batch_content_hash.clone()),
            })
            .unwrap(),
            config_activation_run_id: ACTIVATION_ID.into(),
            config_hash: hash('5'),
            generation_market_date: "2026-07-28".into(),
            aggregator_observed_at_rfc3339_nanos_utc: "2026-07-28T07:00:03.000000000Z".into(),
            source_batch_content_hash,
            registered_feed_snapshot_json: snapshot_json,
            registered_feed_snapshot_hash: snapshot_hash,
            source_batch_attempt_rows: rows,
            source_fact_rows: vec![],
            source_fact_attempt_rows: vec![],
            planned_run_status: RunStatus::Completed,
        }
    }

    fn rebind_empty_source_ingress_stage(stage: &mut SourceIngressStageInputPreimage) {
        let mut snapshot: RegisteredFeedSnapshotPreimage =
            serde_json::from_str(&stage.registered_feed_snapshot_json).unwrap();
        for (ordinal, entry) in snapshot.feeds_sorted.iter_mut().enumerate() {
            entry.ordinal = ordinal as u32;
        }
        stage.registered_feed_snapshot_json = canonical_json(&snapshot).unwrap();
        stage.registered_feed_snapshot_hash = sha256_json(&snapshot).unwrap();
        for row in &mut stage.source_batch_attempt_rows {
            row.registered_feed_snapshot_hash = stage.registered_feed_snapshot_hash.clone();
        }
        stage.source_batch_content_hash = source_batch_content_hash(
            &stage.registered_feed_snapshot_hash,
            snapshot
                .feeds_sorted
                .iter()
                .map(|entry| {
                    stage
                        .source_batch_attempt_rows
                        .iter()
                        .find(|row| row.registered_feed_identity == entry.feed_identity)
                        .unwrap()
                        .feed_attempt_content_hash
                        .clone()
                })
                .collect(),
            vec![],
            vec![],
            stage.aggregator_observed_at_rfc3339_nanos_utc.clone(),
        )
        .unwrap();
        stage.logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::IngressRun,
            source_fact_key: None,
            config_hash: Some(stage.config_hash.clone()),
            sample_key: None,
            outcome_phase: None,
            stored_due_date: None,
            ingress_source_batch_hash: Some(stage.source_batch_content_hash.clone()),
        })
        .unwrap();
    }

    fn board_request_columns() -> RequestEvidenceColumns {
        build_request_evidence(
            RequestParametersPreimage::BoardConstituents(
                BoardConstituentRequestParametersPreimage {
                    domain: DOMAIN_BOARD_CONSTITUENT_REQUEST.into(),
                    artifact_content_hash: hash('a'),
                    binding_audit_hash: hash('b'),
                    provider: "tdx".into(),
                    kind: ProviderBoardKind::Concept,
                    code: "tdx:concept:TEST_CODE_BOARD".into(),
                    name: "TEST_CODE_BOARD".into(),
                    limit: BOARD_DIRECTORY_REQUEST_LIMIT,
                },
            ),
            magic_tdx_capability(
                "MagicTdx-BoardConstituents",
                "magic-tdx-rs.BlockProvider.board_constituents.v0.2.0",
            ),
        )
        .expect("registered board request must build typed evidence")
    }

    #[test]
    fn request_evidence_round_trips_the_exact_row_tuple() {
        let columns = global_news_request_columns();
        require_hash(&columns.request_hash, "request_hash").unwrap();
        require_hash(&columns.request_evidence_hash, "request_evidence_hash").unwrap();
        assert_eq!(
            sha256_bytes(columns.request_evidence_json.as_bytes()),
            columns.request_evidence_hash
        );

        let evidence = columns
            .validate(Some(RequestKind::GlobalNews))
            .expect("row tuple must recover the complete typed request");
        assert_eq!(evidence.request_hash, columns.request_hash);
        assert_eq!(
            evidence.parameters_schema,
            GLOBAL_NEWS_REQUEST_PARAMETERS_SCHEMA
        );
        assert_eq!(
            evidence.canonical_subject,
            registered_global_news_feed_identity(
                "eastmoney_global_news",
                "eastmoney",
                "eastmoney",
                "eastmoney-web",
                "GlobalNews-Eastmoney",
            )
            .unwrap()
        );
    }

    #[test]
    fn request_evidence_accepts_all_four_closed_request_kinds() {
        let relation = direct_relation_row('1');
        let sample = admitted_sample_row(&relation);
        let cases = [
            (global_news_request_columns(), RequestKind::GlobalNews),
            (board_request_columns(), RequestKind::BoardConstituents),
            (t0_request_columns(&sample), RequestKind::T0MarketEvidence),
            (
                outcome_request_columns(),
                RequestKind::OutcomeMarketEvidence,
            ),
        ];
        for (columns, expected_kind) in cases {
            assert_eq!(
                columns.validate(Some(expected_kind)).unwrap().request_kind,
                expected_kind
            );
        }
    }

    #[test]
    fn request_evidence_rejects_cross_feed_capability_swaps() {
        let jin10_identity = registered_global_news_feed_identity(
            "jin10_global_news",
            "jin10",
            "jin10",
            "jin10-flash-v1",
            "GlobalNews-Jin10",
        )
        .unwrap();
        build_request_evidence(
            RequestParametersPreimage::GlobalNews(GlobalNewsRequestParametersPreimage {
                domain: DOMAIN_GLOBAL_NEWS_REQUEST.into(),
                feed_identity: jin10_identity,
                limit: 20,
            }),
            global_news_capability(),
        )
        .expect_err("Jin10 feed identity must reject Eastmoney capability");
    }

    #[test]
    fn request_evidence_rejects_self_consistent_response_provider_swaps() {
        let source = available_source_attempt_row("TEST_CODE_FORGED_PROVIDER");
        assert_eq!(
            source.validate().unwrap_err().code,
            "source_provider_capability_projection_mismatch"
        );

        let mut generation = valid_completed_generation_stage();
        let evaluation = &mut generation.evaluation_attempt_rows[0];
        let mut evidence: ProviderAvailableEvidencePreimage =
            serde_json::from_str(evaluation.available_evidence_json.as_deref().unwrap()).unwrap();
        evidence.provider = "TEST_CODE_FORGED_PROVIDER".into();
        evaluation.provider = Some(evidence.provider.clone());
        evaluation.available_evidence_json = Some(canonical_json(&evidence).unwrap());
        evaluation.available_evidence_hash = Some(sha256_json(&evidence).unwrap());
        assert_eq!(
            generation.validate().unwrap_err().code,
            "evaluation_provider_capability_projection_mismatch"
        );

        let mut attempt = outcome_attempt(OutcomeAttemptResult::Settled);
        let mut evidence: OutcomeProviderAvailableEvidencePreimage =
            serde_json::from_str(attempt.available_evidence_json.as_deref().unwrap()).unwrap();
        evidence.provider_evidence.provider = "TEST_CODE_FORGED_PROVIDER".into();
        attempt.provider = Some(evidence.provider_evidence.provider.clone());
        attempt.available_evidence_json = Some(canonical_json(&evidence).unwrap());
        attempt.available_evidence_hash = Some(sha256_json(&evidence).unwrap());
        let stage = outcome_stage(attempt, vec![outcome_row()], RunStatus::Settled);
        assert_eq!(
            stage.validate().unwrap_err().code,
            "outcome_provider_capability_projection_mismatch"
        );
    }

    #[test]
    fn source_ingress_rejects_noncanonical_registry_batch_and_child_rows() {
        valid_empty_source_ingress_stage()
            .validate()
            .expect("the exact four-feed verified-empty ingress is canonical");

        let mut missing_feed = valid_empty_source_ingress_stage();
        let removed = missing_feed.source_batch_attempt_rows.pop().unwrap();
        let mut snapshot: RegisteredFeedSnapshotPreimage =
            serde_json::from_str(&missing_feed.registered_feed_snapshot_json).unwrap();
        snapshot
            .feeds_sorted
            .retain(|entry| entry.feed_identity != removed.registered_feed_identity);
        missing_feed.registered_feed_snapshot_json = canonical_json(&snapshot).unwrap();
        rebind_empty_source_ingress_stage(&mut missing_feed);
        assert_eq!(
            missing_feed.validate().unwrap_err().code,
            "registered_feed_registry_mismatch"
        );

        let mut config_drift = valid_empty_source_ingress_stage();
        let mut snapshot: RegisteredFeedSnapshotPreimage =
            serde_json::from_str(&config_drift.registered_feed_snapshot_json).unwrap();
        snapshot.feeds_sorted[0].configuration_hash = hash('f');
        config_drift.registered_feed_snapshot_json = canonical_json(&snapshot).unwrap();
        rebind_empty_source_ingress_stage(&mut config_drift);
        assert_eq!(
            config_drift.validate().unwrap_err().code,
            "registered_feed_registry_mismatch"
        );

        let mut forged_batch = valid_empty_source_ingress_stage();
        forged_batch.source_batch_content_hash = hash('f');
        forged_batch.logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::IngressRun,
            source_fact_key: None,
            config_hash: Some(forged_batch.config_hash.clone()),
            sample_key: None,
            outcome_phase: None,
            stored_due_date: None,
            ingress_source_batch_hash: Some(forged_batch.source_batch_content_hash.clone()),
        })
        .unwrap();
        assert_eq!(
            forged_batch.validate().unwrap_err().code,
            "source_batch_content_hash_mismatch"
        );

        let mut invalid_fact = valid_empty_source_ingress_stage();
        invalid_fact
            .source_fact_rows
            .push(SelectionSourceFactRowContentPreimage {
                domain: "wrong-domain".into(),
                source_fact_key: hash('1'),
                event_id: hash('2'),
                payload_schema: GLOBAL_NEWS_SOURCE_FACT_SCHEMA.into(),
                config_activation_run_id: invalid_fact.config_activation_run_id.clone(),
                config_hash: invalid_fact.config_hash.clone(),
                generation_market_date: invalid_fact.generation_market_date.clone(),
                provider_source: "eastmoney-web".into(),
                item_id: "TEST_CODE_ITEM".into(),
                title: "TEST_CODE_TITLE".into(),
                summary: None,
                content: None,
                publisher: "TEST_CODE_PUBLISHER".into(),
                canonical_url: "https://example.invalid/TEST_CODE".into(),
                published_at: "2026-07-28T07:00:00.000000000Z".into(),
                instruments_json: "[]".into(),
                topics_json: "[]".into(),
                language: "zh-CN".into(),
                record_provider: "eastmoney".into(),
                record_source: "eastmoney-web".into(),
                record_source_at: None,
                record_observed_at: "2026-07-28T07:00:01.000000000Z".into(),
                record_batch_id: "TEST_CODE_BATCH".into(),
                record_batch_content_hash: hash('3'),
                provider_content_hash: hash('4'),
                first_ingress_run_id: invalid_fact.stage_run_id.clone(),
                ingress_gate_version: "TEST_CODE_GATE_V1".into(),
                ingress_gate_input_json: "{}".into(),
                ingress_gate_input_hash: hash('5'),
                ingress_decision: IngressDecision::Admitted,
                ingress_reason_code: None,
                ingress_retryable: None,
                ingress_gate_receipt_json: "{}".into(),
                ingress_gate_receipt_hash: hash('6'),
            });
        assert_eq!(invalid_fact.validate().unwrap_err().code, "domain_mismatch");

        let mut invalid_attempt = valid_empty_source_ingress_stage();
        invalid_attempt.source_fact_attempt_rows.push(
            SelectionSourceFactAttemptRowContentPreimage {
                domain: "wrong-domain".into(),
                source_fact_attempt_id: hash('1'),
                ingress_run_id: invalid_attempt.stage_run_id.clone(),
                source_batch_attempt_id: hash('2'),
                provider_ordinal: 0,
                source_fact_key: hash('3'),
                acquired_record_json: "{}".into(),
                acquired_record_hash: hash('4'),
                batch_evidence_json: "{}".into(),
                batch_evidence_hash: hash('5'),
                event_projection_id: hash('6'),
                attempt_result: SourceFactAttemptResult::Replay,
                conflict_hash: None,
                attempted_at: "2026-07-28T07:00:02.000000000Z".into(),
            },
        );
        assert_eq!(
            invalid_attempt.validate().unwrap_err().code,
            "domain_mismatch"
        );

        let mut pending = valid_empty_source_ingress_stage();
        pending.planned_run_status = RunStatus::PendingDependency;
        assert_eq!(
            pending.validate().unwrap_err().code,
            "source_ingress_status_invalid"
        );
    }

    #[test]
    fn stage_validators_recompute_logical_subject_and_outcome_provider_lineage() {
        let mut ingress = valid_empty_source_ingress_stage();
        ingress.logical_subject_key = hash('f');
        assert_eq!(
            ingress.validate().unwrap_err().code,
            "logical_subject_key_mismatch"
        );

        let mut generation = valid_completed_generation_stage();
        generation.logical_subject_key = hash('f');
        assert_eq!(
            generation.validate().unwrap_err().code,
            "logical_subject_key_mismatch"
        );

        let mut outcome = outcome_stage(
            outcome_attempt(OutcomeAttemptResult::Settled),
            vec![outcome_row()],
            RunStatus::Settled,
        );
        outcome.logical_subject_key = hash('f');
        assert_eq!(
            outcome.validate().unwrap_err().code,
            "logical_subject_key_mismatch"
        );

        let mut outcome = outcome_stage(
            outcome_attempt(OutcomeAttemptResult::Settled),
            vec![outcome_row()],
            RunStatus::Settled,
        );
        outcome.outcome_rows[0].provider = "TEST_CODE_FORGED_PROVIDER".into();
        outcome.outcome_attempt_rows[0].settled_outcome_content_hash =
            Some(sha256_json(&outcome.outcome_rows[0]).unwrap());
        assert_eq!(
            outcome.validate().unwrap_err().code,
            "settled_outcome_evidence_projection_mismatch"
        );
    }

    #[test]
    fn request_evidence_rejects_self_consistent_kind_and_subject_swaps() {
        let columns = global_news_request_columns();
        let mut evidence: RequestEvidencePreimage =
            serde_json::from_str(&columns.request_evidence_json).unwrap();

        evidence.parameters_schema = BOARD_CONSTITUENTS_REQUEST_PARAMETERS_SCHEMA.into();
        let swapped_schema = RequestEvidenceColumns {
            request_hash: evidence.request_hash.clone(),
            request_evidence_json: canonical_json(&evidence).unwrap(),
            request_evidence_hash: sha256_json(&evidence).unwrap(),
        };
        swapped_schema
            .validate(Some(RequestKind::GlobalNews))
            .expect_err("a schema from another request kind must reject");

        let mut evidence: RequestEvidencePreimage =
            serde_json::from_str(&columns.request_evidence_json).unwrap();
        evidence.canonical_subject = hash('b');
        evidence.request_hash = sha256_json(&RequestHashPreimage {
            domain: DOMAIN_REQUEST.into(),
            request_kind: RequestKind::GlobalNews,
            canonical_subject: evidence.canonical_subject.clone(),
            parameters_json_hash: evidence.parameters_json_hash.clone(),
            provider_capability_hash: evidence.provider_capability_hash.clone(),
        })
        .unwrap();
        let swapped_subject = RequestEvidenceColumns {
            request_hash: evidence.request_hash.clone(),
            request_evidence_json: canonical_json(&evidence).unwrap(),
            request_evidence_hash: sha256_json(&evidence).unwrap(),
        };
        swapped_subject
            .validate(Some(RequestKind::GlobalNews))
            .expect_err("subject must be reconstructed from the typed parameters");
    }

    #[test]
    fn affected_hash_domains_and_stage_payload_schemas_match_live_versions() {
        assert_eq!(
            DOMAIN_FEED_ATTEMPT_CONTENT,
            "stock_analysis.br174.feed_attempt_content.v2"
        );
        assert_eq!(
            DOMAIN_RELATION_ATTEMPT,
            "stock_analysis.br174.relation_attempt.v2"
        );
        assert_eq!(
            DOMAIN_OUTCOME_ATTEMPT,
            "stock_analysis.br174.outcome_attempt.v3"
        );
        assert_eq!(
            DOMAIN_SOURCE_BATCH_ATTEMPT_ROW,
            "stock_analysis.br174.selection_source_batch_attempts_row.v2"
        );
        assert_eq!(
            DOMAIN_RELATION_ATTEMPT_ROW,
            "stock_analysis.br174.selection_relation_attempts_row.v2"
        );
        assert_eq!(
            DOMAIN_EVALUATION_ATTEMPT_ROW,
            "stock_analysis.br174.selection_evaluation_attempts_row.v2"
        );
        assert_eq!(
            DOMAIN_OUTCOME_ATTEMPT_ROW,
            "stock_analysis.br174.selection_outcome_attempts_row.v3"
        );
        assert_eq!(
            SOURCE_INGRESS_STAGE_PAYLOAD_SCHEMA,
            "source-ingress-stage-v2"
        );
        assert_eq!(GENERATION_STAGE_PAYLOAD_SCHEMA, "generation-stage-v3");
        assert_eq!(OUTCOME_STAGE_PAYLOAD_SCHEMA, "outcome-stage-v3");
    }

    #[test]
    fn typed_outcome_transport_attempts_rotate_every_live_hash_domain() {
        assert_eq!(
            DOMAIN_OUTCOME_ATTEMPT,
            "stock_analysis.br174.outcome_attempt.v3"
        );
        assert_eq!(
            DOMAIN_OUTCOME_ATTEMPT_ROW,
            "stock_analysis.br174.selection_outcome_attempts_row.v3"
        );
        assert_eq!(
            DOMAIN_OUTCOME_PAYLOAD,
            "stock_analysis.br174.outcome_payload.v2"
        );
        assert_eq!(
            DOMAIN_OUTCOME_STAGE,
            "stock_analysis.br174.outcome_stage.v3"
        );
        assert_eq!(OUTCOME_STAGE_PAYLOAD_SCHEMA, "outcome-stage-v3");
    }

    #[test]
    fn expected_wait_rejects_a_half_present_transport_attempt_pair() {
        let mut attempt = outcome_attempt(OutcomeAttemptResult::ExpectedWait);
        attempt.transport_attempts_json = Some("{}".into());
        let error = attempt
            .validate()
            .expect_err("ExpectedWait must retain a NULL/NULL transport-attempt pair");
        assert_eq!(error.code, "outcome_transport_attempts_pair_mismatch");
    }

    #[test]
    fn recovery_stage_payload_is_strict_and_canonically_reserialized() {
        let stage = valid_completed_generation_stage();
        let json = canonical_json(&stage).unwrap();
        let hash = sha256_bytes(json.as_bytes());
        validate_stage_payload_json(
            SubjectKind::GenerationRun,
            GENERATION_STAGE_PAYLOAD_SCHEMA,
            &json,
            &hash,
        )
        .expect("canonical typed generation payload must recover");

        let with_unknown = format!("{},\"unknown\":true}}", json.strip_suffix('}').unwrap());
        validate_stage_payload_json(
            SubjectKind::GenerationRun,
            GENERATION_STAGE_PAYLOAD_SCHEMA,
            &with_unknown,
            &sha256_bytes(with_unknown.as_bytes()),
        )
        .expect_err("deny_unknown_fields must reject recovery payload expansion");

        let with_trailing = format!("{json} trailing");
        validate_stage_payload_json(
            SubjectKind::GenerationRun,
            GENERATION_STAGE_PAYLOAD_SCHEMA,
            &with_trailing,
            &sha256_bytes(with_trailing.as_bytes()),
        )
        .expect_err("restart parsing must consume the complete payload");
    }

    #[test]
    fn provider_error_mapping_is_exhaustive_and_message_free() {
        let cases = [
            (
                ProviderFailureCodeV1::Transport,
                ProviderErrorKind::Transport,
                true,
            ),
            (
                ProviderFailureCodeV1::Protocol,
                ProviderErrorKind::Protocol,
                false,
            ),
            (
                ProviderFailureCodeV1::Timeout,
                ProviderErrorKind::Timeout,
                true,
            ),
            (
                ProviderFailureCodeV1::InvalidData,
                ProviderErrorKind::InvalidData,
                false,
            ),
            (
                ProviderFailureCodeV1::Unsupported,
                ProviderErrorKind::Unsupported,
                false,
            ),
            (
                ProviderFailureCodeV1::Integrity,
                ProviderErrorKind::Integrity,
                false,
            ),
            (
                ProviderFailureCodeV1::SettledBarMissing,
                ProviderErrorKind::InvalidData,
                true,
            ),
            (
                ProviderFailureCodeV1::Unmapped,
                ProviderErrorKind::Integrity,
                false,
            ),
        ];
        for (input, error_kind, retryable) in cases {
            let mapping = input.mapping();
            assert_eq!(mapping.error_kind, error_kind);
            assert_eq!(mapping.retryable, retryable);
            assert!(is_snake_case_token(mapping.diagnostic_code));
        }
        let unmapped = ProviderFailureCodeV1::Unmapped.mapping();
        assert_eq!(unmapped.diagnostic_code, "provider_error_mapping_missing");
        assert_eq!(unmapped.invariant_id, Some("provider-error-codes-v1"));
    }

    #[test]
    fn canonical_decimal_rejects_non_finite_and_negative_zero() {
        assert_eq!(canonical_f64(0.0).unwrap(), "0");
        assert!(canonical_f64(-0.0).is_err());
        assert!(canonical_f64(f64::NAN).is_err());
        assert!(canonical_f64(f64::INFINITY).is_err());
    }
}
