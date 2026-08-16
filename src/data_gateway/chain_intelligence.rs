//! BR-159/BR-160 deterministic A-10 chain-intelligence derivation.
//!
//! Transport stays outside this pure seam.  Provider adapters normalize their
//! admitted batches into these facts, then this module performs only validated
//! identity joins, filtering, ordering and immutable batch construction.

use crate::data_gateway::review::{
    acquisition_request_hash, audit_gateway_result, audit_routed_gateway_result,
    route_exact_date_upper_limit_pool, BatchEvidence, GatewayBatch, GatewayError,
};
use crate::database::chain_intelligence::{
    ChainBatchInput, ChainInput, ChainInputEvidenceInput, ChainMemberInput, ChainRejectionInput,
    ChainVisibilityReceiptInput, VisibleChainBatch,
};
use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
use crate::database::DatabaseManager;
use chrono::{DateTime, FixedOffset, Local, NaiveDate, SecondsFormat};
use magic_market_core::{
    AssetClass, BoardCategory, BoardMembership, BoardMembershipProvider, DataBatch, Exchange,
    InstrumentId, LimitPoolEntry, PositiveU32, ProviderId, SecurityMetadata,
    SecurityMetadataProvider,
};
use magic_tdx_rs::{protocol::constants::PRIMARY_SERVERS, BlockService, TdxError, TdxSmartClient};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainIntelligencePolicy {
    pub calculation_version: String,
    pub taxonomy_version: String,
    pub min_members: usize,
    pub excluded_board_names: BTreeSet<String>,
}

impl TryFrom<&crate::config::ChainIntelligenceConfig> for ChainIntelligencePolicy {
    type Error = GatewayError;

    fn try_from(config: &crate::config::ChainIntelligenceConfig) -> Result<Self, Self::Error> {
        config.validate().map_err(|error| {
            GatewayError::classified(
                "A-10",
                None,
                "invalid_request",
                "chain_policy_invalid",
                false,
                error,
            )
        })?;
        Ok(Self {
            calculation_version: config.calculation_version.clone(),
            taxonomy_version: config.taxonomy_version.clone(),
            min_members: config.min_members,
            excluded_board_names: config
                .excluded_board_names
                .iter()
                .map(|name| name.trim().to_owned())
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainSourceEvidence {
    pub capability: String,
    pub provider: String,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    /// SHA-256 of the canonical admitted source records, not merely the batch
    /// identifier.
    pub records_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpperLimitFact {
    pub instrument_id: String,
    pub security_name: String,
    pub streak: u32,
    pub source_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardMembershipFact {
    pub instrument_id: String,
    pub canonical_board_id: String,
    pub board_name: String,
    pub category: BoardCategory,
}

/// Source-backed item that cannot enter the derivation seam.  Its raw identity
/// is retained only through a one-way hash in immutable rejection evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainSourceRejection {
    pub identity: String,
    pub reason_code: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalLimitFact {
    security_name: String,
    streak: u32,
    source_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalMembership {
    board_name: String,
    category: BoardCategory,
}

#[derive(Serialize)]
struct MemberHashPayload<'a> {
    board_id: &'a str,
    instrument_id: &'a str,
    security_name: &'a str,
    source_event_id: &'a str,
    streak: u32,
}

#[derive(Serialize)]
struct RejectionHashPayload<'a> {
    identity: &'a str,
    reason_code: &'a str,
    retryable: bool,
}

#[derive(Serialize)]
struct BatchIdentityPayload<'a> {
    trading_date: String,
    calculation_version: &'a str,
    taxonomy_version: &'a str,
    ordered_input_batch_ids: Vec<&'a str>,
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::classified(
        "A-10",
        None,
        "partial",
        "chain_input_rejected",
        false,
        message,
    )
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn canonical_hash<T: Serialize>(label: &str, value: &T) -> Result<String, GatewayError> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| invalid(format!("serialize {label} for canonical hash: {error}")))?;
    let mut bytes = Vec::with_capacity(label.len() + payload.len() + 1);
    bytes.extend_from_slice(label.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&payload);
    Ok(sha256_bytes(&bytes))
}

fn require_text(field: &str, value: &str) -> Result<(), GatewayError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(invalid(format!("{field} must be non-empty canonical text")));
    }
    Ok(())
}

fn require_hash(field: &str, value: &str) -> Result<(), GatewayError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!(
            "{field} must be 64 lowercase hex characters"
        )));
    }
    Ok(())
}

