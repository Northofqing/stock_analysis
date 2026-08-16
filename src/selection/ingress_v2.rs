//! Deterministic BR-174 source-ingress preparation.
//!
//! This module is deliberately storage-free. It consumes one opaque,
//! source-admitted [`RawNewsAggregationBatch`] and converts it into the exact
//! schema-v2 rows and recovery envelope that the SQLite owner must later commit
//! with `synchronous=FULL`. Notification projection and simhash mutation happen
//! only after that commit.

use crate::news::aggregator::raw_v2::{
    registered_global_news_feeds, FeedUnavailable, RawGlobalNewsFeedAttempt,
    RawGlobalNewsTerminalKind, RawNewsAggregationBatch, RegisteredGlobalNewsFeed,
    MAGIC_MARKET_DATA_REVISION,
};
use crate::selection::schema_v2::{
    build_request_evidence, canonical_json, sha256_bytes, sha256_json,
    AcquiredGlobalNewsRecordPreimage, ErrorFingerprintPreimage, FeedAttemptContentPreimage,
    FeedAttemptKeyPreimage, FeedAvailableEvidencePreimage, FeedBatchEvidencePreimage,
    FeedBatchQuality, FeedSourceContentPreimage, FeedSourceRecordHashPreimage, FeedStatusKind,
    GlobalNewsRequestParametersPreimage, IngressDecision, IngressGateInputPreimage,
    IngressGateReceiptPreimage, ProviderCapabilityHashPreimage, ProviderErrorDetailPreimage,
    ProviderErrorKind, RequestEvidenceColumns, RequestParametersPreimage,
    RunLogicalSubjectPreimage, RunPayloadPreimage, RunRowHashPreimage,
    RunRowLogicalPrimaryKeyPreimage, RunStatus, SelectionRecoveryEnvelopeRowContentPreimage,
    SelectionSourceBatchAttemptRowContentPreimage, SelectionSourceFactAttemptRowContentPreimage,
    SelectionSourceFactRowContentPreimage, SourceBatchContentPreimage, SourceFactAttemptPreimage,
    SourceFactAttemptResult, SourceFactConflictPreimage, SourceFactContentPreimage,
    SourceFactKeyPreimage, SourceIngressStageInputPreimage, SubjectKind,
    DOMAIN_ACQUIRED_GLOBAL_NEWS_RECORD, DOMAIN_ERROR_FINGERPRINT, DOMAIN_FEED_ATTEMPT_CONTENT,
    DOMAIN_FEED_ATTEMPT_KEY, DOMAIN_FEED_AVAILABLE_EVIDENCE, DOMAIN_FEED_BATCH_EVIDENCE,
    DOMAIN_FEED_SOURCE_CONTENT, DOMAIN_FEED_SOURCE_RECORD, DOMAIN_GLOBAL_NEWS_REQUEST,
    DOMAIN_INGRESS_GATE_INPUT, DOMAIN_INGRESS_GATE_RECEIPT, DOMAIN_INGRESS_PAYLOAD,
    DOMAIN_PROVIDER_CAPABILITY, DOMAIN_PROVIDER_ERROR_DETAIL, DOMAIN_RECOVERY_ENVELOPE_ROW,
    DOMAIN_REGISTERED_FEED_CONFIG, DOMAIN_REGISTERED_FEED_IDENTITY,
    DOMAIN_REGISTERED_FEED_SNAPSHOT, DOMAIN_RUN_LOGICAL_SUBJECT, DOMAIN_RUN_ROW,
    DOMAIN_RUN_ROW_LOGICAL_PK, DOMAIN_SOURCE_ATTEMPT, DOMAIN_SOURCE_BATCH_ATTEMPT_ROW,
    DOMAIN_SOURCE_BATCH_CONTENT, DOMAIN_SOURCE_CONTENT, DOMAIN_SOURCE_FACT_ATTEMPT_ROW,
    DOMAIN_SOURCE_FACT_CONFLICT, DOMAIN_SOURCE_FACT_KEY, DOMAIN_SOURCE_FACT_ROW,
    DOMAIN_SOURCE_INGRESS_STAGE, TABLE_SOURCE_BATCH_ATTEMPT, TABLE_SOURCE_FACT,
    TABLE_SOURCE_FACT_ATTEMPT,
};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const GLOBAL_NEWS_CONTRACT_VERSION: &str = "magic-market-core.NewsProvider.global_news.v0.2.0";
const GLOBAL_NEWS_SOURCE_FACT_SCHEMA: &str = "global-news-source-fact-v2";
const SOURCE_INGRESS_PAYLOAD_SCHEMA: &str = "source-ingress-stage-v2";
const EVENT_ID_DOMAIN: &[u8] = b"BR166_GLOBAL_NEWS_EVENT_V1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingSourceFactAuthority {
    pub source_fact_key: String,
    pub provider_content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIngressPreparationContext {
    pub stage_run_id: String,
    pub config_activation_run_id: String,
    pub config_hash: String,
    pub config_effective_from: DateTime<Utc>,
    pub generation_market_date: NaiveDate,
    pub per_feed_limit: u32,
    pub gate_version: String,
    pub freshness_max_age_secs: u64,
    pub future_tolerance_secs: u64,
    pub evaluated_at: DateTime<Utc>,
    pub enveloped_at: DateTime<Utc>,
}

