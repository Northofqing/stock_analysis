//! BR-172 immutable NewsAI assessment audit.
//!
//! The public seam accepts one already-validated assessment projection and
//! appends it atomically with a SHA-256 chain link. It has no delivery,
//! prediction or producer side effect.

use chrono::{DateTime, FixedOffset, SecondsFormat};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::DatabaseManager;

const SCHEMA_VERSION: i32 = 1;
const CHAIN_GENESIS: &str = "BR172_NEWS_AI_ASSESSMENT_GENESIS_V1";
const SOURCE_IDENTITY_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_SOURCE_IDENTITY_V1\0";
const CONTENT_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_ASSESSMENT_CONTENT_V1\0";
const CHAIN_HASH_DOMAIN: &[u8] = b"BR172_NEWS_AI_ASSESSMENT_CHAIN_V1\0";

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
            analysis_version: request.analysis_version().to_owned(),
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
struct ChainRow {
    #[diesel(sql_type = BigInt)]
    assessment_row_id: i64,
    #[diesel(sql_type = Text)]
    previous_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

fn invalid(message: impl Into<String>) -> NewsAiAssessmentAuditError {
    NewsAiAssessmentAuditError::InvalidInput(message.into())
}

fn source_provider_tag(
    provider: crate::magic_compat::ProviderId,
) -> NewsAiAssessmentAuditResult<&'static str> {
    match provider {
        crate::magic_compat::ProviderId::Eastmoney => Ok("eastmoney"),
        crate::magic_compat::ProviderId::Cailianpress => Ok("cailianpress"),
        crate::magic_compat::ProviderId::Jin10 => Ok("jin10"),
        crate::magic_compat::ProviderId::ThePaper => Ok("thepaper"),
        crate::magic_compat::ProviderId::Sina => Ok("sina"),
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
    let expected_assessment_id = core_assessment_id(&source_identity);
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
) -> NewsAiAssessmentAuditResult<String> {
    hash_serializable(CONTENT_HASH_DOMAIN, assessment)
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
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<CanonicalAssessment> {
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
    Ok(canonical)
}

fn validate_persisted_row(
    row: &PersistedAssessmentRow,
) -> NewsAiAssessmentAuditResult<CanonicalAssessment> {
    if row.schema_version != SCHEMA_VERSION {
        return Err(audit(format!(
            "unsupported assessment schema version {} at row {}",
            row.schema_version, row.id
        )));
    }
    let canonical = canonical_from_row(row)?;
    let expected_source_hash = source_identity_hash(&canonical.source_identity)?;
    let expected_content_hash = assessment_content_hash(&canonical)?;
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
        validate_persisted_row(row)?;
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

pub(super) fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    conn.batch_execute(SCHEMA)
        .map_err(|error| format!("BR-172 create NewsAI assessment schema: {error}"))?;
    validate_news_ai_assessment_chain(conn)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn insert_assessment_in_transaction(
    conn: &mut SqliteConnection,
    input: &NewsAiAssessmentAuditInput,
) -> NewsAiAssessmentAuditResult<NewsAiAssessmentAuditReceipt> {
    let canonical = canonical_assessment(input)?;
    let expected_source_hash = source_identity_hash(&canonical.source_identity)?;
    let expected_content_hash = assessment_content_hash(&canonical)?;
    let previous_hash = validate_news_ai_assessment_chain(conn)?;

    if let Some(existing) = load_by_assessment_id(conn, &canonical.assessment_id)? {
        if existing.content_hash != expected_content_hash
            || existing.source_identity_sha256 != expected_source_hash
        {
            return Err(NewsAiAssessmentAuditError::Conflict {
                assessment_id: canonical.assessment_id,
            });
        }
        validate_persisted_row(&existing)?;
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
    validate_persisted_row(&row)?;
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
    conn.immediate_transaction::<_, NewsAiAssessmentAuditError, _>(|conn| {
        insert_assessment_in_transaction(conn, input)
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

impl DatabaseManager {
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

    pub fn validate_news_ai_assessment_audit(&self) -> NewsAiAssessmentAuditResult<String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| NewsAiAssessmentAuditError::Connection(error.to_string()))?;
        validate_news_ai_assessment_chain(&mut conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magic_compat::ProviderId;
    use crate::magic_compat::SourceEvidence;
    use chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
    use diesel::connection::SimpleConnection;

    #[derive(Debug, QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("NewsAI assessment schema");
        conn
    }

    fn input() -> NewsAiAssessmentAuditInput {
        let timezone = FixedOffset::east_opt(8 * 3600).expect("UTC+08:00");
        NewsAiAssessmentAuditInput {
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
        let record = GlobalNewsRecord {
            item_id: "TEST_CODE_NEWS_ITEM_CORE".to_owned(),
            title: "TEST_CODE exact source-bound contract".to_owned(),
            summary: Some("TEST_CODE disclosed contract evidence".to_owned()),
            content: None,
            publisher: "TEST_CODE publisher".to_owned(),
            canonical_url: "https://example.com/TEST_CODE_NEWS_ITEM_CORE".to_owned(),
            published_at,
            observed_at,
            instruments: vec!["600519".to_owned()],
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
        let fact = AdmittedNewsFact::from_global(&record, &news_batch, "TEST_CODE_600519")
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
            target_code: "TEST_CODE_600519".to_owned(),
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
        let request = NewsAiRequest::try_new(fact, market, Vec::new(), "TEST_CODE_NEWS_AI_CORE_V1")
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
}
