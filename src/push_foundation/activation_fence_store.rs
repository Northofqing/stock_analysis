//! Independent effect control plane. Never appends activation-journal success events.
#![cfg_attr(not(test), allow(dead_code))]

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::Duration;

use crate::monitor::push_job::{canonical_preimage, raw_digest, CanonicalValue};
use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension};

use super::activation_fence::{
    EffectRequest, EffectResult, FenceError, OperationFact, OperationState, Scope,
};

pub(super) struct OperationStore {
    connection: Connection,
    // A separate inode avoids interfering with SQLite's own platform file locks.
    // This real descriptor remains locked until the broker and last worker are gone.
    _ownership: File,
    pub(super) fresh: bool,
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LockIdentity {
    format: String,
    canonical_database_path: String,
    database_device: u64,
    database_inode: u64,
}

struct StoredOperationRow {
    request_bytes: Vec<u8>,
    request_sha256: String,
    request_json: String,
    original_epoch: String,
    state: String,
    result_json: Option<String>,
}

struct StoredCompletionRow {
    request_sha256: String,
    original_epoch: String,
    bytes: Vec<u8>,
    sha256: String,
}

/// Constructed only after the completed effect's final result was exactly read back.
/// Publishing records those already-observed facts, not acknowledgment of this publication.
struct VerifiedWorkerCompletion {
    operation_id: String,
    request_sha256: String,
    original_epoch: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
pub(super) enum CompletionFault {
    None,
    #[cfg(test)]
    FinalResultReadFailure,
    #[cfg(test)]
    CompletionWriteAckLost,
}

fn completion_bytes(request: &EffectRequest, fact: &OperationFact) -> Result<Vec<u8>, FenceError> {
    if fact.state != OperationState::Succeeded || fact.original_epoch != request.broker_epoch {
        return Err(FenceError::Store);
    }
    let result = fact.result.as_ref().ok_or(FenceError::Store)?;
    result.validate(request)?;
    if !result.is_resolved() {
        return Err(FenceError::Store);
    }
    let mut fields = std::collections::BTreeMap::from([
        (
            "operation_id",
            CanonicalValue::String(request.operation_id.clone()),
        ),
        ("request_sha256", CanonicalValue::String(request.digest())),
        (
            "original_epoch",
            CanonicalValue::String(fact.original_epoch.clone()),
        ),
        ("state", CanonicalValue::String("Succeeded".into())),
    ]);
    let domain = match result {
        EffectResult::InitialIntent(result) => {
            fields.insert(
                "intent_id",
                CanonicalValue::String(result.intent_id.clone()),
            );
            fields.insert(
                "initial_intent_sha256",
                CanonicalValue::String(result.initial_intent_sha256.clone()),
            );
            fields.insert(
                "effect_sha256",
                CanonicalValue::String(result.effect_sha256.clone()),
            );
            "ActivationEffectWorkerCompletion/v1"
        }
        EffectResult::Generic(result) => {
            fields.extend(result.canonical_fields());
            "ActivationGenericTransportWorkerCompletion/v1"
        }
        EffectResult::BusinessRecovery(result) => {
            fields.insert("result", CanonicalValue::Object(result.canonical_fields()));
            "ActivationBusinessRecoveryWorkerCompletion/v1"
        }
    };
    Ok(canonical_preimage(domain, &fields))
}

/// Parent aliases resolve before selecting the stable sibling lock path; symlinks and
/// hardlinks cannot select alternate lock files. The parent and both files must remain
/// protected against unlink/replacement by other actors throughout the broker lifetime.
fn regular_file(path: &Path) -> Result<File, FenceError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() || meta.nlink() != 1 => return Err(FenceError::Store),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(FenceError::Store),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|_| FenceError::Store)?;
    let held = file.metadata().map_err(|_| FenceError::Store)?;
    let named = std::fs::symlink_metadata(path).map_err(|_| FenceError::Store)?;
    if !held.is_file()
        || !named.is_file()
        || held.nlink() != 1
        || named.nlink() != 1
        || (held.dev(), held.ino()) != (named.dev(), named.ino())
    {
        return Err(FenceError::Store);
    }
    Ok(file)
}