fn valid_instrument_id(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_excluded_board(
    policy: &ChainIntelligencePolicy,
    board_name: &str,
    category: BoardCategory,
) -> bool {
    !matches!(category, BoardCategory::Industry | BoardCategory::Concept)
        || policy.excluded_board_names.contains(board_name)
}

fn validate_source_evidence(evidence: &[ChainSourceEvidence]) -> Result<(), GatewayError> {
    if evidence.is_empty() {
        return Err(invalid("chain derivation requires source evidence"));
    }
    let mut batch_ids = BTreeSet::new();
    for item in evidence {
        for (field, value) in [
            ("evidence capability", item.capability.as_str()),
            ("evidence provider", item.provider.as_str()),
            ("evidence source", item.source.as_str()),
            ("evidence observed_at", item.observed_at.as_str()),
            ("evidence batch_id", item.batch_id.as_str()),
        ] {
            require_text(field, value)?;
        }
        if item
            .source_at
            .as_deref()
            .is_some_and(|source_at| source_at.trim().is_empty())
        {
            return Err(invalid("evidence source_at must be absent or non-empty"));
        }
        require_hash("evidence records_hash", &item.records_hash)?;
        if !batch_ids.insert(item.batch_id.as_str()) {
            return Err(invalid(format!(
                "duplicate source batch identity {}",
                item.batch_id
            )));
        }
    }
    Ok(())
}

fn canonical_inputs(
    evidence: &[ChainSourceEvidence],
    parent_identity_hash: &str,
) -> Result<Vec<ChainInputEvidenceInput>, GatewayError> {
    evidence
        .iter()
        .enumerate()
        .map(|(ordinal, item)| {
            let input_id = canonical_hash(
                "BR160_INPUT_ID_V2",
                &(
                    parent_identity_hash,
                    ordinal,
                    item.capability.as_str(),
                    item.provider.as_str(),
                    item.batch_id.as_str(),
                ),
            )?;
            let content_hash = canonical_hash("BR160_INPUT_CONTENT_V1", item)?;
            Ok(ChainInputEvidenceInput {
                input_id: format!("chain-input:{input_id}"),
                ordinal: i32::try_from(ordinal)
                    .map_err(|_| invalid("input evidence ordinal overflow"))?,
                capability: item.capability.clone(),
                provider: item.provider.clone(),
                source: item.source.clone(),
                source_at: item.source_at.clone(),
                observed_at: item.observed_at.clone(),
                source_batch_id: item.batch_id.clone(),
                source_batch_hash: item.records_hash.clone(),
                content_hash,
            })
        })
        .collect()
}

fn canonical_limits(
    facts: &[UpperLimitFact],
) -> Result<BTreeMap<String, CanonicalLimitFact>, GatewayError> {
    let mut limits = BTreeMap::new();
    for fact in facts {
        if !valid_instrument_id(&fact.instrument_id) {
            return Err(invalid(format!(
                "invalid upper-limit instrument {}",
                fact.instrument_id
            )));
        }
        require_text("security_name", &fact.security_name)?;
        require_text("source_event_id", &fact.source_event_id)?;
        if fact.streak == 0 {
            return Err(invalid(format!(
                "upper-limit streak must be positive for {}",
                fact.instrument_id
            )));
        }
        let normalized = CanonicalLimitFact {
            security_name: fact.security_name.clone(),
            streak: fact.streak,
            source_event_id: fact.source_event_id.clone(),
        };
        match limits.get(&fact.instrument_id) {
            Some(existing) if existing == &normalized => {}
            Some(_) => {
                return Err(GatewayError::classified(
                    "A-10",
                    None,
                    "conflict",
                    "duplicate_limit_identity_conflict",
                    false,
                    format!("conflicting upper-limit facts for {}", fact.instrument_id),
                ))
            }
            None => {
                limits.insert(fact.instrument_id.clone(), normalized);
            }
        }
    }
    Ok(limits)
}

fn canonical_memberships(
    facts: &[BoardMembershipFact],
) -> Result<BTreeMap<(String, String), CanonicalMembership>, GatewayError> {
    let mut memberships = BTreeMap::new();
    for fact in facts {
        if !valid_instrument_id(&fact.instrument_id) {
            return Err(invalid(format!(
                "invalid board-membership instrument {}",
                fact.instrument_id
            )));
        }
        require_text("canonical_board_id", &fact.canonical_board_id)?;
        require_text("board_name", &fact.board_name)?;
        let key = (fact.instrument_id.clone(), fact.canonical_board_id.clone());
        let normalized = CanonicalMembership {
            board_name: fact.board_name.clone(),
            category: fact.category,
        };
        match memberships.get(&key) {
            Some(existing) if existing == &normalized => {}
            Some(_) => {
                return Err(GatewayError::classified(
                    "A-10",
                    None,
                    "conflict",
                    "duplicate_membership_identity_conflict",
                    false,
                    format!(
                        "conflicting board membership for {} and {}",
                        fact.instrument_id, fact.canonical_board_id
                    ),
                ))
            }
            None => {
                memberships.insert(key, normalized);
            }
        }
    }
    Ok(memberships)
}

fn rejection(
    parent_identity_hash: &str,
    ordinal: usize,
    identity: &str,
    reason_code: &str,
    retryable: bool,
) -> Result<ChainRejectionInput, GatewayError> {
    let payload = RejectionHashPayload {
        identity,
        reason_code,
        retryable,
    };
    let identity_hash = canonical_hash("BR160_REJECTION_IDENTITY_V1", &identity)?;
    let content_hash = canonical_hash("BR160_REJECTION_CONTENT_V1", &payload)?;
    let row_identity = canonical_hash(
        "BR160_REJECTION_ROW_ID_V2",
        &(parent_identity_hash, ordinal, &content_hash),
    )?;
    Ok(ChainRejectionInput {
        rejection_id: format!("chain-rejection:{row_identity}"),
        ordinal: i32::try_from(ordinal).map_err(|_| invalid("rejection ordinal overflow"))?,
        identity_hash,
        reason_code: reason_code.to_owned(),
        retryable,
        content_hash,
    })
}

/// Builds one immutable, same-date A-10 batch from already admitted provider
/// facts.  It never guesses a missing name, board membership, streak or source
/// identity.
pub fn build_chain_intelligence_batch(
    trading_date: NaiveDate,
    created_at: DateTime<FixedOffset>,
    policy: &ChainIntelligencePolicy,
    evidence: &[ChainSourceEvidence],
    upper_limits: &[UpperLimitFact],
    board_memberships: &[BoardMembershipFact],
) -> Result<ChainBatchInput, GatewayError> {
    build_chain_intelligence_batch_with_rejections(
        trading_date,
        created_at,
        policy,
        evidence,
        upper_limits,
        board_memberships,
        &[],
    )
}

fn build_chain_intelligence_batch_with_rejections(
    trading_date: NaiveDate,
    created_at: DateTime<FixedOffset>,
    policy: &ChainIntelligencePolicy,
    evidence: &[ChainSourceEvidence],
    upper_limits: &[UpperLimitFact],
    board_memberships: &[BoardMembershipFact],
    source_rejections: &[ChainSourceRejection],
) -> Result<ChainBatchInput, GatewayError> {
    require_text("calculation_version", &policy.calculation_version)?;
    require_text("taxonomy_version", &policy.taxonomy_version)?;
    if !(3..=100).contains(&policy.min_members) {
        return Err(invalid(format!(
            "min_members must be within 3..=100, got {}",
            policy.min_members
        )));
    }
    validate_source_evidence(evidence)?;
    let identity = BatchIdentityPayload {
        trading_date: trading_date.format("%Y-%m-%d").to_string(),
        calculation_version: &policy.calculation_version,
        taxonomy_version: &policy.taxonomy_version,
        ordered_input_batch_ids: evidence.iter().map(|item| item.batch_id.as_str()).collect(),
    };
    let identity_hash = canonical_hash("BR160_BATCH_ID_V1", &identity)?;
    let inputs = canonical_inputs(evidence, &identity_hash)?;
    let limits = canonical_limits(upper_limits)?;
    let memberships = canonical_memberships(board_memberships)?;

    let mut boards = BTreeMap::<String, (String, BTreeMap<String, CanonicalLimitFact>)>::new();
    let mut represented = BTreeSet::new();
    let mut excluded_only = BTreeSet::new();
    for ((instrument_id, board_id), membership) in memberships {
        let Some(limit) = limits.get(&instrument_id) else {
            continue;
        };
        if is_excluded_board(policy, &membership.board_name, membership.category) {
            excluded_only.insert(instrument_id);
            continue;
        }
        represented.insert(instrument_id.clone());
        excluded_only.remove(&instrument_id);
        let entry = boards
            .entry(board_id.clone())
            .or_insert_with(|| (membership.board_name.clone(), BTreeMap::new()));
        if entry.0 != membership.board_name {
            return Err(GatewayError::classified(
                "A-10",
                None,
                "conflict",
                "board_identity_name_conflict",
                false,
                format!("board {board_id} has conflicting names"),
            ));
        }
        entry.1.insert(instrument_id, limit.clone());
    }

    let mut rejection_specs = source_rejections
        .iter()
        .map(|item| {
            require_text("source rejection identity", &item.identity)?;
            require_text("source rejection reason_code", &item.reason_code)?;
            Ok((
                item.identity.clone(),
                item.reason_code.clone(),
                item.retryable,
            ))
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    for instrument_id in limits.keys() {
        if represented.contains(instrument_id) {
            continue;
        }
        let reason = if excluded_only.contains(instrument_id) {
            "board_membership_excluded"
        } else {
            "board_membership_missing"
        };
        rejection_specs.push((instrument_id.clone(), reason.to_owned(), false));
    }

    let mut chains = Vec::new();
    for (board_id, (board_name, members)) in boards {
        if members.len() < policy.min_members {
            rejection_specs.push((board_id, "chain_below_minimum_members".to_owned(), false));
            continue;
        }
        let mut ordered_members = members.into_iter().collect::<Vec<_>>();
        ordered_members.sort_by(|left, right| {
            right
                .1
                .streak
                .cmp(&left.1.streak)
                .then_with(|| left.1.source_event_id.cmp(&right.1.source_event_id))
                .then_with(|| left.0.cmp(&right.0))
        });
        let continuous_count = ordered_members
            .iter()
            .filter(|(_, member)| member.streak >= 2)
            .count();
        let mut stored_members = Vec::with_capacity(ordered_members.len());
        for (ordinal, (instrument_id, member)) in ordered_members.into_iter().enumerate() {
            let payload = MemberHashPayload {
                board_id: &board_id,
                instrument_id: &instrument_id,
                security_name: &member.security_name,
                source_event_id: &member.source_event_id,
                streak: member.streak,
            };
            let content_hash = canonical_hash("BR160_MEMBER_CONTENT_V1", &payload)?;
            stored_members.push(ChainMemberInput {
                member_id: String::new(),
                ordinal: i32::try_from(ordinal).map_err(|_| invalid("member ordinal overflow"))?,
                instrument_id,
                security_name: member.security_name,
                source_event_id: member.source_event_id,
                streak: i32::try_from(member.streak)
                    .map_err(|_| invalid("member streak overflow"))?,
                content_hash,
            });
        }
        let member_content = stored_members
            .iter()
            .map(|member| (member.ordinal, member.content_hash.as_str()))
            .collect::<Vec<_>>();
        let chain_content_hash = canonical_hash(
            "BR160_CHAIN_CONTENT_V2",
            &(&board_id, &board_name, continuous_count, &member_content),
        )?;
        let chain_row_identity = canonical_hash(
            "BR160_CHAIN_ROW_ID_V2",
            &(
                identity_hash.as_str(),
                board_id.as_str(),
                chain_content_hash.as_str(),
            ),
        )?;
        let chain_row_id = format!("chain-row:{chain_row_identity}");
        for member in &mut stored_members {
            let member_row_identity = canonical_hash(
                "BR160_MEMBER_ROW_ID_V2",
                &(
                    chain_row_id.as_str(),
                    member.ordinal,
                    member.content_hash.as_str(),
                ),
            )?;
            member.member_id = format!("chain-member:{member_row_identity}");
        }
        chains.push(ChainInput {
            chain_row_id,
            chain_id: format!("chain:{chain_content_hash}"),
            canonical_board_id: board_id,
            board_name,
            ordinal: 0,
            upper_limit_count: i32::try_from(stored_members.len())
                .map_err(|_| invalid("chain member count overflow"))?,
            continuous_count: i32::try_from(continuous_count)
                .map_err(|_| invalid("continuous member count overflow"))?,
            content_hash: chain_content_hash,
            members: stored_members,
        });
    }
    chains.sort_by(|left, right| {
        right
            .upper_limit_count
            .cmp(&left.upper_limit_count)
            .then_with(|| right.continuous_count.cmp(&left.continuous_count))
            .then_with(|| left.canonical_board_id.cmp(&right.canonical_board_id))
    });
    for (ordinal, chain) in chains.iter_mut().enumerate() {
        chain.ordinal = i32::try_from(ordinal).map_err(|_| invalid("chain ordinal overflow"))?;
    }

    rejection_specs.sort();
    rejection_specs.dedup();
    let rejections = rejection_specs
        .iter()
        .enumerate()
        .map(|(ordinal, (identity, reason_code, retryable))| {
            rejection(&identity_hash, ordinal, identity, reason_code, *retryable)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut batch = ChainBatchInput {
        batch_id: format!("chain-batch:{identity_hash}"),
        content_hash: "0".repeat(64),
        trading_date,
        calculation_version: policy.calculation_version.clone(),
        taxonomy_version: policy.taxonomy_version.clone(),
        created_at,
        inputs,
        chains,
        rejections,
    };
    batch.content_hash = canonical_hash(
        "BR160_BATCH_CONTENT_V1",
        &(
            &batch.batch_id,
            batch.trading_date,
            &batch.calculation_version,
            &batch.taxonomy_version,
            &batch.inputs,
            &batch.chains,
            &batch.rejections,
        ),
    )?;
    Ok(batch)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ChainIntelligenceGateway;

impl ChainIntelligenceGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Builds and publishes one exact-date A-10 batch. Blocking clients never
    /// escape the worker, which avoids dropping their runtimes in async code.
    pub async fn build_for_date(
        &self,
        trading_date: NaiveDate,
    ) -> Result<VisibleChainBatch, GatewayError> {
        let request_hash =
            acquisition_request_hash("A-10-chain-intelligence", &trading_date.to_string());
        let worker_hash = request_hash.clone();
        let joined =
            tokio::task::spawn_blocking(move || build_visible_batch(trading_date, &worker_hash))
                .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                let failure = GatewayError::classified(
                    "A-10",
                    Some(ProviderId::Custom),
                    "unavailable",
                    "blocking_task_failed",
                    true,
                    format!("A-10 blocking worker failed: {error}"),
                );
                tokio::task::spawn_blocking(move || {
                    audit_chain_failure(&request_hash, &failure)?;
                    Err(failure)
                })
                .await
                .unwrap_or_else(|audit_error| {
                    Err(GatewayError::audit_failure(
                        "A-10",
                        ProviderId::Custom,
                        "blocking_task_failed",
                        format!("A-10 blocking failure audit also failed: {audit_error}"),
                    ))
                })
            }
        }
    }
}

fn build_visible_batch(
    trading_date: NaiveDate,
    request_hash: &str,
) -> Result<VisibleChainBatch, GatewayError> {
    let result = acquire_and_build_batch(trading_date)
        .and_then(|batch| persist_and_publish_batch(&batch, request_hash));
    match result {
        Ok(batch) => Ok(batch),
        Err(error) => {
            audit_chain_failure(request_hash, &error)?;
            Err(error)
        }
    }
}

fn acquire_and_build_batch(trading_date: NaiveDate) -> Result<ChainBatchInput, GatewayError> {
    let config = crate::config::get_chain_intelligence_config().ok_or_else(|| {
        GatewayError::classified(
            "A-10",
            None,
            "unavailable",
            "chain_policy_unavailable",
            false,
            "BR-160 chain_intelligence config is unavailable",
        )
    })?;
    let policy = ChainIntelligencePolicy::try_from(config.as_ref())?;
    let created_at = Local::now().fixed_offset();
    let limit_batch = acquire_limit_pool(trading_date)?;
    let mut evidence = vec![source_evidence("A-10-limit-pool", &limit_batch)?];
    if limit_batch.is_verified_empty() {
        return build_chain_intelligence_batch_with_rejections(
            trading_date,
            created_at,
            &policy,
            &evidence,
            &[],
            &[],
            &[],
        );
    }

    let mut requested = Vec::<InstrumentId>::new();
    let mut seen_requested = BTreeMap::<String, InstrumentId>::new();
    let mut source_rejections = Vec::<ChainSourceRejection>::new();
    for record in limit_batch.records() {
        let instrument = &record.instrument;
        if instrument.asset_class() != AssetClass::Equity {
            return Err(invalid(format!(
                "upper-limit source returned non-equity instrument {:?}:{}",
                instrument.exchange(),
                instrument.code()
            )));
        }
        if record.streak.is_none() {
            source_rejections.push(ChainSourceRejection {
                identity: format!("{:?}:{}", instrument.exchange(), instrument.code()),
                reason_code: "upper_limit_streak_missing".to_owned(),
                retryable: false,
            });
            continue;
        }
        match instrument.exchange() {
            Exchange::Beijing => {
                source_rejections.push(ChainSourceRejection {
                    identity: format!("Beijing:{}", instrument.code()),
                    reason_code: "tdx_board_membership_unsupported".to_owned(),
                    retryable: false,
                });
            }
            Exchange::Shanghai | Exchange::Shenzhen => {
                let key = instrument_identity(instrument);
                match seen_requested.get(&key) {
                    Some(existing) if existing == instrument => {}
                    Some(_) => {
                        return Err(invalid(format!(
                            "upper-limit source returned conflicting identity {key}"
                        )))
                    }
                    None => {
                        seen_requested.insert(key, instrument.clone());
                        requested.push(instrument.clone());
                    }
                }
            }
        }
    }

    if requested.is_empty() {
        return build_chain_intelligence_batch_with_rejections(
            trading_date,
            created_at,
            &policy,
            &evidence,
            &[],
            &[],
            &source_rejections,
        );
    }

    let metadata_batch = acquire_security_metadata(&requested)?;
    evidence.push(source_evidence("A-10-security-metadata", &metadata_batch)?);
    let board_batch = acquire_board_memberships(&requested)?;
    evidence.push(source_evidence("A-10-board-memberships", &board_batch)?);

    let mut names = BTreeMap::new();
    for metadata in metadata_batch.records() {
        let name = metadata.name().ok_or_else(|| {
            invalid(format!(
                "TDX security metadata omitted name for {}",
                instrument_identity(metadata.instrument())
            ))
        })?;
        if names
            .insert(instrument_identity(metadata.instrument()), name.to_owned())
            .is_some()
        {
            return Err(invalid(format!(
                "TDX security metadata duplicated {}",
                instrument_identity(metadata.instrument())
            )));
        }
    }
    if names.len() != requested.len() {
        return Err(invalid(format!(
            "TDX security metadata cardinality mismatch requested={} named={}",
            requested.len(),
            names.len()
        )));
    }

    let upper_limits = limit_batch
        .records()
        .iter()
        .filter(|record| {
            matches!(
                record.instrument.exchange(),
                Exchange::Shanghai | Exchange::Shenzhen
            ) && record.streak.is_some()
        })
        .map(|record| {
            let key = instrument_identity(&record.instrument);
            let security_name = names
                .get(&key)
                .ok_or_else(|| invalid(format!("TDX security name missing for {key}")))?;
            Ok(UpperLimitFact {
                instrument_id: record.instrument.code().to_owned(),
                security_name: security_name.clone(),
                streak: record
                    .streak
                    .map(PositiveU32::get)
                    .ok_or_else(|| invalid(format!("upper-limit streak disappeared for {key}")))?,
                source_event_id: format!("{}:{key}", record.evidence.batch_id()),
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    let memberships = board_batch
        .records()
        .iter()
        .map(|record| BoardMembershipFact {
            instrument_id: record.instrument.code().to_owned(),
            canonical_board_id: record.board_code.as_str().to_owned(),
            board_name: record.board_name.as_str().to_owned(),
            category: record.category,
        })
        .collect::<Vec<_>>();
    build_chain_intelligence_batch_with_rejections(
        trading_date,
        created_at,
        &policy,
        &evidence,
        &upper_limits,
        &memberships,
        &source_rejections,
    )
}

fn acquire_limit_pool(
    trading_date: NaiveDate,
) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
    let request_hash = acquisition_request_hash("A-10-limit-pool", &trading_date.to_string());
    let result = route_exact_date_upper_limit_pool("A-10-limit-pool", trading_date);
    audit_routed_gateway_result("A-10-limit-pool", &request_hash, result)
}

fn acquire_security_metadata(
    instruments: &[InstrumentId],
) -> Result<GatewayBatch<SecurityMetadata>, GatewayError> {
    let request_hash = acquisition_request_hash(
        "A-10-security-metadata",
        &canonical_instrument_request(instruments),
    );
    let result = (|| {
        let provider = TdxSmartClient::new();
        let batch = provider
            .security_metadata(instruments)
            .map_err(|error| tdx_error("A-10-security-metadata", error))?;
        validate_metadata_batch(&batch, instruments)?;
        let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
        if batch.records().is_empty() {
            Ok(GatewayBatch::VerifiedEmpty(evidence))
        } else {
            Ok(GatewayBatch::Available {
                records: batch.into_records(),
                evidence,
            })
        }
    })();
    audit_gateway_result(
        "A-10-security-metadata",
        ProviderId::Tdx,
        &request_hash,
        result,
    )
}

fn acquire_board_memberships(
    instruments: &[InstrumentId],
) -> Result<GatewayBatch<BoardMembership>, GatewayError> {
    let request_hash = acquisition_request_hash(
        "A-10-board-memberships",
        &canonical_instrument_request(instruments),
    );
    let result = (|| {
        let (_, ip, port) = PRIMARY_SERVERS.first().copied().ok_or_else(|| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Tdx),
                "unavailable",
                "tdx_endpoint_unavailable",
                false,
                "TDX primary server list is empty",
            )
        })?;
        let provider = BlockService::new(ip, port, 5.0);
        let batch = provider
            .board_memberships(instruments)
            .map_err(|error| tdx_error("A-10-board-memberships", error))?;
        validate_board_batch(&batch, instruments)?;
        let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
        if batch.records().is_empty() {
            Ok(GatewayBatch::VerifiedEmpty(evidence))
        } else {
            Ok(GatewayBatch::Available {
                records: batch.into_records(),
                evidence,
            })
        }
    })();
    audit_gateway_result(
        "A-10-board-memberships",
        ProviderId::Tdx,
        &request_hash,
        result,
    )
}

fn validate_metadata_batch(
    batch: &DataBatch<SecurityMetadata>,
    requested: &[InstrumentId],
) -> Result<(), GatewayError> {
    let batch_id = batch.provenance().batch_id().ok_or_else(|| {
        GatewayError::invalid_evidence(
            "A-10",
            Some(ProviderId::Tdx),
            "security-metadata batch ID is absent",
        )
    })?;
    let expected = requested
        .iter()
        .map(instrument_identity)
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for record in batch.records() {
        if record.provider() != ProviderId::Tdx
            || record.batch_id() != batch_id
            || record.observed_at() != batch.provenance().fetched_at()
            || record.name().is_none()
        {
            return Err(GatewayError::invalid_evidence(
                "A-10",
                Some(ProviderId::Tdx),
                "TDX security identity record is missing name or batch evidence",
            ));
        }
        let identity = instrument_identity(record.instrument());
        if !expected.contains(&identity) || !actual.insert(identity) {
            return Err(GatewayError::invalid_evidence(
                "A-10",
                Some(ProviderId::Tdx),
                "TDX security identity response is duplicate or unrequested",
            ));
        }
    }
    if actual != expected {
        return Err(GatewayError::invalid_evidence(
            "A-10",
            Some(ProviderId::Tdx),
            "TDX security identity response omitted a requested instrument",
        ));
    }
    Ok(())
}

fn validate_board_batch(
    batch: &DataBatch<BoardMembership>,
    requested: &[InstrumentId],
) -> Result<(), GatewayError> {
    let batch_id = batch.provenance().batch_id().ok_or_else(|| {
        GatewayError::invalid_evidence(
            "A-10",
            Some(ProviderId::Tdx),
            "board-membership batch ID is absent",
        )
    })?;
    let expected = requested
        .iter()
        .map(instrument_identity)
        .collect::<BTreeSet<_>>();
    let mut identities = BTreeSet::new();
    for record in batch.records() {
        if record.evidence.provider() != ProviderId::Tdx
            || record.evidence.batch_id() != batch_id
            || record.evidence.observed_at() != batch.provenance().fetched_at()
            || !expected.contains(&instrument_identity(&record.instrument))
        {
            return Err(GatewayError::invalid_evidence(
                "A-10",
                Some(ProviderId::Tdx),
                "TDX board-membership record differs from request or batch evidence",
            ));
        }
        let identity = (
            instrument_identity(&record.instrument),
            record.board_code.as_str().to_owned(),
        );
        if !identities.insert(identity) {
            return Err(GatewayError::invalid_evidence(
                "A-10",
                Some(ProviderId::Tdx),
                "TDX board-membership response contains a duplicate identity",
            ));
        }
    }
    Ok(())
}

fn source_evidence<T: Serialize>(
    capability: &str,
    batch: &GatewayBatch<T>,
) -> Result<ChainSourceEvidence, GatewayError> {
    let source = batch.evidence();
    Ok(ChainSourceEvidence {
        capability: capability.to_owned(),
        provider: format!("{:?}", source.provider),
        source: source.source.clone(),
        source_at: source.source_at.clone(),
        observed_at: source.observed_at.clone(),
        batch_id: source.batch_id.clone(),
        records_hash: canonical_records_hash(capability, batch.records())?,
    })
}

fn canonical_records_hash<T: Serialize>(
    label: &str,
    records: &[T],
) -> Result<String, GatewayError> {
    let mut rows = records
        .iter()
        .map(|record| {
            serde_json::to_vec(record)
                .map_err(|error| invalid(format!("serialize {label} source record: {error}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort();
    canonical_hash("BR160_SOURCE_RECORDS_V1", &(label, rows))
}

fn canonical_instrument_request(instruments: &[InstrumentId]) -> String {
    let mut identities = instruments
        .iter()
        .map(instrument_identity)
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();
    identities.join(",")
}

fn instrument_identity(instrument: &InstrumentId) -> String {
    format!(
        "{:?}:{:?}:{}",
        instrument.exchange(),
        instrument.asset_class(),
        instrument.code()
    )
}

fn persist_and_publish_batch(
    batch: &ChainBatchInput,
    request_hash: &str,
) -> Result<VisibleChainBatch, GatewayError> {
    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            "A-10",
            ProviderId::Custom,
            "database_unavailable",
            "database is not initialized",
        )
    })?;
    database
        .stage_chain_intelligence_batch(batch)
        .map_err(|error| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "conflict",
                "chain_stage_failed",
                false,
                error,
            )
        })?;
    if let Some(visible) = database
        .load_visible_chain_intelligence_batch(&batch.batch_id)
        .map_err(|error| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "unavailable",
                "chain_read_failed",
                true,
                error,
            )
        })?
    {
        return Ok(visible);
    }

    let audit_hash = audit_chain_success(database, batch, request_hash)?;
    let receipt_hash = canonical_hash(
        "BR160_VISIBILITY_RECEIPT_ID_V1",
        &(&batch.batch_id, &audit_hash, &batch.content_hash),
    )?;
    let receipt = ChainVisibilityReceiptInput {
        receipt_id: format!("chain-visibility:{receipt_hash}"),
        batch_id: batch.batch_id.clone(),
        audit_record_hash: audit_hash,
        content_hash: batch.content_hash.clone(),
        published_at: Local::now().fixed_offset(),
    };
    database
        .publish_chain_intelligence_batch(&receipt)
        .map_err(|error| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "conflict",
                "chain_publish_failed",
                false,
                error,
            )
        })?;
    database
        .load_visible_chain_intelligence_batch(&batch.batch_id)
        .map_err(|error| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "unavailable",
                "chain_read_failed",
                true,
                error,
            )
        })?
        .ok_or_else(|| {
            GatewayError::classified(
                "A-10",
                Some(ProviderId::Custom),
                "unavailable",
                "visibility_receipt_missing",
                true,
                "published A-10 batch is not visible",
            )
        })
}