/// Opaque source-ingress preparation capability.
///
/// External crates may move it into `commit_source_ingress`, but cannot
/// inspect or replace its canonical staging preimages:
///
/// ```compile_fail
/// use stock_analysis::selection::ingress_v2::PreparedSourceIngress;
///
/// fn leak_stage(value: PreparedSourceIngress) {
///     let PreparedSourceIngress { stage_input, .. } = value;
///     drop(stage_input);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSourceIngress {
    stage_input: SourceIngressStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl PreparedSourceIngress {
    pub(crate) fn stage_input(&self) -> &SourceIngressStageInputPreimage {
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
pub struct IngressPreparationError {
    pub code: &'static str,
    pub detail: &'static str,
}

impl IngressPreparationError {
    const fn new(code: &'static str, detail: &'static str) -> Self {
        Self { code, detail }
    }
}

impl fmt::Display for IngressPreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for IngressPreparationError {}

impl From<crate::selection::schema_v2::SchemaV2Error> for IngressPreparationError {
    fn from(_: crate::selection::schema_v2::SchemaV2Error) -> Self {
        Self::new(
            "schema_v2_preimage_invalid",
            "schema-v2 canonical preimage validation failed",
        )
    }
}

struct RegisteredAttempt<'a> {
    registration: RegisteredGlobalNewsFeed,
    configuration_hash: String,
    feed_identity: String,
    attempt: &'a RawGlobalNewsFeedAttempt,
}

struct PreparedRecord {
    source_fact_key: String,
    provider_content_hash: String,
    content: SourceFactContentPreimage,
    event_id: String,
}

struct FeedPreparation {
    row: SelectionSourceBatchAttemptRowContentPreimage,
    feed_attempt_content_hash: String,
    record_hashes: Vec<String>,
    event_ids: Vec<String>,
    records: Vec<PreparedRecord>,
    evidence_json: Option<String>,
    evidence_hash: Option<String>,
    source_content_hash: Option<String>,
}

struct AuthorityState {
    provider_content_hash: String,
    created_in_run: bool,
}

/// Prepare one immutable ingress run. The caller supplies only previously
/// committed source-fact authorities; this function performs no database read
/// or write and never projects notification events.
pub fn prepare_source_ingress(
    batch: &RawNewsAggregationBatch,
    context: &SourceIngressPreparationContext,
    existing_authorities: &[ExistingSourceFactAuthority],
) -> Result<PreparedSourceIngress, IngressPreparationError> {
    validate_context(batch, context)?;
    let registered = bind_registered_attempts(batch)?;
    let (registered_feed_snapshot_json, registered_feed_snapshot_hash) =
        registered_feed_snapshot(&registered)?;
    let mut authorities = authoritative_snapshot(existing_authorities)?;

    let mut feed_preparations = Vec::with_capacity(registered.len());
    for entry in &registered {
        feed_preparations.push(prepare_feed(
            entry,
            context,
            &registered_feed_snapshot_hash,
            batch.observed_at(),
        )?);
    }

    let feed_attempt_hashes = feed_preparations
        .iter()
        .map(|prepared| prepared.feed_attempt_content_hash.clone())
        .collect::<Vec<_>>();
    let source_record_hashes = feed_preparations
        .iter()
        .flat_map(|prepared| prepared.record_hashes.iter().cloned())
        .collect::<Vec<_>>();
    let event_projection_ids = feed_preparations
        .iter()
        .flat_map(|prepared| prepared.event_ids.iter().cloned())
        .collect::<Vec<_>>();
    let aggregator_observed_at = rfc3339_nanos(batch.observed_at());
    let source_batch_preimage = SourceBatchContentPreimage {
        domain: DOMAIN_SOURCE_BATCH_CONTENT.into(),
        registered_feed_snapshot_hash: registered_feed_snapshot_hash.clone(),
        feed_attempt_hashes_in_registered_feed_order: feed_attempt_hashes,
        source_record_hashes_in_feed_then_provider_order: source_record_hashes,
        event_projection_ids_in_feed_then_provider_order: event_projection_ids,
        aggregator_observed_at_rfc3339_nanos_utc: aggregator_observed_at.clone(),
    };
    let source_batch_content_hash = sha256_json(&source_batch_preimage)?;

    let logical_subject = RunLogicalSubjectPreimage {
        domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
        subject_kind: SubjectKind::IngressRun,
        source_fact_key: None,
        config_hash: Some(context.config_hash.clone()),
        sample_key: None,
        outcome_phase: None,
        stored_due_date: None,
        ingress_source_batch_hash: Some(source_batch_content_hash.clone()),
    };
    logical_subject.validate()?;
    let logical_subject_key = sha256_json(&logical_subject)?;

    let mut source_batch_attempt_rows = feed_preparations
        .iter()
        .map(|prepared| prepared.row.clone())
        .collect::<Vec<_>>();
    let mut source_fact_rows = Vec::new();
    let mut source_fact_attempt_rows = Vec::new();

    for (registered_entry, feed) in registered.iter().zip(feed_preparations.iter()) {
        let Some(batch_evidence_json) = &feed.evidence_json else {
            if !feed.records.is_empty() {
                return Err(IngressPreparationError::new(
                    "unavailable_feed_has_records",
                    "an unavailable feed cannot produce source-fact attempts",
                ));
            }
            continue;
        };
        let batch_evidence_hash = feed.evidence_hash.as_ref().ok_or_else(|| {
            IngressPreparationError::new(
                "feed_evidence_hash_missing",
                "a complete feed must retain its evidence hash",
            )
        })?;
        let record_batch_content_hash = feed.source_content_hash.as_ref().ok_or_else(|| {
            IngressPreparationError::new(
                "feed_content_hash_missing",
                "a complete feed must retain its ordered content hash",
            )
        })?;
        let source_batch_attempt_id = feed.row.source_batch_attempt_id.clone();

        for (provider_ordinal, prepared_record) in feed.records.iter().enumerate() {
            let raw_record = available_record_at(registered_entry.attempt, provider_ordinal)?;
            let acquired = AcquiredGlobalNewsRecordPreimage {
                domain: DOMAIN_ACQUIRED_GLOBAL_NEWS_RECORD.into(),
                source_fact_key: prepared_record.source_fact_key.clone(),
                provider_content_hash: prepared_record.provider_content_hash.clone(),
                record: prepared_record.content.clone(),
                record_provider: registered_entry.registration.provider_id.into(),
                record_source: registered_entry.registration.source_contract.into(),
                record_source_at: raw_record.evidence.source_at().map(str::to_owned),
                record_observed_at: raw_record.evidence.observed_at().to_owned(),
                record_batch_id: raw_record.evidence.batch_id().to_owned(),
                record_batch_content_hash: record_batch_content_hash.clone(),
            };
            let acquired_record_json = canonical_json(&acquired)?;
            let acquired_record_hash = sha256_json(&acquired)?;

            let authority = authorities.get(&prepared_record.source_fact_key);
            let (attempt_result, conflict_hash, create_source_fact) = match authority {
                None => (SourceFactAttemptResult::Accepted, None, true),
                Some(authority)
                    if authority.provider_content_hash == prepared_record.provider_content_hash =>
                {
                    (SourceFactAttemptResult::Replay, None, false)
                }
                Some(authority) => {
                    let conflict = SourceFactConflictPreimage {
                        domain: DOMAIN_SOURCE_FACT_CONFLICT.into(),
                        source_fact_key: prepared_record.source_fact_key.clone(),
                        authoritative_provider_content_hash: authority
                            .provider_content_hash
                            .clone(),
                        attempted_provider_content_hash: prepared_record
                            .provider_content_hash
                            .clone(),
                    };
                    (
                        SourceFactAttemptResult::Conflict,
                        Some(sha256_json(&conflict)?),
                        false,
                    )
                }
            };

            if create_source_fact {
                let source_fact_row = source_fact_row(
                    raw_record,
                    prepared_record,
                    context,
                    record_batch_content_hash,
                    &registered_entry.registration,
                )?;
                source_fact_rows.push(source_fact_row);
                authorities.insert(
                    prepared_record.source_fact_key.clone(),
                    AuthorityState {
                        provider_content_hash: prepared_record.provider_content_hash.clone(),
                        created_in_run: true,
                    },
                );
            }

            let source_fact_attempt = SourceFactAttemptPreimage {
                domain: DOMAIN_SOURCE_ATTEMPT.into(),
                ingress_run_id: context.stage_run_id.clone(),
                source_fact_key: prepared_record.source_fact_key.clone(),
                source_batch_attempt_id: source_batch_attempt_id.clone(),
                provider_ordinal: u32::try_from(provider_ordinal).map_err(|_| {
                    IngressPreparationError::new(
                        "provider_ordinal_overflow",
                        "provider ordinal exceeds u32",
                    )
                })?,
                source_batch_id: feed.row.batch_id.clone().ok_or_else(|| {
                    IngressPreparationError::new(
                        "source_batch_id_missing",
                        "available feed has no provider batch ID",
                    )
                })?,
                record_batch_id: raw_record.evidence.batch_id().to_owned(),
                observed_at: raw_record.evidence.observed_at().to_owned(),
                batch_evidence_hash: batch_evidence_hash.clone(),
            };
            let source_fact_attempt_id = sha256_json(&source_fact_attempt)?;
            source_fact_attempt_rows.push(SelectionSourceFactAttemptRowContentPreimage {
                domain: DOMAIN_SOURCE_FACT_ATTEMPT_ROW.into(),
                source_fact_attempt_id,
                ingress_run_id: context.stage_run_id.clone(),
                source_batch_attempt_id: source_batch_attempt_id.clone(),
                provider_ordinal: u32::try_from(provider_ordinal).map_err(|_| {
                    IngressPreparationError::new(
                        "provider_ordinal_overflow",
                        "provider ordinal exceeds u32",
                    )
                })?,
                source_fact_key: prepared_record.source_fact_key.clone(),
                acquired_record_json,
                acquired_record_hash,
                batch_evidence_json: batch_evidence_json.clone(),
                batch_evidence_hash: batch_evidence_hash.clone(),
                event_projection_id: prepared_record.event_id.clone(),
                attempt_result,
                conflict_hash,
                attempted_at: rfc3339_nanos(registered_entry.attempt.attempted_at()),
            });
        }
    }

    // Keep recovery JSON stable by each table's logical primary key.
    source_batch_attempt_rows.sort_by(|left, right| {
        left.source_batch_attempt_id
            .as_bytes()
            .cmp(right.source_batch_attempt_id.as_bytes())
    });
    source_fact_rows.sort_by(|left, right| {
        left.source_fact_key
            .as_bytes()
            .cmp(right.source_fact_key.as_bytes())
    });
    source_fact_attempt_rows.sort_by(|left, right| {
        left.source_fact_attempt_id
            .as_bytes()
            .cmp(right.source_fact_attempt_id.as_bytes())
    });

    let stage_input = SourceIngressStageInputPreimage {
        domain: DOMAIN_SOURCE_INGRESS_STAGE.into(),
        stage_run_id: context.stage_run_id.clone(),
        logical_subject_key: logical_subject_key.clone(),
        config_activation_run_id: context.config_activation_run_id.clone(),
        config_hash: context.config_hash.clone(),
        generation_market_date: context.generation_market_date.to_string(),
        aggregator_observed_at_rfc3339_nanos_utc: aggregator_observed_at.clone(),
        source_batch_content_hash: source_batch_content_hash.clone(),
        registered_feed_snapshot_json: registered_feed_snapshot_json.clone(),
        registered_feed_snapshot_hash: registered_feed_snapshot_hash.clone(),
        source_batch_attempt_rows,
        source_fact_rows,
        source_fact_attempt_rows,
        planned_run_status: RunStatus::Completed,
    };
    let payload_json = canonical_json(&stage_input)?;
    let payload_json_hash = sha256_bytes(payload_json.as_bytes());

    let rows = run_payload_row_hashes(&stage_input)?;
    let run_payload = RunPayloadPreimage {
        domain: DOMAIN_INGRESS_PAYLOAD.into(),
        subject_kind: SubjectKind::IngressRun,
        subject_id: context.stage_run_id.clone(),
        logical_subject_key: logical_subject_key.clone(),
        source_fact_key: None,
        config_activation_run_id: context.config_activation_run_id.clone(),
        config_hash: context.config_hash.clone(),
        config_snapshot_json_hash: None,
        config_activation_content_hash: None,
        config_activation_file_content_hash: None,
        config_effective_from_rfc3339_nanos_utc: None,
        artifact_valid_from: None,
        artifact_expires_at: None,
        executable_revision: None,
        legacy_cutover_snapshot_hash: None,
        generation_market_date: Some(context.generation_market_date.to_string()),
        aggregator_observed_at_rfc3339_nanos_utc: Some(aggregator_observed_at),
        ingress_source_batch_content_hash: Some(source_batch_content_hash.clone()),
        outcome_phase: None,
        stored_due_date: None,
        outcome_claim_id: None,
        planned_outcome_run_id: None,
        outcome_claim_receipt_content_hash: None,
        outcome_claim_due_binding_hash: None,
        outcome_claim_provider_request_hash: None,
        rows,
    };
    let in_memory_payload_hash = sha256_json(&run_payload)?;
    let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
        domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
        stage_run_id: context.stage_run_id.clone(),
        subject_kind: SubjectKind::IngressRun,
        logical_subject_key: logical_subject_key.clone(),
        payload_schema: SOURCE_INGRESS_PAYLOAD_SCHEMA.into(),
        payload_json: payload_json.clone(),
        payload_json_hash: payload_json_hash.clone(),
        in_memory_payload_hash: in_memory_payload_hash.clone(),
        config_activation_run_id: context.config_activation_run_id.clone(),
        config_hash: context.config_hash.clone(),
        enveloped_at: rfc3339_nanos(context.enveloped_at),
    };
    drop(sha256_json(&recovery_envelope)?);

    // `created_in_run` is not serialized. Reading it here keeps the authority
    // state explicitly checked and prevents it from becoming a silent input.
    if authorities
        .values()
        .any(|authority| authority.created_in_run && authority.provider_content_hash.is_empty())
    {
        return Err(IngressPreparationError::new(
            "authoritative_content_hash_missing",
            "new source-fact authority has no provider content hash",
        ));
    }

    Ok(PreparedSourceIngress {
        stage_input,
        run_payload,
        recovery_envelope,
    })
}