fn lock_database(path: &Path) -> Result<(std::path::PathBuf, File, std::fs::Metadata), FenceError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(FenceError::Store);
    }
    let parent = std::fs::canonicalize(path.parent().ok_or(FenceError::Store)?)
        .map_err(|_| FenceError::Store)?;
    let canonical = parent.join(path.file_name().ok_or(FenceError::Store)?);
    let mut lock_name = canonical
        .file_name()
        .ok_or(FenceError::Store)?
        .to_os_string();
    lock_name.push(".broker.lock");
    let mut ownership = regular_file(&parent.join(lock_name))?;
    ownership
        .try_lock_exclusive()
        .map_err(|_| FenceError::AlreadyOwned)?;
    let database = regular_file(&canonical)?;
    let actual = database.metadata().map_err(|_| FenceError::Store)?;
    let expected = LockIdentity {
        format: "ActivationEffectStoreLock/v1".into(),
        canonical_database_path: canonical.to_str().ok_or(FenceError::Store)?.to_owned(),
        database_device: actual.dev(),
        database_inode: actual.ino(),
    };
    let mut bytes = Vec::new();
    (&mut ownership)
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| FenceError::Store)?;
    if bytes.is_empty() {
        let encoded = serde_json::to_vec(&expected).map_err(|_| FenceError::Store)?;
        ownership
            .write_all(&encoded)
            .map_err(|_| FenceError::Store)?;
        ownership.sync_all().map_err(|_| FenceError::Store)?;
        File::open(&parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| FenceError::Store)?;
    } else {
        if bytes.len() > 16 * 1024 {
            return Err(FenceError::Store);
        }
        let stored: LockIdentity = serde_json::from_slice(&bytes).map_err(|_| FenceError::Store)?;
        if stored != expected {
            return Err(FenceError::Store);
        }
    }
    Ok((canonical, ownership, actual))
}