fn audit_chain_success(
    database: &DatabaseManager,
    batch: &ChainBatchInput,
    request_hash: &str,
) -> Result<String, GatewayError> {
    let observed_at = batch
        .created_at
        .to_rfc3339_opts(SecondsFormat::Millis, false);
    let source_at = batch.trading_date.format("%Y-%m-%d").to_string();
    let accepted_count = i64::try_from(batch.chains.len())
        .map_err(|_| invalid("A-10 accepted chain count exceeds SQLite INTEGER"))?;
    let rejected_count = i64::try_from(batch.rejections.len())
        .map_err(|_| invalid("A-10 rejection count exceeds SQLite INTEGER"))?;
    let verified_empty = batch.chains.is_empty();
    let reason_code = if verified_empty {
        "no_eligible_chain"
    } else {
        "accepted"
    };
    let record = DataAcquisitionAuditRecord {
        capability: "A-10-chain-intelligence",
        provider: "UnifiedGateway",
        source: "chain-intelligence-gateway",
        request_hash,
        source_at: Some(&source_at),
        observed_at: &observed_at,
        batch_id: Some(&batch.batch_id),
        outcome: if verified_empty {
            "verified_empty"
        } else {
            "available"
        },
        request_count: 1,
        accepted_count,
        rejected_count,
        reason_code,
        retryable: false,
    };
    database
        .record_data_acquisition(&record)
        .map(|receipt| receipt.record_hash)
        .map_err(|error| {
            GatewayError::audit_failure("A-10", ProviderId::Custom, reason_code, error)
        })
}

