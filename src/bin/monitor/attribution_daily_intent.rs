//! Frozen A-12 report selection before counted notification preparation.
//!
//! The report database remains the attribution authority. This create-once
//! file binds the selected immutable Markdown revision to the exact summary
//! bytes offered to counted delivery. A partial or changed file fails closed.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "attribution-daily-frozen-intent-v1";
const RESERVATION_SCHEMA: &str = "attribution-daily-preparation-reservation-v1";

#[derive(Debug)]
pub(super) struct PreparedAttributionDaily {
    pub summary: String,
    pub report_revision_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FrozenAttributionDaily {
    schema: String,
    business_date: String,
    report_revision_file: String,
    report_sha256: String,
    report_bytes: usize,
    summary: String,
    summary_sha256: String,
    summary_bytes: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PreparationReservation {
    schema: String,
    business_date: String,
}

impl FrozenAttributionDaily {
    pub(super) fn summary(&self) -> &str {
        &self.summary
    }

    pub(super) fn report_revision_file(&self) -> &str {
        &self.report_revision_file
    }
}

#[derive(Debug)]
pub(super) enum FreezeError<E> {
    Prepare(E),
    Storage(String),
}

pub(super) struct AttributionDailyIntentStore {
    report_directory: PathBuf,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl AttributionDailyIntentStore {
    pub(super) fn new(report_directory: &Path) -> Self {
        Self {
            report_directory: report_directory.to_owned(),
        }
    }

    fn intent_path(&self, date: NaiveDate) -> PathBuf {
        self.report_directory
            .join("daily-notification-intents")
            .join(format!("{date}.json"))
    }

    fn reservation_path(&self, date: NaiveDate) -> PathBuf {
        self.report_directory
            .join("daily-notification-intents")
            .join(format!("{date}.prepare"))
    }

    fn inspect_reservation(&self, date: NaiveDate) -> Result<bool, String> {
        let path = self.reservation_path(date);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("A-12 inspect preparation reservation: {error}")),
        };
        if !metadata.file_type().is_file() {
            return Err("A-12 preparation reservation is not a regular file".to_owned());
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("A-12 read preparation reservation: {error}"))?;
        let reservation: PreparationReservation = serde_json::from_slice(&bytes)
            .map_err(|error| format!("A-12 parse preparation reservation: {error}"))?;
        if reservation
            != (PreparationReservation {
                schema: RESERVATION_SCHEMA.to_owned(),
                business_date: date.to_string(),
            })
        {
            return Err("A-12 preparation reservation schema/date mismatch".to_owned());
        }
        Ok(true)
    }

    /// Reserve the day immediately before the first database side effect.
    /// A crash after this point but before the frozen intent is complete must
    /// not recalculate market data or attempt to repair a possible DB commit.
    pub(super) fn reserve_before_commit(&self, date: NaiveDate) -> Result<(), String> {
        if self.inspect_reservation(date)? {
            return Err("A-12 preparation reservation already exists".to_owned());
        }
        fs::create_dir_all(&self.report_directory)
            .map_err(|error| format!("A-12 create report directory: {error}"))?;
        if let Some(parent) = self.report_directory.parent() {
            OpenOptions::new()
                .read(true)
                .open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("A-12 sync report directory parent: {error}"))?;
        }
        let path = self.reservation_path(date);
        let intent_directory = path.parent().expect("fixed A-12 reservation has a parent");
        fs::create_dir_all(intent_directory)
            .map_err(|error| format!("A-12 create intent directory: {error}"))?;
        OpenOptions::new()
            .read(true)
            .open(&self.report_directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("A-12 sync intent directory parent: {error}"))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("A-12 create-once preparation reservation: {error}"))?;
        let bytes = serde_json::to_vec(&PreparationReservation {
            schema: RESERVATION_SCHEMA.to_owned(),
            business_date: date.to_string(),
        })
        .map_err(|error| format!("A-12 serialize preparation reservation: {error}"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("A-12 sync preparation reservation: {error}"))?;
        OpenOptions::new()
            .read(true)
            .open(intent_directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("A-12 sync preparation reservation directory: {error}"))?;
        Ok(())
    }

    fn validate(
        &self,
        date: NaiveDate,
        frozen: FrozenAttributionDaily,
    ) -> Result<FrozenAttributionDaily, String> {
        if frozen.schema != SCHEMA || frozen.business_date != date.to_string() {
            return Err("A-12 frozen intent schema/date mismatch".to_owned());
        }
        if frozen.summary.is_empty()
            || frozen.summary_bytes != frozen.summary.len()
            || digest(frozen.summary.as_bytes()) != frozen.summary_sha256
        {
            return Err("A-12 frozen summary bytes/hash mismatch".to_owned());
        }
        if frozen.report_sha256.len() != 64
            || !frozen
                .report_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("A-12 frozen report hash encoding invalid".to_owned());
        }
        let expected_file = format!("{date}.{}.md", frozen.report_sha256);
        if frozen.report_revision_file != expected_file {
            return Err("A-12 frozen report revision identity mismatch".to_owned());
        }
        let report_path = self.report_directory.join(&expected_file);
        let metadata = fs::symlink_metadata(&report_path)
            .map_err(|error| format!("A-12 inspect frozen report revision: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("A-12 frozen report revision is not a regular file".to_owned());
        }
        let bytes = fs::read(&report_path)
            .map_err(|error| format!("A-12 read frozen report revision: {error}"))?;
        if bytes.len() != frozen.report_bytes || digest(&bytes) != frozen.report_sha256 {
            return Err("A-12 frozen report revision bytes/hash mismatch".to_owned());
        }
        Ok(frozen)
    }

