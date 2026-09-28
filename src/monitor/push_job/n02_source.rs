//! Process-local N02 source witness. This derives Foundation facts from the
//! exact selected admitted records; it does not register or persist a producer.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::canonical::{canonical_preimage, raw_digest, CanonicalValue};
use super::{
    CapturedFacts, ExactBytes, ExternalId, FactsPresence, PushJobError, Sha256Digest,
    SourceContractId, SourceContractVersion, SourceProvider, SourceRef, SourceRefId, SourceTime,
    UtcMicros,
};
use crate::news::aggregator::raw_v2::{
    self, NewsFlashProjectedEvent, NewsFlashRecordEvidenceError, NewsFlashRecordEvidenceV1,
    NewsFlashSourceIdentity, RegisteredGlobalNewsFeed, MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES,
};
use crate::push_foundation::{N02BindingError, N02ReservationBindingV1};
use crate::signal::market_event::{Direction, EventType};

pub const N02_SOURCE_CONTRACT_ID: &str = "news-flash-admitted-records-v1";
pub const N02_SOURCE_CONTRACT_VERSION: &str = "1";
const FACTS_SCHEMA: &str = "N02SelectedFacts/v1";
const SOURCE_OCCURRENCE_SCHEMA: &str = "N02SourceOccurrence/v1";
const SOURCE_ONLY_POLICY: &str = "BR166SourceOnly/v1";
// The admitted record codec caps every selected item at 1 MiB. Canonical
// facts repeat only bounded selected fields and are independently capped.
const MAX_N02_SELECTED_FACTS_BYTES: usize = 8 * MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum N02SourceError {
    #[error("invalid legacy N02 binding: {0}")]
    Binding(N02BindingError),
    #[error("invalid selected source count: {0}")]
    SelectedCount(usize),
    #[error("selected evidence digest differs from the reservation")]
    EvidenceDigestMismatch,
    #[error("selected source mismatch at rank {index}")]
    SourceMismatch { index: usize },
    #[error("duplicate selected event at rank {index}")]
    DuplicateEvent { index: usize },
    #[error("admitted record capture failed at rank {index}: {error:?}")]
    RecordEvidence {
        index: usize,
        error: NewsFlashRecordEvidenceError,
    },
    #[error("registered feed mismatch at rank {index}")]
    RegistrationMismatch { index: usize },
    #[error("projection mismatch at rank {index}: {field}")]
    ProjectionMismatch { index: usize, field: &'static str },
    #[error("admitted record content hash mismatch at rank {index}")]
    ContentHashMismatch { index: usize },
    #[error("invalid source time at rank {index}: {field}")]
    SourceTime { index: usize, field: &'static str },
    #[error("N02 selected facts exceed the bounded canonical size")]
    FactsTooLarge,
    #[error("invalid Foundation source facts: {0}")]
    Foundation(PushJobError),
}

/// Validated immutable association of one legacy reservation with the exact
/// selected admitted records. Its canonical facts are owned once; original
/// admitted-record bytes remain borrowed until this witness is dropped.
#[derive(Debug)]
pub struct N02SourceChainV1<'a> {
    binding: N02ReservationBindingV1,
    records: Vec<&'a NewsFlashRecordEvidenceV1>,
    source_refs: Vec<SourceRef>,
    source_times: Vec<SourceTime>,
    canonical_facts: ExactBytes,
    rendered_raw_sha256: Sha256Digest,
}

impl<'a> N02SourceChainV1<'a> {
    pub fn try_capture(
        binding: N02ReservationBindingV1,
        selected: &'a [NewsFlashProjectedEvent],
        rendered: &[u8],
    ) -> Result<Self, N02SourceError> {
        N02ReservationBindingV1::try_from_reservation_material(
            binding.material().clone(),
            rendered,
        )
        .map_err(N02SourceError::Binding)?;
        if !(1..=3).contains(&selected.len()) {
            return Err(N02SourceError::SelectedCount(selected.len()));
        }
        let material = binding.material();
        if material.sources.len() != selected.len() {
            return Err(N02SourceError::SelectedCount(selected.len()));
        }
        if raw_v2::ordered_news_flash_evidence_sha256(selected) != material.evidence_sha256 {
            return Err(N02SourceError::EvidenceDigestMismatch);
        }

        let contract_id = source_contract_id()?;
        let mut event_ids = HashSet::with_capacity(selected.len());
        let mut records = Vec::with_capacity(selected.len());
        let mut source_refs = Vec::with_capacity(selected.len());
        let mut source_times = Vec::with_capacity(selected.len());
        let mut entries = Vec::with_capacity(selected.len());
        for (index, projected) in selected.iter().enumerate() {
            let source = projected.source();
            if !source_matches_audit(source, &material.sources[index]) {
                return Err(N02SourceError::SourceMismatch { index });
            }
            if !event_ids.insert(source.event_id()) {
                return Err(N02SourceError::DuplicateEvent { index });
            }
            let record =
                projected
                    .record_evidence()
                    .map_err(|error| N02SourceError::RecordEvidence {
                        index,
                        error: *error,
                    })?;
            if record.source() != source {
                return Err(N02SourceError::SourceMismatch { index });
            }
            let registration = record.registration();
            if registration != RegisteredGlobalNewsFeed::for_provider(registration.provider)
                || registration.provider.wire_name() != source.provider()
                || registration.source_contract != source.source()
            {
                return Err(N02SourceError::RegistrationMismatch { index });
            }
            validate_projected(index, projected, record, material.business_date)?;
            let content_sha256 = raw_digest(record.canonical_bytes());
            if record.canonical_bytes().is_empty()
                || record.canonical_bytes().len() > MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES
                || content_sha256.as_str() != record.content_sha256()
            {
                return Err(N02SourceError::ContentHashMismatch { index });
            }
            let observed = source_time(index, source)?;
            let source_ref_id = SourceRefId::try_new(source_occurrence_sha256(
                source,
                record.item_id(),
                content_sha256.as_str(),
            ))
            .map_err(N02SourceError::Foundation)?;
            source_times.push(SourceTime::observed_at(
                source_ref_id.clone(),
                Some(observed),
            ));
            source_refs.push(SourceRef::new(
                source_ref_id,
                SourceProvider::try_new(source.provider().to_owned())
                    .map_err(N02SourceError::Foundation)?,
                ExternalId::try_new(record.item_id().to_owned())
                    .map_err(N02SourceError::Foundation)?,
                contract_id.clone(),
                content_sha256.clone(),
            ));
            entries.push(fact_entry(
                index,
                projected,
                record,
                content_sha256.as_str(),
            )?);
            records.push(record);
        }

        let rendered_raw_sha256 = raw_digest(rendered);
        let fields = BTreeMap::from([
            ("business_date", string(material.business_date.to_string())),
            ("entries", CanonicalValue::Array(entries)),
            ("legacy_evidence_sha256", string(&material.evidence_sha256)),
            (
                "legacy_render_sha256",
                string(&material.news_flash_render_sha256),
            ),
            (
                "legacy_reservation_sha256",
                string(&material.reservation_sha256),
            ),
            ("projection_policy", string(SOURCE_ONLY_POLICY)),
            (
                "rendered_len",
                CanonicalValue::Unsigned(rendered.len() as u64),
            ),
            ("rendered_raw_sha256", string(rendered_raw_sha256.as_str())),
            ("schema", string(FACTS_SCHEMA)),
            ("window", string(&material.window)),
        ]);
        let canonical_bytes = canonical_preimage(FACTS_SCHEMA, &fields);
        if canonical_bytes.len() > MAX_N02_SELECTED_FACTS_BYTES {
            return Err(N02SourceError::FactsTooLarge);
        }
        Ok(Self {
            binding,
            records,
            source_refs,
            source_times,
            canonical_facts: ExactBytes::new(canonical_bytes),
            rendered_raw_sha256,
        })
    }

    pub fn binding(&self) -> &N02ReservationBindingV1 {
        &self.binding
    }

    pub fn records(&self) -> &[&NewsFlashRecordEvidenceV1] {
        &self.records
    }

    pub fn source_refs(&self) -> &[SourceRef] {
        &self.source_refs
    }

    pub fn source_times(&self) -> &[SourceTime] {
        &self.source_times
    }

    pub fn canonical_facts(&self) -> &ExactBytes {
        &self.canonical_facts
    }

    pub fn rendered_raw_sha256(&self) -> &Sha256Digest {
        &self.rendered_raw_sha256
    }

    pub fn into_captured_facts(self) -> Result<CapturedFacts, N02SourceError> {
        CapturedFacts::try_new(
            source_contract_id()?,
            SourceContractVersion::try_new(N02_SOURCE_CONTRACT_VERSION.to_owned())
                .map_err(N02SourceError::Foundation)?,
            self.source_refs,
            self.canonical_facts,
            self.source_times,
            FactsPresence::Present,
            Vec::new(),
        )
        .map_err(N02SourceError::Foundation)
    }
}

fn source_contract_id() -> Result<SourceContractId, N02SourceError> {
    SourceContractId::try_new(N02_SOURCE_CONTRACT_ID.to_owned()).map_err(N02SourceError::Foundation)
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn source_matches_audit(
    source: &NewsFlashSourceIdentity,
    audit: &crate::event::NewsFlashAuditSource,
) -> bool {
    source.event_id() == audit.event_id
        && source.provider() == audit.provider
        && source.source() == audit.source
        && source.published_at().fixed_offset() == audit.published_at
        && source.observed_at().fixed_offset() == audit.observed_at
        && source.batch_id() == audit.batch_id
}

fn validate_projected(
    index: usize,
    projected: &NewsFlashProjectedEvent,
    record: &NewsFlashRecordEvidenceV1,
    business_date: chrono::NaiveDate,
) -> Result<(), N02SourceError> {
    let source = projected.source();
    let event = projected.event();
    let expected_event_id =
        raw_v2::br166_global_news_event_id(record.registration().provider, record.item_id());
    if record.item_id().trim().is_empty()
        || event.event_id != expected_event_id
        || source.event_id() != expected_event_id
    {
        return Err(N02SourceError::ProjectionMismatch {
            index,
            field: "event_id",
        });
    }
    let publication_matches = event
        .provider_publication
        .as_ref()
        .is_some_and(|publication| {
            publication.published_on == event.occurred_at.date_naive()
                && publication.published_at == Some(event.occurred_at)
        });
    if event.event_type != EventType::Other
        || event.direction != Direction::Neutral
        || event.strength != 0
        || event.certainty != 100
        || event.stale
        || event.ai_degraded
        || !event.chains.is_empty()
        || event.full_title.trim().is_empty()
        || event.subject.trim().is_empty()
        || event.object.as_deref() != Some(event.full_title.as_str())
        || event.occurred_at.date_naive() != business_date
        || event.occurred_at.with_timezone(&Utc) != source.published_at()
        || !publication_matches
        || event.provenance.len() != 1
        || event.provenance[0].provider != source.source()
        || event.provenance[0].url.as_deref().is_none_or(str::is_empty)
        || event.provenance[0].fetched_at.with_timezone(&Utc) != source.observed_at()
    {
        return Err(N02SourceError::ProjectionMismatch {
            index,
            field: "source_only_semantics",
        });
    }
    Ok(())
}

fn source_time(
    index: usize,
    source: &NewsFlashSourceIdentity,
) -> Result<UtcMicros, N02SourceError> {
    let published = source.published_at();
    let observed = source.observed_at();
    if published.timestamp() < 0 || observed.timestamp() < 0 || observed < published {
        return Err(N02SourceError::SourceTime {
            index,
            field: "publication_observation_order",
        });
    }
    // SourceTime is microsecond precision; truncate positive nanoseconds
    // toward zero. Full nanoseconds stay in canonical facts and record bytes.
    let micros = observed
        .timestamp()
        .checked_mul(1_000_000)
        .and_then(|seconds| {
            seconds.checked_add(i64::from(observed.timestamp_subsec_nanos() / 1_000))
        })
        .ok_or(N02SourceError::SourceTime {
            index,
            field: "observed_at_overflow",
        })?;
    UtcMicros::try_new(micros).map_err(|_| N02SourceError::SourceTime {
        index,
        field: "observed_at_nonnegative",
    })
}

fn source_occurrence_sha256(
    source: &NewsFlashSourceIdentity,
    item_id: &str,
    content_sha256: &str,
) -> String {
    let published_at = source.published_at().to_rfc3339();
    let observed_at = source.observed_at().to_rfc3339();
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_OCCURRENCE_SCHEMA.as_bytes());
    hasher.update([0]);
    for value in [
        source.event_id(),
        source.provider(),
        source.source(),
        published_at.as_str(),
        observed_at.as_str(),
        source.batch_id(),
        item_id,
        content_sha256,
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn timestamp(value: DateTime<Utc>) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "nanoseconds",
            CanonicalValue::Unsigned(u64::from(value.timestamp_subsec_nanos())),
        ),
        (
            "unix_seconds",
            CanonicalValue::Unsigned(value.timestamp() as u64),
        ),
    ]))
}

