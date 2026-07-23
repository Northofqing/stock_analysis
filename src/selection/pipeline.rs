//! BR-155/BR-156/BR-157 durable event-scoped selection orchestration.
//!
//! This module owns the state transition. It deliberately has no push, paper
//! trading, or order capability.

use crate::database::selection::{
    CompletionStatus, EventCompletion, FeatureSnapshotInput, InboxEvent, SelectionBatchInput,
    SelectionCandidateInput, SelectionRunInput, VisibilityReceiptInput,
};
use crate::news::aggregator::{FeedAttempt, NewsAggregationBatch};
use crate::selection::admission::{
    evaluate_admission, AdmissionDecision, SelectionEvaluationWindow,
};
use crate::selection::audit::{
    AuditAppendReceipt, SelectionAuditContext, SelectionAuditEnvironment, SelectionAuditPhase,
    SelectionAuditRecord, SelectionAuditWriter,
};
use crate::selection::features::{
    compute_daily_features, IntradayVolumeEvidence, RawSelectionFeatures, T0MarketEvidence,
    FEATURE_VERSION,
};
use crate::selection::magic_tdx::{
    fetch_selection_market_batch, SelectionEventReference, SelectionFiveMinuteBar,
    SelectionMarketBatch, SelectionMarketRequest, SelectionMarketWindow, SelectionSourceError,
};
use crate::selection::model::{CandidateIdentity, DirectMentionEvidence};
use crate::selection::relation::{map_events, ChainConfigSnapshot, ChainMatch};
use crate::signal::market_event::MarketEvent;
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveTime};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const RELATION_VERSION: &str = "direct-mention-v1";
const PIPELINE_VERSION: &str = "event-selection-v1";
const DEFAULT_PENDING_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct SelectionEventBatch {
    events: Vec<MarketEvent>,
    source_attempts: Vec<FeedAttempt>,
    observed_at: DateTime<Local>,
    batch_id: String,
    content_hash: String,
}

impl SelectionEventBatch {
    pub fn events(&self) -> &[MarketEvent] {
        &self.events
    }

    pub fn source_attempts(&self) -> &[FeedAttempt] {
        &self.source_attempts
    }

    pub fn observed_at(&self) -> DateTime<Local> {
        self.observed_at
    }

    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn sources_complete(&self) -> bool {
        !self.source_attempts.is_empty()
            && self.source_attempts.iter().all(|attempt| {
                matches!(
                    attempt.status,
                    crate::news::aggregator::FeedAttemptStatus::Succeeded { .. }
                )
            })
    }
}

impl TryFrom<NewsAggregationBatch> for SelectionEventBatch {
    type Error = SelectionPipelineError;

    fn try_from(batch: NewsAggregationBatch) -> Result<Self, Self::Error> {
        if batch.source_attempts.is_empty() {
            return Err(pipeline_error(
                "source_attempts_missing",
                "selection input has no per-feed source evidence",
                true,
            ));
        }

        let mut feed_identities = BTreeSet::new();
        for attempt in &batch.source_attempts {
            let feed_name = attempt.feed_name.trim();
            let source_kind = attempt.source_kind.trim();
            if feed_name.is_empty() || source_kind.is_empty() {
                return Err(pipeline_error(
                    "source_attempt_identity_invalid",
                    "selection source attempt has a blank feed or source kind",
                    false,
                ));
            }
            if !feed_identities.insert((feed_name, source_kind)) {
                return Err(pipeline_error(
                    "duplicate_source_attempt",
                    format!("duplicate selection source attempt: {feed_name}/{source_kind}"),
                    false,
                ));
            }
        }

        let mut event_ids = BTreeSet::new();
        for event in &batch.events {
            if event.event_id.trim().is_empty() {
                return Err(pipeline_error(
                    "event_identity_missing",
                    "selection event identity is blank",
                    false,
                ));
            }
            if !event_ids.insert(event.event_id.as_str()) {
                return Err(pipeline_error(
                    "duplicate_event_identity",
                    format!("selection batch repeats event {}", event.event_id),
                    false,
                ));
            }
        }

        #[derive(Serialize)]
        struct BatchIdentity<'a> {
            events: &'a [MarketEvent],
            source_attempts: &'a [FeedAttempt],
            observed_at: DateTime<FixedOffset>,
        }
        let observed_at = batch.observed_at.fixed_offset();
        let content_hash = stable_hash(
            "stock_analysis.selection_source_batch.v1",
            &BatchIdentity {
                events: &batch.events,
                source_attempts: &batch.source_attempts,
                observed_at,
            },
        )?;
        let batch_id = format!("selection_source_batch_{content_hash}");

