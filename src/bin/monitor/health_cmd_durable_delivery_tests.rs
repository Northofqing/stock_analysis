use super::*;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    root: tempfile::TempDir,
    lease: crate::MonitorInstanceLease,
    now: DateTime<Utc>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let lease =
            crate::acquire_monitor_instance_lease_at(&super::super::lease_path(root.path(), true))
                .unwrap();
        Self {
            root,
            lease,
            now: Utc::now(),
        }
    }
    fn value(&self, observation: CachedDeliveryObservation) -> Snapshot {
        snapshot(&self.lease.boot_id, self.now, observation)
    }
    fn publish(&self, observation: CachedDeliveryObservation) {
        publish_at(self.root.path(), true, &self.value(observation)).unwrap()
    }
    fn report(&self, at: DateTime<Utc>, boot: &str) -> DeliveryHealthReport {
        report_at(self.root.path(), true, Some(boot), at)
    }
}

fn zero(at: DateTime<Utc>) -> CachedDeliveryObservation {
    CachedDeliveryObservation::Observed(DeliveryStatusSnapshot {
        observed_at: at,
        state_counts: DELIVERY_STATUS_STATES
            .map(|state| stock_analysis::durable_delivery::DeliveryStateCount { state, count: 0 }),
        total_decisions: 0,
        locally_pending_decisions: 0,
        deliverable_decisions: 0,
        non_progressable_foreign_attempts: 0,
        non_progressable_manual_reviews: 0,
    })
}

#[test]
fn m3_durable_health_actual_files_distinguish_zero_unknown_stale_old_boot_and_failed_read() {
    let fixture = Fixture::new();
    fixture.publish(CachedDeliveryObservation::NotInitialized);
    let unknown = fixture.report(fixture.now, &fixture.lease.boot_id);
    assert_eq!(unknown.status, "not_initialized");
    assert!(unknown.counts.is_none());
    fixture.publish(zero(fixture.now));
    let actual = fixture.report(fixture.now, &fixture.lease.boot_id);
    assert_eq!(actual.status, "observed");
    assert_eq!(actual.counts.as_ref().unwrap().total_decisions, 0);
    for (at, boot, reason) in [
        (
            fixture.now + Duration::minutes(11),
            fixture.lease.boot_id.as_str(),
            "durable_delivery_snapshot_stale",
        ),
        (
            fixture.now,
            "999:123:2",
            "durable_delivery_snapshot_process_mismatch",
        ),
    ] {
        let report = fixture.report(at, boot);
        assert_eq!(report.reason_code, Some(reason));
        assert!(report.counts.is_none());
        assert!(report.observed_at.is_none());
    }
    fixture.publish(CachedDeliveryObservation::Unavailable(Failure::Sqlite));
    let failed = fixture.report(fixture.now, &fixture.lease.boot_id);
    assert_eq!(failed.status, "unavailable");
    assert_eq!(failed.failure, Some(Failure::Sqlite));
    assert!(failed.counts.is_none());
}

#[test]
fn m3_durable_health_publisher_requires_the_actual_live_lease_before_overwriting() {
    let fixture = Fixture::new();
    fixture.publish(zero(fixture.now));
    let target = path(fixture.root.path(), true);
    let original = std::fs::read(&target).unwrap();
    let foreign = snapshot(
        "789:123:2",
        fixture.now,
        CachedDeliveryObservation::NotInitialized,
    );
    assert_eq!(
        publish_at(fixture.root.path(), true, &foreign),
        Err(Failure::ProcessMismatch)
    );
    assert_eq!(std::fs::read(&target).unwrap(), original);
    #[cfg(unix)]
    {
        fs2::FileExt::unlock(&fixture.lease._file).unwrap();
        let same_boot = fixture.value(CachedDeliveryObservation::Unavailable(Failure::Io));
        assert_eq!(
            publish_at(fixture.root.path(), true, &same_boot),
            Err(Failure::ProcessMismatch)
        );
        assert_eq!(std::fs::read(&target).unwrap(), original);
        assert!(report_at(fixture.root.path(), true, None, fixture.now)
            .counts
            .is_none());
    }
}

