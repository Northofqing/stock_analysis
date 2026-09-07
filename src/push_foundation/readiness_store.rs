//! Atomic, canonical readiness records. Persistence integrity is not source attestation.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::monitor::push_job::{
    canonical_digest, namespace_value, CanonicalValue, MachineCatalog, Namespace, Sha256Digest,
};

use super::readiness_recovery::CandidateReadinessRecord;
use super::readiness_recovery_codec::{decode_readiness_record, ReadinessRecordDecodeError};
use super::readiness_snapshot::{scope_value, CandidateReadinessSnapshot};
use super::readiness_snapshot_codec::{decode_readiness_snapshot, ReadinessDecodeError};
use super::readiness_store_schema::{with_read_only, with_write_transaction, ReadinessSchemaError};

/// Selects one continuity domain, not a permission to evaluate or execute it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadinessStreamId(Sha256Digest);

impl ReadinessStreamId {
    pub(crate) fn for_snapshot(snapshot: &CandidateReadinessSnapshot) -> Self {
        let context = snapshot.context();
        let assessment = snapshot.assessment();
        Self(canonical_digest(
            "OperationalReadinessStream/v1",
            &BTreeMap::from([
                ("namespace", namespace_value(&context.namespace)),
                ("business_date", string(context.business_date.as_str())),
                ("build_commit", string(context.build_commit.as_str())),
                (
                    "generation",
                    CanonicalValue::Unsigned(context.activation_generation),
                ),
                ("manifest_sha256", string(context.manifest_sha256.as_str())),
                (
                    "catalog_sha256",
                    string(assessment.catalog_sha256().as_str()),
                ),
                ("scope", scope_value(assessment.scope())),
                (
                    "enabled_producers",
                    CanonicalValue::Array(
                        assessment
                            .enabled_producers()
                            .iter()
                            .map(|id| string(id.as_str()))
                            .collect(),
                    ),
                ),
            ]),
        ))
    }
}

/// Only a verified persistence receipt. The contained evidence is still candidate material.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredReadinessRecord {
    stream: ReadinessStreamId,
    version: u64,
    candidate: CandidateReadinessRecord,
}