fn validate_context(
    batch: &RawNewsAggregationBatch,
    context: &SourceIngressPreparationContext,
) -> Result<(), IngressPreparationError> {
    require_non_empty(&context.stage_run_id, "stage_run_id")?;
    require_non_empty(
        &context.config_activation_run_id,
        "config_activation_run_id",
    )?;
    require_hash(&context.config_hash)?;
    require_non_empty(&context.gate_version, "gate_version")?;
    if context.per_feed_limit == 0 || context.per_feed_limit > 20 {
        return Err(IngressPreparationError::new(
            "global_news_limit_invalid",
            "per-feed limit must be within 1..=20",
        ));
    }
    if context.evaluated_at < batch.observed_at() {
        return Err(IngressPreparationError::new(
            "evaluation_precedes_aggregation",
            "ingress evaluation cannot precede aggregator observation",
        ));
    }
    if context.enveloped_at < context.evaluated_at {
        return Err(IngressPreparationError::new(
            "envelope_precedes_evaluation",
            "recovery envelope cannot precede ingress evaluation",
        ));
    }
    if batch.observed_at() < context.config_effective_from {
        return Err(IngressPreparationError::new(
            "ingress_precedes_config_effective_from",
            "source acquisition cannot precede config effective time",
        ));
    }
    let shanghai_date = fixed_shanghai()
        .from_utc_datetime(&context.evaluated_at.naive_utc())
        .date_naive();
    if context.generation_market_date != shanghai_date {
        return Err(IngressPreparationError::new(
            "generation_market_date_mismatch",
            "generation market date must equal the evaluated Asia/Shanghai date",
        ));
    }
    Ok(())
}

fn bind_registered_attempts(
    batch: &RawNewsAggregationBatch,
) -> Result<Vec<RegisteredAttempt<'_>>, IngressPreparationError> {
    let canonical = registered_global_news_feeds();
    if batch.attempts().len() != canonical.len() {
        return Err(IngressPreparationError::new(
            "registered_feed_attempt_count_mismatch",
            "every registered feed must have exactly one terminal attempt",
        ));
    }
    let expected = canonical
        .into_iter()
        .map(|registration| (registration.feed_name, registration))
        .collect::<BTreeMap<_, _>>();
    let mut observed = BTreeMap::new();
    for attempt in batch.attempts() {
        if observed
            .insert(attempt.registration().feed_name, attempt)
            .is_some()
        {
            return Err(IngressPreparationError::new(
                "registered_feed_attempt_duplicate",
                "a registered feed produced more than one terminal attempt",
            ));
        }
    }
    let mut bound = Vec::with_capacity(expected.len());
    for (feed_name, canonical_registration) in expected {
        let attempt = observed.get(feed_name).ok_or_else(|| {
            IngressPreparationError::new(
                "registered_feed_attempt_missing",
                "a registered feed has no terminal attempt",
            )
        })?;
        if attempt.registration() != canonical_registration
            || canonical_registration.upstream_revision != MAGIC_MARKET_DATA_REVISION
        {
            return Err(IngressPreparationError::new(
                "registered_feed_configuration_mismatch",
                "runtime feed registration differs from the checked-in registry",
            ));
        }
        let configuration = crate::selection::schema_v2::RegisteredFeedConfigurationPreimage {
            domain: DOMAIN_REGISTERED_FEED_CONFIG.into(),
            gateway_provider: canonical_registration.gateway_provider.into(),
            provider_id: canonical_registration.provider_id.into(),
            source_contract: canonical_registration.source_contract.into(),
            capability_name: canonical_registration.capability_name.into(),
            max_limit: canonical_registration.max_limit,
            upstream_revision: canonical_registration.upstream_revision.into(),
        };
        let configuration_hash = sha256_json(&configuration)?;
        let identity = crate::selection::schema_v2::RegisteredFeedIdentityPreimage {
            domain: DOMAIN_REGISTERED_FEED_IDENTITY.into(),
            feed_name: canonical_registration.feed_name.into(),
            gateway_provider: canonical_registration.gateway_provider.into(),
            configuration_hash: configuration_hash.clone(),
        };
        let feed_identity = sha256_json(&identity)?;
        bound.push(RegisteredAttempt {
            registration: canonical_registration,
            configuration_hash,
            feed_identity,
            attempt,
        });
    }
    bound.sort_by(|left, right| {
        left.feed_identity
            .as_bytes()
            .cmp(right.feed_identity.as_bytes())
    });
    Ok(bound)
}

fn registered_feed_snapshot(
    registered: &[RegisteredAttempt<'_>],
) -> Result<(String, String), IngressPreparationError> {
    let feeds_sorted = registered
        .iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            Ok(crate::selection::schema_v2::RegisteredFeedEntryPreimage {
                ordinal: u32::try_from(ordinal).map_err(|_| {
                    IngressPreparationError::new(
                        "registered_feed_ordinal_overflow",
                        "registered feed ordinal exceeds u32",
                    )
                })?,
                feed_identity: entry.feed_identity.clone(),
                gateway_provider: entry.registration.gateway_provider.into(),
                capability_name: entry.registration.capability_name.into(),
                configuration_hash: entry.configuration_hash.clone(),
            })
        })
        .collect::<Result<Vec<_>, IngressPreparationError>>()?;
    let snapshot = crate::selection::schema_v2::RegisteredFeedSnapshotPreimage {
        domain: DOMAIN_REGISTERED_FEED_SNAPSHOT.into(),
        feeds_sorted,
    };
    Ok((canonical_json(&snapshot)?, sha256_json(&snapshot)?))
}

fn authoritative_snapshot(
    existing: &[ExistingSourceFactAuthority],
) -> Result<BTreeMap<String, AuthorityState>, IngressPreparationError> {
    let mut authorities = BTreeMap::new();
    for authority in existing {
        require_hash(&authority.source_fact_key)?;
        require_hash(&authority.provider_content_hash)?;
        if authorities
            .insert(
                authority.source_fact_key.clone(),
                AuthorityState {
                    provider_content_hash: authority.provider_content_hash.clone(),
                    created_in_run: false,
                },
            )
            .is_some()
        {
            return Err(IngressPreparationError::new(
                "authoritative_source_fact_duplicate",
                "existing source-fact snapshot contains a duplicate key",
            ));
        }
    }
    Ok(authorities)
}

