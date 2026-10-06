//! BR-172 immutable NewsAI assessment and governed-delivery audit.
//!
//! The public seam accepts one already-validated assessment projection and
//! appends it atomically with a SHA-256 chain link. A separate append-only
//! state chain owns exact delivery reservation, sink evidence and prediction
//! linkage; this module performs no physical sink or trading side effect.

#[path = "news_ai/critical_strength.rs"]
mod critical_strength;
#[path = "news_ai/global_critical.rs"]
mod global_critical;
#[cfg(test)]
pub(crate) use global_critical::tests::fixture as global_test_fixture;

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use thiserror::Error;

use super::DatabaseManager;
use crate::monitor::news_ai::{AdmittedNewsFact, NewsAiIdentityV3, NEWS_AI_V3_STORAGE_PREFIX};

const SCHEMA_VERSION: i32 = 1;
const CHAIN_GENESIS: &str = "BR172_NEWS_AI_ASSESSMENT_GENESIS_V1";
const SOURCE_IDENTITY_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_SOURCE_IDENTITY_V1\0";
const CONTENT_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_ASSESSMENT_CONTENT_V1\0";
const CHAIN_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_ASSESSMENT_CHAIN_V1\0";
const DELIVERY_CHAIN_GENESIS: &str = "BR172_NEWS_AI_DELIVERY_GENESIS_V1";
const DELIVERY_ID_DOMAIN: &[u8] = b"BR172_NEWS_AI_DELIVERY_EVENT_ID_V1\0";
const DELIVERY_CONTENT_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_DELIVERY_CONTENT_V1\0";
const DELIVERY_CHAIN_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_DELIVERY_CHAIN_V1\0";
const PREDICTION_LINK_ID_DOMAIN: &[u8] = b"BR172_NEWS_AI_PREDICTION_LINK_ID_V1\0";
const RECOVERY_SNAPSHOT_SCHEMA_VERSION: i32 = 1;

pub const NEWS_AI_ASSESSMENT_MIN_RETENTION_YEARS: i32 = 5;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_ai_assessment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    assessment_id TEXT NOT NULL UNIQUE CHECK (length(trim(assessment_id)) > 0),
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    source_identity_sha256 TEXT NOT NULL CHECK (length(source_identity_sha256) = 64),
    impact TEXT NOT NULL CHECK (
        impact IN ('major_negative', 'negative', 'neutral', 'positive', 'major_positive')
    ),
    confidence INTEGER NOT NULL CHECK (confidence BETWEEN 0 AND 100),
    uncertainty TEXT NOT NULL CHECK (length(trim(uncertainty)) > 0),
    core_logic TEXT NOT NULL CHECK (length(trim(core_logic)) > 0),
    input_evidence_sha256 TEXT NOT NULL CHECK (length(input_evidence_sha256) = 64),
    normalized_prompt_sha256 TEXT NOT NULL CHECK (length(normalized_prompt_sha256) = 64),
    source_provider TEXT NOT NULL CHECK (length(trim(source_provider)) > 0),
    source_batch_id TEXT NOT NULL CHECK (length(trim(source_batch_id)) > 0),
    source_item_id TEXT NOT NULL CHECK (length(trim(source_item_id)) > 0),
    analysis_version TEXT NOT NULL CHECK (length(trim(analysis_version)) > 0),
    target_code TEXT NOT NULL CHECK (length(trim(target_code)) > 0),
    model_provider TEXT NOT NULL CHECK (length(trim(model_provider)) > 0),
    model TEXT NOT NULL CHECK (length(trim(model)) > 0),
    model_upstream_request_id TEXT,
    model_upstream_response_id TEXT NOT NULL CHECK (
        length(trim(model_upstream_response_id)) > 0
    ),
    model_system_sha256 TEXT NOT NULL CHECK (length(model_system_sha256) = 64),
    model_user_sha256 TEXT NOT NULL CHECK (length(model_user_sha256) = 64),
    model_response_sha256 TEXT NOT NULL CHECK (length(model_response_sha256) = 64),
    model_started_at TEXT NOT NULL,
    model_completed_at TEXT NOT NULL,
    minimum_retention_years INTEGER NOT NULL CHECK (minimum_retention_years >= 5),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_news_ai_assessment_source
    ON news_ai_assessment (
        source_provider, source_batch_id, source_item_id, target_code, analysis_version
    );

CREATE TABLE IF NOT EXISTS news_ai_assessment_chain (
    assessment_row_id INTEGER PRIMARY KEY,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK (length(record_hash) = 64),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(assessment_row_id) REFERENCES news_ai_assessment(id)
);

CREATE TRIGGER IF NOT EXISTS trg_news_ai_assessment_no_update
BEFORE UPDATE ON news_ai_assessment
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-172 NewsAI assessment is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_assessment_no_delete
BEFORE DELETE ON news_ai_assessment
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-172 NewsAI assessment is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_assessment_chain_no_update
BEFORE UPDATE ON news_ai_assessment_chain
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-172 NewsAI assessment hash chain is immutable and retained for at least five years'
    );
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_assessment_chain_no_delete
BEFORE DELETE ON news_ai_assessment_chain
BEGIN
    SELECT RAISE(
        ABORT,
        'BR-172 NewsAI assessment hash chain is immutable and retained for at least five years'
    );
END;

CREATE TABLE IF NOT EXISTS news_ai_delivery_card (
    assessment_id TEXT PRIMARY KEY NOT NULL,
    rendered_text TEXT NOT NULL CHECK (length(trim(rendered_text)) > 0),
    rendered_sha256 TEXT NOT NULL CHECK (length(rendered_sha256) = 64),
    FOREIGN KEY(assessment_id) REFERENCES news_ai_assessment(assessment_id)
);
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_card_no_update
BEFORE UPDATE ON news_ai_delivery_card
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI delivery card is immutable');
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_card_no_delete
BEFORE DELETE ON news_ai_delivery_card
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI delivery card is immutable');
END;

CREATE TABLE IF NOT EXISTS news_ai_delivery_recovery_snapshot (
    assessment_id TEXT PRIMARY KEY NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    fact_snapshot TEXT NOT NULL CHECK (length(fact_snapshot) > 0),
    fact_snapshot_sha256 TEXT NOT NULL CHECK (length(fact_snapshot_sha256) = 64),
    FOREIGN KEY(assessment_id) REFERENCES news_ai_assessment(assessment_id)
);
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_recovery_snapshot_no_update
BEFORE UPDATE ON news_ai_delivery_recovery_snapshot
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI recovery snapshot is immutable');
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_recovery_snapshot_no_delete
BEFORE DELETE ON news_ai_delivery_recovery_snapshot
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI recovery snapshot is immutable');
END;

-- Scheduling progress is separate from delivery completion. A claim rotates
-- pending work; it never authorizes a physical send or closes an assessment.
CREATE TABLE IF NOT EXISTS news_ai_recovery_claim (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    assessment_id TEXT NOT NULL REFERENCES news_ai_assessment(assessment_id),
    category TEXT NOT NULL CHECK (category IN ('ready', 'manual')),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((category = 'ready' AND reason = '') OR
           (category = 'manual' AND length(trim(reason)) > 0))
);
CREATE INDEX IF NOT EXISTS idx_news_ai_recovery_claim_assessment
    ON news_ai_recovery_claim(assessment_id, id);
