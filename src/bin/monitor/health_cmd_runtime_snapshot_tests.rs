use super::super as health;
use super::*;
use serde_json::{json, Value};
use std::io::Write;

struct Fixture {
    root: tempfile::TempDir,
    test_mode: bool,
    now: DateTime<Utc>,
    lease: Option<crate::MonitorInstanceLease>,
}

impl Fixture {
    fn new(test_mode: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let lease =
            crate::acquire_monitor_instance_lease_at(&health::lease_path(root.path(), test_mode))
                .unwrap();
        let now = Utc::now();
        let fixture = Self {
            root,
            test_mode,
            now,
            lease: Some(lease),
        };
        fixture.write_banner(Some(now), Some(now));
        health::write_heartbeat_at(
            &health::heartbeat_path(fixture.root.path(), test_mode),
            fixture.boot(),
            now,
        )
        .unwrap();
        health::source_recovery::write_at(
            &health::source_recovery::path(fixture.root.path(), test_mode),
            fixture.boot(),
            &stock_analysis::news::aggregator::raw_v2::GlobalNewsSourceRegistry::new(),
            now,
        )
        .unwrap();
        fixture
    }

    fn boot(&self) -> &str {
        &self.lease.as_ref().unwrap().boot_id
    }

    fn write_banner(&self, account_at: Option<DateTime<Utc>>, data_at: Option<DateTime<Utc>>) {
        health::write_snapshot_at(
            &health::snapshot_path(self.root.path(), self.test_mode),
            &health::HealthSnapshot {
                version: health::SNAPSHOT_VERSION,
                boot_id: self.boot().to_owned(),
                observed_at: self.now,
                account_evaluated_at: account_at,
                data_evaluated_at: data_at,
                account_mode: "Normal".to_owned(),
                data_mode: "Full".to_owned(),
                account_metrics_complete: true,
                missing_capabilities: Vec::new(),
            },
        )
        .unwrap();
    }

    fn report(&self) -> health::HealthReport {
        health::report_at(self.root.path(), self.test_mode, self.now)
    }

    fn source_failure(&self, reason: &str) -> Value {
        let path = health::source_recovery::path(self.root.path(), self.test_mode);
        let mut payload: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let source = &mut payload["sources"][0];
        source["breaker_state"] = json!("open");
        source["warming"] = json!(false);
        source["consecutive_retryable_failures"] = json!(10);
        source["last_attempt_at"] = json!(self.now);
        source["outage_started_at"] = json!(self.now);
        source["last_failure_at"] = json!(self.now);
        source["opened_at"] = json!(self.now);
        source["next_probe_at"] = json!(self.now + chrono::Duration::seconds(60));
        source["last_reason_code"] = json!(reason);
        source["last_retryable"] = json!(true);
        std::fs::write(path, serde_json::to_vec(&payload).unwrap()).unwrap();
        payload
    }
}

fn assert_discarded(report: &health::HealthReport) {
    assert_eq!(report.status, "unhealthy");
    assert_eq!(report.reason_code, Some("health_snapshot_process_mismatch"));
    assert!(!report.monitor_running);
    assert!(!report.snapshot_fresh);
    assert!(!report.heartbeat_fresh);
    assert_eq!(report.account_mode, None);
    assert_eq!(report.data_mode, None);
    assert_eq!(report.account_metrics_complete, None);
    assert!(report.missing_capabilities.is_empty());
    assert_eq!(report.observed_at, None);
    assert_eq!(report.account_evaluated_at, None);
    assert_eq!(report.data_evaluated_at, None);
    assert_eq!(report.heartbeat_observed_at, None);
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["raw_news_source_recovery"]["sources"], json!([]));
    assert_eq!(
        value["raw_news_source_recovery"]["reason_code"],
        "raw_news_source_snapshot_process_mismatch"
    );
    let nested = &value["runtime_snapshot"];
    for component in ["account", "data"] {
        assert_eq!(nested[component]["status"], "unavailable");
        assert_eq!(nested[component]["mode"], Value::Null);
        assert_eq!(nested[component]["evaluated_at"], Value::Null);
    }
    assert_eq!(nested["process"]["boot_identity_sha256"], Value::Null);
    assert_eq!(nested["raw_global_news"]["recovery"]["sources"], json!([]));
    assert!(!health::render_text(report).contains("account_mode=Normal"));
}

