//! Versioned operational observations assembled from the existing health read.
//! This view grants no data, delivery, trading or day-completion authority.

use super::{HealthFileOpenError, HealthReport};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_LEASE_BYTES: u64 = 97;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

fn file_identity(metadata: &Metadata) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.file_type().is_file().then(|| FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        // No portable stable file identity is available on this target. Do not
        // substitute file times or lengths for inode evidence.
        let _ = metadata;
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseLockState {
    ExclusiveHeld,
    Unlocked,
    Unavailable,
}

fn lease_lock_state(file: &File) -> LeaseLockState {
    match fs2::FileExt::try_lock_shared(file) {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            LeaseLockState::ExclusiveHeld
        }
        Ok(()) => {
            if fs2::FileExt::unlock(file).is_ok() {
                LeaseLockState::Unlocked
            } else {
                LeaseLockState::Unavailable
            }
        }
        Err(_) => LeaseLockState::Unavailable,
    }
}

fn lease_bytes(file: &mut File) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_LEASE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_LEASE_BYTES).then_some(bytes)
}

pub(super) struct LeaseWitness {
    file: File,
    identity: FileIdentity,
    raw_bytes: Vec<u8>,
    lock_state: LeaseLockState,
    boot_identity: Option<String>,
}

/// Retains the actual descriptor until every component has been read. Raw
/// bytes, identities and lock states stay private and never enter the report.
pub(super) enum MonitorLeaseObservation {
    Unavailable,
    Invalid,
    Present(LeaseWitness),
}

impl MonitorLeaseObservation {
    pub(super) fn read(path: &Path) -> Self {
        let named = match std::fs::symlink_metadata(path) {
            Ok(metadata) => match file_identity(&metadata) {
                Some(identity) => identity,
                None => return Self::Invalid,
            },
            Err(_) => return Self::Unavailable,
        };
        let mut file = match super::open_regular_health_file(path) {
            Ok(file) => file,
            Err(HealthFileOpenError::Unavailable) => return Self::Unavailable,
            Err(HealthFileOpenError::Invalid) => return Self::Invalid,
        };
        if file.metadata().ok().and_then(|meta| file_identity(&meta)) != Some(named) {
            return Self::Invalid;
        }
        let lock_state = lease_lock_state(&file);
        let Some(raw_bytes) = lease_bytes(&mut file) else {
            return Self::Invalid;
        };
        if lease_lock_state(&file) != lock_state
            || std::fs::symlink_metadata(path)
                .ok()
                .and_then(|meta| file_identity(&meta))
                != Some(named)
            || file.metadata().ok().map(|meta| meta.len()) != Some(raw_bytes.len() as u64)
        {
            return Self::Invalid;
        }
        let boot_identity = std::str::from_utf8(&raw_bytes)
            .ok()
            .map(|raw| raw.trim_end_matches('\n'))
            .filter(|identity| super::valid_boot_id(identity))
            .map(str::to_owned);
        Self::Present(LeaseWitness {
            file,
            identity: named,
            raw_bytes,
            lock_state,
            boot_identity,
        })
    }

    pub(super) fn boot_identity(&self) -> Option<&str> {
        match self {
            Self::Present(witness) if witness.lock_state == LeaseLockState::ExclusiveHeld => {
                witness.boot_identity.as_deref()
            }
            _ => None,
        }
    }