CREATE TABLE IF NOT EXISTS news_ai_recovery_review_notified (
    claim_id INTEGER PRIMARY KEY REFERENCES news_ai_recovery_claim(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE TRIGGER IF NOT EXISTS trg_news_ai_recovery_claim_no_update
BEFORE UPDATE ON news_ai_recovery_claim
BEGIN SELECT RAISE(ABORT, 'BR-172 recovery claims are immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_recovery_claim_no_delete
BEFORE DELETE ON news_ai_recovery_claim
BEGIN SELECT RAISE(ABORT, 'BR-172 recovery claims are immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_recovery_notified_no_update
BEFORE UPDATE ON news_ai_recovery_review_notified
BEGIN SELECT RAISE(ABORT, 'BR-172 review confirmations are immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_recovery_notified_no_delete
BEFORE DELETE ON news_ai_recovery_review_notified
BEGIN SELECT RAISE(ABORT, 'BR-172 review confirmations are immutable'); END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_recovery_notified_manual_only
BEFORE INSERT ON news_ai_recovery_review_notified
WHEN NOT EXISTS (SELECT 1 FROM news_ai_recovery_claim
                 WHERE id = NEW.claim_id AND category = 'manual')
BEGIN SELECT RAISE(ABORT, 'BR-172 only manual claims can be notified'); END;

CREATE TABLE IF NOT EXISTS news_ai_delivery_event (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    event_id TEXT NOT NULL UNIQUE CHECK (length(event_id) = 64),
    delivery_identity_sha256 TEXT NOT NULL CHECK (length(delivery_identity_sha256) = 64),
    assessment_id TEXT NOT NULL CHECK (length(assessment_id) = 64),
    reservation_id TEXT NOT NULL CHECK (length(reservation_id) = 64),
    state TEXT NOT NULL CHECK (
        state IN (
            'reserved', 'sink_started', 'rolled_back', 'delivered',
            'prediction_linked', 'post_sink_recovery'
        )
    ),
    delivery_audit_event_id TEXT,
    prediction_link_id TEXT,
    reason TEXT,
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    minimum_retention_years INTEGER NOT NULL CHECK (minimum_retention_years >= 5),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(assessment_id) REFERENCES news_ai_assessment(assessment_id),
    UNIQUE(delivery_identity_sha256, reservation_id, state),
    CHECK (
        (state IN ('reserved', 'sink_started')
            AND delivery_audit_event_id IS NULL
            AND prediction_link_id IS NULL
            AND reason IS NULL)
        OR (state = 'rolled_back'
            AND delivery_audit_event_id IS NULL
            AND prediction_link_id IS NULL
            AND length(trim(reason)) > 0)
        OR (state = 'delivered'
            AND length(trim(delivery_audit_event_id)) > 0
            AND prediction_link_id IS NULL
            AND reason IS NULL)
        OR (state = 'prediction_linked'
            AND length(trim(delivery_audit_event_id)) > 0
            AND length(trim(prediction_link_id)) > 0
            AND reason IS NULL)
        OR (state = 'post_sink_recovery'
            AND prediction_link_id IS NULL
            AND length(trim(reason)) > 0)
    )
);
CREATE INDEX IF NOT EXISTS idx_news_ai_delivery_identity
    ON news_ai_delivery_event(delivery_identity_sha256, id);

CREATE TABLE IF NOT EXISTS news_ai_delivery_event_chain (
    delivery_event_row_id INTEGER PRIMARY KEY,
    previous_hash TEXT NOT NULL,
    record_hash TEXT NOT NULL UNIQUE CHECK (length(record_hash) = 64),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY(delivery_event_row_id) REFERENCES news_ai_delivery_event(id)
);

CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_event_no_update
BEFORE UPDATE ON news_ai_delivery_event
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI delivery audit is immutable');
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_event_no_delete
BEFORE DELETE ON news_ai_delivery_event
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI delivery audit retention is at least five years');
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_event_chain_no_update
BEFORE UPDATE ON news_ai_delivery_event_chain
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI delivery hash chain is immutable');
END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_delivery_event_chain_no_delete
BEFORE DELETE ON news_ai_delivery_event_chain
BEGIN
    SELECT RAISE(ABORT, 'BR-172 NewsAI delivery hash chain retention is at least five years');
END;
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NewsAiAuditImpact {
    MajorNegative,
    Negative,
    Neutral,
    Positive,
    MajorPositive,
}

impl NewsAiAuditImpact {
    const fn as_str(self) -> &'static str {
        match self {
            Self::MajorNegative => "major_negative",
            Self::Negative => "negative",
            Self::Neutral => "neutral",
            Self::Positive => "positive",
            Self::MajorPositive => "major_positive",
        }
    }

    fn parse(value: &str) -> NewsAiAssessmentAuditResult<Self> {
        match value {
            "major_negative" => Ok(Self::MajorNegative),
            "negative" => Ok(Self::Negative),
            "neutral" => Ok(Self::Neutral),
            "positive" => Ok(Self::Positive),
            "major_positive" => Ok(Self::MajorPositive),
            _ => Err(audit(format!(
                "persisted assessment impact is invalid: {value:?}"
            ))),
        }
    }

    const fn into_core(self) -> crate::monitor::news_ai::NewsImpact {
        match self {
            Self::MajorNegative => crate::monitor::news_ai::NewsImpact::MajorNegative,
            Self::Negative => crate::monitor::news_ai::NewsImpact::Negative,
            Self::Neutral => crate::monitor::news_ai::NewsImpact::Neutral,
            Self::Positive => crate::monitor::news_ai::NewsImpact::Positive,
            Self::MajorPositive => crate::monitor::news_ai::NewsImpact::MajorPositive,
        }
    }
}

impl From<crate::monitor::news_ai::NewsImpact> for NewsAiAuditImpact {
    fn from(value: crate::monitor::news_ai::NewsImpact) -> Self {
        match value {
            crate::monitor::news_ai::NewsImpact::MajorNegative => Self::MajorNegative,
            crate::monitor::news_ai::NewsImpact::Negative => Self::Negative,
            crate::monitor::news_ai::NewsImpact::Neutral => Self::Neutral,
            crate::monitor::news_ai::NewsImpact::Positive => Self::Positive,
            crate::monitor::news_ai::NewsImpact::MajorPositive => Self::MajorPositive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiAssessmentAuditInput {
    business_identity: Option<NewsAiIdentityV3>,
    recovery_envelope: Option<String>,
    assessment_id: String,
    impact: NewsAiAuditImpact,
    confidence: u8,
    uncertainty: String,
    core_logic: String,
    input_evidence_sha256: String,
    normalized_prompt_sha256: String,
    source_provider: String,
    source_batch_id: String,
    source_item_id: String,
    analysis_version: String,
    target_code: String,
    model_provider: String,
    model: String,
    model_upstream_request_id: Option<String>,
    model_upstream_response_id: String,
    model_system_sha256: String,
    model_user_sha256: String,
    model_response_sha256: String,
    model_started_at: DateTime<FixedOffset>,
    model_completed_at: DateTime<FixedOffset>,
}

impl NewsAiAssessmentAuditInput {
    /// Project the pure BR-172 core result without reconstructing any missing
    /// field. The projection still passes through the same canonical
    /// validation during append.
    pub fn from_core(
        request: &crate::monitor::news_ai::NewsAiRequest,
        assessment: &crate::monitor::news_ai::NewsAiAssessment,
    ) -> NewsAiAssessmentAuditResult<Self> {
        if assessment.input_evidence_sha256() != request.evidence_hash() {
            return Err(invalid(
                "assessment input evidence hash differs from its NewsAI request",
            ));
        }
        let request_prompt_hash = hex::encode(Sha256::digest(request.normalized_prompt()));
        if assessment.normalized_prompt_sha256() != request_prompt_hash
            || assessment.receipt().user_sha256() != request_prompt_hash
        {
            return Err(invalid(
                "assessment/model prompt hash differs from normalized request prompt",
            ));
        }
        let source_provider = source_provider_tag(request.fact().provider())?.to_owned();
        let utc = FixedOffset::east_opt(0)
            .ok_or_else(|| audit("UTC fixed offset is unavailable for model receipt"))?;
        Ok(Self {
            business_identity: request.business_identity().cloned(),
            recovery_envelope: request
                .business_identity()
                .map(|identity| {
                    identity
                        .encode_recovery(request.fact())
                        .and_then(|bytes| {
                            String::from_utf8(bytes).map_err(|error| {
                                crate::monitor::news_ai::NewsAiError::AnalysisAuditFailed(
                                    error.to_string(),
                                )
                            })
                        })
                        .map_err(|error| invalid(error.to_string()))
                })
                .transpose()?,
            assessment_id: assessment.assessment_id().to_owned(),
            impact: assessment.impact().into(),
            confidence: assessment.confidence(),
            uncertainty: assessment.uncertainty().to_owned(),
            core_logic: assessment.core_logic().to_owned(),
            input_evidence_sha256: assessment.input_evidence_sha256().to_owned(),
            normalized_prompt_sha256: assessment.normalized_prompt_sha256().to_owned(),
            source_provider,
            source_batch_id: request.fact().source_batch_id().to_owned(),
            source_item_id: request.fact().item_id().to_owned(),
            analysis_version: request
                .business_identity()
                .map(NewsAiIdentityV3::storage_analysis_version)
                .unwrap_or_else(|| request.analysis_version().to_owned()),
            target_code: request.fact().target_code().to_owned(),
            model_provider: assessment.receipt().provider().to_owned(),
            model: assessment.receipt().model().to_owned(),
            model_upstream_request_id: assessment
                .receipt()
                .upstream_request_id()
                .map(str::to_owned),
            model_upstream_response_id: assessment.receipt().upstream_response_id().to_owned(),
            model_system_sha256: assessment.receipt().system_sha256().to_owned(),
            model_user_sha256: assessment.receipt().user_sha256().to_owned(),
            model_response_sha256: assessment.receipt().response_sha256().to_owned(),
            model_started_at: assessment.receipt().started_at().with_timezone(&utc),
            model_completed_at: assessment.receipt().completed_at().with_timezone(&utc),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsAiAssessmentAuditReceipt {
    pub assessment_id: String,
    pub source_identity_sha256: String,
    pub record_hash: String,
    pub inserted: bool,
}

#[derive(Debug, Error)]
pub enum NewsAiAssessmentAuditError {
    #[error("BR-172 assessment conflict for assessment ID {assessment_id}")]
    Conflict { assessment_id: String },
    #[error("BR-172 invalid assessment audit input: {0}")]
    InvalidInput(String),
    #[error("BR-172 assessment audit failure: {0}")]
    Audit(String),
    #[error("BR-172 assessment audit connection error: {0}")]
    Connection(String),
    #[error("BR-172 assessment audit database error: {0}")]
    Database(#[from] diesel::result::Error),
}

pub type NewsAiAssessmentAuditResult<T> = Result<T, NewsAiAssessmentAuditError>;

/// Durable NewsAI work that can be resumed without seeing the original live
/// provider batch again. Legacy rows are surfaced explicitly and never
/// reconstructed from a different, current batch.
pub enum NewsAiPendingRecovery {
    Ready(crate::monitor::news_ai::AuditedNewsAiAssessment),
    ManualReview {
        assessment_id: String,
        reason: String,
        /// Durable scan claim, not a delivery permission. Confirm only after
        /// the manual-review audit publication has actually succeeded.
        claim_id: i64,
    },
}

#[derive(QueryableByName)]
struct RecoveryClaimRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    assessment_id: String,
    #[diesel(sql_type = Text)]
    category: String,
    #[diesel(sql_type = Text)]
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalSourceIdentity {
    source_provider: String,
    source_batch_id: String,
    source_item_id: String,
    target_code: String,
    analysis_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalAssessment {
    assessment_id: String,
    impact: NewsAiAuditImpact,
    confidence: u8,
    uncertainty: String,
    core_logic: String,
    input_evidence_sha256: String,
    normalized_prompt_sha256: String,
    source_identity: CanonicalSourceIdentity,
    model_provider: String,
    model: String,
    model_upstream_request_id: Option<String>,
    model_upstream_response_id: String,
    model_system_sha256: String,
    model_user_sha256: String,
    model_response_sha256: String,
    model_started_at: String,
    model_completed_at: String,
    minimum_retention_years: i32,
}

#[derive(Debug, QueryableByName, Serialize)]
struct PersistedAssessmentRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    assessment_id: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Text)]
    source_identity_sha256: String,
    #[diesel(sql_type = Text)]
    impact: String,
    #[diesel(sql_type = Integer)]
    confidence: i32,
    #[diesel(sql_type = Text)]
    uncertainty: String,
    #[diesel(sql_type = Text)]
    core_logic: String,
    #[diesel(sql_type = Text)]
    input_evidence_sha256: String,
    #[diesel(sql_type = Text)]
    normalized_prompt_sha256: String,
    #[diesel(sql_type = Text)]
    source_provider: String,
    #[diesel(sql_type = Text)]
    source_batch_id: String,
    #[diesel(sql_type = Text)]
    source_item_id: String,
    #[diesel(sql_type = Text)]
    analysis_version: String,
    #[diesel(sql_type = Text)]
    target_code: String,
    #[diesel(sql_type = Text)]
    model_provider: String,
    #[diesel(sql_type = Text)]
    model: String,
    #[diesel(sql_type = Nullable<Text>)]
    model_upstream_request_id: Option<String>,
    #[diesel(sql_type = Text)]
    model_upstream_response_id: String,
    #[diesel(sql_type = Text)]
    model_system_sha256: String,
    #[diesel(sql_type = Text)]
    model_user_sha256: String,
    #[diesel(sql_type = Text)]
    model_response_sha256: String,
    #[diesel(sql_type = Text)]
    model_started_at: String,
    #[diesel(sql_type = Text)]
    model_completed_at: String,
    #[diesel(sql_type = Integer)]
    minimum_retention_years: i32,
    #[diesel(sql_type = Text)]
    created_at: String,
}

#[derive(Debug, QueryableByName)]
struct PersistedDeliveryCardRow {
    #[diesel(sql_type = Text)]
    rendered_text: String,
    #[diesel(sql_type = Text)]
    rendered_sha256: String,
}

#[derive(Debug, QueryableByName)]
struct PersistedRecoverySnapshotRow {
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    fact_snapshot: String,
    #[diesel(sql_type = Text)]
    fact_snapshot_sha256: String,
}

#[derive(Debug, QueryableByName)]
struct ChainRow {
    #[diesel(sql_type = BigInt)]
    assessment_row_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeliveryEventState {
    Reserved,
    SinkStarted,
    RolledBack,
    Delivered,
    PredictionLinked,
    PostSinkRecovery,
}

impl DeliveryEventState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::SinkStarted => "sink_started",
            Self::RolledBack => "rolled_back",
            Self::Delivered => "delivered",
            Self::PredictionLinked => "prediction_linked",
            Self::PostSinkRecovery => "post_sink_recovery",
        }
    }

    fn parse(value: &str) -> NewsAiAssessmentAuditResult<Self> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "sink_started" => Ok(Self::SinkStarted),
            "rolled_back" => Ok(Self::RolledBack),
            "delivered" => Ok(Self::Delivered),
            "prediction_linked" => Ok(Self::PredictionLinked),
            "post_sink_recovery" => Ok(Self::PostSinkRecovery),
            _ => Err(audit(format!(
                "persisted delivery state is invalid: {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct CanonicalDeliveryEvent {
    schema_version: i32,
    event_id: String,
    delivery_identity_sha256: String,
    assessment_id: String,
    reservation_id: String,
    state: DeliveryEventState,
    delivery_audit_event_id: Option<String>,
    prediction_link_id: Option<String>,
    reason: Option<String>,
    minimum_retention_years: i32,
}

#[derive(Debug, Clone, QueryableByName, Serialize)]
struct PersistedDeliveryEventRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Integer)]
    schema_version: i32,
    #[diesel(sql_type = Text)]
    event_id: String,
    #[diesel(sql_type = Text)]
    delivery_identity_sha256: String,
    #[diesel(sql_type = Text)]
    assessment_id: String,
    #[diesel(sql_type = Text)]
    reservation_id: String,
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Nullable<Text>)]
    delivery_audit_event_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    prediction_link_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    reason: Option<String>,
    #[diesel(sql_type = Text)]
    content_hash: String,
    #[diesel(sql_type = Integer)]
    minimum_retention_years: i32,
    #[diesel(sql_type = Text)]
    created_at: String,
}

#[derive(Debug, Clone, QueryableByName)]
struct DeliveryChainRow {
    #[diesel(sql_type = BigInt)]
    delivery_event_row_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

fn invalid(message: impl Into<String>) -> NewsAiAssessmentAuditError {
    NewsAiAssessmentAuditError::InvalidInput(message.into())
}

fn source_provider_tag(
    provider: crate::market_domain::ProviderId,
) -> NewsAiAssessmentAuditResult<&'static str> {
    match provider {
        crate::market_domain::ProviderId::Eastmoney => Ok("eastmoney"),
        crate::market_domain::ProviderId::Cailianpress => Ok("cailianpress"),
        crate::market_domain::ProviderId::Jin10 => Ok("jin10"),
        crate::market_domain::ProviderId::ThePaper => Ok("thepaper"),
        crate::market_domain::ProviderId::Sina => Ok("sina"),
        _ => Err(invalid(format!(
            "news provider is not admitted by BR-172: {provider:?}"
        ))),
    }
}

fn audit(message: impl Into<String>) -> NewsAiAssessmentAuditError {
    NewsAiAssessmentAuditError::Audit(message.into())
}

fn validate_exact_text(field: &str, value: &str) -> NewsAiAssessmentAuditResult<()> {
    if value.is_empty() || value.trim() != value {
        return Err(invalid(format!(
            "{field} must be non-empty and contain no surrounding whitespace"
        )));
    }
    Ok(())
}

fn validate_sha256(field: &str, value: &str) -> NewsAiAssessmentAuditResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid(format!(
            "{field} must be one lowercase SHA-256 hex digest"
        )));
    }
    Ok(())
}

fn validate_target_code(code: &str) -> NewsAiAssessmentAuditResult<()> {
    validate_exact_text("target_code", code)?;
    crate::risk::env_guard::validate_symbol_for_current_env(code).map_err(invalid)
}

fn canonical_assessment(
    input: &NewsAiAssessmentAuditInput,
) -> NewsAiAssessmentAuditResult<CanonicalAssessment> {
    for (field, value) in [
        ("assessment_id", input.assessment_id.as_str()),
        ("uncertainty", input.uncertainty.as_str()),
        ("core_logic", input.core_logic.as_str()),
        ("source_provider", input.source_provider.as_str()),
        ("source_batch_id", input.source_batch_id.as_str()),
        ("source_item_id", input.source_item_id.as_str()),
        ("analysis_version", input.analysis_version.as_str()),
        ("model_provider", input.model_provider.as_str()),
        ("model", input.model.as_str()),
        (
            "model_upstream_response_id",
            input.model_upstream_response_id.as_str(),
        ),
    ] {
        validate_exact_text(field, value)?;
    }
    if let Some(request_id) = &input.model_upstream_request_id {
        validate_exact_text("model_upstream_request_id", request_id)?;
    }
    for (field, value) in [
        (
            "input_evidence_sha256",
            input.input_evidence_sha256.as_str(),
        ),
        (
            "normalized_prompt_sha256",
            input.normalized_prompt_sha256.as_str(),
        ),
        ("model_system_sha256", input.model_system_sha256.as_str()),
        ("model_user_sha256", input.model_user_sha256.as_str()),
        (
            "model_response_sha256",
            input.model_response_sha256.as_str(),
        ),
    ] {
        validate_sha256(field, value)?;
    }
    validate_target_code(&input.target_code)?;
    validate_sha256("assessment_id", &input.assessment_id)?;
    if !matches!(
        input.source_provider.as_str(),
        "eastmoney" | "cailianpress" | "jin10" | "thepaper" | "sina"
    ) {
        return Err(invalid(format!(
            "source_provider is not admitted by BR-172: {:?}",
            input.source_provider
        )));
    }
    if input.confidence > 100 {
        return Err(invalid("confidence must be within 0..=100"));
    }
    if input.model_user_sha256 != input.normalized_prompt_sha256 {
        return Err(invalid(
            "model user hash differs from normalized NewsAI prompt hash",
        ));
    }
    if input.model_completed_at < input.model_started_at {
        return Err(invalid("model completion precedes model start"));
    }

    let source_identity = CanonicalSourceIdentity {
        source_provider: input.source_provider.clone(),
        source_batch_id: input.source_batch_id.clone(),
        source_item_id: input.source_item_id.clone(),
        target_code: input.target_code.clone(),
        analysis_version: input.analysis_version.clone(),
    };
    let expected_assessment_id = if is_v3_format(&input.analysis_version)? {
        let identity = input
            .business_identity
            .as_ref()
            .ok_or_else(|| invalid("v3 identity is missing"))?;
        let envelope = input
            .recovery_envelope
            .as_ref()
            .ok_or_else(|| invalid("v3 recovery envelope is missing"))?;
        let (decoded, fact) = NewsAiIdentityV3::decode_recovery(envelope.as_bytes())
            .map_err(|e| invalid(e.to_string()))?;
        if identity != &decoded
            || identity.storage_analysis_version() != input.analysis_version
            || source_provider_tag(fact.provider())? != input.source_provider
            || fact.source_batch_id() != input.source_batch_id
            || fact.item_id() != input.source_item_id
            || fact.target_code() != input.target_code
        {
            return Err(invalid(
                "v3 envelope is detached from assessment source evidence",
            ));
        }
        identity.digest()
    } else {
        if input.business_identity.is_some() || input.recovery_envelope.is_some() {
            return Err(invalid("legacy row cannot carry v3 material"));
        }
        core_assessment_id(&source_identity)
    };
    if input.assessment_id != expected_assessment_id {
        return Err(invalid(format!(
            "assessment_id differs from exact BR-172 source identity: expected {expected_assessment_id}"
        )));
    }

    Ok(CanonicalAssessment {
        assessment_id: input.assessment_id.clone(),
        impact: input.impact,
        confidence: input.confidence,
        uncertainty: input.uncertainty.clone(),
        core_logic: input.core_logic.clone(),
        input_evidence_sha256: input.input_evidence_sha256.clone(),
        normalized_prompt_sha256: input.normalized_prompt_sha256.clone(),
        source_identity,
        model_provider: input.model_provider.clone(),
        model: input.model.clone(),
        model_upstream_request_id: input.model_upstream_request_id.clone(),
        model_upstream_response_id: input.model_upstream_response_id.clone(),
        model_system_sha256: input.model_system_sha256.clone(),
        model_user_sha256: input.model_user_sha256.clone(),
        model_response_sha256: input.model_response_sha256.clone(),
        model_started_at: input
            .model_started_at
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
        model_completed_at: input
            .model_completed_at
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
        minimum_retention_years: NEWS_AI_ASSESSMENT_MIN_RETENTION_YEARS,
    })
}

fn hash_serializable<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> NewsAiAssessmentAuditResult<String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| audit(format!("cannot serialize assessment hash payload: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

fn source_identity_hash(source: &CanonicalSourceIdentity) -> NewsAiAssessmentAuditResult<String> {
    hash_serializable(SOURCE_IDENTITY_HASH_DOMAIN, source)
}

fn assessment_content_hash(
    assessment: &CanonicalAssessment,
    envelope: Option<&str>,
) -> NewsAiAssessmentAuditResult<String> {
    if is_v3_format(&assessment.source_identity.analysis_version)? {
        let envelope = envelope.ok_or_else(|| audit("v3 content hash requires frozen envelope"))?;
        // Identity hash excludes live evidence, while content/chain retains
        // every byte of the first frozen fact, including its observation.
        let canonical = serde_json::to_vec(assessment).map_err(|e| audit(e.to_string()))?;
        let mut hash = Sha256::new();
        hash.update(b"BR172_NEWS_AI_ASSESSMENT_CONTENT_V3\0");
        for bytes in [canonical.as_slice(), envelope.as_bytes()] {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        Ok(hex::encode(hash.finalize()))
    } else {
        hash_serializable(CONTENT_HASH_DOMAIN, assessment)
    }
}

/// The immutable audited row, not snapshot presence, chooses the codec.
/// Reserve the entire family so unknown versions cannot masquerade as legacy.
fn is_v3_format(version: &str) -> NewsAiAssessmentAuditResult<bool> {
    if let Some(analysis) = version.strip_prefix(NEWS_AI_V3_STORAGE_PREFIX) {
        validate_exact_text("v3 analysis version", analysis)?;
        return Ok(true);
    }
    if version.starts_with("news_ai_identity_") {
        return Err(audit("unsupported NewsAI identity format"));
    }
    Ok(false)
}

fn core_assessment_id(source: &CanonicalSourceIdentity) -> String {
    let mut hasher = Sha256::new();
    for value in [
        source.source_provider.as_str(),
        source.source_batch_id.as_str(),
        source.source_item_id.as_str(),
        source.target_code.as_str(),
        source.analysis_version.as_str(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn source_identity_from_fact(
    fact: &crate::monitor::news_ai::AdmittedNewsFact,
    analysis_version: &str,
) -> NewsAiAssessmentAuditResult<CanonicalSourceIdentity> {
    validate_exact_text("analysis_version", analysis_version)?;
    validate_exact_text("source_batch_id", fact.source_batch_id())?;
    validate_exact_text("source_item_id", fact.item_id())?;
    validate_target_code(fact.target_code())?;
    Ok(CanonicalSourceIdentity {
        source_provider: source_provider_tag(fact.provider())?.to_owned(),
        source_batch_id: fact.source_batch_id().to_owned(),
        source_item_id: fact.item_id().to_owned(),
        target_code: fact.target_code().to_owned(),
        analysis_version: analysis_version.to_owned(),
    })
}

fn load_rows(
    conn: &mut SqliteConnection,
) -> NewsAiAssessmentAuditResult<Vec<PersistedAssessmentRow>> {
    diesel::sql_query(
        "SELECT id, schema_version, assessment_id, content_hash, source_identity_sha256,
                impact, confidence, uncertainty, core_logic, input_evidence_sha256,
                normalized_prompt_sha256, source_provider, source_batch_id, source_item_id,
                analysis_version, target_code, model_provider, model,
                model_upstream_request_id, model_upstream_response_id,
                model_system_sha256, model_user_sha256, model_response_sha256,
                model_started_at, model_completed_at,
                minimum_retention_years, created_at
           FROM news_ai_assessment
          ORDER BY id ASC",
    )
    .load(conn)
    .map_err(NewsAiAssessmentAuditError::from)
}

fn load_chain(conn: &mut SqliteConnection) -> NewsAiAssessmentAuditResult<Vec<ChainRow>> {
    diesel::sql_query(
        "SELECT assessment_row_id, previous_hash, record_hash
           FROM news_ai_assessment_chain
          ORDER BY assessment_row_id ASC",
    )
    .load(conn)
    .map_err(NewsAiAssessmentAuditError::from)
}

fn load_by_assessment_id(
    conn: &mut SqliteConnection,
    assessment_id: &str,
) -> NewsAiAssessmentAuditResult<Option<PersistedAssessmentRow>> {
    diesel::sql_query(
        "SELECT id, schema_version, assessment_id, content_hash, source_identity_sha256,
                impact, confidence, uncertainty, core_logic, input_evidence_sha256,
                normalized_prompt_sha256, source_provider, source_batch_id, source_item_id,
                analysis_version, target_code, model_provider, model,
                model_upstream_request_id, model_upstream_response_id,
                model_system_sha256, model_user_sha256, model_response_sha256,
                model_started_at, model_completed_at,
                minimum_retention_years, created_at
           FROM news_ai_assessment
          WHERE assessment_id = ?
          LIMIT 1",
    )
    .bind::<Text, _>(assessment_id)
    .get_result(conn)
    .optional()
    .map_err(NewsAiAssessmentAuditError::from)
}

fn load_chain_for_row(
    conn: &mut SqliteConnection,
    assessment_row_id: i64,
) -> NewsAiAssessmentAuditResult<ChainRow> {
    diesel::sql_query(
        "SELECT assessment_row_id, previous_hash, record_hash
           FROM news_ai_assessment_chain
          WHERE assessment_row_id = ?",
    )
    .bind::<BigInt, _>(assessment_row_id)
    .get_result(conn)
    .map_err(NewsAiAssessmentAuditError::from)
}

fn canonical_from_row(
    conn: &mut SqliteConnection,
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<(CanonicalAssessment, Option<NewsAiIdentityV3>)> {
    let recovery_envelope = if is_v3_format(&row.analysis_version)? {
        Some(
            load_recovery_bytes(conn, &row.assessment_id)?
                .ok_or_else(|| audit("v3 recovery envelope is missing"))?,
        )
    } else {
        None
    };
    let identity = recovery_envelope
        .as_ref()
        .map(|bytes| {
            NewsAiIdentityV3::decode_recovery(bytes.as_bytes())
                .map(|(identity, _)| identity)
                .map_err(|e| audit(e.to_string()))
        })
        .transpose()?;
    let confidence = u8::try_from(row.confidence).map_err(|error| {
        audit(format!(
            "persisted confidence is invalid at row {}: {error}",
            row.id
        ))
    })?;
    let started_at = DateTime::parse_from_rfc3339(&row.model_started_at).map_err(|error| {
        audit(format!(
            "persisted model_started_at is invalid at row {}: {error}",
            row.id
        ))
    })?;
    let completed_at = DateTime::parse_from_rfc3339(&row.model_completed_at).map_err(|error| {
        audit(format!(
            "persisted model_completed_at is invalid at row {}: {error}",
            row.id
        ))
    })?;
    let canonical = canonical_assessment(&NewsAiAssessmentAuditInput {
        business_identity: identity.clone(),
        recovery_envelope,
        assessment_id: row.assessment_id.clone(),
        impact: NewsAiAuditImpact::parse(&row.impact)?,
        confidence,
        uncertainty: row.uncertainty.clone(),
        core_logic: row.core_logic.clone(),
        input_evidence_sha256: row.input_evidence_sha256.clone(),
        normalized_prompt_sha256: row.normalized_prompt_sha256.clone(),
        source_provider: row.source_provider.clone(),
        source_batch_id: row.source_batch_id.clone(),
        source_item_id: row.source_item_id.clone(),
        analysis_version: row.analysis_version.clone(),
        target_code: row.target_code.clone(),
        model_provider: row.model_provider.clone(),
        model: row.model.clone(),
        model_upstream_request_id: row.model_upstream_request_id.clone(),
        model_upstream_response_id: row.model_upstream_response_id.clone(),
        model_system_sha256: row.model_system_sha256.clone(),
        model_user_sha256: row.model_user_sha256.clone(),
        model_response_sha256: row.model_response_sha256.clone(),
        model_started_at: started_at,
        model_completed_at: completed_at,
    })
    .map_err(|error| {
        audit(format!(
            "persisted assessment row {} is not canonical: {error}",
            row.id
        ))
    })?;
    if canonical.model_started_at != row.model_started_at
        || canonical.model_completed_at != row.model_completed_at
        || canonical.minimum_retention_years != row.minimum_retention_years
    {
        return Err(audit(format!(
            "persisted assessment row {} has non-canonical timestamp or retention semantics",
            row.id
        )));
    }
    Ok((canonical, identity))
}

fn validate_persisted_row(
    conn: &mut SqliteConnection,
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<CanonicalAssessment> {
    if row.schema_version != SCHEMA_VERSION {
        return Err(audit(format!(
            "unsupported assessment schema version {} at row {}",
            row.schema_version, row.id
        )));
    }
    let (canonical, identity) = canonical_from_row(conn, row)?;
    let expected_source_hash = identity
        .map(|identity| Ok(identity.digest()))
        .unwrap_or_else(|| source_identity_hash(&canonical.source_identity))?;
    let envelope = if is_v3_format(&row.analysis_version)? {
        load_recovery_bytes(conn, &row.assessment_id)?
    } else {
        None
    };
    let expected_content_hash = assessment_content_hash(&canonical, envelope.as_deref())?;
    if row.source_identity_sha256 != expected_source_hash
        || row.content_hash != expected_content_hash
    {
        return Err(audit(format!(
            "assessment identity/content hash mismatch at row {}",
            row.id
        )));
    }
    Ok(canonical)
}

fn calculate_chain_hash(
    previous_hash: &str,
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<String> {
    let encoded = serde_json::to_vec(row)
        .map_err(|error| audit(format!("cannot serialize persisted assessment: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(CHAIN_HASH_DOMAIN);
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn validate_news_ai_assessment_chain(
    conn: &mut SqliteConnection,
) -> NewsAiAssessmentAuditResult<String> {
    let rows = load_rows(conn)?;
    let chain = load_chain(conn)?;
    if rows.len() != chain.len() {
        return Err(audit(format!(
            "assessment hash-chain length mismatch: rows={}, links={}",
            rows.len(),
            chain.len()
        )));
    }

    let mut previous_hash = CHAIN_GENESIS.to_owned();
    for (row, link) in rows.iter().zip(chain.iter()) {
        validate_persisted_row(conn, row)?;
        if is_v3_format(&row.analysis_version)?
            && load_frozen_delivery_card(conn, &row.assessment_id)?.is_none()
        {
            return Err(audit("v3 frozen delivery card is missing"));
        }
        if link.assessment_row_id != row.id || link.previous_hash != previous_hash {
            return Err(audit(format!(
                "assessment hash-chain linkage mismatch at row {}",
                row.id
            )));
        }
        let expected_hash = calculate_chain_hash(&previous_hash, row)?;
        if link.record_hash != expected_hash {
            return Err(audit(format!(
                "assessment hash-chain record mismatch at row {}",
                row.id
            )));
        }
        previous_hash = link.record_hash.clone();
    }
    Ok(previous_hash)
}

fn delivery_event_id(
    previous_hash: &str,
    delivery_identity_sha256: &str,
    reservation_id: &str,
    state: DeliveryEventState,
    delivery_audit_event_id: Option<&str>,
    prediction_link_id: Option<&str>,
    reason: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_ID_DOMAIN);
    for value in [
        previous_hash,
        delivery_identity_sha256,
        reservation_id,
        state.as_str(),
        delivery_audit_event_id.unwrap_or(""),
        prediction_link_id.unwrap_or(""),
        reason.unwrap_or(""),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn reservation_id(previous_hash: &str, delivery_identity_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_ID_DOMAIN);
    hasher.update(b"reservation\0");
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(delivery_identity_sha256.as_bytes());
    hex::encode(hasher.finalize())
}

fn canonical_delivery_event(
    row: &PersistedDeliveryEventRow,
) -> NewsAiAssessmentAuditResult<CanonicalDeliveryEvent> {
    if row.schema_version != SCHEMA_VERSION
        || row.minimum_retention_years < NEWS_AI_ASSESSMENT_MIN_RETENTION_YEARS
    {
        return Err(audit(format!(
            "unsupported delivery schema/retention at row {}",
            row.id
        )));
    }
    for (field, value) in [
        ("delivery event ID", row.event_id.as_str()),
        ("delivery identity", row.delivery_identity_sha256.as_str()),
        ("delivery assessment ID", row.assessment_id.as_str()),
        ("delivery reservation ID", row.reservation_id.as_str()),
        ("delivery content hash", row.content_hash.as_str()),
    ] {
        validate_sha256(field, value)?;
    }
    if row.delivery_identity_sha256 != row.assessment_id {
        return Err(audit(format!(
            "delivery identity differs from assessment at row {}",
            row.id
        )));
    }
    for (field, value) in [
        (
            "delivery_audit_event_id",
            row.delivery_audit_event_id.as_deref(),
        ),
        ("prediction_link_id", row.prediction_link_id.as_deref()),
        ("delivery reason", row.reason.as_deref()),
    ] {
        if let Some(value) = value {
            validate_exact_text(field, value)?;
        }
    }
    let state = DeliveryEventState::parse(&row.state)?;
    let fields_valid = match state {
        DeliveryEventState::Reserved | DeliveryEventState::SinkStarted => {
            row.delivery_audit_event_id.is_none()
                && row.prediction_link_id.is_none()
                && row.reason.is_none()
        }
        DeliveryEventState::RolledBack => {
            row.delivery_audit_event_id.is_none()
                && row.prediction_link_id.is_none()
                && row.reason.is_some()
        }
        DeliveryEventState::Delivered => {
            row.delivery_audit_event_id.is_some()
                && row.prediction_link_id.is_none()
                && row.reason.is_none()
        }
        DeliveryEventState::PredictionLinked => {
            row.delivery_audit_event_id.is_some()
                && row.prediction_link_id.is_some()
                && row.reason.is_none()
        }
        DeliveryEventState::PostSinkRecovery => {
            row.prediction_link_id.is_none() && row.reason.is_some()
        }
    };
    if !fields_valid {
        return Err(audit(format!(
            "delivery state fields are inconsistent at row {}",
            row.id
        )));
    }
    Ok(CanonicalDeliveryEvent {
        schema_version: row.schema_version,
        event_id: row.event_id.clone(),
        delivery_identity_sha256: row.delivery_identity_sha256.clone(),
        assessment_id: row.assessment_id.clone(),
        reservation_id: row.reservation_id.clone(),
        state,
        delivery_audit_event_id: row.delivery_audit_event_id.clone(),
        prediction_link_id: row.prediction_link_id.clone(),
        reason: row.reason.clone(),
        minimum_retention_years: row.minimum_retention_years,
    })
}

fn delivery_content_hash(
    canonical: &CanonicalDeliveryEvent,
) -> NewsAiAssessmentAuditResult<String> {
    let encoded = serde_json::to_vec(canonical)
        .map_err(|error| audit(format!("cannot serialize delivery event: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_CONTENT_HASH_DOMAIN);
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

fn load_delivery_rows(
    conn: &mut SqliteConnection,
) -> NewsAiAssessmentAuditResult<Vec<PersistedDeliveryEventRow>> {
    diesel::sql_query(
        "SELECT id, schema_version, event_id, delivery_identity_sha256, assessment_id,
                reservation_id, state, delivery_audit_event_id, prediction_link_id, reason,
                content_hash, minimum_retention_years, created_at
           FROM news_ai_delivery_event
          ORDER BY id ASC",
    )
    .load(conn)
    .map_err(NewsAiAssessmentAuditError::from)
}

fn load_delivery_chain(
    conn: &mut SqliteConnection,
) -> NewsAiAssessmentAuditResult<Vec<DeliveryChainRow>> {
    diesel::sql_query(
        "SELECT delivery_event_row_id, previous_hash, record_hash
           FROM news_ai_delivery_event_chain
          ORDER BY delivery_event_row_id ASC",
    )
    .load(conn)
    .map_err(NewsAiAssessmentAuditError::from)
}

fn delivery_chain_hash(
    previous_hash: &str,
    row: &PersistedDeliveryEventRow,
) -> NewsAiAssessmentAuditResult<String> {
    let encoded = serde_json::to_vec(row).map_err(|error| {
        audit(format!(
            "cannot serialize persisted delivery event: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_CHAIN_HASH_DOMAIN);
    hasher.update(previous_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(encoded);
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn validate_news_ai_delivery_audit(
    conn: &mut SqliteConnection,
) -> NewsAiAssessmentAuditResult<String> {
    let rows = load_delivery_rows(conn)?;
    let chain = load_delivery_chain(conn)?;
    if rows.len() != chain.len() {
        return Err(audit(format!(
            "delivery hash-chain length mismatch: rows={}, links={}",
            rows.len(),
            chain.len()
        )));
    }
    let mut previous_hash = DELIVERY_CHAIN_GENESIS.to_owned();
    let mut states: BTreeMap<String, CanonicalDeliveryEvent> = BTreeMap::new();
    for (row, link) in rows.iter().zip(chain.iter()) {
        let canonical = canonical_delivery_event(row)?;
        if row.content_hash != delivery_content_hash(&canonical)? {
            return Err(audit(format!(
                "delivery content hash mismatch at row {}",
                row.id
            )));
        }
        if link.delivery_event_row_id != row.id || link.previous_hash != previous_hash {
            return Err(audit(format!(
                "delivery hash-chain linkage mismatch at row {}",
                row.id
            )));
        }
        let expected_hash = delivery_chain_hash(&previous_hash, row)?;
        if link.record_hash != expected_hash {
            return Err(audit(format!(
                "delivery hash-chain record mismatch at row {}",
                row.id
            )));
        }

        let prior = states.get(&canonical.delivery_identity_sha256);
        let transition_valid = match canonical.state {
            DeliveryEventState::Reserved => prior.is_none_or(|prior| {
                matches!(
                    prior.state,
                    DeliveryEventState::RolledBack
                        | DeliveryEventState::SinkStarted
                        | DeliveryEventState::PostSinkRecovery
                )
            }),
            DeliveryEventState::SinkStarted => prior.is_some_and(|prior| {
                prior.state == DeliveryEventState::Reserved
                    && prior.reservation_id == canonical.reservation_id
            }),
            DeliveryEventState::RolledBack => prior.is_some_and(|prior| {
                matches!(
                    prior.state,
                    DeliveryEventState::Reserved | DeliveryEventState::SinkStarted
                ) && prior.reservation_id == canonical.reservation_id
            }),
            DeliveryEventState::Delivered => prior.is_some_and(|prior| {
                prior.reservation_id == canonical.reservation_id
                    && (prior.state == DeliveryEventState::SinkStarted
                        || (prior.state == DeliveryEventState::PostSinkRecovery
                            && prior.delivery_audit_event_id == canonical.delivery_audit_event_id))
            }),
            DeliveryEventState::PredictionLinked => prior.is_some_and(|prior| {
                prior.state == DeliveryEventState::Delivered
                    && prior.reservation_id == canonical.reservation_id
                    && prior.delivery_audit_event_id == canonical.delivery_audit_event_id
            }),
            DeliveryEventState::PostSinkRecovery => prior.is_some_and(|prior| {
                prior.state == DeliveryEventState::SinkStarted
                    && prior.reservation_id == canonical.reservation_id
            }),
        };
        if !transition_valid {
            return Err(audit(format!(
                "invalid delivery transition to {} at row {}",
                canonical.state.as_str(),
                row.id
            )));
        }
        states.insert(canonical.delivery_identity_sha256.clone(), canonical);
        previous_hash = link.record_hash.clone();
    }
    Ok(previous_hash)
}

#[allow(clippy::too_many_arguments)]
fn append_delivery_event(
    conn: &mut SqliteConnection,
    delivery_identity_sha256: &str,
    assessment_id: &str,
    reservation_id: &str,
    state: DeliveryEventState,
    delivery_audit_event_id: Option<&str>,
    prediction_link_id: Option<&str>,
    reason: Option<&str>,
) -> NewsAiAssessmentAuditResult<String> {
    validate_sha256("delivery identity", delivery_identity_sha256)?;
    validate_sha256("delivery assessment", assessment_id)?;
    validate_sha256("delivery reservation", reservation_id)?;
    let previous_hash = validate_news_ai_delivery_audit(conn)?;
    let event_id = delivery_event_id(
        &previous_hash,
        delivery_identity_sha256,
        reservation_id,
        state,
        delivery_audit_event_id,
        prediction_link_id,
        reason,
    );
    let canonical = CanonicalDeliveryEvent {
        schema_version: SCHEMA_VERSION,
        event_id: event_id.clone(),
        delivery_identity_sha256: delivery_identity_sha256.to_owned(),
        assessment_id: assessment_id.to_owned(),
        reservation_id: reservation_id.to_owned(),
        state,
        delivery_audit_event_id: delivery_audit_event_id.map(str::to_owned),
        prediction_link_id: prediction_link_id.map(str::to_owned),
        reason: reason.map(str::to_owned),
        minimum_retention_years: NEWS_AI_ASSESSMENT_MIN_RETENTION_YEARS,
    };
    let content_hash = delivery_content_hash(&canonical)?;
    let inserted = diesel::sql_query(
        "INSERT INTO news_ai_delivery_event (
            schema_version, event_id, delivery_identity_sha256, assessment_id,
            reservation_id, state, delivery_audit_event_id, prediction_link_id,
            reason, content_hash, minimum_retention_years
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<Text, _>(&event_id)
    .bind::<Text, _>(delivery_identity_sha256)
    .bind::<Text, _>(assessment_id)
    .bind::<Text, _>(reservation_id)
    .bind::<Text, _>(state.as_str())
    .bind::<Nullable<Text>, _>(delivery_audit_event_id)
    .bind::<Nullable<Text>, _>(prediction_link_id)
    .bind::<Nullable<Text>, _>(reason)
    .bind::<Text, _>(&content_hash)
    .bind::<Integer, _>(NEWS_AI_ASSESSMENT_MIN_RETENTION_YEARS)
    .execute(conn)?;
    if inserted != 1 {
        return Err(audit(format!(
            "delivery append affected {inserted} event rows"
        )));
    }
    let row = diesel::sql_query(
        "SELECT id, schema_version, event_id, delivery_identity_sha256, assessment_id,
                reservation_id, state, delivery_audit_event_id, prediction_link_id, reason,
                content_hash, minimum_retention_years, created_at
           FROM news_ai_delivery_event WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedDeliveryEventRow>(conn)?;
    let record_hash = delivery_chain_hash(&previous_hash, &row)?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO news_ai_delivery_event_chain (
            delivery_event_row_id, previous_hash, record_hash
        ) VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(&previous_hash)
    .bind::<Text, _>(&record_hash)
    .execute(conn)?;
    if chain_inserted != 1 {
        return Err(audit(format!(
            "delivery append affected {chain_inserted} chain rows"
        )));
    }
    validate_news_ai_delivery_audit(conn)?;
    Ok(record_hash)
}

fn latest_delivery_event(
    conn: &mut SqliteConnection,
    delivery_identity_sha256: &str,
) -> NewsAiAssessmentAuditResult<Option<PersistedDeliveryEventRow>> {
    diesel::sql_query(
        "SELECT id, schema_version, event_id, delivery_identity_sha256, assessment_id,
                reservation_id, state, delivery_audit_event_id, prediction_link_id, reason,
                content_hash, minimum_retention_years, created_at
           FROM news_ai_delivery_event
          WHERE delivery_identity_sha256 = ?
          ORDER BY id DESC LIMIT 1",
    )
    .bind::<Text, _>(delivery_identity_sha256)
    .get_result(conn)
    .optional()
    .map_err(NewsAiAssessmentAuditError::from)
}

fn link_recovery_from_event(
    identity: &str,
    latest: &PersistedDeliveryEventRow,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiReserveOutcome> {
    let audit_event_id = latest
        .delivery_audit_event_id
        .as_deref()
        .ok_or_else(|| audit("delivered NewsAI row is missing authoritative audit event ID"))?;
    let reservation = crate::monitor::news_ai::NewsAiDeliveryReservation::try_new(
        identity,
        &latest.reservation_id,
    )
    .map_err(|error| {
        audit(format!(
            "persisted link recovery reservation rejected: {error}"
        ))
    })?;
    let delivery_audit =
        crate::monitor::news_ai::NewsAiDeliveryAuditReceipt::try_new(identity, audit_event_id)
            .map_err(|error| audit(format!("persisted link recovery audit rejected: {error}")))?;
    crate::monitor::news_ai::NewsAiDeliveryLinkRecovery::try_new(reservation, delivery_audit)
        .map(crate::monitor::news_ai::NewsAiReserveOutcome::LinkPending)
        .map_err(|error| audit(format!("persisted link recovery rejected: {error}")))
}

/// A counted decision with this exact identity is durably rejected. Its
/// pre-sink policy denial is final even after the ticket cooldown expires;
/// only a new analysis version can create a new counted decision. Transport
/// errors and other preflight failures remain retryable.
fn is_counted_terminal_denial_reason(reason: &str) -> bool {
    matches!(
        reason,
        "BR172_PRE_SINK_NOT_DELIVERED:durable delivery terminal state=RejectedDurable"
            | "BR172_PRE_SINK_NOT_DELIVERED:durable delivery terminal state=ManualResolvedRejected"
    )
}

/// Scheduling hint only. A matching terminal rollback is safe to skip; any
/// other state still enters the fully validated audit path before work.
fn is_news_ai_terminal_denial_for_fact_on_conn(
    conn: &mut SqliteConnection,
    fact: &crate::monitor::news_ai::AdmittedNewsFact,
    analysis_version: &str,
) -> NewsAiAssessmentAuditResult<bool> {
    let identity = source_identity_from_fact(fact, analysis_version)?;
    let assessment_id = core_assessment_id(&identity);
    let latest = latest_delivery_event(conn, &assessment_id)?;
    Ok(latest.is_some_and(|event| {
        event.assessment_id == assessment_id
            && event.state == DeliveryEventState::RolledBack.as_str()
            && event
                .reason
                .as_deref()
                .is_some_and(is_counted_terminal_denial_reason)
    }))
}

pub(crate) fn reserve_news_ai_delivery_on_conn(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiReserveOutcome> {
    validate_news_ai_assessment_chain(conn)?;
    validate_news_ai_delivery_audit(conn)?;
    if delivery.assessment().impact() == crate::monitor::news_ai::NewsImpact::Neutral {
        return Err(invalid("neutral NewsAI assessment cannot reserve delivery"));
    }
    let identity = delivery.identity().sha256();
    if identity != delivery.assessment().assessment_id() {
        return Err(invalid(
            "delivery identity differs from immutable assessment identity",
        ));
    }
    let assessment = load_by_assessment_id(conn, delivery.assessment().assessment_id())?
        .ok_or_else(|| invalid("delivery assessment is not durably retained"))?;
    let assessment_chain = load_chain_for_row(conn, assessment.id)?;
    if assessment_chain.record_hash != delivery.assessment_audit_record_sha256() {
        return Err(invalid(
            "delivery assessment audit receipt differs from retained chain",
        ));
    }

    if let Some(latest) = latest_delivery_event(conn, identity)? {
        let state = DeliveryEventState::parse(&latest.state)?;
        return match state {
            DeliveryEventState::RolledBack => {
                if latest
                    .reason
                    .as_deref()
                    .is_some_and(is_counted_terminal_denial_reason)
                {
                    return Ok(crate::monitor::news_ai::NewsAiReserveOutcome::Deduped);
                }
                reserve_new_delivery(conn, delivery)
            }
            DeliveryEventState::Reserved => {
                crate::monitor::news_ai::NewsAiDeliveryReservation::try_new(
                    identity,
                    &latest.reservation_id,
                )
                .map(crate::monitor::news_ai::NewsAiReserveOutcome::Reserved)
                .map_err(|error| audit(format!("persisted reservation rejected: {error}")))
            }
            DeliveryEventState::Delivered => link_recovery_from_event(identity, &latest),
            DeliveryEventState::SinkStarted | DeliveryEventState::PostSinkRecovery => {
                // New assessments freeze the exact card before any counted
                // decision. A legacy attempted card without that snapshot is
                // ambiguous and must remain parked for manual review.
                if load_frozen_delivery_card(conn, delivery.assessment().assessment_id())?.is_none()
                {
                    return Ok(crate::monitor::news_ai::NewsAiReserveOutcome::Deduped);
                }
                if state == DeliveryEventState::PostSinkRecovery
                    && latest.delivery_audit_event_id.is_some()
                {
                    append_delivery_event(
                        conn,
                        identity,
                        delivery.assessment().assessment_id(),
                        &latest.reservation_id,
                        DeliveryEventState::Delivered,
                        latest.delivery_audit_event_id.as_deref(),
                        None,
                        None,
                    )?;
                    return link_recovery_from_event(identity, &latest);
                }
                // The counted decision already holds the only physical send.
                // A new BR-172 reservation will re-read that decision and
                // append only missing analytics/audit state.
                reserve_new_delivery(conn, delivery)
            }
            DeliveryEventState::PredictionLinked => {
                Ok(crate::monitor::news_ai::NewsAiReserveOutcome::Deduped)
            }
        };
    }
    reserve_new_delivery(conn, delivery)
}

fn reserve_new_delivery(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiReserveOutcome> {
    let previous_hash = validate_news_ai_delivery_audit(conn)?;
    let identity = delivery.identity().sha256();
    let reservation_id = reservation_id(&previous_hash, identity);
    append_delivery_event(
        conn,
        identity,
        delivery.assessment().assessment_id(),
        &reservation_id,
        DeliveryEventState::Reserved,
        None,
        None,
        None,
    )?;
    crate::monitor::news_ai::NewsAiDeliveryReservation::try_new(identity, &reservation_id)
        .map(crate::monitor::news_ai::NewsAiReserveOutcome::Reserved)
        .map_err(|error| audit(format!("new reservation rejected: {error}")))
}

fn require_current_delivery_state(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    expected: DeliveryEventState,
) -> NewsAiAssessmentAuditResult<PersistedDeliveryEventRow> {
    if reservation.delivery_identity_sha256() != delivery.identity().sha256() {
        return Err(invalid("delivery reservation identity mismatch"));
    }
    validate_news_ai_assessment_chain(conn)?;
    validate_news_ai_delivery_audit(conn)?;
    let latest = latest_delivery_event(conn, delivery.identity().sha256())?
        .ok_or_else(|| invalid("delivery reservation is absent"))?;
    if latest.assessment_id != delivery.assessment().assessment_id()
        || latest.reservation_id != reservation.reservation_id()
    {
        return Err(invalid("delivery reservation is not current"));
    }
    let actual = DeliveryEventState::parse(&latest.state)?;
    if actual != expected {
        return Err(invalid(format!(
            "delivery state {} cannot perform operation requiring {}",
            actual.as_str(),
            expected.as_str()
        )));
    }
    Ok(latest)
}

pub(crate) fn begin_news_ai_sink_attempt_on_conn(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
) -> NewsAiAssessmentAuditResult<()> {
    require_current_delivery_state(conn, delivery, reservation, DeliveryEventState::Reserved)?;
    append_delivery_event(
        conn,
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        reservation.reservation_id(),
        DeliveryEventState::SinkStarted,
        None,
        None,
        None,
    )?;
    Ok(())
}

pub(crate) fn record_news_ai_delivered_on_conn(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    authoritative_delivery_audit_event_id: &str,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiDeliveryAuditReceipt> {
    validate_exact_text(
        "authoritative delivery audit event ID",
        authoritative_delivery_audit_event_id,
    )?;
    require_current_delivery_state(conn, delivery, reservation, DeliveryEventState::SinkStarted)?;
    append_delivery_event(
        conn,
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        reservation.reservation_id(),
        DeliveryEventState::Delivered,
        Some(authoritative_delivery_audit_event_id),
        None,
        None,
    )?;
    crate::monitor::news_ai::NewsAiDeliveryAuditReceipt::try_new(
        delivery.identity().sha256(),
        authoritative_delivery_audit_event_id,
    )
    .map_err(|error| audit(format!("delivery audit receipt rejected: {error}")))
}

fn prediction_link_id(
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    delivery_audit: &crate::monitor::news_ai::NewsAiDeliveryAuditReceipt,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PREDICTION_LINK_ID_DOMAIN);
    for value in [
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        reservation.reservation_id(),
        delivery_audit.audit_event_id(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn link_news_ai_prediction_on_conn(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    delivery_audit: &crate::monitor::news_ai::NewsAiDeliveryAuditReceipt,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiPredictionLinkReceipt> {
    if delivery_audit.delivery_identity_sha256() != delivery.identity().sha256() {
        return Err(invalid("delivery audit identity mismatch"));
    }
    if reservation.delivery_identity_sha256() != delivery.identity().sha256() {
        return Err(invalid("prediction link reservation identity mismatch"));
    }
    validate_news_ai_assessment_chain(conn)?;
    validate_news_ai_delivery_audit(conn)?;
    let latest = latest_delivery_event(conn, delivery.identity().sha256())?
        .ok_or_else(|| invalid("prediction link has no delivered state"))?;
    if latest.assessment_id != delivery.assessment().assessment_id()
        || latest.reservation_id != reservation.reservation_id()
    {
        return Err(invalid("prediction link reservation is not current"));
    }
    if latest.delivery_audit_event_id.as_deref() != Some(delivery_audit.audit_event_id()) {
        return Err(invalid("delivery audit event is not current"));
    }
    match DeliveryEventState::parse(&latest.state)? {
        DeliveryEventState::PredictionLinked => {
            let persisted_link_id = latest
                .prediction_link_id
                .as_deref()
                .ok_or_else(|| audit("prediction-linked row is missing its link ID"))?;
            return crate::monitor::news_ai::NewsAiPredictionLinkReceipt::try_new(
                delivery.identity().sha256(),
                delivery.assessment().assessment_id(),
                delivery_audit.audit_event_id(),
                persisted_link_id,
            )
            .map_err(|error| audit(format!("persisted prediction link rejected: {error}")));
        }
        DeliveryEventState::Delivered => {}
        state => {
            return Err(invalid(format!(
                "delivery state {} cannot link prediction",
                state.as_str()
            )));
        }
    }
    let prediction_link_id = prediction_link_id(delivery, reservation, delivery_audit);
    append_delivery_event(
        conn,
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        reservation.reservation_id(),
        DeliveryEventState::PredictionLinked,
        Some(delivery_audit.audit_event_id()),
        Some(&prediction_link_id),
        None,
    )?;
    crate::monitor::news_ai::NewsAiPredictionLinkReceipt::try_new(
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        delivery_audit.audit_event_id(),
        &prediction_link_id,
    )
    .map_err(|error| audit(format!("prediction link receipt rejected: {error}")))
}

pub(crate) fn record_news_ai_post_sink_recovery_on_conn(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    authoritative_delivery_audit_event_id: Option<&str>,
    reason: &str,
) -> NewsAiAssessmentAuditResult<()> {
    validate_exact_text("post-sink recovery reason", reason)?;
    if let Some(event_id) = authoritative_delivery_audit_event_id {
        validate_exact_text("post-sink delivery audit event ID", event_id)?;
    }
    require_current_delivery_state(conn, delivery, reservation, DeliveryEventState::SinkStarted)?;
    append_delivery_event(
        conn,
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        reservation.reservation_id(),
        DeliveryEventState::PostSinkRecovery,
        authoritative_delivery_audit_event_id,
        None,
        Some(reason),
    )?;
    Ok(())
}

pub(crate) fn rollback_news_ai_delivery_on_conn(
    conn: &mut SqliteConnection,
    delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    reason: &str,
) -> NewsAiAssessmentAuditResult<()> {
    validate_exact_text("delivery rollback reason", reason)?;
    if reservation.delivery_identity_sha256() != delivery.identity().sha256() {
        return Err(invalid("rollback reservation identity mismatch"));
    }
    validate_news_ai_delivery_audit(conn)?;
    let latest = latest_delivery_event(conn, delivery.identity().sha256())?
        .ok_or_else(|| invalid("rollback has no reservation"))?;
    let state = DeliveryEventState::parse(&latest.state)?;
    if latest.reservation_id != reservation.reservation_id() {
        return Err(invalid("rollback reservation ID is not current"));
    }
    if state == DeliveryEventState::RolledBack {
        return Ok(());
    }
    if !matches!(
        state,
        DeliveryEventState::Reserved | DeliveryEventState::SinkStarted
    ) {
        return Err(invalid(format!(
            "delivery state {} cannot rollback",
            state.as_str()
        )));
    }
    append_delivery_event(
        conn,
        delivery.identity().sha256(),
        delivery.assessment().assessment_id(),
        reservation.reservation_id(),
        DeliveryEventState::RolledBack,
        None,
        None,
        Some(reason),
    )?;
    Ok(())
}

pub(super) fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    conn.batch_execute(SCHEMA)
        .map_err(|error| format!("BR-172 create NewsAI assessment schema: {error}"))?;
    conn.batch_execute(critical_strength::SCHEMA)
        .map_err(|error| format!("BR244 score schema: {error}"))?;
    conn.batch_execute(global_critical::SCHEMA).map_err(|e|format!("global N01 immutable schema: {e}"))?;
    validate_news_ai_assessment_chain(conn).map_err(|error| error.to_string())?;
    validate_news_ai_delivery_audit(conn)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn insert_assessment_in_transaction(
    conn: &mut SqliteConnection,
    input: &NewsAiAssessmentAuditInput,
) -> NewsAiAssessmentAuditResult<NewsAiAssessmentAuditReceipt> {
    let canonical = canonical_assessment(input)?;
    let expected_source_hash = input
        .business_identity
        .as_ref()
        .map(|identity| Ok(identity.digest()))
        .unwrap_or_else(|| source_identity_hash(&canonical.source_identity))?;
    let expected_content_hash =
        assessment_content_hash(&canonical, input.recovery_envelope.as_deref())?;
    let previous_hash = validate_news_ai_assessment_chain(conn)?;

    if let Some(existing) = load_by_assessment_id(conn, &canonical.assessment_id)? {
        if existing.content_hash != expected_content_hash
            || existing.source_identity_sha256 != expected_source_hash
        {
            return Err(NewsAiAssessmentAuditError::Conflict {
                assessment_id: canonical.assessment_id,
            });
        }
        validate_persisted_row(conn, &existing)?;
        let link = load_chain_for_row(conn, existing.id)?;
        return Ok(NewsAiAssessmentAuditReceipt {
            assessment_id: existing.assessment_id,
            source_identity_sha256: existing.source_identity_sha256,
            record_hash: link.record_hash,
            inserted: false,
        });
    }

    let inserted = diesel::sql_query(
        "INSERT INTO news_ai_assessment (
            schema_version, assessment_id, content_hash, source_identity_sha256,
            impact, confidence, uncertainty, core_logic, input_evidence_sha256,
            normalized_prompt_sha256, source_provider, source_batch_id, source_item_id,
            analysis_version, target_code, model_provider, model,
            model_upstream_request_id, model_upstream_response_id,
            model_system_sha256, model_user_sha256, model_response_sha256,
            model_started_at, model_completed_at,
            minimum_retention_years
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Integer, _>(SCHEMA_VERSION)
    .bind::<Text, _>(&canonical.assessment_id)
    .bind::<Text, _>(&expected_content_hash)
    .bind::<Text, _>(&expected_source_hash)
    .bind::<Text, _>(canonical.impact.as_str())
    .bind::<Integer, _>(i32::from(canonical.confidence))
    .bind::<Text, _>(&canonical.uncertainty)
    .bind::<Text, _>(&canonical.core_logic)
    .bind::<Text, _>(&canonical.input_evidence_sha256)
    .bind::<Text, _>(&canonical.normalized_prompt_sha256)
    .bind::<Text, _>(&canonical.source_identity.source_provider)
    .bind::<Text, _>(&canonical.source_identity.source_batch_id)
    .bind::<Text, _>(&canonical.source_identity.source_item_id)
    .bind::<Text, _>(&canonical.source_identity.analysis_version)
    .bind::<Text, _>(&canonical.source_identity.target_code)
    .bind::<Text, _>(&canonical.model_provider)
    .bind::<Text, _>(&canonical.model)
    .bind::<Nullable<Text>, _>(canonical.model_upstream_request_id.as_deref())
    .bind::<Text, _>(&canonical.model_upstream_response_id)
    .bind::<Text, _>(&canonical.model_system_sha256)
    .bind::<Text, _>(&canonical.model_user_sha256)
    .bind::<Text, _>(&canonical.model_response_sha256)
    .bind::<Text, _>(&canonical.model_started_at)
    .bind::<Text, _>(&canonical.model_completed_at)
    .bind::<Integer, _>(canonical.minimum_retention_years)
    .execute(conn)?;
    if inserted != 1 {
        return Err(audit(format!(
            "assessment append affected {inserted} fact rows"
        )));
    }

    let row = diesel::sql_query(
        "SELECT id, schema_version, assessment_id, content_hash, source_identity_sha256,
                impact, confidence, uncertainty, core_logic, input_evidence_sha256,
                normalized_prompt_sha256, source_provider, source_batch_id, source_item_id,
                analysis_version, target_code, model_provider, model,
                model_upstream_request_id, model_upstream_response_id,
                model_system_sha256, model_user_sha256, model_response_sha256,
                model_started_at, model_completed_at,
                minimum_retention_years, created_at
           FROM news_ai_assessment
          WHERE id = last_insert_rowid()",
    )
    .get_result::<PersistedAssessmentRow>(conn)?;
    if let Some(envelope) = &input.recovery_envelope {
        freeze_recovery_bytes(conn, &row.assessment_id, envelope)?;
    }
    validate_persisted_row(conn, &row)?;
    let record_hash = calculate_chain_hash(&previous_hash, &row)?;
    let chain_inserted = diesel::sql_query(
        "INSERT INTO news_ai_assessment_chain (
            assessment_row_id, previous_hash, record_hash
        ) VALUES (?, ?, ?)",
    )
    .bind::<BigInt, _>(row.id)
    .bind::<Text, _>(&previous_hash)
    .bind::<Text, _>(&record_hash)
    .execute(conn)?;
    if chain_inserted != 1 {
        return Err(audit(format!(
            "assessment append affected {chain_inserted} chain rows"
        )));
    }

    Ok(NewsAiAssessmentAuditReceipt {
        assessment_id: canonical.assessment_id,
        source_identity_sha256: expected_source_hash,
        record_hash,
        inserted: true,
    })
}

pub(crate) fn append_news_ai_assessment_on_conn(
    conn: &mut SqliteConnection,
    input: &NewsAiAssessmentAuditInput,
) -> NewsAiAssessmentAuditResult<NewsAiAssessmentAuditReceipt> {
    if input.business_identity.is_some() || is_v3_format(&input.analysis_version)? {
        return Err(invalid("v3 requires the complete audited append owner"));
    }
    conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
        insert_assessment_in_transaction(conn, input)
    })
}

fn load_frozen_delivery_card(
    conn: &mut SqliteConnection,
    assessment_id: &str,
) -> NewsAiAssessmentAuditResult<Option<String>> {
    let row = diesel::sql_query(
        "SELECT rendered_text, rendered_sha256
           FROM news_ai_delivery_card
          WHERE assessment_id = ?",
    )
    .bind::<Text, _>(assessment_id)
    .get_result::<PersistedDeliveryCardRow>(conn)
    .optional()?;
    row.map(|row| {
        let actual = hex::encode(Sha256::digest(row.rendered_text.as_bytes()));
        if row.rendered_text.trim().is_empty() || row.rendered_sha256 != actual {
            return Err(audit(format!(
                "frozen NewsAI card is invalid for assessment {assessment_id}"
            )));
        }
        Ok(row.rendered_text)
    })
    .transpose()
}

fn freeze_delivery_card(
    conn: &mut SqliteConnection,
    assessment_id: &str,
    rendered_text: &str,
) -> NewsAiAssessmentAuditResult<String> {
    if let Some(existing) = load_frozen_delivery_card(conn, assessment_id)? {
        return Ok(existing);
    }
    if rendered_text.trim().is_empty() {
        return Err(invalid("NewsAI delivery card is empty"));
    }
    let rendered_sha256 = hex::encode(Sha256::digest(rendered_text.as_bytes()));
    diesel::sql_query(
        "INSERT INTO news_ai_delivery_card (assessment_id, rendered_text, rendered_sha256)
         VALUES (?, ?, ?)",
    )
    .bind::<Text, _>(assessment_id)
    .bind::<Text, _>(rendered_text)
    .bind::<Text, _>(&rendered_sha256)
    .execute(conn)?;
    Ok(rendered_text.to_owned())
}

fn load_recovery_bytes(
    conn: &mut SqliteConnection,
    assessment_id: &str,
) -> NewsAiAssessmentAuditResult<Option<String>> {
    let row = diesel::sql_query(
        "SELECT schema_version, fact_snapshot, fact_snapshot_sha256
           FROM news_ai_delivery_recovery_snapshot
          WHERE assessment_id = ?",
    )
    .bind::<Text, _>(assessment_id)
    .get_result::<PersistedRecoverySnapshotRow>(conn)
    .optional()?;
    row.map(|row| {
        if row.schema_version != RECOVERY_SNAPSHOT_SCHEMA_VERSION {
            return Err(audit(format!(
                "NewsAI recovery snapshot schema is invalid for assessment {assessment_id}"
            )));
        }
        let actual = hex::encode(Sha256::digest(row.fact_snapshot.as_bytes()));
        if row.fact_snapshot_sha256 != actual {
            return Err(audit(format!(
                "NewsAI recovery snapshot hash is invalid for assessment {assessment_id}"
            )));
        }
        Ok(row.fact_snapshot)
    })
    .transpose()
}

fn load_recovery_identity(
    conn: &mut SqliteConnection,
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<Option<NewsAiIdentityV3>> {
    if !is_v3_format(&row.analysis_version)? {
        return Ok(None);
    }
    let bytes = load_recovery_bytes(conn, &row.assessment_id)?
        .ok_or_else(|| audit("v3 recovery envelope is missing"))?;
    let (identity, _) =
        NewsAiIdentityV3::decode_recovery(bytes.as_bytes()).map_err(|e| audit(e.to_string()))?;
    if identity.digest() != row.assessment_id
        || identity.digest() != row.source_identity_sha256
        || identity.storage_analysis_version() != row.analysis_version
    {
        return Err(audit("v3 recovery identity differs from audited row"));
    }
    Ok(Some(identity))
}

fn load_frozen_recovery_fact(
    conn: &mut SqliteConnection,
    assessment_id: &str,
) -> NewsAiAssessmentAuditResult<Option<AdmittedNewsFact>> {
    let row = load_by_assessment_id(conn, assessment_id)?
        .ok_or_else(|| audit("recovery assessment is missing"))?;
    let v3 = is_v3_format(&row.analysis_version)?;
    let Some(bytes) = load_recovery_bytes(conn, assessment_id)? else {
        return if v3 {
            Err(audit("v3 recovery envelope is missing"))
        } else {
            Ok(None)
        };
    };
    if v3 {
        load_recovery_identity(conn, &row)?;
        NewsAiIdentityV3::decode_recovery(bytes.as_bytes())
            .map(|(_, fact)| Some(fact))
            .map_err(|e| audit(e.to_string()))
    } else {
        AdmittedNewsFact::from_recovery_snapshot(bytes.as_bytes())
            .map(Some)
            .map_err(|e| audit(e.to_string()))
    }
}

fn freeze_recovery_fact(
    conn: &mut SqliteConnection,
    assessment_id: &str,
    fact: &crate::monitor::news_ai::AdmittedNewsFact,
) -> NewsAiAssessmentAuditResult<()> {
    if load_frozen_recovery_fact(conn, assessment_id)?.is_some() {
        return Ok(());
    }
    let snapshot = fact
        .recovery_snapshot_canonical()
        .map_err(|error| audit(format!("cannot freeze NewsAI recovery fact: {error}")))?;
    let snapshot = String::from_utf8(snapshot)
        .map_err(|error| audit(format!("NewsAI recovery fact is not UTF-8: {error}")))?;
    freeze_recovery_bytes(conn, assessment_id, &snapshot)
}

fn freeze_recovery_bytes(
    conn: &mut SqliteConnection,
    assessment_id: &str,
    snapshot: &str,
) -> NewsAiAssessmentAuditResult<()> {
    let snapshot_sha256 = hex::encode(Sha256::digest(snapshot.as_bytes()));
    let inserted = diesel::sql_query(
        "INSERT INTO news_ai_delivery_recovery_snapshot (
            assessment_id, schema_version, fact_snapshot, fact_snapshot_sha256
         ) VALUES (?, ?, ?, ?)",
    )
    .bind::<Text, _>(assessment_id)
    .bind::<Integer, _>(RECOVERY_SNAPSHOT_SCHEMA_VERSION)
    .bind::<Text, _>(snapshot)
    .bind::<Text, _>(&snapshot_sha256)
    .execute(conn)?;
    if inserted != 1 {
        return Err(audit(format!(
            "NewsAI recovery snapshot append affected {inserted} rows"
        )));
    }
    Ok(())
}

fn append_audited_news_ai_assessment_on_conn(
    conn: &mut SqliteConnection,
    request: crate::monitor::news_ai::NewsAiRequest,
    assessment: crate::monitor::news_ai::NewsAiAssessment,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::AuditedNewsAiAssessment> {
    let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment)?;
    conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
        let receipt = insert_assessment_in_transaction(conn, &input)?;
        let audited = crate::monitor::news_ai::AuditedNewsAiAssessment::try_from_assessment_audit(
            request,
            assessment,
            &receipt.record_hash,
        )
        .map_err(|error| audit(format!("fresh assessment delivery binding failed: {error}")))?;
        freeze_recovery_fact(
            conn,
            audited.delivery().assessment().assessment_id(),
            audited.delivery().fact(),
        )?;
        let card = freeze_delivery_card(
            conn,
            audited.delivery().assessment().assessment_id(),
            &audited.delivery().render_card(),
        )?;
        audited
            .with_frozen_card(card)
            .map_err(|error| audit(format!("freeze delivery card failed: {error}")))
    })
}

pub(crate) fn has_news_ai_assessment_for_fact_on_conn(
    conn: &mut SqliteConnection,
    fact: &crate::monitor::news_ai::AdmittedNewsFact,
    analysis_version: &str,
) -> NewsAiAssessmentAuditResult<bool> {
    let identity = source_identity_from_fact(fact, analysis_version)?;
    let assessment_id = core_assessment_id(&identity);
    validate_news_ai_assessment_chain(conn)?;
    Ok(load_by_assessment_id(conn, &assessment_id)?.is_some())
}

fn persisted_delivery_assessment(
    conn: &mut SqliteConnection,
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::PersistedNewsAiAssessment> {
    validate_persisted_row(conn, row)?;
    let confidence = u8::try_from(row.confidence).map_err(|error| {
        audit(format!(
            "persisted delivery confidence is invalid at row {}: {error}",
            row.id
        ))
    })?;
    let started_at = DateTime::parse_from_rfc3339(&row.model_started_at)
        .map_err(|error| audit(format!("persisted model start is invalid: {error}")))?
        .with_timezone(&Utc);
    let completed_at = DateTime::parse_from_rfc3339(&row.model_completed_at)
        .map_err(|error| audit(format!("persisted model completion is invalid: {error}")))?
        .with_timezone(&Utc);
    Ok(crate::monitor::news_ai::PersistedNewsAiAssessment {
        assessment_id: row.assessment_id.clone(),
        impact: NewsAiAuditImpact::parse(&row.impact)?.into_core(),
        confidence,
        uncertainty: row.uncertainty.clone(),
        core_logic: row.core_logic.clone(),
        input_evidence_sha256: row.input_evidence_sha256.clone(),
        normalized_prompt_sha256: row.normalized_prompt_sha256.clone(),
        receipt: crate::monitor::news_ai::PersistedModelCallReceipt {
            provider: row.model_provider.clone(),
            model: row.model.clone(),
            upstream_request_id: row.model_upstream_request_id.clone(),
            upstream_response_id: row.model_upstream_response_id.clone(),
            system_sha256: row.model_system_sha256.clone(),
            user_sha256: row.model_user_sha256.clone(),
            response_sha256: row.model_response_sha256.clone(),
            started_at,
            completed_at,
        },
    })
}

pub(crate) fn load_audited_news_ai_assessment_for_fact_on_conn(
    conn: &mut SqliteConnection,
    fact: &crate::monitor::news_ai::AdmittedNewsFact,
    analysis_version: &str,
) -> NewsAiAssessmentAuditResult<Option<crate::monitor::news_ai::AuditedNewsAiAssessment>> {
    let identity = source_identity_from_fact(fact, analysis_version)?;
    let assessment_id = core_assessment_id(&identity);
    validate_news_ai_assessment_chain(conn)?;
    let Some(row) = load_by_assessment_id(conn, &assessment_id)? else {
        return Ok(None);
    };
    let link = load_chain_for_row(conn, row.id)?;
    let persisted = persisted_delivery_assessment(conn, &row)?;
    let audited =
        crate::monitor::news_ai::AuditedNewsAiAssessment::try_from_persisted_assessment_audit(
            fact.clone(),
            analysis_version,
            persisted,
            &link.record_hash,
        )
        .map_err(|error| {
            audit(format!(
                "persisted assessment delivery binding failed: {error}"
            ))
        })?;
    let Some(card) = load_frozen_delivery_card(conn, &assessment_id)? else {
        if let Some(event) = latest_delivery_event(conn, &assessment_id)? {
            if matches!(
                DeliveryEventState::parse(&event.state)?,
                DeliveryEventState::SinkStarted | DeliveryEventState::PostSinkRecovery
            ) {
                return Err(audit(format!(
                    "NewsAI assessment {assessment_id} has prior sink state without a frozen delivery card"
                )));
            }
        }
        return Ok(Some(audited));
    };
    audited
        .with_frozen_card(card)
        .map(Some)
        .map_err(|error| audit(format!("persisted delivery card binding failed: {error}")))
}

fn load_audited_news_ai_assessment_for_identity_on_conn(
    conn: &mut SqliteConnection,
    identity: &NewsAiIdentityV3,
) -> NewsAiAssessmentAuditResult<Option<crate::monitor::news_ai::AuditedNewsAiAssessment>> {
    validate_news_ai_assessment_chain(conn)?;
    let Some(row) = load_by_assessment_id(conn, &identity.digest())? else {
        return Ok(None);
    };
    if load_recovery_identity(conn, &row)?.as_ref() != Some(identity) {
        return Err(audit("retained v3 identity differs from qualified lookup"));
    }
    let fact = load_frozen_recovery_fact(conn, &row.assessment_id)?
        .ok_or_else(|| audit("v3 original fact missing"))?;
    let link = load_chain_for_row(conn, row.id)?;
    let persisted = persisted_delivery_assessment(conn, &row)?;
    let card = load_frozen_delivery_card(conn, &row.assessment_id)?
        .ok_or_else(|| audit("v3 original card missing"))?;
    crate::monitor::news_ai::AuditedNewsAiAssessment::try_from_persisted_identity_audit(
        fact,
        identity.analysis_version(),
        Some(identity),
        persisted,
        &link.record_hash,
    )
    .and_then(|audited| audited.with_frozen_card(card))
    .map(Some)
    .map_err(|e| audit(e.to_string()))
}

fn news_ai_assessment_is_pending(
    latest: Option<&PersistedDeliveryEventRow>,
) -> NewsAiAssessmentAuditResult<bool> {
    let Some(latest) = latest else {
        return Ok(true);
    };
    let state = DeliveryEventState::parse(&latest.state)?;
    Ok(match state {
        DeliveryEventState::PredictionLinked => false,
        DeliveryEventState::RolledBack => !latest
            .reason
            .as_deref()
            .is_some_and(is_counted_terminal_denial_reason),
        DeliveryEventState::Reserved
        | DeliveryEventState::SinkStarted
        | DeliveryEventState::Delivered
        | DeliveryEventState::PostSinkRecovery => true,
    })
}

pub(crate) fn load_pending_news_ai_recoveries_on_conn(
    conn: &mut SqliteConnection,
    limit: usize,
) -> NewsAiAssessmentAuditResult<Vec<NewsAiPendingRecovery>> {
    if limit == 0 || limit > 100 {
        return Err(invalid(
            "pending NewsAI recovery limit must be within 1..=100",
        ));
    }
    conn.immediate_transaction(|conn| claim_pending_news_ai_recoveries(conn, limit))
}

fn claim_pending_news_ai_recoveries(
    conn: &mut SqliteConnection,
    limit: usize,
) -> NewsAiAssessmentAuditResult<Vec<NewsAiPendingRecovery>> {
    validate_news_ai_assessment_chain(conn)?;
    validate_news_ai_delivery_audit(conn)?;
    let claims = diesel::sql_query(
        "SELECT id, assessment_id, category, reason FROM news_ai_recovery_claim ORDER BY id",
    )
    .load::<RecoveryClaimRow>(conn)?;
    let mut manual_next = claims.last().is_some_and(|claim| claim.category == "ready");
    let last_visits: BTreeMap<_, _> = claims
        .iter()
        .map(|claim| (claim.assessment_id.clone(), claim.id))
        .collect();
    let notified: std::collections::BTreeSet<_> = diesel::sql_query(
        "SELECT c.id, c.assessment_id, c.category, c.reason
           FROM news_ai_recovery_review_notified n
           JOIN news_ai_recovery_claim c ON c.id = n.claim_id",
    )
    .load::<RecoveryClaimRow>(conn)?
    .into_iter()
    .map(|claim| (claim.assessment_id, claim.reason))
    .collect();

    let mut ready = Vec::new();
    let mut manual = Vec::new();
    for row in load_rows(conn)? {
        if NewsAiAuditImpact::parse(&row.impact)? == NewsAiAuditImpact::Neutral {
            continue;
        }
        let latest = latest_delivery_event(conn, &row.assessment_id)?;
        if !news_ai_assessment_is_pending(latest.as_ref())? {
            continue;
        }
        let last_visit = last_visits.get(&row.assessment_id).copied().unwrap_or(0);
        let order = (last_visit, row.id);
        let Some(fact) = load_frozen_recovery_fact(conn, &row.assessment_id)? else {
            let reason = "legacy assessment has no immutable recovery snapshot".to_owned();
            if notified.contains(&(row.assessment_id.clone(), reason.clone())) {
                continue;
            }
            manual.push((
                order,
                NewsAiPendingRecovery::ManualReview {
                    assessment_id: row.assessment_id.clone(),
                    reason,
                    claim_id: 0,
                },
            ));
            continue;
        };
        let Some(card) = load_frozen_delivery_card(conn, &row.assessment_id)? else {
            let reason = "legacy assessment has no immutable delivery card".to_owned();
            if notified.contains(&(row.assessment_id.clone(), reason.clone())) {
                continue;
            }
            manual.push((
                order,
                NewsAiPendingRecovery::ManualReview {
                    assessment_id: row.assessment_id.clone(),
                    reason,
                    claim_id: 0,
                },
            ));
            continue;
        };
        let business_identity = load_recovery_identity(conn, &row)?;
        let expected_id = if let Some(identity) = &business_identity {
            identity.digest()
        } else {
            core_assessment_id(&source_identity_from_fact(&fact, &row.analysis_version)?)
        };
        if expected_id != row.assessment_id {
            return Err(audit(format!(
                "NewsAI recovery snapshot identity mismatch for assessment {}",
                row.assessment_id
            )));
        }
        let link = load_chain_for_row(conn, row.id)?;
        let persisted = persisted_delivery_assessment(conn, &row)?;
        let audited =
            crate::monitor::news_ai::AuditedNewsAiAssessment::try_from_persisted_identity_audit(
                fact,
                business_identity
                    .as_ref()
                    .map(NewsAiIdentityV3::analysis_version)
                    .unwrap_or(&row.analysis_version),
                business_identity.as_ref(),
                persisted,
                &link.record_hash,
            )
            .map_err(|error| {
                audit(format!(
                    "pending NewsAI recovery binding failed for assessment {}: {error}",
                    row.assessment_id
                ))
            })?
            .with_frozen_card(card)
            .map_err(|error| {
                audit(format!(
                    "pending NewsAI delivery card binding failed for assessment {}: {error}",
                    row.assessment_id
                ))
            })?;
        ready.push((order, NewsAiPendingRecovery::Ready(audited)));
    }
    ready.sort_by_key(|(order, _)| *order);
    manual.sort_by_key(|(order, _)| *order);
    let mut ready = ready.into_iter();
    let mut manual = manual.into_iter();
    let mut selected = Vec::new();
    while selected.len() < limit {
        let next = if manual_next {
            manual.next().or_else(|| ready.next())
        } else {
            ready.next().or_else(|| manual.next())
        };
        let Some((_, mut work)) = next else { break };
        let (assessment_id, category, reason) = match &work {
            NewsAiPendingRecovery::Ready(audited) => {
                (audited.delivery().assessment().assessment_id(), "ready", "")
            }
            NewsAiPendingRecovery::ManualReview {
                assessment_id,
                reason,
                ..
            } => (assessment_id.as_str(), "manual", reason.as_str()),
        };
        let claim = diesel::sql_query(
            "INSERT INTO news_ai_recovery_claim (assessment_id, category, reason)
             VALUES (?, ?, ?) RETURNING id, assessment_id, category, reason",
        )
        .bind::<Text, _>(assessment_id)
        .bind::<Text, _>(category)
        .bind::<Text, _>(reason)
        .get_result::<RecoveryClaimRow>(conn)?;
        manual_next = category == "ready";
        if let NewsAiPendingRecovery::ManualReview { claim_id, .. } = &mut work {
            *claim_id = claim.id;
        }
        selected.push(work);
    }
    Ok(selected)
}

fn confirm_news_ai_recovery_review_on_conn(
    conn: &mut SqliteConnection,
    claim_id: i64,
) -> NewsAiAssessmentAuditResult<()> {
    // The FK and manual-only trigger reject nonexistent/ready claims. An
    // already-confirmed exact claim is idempotent; it never closes delivery.
    diesel::sql_query(
        "INSERT OR IGNORE INTO news_ai_recovery_review_notified (claim_id) VALUES (?)",
    )
    .bind::<BigInt, _>(claim_id)
    .execute(conn)?;
    Ok(())
}

impl DatabaseManager {
    pub fn load_audited_news_ai_assessment_for_identity(
        &self,
        identity: &NewsAiIdentityV3,
    ) -> NewsAiAssessmentAuditResult<Option<crate::monitor::news_ai::AuditedNewsAiAssessment>> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        load_audited_news_ai_assessment_for_identity_on_conn(&mut conn, identity)
    }

    pub fn is_news_ai_terminal_denial_for_identity(
        &self,
        identity: &NewsAiIdentityV3,
    ) -> NewsAiAssessmentAuditResult<bool> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        validate_news_ai_assessment_chain(&mut conn)?;
        let latest = latest_delivery_event(&mut conn, &identity.digest())?;
        Ok(latest.is_some_and(|event| {
            event.assessment_id == identity.digest()
                && event.state == DeliveryEventState::RolledBack.as_str()
                && event
                    .reason
                    .as_deref()
                    .is_some_and(is_counted_terminal_denial_reason)
        }))
    }

    /// Check the durable exact BR-172 identity before making another model
    /// call. The complete chain is validated first; a corrupt audit can never
    /// masquerade as a successful deduplication hit.
    pub fn has_news_ai_assessment_for_fact(
        &self,
        fact: &crate::monitor::news_ai::AdmittedNewsFact,
        analysis_version: &str,
    ) -> NewsAiAssessmentAuditResult<bool> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        has_news_ai_assessment_for_fact_on_conn(&mut conn, fact, analysis_version)
    }

    pub fn append_news_ai_assessment(
        &self,
        input: &NewsAiAssessmentAuditInput,
    ) -> NewsAiAssessmentAuditResult<NewsAiAssessmentAuditReceipt> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        append_news_ai_assessment_on_conn(&mut conn, input)
    }

    /// Append the receipt-bearing model result and mint the only capability
    /// accepted by the governed delivery state machine.
    pub fn append_audited_news_ai_assessment(
        &self,
        request: crate::monitor::news_ai::NewsAiRequest,
        assessment: crate::monitor::news_ai::NewsAiAssessment,
    ) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::AuditedNewsAiAssessment> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment)
    }

    pub fn load_audited_news_ai_assessment_for_fact(
        &self,
        fact: &crate::monitor::news_ai::AdmittedNewsFact,
        analysis_version: &str,
    ) -> NewsAiAssessmentAuditResult<Option<crate::monitor::news_ai::AuditedNewsAiAssessment>> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        load_audited_news_ai_assessment_for_fact_on_conn(&mut conn, fact, analysis_version)
    }

    /// Recover bounded, nonterminal NewsAI work from immutable persisted
    /// evidence, atomically appending scheduling claims. Claims rotate ready
    /// and manual work but never mark delivery or manual notification complete.
    pub fn load_pending_news_ai_recoveries(
        &self,
        limit: usize,
    ) -> NewsAiAssessmentAuditResult<Vec<NewsAiPendingRecovery>> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        load_pending_news_ai_recoveries_on_conn(&mut conn, limit)
    }

    /// Call only after the manual-review audit publication succeeds. A crash
    /// before this append leaves the item retryable (at-least-once notice).
    pub fn confirm_news_ai_recovery_review(
        &self,
        claim_id: i64,
    ) -> NewsAiAssessmentAuditResult<()> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        confirm_news_ai_recovery_review_on_conn(&mut conn, claim_id)
    }

    pub fn is_news_ai_terminal_denial_for_fact(
        &self,
        fact: &crate::monitor::news_ai::AdmittedNewsFact,
        analysis_version: &str,
    ) -> NewsAiAssessmentAuditResult<bool> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        is_news_ai_terminal_denial_for_fact_on_conn(&mut conn, fact, analysis_version)
    }

    pub fn reserve_news_ai_delivery(
        &self,
        delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
    ) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiReserveOutcome> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
            reserve_news_ai_delivery_on_conn(conn, delivery)
        })
    }

    pub fn begin_news_ai_sink_attempt(
        &self,
        delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
        reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
    ) -> NewsAiAssessmentAuditResult<()> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
            begin_news_ai_sink_attempt_on_conn(conn, delivery, reservation)
        })
    }

    pub fn record_news_ai_delivered(
        &self,
        delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
        reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
        authoritative_delivery_audit: &crate::event::PersistedDeliveryAuditReceipt,
    ) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiDeliveryAuditReceipt> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
            record_news_ai_delivered_on_conn(
                conn,
                delivery,
                reservation,
                authoritative_delivery_audit.envelope_id(),
            )
        })
    }

    pub fn link_news_ai_prediction(
        &self,
        delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
        reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
        delivery_audit: &crate::monitor::news_ai::NewsAiDeliveryAuditReceipt,
    ) -> NewsAiAssessmentAuditResult<crate::monitor::news_ai::NewsAiPredictionLinkReceipt> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
            link_news_ai_prediction_on_conn(conn, delivery, reservation, delivery_audit)
        })
    }

    pub fn rollback_news_ai_delivery(
        &self,
        delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
        reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
        reason: &str,
    ) -> NewsAiAssessmentAuditResult<()> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
            rollback_news_ai_delivery_on_conn(conn, delivery, reservation, reason)
        })
    }

    pub fn record_news_ai_post_sink_recovery(
        &self,
        delivery: &crate::monitor::news_ai::GovernedNewsAiDelivery,
        reservation: &crate::monitor::news_ai::NewsAiDeliveryReservation,
        authoritative_delivery_audit: Option<&crate::event::PersistedDeliveryAuditReceipt>,
        reason: &str,
    ) -> NewsAiAssessmentAuditResult<()> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
            record_news_ai_post_sink_recovery_on_conn(
                conn,
                delivery,
                reservation,
                authoritative_delivery_audit.map(|receipt| receipt.envelope_id()),
                reason,
            )
        })
    }

    pub fn validate_news_ai_assessment_audit(&self) -> NewsAiAssessmentAuditResult<String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        validate_news_ai_assessment_chain(&mut conn)
    }

    pub fn validate_news_ai_delivery_audit(&self) -> NewsAiAssessmentAuditResult<String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        validate_news_ai_delivery_audit(&mut conn)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::market_domain::ProviderId;
    use crate::market_domain::SourceEvidence;
    use crate::monitor::news_ai::{NewsAiChainContext, NewsAiReserveOutcome};
    use chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
    use diesel::connection::SimpleConnection;

    #[derive(Debug, QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    pub(crate) fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("NewsAI assessment schema");
        conn
    }

    fn input() -> NewsAiAssessmentAuditInput {
        let timezone = FixedOffset::east_opt(8 * 3600).expect("UTC+08:00");
        NewsAiAssessmentAuditInput {
            business_identity: None,
            recovery_envelope: None,
            assessment_id: "ed656daca371c716f27357956a9c9778e57bc2d2c7150ac5c51fb661aae8ec73"
                .to_owned(),
            impact: NewsAiAuditImpact::Positive,
            confidence: 82,
            uncertainty: "TEST_CODE contract execution may vary".to_owned(),
            core_logic: "TEST_CODE source-bound contract plus admitted market evidence".to_owned(),
            input_evidence_sha256:
                "1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            normalized_prompt_sha256:
                "2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
            source_provider: "cailianpress".to_owned(),
            source_batch_id: "TEST_CODE_NEWS_BATCH_001".to_owned(),
            source_item_id: "TEST_CODE_NEWS_ITEM_001".to_owned(),
            analysis_version: "TEST_CODE_NEWS_AI_V1".to_owned(),
            target_code: "TEST_CODE_600519".to_owned(),
            model_provider: "TEST_CODE_MODEL_PROVIDER".to_owned(),
            model: "TEST_CODE_MODEL_V1".to_owned(),
            model_upstream_request_id: Some("TEST_CODE_REQUEST_001".to_owned()),
            model_upstream_response_id: "TEST_CODE_RESPONSE_001".to_owned(),
            model_system_sha256: "4444444444444444444444444444444444444444444444444444444444444444"
                .to_owned(),
            model_user_sha256: "2222222222222222222222222222222222222222222222222222222222222222"
                .to_owned(),
            model_response_sha256:
                "3333333333333333333333333333333333333333333333333333333333333333".to_owned(),
            model_started_at: timezone
                .with_ymd_and_hms(2026, 7, 27, 9, 30, 0)
                .single()
                .expect("start"),
            model_completed_at: timezone
                .with_ymd_and_hms(2026, 7, 27, 9, 30, 1)
                .single()
                .expect("completion"),
        }
    }

    fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
        diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
            .get_result::<CountRow>(conn)
            .expect("count")
            .count
    }

    fn observed(value: DateTime<Utc>) -> String {
        format!(
            "{}.{:09}",
            value.timestamp(),
            value.timestamp_subsec_nanos()
        )
    }

    fn core_assessment() -> (
        crate::monitor::news_ai::NewsAiRequest,
        crate::monitor::news_ai::NewsAiAssessment,
    ) {
        core_assessment_for("TEST_CODE_NEWS_ITEM_CORE", "TEST_CODE_600519")
    }

    /// Same admitted market evidence and model response as `core_assessment`,
    /// with the source item and target ticket selectable so a test can build a
    /// second, distinct NewsAI identity for the same ticket (F5b cooldown
    /// evidence) or for another ticket.
    pub(crate) fn core_assessment_for(
        item_id: &str,
        target_code: &str,
    ) -> (
        crate::monitor::news_ai::NewsAiRequest,
        crate::monitor::news_ai::NewsAiAssessment,
    ) {
        use crate::data_gateway::{BatchEvidence, GlobalNewsRecord};
        use crate::monitor::news_ai::{
            AdmittedNewsFact, ModelCallReceipt, NewsAiAssessment, NewsAiRequest, NewsMarketContext,
            NewsMarketEvidenceInput, NewsMarketSnapshot, SettledDailyBarInput,
        };

        let observed_at = Utc
            .with_ymd_and_hms(2026, 7, 27, 1, 0, 3)
            .single()
            .expect("observation");
        let published_at = Utc
            .with_ymd_and_hms(2026, 7, 27, 1, 0, 0)
            .single()
            .expect("publication");
        let news_batch = BatchEvidence {
            provider: ProviderId::Cailianpress,
            source: "cls-v1".to_owned(),
            source_at: Some(published_at.to_rfc3339()),
            observed_at: observed(observed_at),
            batch_id: "TEST_CODE_NEWS_BATCH_CORE".to_owned(),
        };
        // The fact seam requires the item to name the target instrument.
        let instrument = target_code
            .strip_prefix("TEST_CODE_")
            .unwrap_or(target_code)
            .to_owned();
        let record = GlobalNewsRecord {
            item_id: item_id.to_owned(),
            title: "TEST_CODE exact source-bound contract".to_owned(),
            summary: Some("TEST_CODE disclosed contract evidence".to_owned()),
            content: None,
            publisher: "TEST_CODE publisher".to_owned(),
            canonical_url: format!("https://example.com/{item_id}"),
            published_at,
            observed_at,
            instruments: vec![instrument],
            topics: vec!["TEST_CODE contract".to_owned()],
            language: "zh-CN".to_owned(),
            evidence: SourceEvidence::new(
                ProviderId::Cailianpress,
                observed(observed_at),
                "TEST_CODE_NEWS_BATCH_CORE",
            )
            .expect("source evidence")
            .with_source_at(published_at.to_rfc3339())
            .expect("source time"),
        };
        let fact = AdmittedNewsFact::from_global(&record, &news_batch, target_code)
            .expect("admitted fact");
        let latest = NaiveDate::from_ymd_opt(2026, 7, 24).expect("latest trading day");
        let daily_bars = (0..20)
            .map(|index| SettledDailyBarInput {
                date: latest - chrono::Duration::days(index),
                close: 100.0 - index as f64,
                volume: 1_000_000.0 + index as f64,
                settled: true,
            })
            .collect();
        let market = NewsMarketSnapshot::try_from_input(NewsMarketEvidenceInput {
            target_code: target_code.to_owned(),
            context: NewsMarketContext::PostClose,
            as_of: observed_at,
            latest_completed_trading_day: latest,
            daily_bars,
            daily_evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_tdx-bars".to_owned(),
                source_at: Some("2026-07-24T07:00:00Z".to_owned()),
                observed_at: observed(observed_at),
                batch_id: "TEST_CODE_DAILY_BATCH_CORE".to_owned(),
            },
            quote: None,
        })
        .expect("market snapshot");
        let request = NewsAiRequest::try_new(
            fact,
            market,
            Vec::new(),
            "TEST_CODE_NEWS_AI_CORE_V1",
            NewsAiChainContext::default(),
        )
        .expect("NewsAI request");
        let response = r#"{"impact":"positive","confidence":82,"uncertainty":"TEST_CODE execution may vary","core_logic":"TEST_CODE evidence-bound positive impact"}"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_MODEL_PROVIDER",
            "TEST_CODE_MODEL_V1",
            Some("TEST_CODE_REQUEST_CORE"),
            request.normalized_prompt(),
            response,
            observed_at,
            observed_at + chrono::Duration::seconds(1),
        )
        .expect("model receipt");
        let assessment = NewsAiAssessment::from_model_response(&request, response, Some(receipt))
            .expect("NewsAI assessment");
        (request, assessment)
    }

    #[test]
    fn assessment_append_round_trips_through_the_public_connection_seam() {
        let mut conn = connection();
        let receipt = append_news_ai_assessment_on_conn(&mut conn, &input())
            .expect("append assessment audit");

        assert!(receipt.inserted);
        assert_eq!(
            receipt.assessment_id,
            "ed656daca371c716f27357956a9c9778e57bc2d2c7150ac5c51fb661aae8ec73"
        );
        assert_eq!(receipt.record_hash.len(), 64);
        validate_news_ai_assessment_chain(&mut conn).expect("valid assessment chain");
    }

    fn legacy_golden_dump(conn: &mut SqliteConnection) -> String {
        #[derive(QueryableByName)]
        struct Name {
            #[diesel(sql_type = Text)]
            name: String,
        }
        #[derive(QueryableByName)]
        struct Statement {
            #[diesel(sql_type = Text)]
            statement: String,
        }
        let mut sql = String::new();
        for table in [
            "news_ai_assessment",
            "news_ai_assessment_chain",
            "news_ai_delivery_recovery_snapshot",
            "news_ai_delivery_card",
            "news_ai_delivery_event",
            "news_ai_delivery_event_chain",
        ] {
            let names = diesel::sql_query(format!("PRAGMA table_info({table})"))
                .load::<Name>(conn)
                .unwrap();
            let columns = names
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let values = names
                .iter()
                .map(|n| format!("quote({})", n.name))
                .collect::<Vec<_>>()
                .join(" || ',' || ");
            let query = format!("SELECT 'INSERT INTO {table} ({columns}) VALUES (' || {values} || ');' AS statement FROM {table} WHERE rowid <= 2 ORDER BY rowid");
            for row in diesel::sql_query(query).load::<Statement>(conn).unwrap() {
                sql.push_str(&row.statement);
                sql.push('\n');
            }
        }
        sql
    }

    #[test]
    fn v3_legacy_golden_reopen_and_mixed_recovery_keep_original_bytes_and_fairness() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/news_ai_legacy_v1_v2.json")).unwrap();
        let sql = fixture["sql"].as_str().unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let mut conn = SqliteConnection::establish(path).unwrap();
        create_schema(&mut conn).unwrap();
        conn.batch_execute(sql).unwrap();
        create_schema(&mut conn).unwrap();
        assert_eq!(legacy_golden_dump(&mut conn), sql);
        for (index, version) in ["news_ai_v1", "news_ai_v2"].into_iter().enumerate() {
            let (old, _) = core_assessment();
            let request = crate::monitor::news_ai::NewsAiRequest::try_new(
                old.fact().clone(),
                old.market().clone(),
                vec![],
                version,
                NewsAiChainContext::default(),
            )
            .unwrap();
            assert_eq!(
                request.normalized_prompt(),
                fixture["prompts"][index].as_str().unwrap()
            );
        }
        let (request, assessment) = v3_core_assessment();
        let new_id = assessment.assessment_id().to_owned();
        append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).unwrap();
        append_news_ai_assessment_on_conn(&mut conn, &input()).unwrap(); // legacy manual, no invented fact
        drop(conn);
        let mut conn = SqliteConnection::establish(path).unwrap();
        create_schema(&mut conn).unwrap();
        assert_eq!(legacy_golden_dump(&mut conn), sql);
        let mut ready = std::collections::BTreeSet::new();
        let mut manual = 0;
        for _ in 0..8 {
            let work = load_pending_news_ai_recoveries_on_conn(&mut conn, 1).unwrap();
            assert_eq!(work.len(), 1);
            match &work[0] {
                NewsAiPendingRecovery::Ready(audited) => {
                    ready.insert(audited.delivery().identity().sha256().to_owned());
                }
                NewsAiPendingRecovery::ManualReview { .. } => manual += 1,
            }
        }
        assert_eq!(ready.len(), 3);
        assert!(ready.contains(&new_id));
        assert!(manual > 0);
        assert_eq!(legacy_golden_dump(&mut conn), sql);
    }

    fn v3_core_assessment() -> (
        crate::monitor::news_ai::NewsAiRequest,
        crate::monitor::news_ai::NewsAiAssessment,
    ) {
        use crate::monitor::news_ai::{
            ModelCallReceipt, NewsAiAnalysisProfile, NewsAiAssessment, NewsAiIdentityV3,
            NewsAiRequest,
        };
        let (old, _) = core_assessment();
        let profile = NewsAiAnalysisProfile::for_configured_model(
            "TEST_CODE_MODEL_PROVIDER",
            "TEST_CODE_configured_model",
        )
        .unwrap();
        let identity = NewsAiIdentityV3::from_fact(old.fact(), &profile).unwrap();
        let request = NewsAiRequest::try_new_v3(
            old.fact().clone(),
            old.market().clone(),
            vec![],
            identity,
            NewsAiChainContext::default(),
        )
        .unwrap();
        let response = r#"{"impact":"positive","confidence":82,"uncertainty":"TEST_CODE execution may vary","core_logic":"TEST_CODE evidence-bound positive impact"}"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_MODEL_PROVIDER",
            "TEST_CODE_actual_model",
            Some("TEST_CODE_REQUEST_CORE"),
            request.normalized_prompt(),
            response,
            Utc.with_ymd_and_hms(2026, 7, 27, 1, 0, 3).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 27, 1, 0, 4).unwrap(),
        )
        .unwrap();
        let assessment =
            NewsAiAssessment::from_model_response(&request, response, Some(receipt)).unwrap();
        (request, assessment)
    }

    #[test]
    fn v3_append_freezes_audited_identity_and_recovers_original_materials() {
        let mut conn = connection();
        let (request, assessment) = v3_core_assessment();
        let id = assessment.assessment_id().to_owned();
        let audited =
            append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).unwrap();
        assert_eq!(audited.delivery().identity().sha256(), id);
        assert_eq!(
            audited.delivery().assessment().receipt().model(),
            "TEST_CODE_actual_model"
        );
        let pending = load_pending_news_ai_recoveries_on_conn(&mut conn, 1).unwrap();
        let NewsAiPendingRecovery::Ready(recovered) = &pending[0] else {
            panic!("v3 must recover ready")
        };
        assert_eq!(
            recovered.delivery().render_card(),
            audited.delivery().render_card()
        );
        assert_eq!(count(&mut conn, "news_ai_assessment"), 1);
    }

    #[test]
    fn v3_every_append_stage_rolls_back_and_raw_append_cannot_create_partial_material() {
        for table in [
            "news_ai_assessment",
            "news_ai_delivery_recovery_snapshot",
            "news_ai_assessment_chain",
            "news_ai_delivery_card",
        ] {
            let mut conn = connection();
            conn.batch_execute(&format!("CREATE TRIGGER TEST_CODE_abort BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'TEST_CODE fault'); END;")).unwrap();
            let (request, assessment) = v3_core_assessment();
            assert!(
                append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).is_err(),
                "{table}"
            );
            for retained in [
                "news_ai_assessment",
                "news_ai_delivery_recovery_snapshot",
                "news_ai_assessment_chain",
                "news_ai_delivery_card",
            ] {
                assert_eq!(
                    count(&mut conn, retained),
                    0,
                    "failure at {table} retained {retained}"
                );
            }
        }
        let mut conn = connection();
        let (request, assessment) = v3_core_assessment();
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        assert!(append_news_ai_assessment_on_conn(&mut conn, &input).is_err());
        assert_eq!(count(&mut conn, "news_ai_assessment"), 0);
    }

    #[test]
    fn v3_missing_tampered_moved_unknown_material_fails_closed_before_claims() {
        for scenario in [
            "missing",
            "revision",
            "observation",
            "unknown",
            "move",
            "card",
            "format",
        ] {
            let mut conn = connection();
            let (request, assessment) = v3_core_assessment();
            let id = assessment.assessment_id().to_owned();
            append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).unwrap();
            // First prove normal UPDATE/DELETE are denied by existing immutable guards.
            assert!(diesel::sql_query(
                "DELETE FROM news_ai_delivery_recovery_snapshot WHERE assessment_id=?"
            )
            .bind::<Text, _>(&id)
            .execute(&mut conn)
            .is_err());
            assert!(diesel::sql_query("UPDATE news_ai_delivery_recovery_snapshot SET fact_snapshot='{}' WHERE assessment_id=?").bind::<Text,_>(&id).execute(&mut conn).is_err());
            #[derive(QueryableByName)]
            struct TriggerName {
                #[diesel(sql_type=Text)]
                name: String,
            }
            // Privileged corruption fixture: bypass guards, then require audit
            // validation to reject even when the untrusted snapshot SHA is repaired.
            let triggers = diesel::sql_query("SELECT name FROM sqlite_master WHERE type='trigger' AND tbl_name IN ('news_ai_delivery_recovery_snapshot','news_ai_delivery_card','news_ai_assessment')").load::<TriggerName>(&mut conn).unwrap();
            for trigger in triggers {
                conn.batch_execute(&format!("DROP TRIGGER {}", trigger.name))
                    .unwrap();
            }
            match scenario {
                "missing" => {
                    conn.batch_execute("DELETE FROM news_ai_delivery_recovery_snapshot")
                        .unwrap();
                }
                "move" => {
                    append_news_ai_assessment_on_conn(&mut conn, &input()).unwrap();
                    diesel::sql_query(
                        "UPDATE news_ai_delivery_recovery_snapshot SET assessment_id=?",
                    )
                    .bind::<Text, _>(input().assessment_id)
                    .execute(&mut conn)
                    .unwrap();
                }
                "card" => {
                    conn.batch_execute("DELETE FROM news_ai_delivery_card")
                        .unwrap();
                }
                "format" => {
                    conn.batch_execute("UPDATE news_ai_assessment SET analysis_version='news_ai_identity_v4/news_ai_v2'").unwrap();
                }
                _ => {
                    let original = load_recovery_bytes(&mut conn, &id).unwrap().unwrap();
                    let modified = match scenario {
                        "revision" => original.replace(
                            "TEST_CODE exact source-bound contract",
                            "TEST_CODE tampered text",
                        ),
                        "observation" => original
                            .replace("1785114003.000000000", "1785114004.000000000")
                            .replace("2026-07-27T01:00:03Z", "2026-07-27T01:00:04Z"),
                        _ => original.replace("\"identity_version\":3", "\"identity_version\":4"),
                    };
                    assert_ne!(
                        original, modified,
                        "tamper fixture must change bytes: {scenario}"
                    );
                    let repaired_sha = hex::encode(Sha256::digest(modified.as_bytes()));
                    diesel::sql_query("UPDATE news_ai_delivery_recovery_snapshot SET fact_snapshot=?,fact_snapshot_sha256=? WHERE assessment_id=?")
                        .bind::<Text,_>(modified).bind::<Text,_>(repaired_sha).bind::<Text,_>(&id).execute(&mut conn).unwrap();
                }
            }
            assert!(
                load_pending_news_ai_recoveries_on_conn(&mut conn, 1).is_err(),
                "{scenario}"
            );
            assert_eq!(count(&mut conn, "news_ai_recovery_claim"), 0, "{scenario}");
        }
    }

    #[tokio::test]
    async fn v3_same_content_across_batches_skips_second_model_and_market_call_after_reopen() {
        use crate::llm::{LlmError, LlmProvider, ReceiptBearingJson};
        use crate::monitor::news_ai::{NewsAIAnalyzer, NewsAiRequest};
        use std::{
            cell::RefCell,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
        };
        struct CountingProvider(Arc<AtomicUsize>);
        #[async_trait::async_trait]
        impl LlmProvider for CountingProvider {
            fn name(&self) -> &'static str {
                "TEST_CODE_MODEL_PROVIDER"
            }
            fn model(&self) -> &str {
                "TEST_CODE_configured_model"
            }
            async fn chat_json(&self, _: &str, _: &str) -> Result<serde_json::Value, LlmError> {
                unreachable!("receipt-only")
            }
            async fn chat_json_with_receipt(
                &self,
                system: &str,
                user: &str,
            ) -> Result<ReceiptBearingJson, LlmError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(ReceiptBearingJson::test_fixture(
                    self.name(),
                    "TEST_CODE_actual_model",
                    Some("TEST_CODE_request"),
                    "TEST_CODE_response",
                    system,
                    user,
                    r#"{"impact":"positive","confidence":82,"uncertainty":"TEST_CODE uncertainty","core_logic":"TEST_CODE evidence"}"#,
                    Utc.with_ymd_and_hms(2026, 7, 27, 1, 0, 4).unwrap(),
                    Utc.with_ymd_and_hms(2026, 7, 27, 1, 0, 5).unwrap(),
                ))
            }
        }
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let mut conn = SqliteConnection::establish(path).unwrap();
        create_schema(&mut conn).unwrap();
        let conn = RefCell::new(conn);
        let calls = Arc::new(AtomicUsize::new(0));
        let acquired = AtomicUsize::new(0);
        let analyzer = NewsAIAnalyzer::new(Arc::new(CountingProvider(calls.clone())));
        let (old, _) = core_assessment();
        let profile = analyzer.identity_profile().unwrap();
        let identity = NewsAiIdentityV3::from_fact(old.fact(), &profile).unwrap();
        let first = analyzer
            .assess_if_absent(
                identity.clone(),
                |identity| {
                    std::future::ready(
                        load_audited_news_ai_assessment_for_identity_on_conn(
                            &mut conn.borrow_mut(),
                            &identity,
                        )
                        .map(|v| v.is_some())
                        .map_err(|e| e.to_string()),
                    )
                },
                |identity| async {
                    acquired.fetch_add(1, Ordering::SeqCst);
                    NewsAiRequest::try_new_v3(
                        old.fact().clone(),
                        old.market().clone(),
                        vec![],
                        identity,
                        NewsAiChainContext::default(),
                    )
                    .map_err(|e| e.to_string())
                },
            )
            .await
            .unwrap()
            .unwrap();
        let audited =
            append_audited_news_ai_assessment_on_conn(&mut conn.borrow_mut(), first.0, first.1)
                .unwrap();
        let frozen = load_recovery_bytes(&mut conn.borrow_mut(), &identity.digest())
            .unwrap()
            .unwrap();
        let original_card = audited.delivery().render_card();
        let original_fact =
            String::from_utf8(old.fact().recovery_snapshot_canonical().unwrap()).unwrap();
        let other_batch = original_fact
            .replace("TEST_CODE_NEWS_BATCH_CORE", "TEST_CODE_NEWS_BATCH_NEXT")
            .replace("1785114003.000000000", "1785114004.000000000")
            .replace("2026-07-27T01:00:03Z", "2026-07-27T01:00:04Z");
        let next_fact = AdmittedNewsFact::from_recovery_snapshot(other_batch.as_bytes())
            .unwrap()
            .with_target_name("TEST_CODE changed display".to_owned());
        let next_identity = NewsAiIdentityV3::from_fact(&next_fact, &profile).unwrap();
        assert_eq!(next_identity, identity);
        drop(conn.into_inner());
        let mut conn = SqliteConnection::establish(path).unwrap();
        create_schema(&mut conn).unwrap();
        let conn = RefCell::new(conn);
        let second = analyzer
            .assess_if_absent(
                next_identity,
                |identity| {
                    std::future::ready(
                        load_audited_news_ai_assessment_for_identity_on_conn(
                            &mut conn.borrow_mut(),
                            &identity,
                        )
                        .map(|v| v.is_some())
                        .map_err(|e| e.to_string()),
                    )
                },
                |_| async { panic!("retained identity must not acquire new market evidence") },
            )
            .await
            .unwrap();
        assert!(second.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(acquired.load(Ordering::SeqCst), 1);
        assert_eq!(count(&mut conn.borrow_mut(), "news_ai_assessment"), 1);
        assert_eq!(
            load_recovery_bytes(&mut conn.borrow_mut(), &identity.digest())
                .unwrap()
                .unwrap(),
            frozen
        );
        let restored =
            load_audited_news_ai_assessment_for_identity_on_conn(&mut conn.borrow_mut(), &identity)
                .unwrap()
                .unwrap();
        assert_eq!(restored.delivery().render_card(), original_card);
        assert_eq!(
            restored.delivery().fact().source_batch_id(),
            "TEST_CODE_NEWS_BATCH_CORE"
        );
        let recovered = load_pending_news_ai_recoveries_on_conn(&mut conn.borrow_mut(), 1).unwrap();
        assert!(matches!(recovered[0], NewsAiPendingRecovery::Ready(_)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "recovery cannot invoke model"
        );
        let error = analyzer
            .assess_if_absent(
                identity,
                |_| async { Err("TEST_CODE corrupt audit".to_owned()) },
                |_| async { panic!("lookup failure is not cache miss") },
            )
            .await
            .unwrap_err();
        assert!(error.contains("corrupt audit"));
    }

    #[test]
    fn core_assessment_projection_binds_all_source_model_and_analysis_evidence() {
        let (request, assessment) = core_assessment();
        let input =
            NewsAiAssessmentAuditInput::from_core(&request, &assessment).expect("audit projection");
        assert_eq!(input.assessment_id, assessment.assessment_id());
        assert_eq!(input.source_provider, "cailianpress");
        assert_eq!(input.source_batch_id, "TEST_CODE_NEWS_BATCH_CORE");
        assert_eq!(input.source_item_id, "TEST_CODE_NEWS_ITEM_CORE");
        assert_eq!(input.target_code, "TEST_CODE_600519");
        assert_eq!(input.analysis_version, "TEST_CODE_NEWS_AI_CORE_V1");
        assert_eq!(
            input.model_upstream_request_id.as_deref(),
            Some("TEST_CODE_REQUEST_CORE")
        );
        assert_eq!(input.model_upstream_response_id, "TEST_CODE_MODEL_RESPONSE");
        assert_eq!(
            input.input_evidence_sha256,
            assessment.input_evidence_sha256()
        );
        assert_eq!(
            input.normalized_prompt_sha256,
            assessment.normalized_prompt_sha256()
        );

        let mut conn = connection();
        append_news_ai_assessment_on_conn(&mut conn, &input)
            .expect("projected assessment must append");
        validate_news_ai_assessment_chain(&mut conn).expect("valid projected chain");
    }

    #[test]
    fn exact_source_identity_is_checked_before_a_repeat_model_call() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        assert!(
            !has_news_ai_assessment_for_fact_on_conn(
                &mut conn,
                request.fact(),
                request.analysis_version(),
            )
            .expect("clean audit lookup"),
            "unseen identity must remain eligible"
        );

        let input =
            NewsAiAssessmentAuditInput::from_core(&request, &assessment).expect("audit projection");
        append_news_ai_assessment_on_conn(&mut conn, &input).expect("append assessment");

        assert!(
            has_news_ai_assessment_for_fact_on_conn(
                &mut conn,
                request.fact(),
                request.analysis_version(),
            )
            .expect("persisted audit lookup"),
            "exact persisted identity must skip a duplicate model call"
        );
    }

    #[test]
    fn assessment_id_must_match_the_exact_source_identity() {
        let mut conn = connection();
        let mut mismatched = input();
        mismatched.assessment_id =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();

        let error = append_news_ai_assessment_on_conn(&mut conn, &mismatched)
            .expect_err("a caller-provided identity must not detach audit from source");
        assert!(matches!(error, NewsAiAssessmentAuditError::InvalidInput(_)));
    }

    #[test]
    fn br172_existing_assessment_loads_delivery_capability_without_another_model_call() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        let receipt = append_news_ai_assessment_on_conn(&mut conn, &input).unwrap();

        let loaded = load_audited_news_ai_assessment_for_fact_on_conn(
            &mut conn,
            request.fact(),
            request.analysis_version(),
        )
        .unwrap()
        .expect("persisted assessment must remain delivery eligible");

        assert_eq!(
            loaded.delivery().assessment().assessment_id(),
            receipt.assessment_id
        );
        assert_eq!(
            loaded.delivery().assessment_audit_record_sha256(),
            receipt.record_hash
        );
        assert_eq!(loaded.delivery().fact().title(), request.fact().title());
    }

    #[test]
    fn news_ai_recovery_reuses_the_first_rendered_card() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let initial =
            append_audited_news_ai_assessment_on_conn(&mut conn, request.clone(), assessment)
                .unwrap();
        let changed_display_fact = request
            .fact()
            .clone()
            .with_target_name("恢复时变化的名称".to_owned());
        let recovered = load_audited_news_ai_assessment_for_fact_on_conn(
            &mut conn,
            &changed_display_fact,
            request.analysis_version(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            initial.delivery().render_card(),
            recovered.delivery().render_card()
        );
        assert_eq!(
            initial.delivery().business_date(),
            recovered.delivery().business_date()
        );
    }

    #[test]
    fn br172_recovery_queue_is_fair_under_a_ready_backlog() {
        let mut conn = connection();
        for index in 0..12 {
            let (request, assessment) =
                core_assessment_for(&format!("TEST_CODE_READY_{index}"), "TEST_CODE_600519");
            append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).unwrap();
        }
        for index in 0..3 {
            let (request, assessment) =
                core_assessment_for(&format!("TEST_CODE_MANUAL_{index}"), "TEST_CODE_600519");
            let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
            append_news_ai_assessment_on_conn(&mut conn, &input).unwrap();
        }
        let mut ready = std::collections::BTreeSet::new();
        let mut manual = std::collections::BTreeSet::new();
        for _ in 0..3 {
            let pending = load_pending_news_ai_recoveries_on_conn(&mut conn, 2).unwrap();
            assert_eq!(
                pending.len(),
                2,
                "the limit applies to both categories together"
            );
            for work in pending {
                match work {
                    NewsAiPendingRecovery::Ready(audited) => {
                        ready.insert(audited.delivery().assessment().assessment_id().to_owned());
                    }
                    NewsAiPendingRecovery::ManualReview { assessment_id, .. } => {
                        manual.insert(assessment_id);
                    }
                }
            }
        }
        assert_eq!(
            manual.len(),
            3,
            "every manual item gets a turn despite ready backlog"
        );
        assert_eq!(
            ready.len(),
            3,
            "ready progress rotates instead of retrying its prefix"
        );
    }

    #[test]
    fn br172_manual_confirmation_and_rotation_survive_reopen_and_failed_ack() {
        std::fs::create_dir_all("data/test").unwrap();
        let namespace = tempfile::Builder::new()
            .prefix("TEST_CODE_BR172_REVIEW_")
            .tempdir_in("data/test")
            .unwrap();
        let path = namespace.path().join("news_ai.sqlite3");
        let open = || {
            let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
            conn.batch_execute("PRAGMA foreign_keys = ON;").unwrap();
            create_schema(&mut conn).unwrap();
            conn
        };
        let mut conn = open();
        let (request, assessment) = core_assessment_for("TEST_CODE_READY", "TEST_CODE_600519");
        append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).unwrap();
        let (request, assessment) = core_assessment_for("TEST_CODE_MANUAL", "TEST_CODE_600519");
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        let expected = append_news_ai_assessment_on_conn(&mut conn, &input)
            .unwrap()
            .assessment_id;
        assert!(matches!(
            load_pending_news_ai_recoveries_on_conn(&mut conn, 1)
                .unwrap()
                .as_slice(),
            [NewsAiPendingRecovery::Ready(_)]
        ));
        drop(conn);

        let mut conn = open();
        let NewsAiPendingRecovery::ManualReview {
            assessment_id,
            claim_id,
            ..
        } = load_pending_news_ai_recoveries_on_conn(&mut conn, 1)
            .unwrap()
            .pop()
            .unwrap()
        else {
            panic!("a persisted ready claim must give the next turn to manual")
        };
        assert_eq!(assessment_id, expected);
        let first_claim = claim_id;
        // Crash before publication/ack: claiming alone must not close the item.
        drop(conn);

        let mut conn = open();
        assert!(matches!(
            load_pending_news_ai_recoveries_on_conn(&mut conn, 1)
                .unwrap()
                .as_slice(),
            [NewsAiPendingRecovery::Ready(_)]
        ));
        let NewsAiPendingRecovery::ManualReview {
            assessment_id,
            claim_id,
            ..
        } = load_pending_news_ai_recoveries_on_conn(&mut conn, 1)
            .unwrap()
            .pop()
            .unwrap()
        else {
            panic!("unconfirmed manual work must return after reopen")
        };
        assert_eq!(assessment_id, expected);
        assert!(claim_id > first_claim);
        conn.batch_execute(
            "CREATE TRIGGER TEST_CODE_fail_review_ack BEFORE INSERT ON news_ai_recovery_review_notified
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE simulated ack failure'); END;",
        ).unwrap();
        assert!(confirm_news_ai_recovery_review_on_conn(&mut conn, claim_id).is_err());
        conn.batch_execute("DROP TRIGGER TEST_CODE_fail_review_ack;")
            .unwrap();
        drop(conn);

        let mut conn = open();
        let pending = load_pending_news_ai_recoveries_on_conn(&mut conn, 2).unwrap();
        assert_eq!(pending.len(), 2);
        let retry_claim = pending
            .into_iter()
            .find_map(|work| match work {
                NewsAiPendingRecovery::ManualReview {
                    assessment_id,
                    claim_id,
                    ..
                } => {
                    assert_eq!(assessment_id, expected);
                    Some(claim_id)
                }
                _ => None,
            })
            .expect("failed acknowledgement must retain retry eligibility");
        // Publication has succeeded before this ack; repeated ack is idempotent.
        confirm_news_ai_recovery_review_on_conn(&mut conn, retry_claim).unwrap();
        confirm_news_ai_recovery_review_on_conn(&mut conn, retry_claim).unwrap();
        drop(conn);

        let mut conn = open();
        for _ in 0..3 {
            assert!(
                matches!(
                    load_pending_news_ai_recoveries_on_conn(&mut conn, 2)
                        .unwrap()
                        .as_slice(),
                    [NewsAiPendingRecovery::Ready(_)]
                ),
                "confirmed manual notice must not repeat"
            );
        }
        assert_eq!(count(&mut conn, "news_ai_assessment"), 2);
        assert_eq!(count(&mut conn, "news_ai_recovery_review_notified"), 1);
        assert_eq!(
            count(&mut conn, "news_ai_delivery_event"),
            0,
            "manual confirmation is not a delivery terminal state"
        );
    }

    #[test]
    fn br172_failed_scan_rolls_back_every_claim_and_keeps_global_limit() {
        let mut conn = connection();
        let (request, assessment) = core_assessment_for("TEST_CODE_READY", "TEST_CODE_600519");
        append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment).unwrap();
        let (request, assessment) = core_assessment_for("TEST_CODE_MANUAL", "TEST_CODE_600519");
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        append_news_ai_assessment_on_conn(&mut conn, &input).unwrap();
        conn.batch_execute(
            "CREATE TRIGGER TEST_CODE_fail_manual_claim BEFORE INSERT ON news_ai_recovery_claim
             WHEN NEW.category = 'manual'
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE simulated claim failure'); END;",
        )
        .unwrap();
        assert!(load_pending_news_ai_recoveries_on_conn(&mut conn, 2).is_err());
        assert_eq!(
            count(&mut conn, "news_ai_recovery_claim"),
            0,
            "a failed second claim must roll back the first claim too"
        );
        conn.batch_execute("DROP TRIGGER TEST_CODE_fail_manual_claim;")
            .unwrap();
        for limit in [0, 101] {
            assert!(load_pending_news_ai_recoveries_on_conn(&mut conn, limit).is_err());
        }
        assert_eq!(count(&mut conn, "news_ai_recovery_claim"), 0);
        assert!(matches!(
            load_pending_news_ai_recoveries_on_conn(&mut conn, 1)
                .unwrap()
                .as_slice(),
            [NewsAiPendingRecovery::Ready(_)]
        ));
        assert!(matches!(
            load_pending_news_ai_recoveries_on_conn(&mut conn, 1)
                .unwrap()
                .as_slice(),
            [NewsAiPendingRecovery::ManualReview { .. }]
        ));
        assert!(
            confirm_news_ai_recovery_review_on_conn(&mut conn, 1).is_err(),
            "a ready scheduling claim is not a manual acknowledgement capability"
        );
        assert!(confirm_news_ai_recovery_review_on_conn(&mut conn, 999).is_err());
    }

    #[test]
    fn br172_pending_scanner_reconstructs_audited_delivery_without_live_batch() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let initial =
            append_audited_news_ai_assessment_on_conn(&mut conn, request.clone(), assessment)
                .expect("persist audited assessment and recovery evidence");

        let pending = load_pending_news_ai_recoveries_on_conn(&mut conn, 5)
            .expect("scan durable pending NewsAI work");
        assert_eq!(pending.len(), 1);
        let NewsAiPendingRecovery::Ready(recovered) = &pending[0] else {
            panic!("new assessment with a frozen fact must be independently recoverable");
        };
        assert_eq!(
            recovered.delivery().assessment().assessment_id(),
            initial.delivery().assessment().assessment_id()
        );
        assert_eq!(recovered.delivery().fact().title(), request.fact().title());
        assert_eq!(
            recovered.delivery().render_card(),
            initial.delivery().render_card()
        );
    }

    #[test]
    fn br172_pending_scanner_survives_a_real_sqlite_close_and_reopen() {
        std::fs::create_dir_all("data/test").expect("TEST_CODE namespace parent");
        let namespace = tempfile::Builder::new()
            .prefix("TEST_CODE_BR172_PENDING_")
            .tempdir_in("data/test")
            .expect("isolated NewsAI namespace");
        let path = namespace.path().join("news_ai.sqlite3");
        let expected_id = {
            let mut writer =
                SqliteConnection::establish(path.to_str().unwrap()).expect("open NewsAI writer");
            writer
                .batch_execute("PRAGMA foreign_keys = ON;")
                .expect("writer foreign keys");
            create_schema(&mut writer).expect("writer schema");
            let (request, assessment) = core_assessment();
            append_audited_news_ai_assessment_on_conn(&mut writer, request, assessment)
                .expect("persist pending assessment")
                .delivery()
                .assessment()
                .assessment_id()
                .to_owned()
        };

        let mut reader =
            SqliteConnection::establish(path.to_str().unwrap()).expect("reopen NewsAI database");
        reader
            .batch_execute("PRAGMA foreign_keys = ON;")
            .expect("reader foreign keys");
        create_schema(&mut reader).expect("reader schema");
        let pending = load_pending_news_ai_recoveries_on_conn(&mut reader, 5)
            .expect("scan after real reopen");
        let NewsAiPendingRecovery::Ready(recovered) = &pending[0] else {
            panic!("persisted fact must remain independently recoverable after reopen");
        };
        assert_eq!(
            recovered.delivery().assessment().assessment_id(),
            expected_id
        );
    }

    #[test]
    fn br172_pending_scanner_keeps_delivered_for_link_recovery_then_excludes_linked() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let audited = append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment)
            .expect("persist audited assessment");
        let delivery = audited.delivery();
        let NewsAiReserveOutcome::Reserved(reservation) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).expect("reserve delivery")
        else {
            panic!("fresh assessment must reserve");
        };
        begin_news_ai_sink_attempt_on_conn(&mut conn, delivery, &reservation)
            .expect("mark sink started");
        let audit = record_news_ai_delivered_on_conn(
            &mut conn,
            delivery,
            &reservation,
            "TEST_CODE_BR172_DELIVERY_AUDIT",
        )
        .expect("record delivered");

        assert!(matches!(
            load_pending_news_ai_recoveries_on_conn(&mut conn, 5)
                .expect("delivered pending scan")
                .as_slice(),
            [NewsAiPendingRecovery::Ready(_)]
        ));

        link_news_ai_prediction_on_conn(&mut conn, delivery, &reservation, &audit)
            .expect("link prediction");
        assert!(load_pending_news_ai_recoveries_on_conn(&mut conn, 5)
            .expect("linked pending scan")
            .is_empty());
    }

    #[test]
    fn br172_pending_scanner_classifies_legacy_assessment_without_frozen_fact() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        let receipt = append_news_ai_assessment_on_conn(&mut conn, &input)
            .expect("append legacy assessment without recovery snapshot");

        let pending = load_pending_news_ai_recoveries_on_conn(&mut conn, 5)
            .expect("legacy rows must be reported rather than fabricated");
        assert_eq!(pending.len(), 1);
        let NewsAiPendingRecovery::ManualReview {
            assessment_id,
            reason,
            ..
        } = &pending[0]
        else {
            panic!("legacy assessment must not be reconstructed from current news");
        };
        assert_eq!(assessment_id, &receipt.assessment_id);
        assert!(reason.contains("recovery snapshot"));
    }

    #[test]
    fn legacy_sink_attempt_without_frozen_card_cannot_be_replayed() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        append_news_ai_assessment_on_conn(&mut conn, &input).unwrap();
        let audited = load_audited_news_ai_assessment_for_fact_on_conn(
            &mut conn,
            request.fact(),
            request.analysis_version(),
        )
        .unwrap()
        .unwrap();
        let NewsAiReserveOutcome::Reserved(reservation) =
            reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap()
        else {
            panic!("first attempt must reserve");
        };
        begin_news_ai_sink_attempt_on_conn(&mut conn, audited.delivery(), &reservation).unwrap();
        let error = load_audited_news_ai_assessment_for_fact_on_conn(
            &mut conn,
            request.fact(),
            request.analysis_version(),
        )
        .expect_err("an attempted legacy card has no stable recovery identity");
        assert!(error.to_string().contains("without a frozen delivery card"));
    }

    #[test]
    fn frozen_sink_started_recovers_with_new_audit_reservation() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let audited = append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment)
            .expect("freeze first card");
        let delivery = audited.delivery();
        let NewsAiReserveOutcome::Reserved(first) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap()
        else {
            panic!("first reservation expected");
        };
        begin_news_ai_sink_attempt_on_conn(&mut conn, delivery, &first).unwrap();

        let NewsAiReserveOutcome::Reserved(recovery) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap()
        else {
            panic!("frozen card must be eligible for audit-only recovery");
        };
        assert_ne!(first.reservation_id(), recovery.reservation_id());
        begin_news_ai_sink_attempt_on_conn(&mut conn, delivery, &recovery).unwrap();
        record_news_ai_post_sink_recovery_on_conn(
            &mut conn,
            delivery,
            &recovery,
            None,
            "delivery audit write failed",
        )
        .unwrap();
        let NewsAiReserveOutcome::Reserved(after_audit_failure) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap()
        else {
            panic!("missing audit receipt must reopen only BR-172 audit work");
        };
        assert_ne!(
            recovery.reservation_id(),
            after_audit_failure.reservation_id()
        );
        validate_news_ai_delivery_audit(&mut conn).unwrap();
    }

    #[test]
    fn post_sink_recovery_with_audit_receipt_links_without_new_reservation() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let audited = append_audited_news_ai_assessment_on_conn(&mut conn, request, assessment)
            .expect("freeze first card");
        let delivery = audited.delivery();
        let NewsAiReserveOutcome::Reserved(reservation) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap()
        else {
            panic!("first reservation expected");
        };
        begin_news_ai_sink_attempt_on_conn(&mut conn, delivery, &reservation).unwrap();
        record_news_ai_post_sink_recovery_on_conn(
            &mut conn,
            delivery,
            &reservation,
            Some("TEST_CODE_DELIVERY_AUDIT"),
            "L7 write failed",
        )
        .unwrap();

        let NewsAiReserveOutcome::LinkPending(recovery) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap()
        else {
            panic!("persisted authoritative audit must recover the prediction link");
        };
        assert_eq!(
            recovery.reservation().reservation_id(),
            reservation.reservation_id()
        );
        assert_eq!(
            recovery.delivery_audit().audit_event_id(),
            "TEST_CODE_DELIVERY_AUDIT"
        );
        link_news_ai_prediction_on_conn(
            &mut conn,
            delivery,
            recovery.reservation(),
            recovery.delivery_audit(),
        )
        .expect("link only, without another physical attempt");
        validate_news_ai_delivery_audit(&mut conn).unwrap();
    }

    #[test]
    fn br172_rolled_back_delivery_remains_retryable() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        append_news_ai_assessment_on_conn(&mut conn, &input).unwrap();
        let audited = load_audited_news_ai_assessment_for_fact_on_conn(
            &mut conn,
            request.fact(),
            request.analysis_version(),
        )
        .unwrap()
        .unwrap();

        let first = reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap();
        let NewsAiReserveOutcome::Reserved(first) = first else {
            panic!("first exact identity must be reserved");
        };
        rollback_news_ai_delivery_on_conn(
            &mut conn,
            audited.delivery(),
            &first,
            "TEST_CODE_governance_denied",
        )
        .unwrap();

        let retry = reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap();
        let NewsAiReserveOutcome::Reserved(retry) = retry else {
            panic!("rolled-back identity must remain retryable");
        };
        assert_ne!(retry.reservation_id(), first.reservation_id());
        validate_news_ai_delivery_audit(&mut conn).unwrap();
    }

    /// Exact counted decision terminal state persisted by the production port.
    const COUNTED_TERMINAL_DENIAL: &str =
        "BR172_PRE_SINK_NOT_DELIVERED:durable delivery terminal state=RejectedDurable";

    fn audited_assessment(
        conn: &mut SqliteConnection,
        item_id: &str,
        target_code: &str,
    ) -> crate::monitor::news_ai::AuditedNewsAiAssessment {
        let (request, assessment) = core_assessment_for(item_id, target_code);
        let input =
            NewsAiAssessmentAuditInput::from_core(&request, &assessment).expect("audit projection");
        append_news_ai_assessment_on_conn(conn, &input).expect("append assessment");
        load_audited_news_ai_assessment_for_fact_on_conn(
            conn,
            request.fact(),
            request.analysis_version(),
        )
        .expect("load audited assessment")
        .expect("persisted assessment must remain delivery eligible")
    }

    /// Drive one card through the BR-172 audit to `prediction_linked`.
    fn deliver_card(
        conn: &mut SqliteConnection,
        audited: &crate::monitor::news_ai::AuditedNewsAiAssessment,
    ) {
        let delivery = audited.delivery();
        let NewsAiReserveOutcome::Reserved(reservation) =
            reserve_news_ai_delivery_on_conn(conn, delivery).expect("reserve")
        else {
            panic!("unseen identity must reserve");
        };
        begin_news_ai_sink_attempt_on_conn(conn, delivery, &reservation).expect("begin");
        let audit = record_news_ai_delivered_on_conn(
            conn,
            delivery,
            &reservation,
            "TEST_CODE_PERSISTED_ENVELOPE_ID",
        )
        .expect("record delivered");
        link_news_ai_prediction_on_conn(conn, delivery, &reservation, &audit).expect("link");
    }

    /// One full denied round: reserve → begin → rollback.
    fn denied_round(
        conn: &mut SqliteConnection,
        audited: &crate::monitor::news_ai::AuditedNewsAiAssessment,
        reason: &str,
    ) -> crate::monitor::news_ai::NewsAiReserveOutcome {
        let delivery = audited.delivery();
        let outcome = reserve_news_ai_delivery_on_conn(conn, delivery).expect("reserve");
        if let NewsAiReserveOutcome::Reserved(reservation) = &outcome {
            begin_news_ai_sink_attempt_on_conn(conn, delivery, reservation).expect("begin");
            rollback_news_ai_delivery_on_conn(conn, delivery, reservation, reason)
                .expect("rollback");
        }
        outcome
    }

    #[test]
    fn br172_counted_terminal_denial_stops_after_one_rollback() {
        let mut conn = connection();
        let audited = audited_assessment(&mut conn, "TEST_CODE_NEWS_ITEM_CORE", "TEST_CODE_600519");
        let delivery = audited.delivery();
        assert!(!is_news_ai_terminal_denial_for_fact_on_conn(
            &mut conn,
            delivery.fact(),
            delivery.analysis_version()
        )
        .unwrap());

        assert!(matches!(
            denied_round(&mut conn, &audited, COUNTED_TERMINAL_DENIAL),
            NewsAiReserveOutcome::Reserved(_)
        ));
        assert!(is_news_ai_terminal_denial_for_fact_on_conn(
            &mut conn,
            delivery.fact(),
            delivery.analysis_version()
        )
        .unwrap());
        let events = count(&mut conn, "news_ai_delivery_event");
        for _ in 0..3 {
            assert!(matches!(
                reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap(),
                NewsAiReserveOutcome::Deduped
            ));
        }
        assert_eq!(count(&mut conn, "news_ai_delivery_event"), events);
        validate_news_ai_delivery_audit(&mut conn).unwrap();
    }

    #[test]
    fn br172_and_counted_reentry_share_one_physical_delivery() {
        use crate::durable_delivery::{
            AuthoritativeDeliveryRequest, AuthoritativeSinkPort, AuthoritativeSinkResult,
            CoordinatorConfig, DecisionState, DeliveryEnvelope, DeliverySubKind,
            DurableDeliveryCoordinator, DurableDeliveryError, ImmutableAppendPort,
            PushKind as CountedPushKind, TypedReceipt,
        };
        use sha2::{Digest, Sha256};
        use std::collections::BTreeMap;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Append {
            records: Mutex<BTreeMap<String, (Vec<u8>, String)>>,
        }

        impl ImmutableAppendPort for Append {
            fn append_exact(
                &self,
                _record_kind: &str,
                identity: &str,
                canonical_bytes: &[u8],
                sha256: &str,
            ) -> crate::durable_delivery::Result<String> {
                let mut records = self.records.lock().expect("append records");
                let value = (canonical_bytes.to_vec(), sha256.to_owned());
                match records.get(identity) {
                    Some(previous) if previous != &value => Err(
                        DurableDeliveryError::ImmutableAppendConflict(identity.to_owned()),
                    ),
                    Some(_) => Ok(format!("test://{identity}")),
                    None => {
                        records.insert(identity.to_owned(), value);
                        Ok(format!("test://{identity}"))
                    }
                }
            }
        }

        struct Sink(AtomicUsize);

        impl AuthoritativeSinkPort for Sink {
            fn sink_identity(&self) -> &str {
                "TEST_CODE_BR172_COUNTED_SINK"
            }

            fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
                self.0.fetch_add(1, Ordering::SeqCst);
                AuthoritativeSinkResult::Accepted(TypedReceipt {
                    channel: "TEST_CODE_CHANNEL".to_owned(),
                    provider: "TEST_CODE_PROVIDER".to_owned(),
                    message_id: "TEST_CODE_BR172_COUNTED_MESSAGE".to_owned(),
                    platform_message_id: None,
                    accepted_at: Utc.with_ymd_and_hms(2026, 7, 27, 8, 0, 0).unwrap(),
                    latency_ms: Some(1),
                })
            }
        }

        let mut conn = connection();
        let audited = audited_assessment(&mut conn, "TEST_CODE_BR172_COUNTED", "TEST_CODE_600519");
        let delivery = audited.delivery();
        let NewsAiReserveOutcome::Reserved(reservation) =
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).expect("BR-172 reserve")
        else {
            panic!("new assessment must reserve");
        };

        std::fs::create_dir_all("data/test").expect("TEST_CODE namespace parent");
        let namespace = tempfile::Builder::new()
            .prefix("TEST_CODE_BR172_COUNTED_")
            .tempdir_in("data/test")
            .expect("isolated counted namespace");
        let test_code = namespace
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            namespace.path().join("durable_delivery.sqlite3"),
            &test_code,
            format!("owner-{test_code}-0123456789abcdef"),
        ))
        .expect("isolated counted coordinator");
        let identity = delivery.identity().sha256();
        let text = delivery.render_card();
        let subject_hash = hex::encode(Sha256::digest(text.as_bytes()));
        let counted = DeliveryEnvelope::new(
            "2026-07-27",
            CountedPushKind::NewsAiAnalysis,
            DeliverySubKind::None,
            "SSE:EQUITY:TEST_CODE_600519",
            format!("news-ai-analysis:2026-07-27:TEST_CODE_600519:{identity}"),
            identity,
            identity.as_bytes().to_vec(),
            subject_hash,
            text.into_bytes(),
            false,
            None,
        )
        .expect("exact counted envelope");
        let now = Utc.with_ymd_and_hms(2026, 7, 27, 8, 0, 0).unwrap();
        let append = Append::default();
        let sink = Arc::new(Sink(AtomicUsize::new(0)));
        let first = coordinator
            .prepare(&counted, 1, now)
            .expect("counted prepare");
        assert_eq!(first.state, DecisionState::Reserved);
        coordinator
            .reconcile_all_pending(&append, now)
            .expect("counted prepare audit");
        let sinks: Vec<crate::durable_delivery::AuthoritativeSink> = vec![sink.clone()];
        let resumed = coordinator
            .resume_deliverable(&counted.decision_identity, &sinks, now)
            .expect("counted physical delivery");
        assert_eq!(resumed.sink_calls, 1);
        coordinator
            .reconcile_all_pending(&append, now)
            .expect("counted delivery audit");
        assert_eq!(
            coordinator
                .decision_state(&counted.decision_identity)
                .unwrap(),
            DecisionState::Delivered
        );

        // Keep the production adapter's order: counted delivery becomes
        // durable before the BR-172 sink marker is written.
        begin_news_ai_sink_attempt_on_conn(&mut conn, delivery, &reservation)
            .expect("BR-172 sink marker");
        let audit_id = format!("TEST_CODE_COUNTED_{}", counted.decision_identity);
        let audit = record_news_ai_delivered_on_conn(&mut conn, delivery, &reservation, &audit_id)
            .expect("BR-172 delivered audit");
        link_news_ai_prediction_on_conn(&mut conn, delivery, &reservation, &audit)
            .expect("BR-172 prediction link");

        let replay = coordinator
            .prepare(&counted, 1, now)
            .expect("counted replay");
        assert_eq!(replay.state, DecisionState::Delivered);
        assert_eq!(replay.sink_calls, 0);
        assert_eq!(sink.0.load(Ordering::SeqCst), 1);
        assert!(matches!(
            reserve_news_ai_delivery_on_conn(&mut conn, delivery).unwrap(),
            NewsAiReserveOutcome::Deduped
        ));

        // A second source-bound assessment for the same ticket is denied by
        // the real counted cooldown. BR-172 must close its reservation without
        // claiming that the second card reached the physical sink.
        let second = audited_assessment(
            &mut conn,
            "TEST_CODE_BR172_COUNTED_SECOND",
            "TEST_CODE_600519",
        );
        let second_delivery = second.delivery();
        let NewsAiReserveOutcome::Reserved(second_reservation) =
            reserve_news_ai_delivery_on_conn(&mut conn, second_delivery)
                .expect("second BR-172 reserve")
        else {
            panic!("second assessment must reserve independently");
        };
        let second_identity = second_delivery.identity().sha256();
        let second_text = second_delivery.render_card();
        let second_counted = DeliveryEnvelope::new(
            "2026-07-27",
            CountedPushKind::NewsAiAnalysis,
            DeliverySubKind::None,
            "SSE:EQUITY:TEST_CODE_600519",
            format!("news-ai-analysis:2026-07-27:TEST_CODE_600519:{second_identity}"),
            second_identity,
            second_identity.as_bytes().to_vec(),
            hex::encode(Sha256::digest(second_text.as_bytes())),
            second_text.into_bytes(),
            false,
            None,
        )
        .expect("second exact counted envelope");
        let denied = coordinator
            .prepare(&second_counted, 1, now)
            .expect("counted cooldown decision");
        assert_eq!(denied.state, DecisionState::RejectedAuditPending);
        assert_eq!(denied.sink_calls, 0);
        coordinator
            .reconcile_all_pending(&append, now)
            .expect("persist counted cooldown rejection audit");
        assert_eq!(
            coordinator
                .decision_state(&second_counted.decision_identity)
                .unwrap(),
            DecisionState::RejectedDurable
        );
        assert_eq!(sink.0.load(Ordering::SeqCst), 1);
        rollback_news_ai_delivery_on_conn(
            &mut conn,
            second_delivery,
            &second_reservation,
            "BR172_PRE_SINK_NOT_DELIVERED:durable delivery terminal state=RejectedDurable",
        )
        .expect("BR-172 rollback after counted denial");
        assert!(is_news_ai_terminal_denial_for_fact_on_conn(
            &mut conn,
            second_delivery.fact(),
            second_delivery.analysis_version(),
        )
        .unwrap());
        assert!(matches!(
            reserve_news_ai_delivery_on_conn(&mut conn, second_delivery).unwrap(),
            NewsAiReserveOutcome::Deduped
        ));
        validate_news_ai_delivery_audit(&mut conn).expect("BR-172 chain remains valid");
        drop(coordinator);
    }

    #[test]
    fn br172_repeated_sink_failure_is_not_mistaken_for_terminal_policy_denial() {
        let mut conn = connection();
        let sibling =
            audited_assessment(&mut conn, "TEST_CODE_NEWS_ITEM_SIBLING", "TEST_CODE_600519");
        deliver_card(&mut conn, &sibling);
        let audited = audited_assessment(&mut conn, "TEST_CODE_NEWS_ITEM_CORE", "TEST_CODE_600519");
        let fault = "BR172_PRE_SINK_NOT_DELIVERED:counted_sink_rejected reason_code=magiclaw_cli_spawn_failed retry_authorized=true";

        for _ in 0..2 {
            assert!(matches!(
                denied_round(&mut conn, &audited, fault),
                NewsAiReserveOutcome::Reserved(_)
            ));
        }
        assert!(!is_news_ai_terminal_denial_for_fact_on_conn(
            &mut conn,
            audited.delivery().fact(),
            audited.delivery().analysis_version()
        )
        .unwrap());
        assert!(matches!(
            reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap(),
            NewsAiReserveOutcome::Reserved(_)
        ));
    }

    #[test]
    fn br172_preflight_denial_remains_retryable() {
        let mut conn = connection();
        let audited = audited_assessment(&mut conn, "TEST_CODE_NEWS_ITEM_CORE", "TEST_CODE_600519");
        assert!(matches!(
            denied_round(
                &mut conn,
                &audited,
                "BR172_PRE_SINK_NOT_DELIVERED:launch_gate_stage"
            ),
            NewsAiReserveOutcome::Reserved(_)
        ));
        assert!(!is_news_ai_terminal_denial_for_fact_on_conn(
            &mut conn,
            audited.delivery().fact(),
            audited.delivery().analysis_version()
        )
        .unwrap());
        assert!(matches!(
            reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap(),
            NewsAiReserveOutcome::Reserved(_)
        ));
    }

    #[test]
    fn br172_delivery_commit_links_exact_audit_before_deduping_replay() {
        let mut conn = connection();
        let (request, assessment) = core_assessment();
        let input = NewsAiAssessmentAuditInput::from_core(&request, &assessment).unwrap();
        append_news_ai_assessment_on_conn(&mut conn, &input).unwrap();
        let audited = load_audited_news_ai_assessment_for_fact_on_conn(
            &mut conn,
            request.fact(),
            request.analysis_version(),
        )
        .unwrap()
        .unwrap();
        let NewsAiReserveOutcome::Reserved(reservation) =
            reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap()
        else {
            panic!("unseen exact identity must reserve");
        };

        begin_news_ai_sink_attempt_on_conn(&mut conn, audited.delivery(), &reservation).unwrap();
        let delivery_audit = record_news_ai_delivered_on_conn(
            &mut conn,
            audited.delivery(),
            &reservation,
            "TEST_CODE_PERSISTED_ENVELOPE_ID",
        )
        .unwrap();
        let NewsAiReserveOutcome::LinkPending(recovery) =
            reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap()
        else {
            panic!("delivered-but-unlinked identity must recover linkage only");
        };
        assert_eq!(
            recovery.delivery_audit().audit_event_id(),
            delivery_audit.audit_event_id()
        );
        let prediction_link = link_news_ai_prediction_on_conn(
            &mut conn,
            audited.delivery(),
            recovery.reservation(),
            recovery.delivery_audit(),
        )
        .unwrap();
        let idempotent_replay = link_news_ai_prediction_on_conn(
            &mut conn,
            audited.delivery(),
            recovery.reservation(),
            recovery.delivery_audit(),
        )
        .unwrap();

        assert_eq!(
            delivery_audit.delivery_identity_sha256(),
            audited.delivery().identity().sha256()
        );
        assert!(!prediction_link.prediction_link_id().is_empty());
        assert_eq!(
            idempotent_replay.prediction_link_id(),
            prediction_link.prediction_link_id()
        );
        assert!(matches!(
            reserve_news_ai_delivery_on_conn(&mut conn, audited.delivery()).unwrap(),
            NewsAiReserveOutcome::Deduped
        ));
        validate_news_ai_delivery_audit(&mut conn).unwrap();
    }

    #[test]
    fn identical_assessment_is_idempotent_but_same_id_with_changed_content_conflicts() {
        let mut conn = connection();
        let first =
            append_news_ai_assessment_on_conn(&mut conn, &input()).expect("first assessment");
        let replay =
            append_news_ai_assessment_on_conn(&mut conn, &input()).expect("idempotent replay");
        assert!(!replay.inserted);
        assert_eq!(replay.assessment_id, first.assessment_id);
        assert_eq!(replay.record_hash, first.record_hash);
        assert_eq!(count(&mut conn, "news_ai_assessment"), 1);
        assert_eq!(count(&mut conn, "news_ai_assessment_chain"), 1);

        let mut conflicting = input();
        conflicting.confidence = 83;
        assert!(matches!(
            append_news_ai_assessment_on_conn(&mut conn, &conflicting),
            Err(NewsAiAssessmentAuditError::Conflict { .. })
        ));
        assert_eq!(count(&mut conn, "news_ai_assessment"), 1);
        assert_eq!(count(&mut conn, "news_ai_assessment_chain"), 1);
    }

    #[test]
    fn assessment_and_chain_are_immutable_with_five_year_retention_semantics() {
        let mut conn = connection();
        append_news_ai_assessment_on_conn(&mut conn, &input()).expect("append assessment");

        for statement in [
            "UPDATE news_ai_assessment SET confidence = 1",
            "DELETE FROM news_ai_assessment",
            "UPDATE news_ai_assessment_chain SET previous_hash = 'TEST_CODE_TAMPER'",
            "DELETE FROM news_ai_assessment_chain",
        ] {
            let error = diesel::sql_query(statement)
                .execute(&mut conn)
                .expect_err("immutable audit statement must fail");
            assert!(error.to_string().contains("at least five years"));
        }
        assert_eq!(NEWS_AI_ASSESSMENT_MIN_RETENTION_YEARS, 5);
    }

    #[test]
    fn confidence_outside_the_strict_model_range_fails_before_database_write() {
        let mut conn = connection();
        let mut invalid = input();
        invalid.confidence = 101;

        let error = append_news_ai_assessment_on_conn(&mut conn, &invalid)
            .expect_err("out-of-range confidence must fail");
        assert!(matches!(error, NewsAiAssessmentAuditError::InvalidInput(_)));
        assert_eq!(count(&mut conn, "news_ai_assessment"), 0);
        assert_eq!(count(&mut conn, "news_ai_assessment_chain"), 0);
    }

    #[test]
    fn invalid_fields_times_and_test_environment_identity_fail_atomically() {
        let mut conn = connection();

        let mut cases = Vec::new();
        let mut blank_request_id = input();
        blank_request_id.model_upstream_request_id = Some(" ".to_owned());
        cases.push(blank_request_id);

        let mut bad_hash = input();
        bad_hash.input_evidence_sha256 = "NOT_A_SHA256".to_owned();
        cases.push(bad_hash);

        let mut missing_response_id = input();
        missing_response_id.model_upstream_response_id = " ".to_owned();
        cases.push(missing_response_id);

        let mut mismatched_user_hash = input();
        mismatched_user_hash.model_user_sha256 =
            "5555555555555555555555555555555555555555555555555555555555555555".to_owned();
        cases.push(mismatched_user_hash);

        let mut invalid_system_hash = input();
        invalid_system_hash.model_system_sha256 = "NOT_A_SHA256".to_owned();
        cases.push(invalid_system_hash);

        let mut reversed_model_times = input();
        std::mem::swap(
            &mut reversed_model_times.model_started_at,
            &mut reversed_model_times.model_completed_at,
        );
        cases.push(reversed_model_times);

        let mut real_code_in_test = input();
        real_code_in_test.target_code = "600519".to_owned();
        cases.push(real_code_in_test);

        for invalid in cases {
            assert!(matches!(
                append_news_ai_assessment_on_conn(&mut conn, &invalid),
                Err(NewsAiAssessmentAuditError::InvalidInput(_))
            ));
        }
        assert_eq!(count(&mut conn, "news_ai_assessment"), 0);
        assert_eq!(count(&mut conn, "news_ai_assessment_chain"), 0);
    }

    #[test]
    fn retained_fact_tamper_blocks_validation_and_future_append() {
        let mut conn = connection();
        append_news_ai_assessment_on_conn(&mut conn, &input()).expect("append assessment");
        diesel::sql_query("DROP TRIGGER trg_news_ai_assessment_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query("UPDATE news_ai_assessment SET confidence = 81")
            .execute(&mut conn)
            .expect("test-only fact tamper");

        assert!(matches!(
            validate_news_ai_assessment_chain(&mut conn),
            Err(NewsAiAssessmentAuditError::Audit(_))
        ));
        assert!(matches!(
            append_news_ai_assessment_on_conn(&mut conn, &input()),
            Err(NewsAiAssessmentAuditError::Audit(_))
        ));
        assert_eq!(count(&mut conn, "news_ai_assessment"), 1);
        assert_eq!(count(&mut conn, "news_ai_assessment_chain"), 1);
    }

    #[test]
    fn retained_chain_tamper_is_detected() {
        let mut conn = connection();
        append_news_ai_assessment_on_conn(&mut conn, &input()).expect("append assessment");
        diesel::sql_query("DROP TRIGGER trg_news_ai_assessment_chain_no_update")
            .execute(&mut conn)
            .expect("test-only tamper setup");
        diesel::sql_query(
            "UPDATE news_ai_assessment_chain
                SET record_hash = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        )
        .execute(&mut conn)
        .expect("test-only chain tamper");
        assert!(matches!(
            validate_news_ai_assessment_chain(&mut conn),
            Err(NewsAiAssessmentAuditError::Audit(_))
        ));
    }

    #[test]
    fn sqlite_failures_are_propagated_without_fake_success() {
        let mut conn = connection();
        diesel::sql_query("DROP TABLE news_ai_assessment_chain")
            .execute(&mut conn)
            .expect("test-only schema failure setup");

        assert!(matches!(
            append_news_ai_assessment_on_conn(&mut conn, &input()),
            Err(NewsAiAssessmentAuditError::Database(_))
        ));
        assert_eq!(count(&mut conn, "news_ai_assessment"), 0);
    }

    #[test]
    fn repository_migration_installs_the_news_ai_assessment_audit() {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        super::super::DatabaseManager::run_migrations_for_test(&mut conn)
            .expect("repository migrations");
        let installed = diesel::sql_query(
            "SELECT COUNT(*) AS count
               FROM sqlite_master
              WHERE type = 'table'
                AND name IN ('news_ai_assessment', 'news_ai_assessment_chain')",
        )
        .get_result::<CountRow>(&mut conn)
        .expect("migration table query")
        .count;
        assert_eq!(installed, 2);
    }
    const N01_RESPONSE: &str = r#"{"impact":"positive","confidence":73,"uncertainty":"TEST_CODE execution risk","core_logic":"TEST_CODE contract evidence","strength":91}"#;

    pub(crate) fn critical_fixture(conn: &mut SqliteConnection, item: &str) -> crate::monitor::news_ai::AuditedCriticalNews {
        let (request,_) = core_assessment_for(item,"TEST_CODE_600519");
        let result = crate::monitor::news_ai::critical_test_result(request,N01_RESPONSE).unwrap();
        critical_strength::append(conn,result).unwrap().1
    }

    #[test]
    fn news_n01_strict_receipt_audit_and_closed_delivery_evidence() {
        use crate::event::envelope::{DomainEvent,PushDeliveryEvent,NEWS_FLASH_CRITICAL_AUDIT_SCHEMA_VERSION};
        let mut conn = connection();
        let score = critical_fixture(&mut conn,"TEST_CODE_N01_STRICT");
        assert_eq!(score.evidence().strength(),91);
        assert_eq!(score.evidence().digest().unwrap(),score.evidence_sha256());
        let source = score.evidence().source().unwrap();
        let day = source.published_at.date_naive();
        let mut event = PushDeliveryEvent::new_news_flash_attempt("news_flash_critical_v1".into(),source.event_id.clone(),"TEST_CODE_channel".into(),42,
            day,"a".repeat(64),vec![source.clone()],score.evidence_sha256().into(),"b".repeat(64),1,source.observed_at+chrono::Duration::seconds(2));
        event.audit_schema_version = NEWS_FLASH_CRITICAL_AUDIT_SCHEMA_VERSION;
        event.news_critical_evidence = Some(score.evidence().clone());
        event.validate().unwrap();
        let envelope = crate::event::EventEnvelope::from_event(&event,event.news_flash_join_sha256.clone().unwrap(),"TEST_CODE_trace".into(),chrono::Local::now()).unwrap();
        crate::event::PushRecord::try_from_authoritative(&envelope).unwrap();
        let namespace = crate::event::dispatcher::TestAuditNamespace::new("TEST_CODE_N01_GENERIC_REFUSED");
        let dispatcher = namespace.dispatcher();
        assert!(matches!(crate::event::Dispatcher::dispatch(&dispatcher,envelope),
            crate::event::DispatchResult::Failed(_)));
        // Mutations preserve old six source slots while changing the bound full score material.
        for field in ["fact_snapshot","normalized_prompt","response","assessment_audit_sha256","strength"] {
            let mut value = serde_json::to_value(score.evidence()).unwrap();
            if field == "strength" { value[field] = serde_json::json!(92); }
            else { value[field] = serde_json::json!("TEST_CODE_changed"); }
            let bad: crate::monitor::news_ai::CriticalNewsEvidence = serde_json::from_value(value).unwrap();
            event.news_critical_evidence = Some(bad);
            assert!(event.validate().is_err(),"{field}");
        }
        for raw in [r#"{"impact":"positive","confidence":73,"uncertainty":"x","core_logic":"y"}"#,
            r#"{"impact":"positive","confidence":73,"uncertainty":"x","core_logic":"y","strength":101}"#,
            r#"{"impact":"positive","confidence":73,"uncertainty":"x","core_logic":"y","strength":91,"extra":1}"#] {
            let (request,_) = core_assessment_for("TEST_CODE_invalid","TEST_CODE_600519");
            assert!(crate::monitor::news_ai::critical_test_result(request,raw).is_err());
        }
        assert!(conn.batch_execute("UPDATE news_ai_n01_score SET evidence_sha256='x'").is_err());
        assert!(conn.batch_execute("DELETE FROM news_ai_n01_score").is_err());
    }

    #[test]
    fn news_n01_cross_profile_base_and_unknown_never_absent() {
        let mut conn = connection();
        let (request,assessment) = core_assessment_for("TEST_CODE_N01_OLD","TEST_CODE_600519");
        let fact = request.fact().clone();
        append_audited_news_ai_assessment_on_conn(&mut conn,request,assessment).unwrap();
        assert!(critical_strength::has_base(&mut conn,&fact).unwrap());
        // Target/profile/batch cannot reopen the same immutable text revision.
        let (other,_) = core_assessment_for("TEST_CODE_N01_OLD","TEST_CODE_600600");
        assert_eq!(crate::monitor::news_ai::NewsBaseIdentity::from_fact(&fact).unwrap(),
            crate::monitor::news_ai::NewsBaseIdentity::from_fact(other.fact()).unwrap());
        assert!(critical_strength::has_base(&mut conn,other.fact()).unwrap());
        let result = crate::monitor::news_ai::critical_test_result(core_assessment_for("TEST_CODE_N01_OLD","TEST_CODE_600519").0,N01_RESPONSE).unwrap();
        assert!(critical_strength::append(&mut conn,result).is_err());
        // Missing legacy snapshot is Unknown, never a successful cache miss.
        conn.batch_execute("DROP TRIGGER trg_news_ai_delivery_recovery_snapshot_no_delete; DELETE FROM news_ai_delivery_recovery_snapshot").unwrap();
        assert!(critical_strength::has_base(&mut conn,&fact).is_err());
        let (absent,_) = core_assessment_for("TEST_CODE_N01_NEW","TEST_CODE_600519");
        // Corrupt full chain is checked even for an otherwise absent item.
        conn.batch_execute("DROP TRIGGER trg_news_ai_assessment_chain_no_update; UPDATE news_ai_assessment_chain SET record_hash='0000000000000000000000000000000000000000000000000000000000000000'").unwrap();
        assert!(critical_strength::has_base(&mut conn,absent.fact()).is_err());
    }

    #[test]
    fn news_n01_revision_persistence_readback_and_no_recovery_mint() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("TEST_CODE_N01.sqlite");
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        conn.batch_execute("PRAGMA foreign_keys=ON").unwrap();
        create_schema(&mut conn).unwrap();
        let score = critical_fixture(&mut conn,"TEST_CODE_N01_COLD");
        let fact = score.fact().unwrap();
        drop(conn);
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        conn.batch_execute("PRAGMA foreign_keys=ON").unwrap();
        create_schema(&mut conn).unwrap();
        assert!(critical_strength::has_base(&mut conn,&fact).unwrap());
        let id = score.evidence().assessment_id();
        let row = load_by_assessment_id(&mut conn,&id).unwrap().unwrap();
        let identity = load_recovery_identity(&mut conn,&row).unwrap().unwrap();
        let restored = load_audited_news_ai_assessment_for_identity_on_conn(&mut conn,&identity).unwrap().unwrap();
        assert_eq!(restored.delivery().assessment().assessment_id(),id);
        // Original delivery is recoverable; no loader exposes a scored-capability return.
        let result = crate::monitor::news_ai::critical_test_result(core_assessment_for("TEST_CODE_N01_COLD","TEST_CODE_600519").0,N01_RESPONSE).unwrap();
        assert!(critical_strength::append(&mut conn,result).is_err());
        let count: CountRow = diesel::sql_query("SELECT COUNT(*) AS count FROM news_ai_n01_score").get_result(&mut conn).unwrap();
        assert_eq!(count.count,1);
        // A failed score association rolls the original assessment back with it.
        conn.batch_execute("CREATE TRIGGER TEST_CODE_score_abort BEFORE INSERT ON news_ai_n01_score BEGIN SELECT RAISE(ABORT,'TEST_CODE fault'); END").unwrap();
        let result = crate::monitor::news_ai::critical_test_result(core_assessment_for("TEST_CODE_N01_ROLLBACK","TEST_CODE_600519").0,N01_RESPONSE).unwrap();
        let failed_id = result.assessment.assessment_id().to_owned();
        assert!(critical_strength::append(&mut conn,result).is_err());
        assert!(load_by_assessment_id(&mut conn,&failed_id).unwrap().is_none());
    }

}