fn prepare_feed(
    registered: &RegisteredAttempt<'_>,
    context: &SourceIngressPreparationContext,
    registered_feed_snapshot_hash: &str,
    aggregator_observed_at: DateTime<Utc>,
) -> Result<FeedPreparation, IngressPreparationError> {
    if registered.attempt.attempted_at() < context.config_effective_from {
        return Err(IngressPreparationError::new(
            "feed_attempt_precedes_config_effective_from",
            "feed attempt cannot precede config effective time",
        ));
    }
    if registered.attempt.attempted_at() > aggregator_observed_at {
        return Err(IngressPreparationError::new(
            "feed_attempt_after_aggregation",
            "feed attempt cannot start after aggregator observation",
        ));
    }
    if let Some(evidence) = registered.attempt.terminal().evidence() {
        if parse_observed_at(&evidence.observed_at)? > aggregator_observed_at {
            return Err(IngressPreparationError::new(
                "feed_observation_after_aggregation",
                "provider observation cannot follow aggregator observation",
            ));
        }
    }
    let request = global_news_request_evidence(registered, context.per_feed_limit)?;
    let source_batch_attempt_id = sha256_json(&FeedAttemptKeyPreimage {
        domain: DOMAIN_FEED_ATTEMPT_KEY.into(),
        ingress_run_id: context.stage_run_id.clone(),
        feed_identity: registered.feed_identity.clone(),
    })?;

    let terminal = registered.attempt.terminal();
    match terminal.kind() {
        RawGlobalNewsTerminalKind::Available => {
            let records = terminal.records().ok_or_else(|| {
                IngressPreparationError::new(
                    "available_feed_records_missing",
                    "Available terminal has no source records",
                )
            })?;
            let evidence = terminal.evidence().ok_or_else(|| {
                IngressPreparationError::new(
                    "available_feed_evidence_missing",
                    "Available terminal has no batch evidence",
                )
            })?;
            let (evidence_preimage, evidence_json, evidence_hash) =
                complete_feed_evidence(registered, evidence, batch_observed_at(records)?)?;
            let batch_source_at = evidence_preimage.source_at.as_deref().ok_or_else(|| {
                IngressPreparationError::new(
                    "complete_feed_source_at_missing",
                    "Available requires provider source time",
                )
            })?;
            if parse_provider_time(registered.registration, batch_source_at)?
                != records[0].published_at
            {
                return Err(IngressPreparationError::new(
                    "batch_source_at_newest_record_mismatch",
                    "batch source time must equal its newest provider record",
                ));
            }
            let mut prepared_records = Vec::with_capacity(records.len());
            let mut source_record_hashes = Vec::with_capacity(records.len());
            let mut event_ids = Vec::with_capacity(records.len());
            let mut item_ids = BTreeSet::new();
            let mut canonical_urls = BTreeSet::new();
            for (ordinal, record) in records.iter().enumerate() {
                if !item_ids.insert(record.item_id.as_str()) {
                    return Err(IngressPreparationError::new(
                        "provider_item_id_duplicate",
                        "complete provider batch contains a duplicate item identity",
                    ));
                }
                if !canonical_urls.insert(record.canonical_url.as_str()) {
                    return Err(IngressPreparationError::new(
                        "provider_canonical_url_duplicate",
                        "complete provider batch contains a duplicate canonical URL",
                    ));
                }
                let prepared = prepare_record(record, registered, evidence)?;
                let source_record = FeedSourceRecordHashPreimage {
                    domain: DOMAIN_FEED_SOURCE_RECORD.into(),
                    provider_ordinal: u32::try_from(ordinal).map_err(|_| {
                        IngressPreparationError::new(
                            "provider_ordinal_overflow",
                            "provider ordinal exceeds u32",
                        )
                    })?,
                    source_fact_key: prepared.source_fact_key.clone(),
                    provider_content_hash: prepared.provider_content_hash.clone(),
                };
                source_record_hashes.push(sha256_json(&source_record)?);
                event_ids.push(prepared.event_id.clone());
                prepared_records.push(prepared);
            }
            let source_content = FeedSourceContentPreimage {
                domain: DOMAIN_FEED_SOURCE_CONTENT.into(),
                feed_identity: registered.feed_identity.clone(),
                evidence_hash: evidence_hash.clone(),
                record_hashes_in_provider_order: source_record_hashes.clone(),
            };
            let source_content_hash = sha256_json(&source_content)?;
            let feed_attempt = FeedAttemptContentPreimage {
                domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
                feed_identity: registered.feed_identity.clone(),
                request_hash: request.request_hash.clone(),
                request_evidence_hash: request.request_evidence_hash.clone(),
                status_kind: FeedStatusKind::Available,
                record_count: Some(u32::try_from(records.len()).map_err(|_| {
                    IngressPreparationError::new(
                        "record_count_overflow",
                        "feed record count exceeds u32",
                    )
                })?),
                evidence_hash: Some(evidence_hash.clone()),
                source_content_hash: Some(source_content_hash.clone()),
                available_evidence_hash: Some(evidence_hash.clone()),
                failed_stage: None,
                reason_code: None,
                retryable: None,
                detail_hash: None,
                error_fingerprint: None,
            };
            feed_attempt.validate()?;
            let feed_attempt_content_hash = sha256_json(&feed_attempt)?;
            Ok(FeedPreparation {
                row: SelectionSourceBatchAttemptRowContentPreimage {
                    domain: DOMAIN_SOURCE_BATCH_ATTEMPT_ROW.into(),
                    source_batch_attempt_id,
                    ingress_run_id: context.stage_run_id.clone(),
                    config_activation_run_id: context.config_activation_run_id.clone(),
                    config_hash: context.config_hash.clone(),
                    generation_market_date: context.generation_market_date.to_string(),
                    registered_feed_identity: registered.feed_identity.clone(),
                    registered_feed_snapshot_hash: registered_feed_snapshot_hash.into(),
                    request_hash: request.request_hash,
                    request_evidence_json: request.request_evidence_json,
                    request_evidence_hash: request.request_evidence_hash,
                    feed_attempt_content_hash: feed_attempt_content_hash.clone(),
                    status_kind: FeedStatusKind::Available,
                    record_count: feed_attempt.record_count,
                    provider: Some(evidence_preimage.provider.clone()),
                    source: Some(evidence_preimage.source.clone()),
                    source_at: evidence_preimage.source_at.clone(),
                    observed_at: Some(evidence_preimage.observed_at.clone()),
                    batch_id: Some(evidence_preimage.batch_id.clone()),
                    batch_content_hash: Some(source_content_hash.clone()),
                    failed_stage: None,
                    reason_code: None,
                    retryable: None,
                    available_evidence_json: Some(evidence_json.clone()),
                    available_evidence_hash: Some(evidence_hash.clone()),
                    error_detail_json: None,
                    error_detail_hash: None,
                    error_fingerprint: None,
                    attempted_at: rfc3339_nanos(registered.attempt.attempted_at()),
                },
                feed_attempt_content_hash,
                record_hashes: source_record_hashes,
                event_ids,
                records: prepared_records,
                evidence_json: Some(evidence_json),
                evidence_hash: Some(evidence_hash),
                source_content_hash: Some(source_content_hash),
            })
        }
        RawGlobalNewsTerminalKind::VerifiedEmpty => {
            let evidence = terminal.evidence().ok_or_else(|| {
                IngressPreparationError::new(
                    "verified_empty_evidence_missing",
                    "VerifiedEmpty terminal has no batch evidence",
                )
            })?;
            let (evidence_preimage, evidence_json, evidence_hash) =
                complete_feed_evidence(registered, evidence, None)?;
            let source_content = FeedSourceContentPreimage {
                domain: DOMAIN_FEED_SOURCE_CONTENT.into(),
                feed_identity: registered.feed_identity.clone(),
                evidence_hash: evidence_hash.clone(),
                record_hashes_in_provider_order: Vec::new(),
            };
            let source_content_hash = sha256_json(&source_content)?;
            let feed_attempt = FeedAttemptContentPreimage {
                domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
                feed_identity: registered.feed_identity.clone(),
                request_hash: request.request_hash.clone(),
                request_evidence_hash: request.request_evidence_hash.clone(),
                status_kind: FeedStatusKind::VerifiedEmpty,
                record_count: Some(0),
                evidence_hash: Some(evidence_hash.clone()),
                source_content_hash: Some(source_content_hash.clone()),
                available_evidence_hash: Some(evidence_hash.clone()),
                failed_stage: None,
                reason_code: None,
                retryable: None,
                detail_hash: None,
                error_fingerprint: None,
            };
            feed_attempt.validate()?;
            let feed_attempt_content_hash = sha256_json(&feed_attempt)?;
            Ok(FeedPreparation {
                row: SelectionSourceBatchAttemptRowContentPreimage {
                    domain: DOMAIN_SOURCE_BATCH_ATTEMPT_ROW.into(),
                    source_batch_attempt_id,
                    ingress_run_id: context.stage_run_id.clone(),
                    config_activation_run_id: context.config_activation_run_id.clone(),
                    config_hash: context.config_hash.clone(),
                    generation_market_date: context.generation_market_date.to_string(),
                    registered_feed_identity: registered.feed_identity.clone(),
                    registered_feed_snapshot_hash: registered_feed_snapshot_hash.into(),
                    request_hash: request.request_hash,
                    request_evidence_json: request.request_evidence_json,
                    request_evidence_hash: request.request_evidence_hash,
                    feed_attempt_content_hash: feed_attempt_content_hash.clone(),
                    status_kind: FeedStatusKind::VerifiedEmpty,
                    record_count: Some(0),
                    provider: Some(evidence_preimage.provider.clone()),
                    source: Some(evidence_preimage.source.clone()),
                    source_at: evidence_preimage.source_at.clone(),
                    observed_at: Some(evidence_preimage.observed_at.clone()),
                    batch_id: Some(evidence_preimage.batch_id.clone()),
                    batch_content_hash: Some(source_content_hash.clone()),
                    failed_stage: None,
                    reason_code: None,
                    retryable: None,
                    available_evidence_json: Some(evidence_json.clone()),
                    available_evidence_hash: Some(evidence_hash.clone()),
                    error_detail_json: None,
                    error_detail_hash: None,
                    error_fingerprint: None,
                    attempted_at: rfc3339_nanos(registered.attempt.attempted_at()),
                },
                feed_attempt_content_hash,
                record_hashes: Vec::new(),
                event_ids: Vec::new(),
                records: Vec::new(),
                evidence_json: Some(evidence_json),
                evidence_hash: Some(evidence_hash),
                source_content_hash: Some(source_content_hash),
            })
        }
        RawGlobalNewsTerminalKind::Unavailable => {
            let unavailable = terminal.unavailable().ok_or_else(|| {
                IngressPreparationError::new(
                    "unavailable_detail_missing",
                    "Unavailable terminal has no typed failure detail",
                )
            })?;
            prepare_unavailable_feed(
                registered,
                context,
                registered_feed_snapshot_hash,
                request,
                source_batch_attempt_id,
                unavailable,
            )
        }
    }
}

