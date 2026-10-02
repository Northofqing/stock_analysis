//! Bounded, same-boot delivery-state observations; no delivery authority.

use super::*;
use crate::durable_delivery_runtime::CachedDeliveryObservation;
use std::io::{Seek, SeekFrom};
use std::sync::Arc;
use stock_analysis::durable_delivery::{
    DecisionState, DeliveryStatusSnapshot, DELIVERY_STATUS_STATES,
};

const VERSION: u8 = 1;
const COVERAGE: &str = "sqlite_decision_states_and_attempt_leases_only_no_reconcile_or_delivery";
const OBSERVATION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(1);

/// Existing typed durable error variants, plus finite observer failures. No
/// original Display, SQL, filesystem path or identity may enter this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    InvalidConfiguration,
    IsolationViolation,
    InvalidEnvelope,
    PolicyMismatch,
    DecisionIdentityConflict,
    DecisionNotFound,
    IllegalTransition,
    InvalidManualResolution,
    ImmutableAppendConflict,
    AuditPredecessorBlocked,
    Sqlite,
    Serialization,
    Io,
    RuntimeNamespace,
    RuntimeUnavailable,
    ReaderDeadlineExceeded,
    ReaderTaskFailed,
    ProcessMismatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateCount {
    state: DecisionState,
    count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Counts {
    observed_at: DateTime<Utc>,
    state_counts: [StateCount; 14],
    total_decisions: u64,
    locally_pending_decisions: u64,
    deliverable_decisions: u64,
    non_progressable_foreign_attempts: u64,
    non_progressable_manual_reviews: u64,
}

impl From<DeliveryStatusSnapshot> for Counts {
    fn from(value: DeliveryStatusSnapshot) -> Self {
        Self {
            observed_at: value.observed_at,
            state_counts: value.state_counts.map(|entry| StateCount {
                state: entry.state,
                count: entry.count,
            }),
            total_decisions: value.total_decisions,
            locally_pending_decisions: value.locally_pending_decisions,
            deliverable_decisions: value.deliverable_decisions,
            non_progressable_foreign_attempts: value.non_progressable_foreign_attempts,
            non_progressable_manual_reviews: value.non_progressable_manual_reviews,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Observed,
    NotInitialized,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u8,
    boot_id: String,
    published_at: DateTime<Utc>,
    status: Status,
    failure: Option<Failure>,
    counts: Option<Counts>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct DeliveryHealthReport {
    version: u8,
    coverage: &'static str,
    reason_domain: &'static str,
    pub(super) status: &'static str,
    reason_code: Option<&'static str>,
    failure: Option<Failure>,
    published_at: Option<DateTime<Utc>>,
    observed_at: Option<DateTime<Utc>>,
    age_seconds: Option<i64>,
    counts: Option<Counts>,
}

impl DeliveryHealthReport {
    pub(super) fn text_summary(&self) -> String {
        let counts = self.counts.as_ref().map(|counts| {
            format!(
                "durable_delivery_total={} durable_delivery_locally_pending={} durable_delivery_deliverable={} durable_delivery_foreign_live_attempts={} durable_delivery_manual_reviews={}",
                counts.total_decisions,
                counts.locally_pending_decisions,
                counts.deliverable_decisions,
                counts.non_progressable_foreign_attempts,
                counts.non_progressable_manual_reviews,
            )
        }).unwrap_or_else(|| "durable_delivery_counts=not_observed".to_owned());
        format!(
            "durable_delivery_status={} durable_delivery_reason={} durable_delivery_failure={:?} {}",
            self.status,
            self.reason_code.unwrap_or("none"),
            self.failure,
            counts,
        )
    }

    pub(super) fn with_reason(reason: &'static str) -> Self {
        Self {
            version: VERSION,
            coverage: COVERAGE,
            reason_domain: "durable_delivery_state_machine",
            status: "not_observed",
            reason_code: Some(reason),
            failure: None,
            published_at: None,
            observed_at: None,
            age_seconds: None,
            counts: None,
        }
    }
}

fn path(root: &Path, test_mode: bool) -> PathBuf {
    root.join("data")
        .join(if test_mode { "test/health" } else { "health" })
        .join("durable-delivery-health-v1.json")
}

fn validate(snapshot: &Snapshot) -> Result<(), &'static str> {
    let invalid = "durable_delivery_snapshot_invalid";
    if snapshot.version != VERSION || !valid_heartbeat_boot_id(&snapshot.boot_id) {
        return Err(invalid);
    }
    match (&snapshot.status, &snapshot.failure, &snapshot.counts) {
        (Status::NotInitialized, None, None) | (Status::Unavailable, Some(_), None) => {
            return Ok(())
        }
        (Status::Observed, None, Some(_)) => {}
        _ => return Err(invalid),
    }
    let counts = snapshot.counts.as_ref().unwrap();
    if !fresh_at(counts.observed_at, snapshot.published_at) {
        return Err(invalid);
    }
    let mut total = 0_u64;
    for (entry, expected) in counts.state_counts.iter().zip(DELIVERY_STATUS_STATES) {
        if entry.state != expected {
            return Err(invalid);
        }
        total = total.checked_add(entry.count).ok_or(invalid)?;
    }
    let count = |state| {
        counts
            .state_counts
            .iter()
            .find(|entry| entry.state == state)
            .unwrap()
            .count
    };
    let transient = [
        DecisionState::AttemptInFlight,
        DecisionState::AcceptedAuditPending,
        DecisionState::AcceptedTaskTransitionPending,
        DecisionState::RejectedAuditPending,
        DecisionState::RejectedTaskTransitionPending,
        DecisionState::UncertainAuditPending,
        DecisionState::UncertainTaskTransitionPending,
        DecisionState::ManualRejectedAuditPending,
        DecisionState::ManualRejectedTaskTransitionPending,
    ]
    .into_iter()
    .try_fold(0_u64, |sum, state| sum.checked_add(count(state)))
    .ok_or(invalid)?;
    let pending = counts
        .locally_pending_decisions
        .checked_add(counts.non_progressable_foreign_attempts)
        .ok_or(invalid)?;
    let deliverable_max = count(DecisionState::Reserved)
        .checked_add(count(DecisionState::RejectedDurable))
        .ok_or(invalid)?;
    if total != counts.total_decisions
        || pending != transient
        || counts.non_progressable_foreign_attempts > count(DecisionState::AttemptInFlight)
        || counts.non_progressable_manual_reviews != count(DecisionState::UncertainManualReview)
        || counts.deliverable_decisions < count(DecisionState::Reserved)
        || counts.deliverable_decisions > deliverable_max
    {
        return Err(invalid);
    }
    Ok(())
}

fn snapshot(boot_id: &str, now: DateTime<Utc>, observation: CachedDeliveryObservation) -> Snapshot {
    let (status, failure, counts) = match observation {
        CachedDeliveryObservation::NotInitialized => (Status::NotInitialized, None, None),
        CachedDeliveryObservation::Observed(value) => (Status::Observed, None, Some(value.into())),
        CachedDeliveryObservation::Unavailable(failure) => {
            (Status::Unavailable, Some(failure), None)
        }
    };
    Snapshot {
        version: VERSION,
        boot_id: boot_id.to_owned(),
        published_at: now,
        status,
        failure,
        counts,
    }
}

fn publish_at(root: &Path, test_mode: bool, value: &Snapshot) -> Result<(), Failure> {
    validate(value).map_err(|_| Failure::InvalidEnvelope)?;
    let lease_path = super::lease_path(root, test_mode);
    let mut lease = runtime_snapshot::MonitorLeaseObservation::read(&lease_path);
    if lease.boot_identity() != Some(value.boot_id.as_str()) {
        return Err(Failure::ProcessMismatch);
    }
    let bytes = serde_json::to_vec(value).map_err(|_| Failure::Serialization)?;
    super::atomic_replace_bytes_with(&path(root, test_mode), &bytes, "durable health", || {
        if lease.unchanged_at(&lease_path) {
            Ok(())
        } else {
            Err("durable_delivery_process_mismatch".to_owned())
        }
    })
    .map_err(|_| Failure::Io)
}

fn identity(metadata: &std::fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata
            .is_file()
            .then_some((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn read_at(path: &Path) -> Result<Snapshot, &'static str> {
    let invalid = "durable_delivery_snapshot_invalid";
    let unavailable = "durable_delivery_snapshot_unavailable";
    let named = std::fs::symlink_metadata(path)
        .ok()
        .and_then(|meta| identity(&meta))
        .ok_or(unavailable)?;
    let mut file = super::open_regular_health_file(path).map_err(|error| match error {
        HealthFileOpenError::Unavailable => unavailable,
        HealthFileOpenError::Invalid => invalid,
    })?;
    if file.metadata().ok().and_then(|meta| identity(&meta)) != Some(named) {
        return Err(invalid);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable)?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(invalid);
    }
    let value: Snapshot = serde_json::from_slice(&bytes).map_err(|_| invalid)?;
    validate(&value)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| unavailable)?;
    let mut tail = Vec::new();
    (&mut file)
        .take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut tail)
        .map_err(|_| unavailable)?;
    if tail != bytes
        || file.metadata().ok().and_then(|meta| identity(&meta)) != Some(named)
        || std::fs::symlink_metadata(path)
            .ok()
            .and_then(|meta| identity(&meta))
            != Some(named)
    {
        return Err(invalid);
    }
    Ok(value)
}

pub(super) fn report_at(
    root: &Path,
    test_mode: bool,
    boot_id: Option<&str>,
    now: DateTime<Utc>,
) -> DeliveryHealthReport {
    let value = match read_at(&path(root, test_mode)) {
        Ok(value) => value,
        Err(reason) => return DeliveryHealthReport::with_reason(reason),
    };
    if boot_id != Some(value.boot_id.as_str()) {
        return DeliveryHealthReport::with_reason("durable_delivery_snapshot_process_mismatch");
    }
    if !fresh_at(value.published_at, now)
        || value
            .counts
            .as_ref()
            .is_some_and(|counts| !fresh_at(counts.observed_at, now))
    {
        return DeliveryHealthReport::with_reason("durable_delivery_snapshot_stale");
    }
    let observed_at = value.counts.as_ref().map(|counts| counts.observed_at);
    DeliveryHealthReport {
        version: VERSION,
        coverage: COVERAGE,
        reason_domain: "durable_delivery_state_machine",
        status: match value.status {
            Status::Observed => "observed",
            Status::NotInitialized => "not_initialized",
            Status::Unavailable => "unavailable",
        },
        reason_code: match value.status {
            Status::Observed => None,
            Status::NotInitialized => Some("durable_delivery_runtime_not_initialized"),
            Status::Unavailable => Some("durable_delivery_read_failed"),
        },
        failure: value.failure,
        published_at: Some(value.published_at),
        observed_at,
        age_seconds: observed_at.map(|at| now.signed_duration_since(at).num_seconds()),
        counts: value.counts,
    }
}

/// A timed-out blocking operation cannot be cancelled. Retain its handle until
/// it finishes, discard that result, and never accumulate replacement workers.
async fn run_scheduler<Read, Publish>(
    mut interval: tokio::time::Interval,
    deadline: std::time::Duration,
    read: Arc<Read>,
    mut publish: Publish,
) where
    Read: Fn() -> CachedDeliveryObservation + Send + Sync + 'static,
    Publish: FnMut(CachedDeliveryObservation),
{
    loop {
        let reader = Arc::clone(&read);
        let mut task = tokio::task::spawn_blocking(move || reader());
        match tokio::time::timeout(deadline, &mut task).await {
            Ok(Ok(value)) => publish(value),
            Ok(Err(_)) => publish(CachedDeliveryObservation::Unavailable(
                Failure::ReaderTaskFailed,
            )),
            Err(_) => {
                publish(CachedDeliveryObservation::Unavailable(
                    Failure::ReaderDeadlineExceeded,
                ));
                loop {
                    tokio::select! {
                        _ = &mut task => break, // Its late result is never published.
                        _ = interval.tick() => publish(CachedDeliveryObservation::Unavailable(Failure::ReaderDeadlineExceeded)),
                    }
                }
            }
        }
        interval.tick().await;
    }
}

pub(crate) fn start_resident(test_mode: bool, boot_id: String) -> tokio::task::JoinHandle<()> {
    let root = stock_analysis::production_root::root_for_mode(test_mode).to_path_buf();
    tokio::spawn(run_scheduler(
        crate::operational_heartbeat_interval(HEARTBEAT_INTERVAL),
        OBSERVATION_DEADLINE,
        Arc::new(crate::durable_delivery_runtime::read_cached_delivery_status),
        move |observation| {
            let value = snapshot(&boot_id, Utc::now(), observation);
            if let Err(failure) = publish_at(&root, test_mode, &value) {
                // Only the finite enum, never the underlying error Display.
                log::warn!("[health][durable_delivery] publish_failed failure={failure:?}");
            }
        },
    ))
}

#[cfg(test)]
#[path = "health_cmd_durable_delivery_tests.rs"]
mod tests;
