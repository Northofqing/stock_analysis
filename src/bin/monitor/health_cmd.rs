//! Read-only operational health command backed by the live banner owner.
//!
//! This first v19 slice reports account/data banner health. Breakers, error
//! counts, and per-source recovery are deliberately outside its coverage.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const SNAPSHOT_VERSION: u8 = 2;
const MAX_SNAPSHOT_BYTES: u64 = 4_096;
const MAX_SNAPSHOT_AGE: Duration = Duration::minutes(10);
const MAX_CLOCK_LEAD: Duration = Duration::seconds(5);

pub fn new_boot_id() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "{}:{}:{}",
        std::process::id(),
        nanos,
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthCommand {
    json: bool,
    test_mode: bool,
}

pub fn parse(args: &[String]) -> Result<Option<HealthCommand>, &'static str> {
    let arguments = args.get(1..).unwrap_or_default();
    if !arguments.iter().any(|argument| argument == "--health") {
        return Ok(None);
    }
    let mut command = HealthCommand {
        json: false,
        test_mode: false,
    };
    let mut health_seen = false;
    for argument in arguments {
        match argument.as_str() {
            "--health" if !health_seen => health_seen = true,
            "--json" if !command.json => command.json = true,
            "--test" if !command.test_mode => command.test_mode = true,
            _ => return Err("usage: monitor --health [--json] [--test]"),
        }
    }
    Ok(Some(command))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HealthSnapshot {
    version: u8,
    boot_id: String,
    observed_at: DateTime<Utc>,
    account_evaluated_at: Option<DateTime<Utc>>,
    data_evaluated_at: Option<DateTime<Utc>>,
    account_mode: String,
    data_mode: String,
    account_metrics_complete: bool,
    missing_capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HealthReport {
    status: &'static str,
    reason_code: Option<&'static str>,
    monitor_running: bool,
    snapshot_fresh: bool,
    observed_at: Option<DateTime<Utc>>,
    observed_age_seconds: Option<i64>,
    account_evaluated_at: Option<DateTime<Utc>>,
    account_age_seconds: Option<i64>,
    data_evaluated_at: Option<DateTime<Utc>>,
    data_age_seconds: Option<i64>,
    account_mode: Option<String>,
    data_mode: Option<String>,
    account_metrics_complete: Option<bool>,
    missing_capabilities: Vec<String>,
    coverage: &'static str,
}

fn snapshot_path(root: &Path, test_mode: bool) -> PathBuf {
    root.join("data")
        .join(if test_mode { "test/health" } else { "health" })
        .join("monitor-banner-v2.json")
}

fn lease_path(root: &Path, test_mode: bool) -> PathBuf {
    root.join("data")
        .join("locks")
        .join(if test_mode { "test" } else { "production" })
        .join("monitor-delivery.lock")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EvaluationTimes {
    account_evaluated_at: Option<DateTime<Utc>>,
    data_evaluated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug)]
pub enum EvaluationUpdate {
    AccountAndData {
        account_at: DateTime<Utc>,
        data_at: DateTime<Utc>,
    },
    Data(DateTime<Utc>),
    Synthetic,
}

impl EvaluationTimes {
    pub fn apply(&mut self, update: EvaluationUpdate) {
        match update {
            EvaluationUpdate::AccountAndData {
                account_at,
                data_at,
            } => {
                self.account_evaluated_at = Some(account_at);
                self.data_evaluated_at = Some(data_at);
            }
            EvaluationUpdate::Data(at) => self.data_evaluated_at = Some(at),
            EvaluationUpdate::Synthetic => *self = Self::default(),
        }
    }
}

pub fn write_banner_snapshot(
    test_mode: bool,
    boot_id: &str,
    banner: &crate::push_templates::BannerCtx,
    times: EvaluationTimes,
) -> Result<(), String> {
    let root = stock_analysis::production_root::root_for_mode(test_mode);
    write_banner_snapshot_at(
        &snapshot_path(root, test_mode),
        boot_id,
        banner,
        times,
        Utc::now(),
    )
}

fn write_banner_snapshot_at(
    path: &Path,
    boot_id: &str,
    banner: &crate::push_templates::BannerCtx,
    times: EvaluationTimes,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let snapshot = HealthSnapshot {
        version: SNAPSHOT_VERSION,
        boot_id: boot_id.to_owned(),
        observed_at: now,
        account_evaluated_at: times.account_evaluated_at,
        data_evaluated_at: times.data_evaluated_at,
        account_mode: banner.account_mode.label().to_owned(),
        data_mode: banner.data_mode.label().to_owned(),
        account_metrics_complete: banner.account_metrics_complete,
        missing_capabilities: banner
            .data_missing_note
            .as_deref()
            .map(|note| note.split('/').map(str::to_owned).collect())
            .unwrap_or_default(),
    };
    write_snapshot_at(path, &snapshot)
}

fn write_snapshot_at(path: &Path, snapshot: &HealthSnapshot) -> Result<(), String> {
    validate_snapshot(snapshot).map_err(str::to_owned)?;
    let parent = path.parent().ok_or("health snapshot path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| format!("create health directory: {error}"))?;
    let bytes = serde_json::to_vec(snapshot)
        .map_err(|error| format!("serialize health snapshot: {error}"))?;
    let temp = parent.join(format!(
        ".monitor-banner-{}-{}.tmp",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temp)
            .map_err(|error| format!("create health snapshot temp: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("write health snapshot: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync health snapshot: {error}"))?;
        std::fs::rename(&temp, path).map_err(|error| format!("replace health snapshot: {error}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn validate_snapshot(snapshot: &HealthSnapshot) -> Result<(), &'static str> {
    if snapshot.version != SNAPSHOT_VERSION
        || !valid_boot_id(&snapshot.boot_id)
        || !matches!(
            snapshot.account_mode.as_str(),
            "Normal" | "ReduceOnly" | "Frozen"
        )
        || !matches!(snapshot.data_mode.as_str(), "Full" | "Degraded" | "Unsafe")
        || snapshot.missing_capabilities.len() > 5
        || snapshot.missing_capabilities.iter().any(|capability| {
            !matches!(
                capability.as_str(),
                "Quote" | "Kline" | "MoneyFlow" | "News" | "OrderBook"
            )
        })
    {
        return Err("health_snapshot_invalid");
    }
    let unique = snapshot
        .missing_capabilities
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if unique.len() != snapshot.missing_capabilities.len() {
        return Err("health_snapshot_invalid");
    }
    Ok(())
}

fn valid_boot_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b':')
}

fn read_snapshot_at(path: &Path) -> Result<HealthSnapshot, &'static str> {
    let file = std::fs::File::open(path).map_err(|_| "health_snapshot_unavailable")?;
    let mut bytes = Vec::new();
    file.take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "health_snapshot_unavailable")?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err("health_snapshot_invalid");
    }
    let snapshot: HealthSnapshot =
        serde_json::from_slice(&bytes).map_err(|_| "health_snapshot_invalid")?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn monitor_lease_identity(path: &Path) -> Option<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return None;
    };
    match fs2::FileExt::try_lock_shared(&file) {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            let mut bytes = Vec::new();
            file.take(97).read_to_end(&mut bytes).ok()?;
            let identity = std::str::from_utf8(&bytes).ok()?.trim_end_matches('\n');
            valid_boot_id(identity).then(|| identity.to_owned())
        }
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            None
        }
        Err(_) => None,
    }
}

