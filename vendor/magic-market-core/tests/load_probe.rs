use magic_market_core::{verify_serial_load, LoadProbeError, ProbeRequestTracker, ProbeStatus};
use std::thread;
use std::time::Duration;

#[test]
fn actual_request_starts_and_concurrency_are_machine_verified() {
    let mut tracker = ProbeRequestTracker::default();
    tracker.request_started();
    tracker.request_finished().unwrap();
    thread::sleep(Duration::from_millis(2));
    tracker.request_started();
    tracker.request_finished().unwrap();

    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.request_starts(), 2);
    assert_eq!(snapshot.maximum_concurrency(), 1);
    assert_eq!(snapshot.active_requests(), 0);
    assert!(snapshot.minimum_start_gap().unwrap() >= Duration::from_millis(1));
    assert_eq!(
        verify_serial_load(&snapshot, Duration::from_millis(1)).unwrap(),
        ProbeStatus::Admitted
    );
}

#[test]
fn empty_inflight_concurrent_and_underpaced_snapshots_fail() {
    let empty = ProbeRequestTracker::default().snapshot();
    assert_eq!(
        verify_serial_load(&empty, Duration::from_secs(1)).unwrap_err(),
        LoadProbeError::NoRequestStarts
    );

    let mut inflight = ProbeRequestTracker::default();
    inflight.request_started();
    assert_eq!(
        verify_serial_load(&inflight.snapshot(), Duration::from_secs(1)).unwrap_err(),
        LoadProbeError::RequestsStillActive { active: 1 }
    );

    let mut concurrent = ProbeRequestTracker::default();
    concurrent.request_started();
    concurrent.request_started();
    concurrent.request_finished().unwrap();
    concurrent.request_finished().unwrap();
    assert_eq!(
        verify_serial_load(&concurrent.snapshot(), Duration::ZERO).unwrap_err(),
        LoadProbeError::ConcurrentRequests { maximum: 2 }
    );

    let mut underpaced = ProbeRequestTracker::default();
    underpaced.request_started();
    underpaced.request_finished().unwrap();
    underpaced.request_started();
    underpaced.request_finished().unwrap();
    assert!(matches!(
        verify_serial_load(&underpaced.snapshot(), Duration::from_secs(1)),
        Err(LoadProbeError::StartGapTooShort { .. })
    ));
}

#[test]
fn finishing_without_an_active_request_is_rejected() {
    let mut tracker = ProbeRequestTracker::default();
    assert_eq!(
        tracker.request_finished().unwrap_err(),
        LoadProbeError::FinishWithoutStart
    );
}

#[test]
fn one_serial_request_needs_no_inter_request_gap() {
    let mut tracker = ProbeRequestTracker::default();
    tracker.request_started();
    tracker.request_finished().unwrap();

    let snapshot = tracker.snapshot();
    assert_eq!(snapshot.minimum_start_gap(), None);
    assert_eq!(
        verify_serial_load(&snapshot, Duration::from_secs(60)).unwrap(),
        ProbeStatus::Admitted
    );
}