    pub(super) fn unchanged_at(&mut self, path: &Path) -> bool {
        let after = Self::read(path);
        match (self, after) {
            (Self::Unavailable, Self::Unavailable) | (Self::Invalid, Self::Invalid) => true,
            (Self::Present(before), Self::Present(after)) => {
                before.identity == after.identity
                    && before.raw_bytes == after.raw_bytes
                    && before.lock_state == after.lock_state
                    && before.boot_identity == after.boot_identity
                    && before
                        .file
                        .metadata()
                        .ok()
                        .and_then(|meta| file_identity(&meta))
                        == Some(before.identity)
                    && before.file.metadata().ok().map(|meta| meta.len())
                        == Some(before.raw_bytes.len() as u64)
                    && lease_bytes(&mut before.file).as_deref() == Some(before.raw_bytes.as_slice())
                    && lease_lock_state(&before.file) == before.lock_state
                    && std::fs::symlink_metadata(path)
                        .ok()
                        .and_then(|meta| file_identity(&meta))
                        == Some(before.identity)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct RuntimeHealthSnapshot {
    version: u8,
    checked_at: DateTime<Utc>,
    coverage: &'static str,
    process: ProcessObservation,
    account: AccountObservation,
    data: DataObservation,
    raw_global_news: RawGlobalNewsObservation,
    durable_delivery: NotObserved,
    data_quality: NotObserved,
    other_sources: NotObserved,
    quiet_halted_policy: NotObserved,
}

#[derive(Debug, Serialize)]
struct ProcessObservation {
    coverage: &'static str,
    reason_domain: &'static str,
    status: &'static str,
    reason_code: Option<&'static str>,
    monitor_running: bool,
    boot_identity_sha256: Option<String>,
    heartbeat_observed_at: Option<DateTime<Utc>>,
    heartbeat_age_seconds: Option<i64>,
    heartbeat_fresh: bool,
}

#[derive(Debug, Serialize)]
struct AccountObservation {
    coverage: &'static str,
    reason_domain: &'static str,
    status: &'static str,
    reason_code: Option<&'static str>,
    banner_observed_at: Option<DateTime<Utc>>,
    evaluated_at: Option<DateTime<Utc>>,
    age_seconds: Option<i64>,
    mode: Option<String>,
    metrics_complete: Option<bool>,
}

#[derive(Debug, Serialize)]
struct DataObservation {
    coverage: &'static str,
    reason_domain: &'static str,
    status: &'static str,
    reason_code: Option<&'static str>,
    banner_observed_at: Option<DateTime<Utc>>,
    evaluated_at: Option<DateTime<Utc>>,
    age_seconds: Option<i64>,
    mode: Option<String>,
    missing_capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RawGlobalNewsObservation {
    reason_domain: &'static str,
    recovery: super::source_recovery::SourceRecoveryReport,
}

#[derive(Debug, Serialize)]
struct NotObserved {
    status: &'static str,
    coverage: &'static str,
}

impl NotObserved {
    fn new(coverage: &'static str) -> Self {
        Self {
            status: "not_observed",
            coverage,
        }
    }
}

fn banner_observation_error(report: &HealthReport) -> Option<&'static str> {
    if !report.snapshot_fresh {
        report.reason_code
    } else if !report.monitor_running {
        Some("monitor_not_running")
    } else if report.reason_code == Some("health_snapshot_process_mismatch") {
        report.reason_code
    } else {
        None
    }
}

impl RuntimeHealthSnapshot {
    pub(super) fn text_summary(&self) -> String {
        format!(
            "runtime_snapshot_version={} runtime_snapshot_coverage={} runtime_account_status={} runtime_data_status={} durable_delivery_status={} data_quality_status={} other_sources_status={} quiet_halted_policy_status={}",
            self.version,
            self.coverage,
            self.account.status,
            self.data.status,
            self.durable_delivery.status,
            self.data_quality.status,
            self.other_sources.status,
            self.quiet_halted_policy.status,
        )
    }

    pub(super) fn from_report(
        report: &HealthReport,
        now: DateTime<Utc>,
        lease_identity: Option<&str>,
    ) -> Self {
        let banner_error = banner_observation_error(report);
        let account_reason = banner_error.or_else(|| {
            if report.account_evaluated_at.is_none() {
                Some("account_evaluation_missing")
            } else if !super::fresh_at(report.account_evaluated_at.unwrap(), now) {
                Some("account_evaluation_stale")
            } else if report.account_mode.as_deref() != Some("Normal")
                || report.account_metrics_complete != Some(true)
            {
                Some("banner_unhealthy")
            } else {
                None
            }
        });
        let data_reason = banner_error.or_else(|| {
            if report.data_evaluated_at.is_none() {
                Some("data_evaluation_missing")
            } else if !super::fresh_at(report.data_evaluated_at.unwrap(), now) {
                Some("data_evaluation_stale")
            } else if report.data_mode.as_deref() != Some("Full")
                || !report.missing_capabilities.is_empty()
            {
                Some("banner_unhealthy")
            } else {
                None
            }
        });
        let component_status = |reason: Option<&'static str>| {
            if banner_error.is_some() {
                "unavailable"
            } else if reason.is_some() {
                "unhealthy"
            } else {
                "ok"
            }
        };
        Self {
            version: 1,
            checked_at: now,
            coverage: "readonly_process_banner_account_data_and_four_raw_global_news_sources_only",
            process: ProcessObservation {
                coverage: "monitor_lease_identity_and_process_heartbeat_only",
                reason_domain: "process_health",
                status: report.heartbeat_status,
                reason_code: report.heartbeat_reason_code,
                monitor_running: report.monitor_running,
                boot_identity_sha256: lease_identity.map(|identity| {
                    let mut hash = Sha256::new();
                    hash.update(b"stock_analysis.runtime_health.monitor_boot_identity.v1\0");
                    hash.update(identity.as_bytes());
                    hex::encode(hash.finalize())
                }),
                heartbeat_observed_at: report.heartbeat_observed_at,
                heartbeat_age_seconds: report.heartbeat_age_seconds,
                heartbeat_fresh: report.heartbeat_fresh,
            },
            account: AccountObservation {
                coverage: "banner_account_mode_and_evaluation_time_only",
                reason_domain: "banner_account_health",
                status: component_status(account_reason),
                reason_code: account_reason,
                banner_observed_at: report.observed_at,
                evaluated_at: report.account_evaluated_at,
                age_seconds: report.account_age_seconds,
                mode: report.account_mode.clone(),
                metrics_complete: report.account_metrics_complete,
            },
            data: DataObservation {
                coverage: "banner_data_mode_and_capability_freshness_only",
                reason_domain: "banner_data_health",
                status: component_status(data_reason),
                reason_code: data_reason,
                banner_observed_at: report.observed_at,
                evaluated_at: report.data_evaluated_at,
                age_seconds: report.data_age_seconds,
                mode: report.data_mode.clone(),
                missing_capabilities: report.missing_capabilities.clone(),
            },
            raw_global_news: RawGlobalNewsObservation {
                reason_domain: "raw_global_news_source_recovery",
                recovery: report.raw_news_source_recovery.clone(),
            },
            durable_delivery: NotObserved::new("durable_decision_states_and_attempt_leases"),
            data_quality: NotObserved::new("resident_scanner_data_quality_statistics"),
            other_sources: NotObserved::new(
                "sources_outside_four_registered_raw_global_news_feeds",
            ),
            quiet_halted_policy: NotObserved::new("platform_quiet_halted_run_mode"),
        }
    }
}

#[cfg(test)]
#[path = "health_cmd_runtime_snapshot_tests.rs"]
mod tests;