fn report_at(root: &Path, test_mode: bool, now: DateTime<Utc>) -> HealthReport {
    report_from(
        read_snapshot_at(&snapshot_path(root, test_mode)),
        monitor_lease_identity(&lease_path(root, test_mode)),
        now,
    )
}

fn fresh_at(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    let age = now.signed_duration_since(at);
    age >= -MAX_CLOCK_LEAD && age <= MAX_SNAPSHOT_AGE
}

fn report_from(
    snapshot: Result<HealthSnapshot, &'static str>,
    lease_identity: Option<String>,
    now: DateTime<Utc>,
) -> HealthReport {
    let mut report = HealthReport {
        status: "unhealthy",
        reason_code: None,
        monitor_running: lease_identity.is_some(),
        snapshot_fresh: false,
        observed_at: None,
        observed_age_seconds: None,
        account_evaluated_at: None,
        account_age_seconds: None,
        data_evaluated_at: None,
        data_age_seconds: None,
        account_mode: None,
        data_mode: None,
        account_metrics_complete: None,
        missing_capabilities: Vec::new(),
        coverage: "banner_account_data_only",
    };
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(reason) => {
            report.reason_code = Some(reason);
            return report;
        }
    };
    report.observed_at = Some(snapshot.observed_at);
    report.observed_age_seconds = Some(
        now.signed_duration_since(snapshot.observed_at)
            .num_seconds(),
    );
    report.account_evaluated_at = snapshot.account_evaluated_at;
    report.account_age_seconds = snapshot
        .account_evaluated_at
        .map(|at| now.signed_duration_since(at).num_seconds());
    report.data_evaluated_at = snapshot.data_evaluated_at;
    report.data_age_seconds = snapshot
        .data_evaluated_at
        .map(|at| now.signed_duration_since(at).num_seconds());
    report.account_mode = Some(snapshot.account_mode.clone());
    report.data_mode = Some(snapshot.data_mode.clone());
    report.account_metrics_complete = Some(snapshot.account_metrics_complete);
    report.missing_capabilities = snapshot.missing_capabilities.clone();
    let age = now.signed_duration_since(snapshot.observed_at);
    if age < -MAX_CLOCK_LEAD || age > MAX_SNAPSHOT_AGE {
        report.reason_code = Some("health_snapshot_stale");
    } else {
        report.snapshot_fresh = true;
        if lease_identity.is_none() {
            report.reason_code = Some("monitor_not_running");
        } else if lease_identity.as_deref() != Some(snapshot.boot_id.as_str()) {
            report.reason_code = Some("health_snapshot_process_mismatch");
        } else if snapshot.account_evaluated_at.is_none() {
            report.reason_code = Some("account_evaluation_missing");
        } else if !fresh_at(snapshot.account_evaluated_at.unwrap(), now) {
            report.reason_code = Some("account_evaluation_stale");
        } else if snapshot.data_evaluated_at.is_none() {
            report.reason_code = Some("data_evaluation_missing");
        } else if !fresh_at(snapshot.data_evaluated_at.unwrap(), now) {
            report.reason_code = Some("data_evaluation_stale");
        } else if snapshot.account_mode != "Normal"
            || snapshot.data_mode != "Full"
            || !snapshot.account_metrics_complete
            || !snapshot.missing_capabilities.is_empty()
        {
            report.reason_code = Some("banner_unhealthy");
        } else {
            report.status = "ok";
        }
    }
    report
}