#[test]
fn m3_durable_health_unknown_secret_duplicate_fields_and_count_overflow_fail_closed() {
    let fixture = Fixture::new();
    let payload =
        serde_json::to_value(fixture.value(CachedDeliveryObservation::Unavailable(Failure::Io)))
            .unwrap();
    let secret = "TEST_CODE_SECRET_ACCOUNT_DATABASE_PASSWORD";
    let mut unknown = payload.clone();
    unknown["failure"] = json!(secret);
    let duplicate = serde_json::to_string(&payload).unwrap().replacen(
        "\"version\":1",
        "\"version\":1,\"version\":1",
        1,
    );
    let mut counts = serde_json::to_value(fixture.value(zero(fixture.now))).unwrap();
    counts["counts"]["state_counts"][0]["count"] = json!(u64::MAX);
    counts["counts"]["state_counts"][1]["count"] = json!(1);
    let mut extra = payload;
    extra["secret"] = json!(secret);
    for bytes in [
        serde_json::to_vec(&unknown).unwrap(),
        duplicate.into_bytes(),
        serde_json::to_vec(&counts).unwrap(),
        serde_json::to_vec(&extra).unwrap(),
    ] {
        std::fs::create_dir_all(path(fixture.root.path(), true).parent().unwrap()).unwrap();
        std::fs::write(path(fixture.root.path(), true), bytes).unwrap();
        let report = fixture.report(fixture.now, &fixture.lease.boot_id);
        assert_eq!(
            report.reason_code,
            Some("durable_delivery_snapshot_invalid")
        );
        assert!(report.counts.is_none());
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains(secret));
        let health = super::super::report_at(fixture.root.path(), true, fixture.now);
        assert!(!super::super::render_text(&health).contains(secret));
        assert!(!serde_json::to_string(&health).unwrap().contains(secret));
    }
}

#[cfg(unix)]
#[test]
fn m3_durable_health_lease_tail_clears_previously_observed_counts_and_banner_is_not_promoted() {
    for mode in 0..3 {
        let fixture = Fixture::new();
        fixture.publish(zero(fixture.now));
        super::super::write_snapshot_at(
            &super::super::snapshot_path(fixture.root.path(), true),
            &HealthSnapshot {
                version: SNAPSHOT_VERSION,
                boot_id: fixture.lease.boot_id.clone(),
                observed_at: fixture.now,
                account_evaluated_at: Some(fixture.now),
                data_evaluated_at: Some(fixture.now),
                account_mode: "Frozen".to_owned(),
                data_mode: "Unsafe".to_owned(),
                account_metrics_complete: false,
                missing_capabilities: vec!["Kline".to_owned()],
            },
        )
        .unwrap();
        let initial = super::super::report_at(fixture.root.path(), true, fixture.now);
        assert_eq!(initial.status, "unhealthy");
        assert_eq!(
            serde_json::to_value(initial).unwrap()["runtime_snapshot"]["durable_delivery"]
                ["status"],
            "observed"
        );
        let lease_path = super::super::lease_path(fixture.root.path(), true);
        let mut replacement = None;
        let report =
            super::super::report_at_with_boundary(fixture.root.path(), true, fixture.now, || {
                match mode {
                    0 => std::fs::write(&lease_path, "789:123:2").unwrap(),
                    1 => {
                        let other = lease_path.with_extension("TEST_CODE-new");
                        let file = std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create_new(true)
                            .open(&other)
                            .unwrap();
                        (&file).write_all(fixture.lease.boot_id.as_bytes()).unwrap();
                        fs2::FileExt::try_lock_exclusive(&file).unwrap();
                        std::fs::rename(other, &lease_path).unwrap();
                        replacement = Some(file);
                    }
                    _ => fs2::FileExt::unlock(&fixture.lease._file).unwrap(),
                }
            });
        let value = serde_json::to_value(report).unwrap();
        let durable = &value["runtime_snapshot"]["durable_delivery"];
        assert_eq!(
            durable["reason_code"],
            "durable_delivery_snapshot_process_mismatch"
        );
        assert_eq!(durable["counts"], Value::Null);
        assert_eq!(durable["observed_at"], Value::Null);
        drop(replacement.take());
    }
}

