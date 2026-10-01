//! Strict restart reconstruction of a previously captured admitted record.
//! This crate-only reader grants no acquisition or receipt capability.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{capture_admitted_record, provider_id_wire_name, SCHEMA};
use crate::data_gateway::{BatchEvidence, GlobalNewsProvider, GlobalNewsRecord};
use crate::market_domain::SourceEvidence;
use crate::news::aggregator::raw_v2::{
    NewsFlashProjectedEvent, NewsFlashSourceIdentity, RegisteredGlobalNewsFeed,
    MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES,
};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum NewsFlashRecordReplayError {
    #[error("admitted record bytes exceed the canonical bound")]
    TooLarge,
    #[error("invalid admitted record field: {0}")]
    InvalidField(&'static str),
    #[error("admitted record bytes are not the exact canonical encoding")]
    NonCanonical,
}

type ReplayResult<T> = Result<T, NewsFlashRecordReplayError>;

fn field<'a>(value: &'a Value, name: &'static str) -> ReplayResult<&'a str> {
    value[name]
        .as_str()
        .ok_or(NewsFlashRecordReplayError::InvalidField(name))
}

fn optional(value: &Value, name: &'static str) -> ReplayResult<Option<String>> {
    match &value[name] {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(NewsFlashRecordReplayError::InvalidField(name)),
    }
}

fn keys(value: &Value, expected: &[&'static str]) -> ReplayResult<()> {
    let object = value
        .as_object()
        .ok_or(NewsFlashRecordReplayError::NonCanonical)?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(NewsFlashRecordReplayError::NonCanonical);
    }
    Ok(())
}

fn timestamp(value: &Value) -> ReplayResult<DateTime<Utc>> {
    keys(value, &["nanoseconds", "unix_seconds"])?;
    let seconds = value["unix_seconds"]
        .as_i64()
        .ok_or(NewsFlashRecordReplayError::InvalidField("unix_seconds"))?;
    let nanos = value["nanoseconds"]
        .as_u64()
        .and_then(|nanos| u32::try_from(nanos).ok())
        .ok_or(NewsFlashRecordReplayError::InvalidField("nanoseconds"))?;
    DateTime::from_timestamp(seconds, nanos)
        .ok_or(NewsFlashRecordReplayError::InvalidField("timestamp"))
}

fn texts(value: &Value, name: &'static str) -> ReplayResult<Vec<String>> {
    value[name]
        .as_array()
        .ok_or(NewsFlashRecordReplayError::InvalidField(name))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(NewsFlashRecordReplayError::InvalidField(name))
        })
        .collect()
}

fn registration(value: &Value) -> ReplayResult<RegisteredGlobalNewsFeed> {
    keys(
        value,
        &[
            "capability_name",
            "feed_name",
            "gateway_provider",
            "max_limit",
            "provider",
            "provider_id",
            "source_contract",
            "upstream_revision",
        ],
    )?;
    let provider = GlobalNewsProvider::from_wire_name(field(value, "provider")?)
        .ok_or(NewsFlashRecordReplayError::InvalidField("provider"))?;
    let expected = RegisteredGlobalNewsFeed::for_provider(provider);
    if field(value, "capability_name")? != expected.capability_name
        || field(value, "feed_name")? != expected.feed_name
        || field(value, "gateway_provider")? != expected.gateway_provider
        || value["max_limit"].as_u64() != Some(u64::from(expected.max_limit))
        || field(value, "provider_id")? != expected.provider_id
        || field(value, "source_contract")? != expected.source_contract
        || field(value, "upstream_revision")? != expected.upstream_revision
    {
        return Err(NewsFlashRecordReplayError::InvalidField("registration"));
    }
    Ok(expected)
}