fn prepare_unavailable_feed(
    registered: &RegisteredAttempt<'_>,
    context: &SourceIngressPreparationContext,
    registered_feed_snapshot_hash: &str,
    request: RequestEvidenceColumns,
    source_batch_attempt_id: String,
    unavailable: &FeedUnavailable,
) -> Result<FeedPreparation, IngressPreparationError> {
    require_snake_token(unavailable.failed_stage(), "failed_stage")?;
    require_snake_token(unavailable.reason_code(), "reason_code")?;
    let (error_kind, diagnostic_code, invariant_id) =
        closed_feed_error_mapping(unavailable.diagnostic_code())?;

    let (
        available_evidence_json,
        available_evidence_hash,
        provider,
        source,
        source_at,
        observed_at,
        batch_id,
    ) = match unavailable.available_evidence() {
        None => (None, None, None, None, None, None, None),
        Some(evidence) => {
            validate_evidence_registration(registered, evidence)?;
            let partial = FeedAvailableEvidencePreimage {
                domain: DOMAIN_FEED_AVAILABLE_EVIDENCE.into(),
                feed_identity: registered.feed_identity.clone(),
                provider: Some(registered.registration.provider_id.into()),
                source: Some(evidence.source.clone()),
                source_at: evidence.source_at.clone(),
                observed_at: Some(evidence.observed_at.clone()),
                batch_id: Some(evidence.batch_id.clone()),
                batch_content_hash: None,
            };
            let json = canonical_json(&partial)?;
            let hash = sha256_json(&partial)?;
            (
                Some(json),
                Some(hash),
                partial.provider,
                partial.source,
                partial.source_at,
                partial.observed_at,
                partial.batch_id,
            )
        }
    };

    let detail = ProviderErrorDetailPreimage {
        domain: DOMAIN_PROVIDER_ERROR_DETAIL.into(),
        error_kind,
        provider: registered.registration.provider_id.into(),
        operation: unavailable.failed_stage().into(),
        error_code: Some(unavailable.reason_code().into()),
        http_status: None,
        timeout_ms: None,
        invariant_id: invariant_id.map(str::to_owned),
        diagnostic_code: diagnostic_code.into(),
    };
    detail.validate()?;
    let error_detail_json = canonical_json(&detail)?;
    let error_detail_hash = sha256_json(&detail)?;
    let fingerprint = ErrorFingerprintPreimage {
        domain: DOMAIN_ERROR_FINGERPRINT.into(),
        failed_stage: unavailable.failed_stage().into(),
        reason_code: unavailable.reason_code().into(),
        retryable: unavailable.retryable(),
        available_evidence_hash: available_evidence_hash.clone(),
        detail_hash: error_detail_hash.clone(),
    };
    let error_fingerprint = sha256_json(&fingerprint)?;
    let feed_attempt = FeedAttemptContentPreimage {
        domain: DOMAIN_FEED_ATTEMPT_CONTENT.into(),
        feed_identity: registered.feed_identity.clone(),
        request_hash: request.request_hash.clone(),
        request_evidence_hash: request.request_evidence_hash.clone(),
        status_kind: FeedStatusKind::Unavailable,
        record_count: None,
        evidence_hash: None,
        source_content_hash: None,
        available_evidence_hash: available_evidence_hash.clone(),
        failed_stage: Some(unavailable.failed_stage().into()),
        reason_code: Some(unavailable.reason_code().into()),
        retryable: Some(unavailable.retryable()),
        detail_hash: Some(error_detail_hash.clone()),
        error_fingerprint: Some(error_fingerprint.clone()),
    };
    feed_attempt.validate()?;
    let feed_attempt_content_hash = sha256_json(&feed_attempt)?;

    Ok(FeedPreparation {
        row: SelectionSourceBatchAttemptRowContentPreimage {
            domain: DOMAIN_SOURCE_BATCH_ATTEMPT_ROW.into(),
            source_batch_attempt_id,
            ingress_run_id: context.stage_run_id.clone(),
            config_activation_run_id: context.config_activation_run_id.clone(),
            config_hash: context.config_hash.clone(),
            generation_market_date: context.generation_market_date.to_string(),
            registered_feed_identity: registered.feed_identity.clone(),
            registered_feed_snapshot_hash: registered_feed_snapshot_hash.into(),
            request_hash: request.request_hash,
            request_evidence_json: request.request_evidence_json,
            request_evidence_hash: request.request_evidence_hash,
            feed_attempt_content_hash: feed_attempt_content_hash.clone(),
            status_kind: FeedStatusKind::Unavailable,
            record_count: None,
            provider,
            source,
            source_at,
            observed_at,
            batch_id,
            batch_content_hash: None,
            failed_stage: Some(unavailable.failed_stage().into()),
            reason_code: Some(unavailable.reason_code().into()),
            retryable: Some(unavailable.retryable()),
            available_evidence_json,
            available_evidence_hash,
            error_detail_json: Some(error_detail_json),
            error_detail_hash: Some(error_detail_hash),
            error_fingerprint: Some(error_fingerprint),
            attempted_at: rfc3339_nanos(registered.attempt.attempted_at()),
        },
        feed_attempt_content_hash,
        record_hashes: Vec::new(),
        event_ids: Vec::new(),
        records: Vec::new(),
        evidence_json: None,
        evidence_hash: None,
        source_content_hash: None,
    })
}

fn complete_feed_evidence(
    registered: &RegisteredAttempt<'_>,
    evidence: &crate::data_gateway::BatchEvidence,
    record_observed_at: Option<DateTime<Utc>>,
) -> Result<(FeedBatchEvidencePreimage, String, String), IngressPreparationError> {
    validate_evidence_registration(registered, evidence)?;
    let source_at = evidence.source_at.as_deref().ok_or_else(|| {
        IngressPreparationError::new(
            "complete_feed_source_at_missing",
            "Available and VerifiedEmpty require provider source time",
        )
    })?;
    parse_provider_time(registered.registration, source_at)?;
    let observed = parse_observed_at(&evidence.observed_at)?;
    if let Some(record_observed_at) = record_observed_at {
        if observed != record_observed_at {
            return Err(IngressPreparationError::new(
                "record_batch_observation_mismatch",
                "record and batch observation times differ",
            ));
        }
    }
    let preimage = FeedBatchEvidencePreimage {
        domain: DOMAIN_FEED_BATCH_EVIDENCE.into(),
        feed_identity: registered.feed_identity.clone(),
        provider: registered.registration.provider_id.into(),
        source: evidence.source.clone(),
        source_at: evidence.source_at.clone(),
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        batch_quality: FeedBatchQuality::Complete,
    };
    Ok((
        preimage.clone(),
        canonical_json(&preimage)?,
        sha256_json(&preimage)?,
    ))
}

fn validate_evidence_registration(
    registered: &RegisteredAttempt<'_>,
    evidence: &crate::data_gateway::BatchEvidence,
) -> Result<(), IngressPreparationError> {
    if evidence.provider != registered.registration.provider.provider_id()
        || evidence.source != registered.registration.source_contract
        || evidence.batch_id.is_empty()
        || evidence.observed_at.is_empty()
    {
        return Err(IngressPreparationError::new(
            "feed_evidence_registration_mismatch",
            "feed evidence differs from the immutable registration",
        ));
    }
    Ok(())
}

fn batch_observed_at(
    records: &[crate::data_gateway::GlobalNewsRecord],
) -> Result<Option<DateTime<Utc>>, IngressPreparationError> {
    let first = records.first().ok_or_else(|| {
        IngressPreparationError::new(
            "available_feed_empty",
            "Available requires at least one source record",
        )
    })?;
    if records
        .iter()
        .any(|record| record.observed_at != first.observed_at)
    {
        return Err(IngressPreparationError::new(
            "record_observation_batch_mismatch",
            "all records in a complete feed must share one observation time",
        ));
    }
    Ok(Some(first.observed_at))
}

fn prepare_record(
    record: &crate::data_gateway::GlobalNewsRecord,
    registered: &RegisteredAttempt<'_>,
    batch_evidence: &crate::data_gateway::BatchEvidence,
) -> Result<PreparedRecord, IngressPreparationError> {
    validate_record(record, registered, batch_evidence)?;
    let instruments_sorted = sorted_unique(&record.instruments, "duplicate_instrument")?;
    let topics_sorted = sorted_unique(&record.topics, "duplicate_topic")?;
    let provider_source = registered.registration.source_contract.to_owned();
    let key = SourceFactKeyPreimage {
        domain: DOMAIN_SOURCE_FACT_KEY.into(),
        provider_source: provider_source.clone(),
        item_id: record.item_id.clone(),
    };
    let source_fact_key = sha256_json(&key)?;
    let content = SourceFactContentPreimage {
        domain: DOMAIN_SOURCE_CONTENT.into(),
        provider_source: provider_source.clone(),
        item_id: record.item_id.clone(),
        title: record.title.clone(),
        summary: record.summary.clone(),
        content: record.content.clone(),
        publisher: record.publisher.clone(),
        canonical_url: record.canonical_url.clone(),
        published_at_rfc3339_nanos_utc: rfc3339_nanos(record.published_at),
        instruments_sorted,
        topics_sorted,
        language: record.language.clone(),
        record_source: registered.registration.source_contract.into(),
        record_source_at: record.evidence.source_at().map(str::to_owned),
    };
    let provider_content_hash = sha256_json(&content)?;
    Ok(PreparedRecord {
        source_fact_key,
        provider_content_hash,
        content,
        event_id: event_projection_id(&provider_source, &record.item_id),
    })
}