fn audit_chain_failure(request_hash: &str, error: &GatewayError) -> Result<(), GatewayError> {
    let database = DatabaseManager::try_get().ok_or_else(|| {
        GatewayError::audit_failure(
            "A-10",
            ProviderId::Custom,
            error.reason_code(),
            "database is not initialized",
        )
    })?;
    let observed_at = Local::now()
        .fixed_offset()
        .to_rfc3339_opts(SecondsFormat::Millis, false);
    let record = DataAcquisitionAuditRecord {
        capability: "A-10-chain-intelligence",
        provider: "UnifiedGateway",
        source: "chain-intelligence-gateway",
        request_hash,
        source_at: None,
        observed_at: &observed_at,
        batch_id: None,
        outcome: error.audit_outcome(),
        request_count: 1,
        accepted_count: 0,
        rejected_count: 1,
        reason_code: error.reason_code(),
        retryable: error.retryable(),
    };
    database
        .record_data_acquisition(&record)
        .map(|_| ())
        .map_err(|audit_error| {
            GatewayError::audit_failure(
                "A-10",
                ProviderId::Custom,
                error.reason_code(),
                audit_error,
            )
        })
}

fn tdx_error(capability: &'static str, error: TdxError) -> GatewayError {
    let message = error.to_string();
    match error {
        TdxError::Io(_)
        | TdxError::Connection(_)
        | TdxError::ConnectionTimeout
        | TdxError::SetupFailed(_)
        | TdxError::Disconnected
        | TdxError::RetryExhausted(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        TdxError::Unsupported(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        TdxError::HistoricalBarCardinality {
            offset,
            actual,
            expected_page,
            requested_total,
        } => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "partial",
            "provider_batch_rejected",
            false,
            format!(
                "Magic TDX historical-bar cardinality mismatch: offset={offset} actual={actual} \
                 expected_page={expected_page} requested_total={requested_total}"
            ),
        ),
        TdxError::Parse(_)
        | TdxError::InvalidData(_)
        | TdxError::ResponseParse(_)
        | TdxError::Core(_)
        | TdxError::Coded(_)
        | TdxError::FileNotFound(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "partial",
            "provider_batch_rejected",
            false,
            message,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2099-01-02T16:00:00+08:00").expect("timestamp")
    }

    fn evidence() -> Vec<ChainSourceEvidence> {
        vec![
            ChainSourceEvidence {
                capability: "TEST_CODE_limit_pool".to_owned(),
                provider: "TEST_CODE_eastmoney".to_owned(),
                source: "TEST_CODE_limit_pool".to_owned(),
                source_at: Some("2099-01-02".to_owned()),
                observed_at: "4070995200".to_owned(),
                batch_id: "TEST_CODE_limit_batch".to_owned(),
                records_hash: "a".repeat(64),
            },
            ChainSourceEvidence {
                capability: "TEST_CODE_board_memberships".to_owned(),
                provider: "TEST_CODE_tdx".to_owned(),
                source: "TEST_CODE_block_files".to_owned(),
                source_at: None,
                observed_at: "4070995200123456789".to_owned(),
                batch_id: "TEST_CODE_board_batch".to_owned(),
                records_hash: "b".repeat(64),
            },
            ChainSourceEvidence {
                capability: "TEST_CODE_security_metadata".to_owned(),
                provider: "TEST_CODE_tdx".to_owned(),
                source: "TEST_CODE_security_list".to_owned(),
                source_at: None,
                observed_at: "4070995200".to_owned(),
                batch_id: "TEST_CODE_metadata_batch".to_owned(),
                records_hash: "c".repeat(64),
            },
        ]
    }

    fn policy() -> ChainIntelligencePolicy {
        ChainIntelligencePolicy {
            calculation_version: "TEST_CODE_chain-intelligence-v1".to_owned(),
            taxonomy_version: "TEST_CODE_tdx-board-exclusions-v1".to_owned(),
            min_members: 3,
            excluded_board_names: ["东方财富热股".to_owned()].into_iter().collect(),
        }
    }

    fn limits() -> Vec<UpperLimitFact> {
        [
            ("000001", "甲公司", 1),
            ("000002", "乙公司", 3),
            ("000003", "丙公司", 2),
            ("000004", "丁公司", 1),
        ]
        .into_iter()
        .map(|(code, name, streak)| UpperLimitFact {
            instrument_id: code.to_owned(),
            security_name: name.to_owned(),
            streak,
            source_event_id: format!("TEST_CODE_limit_event_{code}"),
        })
        .collect()
    }

    fn memberships() -> Vec<BoardMembershipFact> {
        [
            ("000001", "tdx:gn.dat:主线甲", "主线甲"),
            ("000002", "tdx:gn.dat:主线甲", "主线甲"),
            ("000003", "tdx:gn.dat:主线甲", "主线甲"),
            ("000004", "tdx:gn.dat:东方财富热股", "东方财富热股"),
        ]
        .into_iter()
        .map(|(code, board_id, board_name)| BoardMembershipFact {
            instrument_id: code.to_owned(),
            canonical_board_id: board_id.to_owned(),
            board_name: board_name.to_owned(),
            category: BoardCategory::Concept,
        })
        .collect()
    }

    #[test]
    fn br160_build_is_deterministic_and_filters_generic_boards() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let first = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits(),
            &memberships(),
        )
        .expect("first build");
        let mut reversed_limits = limits();
        reversed_limits.reverse();
        let mut reversed_memberships = memberships();
        reversed_memberships.reverse();
        let second = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &reversed_limits,
            &reversed_memberships,
        )
        .expect("second build");
        assert_eq!(first, second);
        assert_eq!(first.chains.len(), 1);
        assert_eq!(first.chains[0].board_name, "主线甲");
        assert_eq!(
            first.chains[0]
                .members
                .iter()
                .map(|member| member.instrument_id.as_str())
                .collect::<Vec<_>>(),
            vec!["000002", "000003", "000001"]
        );
        assert!(first
            .rejections
            .iter()
            .any(|rejection| { rejection.reason_code == "board_membership_excluded" }));
    }

    #[test]
    fn br160_stable_provider_input_gets_a_parent_scoped_evidence_identity() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let first_evidence = evidence();
        let mut second_evidence = evidence();
        second_evidence[0].batch_id = "TEST_CODE_limit_batch_next_observation".to_owned();
        second_evidence[0].observed_at = "4070995260".to_owned();

        let first = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &first_evidence,
            &limits(),
            &memberships(),
        )
        .expect("first derived batch");
        let second = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &second_evidence,
            &limits(),
            &memberships(),
        )
        .expect("second derived batch");
        let first_board = first
            .inputs
            .iter()
            .find(|input| input.capability == "TEST_CODE_board_memberships")
            .expect("first stable board input");
        let second_board = second
            .inputs
            .iter()
            .find(|input| input.capability == "TEST_CODE_board_memberships")
            .expect("second stable board input");

        assert_ne!(first.batch_id, second.batch_id);
        assert_eq!(first_board.source_batch_id, second_board.source_batch_id);
        assert_ne!(
            first_board.input_id, second_board.input_id,
            "the child evidence identity must bind the parent derived batch"
        );
        assert_ne!(
            first.chains[0].chain_row_id, second.chains[0].chain_row_id,
            "the chain row identity must bind the parent derived batch"
        );
        assert_ne!(
            first.chains[0].members[0].member_id, second.chains[0].members[0].member_id,
            "the member row identity must bind the parent chain row"
        );
        assert_ne!(
            first.rejections[0].rejection_id, second.rejections[0].rejection_id,
            "the rejection row identity must bind the parent derived batch"
        );
    }

    #[test]
    fn br160_conflicting_membership_identity_rejects_whole_batch() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let mut conflict = memberships();
        conflict.push(BoardMembershipFact {
            instrument_id: "000001".to_owned(),
            canonical_board_id: "tdx:gn.dat:主线甲".to_owned(),
            board_name: "冲突名称".to_owned(),
            category: BoardCategory::Concept,
        });
        let error = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits(),
            &conflict,
        )
        .expect_err("conflicting membership must fail");
        assert_eq!(
            error.reason_code(),
            "duplicate_membership_identity_conflict"
        );
    }

    #[test]
    fn br160_missing_membership_is_rejected_not_fabricated() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let batch = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits()[..3],
            &memberships()[..2],
        )
        .expect("build with missing membership");
        assert!(batch.chains.is_empty());
        assert!(batch
            .rejections
            .iter()
            .any(|rejection| rejection.reason_code == "board_membership_missing"));
        assert!(batch
            .rejections
            .iter()
            .any(|rejection| rejection.reason_code == "chain_below_minimum_members"));
    }

    #[test]
    fn br160_source_unsupported_identity_is_retained_as_hashed_rejection() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let batch = build_chain_intelligence_batch_with_rejections(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits()[..3],
            &memberships()[..3],
            &[ChainSourceRejection {
                identity: "Beijing:TEST_CODE_920001".to_owned(),
                reason_code: "tdx_board_membership_unsupported".to_owned(),
                retryable: false,
            }],
        )
        .expect("unsupported source identity remains explicit");
        let rejection = batch
            .rejections
            .iter()
            .find(|item| item.reason_code == "tdx_board_membership_unsupported")
            .expect("source rejection");
        assert_eq!(rejection.identity_hash.len(), 64);
        assert!(!format!("{rejection:?}").contains("TEST_CODE_920001"));
    }

    #[test]
    fn br160_policy_and_evidence_boundaries_fail_closed() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        for mutate in 0..8 {
            let mut value = evidence();
            match mutate {
                0 => value.clear(),
                1 => value[0].capability.clear(),
                2 => value[0].provider = "TEST_CODE_bad\nprovider".to_owned(),
                3 => value[0].source_at = Some("   ".to_owned()),
                4 => value[0].records_hash = "not-a-hash".to_owned(),
                5 => value[1].batch_id = value[0].batch_id.clone(),
                6 => value[0].observed_at.clear(),
                7 => value[0].source.clear(),
                _ => unreachable!(),
            }
            assert!(
                build_chain_intelligence_batch(
                    date,
                    timestamp(),
                    &policy(),
                    &value,
                    &limits(),
                    &memberships(),
                )
                .is_err(),
                "evidence mutation {mutate} must fail"
            );
        }

        for min_members in [0, 2, 101] {
            let mut bad_policy = policy();
            bad_policy.min_members = min_members;
            assert!(build_chain_intelligence_batch(
                date,
                timestamp(),
                &bad_policy,
                &evidence(),
                &limits(),
                &memberships(),
            )
            .is_err());
        }
        for clear_calculation in [true, false] {
            let mut bad_policy = policy();
            if clear_calculation {
                bad_policy.calculation_version.clear();
            } else {
                bad_policy.taxonomy_version.clear();
            }
            assert!(build_chain_intelligence_batch(
                date,
                timestamp(),
                &bad_policy,
                &evidence(),
                &limits(),
                &memberships(),
            )
            .is_err());
        }
    }

    #[test]
    fn br160_limit_and_membership_identity_boundaries_are_atomic() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let assert_limits_fail = |bad_limits: Vec<UpperLimitFact>| {
            assert!(build_chain_intelligence_batch(
                date,
                timestamp(),
                &policy(),
                &evidence(),
                &bad_limits,
                &memberships(),
            )
            .is_err());
        };
        let mut invalid_code = limits();
        invalid_code[0].instrument_id = "TEST_CODE".to_owned();
        assert_limits_fail(invalid_code);
        let mut missing_name = limits();
        missing_name[0].security_name.clear();
        assert_limits_fail(missing_name);
        let mut missing_event = limits();
        missing_event[0].source_event_id.clear();
        assert_limits_fail(missing_event);
        let mut zero_streak = limits();
        zero_streak[0].streak = 0;
        assert_limits_fail(zero_streak);
        let mut duplicate_conflict = limits();
        duplicate_conflict.push(UpperLimitFact {
            instrument_id: "000001".to_owned(),
            security_name: "冲突公司".to_owned(),
            streak: 2,
            source_event_id: "TEST_CODE_conflict".to_owned(),
        });
        let error = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &duplicate_conflict,
            &memberships(),
        )
        .expect_err("conflicting limit identity");
        assert_eq!(error.reason_code(), "duplicate_limit_identity_conflict");

        let mut exact_duplicate = limits();
        exact_duplicate.push(exact_duplicate[0].clone());
        assert!(build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &exact_duplicate,
            &memberships(),
        )
        .is_ok());

        for mutation in 0..3 {
            let mut bad = memberships();
            match mutation {
                0 => bad[0].instrument_id = "TEST_CODE".to_owned(),
                1 => bad[0].canonical_board_id.clear(),
                2 => bad[0].board_name = "\n".to_owned(),
                _ => unreachable!(),
            }
            assert!(build_chain_intelligence_batch(
                date,
                timestamp(),
                &policy(),
                &evidence(),
                &limits(),
                &bad,
            )
            .is_err());
        }
        let mut duplicate_membership = memberships();
        duplicate_membership.push(duplicate_membership[0].clone());
        assert!(build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits(),
            &duplicate_membership,
        )
        .is_ok());
    }

    #[test]
    fn br160_board_conflicts_categories_and_ordering_are_explicit() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        let mut conflicting_names = memberships();
        conflicting_names[3].canonical_board_id = "tdx:gn.dat:主线甲".to_owned();
        conflicting_names[3].board_name = "同ID不同名称".to_owned();
        let error = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits(),
            &conflicting_names,
        )
        .expect_err("board identity/name conflict");
        assert_eq!(error.reason_code(), "board_identity_name_conflict");

        let mut non_chain_categories = memberships();
        non_chain_categories[0].category = BoardCategory::Region;
        non_chain_categories[1].category = BoardCategory::Unknown;
        non_chain_categories[2].category = BoardCategory::Region;
        let batch = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &limits(),
            &non_chain_categories,
        )
        .expect("unsupported board categories become explicit rejections");
        assert!(batch.chains.is_empty());
        assert!(batch
            .rejections
            .iter()
            .any(|item| item.reason_code == "board_membership_excluded"));

        let mut expanded_limits = limits();
        expanded_limits.extend([
            UpperLimitFact {
                instrument_id: "000005".to_owned(),
                security_name: "戊公司".to_owned(),
                streak: 4,
                source_event_id: "TEST_CODE_limit_event_000005".to_owned(),
            },
            UpperLimitFact {
                instrument_id: "000006".to_owned(),
                security_name: "己公司".to_owned(),
                streak: 2,
                source_event_id: "TEST_CODE_limit_event_000006".to_owned(),
            },
            UpperLimitFact {
                instrument_id: "000007".to_owned(),
                security_name: "庚公司".to_owned(),
                streak: 1,
                source_event_id: "TEST_CODE_limit_event_000007".to_owned(),
            },
        ]);
        let mut expanded_memberships = memberships();
        expanded_memberships.extend(["000005", "000006", "000007"].into_iter().map(|code| {
            BoardMembershipFact {
                instrument_id: code.to_owned(),
                canonical_board_id: "tdx:hy.dat:主线乙".to_owned(),
                board_name: "主线乙".to_owned(),
                category: BoardCategory::Industry,
            }
        }));
        let ordered = build_chain_intelligence_batch(
            date,
            timestamp(),
            &policy(),
            &evidence(),
            &expanded_limits,
            &expanded_memberships,
        )
        .expect("two eligible chains");
        assert_eq!(ordered.chains.len(), 2);
        assert_eq!(ordered.chains[0].board_name, "主线甲");
        assert_eq!(ordered.chains[0].ordinal, 0);
        assert_eq!(ordered.chains[1].board_name, "主线乙");
        assert_eq!(ordered.chains[1].ordinal, 1);
    }

    #[test]
    fn br160_source_rejections_and_canonical_helpers_preserve_identity_rules() {
        let date = NaiveDate::from_ymd_opt(2099, 1, 2).expect("date");
        for mutation in 0..2 {
            let rejection = ChainSourceRejection {
                identity: if mutation == 0 {
                    String::new()
                } else {
                    "TEST_CODE_identity".to_owned()
                },
                reason_code: if mutation == 1 {
                    String::new()
                } else {
                    "TEST_CODE_reason".to_owned()
                },
                retryable: true,
            };
            assert!(build_chain_intelligence_batch_with_rejections(
                date,
                timestamp(),
                &policy(),
                &evidence(),
                &limits()[..3],
                &memberships()[..3],
                &[rejection],
            )
            .is_err());
        }

        let sh = InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap();
        let sz = InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity).unwrap();
        assert_eq!(
            canonical_instrument_request(&[sh.clone(), sz.clone(), sh]),
            "Shanghai:Equity:600396,Shenzhen:Equity:000001"
        );
        assert_eq!(instrument_identity(&sz), "Shenzhen:Equity:000001");
        let _gateway = ChainIntelligenceGateway::new();

        let batch = GatewayBatch::Available {
            records: vec!["TEST_CODE_b", "TEST_CODE_a"],
            evidence: BatchEvidence {
                provider: ProviderId::Custom,
                source: "TEST_CODE_source".to_owned(),
                source_at: Some("2099-01-02".to_owned()),
                observed_at: "4070995200".to_owned(),
                batch_id: "TEST_CODE_batch".to_owned(),
            },
        };
        let source = source_evidence("TEST_CODE_capability", &batch).unwrap();
        assert_eq!(source.provider, "Custom");
        assert_eq!(source.records_hash.len(), 64);
        let reverse = GatewayBatch::Available {
            records: vec!["TEST_CODE_a", "TEST_CODE_b"],
            evidence: batch.evidence().clone(),
        };
        assert_eq!(
            source.records_hash,
            source_evidence("TEST_CODE_capability", &reverse)
                .unwrap()
                .records_hash
        );
    }

    #[test]
    fn tdx_error_classifier_keeps_retryability_and_outcomes_distinct() {
        let tdx = [
            tdx_error(
                "TEST_CODE_capability",
                TdxError::Io(std::io::Error::other("TEST_CODE_io")),
            ),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::Connection("TEST_CODE_connection".to_owned()),
            ),
            tdx_error("TEST_CODE_capability", TdxError::ConnectionTimeout),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::SetupFailed("TEST_CODE_setup".to_owned()),
            ),
            tdx_error("TEST_CODE_capability", TdxError::Disconnected),
            tdx_error("TEST_CODE_capability", TdxError::RetryExhausted(1)),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::Unsupported("TEST_CODE_no".to_owned()),
            ),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::Parse("TEST_CODE_parse".to_owned()),
            ),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::InvalidData("TEST_CODE_invalid".to_owned()),
            ),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::ResponseParse("TEST_CODE_response".to_owned()),
            ),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::FileNotFound("TEST_CODE_missing".to_owned()),
            ),
            tdx_error(
                "TEST_CODE_capability",
                TdxError::Core(crate::magic_compat::NonEmptyText::new("").expect_err("core error")),
            ),
        ];
        assert!(tdx[..6].iter().all(GatewayError::retryable));
        assert_eq!(tdx[6].audit_outcome(), "unsupported");
        assert!(tdx[7..].iter().all(|error| !error.retryable()));

        let cardinality = tdx_error(
            "TEST_CODE_capability",
            TdxError::HistoricalBarCardinality {
                offset: 800,
                actual: 99,
                expected_page: 100,
                requested_total: 900,
            },
        );
        assert_eq!(cardinality.audit_outcome(), "partial");
        assert_eq!(cardinality.reason_code(), "provider_batch_rejected");
        assert!(!cardinality.retryable());
        let cardinality_message = cardinality.to_string();
        for expected in [
            "offset=800",
            "actual=99",
            "expected_page=100",
            "requested_total=900",
        ] {
            assert!(cardinality_message.contains(expected));
        }
    }
}