        Ok(Self {
            events: batch.events,
            source_attempts: batch.source_attempts,
            observed_at: batch.observed_at,
            batch_id,
            content_hash,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SelectionContext {
    pub window: SelectionMarketWindow,
    pub evaluation_at: DateTime<Local>,
    pub expected_latest_settled_date: NaiveDate,
    pub pending_limit: usize,
}

impl SelectionContext {
    pub fn new(
        window: SelectionMarketWindow,
        evaluation_at: DateTime<Local>,
        expected_latest_settled_date: NaiveDate,
    ) -> Self {
        Self {
            window,
            evaluation_at,
            expected_latest_settled_date,
            pending_limit: DEFAULT_PENDING_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedSelection {
    pub evaluated_events: usize,
    pub admitted_candidates: usize,
    pub rejected_candidates: usize,
    pub pending_events: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEmptySelection {
    pub evaluated_events: usize,
    pub source_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableSelection {
    pub reason_code: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionRunOutcome {
    Completed(CompletedSelection),
    VerifiedEmpty(VerifiedEmptySelection),
    Unavailable(UnavailableSelection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionPipelineError {
    code: String,
    message: String,
    retryable: bool,
}

impl SelectionPipelineError {
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }
}

impl std::fmt::Display for SelectionPipelineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SelectionPipelineError {}

#[async_trait]
pub(crate) trait SelectionMarketPort: Send + Sync {
    async fn fetch(
        &self,
        request: SelectionMarketRequest,
    ) -> Result<SelectionMarketBatch, SelectionPipelineError>;
}

pub(crate) trait SelectionConfigPort: Send + Sync {
    fn snapshot(&self) -> Result<ChainConfigSnapshot, SelectionPipelineError>;
}

pub(crate) trait SelectionRepositoryPort: Send + Sync {
    fn ingest_event(&self, event: &InboxEvent) -> Result<(), SelectionPipelineError>;
    fn pending_events(&self, limit: usize) -> Result<Vec<InboxEvent>, SelectionPipelineError>;
    fn stage_batch(&self, batch: &SelectionBatchInput) -> Result<(), SelectionPipelineError>;
    fn run_is_visible(&self, run_id: &str) -> Result<bool, SelectionPipelineError>;
    fn publish_visibility(
        &self,
        receipt: &VisibilityReceiptInput,
    ) -> Result<(), SelectionPipelineError>;
    fn append_completion(&self, completion: &EventCompletion)
        -> Result<(), SelectionPipelineError>;
}

pub(crate) trait SelectionAuditPort: Send + Sync {
    fn append(
        &self,
        record: SelectionAuditRecord,
    ) -> Result<AuditAppendReceipt, SelectionPipelineError>;
}

pub(crate) struct SelectionPipeline {
    repository: Arc<dyn SelectionRepositoryPort>,
    market: Arc<dyn SelectionMarketPort>,
    config: Arc<dyn SelectionConfigPort>,
    audit: Arc<dyn SelectionAuditPort>,
}

impl SelectionPipeline {
    pub(crate) fn new(
        repository: Arc<dyn SelectionRepositoryPort>,
        market: Arc<dyn SelectionMarketPort>,
        config: Arc<dyn SelectionConfigPort>,
        audit: Arc<dyn SelectionAuditPort>,
    ) -> Self {
        Self {
            repository,
            market,
            config,
            audit,
        }
    }

    pub(crate) async fn evaluate(
        &self,
        batch: SelectionEventBatch,
        context: SelectionContext,
    ) -> SelectionRunOutcome {
        match self.evaluate_inner(&batch, &context).await {
            Ok(outcome) => outcome,
            Err(error) => SelectionRunOutcome::Unavailable(UnavailableSelection {
                reason_code: error.code,
                retryable: error.retryable,
            }),
        }
    }

    async fn evaluate_inner(
        &self,
        batch: &SelectionEventBatch,
        context: &SelectionContext,
    ) -> Result<SelectionRunOutcome, SelectionPipelineError> {
        validate_context(context)?;
        let now = context.evaluation_at.fixed_offset();
        let evaluation_market_date = context.evaluation_at.date_naive();
        let mut rejected_events = 0usize;

        for event in batch.events() {
            let provider = event_provider(event);
            let event_hash = event_content_hash(event)?;
            if provider.is_none() {
                self.append_rejection(
                    event,
                    &event.event_id,
                    &event_hash,
                    vec!["provider_missing".to_owned()],
                    vec!["BR-155".to_owned(), "2.2".to_owned()],
                    false,
                    None,
                    None,
                    None,
                    now,
                )?;
                rejected_events += 1;
                continue;
            }

            let inbox = InboxEvent {
                event_id: event.event_id.clone(),
                content_hash: event_hash.clone(),
                payload_json: serde_json::to_string(event).map_err(|error| {
                    pipeline_error(
                        "event_serialize_failed",
                        format!("serialize selection event: {error}"),
                        false,
                    )
                })?,
                provider: provider.clone().expect("provider checked"),
                provider_published_at: event
                    .provider_publication
                    .as_ref()
                    .and_then(|publication| publication.published_at)
                    .map(|timestamp| timestamp.fixed_offset()),
                provider_published_on: event.provider_publication.as_ref().and_then(
                    |publication| {
                        publication
                            .published_at
                            .is_none()
                            .then_some(publication.published_on)
                    },
                ),
                observed_at: batch.observed_at().fixed_offset(),
                source_batch_id: batch.batch_id().to_owned(),
                source_batch_hash: batch.content_hash().to_owned(),
                evaluation_market_date,
            };

            let ingested = SelectionAuditRecord::new(
                SelectionAuditPhase::Ingested,
                &event.event_id,
                &event_hash,
                now,
            )
            .with_context(SelectionAuditContext {
                event_identity_hash: Some(identity_hash("event", &event.event_id)),
                provider: provider.clone(),
                provider_published_at: inbox.provider_published_at,
                observed_at: Some(inbox.observed_at),
                rule_ids: vec!["BR-155".to_owned()],
                ..SelectionAuditContext::default()
            });
            self.audit.append(ingested)?;
            self.repository.ingest_event(&inbox)?;
        }

        let pending = self.repository.pending_events(context.pending_limit)?;
        if pending.is_empty() {
            if rejected_events > 0 {
                return Ok(SelectionRunOutcome::Completed(CompletedSelection {
                    evaluated_events: rejected_events,
                    admitted_candidates: 0,
                    rejected_candidates: 0,
                    pending_events: 0,
                }));
            }
            if batch.sources_complete() {
                return Ok(SelectionRunOutcome::VerifiedEmpty(VerifiedEmptySelection {
                    evaluated_events: 0,
                    source_count: batch.source_attempts().len(),
                }));
            }
            return Err(pipeline_error(
                "source_batch_incomplete",
                "incomplete feed evidence cannot be verified empty",
                true,
            ));
        }

        let mut eligible = Vec::new();
        let mut terminal_before_market = Vec::new();
        for inbox in pending {
            let event: MarketEvent =
                serde_json::from_str(&inbox.payload_json).map_err(|error| {
                    pipeline_error(
                        "persisted_event_invalid",
                        format!("deserialize persisted selection event: {error}"),
                        false,
                    )
                })?;
            match validate_event_gate(&event, context.evaluation_at) {
                Ok(()) => eligible.push((inbox, event)),
                Err(reason_code) => {
                    self.append_rejection(
                        &event,
                        &event.event_id,
                        &event_content_hash(&event)?,
                        vec![reason_code.to_owned()],
                        vec!["BR-155".to_owned(), "2.4".to_owned()],
                        false,
                        None,
                        None,
                        None,
                        now,
                    )?;
                    terminal_before_market.push(rejected_completion(
                        &event.event_id,
                        reason_code,
                        now,
                    )?);
                }
            }
        }
        for completion in &terminal_before_market {
            self.repository.append_completion(completion)?;
        }
        rejected_events += terminal_before_market.len();

        if eligible.is_empty() {
            return Ok(SelectionRunOutcome::Completed(CompletedSelection {
                evaluated_events: rejected_events,
                admitted_candidates: 0,
                rejected_candidates: 0,
                pending_events: 0,
            }));
        }

        let config = self.config.snapshot()?;
        let events = eligible
            .iter()
            .map(|(_, event)| event.clone())
            .collect::<Vec<_>>();
        let mappings = map_events(&events, &config)
            .into_iter()
            .map(|mapping| (mapping.event_id.clone(), mapping))
            .collect::<BTreeMap<_, _>>();
        let request = SelectionMarketRequest {
            event_references: events
                .iter()
                .map(|event| SelectionEventReference {
                    event_id: event.event_id.clone(),
                    text: event_text(event),
                })
                .collect(),
            window: context.window,
            evaluation_at: context.evaluation_at,
            expected_latest_settled_date: context.expected_latest_settled_date,
        };
        let market_batch = self.market.fetch(request).await?;
        let market_batch_hash = market_batch_hash(&market_batch)?;
        let records = market_batch
            .records
            .iter()
            .map(|record| (record.security.code.as_str(), record))
            .collect::<BTreeMap<_, _>>();

        let mut admitted = Vec::new();
        let mut feature_snapshots = Vec::new();
        let mut completions = Vec::new();
        let mut rejected_candidates = 0usize;
        let mut events_with_relations = 0usize;
        let mut retry_pending = 0usize;

        for (inbox, event) in &eligible {
            let mapping = mappings.get(&event.event_id).ok_or_else(|| {
                pipeline_error(
                    "event_mapping_missing",
                    format!("missing mapping for event {}", event.event_id),
                    false,
                )
            })?;
            let mentions = market_batch
                .event_mentions
                .get(&event.event_id)
                .cloned()
                .unwrap_or_default();

            if mapping.chains.is_empty() || mentions.is_empty() {
                completions.push(completed_completion(&event.event_id, now)?);
                continue;
            }
            events_with_relations += 1;
            let mut event_has_terminal_ticket = false;
            let mut event_has_retryable_source_failure = false;

            for chain in &mapping.chains {
                for mention in &mentions {
                    let Some(record) = records.get(mention.security.code.as_str()).copied() else {
                        let rejection = market_batch.rejections.iter().find(|rejection| {
                            rejection.event_id.as_deref() == Some(event.event_id.as_str())
                                && rejection.security_code.as_deref()
                                    == Some(mention.security.code.as_str())
                        });
                        let reason_code = rejection
                            .map(|rejection| rejection.reason_code.clone())
                            .unwrap_or_else(|| "magic_tdx_security_record_missing".to_owned());
                        let retryable = rejection.is_none_or(|rejection| rejection.retryable);
                        self.append_rejection(
                            event,
                            &ticket_subject(event, chain, mention),
                            &stable_hash(
                                "stock_analysis.selection_source_rejection.v1",
                                &(
                                    &event.event_id,
                                    &chain.chain_id,
                                    &mention.security.code,
                                    &reason_code,
                                ),
                            )?,
                            vec![reason_code],
                            vec!["BR-155".to_owned(), "2.1".to_owned()],
                            retryable,
                            Some(chain),
                            Some(mention),
                            Some(&market_batch.batch_id),
                            now,
                        )?;
                        event_has_retryable_source_failure |= retryable;
                        rejected_candidates += 1;
                        continue;
                    };

                    let features = match features_for_record(context.window, record) {
                        Ok(features) => features,
                        Err(error) => {
                            self.append_rejection(
                                event,
                                &ticket_subject(event, chain, mention),
                                &stable_hash(
                                    "stock_analysis.selection_feature_rejection.v1",
                                    &(
                                        &event.event_id,
                                        &chain.chain_id,
                                        &mention.security.code,
                                        error.code(),
                                    ),
                                )?,
                                vec![error.code().to_owned()],
                                vec!["BR-156".to_owned(), "2.2".to_owned(), "2.3".to_owned()],
                                false,
                                Some(chain),
                                Some(mention),
                                Some(&market_batch.batch_id),
                                now,
                            )?;
                            event_has_terminal_ticket = true;
                            rejected_candidates += 1;
                            continue;
                        }
                    };
                    let t0_market_evidence = match t0_market_evidence(context.window, record) {
                        Ok(evidence) => evidence,
                        Err(error) => {
                            self.append_rejection(
                                event,
                                &ticket_subject(event, chain, mention),
                                &stable_hash(
                                    "stock_analysis.selection_t0_evidence_rejection.v1",
                                    &(
                                        &event.event_id,
                                        &chain.chain_id,
                                        &mention.security.code,
                                        error.code(),
                                    ),
                                )?,
                                vec![error.code().to_owned()],
                                vec!["BR-156".to_owned(), "2.2".to_owned(), "2.3".to_owned()],
                                false,
                                Some(chain),
                                Some(mention),
                                Some(&market_batch.batch_id),
                                now,
                            )?;
                            event_has_terminal_ticket = true;
                            rejected_candidates += 1;
                            continue;
                        }
                    };
                    let decision = evaluate_admission(evaluation_window(context.window), &features);
                    match decision {
                        AdmissionDecision::Rejected(rejection) => {
                            self.append_rejection(
                                event,
                                &ticket_subject(event, chain, mention),
                                &stable_hash(
                                    "stock_analysis.selection_admission_rejection.v1",
                                    &(
                                        &event.event_id,
                                        &chain.chain_id,
                                        &mention.security.code,
                                        &rejection,
                                    ),
                                )?,
                                rejection
                                    .failures
                                    .iter()
                                    .map(|failure| failure.code.clone())
                                    .collect(),
                                vec!["BR-156".to_owned()],
                                false,
                                Some(chain),
                                Some(mention),
                                Some(&market_batch.batch_id),
                                now,
                            )?;
                            event_has_terminal_ticket = true;
                            rejected_candidates += 1;
                        }
                        AdmissionDecision::Admitted { admission_version } => {
                            event_has_terminal_ticket = true;
                            admitted.push(PreparedCandidate {
                                event: event.clone(),
                                chain: chain.clone(),
                                mention: mention.clone(),
                                features,
                                t0_market_evidence,
                                admission_version,
                                observed_at: record.observed_at.fixed_offset(),
                                inbox_content_hash: inbox.content_hash.clone(),
                            });
                        }
                    }
                }
            }

            if event_has_terminal_ticket {
                if admitted
                    .iter()
                    .any(|candidate| candidate.event.event_id == event.event_id)
                {
                    completions.push(completed_completion(&event.event_id, now)?);
                } else {
                    completions.push(rejected_completion(
                        &event.event_id,
                        "all_candidates_rejected",
                        now,
                    )?);
                }
            } else if event_has_retryable_source_failure {
                retry_pending += 1;
            } else {
                completions.push(completed_completion(&event.event_id, now)?);
            }
        }

        admitted.sort_by(|left, right| {
            publication_sort_key(&left.event)
                .cmp(&publication_sort_key(&right.event))
                .then_with(|| left.event.event_id.cmp(&right.event.event_id))
                .then_with(|| left.chain.chain_id.cmp(&right.chain.chain_id))
                .then_with(|| left.mention.security.code.cmp(&right.mention.security.code))
        });
        admitted.dedup_by(|left, right| {
            left.event.event_id == right.event.event_id
                && left.chain.chain_id == right.chain.chain_id
                && left.mention.security.code == right.mention.security.code
        });

        let prepared_content_hash = stable_hash(
            "stock_analysis.selection_prepared.v1",
            &PreparedIdentity {
                source_batch_hash: batch.content_hash(),
                config_hash: config.content_hash(),
                magic_tdx_batch_hash: &market_batch_hash,
                candidates: admitted
                    .iter()
                    .map(PreparedCandidate::identity_tuple)
                    .collect(),
                rejected_candidates,
            },
        )?;
        let run_id = format!(
            "selection_run_{}",
            stable_hash(
                "stock_analysis.selection_run_identity.v1",
                &(
                    evaluation_market_date,
                    config.content_hash(),
                    &market_batch.batch_id,
                    &prepared_content_hash,
                ),
            )?
        );
        self.audit.append(
            SelectionAuditRecord::new(
                SelectionAuditPhase::Prepared,
                &run_id,
                &prepared_content_hash,
                now,
            )
            .with_context(SelectionAuditContext {
                magic_tdx_batch_id: Some(market_batch.batch_id.clone()),
                rule_ids: vec![
                    "BR-155".to_owned(),
                    "BR-156".to_owned(),
                    "BR-157".to_owned(),
                ],
                ..SelectionAuditContext::default()
            }),
        )?;

        if admitted.is_empty() {
            for completion in &completions {
                self.repository.append_completion(completion)?;
            }
            if retry_pending > 0 {
                return Err(pipeline_error(
                    "candidate_sources_retryable",
                    "all direct candidate market sources are retryable failures",
                    true,
                ));
            }
            if events_with_relations == 0 && batch.sources_complete() {
                return Ok(SelectionRunOutcome::VerifiedEmpty(VerifiedEmptySelection {
                    evaluated_events: eligible.len() + rejected_events,
                    source_count: batch.source_attempts().len(),
                }));
            }
            if events_with_relations == 0 && !batch.sources_complete() {
                return Err(pipeline_error(
                    "source_batch_incomplete",
                    "incomplete feed evidence cannot be verified empty",
                    true,
                ));
            }
            return Ok(SelectionRunOutcome::Completed(CompletedSelection {
                evaluated_events: completions.len() + rejected_events,
                admitted_candidates: 0,
                rejected_candidates,
                pending_events: retry_pending,
            }));
        }

        let run = SelectionRunInput {
            run_id: run_id.clone(),
            content_hash: stable_hash(
                "stock_analysis.selection_run.v1",
                &(
                    &run_id,
                    evaluation_market_date,
                    config.content_hash(),
                    &market_batch.batch_id,
                    &market_batch_hash,
                ),
            )?,
            evaluation_market_date,
            config_hash: config.content_hash().to_owned(),
            magic_tdx_batch_id: market_batch.batch_id.clone(),
            magic_tdx_batch_hash: market_batch_hash.clone(),
            created_at: now,
        };

        for (ordinal, prepared) in admitted.iter().enumerate() {
            let candidate_identity = CandidateIdentity::new(
                &prepared.event.event_id,
                &prepared.chain.chain_id,
                &prepared.mention.security.code,
                RELATION_VERSION,
                FEATURE_VERSION,
                evaluation_market_date,
            );
            let candidate_id = candidate_identity.as_str().to_owned();
            let ordinal = i32::try_from(ordinal).map_err(|_| {
                pipeline_error(
                    "candidate_ordinal_overflow",
                    "candidate ordinal exceeds i32",
                    false,
                )
            })?;
            let candidate_content_hash = stable_hash(
                "stock_analysis.selection_candidate_row.v1",
                &(
                    &candidate_id,
                    &run_id,
                    &prepared.event.event_id,
                    &prepared.chain.chain_id,
                    &prepared.mention.security,
                    RELATION_VERSION,
                    FEATURE_VERSION,
                    ordinal,
                    evaluation_market_date,
                ),
            )?;
            let snapshot_payload = FeaturePayload {
                pipeline_version: PIPELINE_VERSION,
                admission_version: &prepared.admission_version,
                event_id: &prepared.event.event_id,
                event_content_hash: &prepared.inbox_content_hash,
                chain: &prepared.chain,
                relation: &prepared.mention,
                features: &prepared.features,
                t0_market_evidence: &prepared.t0_market_evidence,
                magic_tdx_batch_id: &market_batch.batch_id,
                magic_tdx_batch_hash: &market_batch_hash,
            };
            let payload_json = serde_json::to_string(&snapshot_payload).map_err(|error| {
                pipeline_error(
                    "feature_snapshot_serialize_failed",
                    format!("serialize selection feature snapshot: {error}"),
                    false,
                )
            })?;
            let feature_content_hash = stable_hash(
                "stock_analysis.selection_feature_snapshot.v1",
                &snapshot_payload,
            )?;
            let feature_snapshot_id = format!("selection_feature_{feature_content_hash}");

            admitted_candidates_push(
                &mut feature_snapshots,
                CandidateRows {
                    candidate: SelectionCandidateInput {
                        candidate_id: candidate_id.clone(),
                        run_id: run_id.clone(),
                        event_id: prepared.event.event_id.clone(),
                        chain_id: prepared.chain.chain_id.clone(),
                        stock_code: prepared.mention.security.code.clone(),
                        stock_name: prepared.mention.security.name.clone(),
                        relation_version: RELATION_VERSION.to_owned(),
                        feature_version: FEATURE_VERSION.to_owned(),
                        ordinal,
                        content_hash: candidate_content_hash,
                        evaluation_market_date,
                    },
                    feature: FeatureSnapshotInput {
                        feature_snapshot_id,
                        candidate_id,
                        content_hash: feature_content_hash,
                        payload_json,
                        source_batch_id: market_batch.batch_id.clone(),
                        source_batch_hash: market_batch_hash.clone(),
                        observed_at: prepared.observed_at,
                    },
                },
            );
        }
        let (candidates, feature_snapshots): (Vec<_>, Vec<_>) =
            feature_snapshots.into_iter().unzip();
        let staged = SelectionBatchInput {
            run,
            candidates,
            feature_snapshots,
        };
        self.repository.stage_batch(&staged)?;

        if !self.repository.run_is_visible(&run_id)? {
            let committed = self.audit.append(
                SelectionAuditRecord::new(
                    SelectionAuditPhase::Committed,
                    &run_id,
                    &prepared_content_hash,
                    now,
                )
                .with_context(SelectionAuditContext {
                    magic_tdx_batch_id: Some(market_batch.batch_id.clone()),
                    rule_ids: vec!["BR-157".to_owned()],
                    ..SelectionAuditContext::default()
                }),
            )?;
            let receipt_id = format!("selection_visibility_{}", identity_hash("run", &run_id));
            let receipt_content_hash = stable_hash(
                "stock_analysis.selection_visibility.v1",
                &(&receipt_id, &run_id, &prepared_content_hash),
            )?;
            self.repository
                .publish_visibility(&VisibilityReceiptInput {
                    receipt_id,
                    run_id: run_id.clone(),
                    audit_record_hash: committed.record_hash,
                    content_hash: receipt_content_hash,
                    published_at: now,
                })?;
        }

        for completion in &completions {
            self.repository.append_completion(completion)?;
        }
        self.audit.append(
            SelectionAuditRecord::new(
                SelectionAuditPhase::Completed,
                &run_id,
                &prepared_content_hash,
                now,
            )
            .with_context(SelectionAuditContext {
                magic_tdx_batch_id: Some(market_batch.batch_id.clone()),
                rule_ids: vec!["BR-157".to_owned()],
                ..SelectionAuditContext::default()
            }),
        )?;

        Ok(SelectionRunOutcome::Completed(CompletedSelection {
            evaluated_events: completions.len() + rejected_events,
            admitted_candidates: staged.candidates.len(),
            rejected_candidates,
            pending_events: retry_pending,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn append_rejection(
        &self,
        event: &MarketEvent,
        subject_id: &str,
        content_hash: &str,
        reason_codes: Vec<String>,
        rule_ids: Vec<String>,
        retryable: bool,
        chain: Option<&ChainMatch>,
        mention: Option<&DirectMentionEvidence>,
        magic_tdx_batch_id: Option<&str>,
        recorded_at: DateTime<FixedOffset>,
    ) -> Result<(), SelectionPipelineError> {
        self.audit.append(
            SelectionAuditRecord::new(
                SelectionAuditPhase::Rejected,
                subject_id,
                content_hash,
                recorded_at,
            )
            .with_context(SelectionAuditContext {
                event_identity_hash: Some(identity_hash("event", &event.event_id)),
                chain_identity_hash: chain.map(|chain| identity_hash("chain", &chain.chain_id)),
                security_identity_hash: mention
                    .map(|mention| identity_hash("security", &mention.security.code)),
                provider: event_provider(event),
                provider_published_at: event
                    .provider_publication
                    .as_ref()
                    .and_then(|publication| publication.published_at)
                    .map(|timestamp| timestamp.fixed_offset()),
                observed_at: Some(event.occurred_at.fixed_offset()),
                magic_tdx_batch_id: magic_tdx_batch_id.map(str::to_owned),
                reason_codes,
                rule_ids,
                retryable: Some(retryable),
            }),
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct PreparedCandidate {
    event: MarketEvent,
    chain: ChainMatch,
    mention: DirectMentionEvidence,
    features: RawSelectionFeatures,
    t0_market_evidence: T0MarketEvidence,
    admission_version: String,
    observed_at: DateTime<FixedOffset>,
    inbox_content_hash: String,
}

impl PreparedCandidate {
    fn identity_tuple(&self) -> (&str, &str, &str) {
        (
            &self.event.event_id,
            &self.chain.chain_id,
            &self.mention.security.code,
        )
    }
}

#[derive(Serialize)]
struct PreparedIdentity<'a> {
    source_batch_hash: &'a str,
    config_hash: &'a str,
    magic_tdx_batch_hash: &'a str,
    candidates: Vec<(&'a str, &'a str, &'a str)>,
    rejected_candidates: usize,
}

#[derive(Serialize)]
struct FeaturePayload<'a> {
    pipeline_version: &'static str,
    admission_version: &'a str,
    event_id: &'a str,
    event_content_hash: &'a str,
    chain: &'a ChainMatch,
    relation: &'a DirectMentionEvidence,
    features: &'a RawSelectionFeatures,
    t0_market_evidence: &'a T0MarketEvidence,
    magic_tdx_batch_id: &'a str,
    magic_tdx_batch_hash: &'a str,
}

struct CandidateRows {
    candidate: SelectionCandidateInput,
    feature: FeatureSnapshotInput,
}

fn admitted_candidates_push(
    rows: &mut Vec<(SelectionCandidateInput, FeatureSnapshotInput)>,
    values: CandidateRows,
) {
    rows.push((values.candidate, values.feature));
}

fn validate_context(context: &SelectionContext) -> Result<(), SelectionPipelineError> {
    if context.pending_limit == 0 {
        return Err(pipeline_error(
            "pending_limit_invalid",
            "selection pending limit must be greater than zero",
            false,
        ));
    }
    if context.expected_latest_settled_date > context.evaluation_at.date_naive() {
        return Err(pipeline_error(
            "settled_date_future",
            "selection settled date is after evaluation date",
            false,
        ));
    }
    Ok(())
}

fn validate_event_gate(
    event: &MarketEvent,
    evaluation_at: DateTime<Local>,
) -> Result<(), &'static str> {
    if event.full_title.trim().is_empty() || event.subject.trim().is_empty() {
        return Err("event_title_missing");
    }
    if event_provider(event).is_none() {
        return Err("provider_missing");
    }
    if event.stale {
        return Err("source_event_stale");
    }
    let publication = event
        .provider_publication
        .as_ref()
        .ok_or("provider_publication_missing")?;
    let evaluation_date = evaluation_at.date_naive();
    if publication.published_on < evaluation_date {
        return Err("provider_publication_stale");
    }
    if publication.published_on > evaluation_date {
        return Err("provider_publication_future");
    }
    if publication
        .published_at
        .is_some_and(|published_at| published_at > evaluation_at)
    {
        return Err("provider_publication_future");
    }
    Ok(())
}

fn event_provider(event: &MarketEvent) -> Option<String> {
    event
        .provenance
        .iter()
        .map(|source| source.provider.trim())
        .find(|provider| !provider.is_empty())
        .map(str::to_owned)
}

fn event_text(event: &MarketEvent) -> String {
    let mut text = event.full_title.clone();
    text.push('\n');
    text.push_str(&event.subject);
    if let Some(object) = event.object.as_deref() {
        text.push('\n');
        text.push_str(object);
    }
    text
}

fn publication_sort_key(event: &MarketEvent) -> (NaiveDate, Option<DateTime<FixedOffset>>) {
    event
        .provider_publication
        .as_ref()
        .map(|publication| {
            (
                publication.published_on,
                publication
                    .published_at
                    .map(|timestamp| timestamp.fixed_offset()),
            )
        })
        .unwrap_or((NaiveDate::MIN, None))
}

fn evaluation_window(window: SelectionMarketWindow) -> SelectionEvaluationWindow {
    match window {
        SelectionMarketWindow::Intraday => SelectionEvaluationWindow::Intraday,
        SelectionMarketWindow::PostClose => SelectionEvaluationWindow::PostClose,
    }
}

fn features_for_record(
    window: SelectionMarketWindow,
    record: &crate::selection::magic_tdx::SelectionMarketRecord,
) -> Result<RawSelectionFeatures, crate::selection::features::FeatureError> {
    let mut features = compute_daily_features(&record.daily_bars)?;
    if window == SelectionMarketWindow::Intraday {
        let evidence = intraday_volume_evidence(&record.five_minute_bars, record.observed_at)?;
        features = features.with_intraday_volume_pace(&evidence)?;
        if let Some(quote) = record.quote.as_ref() {
            if let Some(ma5) = features.ma5 {
                features.price_vs_ma5 = Some(quote.price / ma5 - 1.0);
            }
            if let Some(ma10) = features.ma10 {
                features.price_vs_ma10 = Some(quote.price / ma10 - 1.0);
            }
            if let Some(ma20) = features.ma20 {
                features.price_vs_ma20 = Some(quote.price / ma20 - 1.0);
            }
            if record.daily_bars.len() >= 5 {
                let base = record.daily_bars[record.daily_bars.len() - 5].close;
                features.five_day_return = Some(quote.price / base - 1.0);
            }
        }
    }
    Ok(features)
}

fn t0_market_evidence(
    window: SelectionMarketWindow,
    record: &crate::selection::magic_tdx::SelectionMarketRecord,
) -> Result<T0MarketEvidence, crate::selection::features::FeatureError> {
    let count = record.daily_bars.len();
    if count < 21 {
        return Err(crate::selection::features::FeatureError::new(
            "t0_market_history_insufficient",
            "T0 market evidence requires twenty-one settled daily bars",
        ));
    }
    let latest = &record.daily_bars[count - 1];
    let prior_5d_average_volume = record.daily_bars[count - 6..count - 1]
        .iter()
        .map(|bar| bar.volume)
        .sum::<f64>()
        / 5.0;
    let prior_20d_average_volume = record.daily_bars[count - 21..count - 1]
        .iter()
        .map(|bar| bar.volume)
        .sum::<f64>()
        / 20.0;
    let (evaluation_price, observed_volume) = match window {
        SelectionMarketWindow::Intraday => {
            let quote = record.quote.as_ref().ok_or_else(|| {
                crate::selection::features::FeatureError::new(
                    "t0_quote_missing",
                    "intraday T0 market evidence requires a validated quote",
                )
            })?;
            (quote.price, quote.volume)
        }
        SelectionMarketWindow::PostClose => (latest.close, latest.volume),
    };
    for (field, value, strictly_positive) in [
        ("evaluation_price", evaluation_price, true),
        ("observed_volume", observed_volume, true),
        ("latest_settled_close", latest.close, true),
        ("latest_settled_volume", latest.volume, true),
        ("prior_5d_average_volume", prior_5d_average_volume, true),
        ("prior_20d_average_volume", prior_20d_average_volume, true),
    ] {
        if !value.is_finite() || (strictly_positive && value <= 0.0) || value < 0.0 {
            return Err(crate::selection::features::FeatureError::new(
                "t0_market_evidence_invalid",
                format!("{field} is invalid: {value}"),
            ));
        }
    }
    Ok(T0MarketEvidence {
        evaluation_price,
        observed_volume,
        latest_settled_market_date: latest.market_date,
        latest_settled_close: latest.close,
        latest_settled_volume: latest.volume,
        prior_5d_average_volume,
        prior_20d_average_volume,
    })
}

fn intraday_volume_evidence(
    bars: &[SelectionFiveMinuteBar],
    observed_at: DateTime<Local>,
) -> Result<IntradayVolumeEvidence, crate::selection::features::FeatureError> {
    let current_date = observed_at.date_naive();
    let mut by_date: BTreeMap<NaiveDate, Vec<&SelectionFiveMinuteBar>> = BTreeMap::new();
    for bar in bars {
        by_date
            .entry(bar.ended_at.date_naive())
            .or_default()
            .push(bar);
    }
    let current = by_date.get(&current_date).ok_or_else(|| {
        crate::selection::features::FeatureError::new(
            "intraday_volume_current_session_missing",
            "current-session completed five-minute volume is missing",
        )
    })?;
    let completed_through = current
        .iter()
        .map(|bar| bar.ended_at.time())
        .max()
        .unwrap_or(NaiveTime::MIN);
    let cumulative_volume = current
        .iter()
        .filter(|bar| bar.ended_at.time() <= completed_through)
        .map(|bar| bar.volume)
        .sum();
    let historical_same_slot_volumes = by_date
        .into_iter()
        .filter(|(date, _)| *date < current_date)
        .map(|(_, bars)| {
            bars.into_iter()
                .filter(|bar| bar.ended_at.time() <= completed_through)
                .map(|bar| bar.volume)
                .sum()
        })
        .collect();
    Ok(IntradayVolumeEvidence {
        cumulative_volume,
        historical_same_slot_volumes,
    })
}

fn event_content_hash(event: &MarketEvent) -> Result<String, SelectionPipelineError> {
    stable_hash("stock_analysis.selection_event.v1", event)
}

fn market_batch_hash(batch: &SelectionMarketBatch) -> Result<String, SelectionPipelineError> {
    #[derive(Serialize)]
    struct MarketEvidence<'a> {
        batch_id: &'a str,
        observed_at: DateTime<FixedOffset>,
        master_batch_id: &'a str,
        master_observed_at: DateTime<FixedOffset>,
        identities: &'a [crate::selection::model::SecurityIdentity],
        event_mentions: &'a BTreeMap<String, Vec<DirectMentionEvidence>>,
        records: &'a [crate::selection::magic_tdx::SelectionMarketRecord],
        rejections: &'a [crate::selection::magic_tdx::SelectionSourceRejection],
    }
    stable_hash(
        "stock_analysis.selection_magic_tdx_batch.v1",
        &MarketEvidence {
            batch_id: &batch.batch_id,
            observed_at: batch.observed_at.fixed_offset(),
            master_batch_id: &batch.master.batch_id,
            master_observed_at: batch.master.observed_at.fixed_offset(),
            identities: batch.master.identities(),
            event_mentions: &batch.event_mentions,
            records: &batch.records,
            rejections: &batch.rejections,
        },
    )
}

fn completed_completion(
    event_id: &str,
    completed_at: DateTime<FixedOffset>,
) -> Result<EventCompletion, SelectionPipelineError> {
    completion(event_id, CompletionStatus::Completed, None, completed_at)
}

fn rejected_completion(
    event_id: &str,
    reason_code: &str,
    completed_at: DateTime<FixedOffset>,
) -> Result<EventCompletion, SelectionPipelineError> {
    completion(
        event_id,
        CompletionStatus::Rejected,
        Some(reason_code),
        completed_at,
    )
}

fn completion(
    event_id: &str,
    status: CompletionStatus,
    reason_code: Option<&str>,
    completed_at: DateTime<FixedOffset>,
) -> Result<EventCompletion, SelectionPipelineError> {
    let content_hash = stable_hash(
        "stock_analysis.selection_completion.v1",
        &(event_id, status_label(status), reason_code),
    )?;
    Ok(EventCompletion {
        completion_id: format!("selection_completion_{content_hash}"),
        event_id: event_id.to_owned(),
        content_hash,
        status,
        reason_code: reason_code.map(str::to_owned),
        completed_at,
    })
}

fn status_label(status: CompletionStatus) -> &'static str {
    match status {
        CompletionStatus::Completed => "completed",
        CompletionStatus::Rejected => "rejected",
    }
}

fn ticket_subject(
    event: &MarketEvent,
    chain: &ChainMatch,
    mention: &DirectMentionEvidence,
) -> String {
    format!(
        "{}:{}:{}",
        event.event_id, chain.chain_id, mention.security.code
    )
}

fn identity_hash(domain: &str, identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.selection_identity.v1\0");
    hasher.update(domain.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity.as_bytes());
    hex::encode(hasher.finalize())
}

fn stable_hash<T: Serialize>(domain: &str, value: &T) -> Result<String, SelectionPipelineError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        pipeline_error(
            "identity_serialize_failed",
            format!("serialize {domain} identity: {error}"),
            false,
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update(b"\0");
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn pipeline_error(
    code: impl Into<String>,
    message: impl Into<String>,
    retryable: bool,
) -> SelectionPipelineError {
    SelectionPipelineError {
        code: code.into(),
        message: message.into(),
        retryable,
    }
}

struct ProductionMarketPort;

#[async_trait]
impl SelectionMarketPort for ProductionMarketPort {
    async fn fetch(
        &self,
        request: SelectionMarketRequest,
    ) -> Result<SelectionMarketBatch, SelectionPipelineError> {
        fetch_selection_market_batch(request)
            .await
            .map_err(source_pipeline_error)
    }
}

fn source_pipeline_error(error: SelectionSourceError) -> SelectionPipelineError {
    pipeline_error(error.code(), error.to_string(), error.retryable())
}

struct ProductionConfigPort;

impl SelectionConfigPort for ProductionConfigPort {
    fn snapshot(&self) -> Result<ChainConfigSnapshot, SelectionPipelineError> {
        let rules = crate::config::get_chain_rules().ok_or_else(|| {
            pipeline_error(
                "chain_config_unavailable",
                "validated chain configuration is unavailable",
                true,
            )
        })?;
        ChainConfigSnapshot::from_rules(rules.as_slice())
            .map_err(|error| pipeline_error(error.reason_code(), error.to_string(), false))
    }
}

struct ProductionRepositoryPort;

impl ProductionRepositoryPort {
    fn with_repository<T>(
        &self,
        operation: impl FnOnce(
            &mut crate::database::selection::SelectionRepository<'_>,
        ) -> Result<T, crate::database::selection::SelectionStoreError>,
    ) -> Result<T, SelectionPipelineError> {
        let database = crate::database::DatabaseManager::try_get().ok_or_else(|| {
            pipeline_error(
                "selection_database_unavailable",
                "database manager is not initialized",
                true,
            )
        })?;
        let mut connection = database.get_conn().map_err(|error| {
            pipeline_error(
                "selection_database_unavailable",
                format!("acquire selection database connection: {error}"),
                true,
            )
        })?;
        let mut repository = crate::database::selection::SelectionRepository::new(&mut connection);
        operation(&mut repository)
            .map_err(|error| pipeline_error("selection_database_failure", error.to_string(), true))
    }
}

impl SelectionRepositoryPort for ProductionRepositoryPort {
    fn ingest_event(&self, event: &InboxEvent) -> Result<(), SelectionPipelineError> {
        self.with_repository(|repository| repository.ingest_event(event))
            .map(|_| ())
    }

    fn pending_events(&self, limit: usize) -> Result<Vec<InboxEvent>, SelectionPipelineError> {
        self.with_repository(|repository| repository.pending_events(limit))
    }

    fn stage_batch(&self, batch: &SelectionBatchInput) -> Result<(), SelectionPipelineError> {
        self.with_repository(|repository| repository.stage_batch(batch))
            .map(|_| ())
    }

    fn run_is_visible(&self, run_id: &str) -> Result<bool, SelectionPipelineError> {
        self.with_repository(|repository| repository.run_is_visible(run_id))
    }

    fn publish_visibility(
        &self,
        receipt: &VisibilityReceiptInput,
    ) -> Result<(), SelectionPipelineError> {
        self.with_repository(|repository| repository.publish_visibility(receipt))
            .map(|_| ())
    }

    fn append_completion(
        &self,
        completion: &EventCompletion,
    ) -> Result<(), SelectionPipelineError> {
        self.with_repository(|repository| repository.append_completion(completion))
            .map(|_| ())
    }
}

struct ProductionAuditPort {
    writer: SelectionAuditWriter,
}

impl SelectionAuditPort for ProductionAuditPort {
    fn append(
        &self,
        record: SelectionAuditRecord,
    ) -> Result<AuditAppendReceipt, SelectionPipelineError> {
        self.writer
            .append(record)
            .map_err(|error| pipeline_error(error.code(), error.to_string(), true))
    }
}

pub async fn evaluate_market_events(
    batch: SelectionEventBatch,
    context: SelectionContext,
) -> SelectionRunOutcome {
    let pipeline = SelectionPipeline::new(
        Arc::new(ProductionRepositoryPort),
        Arc::new(ProductionMarketPort),
        Arc::new(ProductionConfigPort),
        Arc::new(ProductionAuditPort {
            writer: SelectionAuditWriter::for_environment(
                "data/audit",
                SelectionAuditEnvironment::Production,
            ),
        }),
    );
    pipeline.evaluate(batch, context).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChainRuleConfig;
    use crate::news::aggregator::{FeedAttemptStatus, SourceKind};
    use crate::selection::model::{
        DirectMentionKind, SecurityIdentity, SecurityMarket, SecurityMasterSnapshot,
    };
    use crate::selection::quality::{PriceAdjustment, SelectionBar};
    use crate::signal::market_event::{Direction, EventType, ProviderPublication, SourceRef};
    use chrono::TimeZone;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepositoryState {
        inbox: BTreeMap<String, InboxEvent>,
        completions: BTreeMap<String, EventCompletion>,
        staged: Vec<SelectionBatchInput>,
        visible: BTreeSet<String>,
    }

    #[derive(Default)]
    struct FakeRepository {
        state: Mutex<FakeRepositoryState>,
    }

    impl SelectionRepositoryPort for FakeRepository {
        fn ingest_event(&self, event: &InboxEvent) -> Result<(), SelectionPipelineError> {
            let mut state = self.state.lock().expect("repository state");
            if let Some(existing) = state.inbox.get(&event.event_id) {
                if existing.content_hash != event.content_hash {
                    return Err(pipeline_error(
                        "event_conflict",
                        "event content conflict",
                        false,
                    ));
                }
                return Ok(());
            }
            state.inbox.insert(event.event_id.clone(), event.clone());
            Ok(())
        }

        fn pending_events(&self, limit: usize) -> Result<Vec<InboxEvent>, SelectionPipelineError> {
            let state = self.state.lock().expect("repository state");
            Ok(state
                .inbox
                .values()
                .filter(|event| !state.completions.contains_key(&event.event_id))
                .take(limit)
                .cloned()
                .collect())
        }

        fn stage_batch(&self, batch: &SelectionBatchInput) -> Result<(), SelectionPipelineError> {
            let mut state = self.state.lock().expect("repository state");
            if !state
                .staged
                .iter()
                .any(|existing| existing.run.run_id == batch.run.run_id)
            {
                state.staged.push(batch.clone());
            }
            Ok(())
        }

        fn run_is_visible(&self, run_id: &str) -> Result<bool, SelectionPipelineError> {
            Ok(self
                .state
                .lock()
                .expect("repository state")
                .visible
                .contains(run_id))
        }

        fn publish_visibility(
            &self,
            receipt: &VisibilityReceiptInput,
        ) -> Result<(), SelectionPipelineError> {
            self.state
                .lock()
                .expect("repository state")
                .visible
                .insert(receipt.run_id.clone());
            Ok(())
        }

        fn append_completion(
            &self,
            completion: &EventCompletion,
        ) -> Result<(), SelectionPipelineError> {
            self.state
                .lock()
                .expect("repository state")
                .completions
                .entry(completion.event_id.clone())
                .or_insert_with(|| completion.clone());
            Ok(())
        }
    }

    struct FakeMarket {
        result: Mutex<Option<Result<SelectionMarketBatch, SelectionPipelineError>>>,
    }

    #[async_trait]
    impl SelectionMarketPort for FakeMarket {
        async fn fetch(
            &self,
            _request: SelectionMarketRequest,
        ) -> Result<SelectionMarketBatch, SelectionPipelineError> {
            self.result
                .lock()
                .expect("market result")
                .take()
                .expect("one market call")
        }
    }

    struct FakeConfig(ChainConfigSnapshot);

    impl SelectionConfigPort for FakeConfig {
        fn snapshot(&self) -> Result<ChainConfigSnapshot, SelectionPipelineError> {
            Ok(self.0.clone())
        }
    }

    struct FakeAudit {
        fail_phase: Option<SelectionAuditPhase>,
        phases: Mutex<Vec<SelectionAuditPhase>>,
    }

    impl SelectionAuditPort for FakeAudit {
        fn append(
            &self,
            record: SelectionAuditRecord,
        ) -> Result<AuditAppendReceipt, SelectionPipelineError> {
            self.phases.lock().expect("audit phases").push(record.phase);
            if self.fail_phase == Some(record.phase) {
                return Err(pipeline_error(
                    "audit_injected_failure",
                    "injected audit failure",
                    true,
                ));
            }
            Ok(AuditAppendReceipt {
                record_hash: identity_hash("audit", &format!("{:?}", record.phase)),
                previous_hash: None,
            })
        }
    }

    struct Harness {
        pipeline: SelectionPipeline,
        repository: Arc<FakeRepository>,
        audit: Arc<FakeAudit>,
    }

    fn harness(
        market: Result<SelectionMarketBatch, SelectionPipelineError>,
        fail_phase: Option<SelectionAuditPhase>,
    ) -> Harness {
        let repository = Arc::new(FakeRepository::default());
        let audit = Arc::new(FakeAudit {
            fail_phase,
            phases: Mutex::new(Vec::new()),
        });
        let pipeline = SelectionPipeline::new(
            repository.clone(),
            Arc::new(FakeMarket {
                result: Mutex::new(Some(market)),
            }),
            Arc::new(FakeConfig(test_config())),
            audit.clone(),
        );
        Harness {
            pipeline,
            repository,
            audit,
        }
    }

    #[tokio::test]
    async fn source_failure_never_becomes_verified_empty() {
        let harness = harness(Err(pipeline_error("unused", "must not fetch", true)), None);
        let outcome = harness
            .pipeline
            .evaluate(incomplete_empty_batch(), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::Unavailable(UnavailableSelection {
                reason_code,
                ..
            }) if reason_code == "source_batch_incomplete"
        ));
    }

    #[tokio::test]
    async fn magic_tdx_failure_keeps_event_pending_for_retry() {
        let harness = harness(
            Err(pipeline_error(
                "magic_tdx_transport",
                "transport unavailable",
                true,
            )),
            None,
        );
        let outcome = harness
            .pipeline
            .evaluate(complete_batch(test_event()), test_context())
            .await;

        assert!(matches!(outcome, SelectionRunOutcome::Unavailable(_)));
        let state = harness.repository.state.lock().expect("repository state");
        assert_eq!(state.inbox.len(), 1);
        assert!(state.completions.is_empty());
    }

    #[tokio::test]
    async fn candidate_is_invisible_until_committed_audit_and_receipt() {
        let event = test_event();
        let harness = harness(
            Ok(test_market_batch(&event, passing_bars())),
            Some(SelectionAuditPhase::Committed),
        );
        let outcome = harness
            .pipeline
            .evaluate(complete_batch(event), test_context())
            .await;

        assert!(matches!(outcome, SelectionRunOutcome::Unavailable(_)));
        let state = harness.repository.state.lock().expect("repository state");
        assert_eq!(state.staged.len(), 1);
        assert!(state.visible.is_empty());
        assert!(state.completions.is_empty());
    }

    #[tokio::test]
    async fn hard_rejected_ticket_never_enters_formal_candidate_or_visibility() {
        let event = test_event();
        let harness = harness(Ok(test_market_batch(&event, weak_bars())), None);
        let outcome = harness
            .pipeline
            .evaluate(complete_batch(event), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::Completed(CompletedSelection {
                admitted_candidates: 0,
                rejected_candidates: 1,
                ..
            })
        ));
        let state = harness.repository.state.lock().expect("repository state");
        assert!(state.staged.is_empty());
        assert!(state.visible.is_empty());
        assert_eq!(state.completions.len(), 1);
        assert!(harness
            .audit
            .phases
            .lock()
            .expect("audit phases")
            .contains(&SelectionAuditPhase::Rejected));
    }

    #[tokio::test]
    async fn admitted_candidate_becomes_visible_only_after_commit_receipt() {
        let event = test_event();
        let harness = harness(Ok(test_market_batch(&event, passing_bars())), None);
        let outcome = harness
            .pipeline
            .evaluate(complete_batch(event), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::Completed(CompletedSelection {
                admitted_candidates: 1,
                rejected_candidates: 0,
                ..
            })
        ));
        let state = harness.repository.state.lock().expect("repository state");
        assert_eq!(state.staged.len(), 1);
        assert_eq!(state.staged[0].candidates.len(), 1);
        let snapshot: serde_json::Value =
            serde_json::from_str(&state.staged[0].feature_snapshots[0].payload_json)
                .expect("feature snapshot JSON");
        let t0 = snapshot
            .get("t0_market_evidence")
            .expect("immutable T0 market evidence");
        assert!(t0.get("evaluation_price").is_some());
        assert!(t0.get("observed_volume").is_some());
        assert!(t0.get("prior_5d_average_volume").is_some());
        assert!(t0.get("prior_20d_average_volume").is_some());
        assert_eq!(state.visible.len(), 1);
        assert_eq!(state.completions.len(), 1);
    }

    #[tokio::test]
    async fn stale_event_is_terminally_rejected_before_magic_tdx() {
        let mut event = test_event();
        event.stale = true;
        let harness = harness(Err(pipeline_error("unused", "must not fetch", true)), None);
        let outcome = harness
            .pipeline
            .evaluate(complete_batch(event), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::Completed(CompletedSelection {
                admitted_candidates: 0,
                ..
            })
        ));
        let state = harness.repository.state.lock().expect("repository state");
        assert!(state.staged.is_empty());
        assert_eq!(state.completions.len(), 1);
        assert_eq!(
            state
                .completions
                .values()
                .next()
                .expect("stale completion")
                .reason_code
                .as_deref(),
            Some("source_event_stale")
        );
    }

    #[tokio::test]
    async fn same_security_in_two_events_keeps_two_event_scoped_candidates() {
        let first = test_event();
        let mut second = test_event();
        second.event_id = "TEST_CODE_event_000002".to_owned();
        let market = test_market_batch_for_events(&[&first, &second], passing_bars());
        let harness = harness(Ok(market), None);
        let outcome = harness
            .pipeline
            .evaluate(selection_batch(vec![first, second], true), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::Completed(CompletedSelection {
                admitted_candidates: 2,
                ..
            })
        ));
        let state = harness.repository.state.lock().expect("repository state");
        assert_eq!(state.staged[0].candidates.len(), 2);
        assert_ne!(
            state.staged[0].candidates[0].candidate_id,
            state.staged[0].candidates[1].candidate_id
        );
    }

    #[tokio::test]
    async fn replay_after_completion_is_idempotent_and_does_not_restage() {
        let event = test_event();
        let market = test_market_batch(&event, passing_bars());
        let repository = Arc::new(FakeRepository::default());
        let audit = Arc::new(FakeAudit {
            fail_phase: None,
            phases: Mutex::new(Vec::new()),
        });
        let pipeline = SelectionPipeline::new(
            repository.clone(),
            Arc::new(FakeMarket {
                result: Mutex::new(Some(Ok(market))),
            }),
            Arc::new(FakeConfig(test_config())),
            audit,
        );
        let batch = complete_batch(event);

        let first = pipeline.evaluate(batch.clone(), test_context()).await;
        let second = pipeline.evaluate(batch, test_context()).await;

        assert!(matches!(first, SelectionRunOutcome::Completed(_)));
        assert!(matches!(second, SelectionRunOutcome::VerifiedEmpty(_)));
        let state = repository.state.lock().expect("repository state");
        assert_eq!(state.staged.len(), 1);
        assert_eq!(state.visible.len(), 1);
        assert_eq!(state.completions.len(), 1);
    }

    #[tokio::test]
    async fn retryable_failure_of_one_ticket_does_not_hide_valid_ticket() {
        let event = test_event();
        let harness = harness(
            Ok(test_market_batch_with_isolated_failure(
                &event,
                passing_bars(),
            )),
            None,
        );
        let outcome = harness
            .pipeline
            .evaluate(complete_batch(event), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::Completed(CompletedSelection {
                admitted_candidates: 1,
                rejected_candidates: 1,
                pending_events: 0,
                ..
            })
        ));
        let state = harness.repository.state.lock().expect("repository state");
        assert_eq!(state.staged[0].candidates.len(), 1);
        assert_eq!(state.staged[0].candidates[0].stock_code, "TEST_CODE_000001");
        assert_eq!(state.completions.len(), 1);
    }

    #[tokio::test]
    async fn complete_source_batch_with_no_events_is_verified_empty() {
        let harness = harness(Err(pipeline_error("unused", "must not fetch", true)), None);
        let outcome = harness
            .pipeline
            .evaluate(complete_empty_batch(), test_context())
            .await;

        assert!(matches!(
            outcome,
            SelectionRunOutcome::VerifiedEmpty(VerifiedEmptySelection {
                evaluated_events: 0,
                ..
            })
        ));
    }

    fn test_context() -> SelectionContext {
        SelectionContext::new(
            SelectionMarketWindow::PostClose,
            Local
                .with_ymd_and_hms(2026, 7, 23, 16, 0, 0)
                .single()
                .expect("test evaluation time"),
            NaiveDate::from_ymd_opt(2026, 7, 23).expect("test market date"),
        )
    }

    fn test_event() -> MarketEvent {
        let occurred_at = Local
            .with_ymd_and_hms(2026, 7, 23, 9, 30, 0)
            .single()
            .expect("test event time");
        let mut event = MarketEvent::new_with_title(
            EventType::Policy,
            "芯片政策".to_owned(),
            "芯片政策明确支持测试股份 TEST_CODE_000001".to_owned(),
            None,
            Direction::Bull,
            80,
            90,
        );
        event.event_id = "TEST_CODE_event_000001".to_owned();
        event.occurred_at = occurred_at;
        event.provider_publication = Some(ProviderPublication {
            published_on: occurred_at.date_naive(),
            published_at: Some(occurred_at),
        });
        event.provenance = vec![SourceRef {
            provider: "TEST_CODE_provider".to_owned(),
            url: None,
            fetched_at: occurred_at,
        }];
        event
    }

    fn complete_batch(event: MarketEvent) -> SelectionEventBatch {
        selection_batch(vec![event], true)
    }

    fn complete_empty_batch() -> SelectionEventBatch {
        selection_batch(Vec::new(), true)
    }

    fn incomplete_empty_batch() -> SelectionEventBatch {
        selection_batch(Vec::new(), false)
    }

    fn selection_batch(events: Vec<MarketEvent>, complete: bool) -> SelectionEventBatch {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 10, 0, 0)
            .single()
            .expect("test observed time");
        SelectionEventBatch::try_from(NewsAggregationBatch {
            events,
            source_attempts: vec![FeedAttempt {
                feed_name: "TEST_CODE_feed".to_owned(),
                source_kind: SourceKind::Policy.label().to_owned(),
                status: if complete {
                    FeedAttemptStatus::Succeeded { event_count: 1 }
                } else {
                    FeedAttemptStatus::Failed {
                        reason_code: "TEST_CODE_failure".to_owned(),
                        message: "test source unavailable".to_owned(),
                    }
                },
            }],
            observed_at,
        })
        .expect("valid test selection batch")
    }

    fn test_config() -> ChainConfigSnapshot {
        ChainConfigSnapshot::from_rules(&[ChainRuleConfig {
            chain: "TEST_CODE_chain_chip".to_owned(),
            logic: "direct keyword".to_owned(),
            board_keyword: String::new(),
            keywords: vec!["芯片".to_owned()],
            enabled: true,
            priority: 100,
            category: "test".to_owned(),
            generic: false,
        }])
        .expect("valid test chain config")
    }

    fn test_market_batch(event: &MarketEvent, bars: Vec<SelectionBar>) -> SelectionMarketBatch {
        test_market_batch_for_events(&[event], bars)
    }

    fn test_market_batch_for_events(
        events: &[&MarketEvent],
        bars: Vec<SelectionBar>,
    ) -> SelectionMarketBatch {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 16, 0, 0)
            .single()
            .expect("test market time");
        let security = SecurityIdentity {
            code: "TEST_CODE_000001".to_owned(),
            name: "测试股份".to_owned(),
            market: SecurityMarket::Shanghai,
        };
        let master = SecurityMasterSnapshot::new(
            vec![security.clone()],
            "TEST_CODE_master_batch".to_owned(),
            observed_at,
        )
        .expect("valid test master");
        SelectionMarketBatch {
            master,
            event_mentions: events
                .iter()
                .map(|event| {
                    (
                        event.event_id.clone(),
                        vec![DirectMentionEvidence {
                            security: security.clone(),
                            matched_by: DirectMentionKind::ExactSecurityCode,
                            master_batch_id: "TEST_CODE_master_batch".to_owned(),
                        }],
                    )
                })
                .collect(),
            records: vec![crate::selection::magic_tdx::SelectionMarketRecord {
                security,
                daily_bars: bars,
                quote: None,
                five_minute_bars: Vec::new(),
                observed_at,
            }],
            rejections: Vec::new(),
            observed_at,
            batch_id: "TEST_CODE_magic_tdx_batch".to_owned(),
        }
    }

    fn test_market_batch_with_isolated_failure(
        event: &MarketEvent,
        bars: Vec<SelectionBar>,
    ) -> SelectionMarketBatch {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 16, 0, 0)
            .single()
            .expect("test market time");
        let admitted_security = SecurityIdentity {
            code: "TEST_CODE_000001".to_owned(),
            name: "测试股份".to_owned(),
            market: SecurityMarket::Shanghai,
        };
        let unavailable_security = SecurityIdentity {
            code: "TEST_CODE_000002".to_owned(),
            name: "测试科技".to_owned(),
            market: SecurityMarket::Shenzhen,
        };
        let master = SecurityMasterSnapshot::new(
            vec![admitted_security.clone(), unavailable_security.clone()],
            "TEST_CODE_master_batch".to_owned(),
            observed_at,
        )
        .expect("valid test master");
        SelectionMarketBatch {
            master,
            event_mentions: BTreeMap::from([(
                event.event_id.clone(),
                vec![
                    DirectMentionEvidence {
                        security: admitted_security.clone(),
                        matched_by: DirectMentionKind::ExactSecurityCode,
                        master_batch_id: "TEST_CODE_master_batch".to_owned(),
                    },
                    DirectMentionEvidence {
                        security: unavailable_security,
                        matched_by: DirectMentionKind::ExactSecurityName,
                        master_batch_id: "TEST_CODE_master_batch".to_owned(),
                    },
                ],
            )]),
            records: vec![crate::selection::magic_tdx::SelectionMarketRecord {
                security: admitted_security,
                daily_bars: bars,
                quote: None,
                five_minute_bars: Vec::new(),
                observed_at,
            }],
            rejections: vec![crate::selection::magic_tdx::SelectionSourceRejection {
                event_id: Some(event.event_id.clone()),
                security_code: Some("TEST_CODE_000002".to_owned()),
                reason_code: "magic_tdx_quote_unavailable".to_owned(),
                retryable: true,
            }],
            observed_at,
            batch_id: "TEST_CODE_magic_tdx_batch".to_owned(),
        }
    }

    fn passing_bars() -> Vec<SelectionBar> {
        bars_with(
            |index| 10.0 + index as f64 * 0.05,
            |index| {
                if index == 20 {
                    200.0
                } else {
                    100.0
                }
            },
        )
    }

    fn weak_bars() -> Vec<SelectionBar> {
        bars_with(|index| 12.0 - index as f64 * 0.05, |_| 100.0)
    }

    fn bars_with(close: impl Fn(usize) -> f64, volume: impl Fn(usize) -> f64) -> Vec<SelectionBar> {
        let mut date = NaiveDate::from_ymd_opt(2026, 6, 24).expect("test bar start");
        (0..21)
            .map(|index| {
                let close = close(index);
                let bar = SelectionBar {
                    code: "TEST_CODE_000001".to_owned(),
                    market_date: date,
                    open: close,
                    high: close,
                    low: close,
                    close,
                    volume: volume(index),
                    amount: close * volume(index),
                    settled: true,
                    adjustment: PriceAdjustment::Unadjusted,
                    reference_previous_close: None,
                };
                date = crate::calendar::next_trading_day(date);
                bar
            })
            .collect()
    }
}