fn validate_record(
    record: &crate::data_gateway::GlobalNewsRecord,
    registered: &RegisteredAttempt<'_>,
    batch_evidence: &crate::data_gateway::BatchEvidence,
) -> Result<(), IngressPreparationError> {
    for (value, field) in [
        (record.item_id.as_str(), "item_id"),
        (record.title.as_str(), "title"),
        (record.publisher.as_str(), "publisher"),
        (record.canonical_url.as_str(), "canonical_url"),
        (record.language.as_str(), "language"),
    ] {
        require_non_empty(value, field)?;
    }
    reject_control(&record.item_id)?;
    reject_control(registered.registration.source_contract)?;
    if !record.canonical_url.starts_with("https://") {
        return Err(IngressPreparationError::new(
            "canonical_url_invalid",
            "global-news canonical URL must use HTTPS",
        ));
    }
    if record.evidence.provider() != registered.registration.provider.provider_id()
        || record.evidence.batch_id() != batch_evidence.batch_id
        || record.evidence.observed_at() != batch_evidence.observed_at
    {
        return Err(IngressPreparationError::new(
            "record_batch_evidence_mismatch",
            "record evidence differs from its complete feed evidence",
        ));
    }
    let record_source_at = record.evidence.source_at().ok_or_else(|| {
        IngressPreparationError::new(
            "record_source_at_missing",
            "provider publication evidence must remain explicit",
        )
    })?;
    if parse_provider_time(registered.registration, record_source_at)? != record.published_at {
        return Err(IngressPreparationError::new(
            "record_publication_evidence_mismatch",
            "record publication differs from provider source evidence",
        ));
    }
    if parse_observed_at(record.evidence.observed_at())? != record.observed_at {
        return Err(IngressPreparationError::new(
            "record_observation_evidence_mismatch",
            "record observation differs from provider evidence",
        ));
    }
    Ok(())
}

fn source_fact_row(
    record: &crate::data_gateway::GlobalNewsRecord,
    prepared: &PreparedRecord,
    context: &SourceIngressPreparationContext,
    record_batch_content_hash: &str,
    registration: &RegisteredGlobalNewsFeed,
) -> Result<SelectionSourceFactRowContentPreimage, IngressPreparationError> {
    let gate_input = IngressGateInputPreimage {
        domain: DOMAIN_INGRESS_GATE_INPUT.into(),
        source_fact_key: prepared.source_fact_key.clone(),
        config_activation_run_id: context.config_activation_run_id.clone(),
        config_hash: context.config_hash.clone(),
        provider_published_at_rfc3339_nanos_utc: rfc3339_nanos(record.published_at),
        record_observed_at: record.evidence.observed_at().to_owned(),
        batch_observed_at: record.evidence.observed_at().to_owned(),
        batch_content_hash: record_batch_content_hash.into(),
        evaluated_at_rfc3339_nanos_utc: rfc3339_nanos(context.evaluated_at),
        freshness_max_age_secs: context.freshness_max_age_secs,
        future_tolerance_secs: context.future_tolerance_secs,
        gate_version: context.gate_version.clone(),
    };
    let ingress_gate_input_json = canonical_json(&gate_input)?;
    let ingress_gate_input_hash = sha256_json(&gate_input)?;
    let (ingress_decision, ingress_reason_code, ingress_retryable) =
        evaluate_ingress_gate(record.published_at, record.observed_at, context)?;
    let gate_receipt = IngressGateReceiptPreimage {
        domain: DOMAIN_INGRESS_GATE_RECEIPT.into(),
        ingress_run_id: context.stage_run_id.clone(),
        source_fact_key: prepared.source_fact_key.clone(),
        ingress_gate_input_hash: ingress_gate_input_hash.clone(),
        decision: ingress_decision,
        reason_code: ingress_reason_code.clone(),
        retryable: ingress_retryable,
        evaluated_at_rfc3339_nanos_utc: rfc3339_nanos(context.evaluated_at),
    };
    gate_receipt.validate()?;
    let ingress_gate_receipt_json = canonical_json(&gate_receipt)?;
    let ingress_gate_receipt_hash = sha256_json(&gate_receipt)?;
    Ok(SelectionSourceFactRowContentPreimage {
        domain: DOMAIN_SOURCE_FACT_ROW.into(),
        source_fact_key: prepared.source_fact_key.clone(),
        event_id: prepared.event_id.clone(),
        payload_schema: GLOBAL_NEWS_SOURCE_FACT_SCHEMA.into(),
        config_activation_run_id: context.config_activation_run_id.clone(),
        config_hash: context.config_hash.clone(),
        generation_market_date: context.generation_market_date.to_string(),
        provider_source: registration.source_contract.into(),
        item_id: record.item_id.clone(),
        title: record.title.clone(),
        summary: record.summary.clone(),
        content: record.content.clone(),
        publisher: record.publisher.clone(),
        canonical_url: record.canonical_url.clone(),
        published_at: rfc3339_nanos(record.published_at),
        instruments_json: canonical_json(&prepared.content.instruments_sorted)?,
        topics_json: canonical_json(&prepared.content.topics_sorted)?,
        language: record.language.clone(),
        record_provider: registration.provider_id.into(),
        record_source: registration.source_contract.into(),
        record_source_at: record.evidence.source_at().map(str::to_owned),
        record_observed_at: record.evidence.observed_at().to_owned(),
        record_batch_id: record.evidence.batch_id().to_owned(),
        record_batch_content_hash: record_batch_content_hash.into(),
        provider_content_hash: prepared.provider_content_hash.clone(),
        first_ingress_run_id: context.stage_run_id.clone(),
        ingress_gate_version: context.gate_version.clone(),
        ingress_gate_input_json,
        ingress_gate_input_hash,
        ingress_decision,
        ingress_reason_code,
        ingress_retryable,
        ingress_gate_receipt_json,
        ingress_gate_receipt_hash,
    })
}

fn evaluate_ingress_gate(
    published_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    context: &SourceIngressPreparationContext,
) -> Result<(IngressDecision, Option<String>, Option<bool>), IngressPreparationError> {
    let tolerance = i64::try_from(context.future_tolerance_secs).map_err(|_| {
        IngressPreparationError::new(
            "future_tolerance_overflow",
            "future tolerance exceeds supported duration",
        )
    })?;
    let max_age = i64::try_from(context.freshness_max_age_secs).map_err(|_| {
        IngressPreparationError::new(
            "freshness_threshold_overflow",
            "freshness threshold exceeds supported duration",
        )
    })?;
    let future_boundary = observed_at
        .checked_add_signed(chrono::Duration::seconds(tolerance))
        .ok_or_else(|| {
            IngressPreparationError::new(
                "future_boundary_overflow",
                "future tolerance cannot be represented",
            )
        })?;
    let published_local = fixed_shanghai()
        .from_utc_datetime(&published_at.naive_utc())
        .date_naive();
    let observed_local = fixed_shanghai()
        .from_utc_datetime(&observed_at.naive_utc())
        .date_naive();
    if published_at > future_boundary || published_local > observed_local {
        return Ok((
            IngressDecision::Rejected,
            Some("provider_publication_future".into()),
            Some(false),
        ));
    }
    let stale_boundary = published_at
        .checked_add_signed(chrono::Duration::seconds(max_age))
        .ok_or_else(|| {
            IngressPreparationError::new(
                "freshness_boundary_overflow",
                "freshness threshold cannot be represented",
            )
        })?;
    if published_local != observed_local || stale_boundary < observed_at {
        return Ok((
            IngressDecision::Rejected,
            Some("provider_publication_stale".into()),
            Some(false),
        ));
    }
    Ok((IngressDecision::Admitted, None, None))
}

fn global_news_request_evidence(
    registered: &RegisteredAttempt<'_>,
    limit: u32,
) -> Result<RequestEvidenceColumns, IngressPreparationError> {
    if limit > registered.registration.max_limit {
        return Err(IngressPreparationError::new(
            "global_news_limit_exceeds_registration",
            "request limit exceeds registered provider maximum",
        ));
    }
    let parameters = GlobalNewsRequestParametersPreimage {
        domain: DOMAIN_GLOBAL_NEWS_REQUEST.into(),
        feed_identity: registered.feed_identity.clone(),
        limit,
    };
    let capability = ProviderCapabilityHashPreimage {
        domain: DOMAIN_PROVIDER_CAPABILITY.into(),
        provider: registered.registration.gateway_provider.into(),
        capability_name: registered.registration.capability_name.into(),
        contract_version: GLOBAL_NEWS_CONTRACT_VERSION.into(),
        upstream_revision: registered.registration.upstream_revision.into(),
    };
    Ok(build_request_evidence(
        RequestParametersPreimage::GlobalNews(parameters),
        capability,
    )?)
}