impl OperationStore {
    pub(super) fn open(path: &Path, epoch: &str) -> Result<Self, FenceError> {
        let (canonical, ownership, actual) = lock_database(path)?;
        let connection = Connection::open(&canonical).map_err(|_| FenceError::Store)?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(|_| FenceError::Store)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS effect_store_identity (
                 singleton INTEGER PRIMARY KEY CHECK(singleton=1), format TEXT NOT NULL,
                 canonical_path TEXT NOT NULL, device TEXT NOT NULL, inode TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS effect_broker_epochs (epoch TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS effect_operations (
                 operation_id TEXT PRIMARY KEY, request_bytes BLOB NOT NULL,
                 request_sha256 TEXT NOT NULL, request_json TEXT NOT NULL,
                 original_epoch TEXT NOT NULL, scope_json TEXT NOT NULL,
                 state TEXT NOT NULL CHECK(state IN ('Running','Succeeded','Unresolved')),
                 result_json TEXT, candidate_result_json TEXT);
             CREATE TABLE IF NOT EXISTS effect_worker_completions (
                 operation_id TEXT PRIMARY KEY, request_sha256 TEXT NOT NULL,
                 original_epoch TEXT NOT NULL, completion_bytes BLOB NOT NULL,
                 completion_sha256 TEXT NOT NULL);
             CREATE TRIGGER IF NOT EXISTS effect_worker_completions_immutable
                 BEFORE UPDATE ON effect_worker_completions BEGIN SELECT RAISE(ABORT,'immutable worker completion'); END;
             CREATE TRIGGER IF NOT EXISTS effect_worker_completions_no_delete
                 BEFORE DELETE ON effect_worker_completions BEGIN SELECT RAISE(ABORT,'immutable worker completion'); END;",
            )
            .map_err(|_| FenceError::Store)?;
        let path_text = canonical.to_str().ok_or(FenceError::Store)?;
        connection.execute("INSERT OR IGNORE INTO effect_store_identity VALUES(1,'ActivationEffectStore/v1',?1,?2,?3)",
            params![path_text, actual.dev().to_string(), actual.ino().to_string()]).map_err(|_| FenceError::Store)?;
        let identity: (String, String, String, String) = connection.query_row(
            "SELECT format,canonical_path,device,inode FROM effect_store_identity WHERE singleton=1", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(|_| FenceError::Store)?;
        if identity
            != (
                "ActivationEffectStore/v1".into(),
                path_text.into(),
                actual.dev().to_string(),
                actual.ino().to_string(),
            )
        {
            return Err(FenceError::Store);
        }
        let epochs: u64 = connection
            .query_row("SELECT count(*) FROM effect_broker_epochs", [], |r| {
                r.get(0)
            })
            .map_err(|_| FenceError::Store)?;
        connection
            .execute("INSERT INTO effect_broker_epochs VALUES(?1)", [epoch])
            .map_err(|_| FenceError::Stale)?;
        // Process death ends this in-process worker, but does not resolve a possibly committed effect.
        connection
            .execute(
                "UPDATE effect_operations SET state='Unresolved',result_json=NULL WHERE state='Running'
                 OR (state='Succeeded' AND NOT EXISTS(SELECT 1 FROM effect_worker_completions c WHERE c.operation_id=effect_operations.operation_id))",
                [],
            )
            .map_err(|_| FenceError::Store)?;
        let store = Self {
            connection,
            _ownership: ownership,
            fresh: epochs == 0,
        };
        // Presence alone is insufficient. Refuse startup if a surviving terminal row's
        // completion bytes do not exactly attest its request, epoch and completed result.
        {
            let mut statement = store
                .connection
                .prepare("SELECT request_json FROM effect_operations WHERE state='Succeeded'")
                .map_err(|_| FenceError::Store)?;
            let rows = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|_| FenceError::Store)?;
            for row in rows {
                let request: EffectRequest =
                    serde_json::from_str(&row.map_err(|_| FenceError::Store)?)
                        .map_err(|_| FenceError::Store)?;
                if !matches!(
                    store.query(&request)?,
                    Some(OperationFact {
                        state: OperationState::Succeeded,
                        ..
                    })
                ) {
                    return Err(FenceError::Store);
                }
            }
        }
        Ok(store)
    }

    pub(super) fn query(
        &self,
        request: &EffectRequest,
    ) -> Result<Option<OperationFact>, FenceError> {
        let Some(mut fact) = self.recorded_result(request)? else {
            return Ok(None);
        };
        if fact.state == OperationState::Succeeded && !self.has_completion(request, &fact)? {
            fact.state = OperationState::Unresolved;
            fact.result = None;
        }
        Ok(Some(fact))
    }

    fn recorded_result(
        &self,
        request: &EffectRequest,
    ) -> Result<Option<OperationFact>, FenceError> {
        let row: Option<StoredOperationRow> = self.connection.query_row(
            "SELECT request_bytes,request_sha256,request_json,original_epoch,state,result_json FROM effect_operations WHERE operation_id=?1",
            [&request.operation_id], |r| Ok(StoredOperationRow {
                request_bytes:r.get(0)?,request_sha256:r.get(1)?,request_json:r.get(2)?,
                original_epoch:r.get(3)?,state:r.get(4)?,result_json:r.get(5)? }))
            .optional().map_err(|_| FenceError::Store)?;
        let Some(StoredOperationRow {
            request_bytes: bytes,
            request_sha256: digest,
            request_json: json,
            original_epoch: epoch,
            state,
            result_json: result,
        }) = row
        else {
            return Ok(None);
        };
        let persisted: EffectRequest =
            serde_json::from_str(&json).map_err(|_| FenceError::Store)?;
        if persisted.canonical_bytes() != bytes
            || persisted.digest() != digest
            || persisted.broker_epoch != epoch
        {
            return Err(FenceError::Store);
        }
        if request.canonical_bytes() != bytes {
            return Err(FenceError::Conflict);
        }
        let state = match state.as_str() {
            "Running" => OperationState::Running,
            "Unresolved" => OperationState::Unresolved,
            "Succeeded" => OperationState::Succeeded,
            _ => return Err(FenceError::Store),
        };
        let result: Option<EffectResult> = result
            .map(|v| serde_json::from_str(&v).map_err(|_| FenceError::Store))
            .transpose()?;
        if let Some(result) = &result {
            result.validate(request)?;
        }
        if (state == OperationState::Succeeded)
            != result.as_ref().is_some_and(EffectResult::is_resolved)
            || (state == OperationState::Running && result.is_some())
        {
            return Err(FenceError::Store);
        }
        Ok(Some(OperationFact {
            original_epoch: epoch,
            state,
            result,
        }))
    }

    pub(super) fn register(&self, request: &EffectRequest) -> Result<(), FenceError> {
        self.connection
            .execute(
                "INSERT INTO effect_operations VALUES(?1,?2,?3,?4,?5,?6,'Running',NULL,NULL)",
                params![
                    request.operation_id,
                    request.canonical_bytes(),
                    request.digest(),
                    serde_json::to_string(request).map_err(|_| FenceError::Store)?,
                    request.broker_epoch,
                    serde_json::to_string(&request.scope).map_err(|_| FenceError::Store)?
                ],
            )
            .map_err(|_| FenceError::Store)?;
        match self.query(request)? {
            Some(fact) if fact.state == OperationState::Running => Ok(()),
            _ => Err(FenceError::Store),
        }
    }

    pub(super) fn stage_result(
        &self,
        request: &EffectRequest,
        fact: &OperationFact,
    ) -> Result<(), FenceError> {
        let candidate = serde_json::to_string(fact).map_err(|_| FenceError::Store)?;
        let rows = self.connection.execute(
            "UPDATE effect_operations SET candidate_result_json=?2 WHERE operation_id=?1 AND state='Running' AND request_sha256=?3",
            params![request.operation_id,candidate,request.digest()]).map_err(|_| FenceError::Store)?;
        let stored: String = self
            .connection
            .query_row(
                "SELECT candidate_result_json FROM effect_operations WHERE operation_id=?1",
                [&request.operation_id],
                |r| r.get(0),
            )
            .map_err(|_| FenceError::Store)?;
        if rows != 1 || stored != candidate {
            return Err(FenceError::Store);
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        request: &EffectRequest,
        fact: &OperationFact,
        fault: CompletionFault,
    ) -> Result<(), FenceError> {
        let state = match fact.state {
            OperationState::Succeeded => "Succeeded",
            OperationState::Unresolved => "Unresolved",
            OperationState::Running => return Err(FenceError::Store),
        };
        let result = fact
            .result
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| FenceError::Store)?;
        let rows = self.connection.execute(
            "UPDATE effect_operations SET state=?2,result_json=?3 WHERE operation_id=?1 AND state='Running' AND request_sha256=?4",
            params![request.operation_id,state,result,request.digest()]).map_err(|_| FenceError::Store)?;
        if rows != 1 {
            return Err(FenceError::Store);
        }
        #[cfg(test)]
        if matches!(fault, CompletionFault::FinalResultReadFailure) {
            // The real final UPDATE above committed. Fail its confirmation read deterministically
            // inside SQLite, without changing the stored request/result or the normal query path.
            return self.connection.query_row(
                "SELECT fixture_absent_confirmation_column FROM effect_operations WHERE operation_id=?1",
                [&request.operation_id], |_| Ok(())).map_err(|_| FenceError::Store);
        }
        if self.recorded_result(request)?.as_ref() != Some(fact) {
            return Err(FenceError::Store);
        }
        if fact.state == OperationState::Unresolved {
            return Ok(());
        }
        let verified = VerifiedWorkerCompletion {
            operation_id: request.operation_id.clone(),
            request_sha256: request.digest(),
            original_epoch: fact.original_epoch.clone(),
            bytes: completion_bytes(request, fact)?,
        };
        self.publish_completion(verified, request, fact, fault)
    }

    fn publish_completion(
        &self,
        verified: VerifiedWorkerCompletion,
        request: &EffectRequest,
        fact: &OperationFact,
        fault: CompletionFault,
    ) -> Result<(), FenceError> {
        let written = self.connection.execute(
            "INSERT OR IGNORE INTO effect_worker_completions VALUES(?1,?2,?3,?4,?5)",
            params![
                verified.operation_id,
                verified.request_sha256,
                verified.original_epoch,
                verified.bytes,
                raw_digest(&verified.bytes).as_str()
            ],
        );
        #[cfg(test)]
        let written = if matches!(fault, CompletionFault::CompletionWriteAckLost) {
            // Actual proof publication happened; discard only its write acknowledgment.
            written.map_err(|_| FenceError::Store)?;
            Err(rusqlite::Error::InvalidQuery)
        } else {
            written
        };
        #[cfg(not(test))]
        let _ = fault;
        // Whether the write returned success or an ambiguous error, an exact durable proof
        // resolves publication. Its validity comes from the PRECEDING result read, so there
        // is no recursive requirement to record acknowledgment of the proof's own write.
        let _ = written;
        if self.has_completion(request, fact)? {
            Ok(())
        } else {
            Err(FenceError::Store)
        }
    }

    fn has_completion(
        &self,
        request: &EffectRequest,
        fact: &OperationFact,
    ) -> Result<bool, FenceError> {
        let expected = completion_bytes(request, fact)?;
        let row: Option<StoredCompletionRow> = self.connection.query_row(
            "SELECT request_sha256,original_epoch,completion_bytes,completion_sha256 FROM effect_worker_completions WHERE operation_id=?1",
            [&request.operation_id], |r| Ok(StoredCompletionRow {request_sha256:r.get(0)?,original_epoch:r.get(1)?,bytes:r.get(2)?,sha256:r.get(3)?}))
            .optional().map_err(|_| FenceError::Store)?;
        let Some(StoredCompletionRow {
            request_sha256: request_sha,
            original_epoch: epoch,
            bytes,
            sha256: digest,
        }) = row
        else {
            return Ok(false);
        };
        if request_sha != request.digest()
            || epoch != fact.original_epoch
            || bytes != expected
            || digest != raw_digest(&bytes).as_str()
        {
            return Err(FenceError::Store);
        }
        Ok(true)
    }

    pub(super) fn unresolved(&self, scope: &Scope) -> Result<u64, FenceError> {
        // An old generation remains a responsibility of this same namespace/Unit. A restart
        // with a new tuple must not hide it by selecting only the new generation's empty rows.
        let mut statement = self
            .connection
            .prepare("SELECT scope_json,request_json FROM effect_operations")
            .map_err(|_| FenceError::Store)?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| FenceError::Store)?;
        let mut count = 0;
        for row in rows {
            let (scope_json, request_json) = row.map_err(|_| FenceError::Store)?;
            let old: Scope = serde_json::from_str(&scope_json).map_err(|_| FenceError::Store)?;
            let request: EffectRequest =
                serde_json::from_str(&request_json).map_err(|_| FenceError::Store)?;
            let fact = self.query(&request)?.ok_or(FenceError::Store)?;
            if request.scope != old {
                return Err(FenceError::Store);
            }
            if old.namespace == scope.namespace
                && old.unit == scope.unit
                && fact.state != OperationState::Succeeded
            {
                count += 1;
            }
        }
        Ok(count)
    }

    #[cfg(test)]
    pub(super) fn fail_registration(&self) {
        self.connection.execute_batch("CREATE TRIGGER fixture_registration_failure BEFORE INSERT ON effect_operations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    }
}