#[test]
fn m3_runtime_health_snapshot_preserves_scopes_and_marks_unconnected_domains_not_observed() {
    let fixture = Fixture::new(true);
    let files = [
        health::lease_path(fixture.root.path(), true),
        health::snapshot_path(fixture.root.path(), true),
        health::heartbeat_path(fixture.root.path(), true),
        health::source_recovery::path(fixture.root.path(), true),
    ];
    let before: Vec<_> = files
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let report = fixture.report();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(report.status, "ok");
    assert_eq!(
        value["coverage"],
        "banner_account_data_and_process_liveness_only"
    );
    let nested = &value["runtime_snapshot"];
    assert_eq!(nested["version"], 2);
    assert_eq!(nested["checked_at"], json!(fixture.now));
    assert_eq!(nested["process"]["status"], "ok");
    assert_eq!(nested["account"]["status"], "ok");
    assert_eq!(nested["data"]["status"], "ok");
    assert_eq!(
        nested["raw_global_news"]["recovery"],
        value["raw_news_source_recovery"]
    );
    assert_eq!(nested["raw_global_news"]["recovery"]["status"], "warming");
    assert_eq!(
        nested["raw_global_news"]["recovery"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(nested.get("status").is_none());
    for domain in [
        "durable_delivery",
        "data_quality",
        "other_sources",
        "quiet_halted_policy",
    ] {
        assert_eq!(nested[domain]["status"], "not_observed");
        assert!(nested[domain].get("count").is_none());
        if domain == "durable_delivery" {
            assert_eq!(
                nested[domain]["reason_code"],
                "durable_delivery_snapshot_unavailable"
            );
            assert!(nested[domain]["counts"].is_null());
        } else {
            assert!(nested[domain].get("reason_code").is_none());
        }
    }
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains(fixture.boot()));
    assert!(!output.contains("Ready"));
    let text = health::render_text(&report);
    assert!(text.contains("runtime_snapshot_version=2"));
    assert!(text.contains("runtime_account_status=ok"));
    for domain in [
        "durable_delivery",
        "data_quality",
        "other_sources",
        "quiet_halted_policy",
    ] {
        assert!(text.contains(&format!("{domain}_status=not_observed")));
    }
    assert_eq!(
        before,
        files
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>()
    );
    assert!(!fixture
        .root
        .path()
        .join("data/durable_delivery.sqlite3")
        .exists());
}

#[cfg(unix)]
#[test]
fn m3_runtime_health_same_inode_boot_or_raw_byte_change_discards_all_old_observations() {
    for change_boot in [true, false] {
        let fixture = Fixture::new(true);
        assert_eq!(fixture.report().status, "ok");
        let path = health::lease_path(fixture.root.path(), true);
        let before_identity = file_identity(&std::fs::metadata(&path).unwrap()).unwrap();
        let report =
            health::report_at_with_boundary(fixture.root.path(), true, fixture.now, || {
                let bytes = if change_boot {
                    "789:1000:2".to_owned()
                } else {
                    format!("{}\n", fixture.boot())
                };
                std::fs::write(&path, bytes).unwrap();
                assert_eq!(
                    file_identity(&std::fs::metadata(&path).unwrap()),
                    Some(before_identity)
                );
            });
        assert_discarded(&report);
    }
}

#[cfg(unix)]
#[test]
fn m3_runtime_health_same_boot_and_bytes_replaced_locked_inode_discards_old_observations() {
    let fixture = Fixture::new(true);
    assert_eq!(fixture.report().status, "ok");
    let path = health::lease_path(fixture.root.path(), true);
    let before_identity = file_identity(&std::fs::metadata(&path).unwrap()).unwrap();
    let mut replacement_lease = None;
    let report = health::report_at_with_boundary(fixture.root.path(), true, fixture.now, || {
        let replacement_path = path.with_extension("TEST_CODE-replacement");
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&replacement_path)
            .unwrap();
        file.write_all(fixture.boot().as_bytes()).unwrap();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();
        std::fs::rename(replacement_path, &path).unwrap();
        assert_ne!(
            file_identity(&std::fs::metadata(&path).unwrap()),
            Some(before_identity)
        );
        replacement_lease = Some(file);
    });
    // Both the retained old inode and the replacement remain exclusively locked.
    assert!(fixture.lease.is_some());
    assert!(replacement_lease.is_some());
    assert_discarded(&report);
}

#[cfg(unix)]
#[test]
fn m3_runtime_health_same_inode_lock_state_changes_discard_old_observations() {
    let mut fixture = Fixture::new(true);
    assert_eq!(fixture.report().status, "ok");
    let path = health::lease_path(fixture.root.path(), true);
    let root = fixture.root.path().to_path_buf();
    let now = fixture.now;
    let before_identity = file_identity(&std::fs::metadata(&path).unwrap()).unwrap();
    let released = health::report_at_with_boundary(&root, true, now, || {
        drop(fixture.lease.take());
    });
    assert_eq!(
        file_identity(&std::fs::metadata(&path).unwrap()),
        Some(before_identity)
    );
    assert_discarded(&released);

    let mut newly_locked = None;
    let acquired = health::report_at_with_boundary(&root, true, now, || {
        let file = std::fs::OpenOptions::new().read(true).open(&path).unwrap();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();
        newly_locked = Some(file);
    });
    assert!(newly_locked.is_some());
    assert_discarded(&acquired);
}