fn run_payload_row_hashes(
    stage: &SourceIngressStageInputPreimage,
) -> Result<Vec<String>, IngressPreparationError> {
    let mut rows = Vec::new();
    for row in &stage.source_batch_attempt_rows {
        rows.push(run_row_hash(
            TABLE_SOURCE_BATCH_ATTEMPT,
            "selection_source_batch_attempts",
            vec![row.source_batch_attempt_id.clone()],
            sha256_json(row)?,
        )?);
    }
    for row in &stage.source_fact_rows {
        rows.push(run_row_hash(
            TABLE_SOURCE_FACT,
            "selection_source_facts_v2",
            vec![row.source_fact_key.clone()],
            sha256_json(row)?,
        )?);
    }
    for row in &stage.source_fact_attempt_rows {
        rows.push(run_row_hash(
            TABLE_SOURCE_FACT_ATTEMPT,
            "selection_source_fact_attempts",
            vec![row.source_fact_attempt_id.clone()],
            sha256_json(row)?,
        )?);
    }
    rows.sort_by(|left, right| {
        (left.table_ordinal, left.logical_primary_key.as_bytes())
            .cmp(&(right.table_ordinal, right.logical_primary_key.as_bytes()))
    });
    rows.into_iter().map(|row| Ok(sha256_json(&row)?)).collect()
}

fn run_row_hash(
    table_ordinal: u8,
    table_name: &str,
    key_parts: Vec<String>,
    row_content_hash: String,
) -> Result<RunRowHashPreimage, IngressPreparationError> {
    let logical_pk = RunRowLogicalPrimaryKeyPreimage {
        domain: DOMAIN_RUN_ROW_LOGICAL_PK.into(),
        table_ordinal,
        key_parts,
    };
    Ok(RunRowHashPreimage {
        domain: DOMAIN_RUN_ROW.into(),
        table_ordinal,
        table_name: table_name.into(),
        logical_primary_key: sha256_json(&logical_pk)?,
        row_content_hash,
    })
}

fn available_record_at(
    attempt: &RawGlobalNewsFeedAttempt,
    ordinal: usize,
) -> Result<&crate::data_gateway::GlobalNewsRecord, IngressPreparationError> {
    let terminal = attempt.terminal();
    terminal
        .records()
        .ok_or_else(|| {
            IngressPreparationError::new(
                "non_available_feed_record_access",
                "only Available feeds may expose source records",
            )
        })?
        .get(ordinal)
        .ok_or_else(|| {
            IngressPreparationError::new(
                "provider_record_ordinal_missing",
                "provider record ordinal is absent",
            )
        })
}

fn closed_feed_error_mapping(
    diagnostic_code: &str,
) -> Result<(ProviderErrorKind, &'static str, Option<&'static str>), IngressPreparationError> {
    match diagnostic_code {
        "provider_batch_unavailable" => Ok((
            ProviderErrorKind::Transport,
            "provider_batch_unavailable",
            None,
        )),
        "provider_evidence_invalid" => Ok((
            ProviderErrorKind::InvalidData,
            "provider_evidence_invalid",
            None,
        )),
        "available_batch_empty" => Ok((
            ProviderErrorKind::InvalidData,
            "available_batch_empty",
            None,
        )),
        "provider_request_invalid" => Ok((
            ProviderErrorKind::InvalidData,
            "provider_request_invalid",
            None,
        )),
        "provider_audit_unavailable" => Ok((
            ProviderErrorKind::Integrity,
            "provider_audit_unavailable",
            Some("provider-acquisition-audit-v1"),
        )),
        "provider_error_mapping_missing" => Ok((
            ProviderErrorKind::Integrity,
            "provider_error_mapping_missing",
            Some("provider-error-codes-v1"),
        )),
        _ => Err(IngressPreparationError::new(
            "unregistered_provider_diagnostic",
            "feed diagnostic code is outside the closed provider registry",
        )),
    }
}

fn parse_provider_time(
    registration: RegisteredGlobalNewsFeed,
    value: &str,
) -> Result<DateTime<Utc>, IngressPreparationError> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp.with_timezone(&Utc));
    }
    if registration.provider == crate::data_gateway::GlobalNewsProvider::Eastmoney {
        let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").map_err(|_| {
            IngressPreparationError::new(
                "provider_publication_time_invalid",
                "Eastmoney publication time does not match its registered format",
            )
        })?;
        return fixed_shanghai()
            .from_local_datetime(&naive)
            .single()
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .ok_or_else(|| {
                IngressPreparationError::new(
                    "provider_publication_time_ambiguous",
                    "provider publication time is ambiguous",
                )
            });
    }
    Err(IngressPreparationError::new(
        "provider_publication_time_invalid",
        "provider publication time does not match RFC3339",
    ))
}

fn parse_observed_at(value: &str) -> Result<DateTime<Utc>, IngressPreparationError> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp.with_timezone(&Utc));
    }
    let (seconds, nanos) = value.split_once('.').ok_or_else(|| {
        IngressPreparationError::new(
            "provider_observation_time_invalid",
            "observation time must be RFC3339 or seconds.nanoseconds",
        )
    })?;
    if nanos.len() != 9 || !nanos.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(IngressPreparationError::new(
            "provider_observation_time_invalid",
            "epoch observation nanoseconds must contain exactly nine digits",
        ));
    }
    let seconds = seconds.parse::<i64>().map_err(|_| {
        IngressPreparationError::new(
            "provider_observation_time_invalid",
            "epoch observation seconds are invalid",
        )
    })?;
    let nanos = nanos.parse::<u32>().map_err(|_| {
        IngressPreparationError::new(
            "provider_observation_time_invalid",
            "epoch observation nanoseconds are invalid",
        )
    })?;
    DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
        IngressPreparationError::new(
            "provider_observation_time_invalid",
            "epoch observation time is out of range",
        )
    })
}

fn sorted_unique(
    values: &[String],
    duplicate_code: &'static str,
) -> Result<Vec<String>, IngressPreparationError> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    if sorted.iter().any(String::is_empty) {
        return Err(IngressPreparationError::new(
            "source_record_list_value_empty",
            "source record list contains an empty value",
        ));
    }
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(IngressPreparationError::new(
            duplicate_code,
            "source record list contains a duplicate value",
        ));
    }
    Ok(sorted)
}

fn event_projection_id(provider_source: &str, item_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(EVENT_ID_DOMAIN);
    hasher.update(provider_source.as_bytes());
    hasher.update(b"\0");
    hasher.update(item_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn fixed_shanghai() -> FixedOffset {
    FixedOffset::east_opt(8 * 60 * 60).expect("UTC+08:00 is a valid fixed offset")
}

fn rfc3339_nanos(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn require_non_empty(value: &str, _field: &'static str) -> Result<(), IngressPreparationError> {
    if value.is_empty() {
        Err(IngressPreparationError::new(
            "required_field_empty",
            "required source-ingress field is empty",
        ))
    } else {
        Ok(())
    }
}

fn require_hash(value: &str) -> Result<(), IngressPreparationError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(IngressPreparationError::new(
            "invalid_sha256",
            "identity hash must be lowercase 64-hex",
        ))
    }
}

fn require_snake_token(value: &str, _field: &'static str) -> Result<(), IngressPreparationError> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Ok(())
    } else {
        Err(IngressPreparationError::new(
            "invalid_reason_token",
            "failure fields must be lowercase ASCII snake_case",
        ))
    }
}

