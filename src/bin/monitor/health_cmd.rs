//! Read-only operational health command backed by the live banner owner.
//!
//! The banner and process lease determine overall health. The four registered
//! raw GlobalNews feeds have a separately scoped recovery observation.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const SNAPSHOT_VERSION: u8 = 2;
const HEARTBEAT_VERSION: u8 = 1;
const MAX_SNAPSHOT_BYTES: u64 = 4_096;
const MAX_SNAPSHOT_AGE: Duration = Duration::minutes(10);
const MAX_CLOCK_LEAD: Duration = Duration::seconds(5);
#[path = "health_cmd_source_recovery.rs"]
mod source_recovery;
pub use source_recovery::write_raw_news_source_snapshot;
// Process-liveness reporting policy, not an availability SLA.
pub const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
pub const MAX_HEARTBEAT_AGE: Duration = Duration::minutes(10);

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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProcessHeartbeat {
    version: u8,
    boot_id: String,
    observed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct HealthReport {
    status: &'static str,
    reason_code: Option<&'static str>,
    monitor_running: bool,
    snapshot_fresh: bool,
    heartbeat_fresh: bool,
    heartbeat_status: &'static str,
    heartbeat_reason_code: Option<&'static str>,
    heartbeat_observed_at: Option<DateTime<Utc>>,
    heartbeat_age_seconds: Option<i64>,
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
    raw_news_source_recovery: source_recovery::SourceRecoveryReport,
}

fn snapshot_path(root: &Path, test_mode: bool) -> PathBuf {
    root.join("data")
        .join(if test_mode { "test/health" } else { "health" })
        .join("monitor-banner-v2.json")
}

fn heartbeat_path(root: &Path, test_mode: bool) -> PathBuf {
    root.join("data")
        .join(if test_mode { "test/health" } else { "health" })
        .join("heartbeat.json")
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
    fn accepts(current: Option<DateTime<Utc>>, incoming: DateTime<Utc>) -> bool {
        current.is_none_or(|at| incoming >= at)
    }

    pub fn apply(&mut self, update: EvaluationUpdate) -> (bool, bool) {
        match update {
            EvaluationUpdate::AccountAndData {
                account_at,
                data_at,
            } => {
                let account = Self::accepts(self.account_evaluated_at, account_at);
                let data = Self::accepts(self.data_evaluated_at, data_at);
                if account {
                    self.account_evaluated_at = Some(account_at);
                }
                if data {
                    self.data_evaluated_at = Some(data_at);
                }
                (account, data)
            }
            EvaluationUpdate::Data(at) => {
                let data = Self::accepts(self.data_evaluated_at, at);
                if data {
                    self.data_evaluated_at = Some(at);
                }
                (false, data)
            }
            EvaluationUpdate::Synthetic => {
                *self = Self::default();
                (true, true)
            }
        }
    }

    /// Called under the banner write lock. Each owner contributes its values
    /// only when its evaluation time is accepted, so evidence and mode cannot
    /// come from different evaluation batches.
    pub fn merge_banner(
        &mut self,
        current: Option<&crate::push_templates::BannerCtx>,
        incoming: crate::push_templates::BannerCtx,
        update: EvaluationUpdate,
    ) -> crate::push_templates::BannerCtx {
        let (account, data) = self.apply(update);
        if matches!(update, EvaluationUpdate::Synthetic) {
            return incoming;
        }
        let Some(current) = current else {
            return incoming;
        };
        let mut merged = current.clone();
        if account {
            merged.account_mode = incoming.account_mode;
            merged.total_pos = incoming.total_pos;
            merged.today_pnl = incoming.today_pnl;
            merged.account_metrics_complete = incoming.account_metrics_complete;
            merged.account_fact = incoming.account_fact;
        }
        if data {
            merged.data_mode = incoming.data_mode;
            merged.data_missing_note = incoming.data_missing_note;
        }
        merged
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
    let bytes = serde_json::to_vec(snapshot)
        .map_err(|error| format!("serialize health snapshot: {error}"))?;
    atomic_replace_bytes(path, &bytes, "health snapshot")
}

pub fn write_process_heartbeat(
    test_mode: bool,
    boot_id: &str,
    observed_at: DateTime<Utc>,
) -> Result<(), String> {
    let root = stock_analysis::production_root::root_for_mode(test_mode);
    write_heartbeat_at(&heartbeat_path(root, test_mode), boot_id, observed_at)
}

pub(crate) fn write_heartbeat_at(
    path: &Path,
    boot_id: &str,
    observed_at: DateTime<Utc>,
) -> Result<(), String> {
    let heartbeat = ProcessHeartbeat {
        version: HEARTBEAT_VERSION,
        boot_id: boot_id.to_owned(),
        observed_at,
    };
    validate_heartbeat_shape(&heartbeat).map_err(str::to_owned)?;
    let bytes = serde_json::to_vec(&heartbeat)
        .map_err(|error| format!("serialize process heartbeat: {error}"))?;
    atomic_replace_bytes(path, &bytes, "process heartbeat")
}

fn atomic_replace_bytes(path: &Path, bytes: &[u8], kind: &str) -> Result<(), String> {
    atomic_replace_bytes_with(path, bytes, kind, || Ok(()))
}

fn atomic_replace_bytes_with(
    path: &Path,
    bytes: &[u8],
    kind: &str,
    before_rename: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(format!("{kind} exceeds size limit"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{kind} path has no parent"))?;
    std::fs::create_dir_all(parent).map_err(|error| format!("create health directory: {error}"))?;
    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = parent.join(format!(
        ".monitor-health-{}-{}-{}.tmp",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
            .map_err(|error| format!("create {kind} temp: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("write {kind}: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync {kind}: {error}"))?;
        before_rename()?;
        std::fs::rename(&temp, path).map_err(|error| format!("replace {kind}: {error}"))?;
        // A failure here is a durability error: rename already replaced the destination.
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync health directory after replacing {kind}: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn valid_heartbeat_boot_id(value: &str) -> bool {
    value.len() <= 96
        && value.split(':').count() == 3
        && value
            .split(':')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_heartbeat_shape(heartbeat: &ProcessHeartbeat) -> Result<(), &'static str> {
    if heartbeat.version != HEARTBEAT_VERSION
        || !valid_heartbeat_boot_id(&heartbeat.boot_id)
        || heartbeat.observed_at.timestamp() < 0
    {
        return Err("process_heartbeat_invalid");
    }
    Ok(())
}

fn validate_heartbeat_at(
    heartbeat: &ProcessHeartbeat,
    now: DateTime<Utc>,
) -> Result<(), &'static str> {
    validate_heartbeat_shape(heartbeat)?;
    if heartbeat.observed_at > now + MAX_CLOCK_LEAD {
        return Err("process_heartbeat_invalid");
    }
    Ok(())
}

fn read_heartbeat_at(path: &Path, now: DateTime<Utc>) -> Result<ProcessHeartbeat, &'static str> {
    let file = std::fs::File::open(path).map_err(|_| "process_heartbeat_unavailable")?;
    let mut bytes = Vec::new();
    file.take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "process_heartbeat_unavailable")?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err("process_heartbeat_invalid");
    }
    let heartbeat: ProcessHeartbeat =
        serde_json::from_slice(&bytes).map_err(|_| "process_heartbeat_invalid")?;
    validate_heartbeat_at(&heartbeat, now)?;
    Ok(heartbeat)
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
    let lease_identity = monitor_lease_identity(&lease_path(root, test_mode));
    let mut report = report_from(
        read_snapshot_at(&snapshot_path(root, test_mode)),
        read_heartbeat_at(&heartbeat_path(root, test_mode), now),
        lease_identity.clone(),
        now,
    );
    report.raw_news_source_recovery =
        source_recovery::report_at(root, test_mode, lease_identity.as_deref(), now);
    report
}

fn fresh_at(at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    let age = now.signed_duration_since(at);
    age >= -MAX_CLOCK_LEAD && age <= MAX_SNAPSHOT_AGE
}

fn report_from(
    snapshot: Result<HealthSnapshot, &'static str>,
    heartbeat: Result<ProcessHeartbeat, &'static str>,
    lease_identity: Option<String>,
    now: DateTime<Utc>,
) -> HealthReport {
    let mut report = HealthReport {
        status: "unhealthy",
        reason_code: None,
        monitor_running: lease_identity.is_some(),
        snapshot_fresh: false,
        heartbeat_fresh: false,
        heartbeat_status: "unhealthy",
        heartbeat_reason_code: None,
        heartbeat_observed_at: None,
        heartbeat_age_seconds: None,
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
        coverage: "banner_account_data_and_process_liveness_only",
        raw_news_source_recovery: source_recovery::SourceRecoveryReport::unavailable(),
    };
    match heartbeat {
        Err(reason) => report.heartbeat_reason_code = Some(reason),
        Ok(heartbeat) => {
            let age = now.signed_duration_since(heartbeat.observed_at);
            report.heartbeat_observed_at = Some(heartbeat.observed_at);
            report.heartbeat_age_seconds = Some(age.num_seconds());
            if age < -MAX_CLOCK_LEAD {
                report.heartbeat_reason_code = Some("process_heartbeat_invalid");
            } else if age > MAX_HEARTBEAT_AGE {
                report.heartbeat_reason_code = Some("process_heartbeat_stale");
            } else {
                report.heartbeat_fresh = true;
                if lease_identity.is_none() {
                    report.heartbeat_reason_code = Some("process_heartbeat_monitor_not_running");
                } else if lease_identity.as_deref() != Some(heartbeat.boot_id.as_str()) {
                    report.heartbeat_reason_code = Some("process_heartbeat_process_mismatch");
                } else {
                    report.heartbeat_status = "ok";
                }
            }
        }
    }
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
        } else if let Some(reason) = report.heartbeat_reason_code {
            report.reason_code = Some(reason);
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
            "status={} reason={} monitor_running={} snapshot_fresh={} heartbeat_fresh={} heartbeat_status={} heartbeat_reason_code={} heartbeat_observed_at={} heartbeat_age_seconds={} observed_at={} observed_age_seconds={} account_evaluated_at={} account_age_seconds={} data_evaluated_at={} data_age_seconds={} account_mode={} data_mode={} account_metrics_complete={} missing_capabilities={} coverage={} raw_news_source_recovery_status={} raw_news_source_recovery_reason={} raw_news_source_recovery_sources={}",
            report.status,
            report.reason_code.unwrap_or("none"),
            report.monitor_running,
            report.snapshot_fresh,
            report.heartbeat_fresh,
            report.heartbeat_status,
            report.heartbeat_reason_code.unwrap_or("none"),
            report.heartbeat_observed_at.map_or_else(|| "missing".to_owned(), |at| at.to_rfc3339()),
            report.heartbeat_age_seconds.map_or_else(|| "missing".to_owned(), |age| age.to_string()),
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
            report.raw_news_source_recovery.status,
            report.raw_news_source_recovery.reason_code.unwrap_or("none"),
            report.raw_news_source_recovery.source_summary(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heartbeat(at: DateTime<Utc>) -> ProcessHeartbeat {
        ProcessHeartbeat {
            version: HEARTBEAT_VERSION,
            boot_id: "123:456:1".to_owned(),
            observed_at: at,
        }
    }

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
        let report = report_from(
            read_snapshot_at(&path),
            Ok(heartbeat(now)),
            Some("123:456:1".to_owned()),
            now,
        );
        assert_eq!(report.status, "ok");
        assert!(report.reason_code.is_none());
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["coverage"],
            "banner_account_data_and_process_liveness_only"
        );
        assert_eq!(parsed["account_mode"], "Normal");
        assert!(parsed.get("today_pnl").is_none());
    }

    #[test]
    fn health_snapshot_fails_closed_for_missing_stale_dead_or_unhealthy_banner() {
        let now = Utc::now();
        assert_eq!(
            report_from(
                Err("health_snapshot_unavailable"),
                Ok(heartbeat(now)),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("health_snapshot_unavailable")
        );
        assert_eq!(
            report_from(
                Ok(snapshot(now - Duration::minutes(11))),
                Ok(heartbeat(now)),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("health_snapshot_stale")
        );
        assert_eq!(
            report_from(Ok(snapshot(now)), Ok(heartbeat(now)), None, now).reason_code,
            Some("monitor_not_running")
        );
        assert_eq!(
            report_from(
                Ok(snapshot(now)),
                Ok(heartbeat(now)),
                Some("123:456:2".to_owned()),
                now
            )
            .reason_code,
            Some("health_snapshot_process_mismatch")
        );
        let mut degraded = snapshot(now);
        degraded.data_mode = "Degraded".to_owned();
        degraded.missing_capabilities = vec!["Quote".to_owned()];
        assert_eq!(
            report_from(
                Ok(degraded),
                Ok(heartbeat(now)),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("banner_unhealthy")
        );
        let mut frozen = snapshot(now);
        frozen.account_mode = "Frozen".to_owned();
        assert_eq!(
            report_from(
                Ok(frozen),
                Ok(heartbeat(now)),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("banner_unhealthy")
        );
        let mut unsafe_data = snapshot(now);
        unsafe_data.data_mode = "Unsafe".to_owned();
        assert_eq!(
            report_from(
                Ok(unsafe_data),
                Ok(heartbeat(now)),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
            Some("banner_unhealthy")
        );
        let mut incomplete = snapshot(now);
        incomplete.account_metrics_complete = false;
        assert_eq!(
            report_from(
                Ok(incomplete),
                Ok(heartbeat(now)),
                Some("123:456:1".to_owned()),
                now
            )
            .reason_code,
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
            Ok(heartbeat(hour_later)),
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
                Ok(heartbeat(hour_later)),
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
    fn out_of_order_evaluations_keep_each_owners_values_with_its_time() {
        use crate::push_templates::{AccountMode, BannerCtx, DataMode};
        let root = tempfile::tempdir().unwrap();
        let path = snapshot_path(root.path(), false);
        let t0 = DateTime::parse_from_rfc3339("2026-09-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = t0 + Duration::minutes(1);
        let t2 = t0 + Duration::minutes(2);
        let t3 = t0 + Duration::minutes(3);
        let t4 = t0 + Duration::minutes(4);
        let t5 = t0 + Duration::minutes(5);
        let old = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            account_fact: None,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        let mut newer = old.clone();
        newer.account_mode = AccountMode::Frozen;
        newer.total_pos = Some(8);
        newer.data_mode = DataMode::Degraded;
        newer.data_missing_note = Some("Quote".to_owned());
        let mut times = EvaluationTimes::default();
        let mut banner = times.merge_banner(
            None,
            old.clone(),
            EvaluationUpdate::AccountAndData {
                account_at: t0,
                data_at: t0,
            },
        );
        banner = times.merge_banner(
            Some(&banner),
            newer,
            EvaluationUpdate::AccountAndData {
                account_at: t3,
                data_at: t1,
            },
        );
        banner = times.merge_banner(
            Some(&banner),
            old.clone(),
            EvaluationUpdate::AccountAndData {
                account_at: t0,
                data_at: t0,
            },
        );
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, t3).unwrap();
        let saved = read_snapshot_at(&path).unwrap();
        assert_eq!(saved.account_evaluated_at, Some(t3));
        assert_eq!(saved.data_evaluated_at, Some(t1));
        assert_eq!(saved.account_mode, "Frozen");
        assert_eq!(saved.data_mode, "Degraded");
        assert_eq!(banner.total_pos, Some(8));

        // DataMode started from the old banner before the newer account commit.
        let mut stale_read = old.clone();
        stale_read.data_mode = DataMode::Unsafe;
        banner = times.merge_banner(Some(&banner), stale_read, EvaluationUpdate::Data(t4));
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, t4).unwrap();
        let saved = read_snapshot_at(&path).unwrap();
        assert_eq!(saved.account_evaluated_at, Some(t3));
        assert_eq!(saved.data_evaluated_at, Some(t4));
        assert_eq!(saved.account_mode, "Frozen");
        assert_eq!(saved.data_mode, "Unsafe");
        assert_eq!(banner.total_pos, Some(8));

        // A newer account evaluation may bring an older data result; data stays Unsafe.
        banner = times.merge_banner(
            Some(&banner),
            old,
            EvaluationUpdate::AccountAndData {
                account_at: t5,
                data_at: t2,
            },
        );
        write_banner_snapshot_at(&path, "123:456:1", &banner, times, t5).unwrap();
        let saved = read_snapshot_at(&path).unwrap();
        assert_eq!(saved.account_evaluated_at, Some(t5));
        assert_eq!(saved.data_evaluated_at, Some(t4));
        assert_eq!(saved.account_mode, "Normal");
        assert_eq!(saved.data_mode, "Unsafe");
        assert_eq!(banner.total_pos, Some(0));
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
                report_from(
                    Ok(item.clone()),
                    Ok(heartbeat(now)),
                    Some(item.boot_id.clone()),
                    now
                )
                .reason_code,
                Some(reason)
            );
        }
        item = snapshot(now + Duration::seconds(6));
        assert_eq!(
            report_from(Ok(item), Ok(heartbeat(now)), Some("123:456:1".into()), now).reason_code,
            Some("health_snapshot_stale")
        );
    }

    #[test]
    fn text_and_json_expose_matching_evidence() {
        let now = Utc::now();
        let report = report_from(
            Ok(snapshot(now - Duration::seconds(3))),
            Ok(heartbeat(now - Duration::seconds(3))),
            Some("123:456:1".into()),
            now,
        );
        let value = serde_json::to_value(&report).unwrap();
        let text = render_text(&report);
        for key in [
            "observed_at",
            "account_evaluated_at",
            "data_evaluated_at",
            "heartbeat_observed_at",
        ] {
            let expected = DateTime::parse_from_rfc3339(value[key].as_str().unwrap())
                .unwrap()
                .to_rfc3339();
            assert!(text.contains(&format!("{key}={expected}")), "{text}");
        }
        for key in [
            "observed_age_seconds",
            "account_age_seconds",
            "data_age_seconds",
            "heartbeat_age_seconds",
        ] {
            assert!(
                text.contains(&format!("{key}={}", value[key].as_i64().unwrap())),
                "{text}"
            );
        }
        for key in ["status", "heartbeat_status", "coverage"] {
            assert!(
                text.contains(&format!("{key}={}", value[key].as_str().unwrap())),
                "{text}"
            );
        }
        assert!(text.contains("heartbeat_fresh=true"));
        assert!(text.contains("heartbeat_reason_code=none"));
        assert_eq!(value["heartbeat_reason_code"], serde_json::Value::Null);
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
        write_heartbeat_at(
            &heartbeat_path(root.path(), false),
            &lease.boot_id,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(report_at(root.path(), false, Utc::now()).status, "ok");
        drop(lease);
        let stopped = report_at(root.path(), false, Utc::now());
        assert_eq!(stopped.reason_code, Some("monitor_not_running"));
        assert_eq!(
            stopped.heartbeat_reason_code,
            Some("process_heartbeat_monitor_not_running")
        );
        let replacement = crate::acquire_monitor_instance_lease_at(&lease_file).unwrap();
        assert_ne!(replacement.boot_id, current.boot_id);
        let turned = report_at(root.path(), false, Utc::now());
        assert_eq!(turned.reason_code, Some("health_snapshot_process_mismatch"));
        assert_eq!(
            turned.heartbeat_reason_code,
            Some("process_heartbeat_process_mismatch")
        );
        current.boot_id = replacement.boot_id.clone();
        write_snapshot_at(&snapshot_path(root.path(), false), &current).unwrap();
        let old_heartbeat = report_at(root.path(), false, Utc::now());
        assert_eq!(
            old_heartbeat.reason_code,
            Some("process_heartbeat_process_mismatch")
        );
        write_heartbeat_at(
            &heartbeat_path(root.path(), false),
            &replacement.boot_id,
            Utc::now(),
        )
        .unwrap();
        assert_eq!(report_at(root.path(), false, Utc::now()).status, "ok");
    }

    #[test]
    fn heartbeat_failures_are_visible_and_gate_an_otherwise_healthy_report() {
        let root = tempfile::tempdir().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let banner_path = snapshot_path(root.path(), false);
        let heartbeat_path = heartbeat_path(root.path(), false);
        write_snapshot_at(&banner_path, &snapshot(now)).unwrap();
        let report = |banner| {
            report_from(
                banner,
                read_heartbeat_at(&heartbeat_path, now),
                Some("123:456:1".to_owned()),
                now,
            )
        };
        let missing = report(read_snapshot_at(&banner_path));
        assert_eq!(missing.reason_code, Some("process_heartbeat_unavailable"));
        assert_eq!(missing.heartbeat_reason_code, missing.reason_code);
        assert_eq!(missing.heartbeat_status, "unhealthy");
        assert!(!missing.heartbeat_fresh);
        let missing_json = serde_json::to_value(&missing).unwrap();
        let missing_text = render_text(&missing);
        assert_eq!(
            missing_json["heartbeat_reason_code"],
            "process_heartbeat_unavailable"
        );
        assert_eq!(missing_json["heartbeat_fresh"], false);
        assert!(missing_text.contains("heartbeat_reason_code=process_heartbeat_unavailable"));
        assert!(missing_text.contains("heartbeat_fresh=false"));
        assert!(missing_text.contains("heartbeat_observed_at=missing"));

        std::fs::write(&heartbeat_path, b"{bad json}").unwrap();
        let invalid = report(read_snapshot_at(&banner_path));
        assert_eq!(invalid.reason_code, Some("process_heartbeat_invalid"));
        assert_eq!(invalid.heartbeat_observed_at, None);
        write_heartbeat_at(
            &heartbeat_path,
            "123:456:1",
            now - MAX_HEARTBEAT_AGE - Duration::seconds(1),
        )
        .unwrap();
        let stale = report(read_snapshot_at(&banner_path));
        assert_eq!(stale.reason_code, Some("process_heartbeat_stale"));
        assert_eq!(stale.heartbeat_age_seconds, Some(601));
        assert!(!stale.heartbeat_fresh);

        write_heartbeat_at(
            &heartbeat_path,
            "123:456:1",
            now + MAX_CLOCK_LEAD + Duration::seconds(1),
        )
        .unwrap();
        let future = report(read_snapshot_at(&banner_path));
        assert_eq!(future.reason_code, Some("process_heartbeat_invalid"));
        assert!(!future.heartbeat_fresh);

        write_heartbeat_at(&heartbeat_path, "123:456:2", now).unwrap();
        let wrong_boot = report(read_snapshot_at(&banner_path));
        assert_eq!(
            wrong_boot.reason_code,
            Some("process_heartbeat_process_mismatch")
        );
        assert!(wrong_boot.heartbeat_fresh);
        assert_eq!(wrong_boot.heartbeat_status, "unhealthy");

        write_heartbeat_at(&heartbeat_path, "123:456:1", now).unwrap();
        let current = report(read_snapshot_at(&banner_path));
        assert_eq!(current.status, "ok");
        assert_eq!(current.heartbeat_status, "ok");
        assert!(current.heartbeat_fresh);
        assert_eq!(current.heartbeat_reason_code, None);

        std::fs::remove_file(&heartbeat_path).unwrap();
        let invalid_banner = report(Err("health_snapshot_invalid"));
        assert_eq!(invalid_banner.reason_code, Some("health_snapshot_invalid"));
        assert_eq!(
            invalid_banner.heartbeat_reason_code,
            Some("process_heartbeat_unavailable")
        );
        let stale_banner = report(Ok(snapshot(now - Duration::minutes(11))));
        assert_eq!(stale_banner.reason_code, Some("health_snapshot_stale"));
        assert_eq!(
            stale_banner.heartbeat_reason_code,
            Some("process_heartbeat_unavailable")
        );
    }

    #[test]
    fn process_heartbeat_roundtrips_and_isolates_roots() {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let production = heartbeat_path(root.path(), false);
        let test = heartbeat_path(root.path(), true);
        assert_eq!(production, root.path().join("data/health/heartbeat.json"));
        assert_eq!(test, root.path().join("data/test/health/heartbeat.json"));
        write_heartbeat_at(&production, "123:456:1", now).unwrap();
        assert_eq!(
            read_heartbeat_at(&production, now).unwrap(),
            ProcessHeartbeat {
                version: HEARTBEAT_VERSION,
                boot_id: "123:456:1".to_owned(),
                observed_at: now,
            }
        );
        assert_eq!(
            read_heartbeat_at(&test, now).unwrap_err(),
            "process_heartbeat_unavailable"
        );
        write_heartbeat_at(&test, "789:1000:2", now).unwrap();
        assert_eq!(read_heartbeat_at(&test, now).unwrap().boot_id, "789:1000:2");
        assert_eq!(
            read_heartbeat_at(&production, now).unwrap().boot_id,
            "123:456:1"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&production).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn process_heartbeat_rejects_bad_schema_identity_and_time() {
        let root = tempfile::tempdir().unwrap();
        let path = heartbeat_path(root.path(), false);
        let now = DateTime::parse_from_rfc3339("2026-09-29T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(write_heartbeat_at(&path, "123::1", now).is_err());
        assert!(!path.exists());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for payload in [
            r#"{"version":1,"boot_id":"123:456:1","observed_at":"#,
            r#"{"version":2,"boot_id":"123:456:1","observed_at":"2026-09-29T00:00:00Z"}"#,
            r#"{"version":1,"boot_id":"123:456:1","observed_at":"2026-09-29T00:00:00Z","account_mode":"Normal"}"#,
            r#"{"version":1,"boot_id":"123::1","observed_at":"2026-09-29T00:00:00Z"}"#,
            r#"{"version":1,"boot_id":"123:456:1","observed_at":"invalid"}"#,
            r#"{"version":1,"boot_id":"123:456:1","observed_at":"1969-12-31T23:59:59Z"}"#,
            r#"{"version":1,"boot_id":"123:456:1","observed_at":"2026-09-29T00:00:06Z"}"#,
        ] {
            std::fs::write(&path, payload).unwrap();
            assert_eq!(
                read_heartbeat_at(&path, now).unwrap_err(),
                "process_heartbeat_invalid",
                "{payload}"
            );
        }
        std::fs::write(&path, vec![b'x'; MAX_SNAPSHOT_BYTES as usize + 1]).unwrap();
        assert_eq!(
            read_heartbeat_at(&path, now).unwrap_err(),
            "process_heartbeat_invalid"
        );
        // Old observations remain structurally readable for Task 3's stale result.
        write_heartbeat_at(
            &path,
            "123:456:1",
            now - MAX_HEARTBEAT_AGE - Duration::seconds(1),
        )
        .unwrap();
        assert!(read_heartbeat_at(&path, now).is_ok());
        assert_eq!(HEARTBEAT_INTERVAL.as_secs(), 60);
    }

    #[test]
    fn process_heartbeat_pre_rename_failure_preserves_prior_file_and_cleans_temp() {
        let root = tempfile::tempdir().unwrap();
        let path = heartbeat_path(root.path(), false);
        let now = Utc::now();
        write_heartbeat_at(&path, "123:456:1", now).unwrap();
        let prior = std::fs::read(&path).unwrap();
        let replacement = serde_json::to_vec(&ProcessHeartbeat {
            version: HEARTBEAT_VERSION,
            boot_id: "123:456:2".to_owned(),
            observed_at: now + Duration::seconds(1),
        })
        .unwrap();
        let error = atomic_replace_bytes_with(&path, &replacement, "process heartbeat", || {
            Err("injected failure before rename".to_owned())
        })
        .unwrap_err();
        assert_eq!(error, "injected failure before rename");
        assert_eq!(std::fs::read(&path).unwrap(), prior);
        let files: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files, vec!["heartbeat.json"]);
    }

    #[test]
    fn health_command_reports_scoped_raw_news_recovery_without_changing_banner_verdict() {
        let root = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let lease_file = lease_path(root.path(), false);
        let lease = crate::acquire_monitor_instance_lease_at(&lease_file).unwrap();
        let mut banner = snapshot(now);
        banner.boot_id = lease.boot_id.clone();
        write_snapshot_at(&snapshot_path(root.path(), false), &banner).unwrap();
        write_heartbeat_at(&heartbeat_path(root.path(), false), &lease.boot_id, now).unwrap();
        let registry = stock_analysis::news::aggregator::raw_v2::GlobalNewsSourceRegistry::new();
        source_recovery::write_at(
            &source_recovery::path(root.path(), false),
            &lease.boot_id,
            &registry,
            now,
        )
        .unwrap();
        let report = report_at(root.path(), false, now);
        assert_eq!(report.status, "ok");
        assert_eq!(report.raw_news_source_recovery.status, "warming");
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["raw_news_source_recovery"]["sources"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            json["raw_news_source_recovery"]["coverage"],
            "four_registered_raw_global_news_feeds_only"
        );
    }
}