impl StoredReadinessRecord {
    pub(crate) fn version(&self) -> u64 {
        self.version
    }
    pub(crate) fn candidate(&self) -> &CandidateReadinessRecord {
        &self.candidate
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ReadinessStoreError {
    #[error(transparent)]
    Schema(#[from] ReadinessSchemaError),
    #[error(transparent)]
    Snapshot(#[from] ReadinessDecodeError),
    #[error(transparent)]
    Event(#[from] ReadinessRecordDecodeError),
    #[error("readiness record namespace does not match the selected store")]
    NamespaceMismatch,
    #[error("readiness record is missing from the committed chain")]
    RecordMissing,
    #[error("readiness head changed or does not match the proposed predecessor")]
    HeadConflict,
    #[error("readiness storage integrity check failed: {check}")]
    Corrupt { check: &'static str },
    #[error("readiness storage operation failed: {operation}")]
    Storage { operation: &'static str },
}

/// Explicit borrowed configuration; construction performs no I/O or initialization.
pub(crate) struct ReadinessRecordStore<'a> {
    path: &'a Path,
    namespace: &'a Namespace,
    catalog: &'a MachineCatalog,
}

impl<'a> ReadinessRecordStore<'a> {
    pub(crate) fn at(
        path: &'a Path,
        namespace: &'a Namespace,
        catalog: &'a MachineCatalog,
    ) -> Self {
        Self {
            path,
            namespace,
            catalog,
        }
    }

    pub(crate) fn load_head(
        &self,
        stream: &ReadinessStreamId,
    ) -> Result<Option<StoredReadinessRecord>, ReadinessStoreError> {
        with_read_only(self.path, self.namespace, |connection| {
            Ok(self
                .head_chain(connection, stream)?
                .and_then(|chain| chain.last().cloned()))
        })
    }

    pub(crate) fn load_record(
        &self,
        snapshot_id: &Sha256Digest,
    ) -> Result<StoredReadinessRecord, ReadinessStoreError> {
        with_read_only(self.path, self.namespace, |connection| {
            let chain = self.record_chain(connection, snapshot_id)?;
            let target = chain.last().ok_or(corrupt("empty_chain"))?;
            let committed = self
                .head_chain(connection, &target.stream)?
                .ok_or(ReadinessStoreError::RecordMissing)?;
            committed
                .into_iter()
                .find(|record| record == target)
                .ok_or(ReadinessStoreError::RecordMissing)
        })
    }

    pub(crate) fn append(
        &self,
        expected: Option<&StoredReadinessRecord>,
        candidate: &CandidateReadinessRecord,
    ) -> Result<StoredReadinessRecord, ReadinessStoreError> {
        let snapshot = candidate.snapshot();
        if &snapshot.context().namespace != self.namespace {
            return Err(ReadinessStoreError::NamespaceMismatch);
        }
        // Re-evaluate against this store's exact catalog before accepting a candidate from a caller.
        decode_readiness_snapshot(
            self.catalog,
            snapshot.snapshot_id(),
            &snapshot.canonical_bytes(),
        )?;
        let stream = ReadinessStreamId::for_snapshot(snapshot);
        with_write_transaction(self.path, self.namespace, |connection| {
            let chain = self.head_chain(connection, &stream)?.unwrap_or_default();
            if let Some(previous_commit) = chain
                .iter()
                .find(|record| record.candidate.snapshot().snapshot_id() == snapshot.snapshot_id())
            {
                return if &previous_commit.candidate == candidate {
                    Ok(previous_commit.clone())
                } else {
                    Err(corrupt("replay_material"))
                };
            }
            let current = chain.last();
            if current != expected
                || candidate.before_snapshot_id()
                    != current.map(|record| record.candidate.snapshot().snapshot_id())
            {
                return Err(ReadinessStoreError::HeadConflict);
            }
            let rebuilt = decode_readiness_record(
                current.map(|record| record.candidate.snapshot()),
                snapshot,
                candidate.event_sha256(),
                &candidate.event_bytes(),
            )?;
            if &rebuilt != candidate {
                return Err(corrupt("candidate_record"));
            }
            let version = current
                .map_or(0, |record| record.version)
                .checked_add(1)
                .and_then(|value| i64::try_from(value).ok())
                .ok_or(corrupt("head_version_overflow"))?;
            connection.execute(
                "INSERT INTO operational_readiness_recovery_event(event_id,event_sha256,before_snapshot_id,after_snapshot_id,canonical_bytes) VALUES(?1,?2,?3,?4,?5)",
                params![snapshot.recovery_event_id().as_str(), candidate.event_sha256().as_str(), candidate.before_snapshot_id().map(|id| id.as_str()), snapshot.snapshot_id().as_str(), candidate.event_bytes()],
            ).map_err(|_| storage("insert_event"))?;
            connection.execute(
                "INSERT INTO operational_readiness_snapshot(snapshot_id,event_id,canonical_bytes) VALUES(?1,?2,?3)",
                params![snapshot.snapshot_id().as_str(), snapshot.recovery_event_id().as_str(), snapshot.canonical_bytes()],
            ).map_err(|_| storage("insert_snapshot"))?;
            let changed = if let Some(current) = current {
                connection.execute(
                    "UPDATE operational_readiness_head SET version=?1,snapshot_id=?2,event_id=?3 WHERE scope_key=?4 AND version=?5 AND snapshot_id=?6 AND event_id=?7",
                    params![version, snapshot.snapshot_id().as_str(), snapshot.recovery_event_id().as_str(), stream.0.as_str(), current.version as i64, current.candidate.snapshot().snapshot_id().as_str(), current.candidate.snapshot().recovery_event_id().as_str()],
                )
            } else {
                connection.execute(
                    "INSERT INTO operational_readiness_head(scope_key,version,snapshot_id,event_id) VALUES(?1,?2,?3,?4)",
                    params![stream.0.as_str(), version, snapshot.snapshot_id().as_str(), snapshot.recovery_event_id().as_str()],
                )
            }.map_err(|_| storage("head_cas"))?;
            if changed != 1 {
                return Err(ReadinessStoreError::HeadConflict);
            }
            self.head_chain(connection, &stream)?
                .and_then(|chain| chain.last().cloned())
                .ok_or(corrupt("head_after_append"))
        })
    }

    fn head_chain(
        &self,
        connection: &Connection,
        stream: &ReadinessStreamId,
    ) -> Result<Option<Vec<StoredReadinessRecord>>, ReadinessStoreError> {
        let head: Option<(i64, String, String)> = connection.query_row(
            "SELECT version,snapshot_id,event_id FROM operational_readiness_head WHERE scope_key=?1",
            [stream.0.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|_| corrupt("head_row"))?;
        let Some((version, snapshot_id, event_id)) = head else {
            return Ok(None);
        };
        let chain = self.record_chain(connection, &digest(&snapshot_id)?)?;
        let last = chain.last().ok_or(corrupt("empty_chain"))?;
        if &last.stream != stream
            || i64::try_from(last.version).ok() != Some(version)
            || last.candidate.snapshot().recovery_event_id().as_str() != event_id
        {
            return Err(corrupt("head_join"));
        }
        Ok(Some(chain))
    }

    fn record_chain(
        &self,
        connection: &Connection,
        snapshot_id: &Sha256Digest,
    ) -> Result<Vec<StoredReadinessRecord>, ReadinessStoreError> {
        let mut pending = Some(snapshot_id.clone());
        let mut visited = BTreeSet::new();
        let mut rows = vec![];
        let mut stream = None;
        while let Some(id) = pending {
            if !visited.insert(id.as_str().to_owned()) {
                return Err(corrupt("chain_cycle"));
            }
            let row = connection.query_row(
                "SELECT s.event_id,s.canonical_bytes,e.event_sha256,e.before_snapshot_id,e.after_snapshot_id,e.canonical_bytes FROM operational_readiness_snapshot s JOIN operational_readiness_recovery_event e ON e.event_id=s.event_id WHERE s.snapshot_id=?1",
                [id.as_str()], |row| Ok(RawRecord {
                    event_id: row.get(0)?, snapshot_bytes: row.get(1)?, event_sha256: row.get(2)?,
                    before_snapshot_id: row.get(3)?, after_snapshot_id: row.get(4)?, event_bytes: row.get(5)?,
                }),
            ).optional().map_err(|_| corrupt("record_row"))?.ok_or(ReadinessStoreError::RecordMissing)?;
            let snapshot = decode_readiness_snapshot(self.catalog, &id, &row.snapshot_bytes)?;
            if &snapshot.context().namespace != self.namespace {
                return Err(ReadinessStoreError::NamespaceMismatch);
            }
            if snapshot.recovery_event_id().as_str() != row.event_id
                || id.as_str() != row.after_snapshot_id
            {
                return Err(corrupt("event_snapshot_join"));
            }
            let this_stream = ReadinessStreamId::for_snapshot(&snapshot);
            if stream
                .as_ref()
                .is_some_and(|expected| expected != &this_stream)
            {
                return Err(corrupt("chain_stream"));
            }
            stream = Some(this_stream);
            pending = row.before_snapshot_id.as_deref().map(digest).transpose()?;
            rows.push((snapshot, digest(&row.event_sha256)?, row.event_bytes));
        }
        let mut chain: Vec<StoredReadinessRecord> = Vec::with_capacity(rows.len());
        for (snapshot, event_sha256, event_bytes) in rows.into_iter().rev() {
            let before = chain.last().map(|record| record.candidate.snapshot());
            let candidate =
                decode_readiness_record(before, &snapshot, &event_sha256, &event_bytes)?;
            let version = u64::try_from(chain.len())
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(corrupt("chain_length"))?;
            chain.push(StoredReadinessRecord {
                stream: ReadinessStreamId::for_snapshot(&snapshot),
                version,
                candidate,
            });
        }
        Ok(chain)
    }
}

// Never derive Debug: these persistence bytes contain protected source references.
struct RawRecord {
    event_id: String,
    snapshot_bytes: Vec<u8>,
    event_sha256: String,
    before_snapshot_id: Option<String>,
    after_snapshot_id: String,
    event_bytes: Vec<u8>,
}

fn digest(value: &str) -> Result<Sha256Digest, ReadinessStoreError> {
    Sha256Digest::parse("readiness record", value).map_err(|_| corrupt("digest_field"))
}
fn corrupt(check: &'static str) -> ReadinessStoreError {
    ReadinessStoreError::Corrupt { check }
}
fn storage(operation: &'static str) -> ReadinessStoreError {
    ReadinessStoreError::Storage { operation }
}
fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}