pub fn run(command: HealthCommand) -> i32 {
    let root = stock_analysis::production_root::root_for_mode(command.test_mode);
    let report = report_at(root, command.test_mode, Utc::now());
    if command.json {
        match serde_json::to_string(&report) {
            Ok(json) => println!("{json}"),
            Err(_) => return 2,
        }
    } else {
        println!("{}", render_text(&report));
    }
    if report.status == "ok" {
        0
    } else {
        1
    }
}

fn render_text(report: &HealthReport) -> String {
    format!(
            "status={} reason={} monitor_running={} snapshot_fresh={} observed_at={} observed_age_seconds={} account_evaluated_at={} account_age_seconds={} data_evaluated_at={} data_age_seconds={} account_mode={} data_mode={} account_metrics_complete={} missing_capabilities={} coverage={}",
            report.status,
            report.reason_code.unwrap_or("none"),
            report.monitor_running,
            report.snapshot_fresh,
            report.observed_at.map_or_else(|| "missing".to_owned(), |at| at.to_rfc3339()),
            report.observed_age_seconds.map_or_else(|| "missing".to_owned(), |age| age.to_string()),
            report.account_evaluated_at.map_or_else(|| "missing".to_owned(), |at| at.to_rfc3339()),
            report.account_age_seconds.map_or_else(|| "missing".to_owned(), |age| age.to_string()),
            report.data_evaluated_at.map_or_else(|| "missing".to_owned(), |at| at.to_rfc3339()),
            report.data_age_seconds.map_or_else(|| "missing".to_owned(), |age| age.to_string()),
            report.account_mode.as_deref().unwrap_or("missing"),
            report.data_mode.as_deref().unwrap_or("missing"),
            report.account_metrics_complete.map_or_else(|| "missing".to_owned(), |value| value.to_string()),
            if report.missing_capabilities.is_empty() { "none".to_owned() } else { report.missing_capabilities.join(",") },
            report.coverage,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(at: DateTime<Utc>) -> HealthSnapshot {
        HealthSnapshot {
            version: SNAPSHOT_VERSION,
            boot_id: "123:456:1".to_owned(),
            observed_at: at,
            account_evaluated_at: Some(at),
            data_evaluated_at: Some(at),
            account_mode: "Normal".to_owned(),
            data_mode: "Full".to_owned(),
            account_metrics_complete: true,
            missing_capabilities: Vec::new(),
        }
    }

    #[test]
    fn health_cli_accepts_only_read_only_modes() {
        let args = ["monitor", "--health", "--json", "--test"].map(str::to_owned);
        assert_eq!(
            parse(&args).unwrap(),
            Some(HealthCommand {
                json: true,
                test_mode: true
            })
        );
        assert_eq!(parse(&["monitor".to_owned()]).unwrap(), None);
        assert!(parse(&[
            "monitor".to_owned(),
            "--health".to_owned(),
            "--push".to_owned()
        ])
        .is_err());
        assert!(parse(&[
            "monitor".to_owned(),
            "--health".to_owned(),
            "--health".to_owned()
        ])
        .is_err());
    }

    #[test]
    fn health_snapshot_reports_live_fresh_banner_and_valid_json() {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let path = snapshot_path(root.path(), false);
        write_snapshot_at(&path, &snapshot(now)).unwrap();
        assert_eq!(read_snapshot_at(&path).unwrap().observed_at, now);
        let report = report_from(read_snapshot_at(&path), Some("123:456:1".to_owned()), now);
        assert_eq!(report.status, "ok");
        assert!(report.reason_code.is_none());
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["coverage"], "banner_account_data_only");
        assert_eq!(parsed["account_mode"], "Normal");
        assert!(parsed.get("today_pnl").is_none());
    }

    #[test]
    fn health_snapshot_fails_closed_for_missing_stale_dead_or_unhealthy_banner() {
        let now = Utc::now();
        assert_eq!(
            report_from(
                Err("health_snapshot_unavailable"),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("health_snapshot_unavailable")
        );
        assert_eq!(
            report_from(
                Ok(snapshot(now - Duration::minutes(11))),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("health_snapshot_stale")
        );
        assert_eq!(
            report_from(Ok(snapshot(now)), None, now).reason_code,
            Some("monitor_not_running")
        );
        assert_eq!(
            report_from(Ok(snapshot(now)), Some("123:456:2".to_owned()), now).reason_code,
            Some("health_snapshot_process_mismatch")
        );
        let mut degraded = snapshot(now);
        degraded.data_mode = "Degraded".to_owned();
        degraded.missing_capabilities = vec!["Quote".to_owned()];
        assert_eq!(
            report_from(Ok(degraded), Some("123:456:1".to_owned()), now).reason_code,
            Some("banner_unhealthy")
        );
        let mut frozen = snapshot(now);
        frozen.account_mode = "Frozen".to_owned();
        assert_eq!(
            report_from(Ok(frozen), Some("123:456:1".to_owned()), now).reason_code,
            Some("banner_unhealthy")
        );
        let mut unsafe_data = snapshot(now);
        unsafe_data.data_mode = "Unsafe".to_owned();
        assert_eq!(
            report_from(Ok(unsafe_data), Some("123:456:1".to_owned()), now).reason_code,
            Some("banner_unhealthy")
        );
        let mut incomplete = snapshot(now);
        incomplete.account_metrics_complete = false;
        assert_eq!(
            report_from(Ok(incomplete), Some("123:456:1".to_owned()), now).reason_code,
            Some("banner_unhealthy")
        );
    }

    #[test]
    fn health_reader_rejects_malformed_and_oversized_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let path = snapshot_path(root.path(), false);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{bad json}").unwrap();
        assert_eq!(
            read_snapshot_at(&path).unwrap_err(),
            "health_snapshot_invalid"
        );
        std::fs::write(&path, vec![b'x'; MAX_SNAPSHOT_BYTES as usize + 1]).unwrap();
        assert_eq!(
            read_snapshot_at(&path).unwrap_err(),
            "health_snapshot_invalid"
        );
    }

    #[test]
    fn successive_writes_keep_each_evaluation_origin() {
        let root = tempfile::tempdir().unwrap();
        let path = snapshot_path(root.path(), false);
        let t0 = DateTime::parse_from_rfc3339("2026-09-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let banner = crate::push_templates::BannerCtx {
            account_mode: crate::push_templates::AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            account_fact: None,
            data_mode: crate::push_templates::DataMode::Full,
            data_missing_note: None,
        };
        let mut times = EvaluationTimes::default();
        times.apply(EvaluationUpdate::AccountAndData {
            account_at: t0,
            data_at: t0,
        });
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, t0).unwrap();
        let t1 = t0 + Duration::minutes(1);
        times.apply(EvaluationUpdate::Data(t1));
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, t1).unwrap();
        let saved = read_snapshot_at(&path).unwrap();
        assert_eq!(saved.observed_at, t1);
        assert_eq!(saved.account_evaluated_at, Some(t0));
        assert_eq!(saved.data_evaluated_at, Some(t1));
        let hour_later = t0 + Duration::hours(1);
        times.apply(EvaluationUpdate::Data(hour_later));
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, hour_later).unwrap();
        let report = report_from(
            read_snapshot_at(&path),
            Some("123:456:1".into()),
            hour_later,
        );
        assert_eq!(report.reason_code, Some("account_evaluation_stale"));
        // A second rendering of the same account batch carries its original time.
        times.apply(EvaluationUpdate::AccountAndData {
            account_at: t0,
            data_at: hour_later,
        });
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, hour_later).unwrap();
        assert_eq!(
            read_snapshot_at(&path).unwrap().account_evaluated_at,
            Some(t0)
        );
        times.apply(EvaluationUpdate::AccountAndData {
            account_at: hour_later,
            data_at: hour_later,
        });
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, hour_later).unwrap();
        assert_eq!(
            report_from(
                read_snapshot_at(&path),
                Some("123:456:1".into()),
                hour_later
            )
            .status,
            "ok"
        );
        times.apply(EvaluationUpdate::Synthetic);
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, hour_later).unwrap();
        assert_eq!(read_snapshot_at(&path).unwrap().account_evaluated_at, None);
        assert_eq!(read_snapshot_at(&path).unwrap().data_evaluated_at, None);
    }

    #[test]
    fn health_rejects_old_version_and_missing_or_stale_evaluations() {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let old = root.path().join("data/health/monitor-banner-v1.json");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, r#"{"version":1}"#).unwrap();
        assert_eq!(
            read_snapshot_at(&snapshot_path(root.path(), false)).unwrap_err(),
            "health_snapshot_unavailable"
        );
        let mut legacy = snapshot(now);
        legacy.version = 1;
        let path = snapshot_path(root.path(), false);
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(
            read_snapshot_at(&path).unwrap_err(),
            "health_snapshot_invalid"
        );

        let mut item = snapshot(now);
        for (account, data, reason) in [
            (None, Some(now), "account_evaluation_missing"),
            (
                Some(now - Duration::minutes(11)),
                Some(now),
                "account_evaluation_stale",
            ),
            (
                Some(now + Duration::seconds(6)),
                Some(now),
                "account_evaluation_stale",
            ),
            (Some(now), None, "data_evaluation_missing"),
            (
                Some(now),
                Some(now - Duration::minutes(11)),
                "data_evaluation_stale",
            ),
            (
                Some(now),
                Some(now + Duration::seconds(6)),
                "data_evaluation_stale",
            ),
        ] {
            item.account_evaluated_at = account;
            item.data_evaluated_at = data;
            assert_eq!(
                report_from(Ok(item.clone()), Some(item.boot_id.clone()), now).reason_code,
                Some(reason)
            );
        }
        item = snapshot(now + Duration::seconds(6));
        assert_eq!(
            report_from(Ok(item), Some("123:456:1".into()), now).reason_code,
            Some("health_snapshot_stale")
        );
    }

    #[test]
    fn text_and_json_expose_matching_evidence() {
        let now = Utc::now();
        let report = report_from(
            Ok(snapshot(now - Duration::seconds(3))),
            Some("123:456:1".into()),
            now,
        );
        let value = serde_json::to_value(&report).unwrap();
        let text = render_text(&report);
        for key in ["observed_at", "account_evaluated_at", "data_evaluated_at"] {
            let expected = DateTime::parse_from_rfc3339(value[key].as_str().unwrap())
                .unwrap()
                .to_rfc3339();
            assert!(text.contains(&format!("{key}={expected}")), "{text}");
        }
        for key in [
            "observed_age_seconds",
            "account_age_seconds",
            "data_age_seconds",
        ] {
            assert!(
                text.contains(&format!("{key}={}", value[key].as_i64().unwrap())),
                "{text}"
            );
        }
    }

    #[test]
    fn health_snapshot_is_bound_to_current_monitor_lease_owner() {
        let root = tempfile::tempdir().unwrap();
        let lease_file = lease_path(root.path(), false);
        let lease = crate::acquire_monitor_instance_lease_at(&lease_file).unwrap();
        assert_eq!(
            monitor_lease_identity(&lease_file),
            Some(lease.boot_id.clone())
        );
        let mut current = snapshot(Utc::now());
        current.boot_id = lease.boot_id.clone();
        write_snapshot_at(&snapshot_path(root.path(), false), &current).unwrap();
        assert_eq!(report_at(root.path(), false, Utc::now()).status, "ok");
        drop(lease);
        assert_eq!(
            report_at(root.path(), false, Utc::now()).reason_code,
            Some("monitor_not_running")
        );
        let replacement = crate::acquire_monitor_instance_lease_at(&lease_file).unwrap();
        assert_ne!(replacement.boot_id, current.boot_id);
        assert_eq!(
            report_at(root.path(), false, Utc::now()).reason_code,
            Some("health_snapshot_process_mismatch")
        );
    }
}