#[test]
fn m3_runtime_health_fresh_heartbeat_does_not_refresh_account_or_data_evaluations() {
    let fixture = Fixture::new(true);
    let stale = fixture.now - chrono::Duration::minutes(11);
    let data_at = fixture.now - chrono::Duration::minutes(2);
    fixture.write_banner(Some(stale), Some(data_at));
    let report = fixture.report();
    assert_eq!(report.heartbeat_status, "ok");
    assert_eq!(report.reason_code, Some("account_evaluation_stale"));
    let nested = serde_json::to_value(&report).unwrap()["runtime_snapshot"].clone();
    assert_eq!(
        nested["process"]["heartbeat_observed_at"],
        json!(fixture.now)
    );
    assert_eq!(nested["account"]["evaluated_at"], json!(stale));
    assert_eq!(nested["account"]["reason_code"], "account_evaluation_stale");
    assert_eq!(nested["data"]["evaluated_at"], json!(data_at));
    assert_eq!(nested["data"]["status"], "ok");
    fixture.write_banner(Some(fixture.now), Some(stale));
    let second = serde_json::to_value(fixture.report()).unwrap();
    assert_eq!(second["runtime_snapshot"]["account"]["status"], "ok");
    assert_eq!(
        second["runtime_snapshot"]["data"]["reason_code"],
        "data_evaluation_stale"
    );
}

#[test]
fn m3_runtime_health_local_transport_open_source_preserves_existing_reason_and_scope() {
    let fixture = Fixture::new(true);
    let payload = fixture.source_failure("external_transport_unavailable");
    let report = fixture.report();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        report.status, "ok",
        "source recovery cannot replace the banner verdict"
    );
    let recovery = &value["runtime_snapshot"]["raw_global_news"]["recovery"];
    assert_eq!(recovery["status"], "degraded");
    assert_eq!(recovery["reason_code"], "raw_news_source_degraded");
    assert_eq!(recovery["sources"], payload["sources"]);
    assert_eq!(recovery["sources"][0]["consecutive_retryable_failures"], 10);
    assert_eq!(recovery["sources"][0]["last_retryable"], true);
    assert_eq!(recovery["sources"][1]["warming"], true);
    assert_eq!(value["raw_news_source_recovery"], *recovery);
    assert!(health::render_text(&report).contains("reason=external_transport_unavailable"));
}

#[test]
fn m3_runtime_health_unknown_secret_like_reason_is_not_echoed_in_any_report() {
    let fixture = Fixture::new(true);
    let secret = "test_code_secret_token_never_echo";
    assert!(secret
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte == b'_'));
    fixture.source_failure(secret);
    let report = fixture.report();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value["raw_news_source_recovery"]["reason_code"],
        "raw_news_source_snapshot_invalid"
    );
    assert_eq!(value["raw_news_source_recovery"]["sources"], json!([]));
    for output in [
        serde_json::to_string(&report).unwrap(),
        health::render_text(&report),
        format!("{report:?}"),
    ] {
        assert!(!output.contains(secret));
    }
    assert_eq!(report.status, "ok");
}

#[test]
fn m3_runtime_health_unknown_payload_fields_never_escape_and_test_paths_do_not_fall_back() {
    let fixture = Fixture::new(false);
    let path = health::snapshot_path(fixture.root.path(), false);
    let mut payload: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let secret = "TEST_CODE-secret /sensitive/path SELECT account FROM ledger prompt-envelope";
    payload["prompt"] = json!(secret);
    std::fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();
    let report = fixture.report();
    assert_eq!(report.reason_code, Some("health_snapshot_invalid"));
    assert_eq!(report.account_mode, None);
    for output in [
        serde_json::to_string(&report).unwrap(),
        health::render_text(&report),
        format!("{report:?}"),
    ] {
        assert!(!output.contains(secret));
        assert!(!output.contains(fixture.root.path().to_str().unwrap()));
    }
    let test = health::report_at(fixture.root.path(), true, fixture.now);
    assert!(!test.monitor_running);
    assert_eq!(test.account_mode, None);
    assert_eq!(
        test.raw_news_source_recovery.reason_code,
        Some("raw_news_source_snapshot_unavailable")
    );
    assert!(!health::snapshot_path(fixture.root.path(), true).exists());
}