#[cfg(unix)]
#[test]
fn m3_durable_health_non_regular_and_oversize_files_are_bounded_without_following() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let target = path(fixture.root.path(), true);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, vec![b' '; MAX_SNAPSHOT_BYTES as usize + 1]).unwrap();
    assert!(fixture
        .report(fixture.now, &fixture.lease.boot_id)
        .counts
        .is_none());
    std::fs::remove_file(&target).unwrap();
    let hidden = target.with_extension("TEST_CODE-hidden");
    std::fs::write(
        &hidden,
        serde_json::to_vec(&fixture.value(zero(fixture.now))).unwrap(),
    )
    .unwrap();
    symlink(&hidden, &target).unwrap();
    assert!(fixture
        .report(fixture.now, &fixture.lease.boot_id)
        .counts
        .is_none());
    std::fs::remove_file(&target).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(fixture
        .report(fixture.now, &fixture.lease.boot_id)
        .counts
        .is_none());
}

#[tokio::test(start_paused = true)]
async fn m3_durable_health_slow_worker_is_single_and_late_success_never_replaces_failure_heartbeat_continues(
) {
    let fixture = Fixture::new();
    fixture.publish(zero(fixture.now));
    let calls = Arc::new(AtomicUsize::new(0));
    let reader_calls = Arc::clone(&calls);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let receiver = std::sync::Mutex::new(release_rx);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let entered = std::sync::Mutex::new(Some(entered_tx));
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let finished = std::sync::Mutex::new(Some(finished_tx));
    let anchor = fixture.now;
    let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let published = Arc::clone(&outcomes);
    let root = fixture.root.path().to_path_buf();
    let boot = fixture.lease.boot_id.clone();
    let task = tokio::spawn(run_scheduler(
        crate::operational_heartbeat_interval(std::time::Duration::from_secs(60)),
        OBSERVATION_DEADLINE,
        Arc::new(move || {
            reader_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(sender) = entered.lock().unwrap().take() {
                let _ = sender.send(());
            }
            receiver.lock().unwrap().recv().unwrap();
            if let Some(sender) = finished.lock().unwrap().take() {
                let _ = sender.send(());
            }
            zero(anchor)
        }),
        move |observation| {
            let value = snapshot(&boot, anchor, observation);
            published.lock().unwrap().push(value.status);
            publish_at(&root, true, &value).unwrap();
        },
    ));
    let heartbeat_calls = Arc::new(AtomicUsize::new(0));
    let writes = Arc::clone(&heartbeat_calls);
    let heartbeat_path = super::super::heartbeat_path(fixture.root.path(), true);
    let heartbeat_boot = fixture.lease.boot_id.clone();
    let heartbeat = tokio::spawn(crate::run_operational_heartbeat_scheduler(
        crate::operational_heartbeat_interval(std::time::Duration::from_secs(60)),
        move || anchor,
        move |at| {
            writes.fetch_add(1, Ordering::SeqCst);
            super::super::write_heartbeat_at(&heartbeat_path, &heartbeat_boot, at)
        },
        0,
    ));
    entered_rx.await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    for _ in 0..3 {
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(heartbeat_calls.load(Ordering::SeqCst) >= 3);
    assert_eq!(
        fixture.report(fixture.now, &fixture.lease.boot_id).failure,
        Some(Failure::ReaderDeadlineExceeded)
    );
    assert!(fixture
        .report(fixture.now, &fixture.lease.boot_id)
        .counts
        .is_none());
    release_tx.send(()).unwrap();
    finished_rx.await.unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(outcomes
        .lock()
        .unwrap()
        .iter()
        .all(|status| *status == Status::Unavailable));
    assert!(fixture
        .report(fixture.now, &fixture.lease.boot_id)
        .counts
        .is_none());
    task.abort();
    heartbeat.abort();
    let _ = task.await;
    let _ = heartbeat.await;
}
