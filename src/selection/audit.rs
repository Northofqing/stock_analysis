//! BR-157 authoritative append-only hash-chain audit for shadow selection.

use chrono::{DateTime, FixedOffset};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use thiserror::Error;

pub const AUDIT_SCHEMA_VERSION: u16 = 1;
pub const AUDIT_DOMAIN: &str = "stock_analysis.selection_audit.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionAuditPhase {
    Ingested,
    Prepared,
    Committed,
    Rejected,
    Completed,
    T0Close,
    D1Settled,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionAuditContext {
    pub event_identity_hash: Option<String>,
    pub chain_identity_hash: Option<String>,
    pub security_identity_hash: Option<String>,
    pub provider: Option<String>,
    pub provider_published_at: Option<DateTime<FixedOffset>>,
    pub observed_at: Option<DateTime<FixedOffset>>,
    pub magic_tdx_batch_id: Option<String>,
    pub reason_codes: Vec<String>,
    pub rule_ids: Vec<String>,
    pub retryable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionAuditRecord {
    pub schema_version: u16,
    pub domain: String,
    pub phase: SelectionAuditPhase,
    pub subject_id: String,
    pub content_hash: String,
    pub context: SelectionAuditContext,
    pub previous_hash: Option<String>,
    pub recorded_at: DateTime<FixedOffset>,
    pub record_hash: String,
}

impl SelectionAuditRecord {
    pub fn new(
        phase: SelectionAuditPhase,
        subject_id: impl Into<String>,
        content_hash: impl Into<String>,
        recorded_at: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            schema_version: AUDIT_SCHEMA_VERSION,
            domain: AUDIT_DOMAIN.to_owned(),
            phase,
            subject_id: subject_id.into(),
            content_hash: content_hash.into(),
            context: SelectionAuditContext::default(),
            previous_hash: None,
            recorded_at,
            record_hash: String::new(),
        }
    }

    pub fn with_context(mut self, context: SelectionAuditContext) -> Self {
        self.context = context;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAuditEnvironment {
    Production,
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditAppendReceipt {
    pub record_hash: String,
    pub previous_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditValidationReceipt {
    pub record_count: usize,
    pub tail_hash: Option<String>,
}

#[derive(Debug, Error)]
pub enum SelectionAuditError {
    #[error("selection audit chain invalid: {0}")]
    ChainInvalid(String),
    #[error("selection audit record invalid: {0}")]
    InvalidRecord(String),
    #[error("selection audit lock failed: {0}")]
    Lock(String),
    #[error("selection audit I/O failed: {0}")]
    Io(String),
}

impl SelectionAuditError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ChainInvalid(_) => "audit_chain_invalid",
            Self::InvalidRecord(_) => "audit_record_invalid",
            Self::Lock(_) => "audit_lock_failed",
            Self::Io(_) => "audit_io_failure",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SelectionAuditWriter {
    path: PathBuf,
    lock_path: PathBuf,
}

impl SelectionAuditWriter {
    pub fn for_environment(root: impl AsRef<Path>, environment: SelectionAuditEnvironment) -> Self {
        let namespace = match environment {
            SelectionAuditEnvironment::Production => "production",
            SelectionAuditEnvironment::Test => "test",
        };
        let base = root.as_ref().join(namespace);
        Self {
            path: base.join("selection-audit.jsonl"),
            lock_path: base.join("selection-audit.lock"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    pub fn append(
        &self,
        mut record: SelectionAuditRecord,
    ) -> Result<AuditAppendReceipt, SelectionAuditError> {
        validate_new_record(&record)?;
        self.with_exclusive_lock(|| {
            let chain = validate_chain_path(&self.path)?;
            record.previous_hash = chain.tail_hash;
            record.record_hash = calculate_record_hash(&record)?;
            let serialized = serde_json::to_vec(&record).map_err(|error| {
                SelectionAuditError::InvalidRecord(format!(
                    "serialize strict selection audit record: {error}"
                ))
            })?;

            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .map_err(|error| {
                    SelectionAuditError::Io(format!(
                        "open append-only audit {}: {error}",
                        self.path.display()
                    ))
                })?;
            file.write_all(&serialized).map_err(|error| {
                SelectionAuditError::Io(format!(
                    "append audit record to {}: {error}",
                    self.path.display()
                ))
            })?;
            file.write_all(b"\n").map_err(|error| {
                SelectionAuditError::Io(format!(
                    "append audit newline to {}: {error}",
                    self.path.display()
                ))
            })?;
            file.flush().map_err(|error| {
                SelectionAuditError::Io(format!("flush audit {}: {error}", self.path.display()))
            })?;
            file.sync_data().map_err(|error| {
                SelectionAuditError::Io(format!("sync audit {}: {error}", self.path.display()))
            })?;

            Ok(AuditAppendReceipt {
                record_hash: record.record_hash.clone(),
                previous_hash: record.previous_hash.clone(),
            })
        })
    }

    pub fn validate(&self) -> Result<AuditValidationReceipt, SelectionAuditError> {
        self.with_exclusive_lock(|| validate_chain_path(&self.path))
    }

    fn with_exclusive_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, SelectionAuditError>,
    ) -> Result<T, SelectionAuditError> {
        static PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _process_guard = PROCESS_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| SelectionAuditError::Lock("process audit mutex is poisoned".to_owned()))?;

        let parent = self.path.parent().ok_or_else(|| {
            SelectionAuditError::Io(format!("audit path has no parent: {}", self.path.display()))
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            SelectionAuditError::Io(format!(
                "create audit directory {}: {error}",
                parent.display()
            ))
        })?;
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.lock_path)
            .map_err(|error| {
                SelectionAuditError::Lock(format!(
                    "open audit lock {}: {error}",
                    self.lock_path.display()
                ))
            })?;
        FileExt::lock_exclusive(&lock_file).map_err(|error| {
            SelectionAuditError::Lock(format!(
                "acquire audit lock {}: {error}",
                self.lock_path.display()
            ))
        })?;

        let result = operation();
        let unlock = FileExt::unlock(&lock_file).map_err(|error| {
            SelectionAuditError::Lock(format!(
                "release audit lock {}: {error}",
                self.lock_path.display()
            ))
        });
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

#[derive(Serialize)]
struct AuditHashPayload<'a> {
    schema_version: u16,
    domain: &'a str,
    phase: SelectionAuditPhase,
    subject_id: &'a str,
    content_hash: &'a str,
    context: &'a SelectionAuditContext,
    previous_hash: &'a Option<String>,
    recorded_at: &'a DateTime<FixedOffset>,
}

fn calculate_record_hash(record: &SelectionAuditRecord) -> Result<String, SelectionAuditError> {
    let payload = AuditHashPayload {
        schema_version: record.schema_version,
        domain: &record.domain,
        phase: record.phase,
        subject_id: &record.subject_id,
        content_hash: &record.content_hash,
        context: &record.context,
        previous_hash: &record.previous_hash,
        recorded_at: &record.recorded_at,
    };
    let bytes = serde_json::to_vec(&payload).map_err(|error| {
        SelectionAuditError::InvalidRecord(format!(
            "serialize selection audit hash payload: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.selection_audit_record.v1\0");
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn validate_new_record(record: &SelectionAuditRecord) -> Result<(), SelectionAuditError> {
    if record.previous_hash.is_some() || !record.record_hash.is_empty() {
        return Err(SelectionAuditError::InvalidRecord(
            "caller must not supply previous_hash or record_hash".to_owned(),
        ));
    }
    validate_record_fields(record, false)
}

fn validate_record_fields(
    record: &SelectionAuditRecord,
    persisted: bool,
) -> Result<(), SelectionAuditError> {
    let invalid = |message: String| {
        if persisted {
            SelectionAuditError::ChainInvalid(message)
        } else {
            SelectionAuditError::InvalidRecord(message)
        }
    };
    if record.schema_version != AUDIT_SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported schema_version {}, expected {AUDIT_SCHEMA_VERSION}",
            record.schema_version
        )));
    }
    if record.domain != AUDIT_DOMAIN {
        return Err(invalid(format!(
            "invalid audit domain {:?}, expected {AUDIT_DOMAIN:?}",
            record.domain
        )));
    }
    for (field, value) in [
        ("subject_id", record.subject_id.as_str()),
        ("content_hash", record.content_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(invalid(format!("{field} must not be empty")));
        }
    }
    for (field, value) in [
        (
            "event_identity_hash",
            record.context.event_identity_hash.as_deref(),
        ),
        (
            "chain_identity_hash",
            record.context.chain_identity_hash.as_deref(),
        ),
        (
            "security_identity_hash",
            record.context.security_identity_hash.as_deref(),
        ),
        ("provider", record.context.provider.as_deref()),
        (
            "magic_tdx_batch_id",
            record.context.magic_tdx_batch_id.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(invalid(format!("{field} must not be blank when present")));
        }
    }
    if record
        .context
        .reason_codes
        .iter()
        .any(|code| code.trim().is_empty())
    {
        return Err(invalid("reason_codes contain a blank code".to_owned()));
    }
    if record
        .context
        .rule_ids
        .iter()
        .any(|rule| rule.trim().is_empty())
    {
        return Err(invalid("rule_ids contain a blank rule".to_owned()));
    }
    if record.phase == SelectionAuditPhase::Rejected
        && (record.context.event_identity_hash.is_none()
            || record.context.reason_codes.is_empty()
            || record.context.rule_ids.is_empty()
            || record.context.retryable.is_none())
    {
        return Err(invalid(
            "rejected audit requires event identity, reasons, rule IDs, and retryable".to_owned(),
        ));
    }
    if persisted && record.record_hash.trim().is_empty() {
        return Err(SelectionAuditError::ChainInvalid(
            "persisted record_hash is empty".to_owned(),
        ));
    }
    Ok(())
}

fn validate_chain_path(path: &Path) -> Result<AuditValidationReceipt, SelectionAuditError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(AuditValidationReceipt {
                record_count: 0,
                tail_hash: None,
            });
        }
        Err(error) => {
            return Err(SelectionAuditError::Io(format!(
                "open audit {} for validation: {error}",
                path.display()
            )));
        }
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| {
        SelectionAuditError::Io(format!("read audit {}: {error}", path.display()))
    })?;
    if bytes.is_empty() {
        return Ok(AuditValidationReceipt {
            record_count: 0,
            tail_hash: None,
        });
    }
    if bytes.last() != Some(&b'\n') {
        return Err(SelectionAuditError::ChainInvalid(format!(
            "audit {} has a partial tail or missing final newline",
            path.display()
        )));
    }

    let mut expected_previous: Option<String> = None;
    let mut record_count = 0;
    for (index, line) in bytes[..bytes.len() - 1]
        .split(|byte| *byte == b'\n')
        .enumerate()
    {
        if line.is_empty() {
            return Err(SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} is empty",
                path.display(),
                index + 1
            )));
        }
        let record = serde_json::from_slice::<SelectionAuditRecord>(line).map_err(|error| {
            SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} is not a strict record: {error}",
                path.display(),
                index + 1
            ))
        })?;
        validate_record_fields(&record, true)?;
        if record.previous_hash != expected_previous {
            return Err(SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} previous_hash mismatch",
                path.display(),
                index + 1
            )));
        }
        let expected_hash = calculate_record_hash(&record).map_err(|error| {
            SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} hash calculation failed: {error}",
                path.display(),
                index + 1
            ))
        })?;
        if record.record_hash != expected_hash {
            return Err(SelectionAuditError::ChainInvalid(format!(
                "audit {} line {} record_hash mismatch",
                path.display(),
                index + 1
            )));
        }
        expected_previous = Some(record.record_hash);
        record_count += 1;
    }
    Ok(AuditValidationReceipt {
        record_count,
        tail_hash: expected_previous,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempAuditRoot(PathBuf);

    impl TempAuditRoot {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "stock-analysis-selection-audit-{label}-{}-{nonce}",
                std::process::id()
            ));
            Self(path)
        }
    }

    impl Drop for TempAuditRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn timestamp(second: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 60 * 60)
            .expect("offset")
            .with_ymd_and_hms(2026, 7, 23, 10, 0, second)
            .single()
            .expect("timestamp")
    }

    fn record(phase: SelectionAuditPhase, subject: &str) -> SelectionAuditRecord {
        SelectionAuditRecord::new(
            phase,
            subject,
            format!("{subject}-content-hash"),
            timestamp(0),
        )
        .with_context(SelectionAuditContext {
            event_identity_hash: Some(format!("{subject}-event-hash")),
            reason_codes: if phase == SelectionAuditPhase::Rejected {
                vec!["trend_alignment_failed".to_owned()]
            } else {
                Vec::new()
            },
            rule_ids: vec!["BR-157".to_owned()],
            retryable: (phase == SelectionAuditPhase::Rejected).then_some(false),
            ..SelectionAuditContext::default()
        })
    }

    fn test_writer(root: &TempAuditRoot) -> SelectionAuditWriter {
        SelectionAuditWriter::for_environment(&root.0, SelectionAuditEnvironment::Test)
    }

    #[test]
    fn prepared_then_committed_returns_chain_hash_receipt() {
        let root = TempAuditRoot::new("chain");
        let writer = test_writer(&root);
        let prepared = writer
            .append(record(SelectionAuditPhase::Prepared, "TEST_CODE_run-1"))
            .expect("prepared audit");
        let committed = writer
            .append(record(SelectionAuditPhase::Committed, "TEST_CODE_run-1"))
            .expect("committed audit");
        assert_eq!(committed.previous_hash, Some(prepared.record_hash));
        assert_eq!(
            writer.validate().expect("valid chain"),
            AuditValidationReceipt {
                record_count: 2,
                tail_hash: Some(committed.record_hash),
            }
        );
    }

    #[test]
    fn unknown_field_or_corrupted_hash_blocks_append() {
        for (label, replacement) in [
            (
                "unknown",
                r#"{"schema_version":1,"domain":"stock_analysis.selection_audit.v1","phase":"prepared","subject_id":"TEST_CODE_run","content_hash":"content","context":{"event_identity_hash":null,"chain_identity_hash":null,"security_identity_hash":null,"provider":null,"provider_published_at":null,"observed_at":null,"magic_tdx_batch_id":null,"reason_codes":[],"rule_ids":[],"retryable":null},"previous_hash":null,"recorded_at":"2026-07-23T10:00:00+08:00","record_hash":"bad","unknown":true}
"#,
            ),
            (
                "hash",
                r#"{"schema_version":1,"domain":"stock_analysis.selection_audit.v1","phase":"prepared","subject_id":"TEST_CODE_run","content_hash":"content","context":{"event_identity_hash":null,"chain_identity_hash":null,"security_identity_hash":null,"provider":null,"provider_published_at":null,"observed_at":null,"magic_tdx_batch_id":null,"reason_codes":[],"rule_ids":[],"retryable":null},"previous_hash":null,"recorded_at":"2026-07-23T10:00:00+08:00","record_hash":"bad"}
"#,
            ),
        ] {
            let root = TempAuditRoot::new(label);
            let writer = test_writer(&root);
            fs::create_dir_all(writer.path().parent().expect("parent")).expect("create dir");
            fs::write(writer.path(), replacement).expect("corrupt audit");
            let error = writer
                .append(record(SelectionAuditPhase::Committed, "TEST_CODE_run"))
                .expect_err("corrupt chain must block");
            assert_eq!(error.code(), "audit_chain_invalid");
        }
    }

    #[test]
    fn missing_final_newline_blocks_append() {
        let root = TempAuditRoot::new("newline");
        let writer = test_writer(&root);
        writer
            .append(record(SelectionAuditPhase::Prepared, "TEST_CODE_run"))
            .expect("first record");
        let bytes = fs::read(writer.path()).expect("read audit");
        fs::write(writer.path(), &bytes[..bytes.len() - 1]).expect("remove newline");

        let error = writer
            .append(record(SelectionAuditPhase::Committed, "TEST_CODE_run"))
            .expect_err("partial tail must block");
        assert_eq!(error.code(), "audit_chain_invalid");
    }

    #[test]
    fn caller_cannot_supply_previous_or_record_hash() {
        let root = TempAuditRoot::new("caller-hash");
        let writer = test_writer(&root);
        let mut supplied = record(SelectionAuditPhase::Prepared, "TEST_CODE_run");
        supplied.previous_hash = Some("caller-controlled".to_owned());
        supplied.record_hash = "caller-controlled".to_owned();
        let error = writer
            .append(supplied)
            .expect_err("writer owns linkage and hash");
        assert_eq!(error.code(), "audit_record_invalid");
    }

    #[test]
    fn production_and_test_paths_are_physically_distinct() {
        let root = TempAuditRoot::new("isolation");
        let production =
            SelectionAuditWriter::for_environment(&root.0, SelectionAuditEnvironment::Production);
        let test = SelectionAuditWriter::for_environment(&root.0, SelectionAuditEnvironment::Test);
        assert_ne!(production.path(), test.path());
        assert_ne!(production.lock_path(), test.lock_path());
        assert!(production.path().to_string_lossy().contains("production"));
        assert!(test.path().to_string_lossy().contains("test"));
    }

    #[test]
    fn concurrent_writers_form_one_valid_serialized_chain() {
        let root = TempAuditRoot::new("concurrent");
        let writer = Arc::new(test_writer(&root));
        let barrier = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|index| {
                let writer = Arc::clone(&writer);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    writer.append(SelectionAuditRecord::new(
                        SelectionAuditPhase::Prepared,
                        format!("TEST_CODE_run-{index}"),
                        format!("content-hash-{index}"),
                        timestamp(u32::try_from(index).expect("test index")),
                    ))
                })
            })
            .collect::<Vec<_>>();

        for handle in threads {
            handle
                .join()
                .expect("writer thread")
                .expect("concurrent append");
        }
        let receipt = writer.validate().expect("serialized chain");
        assert_eq!(receipt.record_count, 8);
        assert!(receipt.tail_hash.is_some());
    }

    #[test]
    fn append_writes_one_complete_json_line_and_sync_contract_is_readable() {
        let root = TempAuditRoot::new("jsonl");
        let writer = test_writer(&root);
        writer
            .append(record(SelectionAuditPhase::Rejected, "TEST_CODE_candidate"))
            .expect("append");
        let bytes = fs::read(writer.path()).expect("read");
        assert_eq!(bytes.last(), Some(&b'\n'));
        let parsed = serde_json::from_slice::<SelectionAuditRecord>(
            bytes.strip_suffix(b"\n").expect("newline"),
        )
        .expect("strict record");
        assert_eq!(
            parsed.context.reason_codes,
            ["trend_alignment_failed".to_owned()]
        );
        assert_ne!(
            parsed.recorded_at.with_timezone(&Utc),
            DateTime::<Utc>::UNIX_EPOCH
        );

        let mut file = OpenOptions::new()
            .append(true)
            .open(writer.path())
            .expect("audit remains appendable");
        file.flush().expect("flush");
    }
}