/// Rebuild the same SourceOnly projection from stored canonical bytes, then
/// encode it again with the original bounded codec. No caller-supplied digest,
/// source identity, or market-event semantics are trusted.
pub(crate) fn replay_n02_admitted_record(bytes: &[u8]) -> ReplayResult<NewsFlashProjectedEvent> {
    if bytes.is_empty() || bytes.len() > MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES {
        return Err(NewsFlashRecordReplayError::TooLarge);
    }
    let root: Value =
        serde_json::from_slice(bytes).map_err(|_| NewsFlashRecordReplayError::NonCanonical)?;
    keys(
        &root,
        &[
            "batch_evidence",
            "record",
            "registration",
            "schema",
            "source_identity",
        ],
    )?;
    if field(&root, "schema")? != SCHEMA {
        return Err(NewsFlashRecordReplayError::InvalidField("schema"));
    }
    let registration = registration(&root["registration"])?;
    let provider = registration.provider;
    let provider_wire = provider_id_wire_name(provider.provider_id());

    let batch_value = &root["batch_evidence"];
    keys(
        batch_value,
        &["batch_id", "observed_at", "provider", "source", "source_at"],
    )?;
    if field(batch_value, "provider")? != provider_wire {
        return Err(NewsFlashRecordReplayError::InvalidField("batch_provider"));
    }
    let batch = BatchEvidence {
        provider: provider.provider_id(),
        source: field(batch_value, "source")?.to_owned(),
        source_at: optional(batch_value, "source_at")?,
        observed_at: field(batch_value, "observed_at")?.to_owned(),
        batch_id: field(batch_value, "batch_id")?.to_owned(),
    };

    let record_value = &root["record"];
    keys(
        record_value,
        &[
            "canonical_url",
            "content",
            "evidence",
            "instruments",
            "item_id",
            "language",
            "observed_at",
            "published_at",
            "publisher",
            "summary",
            "title",
            "topics",
        ],
    )?;
    let evidence_value = &record_value["evidence"];
    keys(
        evidence_value,
        &["batch_id", "observed_at", "provider", "source_at"],
    )?;
    if field(evidence_value, "provider")? != provider_wire {
        return Err(NewsFlashRecordReplayError::InvalidField("record_provider"));
    }
    let mut evidence = SourceEvidence::new(
        provider.provider_id(),
        field(evidence_value, "observed_at")?,
        field(evidence_value, "batch_id")?,
    )
    .map_err(|_| NewsFlashRecordReplayError::InvalidField("record_evidence"))?;
    if let Some(source_at) = optional(evidence_value, "source_at")? {
        evidence = evidence
            .with_source_at(source_at)
            .map_err(|_| NewsFlashRecordReplayError::InvalidField("record_source_at"))?;
    }
    let record = GlobalNewsRecord {
        item_id: field(record_value, "item_id")?.to_owned(),
        title: field(record_value, "title")?.to_owned(),
        summary: optional(record_value, "summary")?,
        content: optional(record_value, "content")?,
        publisher: field(record_value, "publisher")?.to_owned(),
        canonical_url: field(record_value, "canonical_url")?.to_owned(),
        published_at: timestamp(&record_value["published_at"])?,
        observed_at: timestamp(&record_value["observed_at"])?,
        instruments: texts(record_value, "instruments")?,
        topics: texts(record_value, "topics")?,
        language: field(record_value, "language")?.to_owned(),
        evidence,
    };
    if crate::data_gateway::global_news::validate_global_news_batch_evidence(provider, &batch)
        .is_err()
        || super::super::validate_news_flash_record(
            provider,
            &record,
            &batch,
            crate::risk::env_guard::current_env(),
        )
        .is_err()
    {
        return Err(NewsFlashRecordReplayError::InvalidField("admission"));
    }
    let event = super::super::super::feed::record_to_market_event(provider, &record)
        .map_err(|_| NewsFlashRecordReplayError::InvalidField("projection"))?;
    let source = NewsFlashSourceIdentity {
        event_id: event.event_id.clone(),
        provider: provider.wire_name().to_owned(),
        source: batch.source.clone(),
        published_at: record.published_at,
        observed_at: record.observed_at,
        batch_id: batch.batch_id.clone(),
    };
    let source_value = &root["source_identity"];
    keys(
        source_value,
        &[
            "batch_id",
            "event_id",
            "observed_at",
            "provider",
            "published_at",
            "source",
        ],
    )?;
    if field(source_value, "batch_id")? != source.batch_id()
        || field(source_value, "event_id")? != source.event_id()
        || timestamp(&source_value["observed_at"])? != source.observed_at()
        || field(source_value, "provider")? != source.provider()
        || timestamp(&source_value["published_at"])? != source.published_at()
        || field(source_value, "source")? != source.source()
    {
        return Err(NewsFlashRecordReplayError::InvalidField("source_identity"));
    }
    let captured = capture_admitted_record(registration, &record, &batch, &source)
        .map_err(|_| NewsFlashRecordReplayError::NonCanonical)?;
    if captured.canonical_bytes() != bytes {
        return Err(NewsFlashRecordReplayError::NonCanonical);
    }
    Ok(NewsFlashProjectedEvent {
        event,
        source,
        record_evidence: Ok(Arc::new(captured)),
    })
}