    pub(super) fn load(&self, date: NaiveDate) -> Result<Option<FrozenAttributionDaily>, String> {
        let path = self.intent_path(date);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("A-12 inspect frozen intent: {error}")),
        };
        if !metadata.file_type().is_file() {
            return Err("A-12 frozen intent is not a regular file".to_owned());
        }
        let bytes = fs::read(&path).map_err(|error| format!("A-12 read frozen intent: {error}"))?;
        let frozen: FrozenAttributionDaily = serde_json::from_slice(&bytes)
            .map_err(|error| format!("A-12 parse frozen intent: {error}"))?;
        if !self.inspect_reservation(date)? {
            return Err("A-12 frozen intent has no preparation reservation".to_owned());
        }
        self.validate(date, frozen).map(Some)
    }

    fn has_unbound_report_revision(&self, date: NaiveDate) -> Result<bool, String> {
        let entries = match fs::read_dir(&self.report_directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("A-12 scan report revisions: {error}")),
        };
        let prefix = format!("{date}.");
        for entry in entries {
            let entry = entry.map_err(|error| format!("A-12 read report directory: {error}"))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.starts_with(&prefix) && name.ends_with(".md")) || name == format!("{date}.md")
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn freeze(
        &self,
        date: NaiveDate,
        prepared: PreparedAttributionDaily,
    ) -> Result<FrozenAttributionDaily, String> {
        if !self.inspect_reservation(date)? {
            return Err("A-12 report preparation was not reserved before commit".to_owned());
        }
        let metadata = fs::symlink_metadata(&prepared.report_revision_path)
            .map_err(|error| format!("A-12 inspect prepared report revision: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("A-12 prepared report revision is not a regular file".to_owned());
        }
        let report_bytes = fs::read(&prepared.report_revision_path)
            .map_err(|error| format!("A-12 read prepared report revision: {error}"))?;
        let report_sha256 = digest(&report_bytes);
        let report_revision_file = format!("{date}.{report_sha256}.md");
        if prepared.report_revision_path != self.report_directory.join(&report_revision_file) {
            return Err("A-12 prepared report revision path does not match its bytes".to_owned());
        }
        let frozen = FrozenAttributionDaily {
            schema: SCHEMA.to_owned(),
            business_date: date.to_string(),
            report_revision_file,
            report_sha256,
            report_bytes: report_bytes.len(),
            summary_sha256: digest(prepared.summary.as_bytes()),
            summary_bytes: prepared.summary.len(),
            summary: prepared.summary,
        };
        let frozen = self.validate(date, frozen)?;

        // The report file was synced by persist_report_revision; sync its name
        // before a durable intent can refer to that directory entry.
        OpenOptions::new()
            .read(true)
            .open(&self.report_directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("A-12 sync report revision directory: {error}"))?;
        let path = self.intent_path(date);
        let intent_directory = path.parent().expect("fixed A-12 intent path has a parent");
        fs::create_dir_all(intent_directory)
            .map_err(|error| format!("A-12 create intent directory: {error}"))?;
        OpenOptions::new()
            .read(true)
            .open(&self.report_directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("A-12 sync intent directory parent: {error}"))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("A-12 create-once frozen intent: {error}"))?;
        let bytes = serde_json::to_vec(&frozen)
            .map_err(|error| format!("A-12 serialize frozen intent: {error}"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("A-12 sync frozen intent: {error}"))?;
        OpenOptions::new()
            .read(true)
            .open(intent_directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("A-12 sync frozen intent directory: {error}"))?;
        Ok(frozen)
    }

    pub(super) fn load_or_freeze<E, F>(
        &self,
        date: NaiveDate,
        prepare: F,
    ) -> Result<FrozenAttributionDaily, FreezeError<E>>
    where
        F: FnOnce(&Self) -> Result<PreparedAttributionDaily, FreezeError<E>>,
    {
        if let Some(frozen) = self.load(date).map_err(FreezeError::Storage)? {
            return Ok(frozen);
        }
        if self
            .inspect_reservation(date)
            .map_err(FreezeError::Storage)?
        {
            return Err(FreezeError::Storage(
                "A-12 preparation reservation exists without frozen summary; manual review required"
                    .to_owned(),
            ));
        }
        if self
            .has_unbound_report_revision(date)
            .map_err(FreezeError::Storage)?
        {
            return Err(FreezeError::Storage(
                "A-12 report revision exists without a frozen notification intent; manual review required"
                    .to_owned(),
            ));
        }
        let prepared = prepare(self)?;
        self.freeze(date, prepared).map_err(FreezeError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AttributionDailyIntentStore, FreezeError, FrozenAttributionDaily, PreparedAttributionDaily,
    };
    use chrono::NaiveDate;
    use std::fs;
    use stock_analysis::performance::report::persist_report_revision;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
    }

    fn freeze_once(store: &AttributionDailyIntentStore) -> FrozenAttributionDaily {
        store
            .load_or_freeze(date(), |store| {
                store
                    .reserve_before_commit(date())
                    .map_err(FreezeError::<String>::Storage)?;
                let report_revision_path = persist_report_revision(
                    &store.report_directory,
                    date(),
                    b"TEST_CODE_attribution_report_revision_A",
                )
                .map_err(FreezeError::<String>::Storage)?;
                Ok(PreparedAttributionDaily {
                    summary: "TEST_CODE_exact attribution summary A\n".to_owned(),
                    report_revision_path,
                })
            })
            .unwrap()
    }

    #[test]
    fn frozen_revision_and_exact_summary_survive_restart_without_prepare() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttributionDailyIntentStore::new(dir.path());
        let first = freeze_once(&store);
        let restarted = AttributionDailyIntentStore::new(dir.path());
        let restored: FrozenAttributionDaily = restarted
            .load_or_freeze(date(), |_| -> Result<_, FreezeError<String>> {
                panic!("frozen A-12 summary must bypass recomputation")
            })
            .unwrap();
        assert_eq!(first, restored);
        assert_eq!(
            restored.summary(),
            "TEST_CODE_exact attribution summary A\n"
        );
        assert_eq!(
            fs::read(dir.path().join(restored.report_revision_file())).unwrap(),
            b"TEST_CODE_attribution_report_revision_A"
        );
    }

    #[test]
    fn reservation_without_frozen_intent_blocks_recomputation_after_crash() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttributionDailyIntentStore::new(dir.path());
        store.reserve_before_commit(date()).unwrap();
        let restarted = AttributionDailyIntentStore::new(dir.path());
        let result: Result<_, FreezeError<String>> = restarted.load_or_freeze(date(), |_| {
            panic!("a possible database commit must never be recomputed")
        });
        assert!(matches!(result, Err(FreezeError::Storage(_))));
    }

    #[test]
    fn unbound_report_revision_blocks_recomputation() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttributionDailyIntentStore::new(dir.path());
        persist_report_revision(dir.path(), date(), b"TEST_CODE_prior_report").unwrap();
        let result: Result<_, FreezeError<String>> = store.load_or_freeze(date(), |_| {
            panic!("existing report revision must not be replaced")
        });
        assert!(matches!(result, Err(FreezeError::Storage(_))));
    }

    #[test]
    fn partial_intent_and_mutated_report_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttributionDailyIntentStore::new(dir.path());
        let first = freeze_once(&store);
        let report_path = dir.path().join(first.report_revision_file());
        fs::write(&report_path, b"TEST_CODE_mutated_report").unwrap();
        let result: Result<_, FreezeError<String>> = store.load_or_freeze(date(), |_| {
            panic!("changed report must not trigger recomputation")
        });
        assert!(matches!(result, Err(FreezeError::Storage(_))));

        fs::write(store.intent_path(date()), b"{\"truncated\":").unwrap();
        let result: Result<_, FreezeError<String>> = store.load_or_freeze(date(), |_| {
            panic!("partial intent must not trigger recomputation")
        });
        assert!(matches!(result, Err(FreezeError::Storage(_))));
    }

    #[test]
    fn changed_summary_bytes_fail_closed_without_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttributionDailyIntentStore::new(dir.path());
        freeze_once(&store);
        let path = store.intent_path(date());
        let mut intent: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        intent["summary"] = serde_json::Value::String("TEST_CODE_other summary".to_owned());
        fs::write(&path, serde_json::to_vec(&intent).unwrap()).unwrap();

        let result: Result<_, FreezeError<String>> = store.load_or_freeze(date(), |_| {
            panic!("changed frozen summary must not be regenerated")
        });
        assert!(matches!(result, Err(FreezeError::Storage(_))));
        assert_eq!(
            fs::read(path).unwrap(),
            serde_json::to_vec(&intent).unwrap()
        );
    }

    #[test]
    fn failure_before_reservation_can_retry_but_after_reservation_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttributionDailyIntentStore::new(dir.path());
        let before: Result<_, FreezeError<&str>> =
            store.load_or_freeze(date(), |_| Err(FreezeError::Prepare("prices unavailable")));
        assert!(matches!(
            before,
            Err(FreezeError::Prepare("prices unavailable"))
        ));
        let after: Result<_, FreezeError<&str>> = store.load_or_freeze(date(), |store| {
            store
                .reserve_before_commit(date())
                .map_err(FreezeError::Storage)?;
            Err(FreezeError::Prepare("database result uncertain"))
        });
        assert!(matches!(
            after,
            Err(FreezeError::Prepare("database result uncertain"))
        ));
        let retry: Result<_, FreezeError<&str>> = store.load_or_freeze(date(), |_| {
            panic!("reservation must guard a failed database commit")
        });
        assert!(matches!(retry, Err(FreezeError::Storage(_))));
    }
}