fn reject_control(value: &str) -> Result<(), IngressPreparationError> {
    if value.chars().any(char::is_control) {
        Err(IngressPreparationError::new(
            "identity_control_character",
            "source-fact identity contains a control character",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{BatchEvidence, GlobalNewsProvider, GlobalNewsRecord};
    use crate::magic_compat::SourceEvidence;
    use crate::news::aggregator::raw_v2::{RawGlobalNewsFeedAttempt, TestRawGlobalNewsTerminal};

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("TEST_CODE valid RFC3339")
            .with_timezone(&Utc)
    }

    fn test_context() -> SourceIngressPreparationContext {
        SourceIngressPreparationContext {
            stage_run_id: "01900000-0000-7000-8000-000000000174".into(),
            config_activation_run_id: "01900000-0000-7000-8000-000000000173".into(),
            config_hash: "a".repeat(64),
            config_effective_from: utc("2026-07-28T00:00:00Z"),
            generation_market_date: NaiveDate::from_ymd_opt(2026, 7, 28)
                .expect("TEST_CODE generation date"),
            per_feed_limit: 20,
            gate_version: "br137-ingress-v1".into(),
            freshness_max_age_secs: 86_400,
            future_tolerance_secs: 0,
            evaluated_at: utc("2026-07-28T02:00:10Z"),
            enveloped_at: utc("2026-07-28T02:00:11Z"),
        }
    }

    fn evidence(provider: GlobalNewsProvider, source_at: &str) -> BatchEvidence {
        BatchEvidence {
            provider: provider.provider_id(),
            source: provider.source().into(),
            source_at: Some(source_at.into()),
            observed_at: "2026-07-28T02:00:05.000000000Z".into(),
            batch_id: format!("TEST_CODE_{}_batch", provider.feed_name()),
        }
    }

    fn record(
        provider: GlobalNewsProvider,
        item_id: &str,
        title: &str,
        published_at: &str,
    ) -> GlobalNewsRecord {
        let evidence = evidence(provider, published_at);
        GlobalNewsRecord {
            item_id: item_id.into(),
            title: title.into(),
            summary: Some(format!("TEST_CODE summary {item_id}")),
            content: None,
            publisher: "TEST_CODE publisher".into(),
            canonical_url: format!("https://example.com/{item_id}"),
            published_at: utc(published_at),
            observed_at: utc("2026-07-28T02:00:05Z"),
            instruments: vec!["TEST_CODE_600000".into()],
            topics: vec!["TEST_CODE topic".into()],
            language: "zh-CN".into(),
            evidence: SourceEvidence::new(
                provider.provider_id(),
                evidence.observed_at,
                evidence.batch_id,
            )
            .expect("TEST_CODE record evidence")
            .with_source_at(published_at)
            .expect("TEST_CODE record publication"),
        }
    }

    fn raw_batch(
        eastmoney_records: Vec<GlobalNewsRecord>,
        jin10_unavailable: bool,
    ) -> RawNewsAggregationBatch {
        let attempted_at = utc("2026-07-28T02:00:04Z");
        let attempts = registered_global_news_feeds()
            .into_iter()
            .map(|registration| {
                let terminal = if registration.provider == GlobalNewsProvider::Eastmoney {
                    TestRawGlobalNewsTerminal::Available {
                        records: eastmoney_records.clone(),
                        evidence: evidence(
                            registration.provider,
                            eastmoney_records
                                .first()
                                .map(|record| {
                                    record.evidence.source_at().expect("TEST_CODE publication")
                                })
                                .unwrap_or("2026-07-28T02:00:00Z"),
                        ),
                    }
                } else if registration.provider == GlobalNewsProvider::Jin10 && jin10_unavailable {
                    TestRawGlobalNewsTerminal::Unavailable {
                        failed_stage: "global_news_gateway",
                        diagnostic_code: "provider_batch_unavailable",
                        reason_code: "no_verified_batch",
                        retryable: true,
                        available_evidence: None,
                    }
                } else {
                    TestRawGlobalNewsTerminal::VerifiedEmpty {
                        evidence: evidence(registration.provider, "2026-07-28T02:00:00Z"),
                    }
                };
                RawGlobalNewsFeedAttempt::test_fixture(
                    "TEST_CODE_ingress_attempt",
                    registration,
                    attempted_at,
                    terminal,
                )
            })
            .collect();
        RawNewsAggregationBatch::test_fixture(
            "TEST_CODE_ingress_batch",
            attempts,
            utc("2026-07-28T02:00:06Z"),
        )
    }

    #[test]
    fn preserves_available_verified_empty_and_unavailable_terminals() {
        let batch = raw_batch(
            vec![record(
                GlobalNewsProvider::Eastmoney,
                "TEST_CODE_item",
                "TEST_CODE title",
                "2026-07-28T02:00:00Z",
            )],
            true,
        );
        let prepared =
            prepare_source_ingress(&batch, &test_context(), &[]).expect("TEST_CODE ingress");

        assert_eq!(prepared.stage_input.source_batch_attempt_rows.len(), 4);
        assert_eq!(prepared.stage_input.source_fact_attempt_rows.len(), 1);
        assert_eq!(prepared.stage_input.source_fact_rows.len(), 1);
        let statuses = prepared
            .stage_input
            .source_batch_attempt_rows
            .iter()
            .map(|row| row.status_kind)
            .collect::<Vec<_>>();
        assert!(statuses.contains(&FeedStatusKind::Available));
        assert!(statuses.contains(&FeedStatusKind::VerifiedEmpty));
        assert!(statuses.contains(&FeedStatusKind::Unavailable));
        for row in &prepared.stage_input.source_batch_attempt_rows {
            assert!(!row.request_evidence_json.is_empty());
            assert_eq!(
                sha256_bytes(row.request_evidence_json.as_bytes()),
                row.request_evidence_hash
            );
            row.validate()
                .expect("every terminal retains typed request evidence");
        }
        let unavailable = prepared
            .stage_input
            .source_batch_attempt_rows
            .iter()
            .find(|row| row.status_kind == FeedStatusKind::Unavailable)
            .expect("TEST_CODE unavailable row");
        assert!(unavailable.batch_content_hash.is_none());
        assert!(unavailable.error_detail_json.is_some());
        assert!(unavailable.error_fingerprint.is_some());
        assert_eq!(
            prepared.recovery_envelope.payload_json,
            canonical_json(&prepared.stage_input).expect("TEST_CODE canonical stage input")
        );
        assert_eq!(
            prepared.recovery_envelope.payload_json_hash,
            sha256_bytes(prepared.recovery_envelope.payload_json.as_bytes())
        );
    }

    #[test]
    fn request_evidence_tamper_is_rejected_before_staging() {
        let prepared = prepare_source_ingress(
            &raw_batch(
                vec![record(
                    GlobalNewsProvider::Eastmoney,
                    "TEST_CODE_request_evidence",
                    "TEST_CODE request evidence",
                    "2026-07-28T02:00:00Z",
                )],
                false,
            ),
            &test_context(),
            &[],
        )
        .expect("TEST_CODE ingress");
        let mut row = prepared.stage_input.source_batch_attempt_rows[0].clone();
        row.request_evidence_json.push(' ');
        assert_eq!(
            row.validate()
                .expect_err("non-canonical request evidence must fail")
                .code,
            "typed_json_noncanonical"
        );
    }

    #[test]
    fn first_source_is_authoritative_then_replay_and_conflict_append() {
        let provider = GlobalNewsProvider::Eastmoney;
        let first_batch = raw_batch(
            vec![record(
                provider,
                "TEST_CODE_identity",
                "TEST_CODE authoritative",
                "2026-07-28T02:00:00Z",
            )],
            false,
        );
        let first = prepare_source_ingress(&first_batch, &test_context(), &[])
            .expect("TEST_CODE first ingress");
        assert_eq!(first.stage_input.source_fact_rows.len(), 1);
        assert_eq!(
            first.stage_input.source_fact_attempt_rows[0].attempt_result,
            SourceFactAttemptResult::Accepted
        );

        let authority = ExistingSourceFactAuthority {
            source_fact_key: first.stage_input.source_fact_rows[0]
                .source_fact_key
                .clone(),
            provider_content_hash: first.stage_input.source_fact_rows[0]
                .provider_content_hash
                .clone(),
        };
        let repeated_batch = raw_batch(
            vec![record(
                provider,
                "TEST_CODE_identity",
                "TEST_CODE authoritative",
                "2026-07-28T02:00:00Z",
            )],
            false,
        );
        let replayed = prepare_source_ingress(
            &repeated_batch,
            &test_context(),
            std::slice::from_ref(&authority),
        )
        .expect("TEST_CODE replay");
        assert!(replayed.stage_input.source_fact_rows.is_empty());
        assert_eq!(
            replayed.stage_input.source_fact_attempt_rows[0].attempt_result,
            SourceFactAttemptResult::Replay
        );
        assert!(replayed.stage_input.source_fact_attempt_rows[0]
            .conflict_hash
            .is_none());

        let conflict_batch = raw_batch(
            vec![record(
                provider,
                "TEST_CODE_identity",
                "TEST_CODE conflicting content",
                "2026-07-28T02:00:00Z",
            )],
            false,
        );
        let conflicted = prepare_source_ingress(&conflict_batch, &test_context(), &[authority])
            .expect("TEST_CODE conflict");
        assert!(conflicted.stage_input.source_fact_rows.is_empty());
        assert_eq!(
            conflicted.stage_input.source_fact_attempt_rows[0].attempt_result,
            SourceFactAttemptResult::Conflict
        );
        assert!(conflicted.stage_input.source_fact_attempt_rows[0]
            .conflict_hash
            .is_some());
    }

    #[test]
    fn same_day_is_admitted_while_prior_day_and_future_are_rejected() {
        let provider = GlobalNewsProvider::Eastmoney;
        let records = vec![
            record(
                provider,
                "TEST_CODE_future",
                "TEST_CODE future",
                "2026-07-28T02:00:06Z",
            ),
            record(
                provider,
                "TEST_CODE_same_day",
                "TEST_CODE same day",
                "2026-07-28T01:00:00Z",
            ),
            record(
                provider,
                "TEST_CODE_stale",
                "TEST_CODE stale",
                "2026-07-27T15:59:59Z",
            ),
        ];
        let prepared = prepare_source_ingress(&raw_batch(records, false), &test_context(), &[])
            .expect("TEST_CODE freshness ingress");
        let decision = |item_id: &str| {
            prepared
                .stage_input
                .source_fact_rows
                .iter()
                .find(|row| row.item_id == item_id)
                .expect("TEST_CODE source fact decision")
        };
        let same_day = decision("TEST_CODE_same_day");
        assert_eq!(same_day.ingress_decision, IngressDecision::Admitted);
        assert!(same_day.ingress_reason_code.is_none());
        let stale = decision("TEST_CODE_stale");
        assert_eq!(stale.ingress_decision, IngressDecision::Rejected);
        assert_eq!(
            stale.ingress_reason_code.as_deref(),
            Some("provider_publication_stale")
        );
        let future = decision("TEST_CODE_future");
        assert_eq!(future.ingress_decision, IngressDecision::Rejected);
        assert_eq!(
            future.ingress_reason_code.as_deref(),
            Some("provider_publication_future")
        );
    }
}