fn fact_entry(
    index: usize,
    projected: &NewsFlashProjectedEvent,
    record: &NewsFlashRecordEvidenceV1,
    content_sha256: &str,
) -> Result<CanonicalValue, N02SourceError> {
    let source = projected.source();
    let event = projected.event();
    let registration = record.registration();
    source_time(index, source)?;
    Ok(CanonicalValue::Object(BTreeMap::from([
        (
            "admitted_bytes_len",
            CanonicalValue::Unsigned(record.canonical_bytes().len() as u64),
        ),
        ("admitted_content_sha256", string(content_sha256)),
        ("item_id", string(record.item_id())),
        ("rank", CanonicalValue::Unsigned(index as u64)),
        (
            "registered_feed",
            CanonicalValue::Object(BTreeMap::from([
                ("capability_name", string(registration.capability_name)),
                ("feed_name", string(registration.feed_name)),
                ("gateway_provider", string(registration.gateway_provider)),
                (
                    "max_limit",
                    CanonicalValue::Unsigned(u64::from(registration.max_limit)),
                ),
                ("provider_id", string(registration.provider_id)),
                ("source_contract", string(registration.source_contract)),
                ("upstream_revision", string(registration.upstream_revision)),
            ])),
        ),
        (
            "selected_semantics",
            CanonicalValue::Object(BTreeMap::from([
                ("ai_degraded", CanonicalValue::Bool(event.ai_degraded)),
                (
                    "certainty",
                    CanonicalValue::Unsigned(u64::from(event.certainty)),
                ),
                ("direction", string("Neutral")),
                ("event_type", string("Other")),
                (
                    "object",
                    string(event.object.as_deref().expect("validated title object")),
                ),
                (
                    "occurred_at",
                    timestamp(event.occurred_at.with_timezone(&Utc)),
                ),
                (
                    "provenance_url",
                    string(event.provenance[0].url.as_deref().expect("validated URL")),
                ),
                ("simhash", CanonicalValue::Unsigned(event.simhash)),
                ("stale", CanonicalValue::Bool(event.stale)),
                (
                    "strength",
                    CanonicalValue::Unsigned(u64::from(event.strength)),
                ),
                ("subject", string(&event.subject)),
                ("title", string(&event.full_title)),
            ])),
        ),
        (
            "source_identity",
            CanonicalValue::Object(BTreeMap::from([
                ("batch_id", string(source.batch_id())),
                ("event_id", string(source.event_id())),
                ("observed_at", timestamp(source.observed_at())),
                ("provider", string(source.provider())),
                ("published_at", timestamp(source.published_at())),
                ("source", string(source.source())),
            ])),
        ),
    ])))
}

#[cfg(test)]
mod tests;
