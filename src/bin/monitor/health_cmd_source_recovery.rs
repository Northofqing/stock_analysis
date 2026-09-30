//! Cross-process, read-only view of the resident raw GlobalNews source registry.
//! The file carries one monitor boot identity and exactly four allowlisted feeds.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use stock_analysis::news::aggregator::raw_v2::{
    registered_global_news_feeds, GlobalNewsSourceRegistry, SourceBreakerState,
    SourceRecoverySnapshot, GLOBAL_NEWS_BREAKER_COVERAGE,
};

const VERSION: u8 = 1;

pub(super) fn path(root: &Path, test_mode: bool) -> PathBuf {
    root.join("data")
        .join(if test_mode { "test/health" } else { "health" })
        .join("raw-global-news-sources-v1.json")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceRecoveryFile {
    version: u8,
    boot_id: String,
    observed_at: DateTime<Utc>,
    coverage: String,
    registry_started_at: DateTime<Utc>,
    sources: Vec<SourceRecoveryRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceRecoveryRow {
    provider: String,
    breaker_state: String,
    warming: bool,
    consecutive_retryable_failures: u32,
    last_attempt_at: Option<DateTime<Utc>>,
    last_successful_pull_at: Option<DateTime<Utc>>,
    outage_started_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    opened_at: Option<DateTime<Utc>>,
    next_probe_at: Option<DateTime<Utc>>,
    last_reason_code: Option<String>,
    last_retryable: Option<bool>,
}

impl SourceRecoveryRow {
    fn from_snapshot(source: SourceRecoverySnapshot) -> Self {
        Self {
            provider: source.registration.provider.wire_name().to_owned(),
            breaker_state: match source.state {
                SourceBreakerState::Closed => "closed",
                SourceBreakerState::Open => "open",
                SourceBreakerState::HalfOpen => "half_open",
            }
            .to_owned(),
            warming: source.warming,
            consecutive_retryable_failures: source.consecutive_retryable_failures,
            last_attempt_at: source.last_attempt_at,
            last_successful_pull_at: source.last_successful_pull_at,
            outage_started_at: source.outage_started_at,
            last_failure_at: source.last_failure_at,
            opened_at: source.opened_at,
            next_probe_at: source.next_probe_at,
            last_reason_code: source.last_reason_code.map(str::to_owned),
            last_retryable: source.last_retryable,
        }
    }

    fn degraded(&self) -> bool {
        self.breaker_state != "closed" || self.outage_started_at.is_some()
    }
}

fn validate(file: &SourceRecoveryFile) -> Result<(), &'static str> {
    if file.version != VERSION
        || !super::valid_boot_id(&file.boot_id)
        || file.coverage != GLOBAL_NEWS_BREAKER_COVERAGE
        || file.registry_started_at > file.observed_at + super::MAX_CLOCK_LEAD
        || file.sources.len() != 4
    {
        return Err("raw_news_source_snapshot_invalid");
    }
    for (source, expected) in file
        .sources
        .iter()
        .zip(registered_global_news_feeds().iter())
    {
        if source.provider != expected.provider.wire_name()
            || !matches!(
                source.breaker_state.as_str(),
                "closed" | "open" | "half_open"
            )
            || source.warming != source.last_attempt_at.is_none()
            || source.last_reason_code.as_ref().is_some_and(|reason| {
                reason.is_empty()
                    || reason.len() > 64
                    || !reason.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
            || source.last_failure_at.is_some() != source.last_reason_code.is_some()
            || source.last_failure_at.is_some() != source.last_retryable.is_some()
            || source.last_successful_pull_at.is_some() && source.last_attempt_at.is_none()
            || match source.breaker_state.as_str() {
                "closed" => source.opened_at.is_some() || source.next_probe_at.is_some(),
                "open" => {
                    source.opened_at.is_none()
                        || source.next_probe_at.is_none()
                        || source.outage_started_at.is_none()
                }
                "half_open" => {
                    source.opened_at.is_none()
                        || source.next_probe_at.is_some()
                        || source.outage_started_at.is_none()
                }
                _ => true,
            }
            || [
                source.last_attempt_at,
                source.last_successful_pull_at,
                source.outage_started_at,
                source.last_failure_at,
                source.opened_at,
            ]
            .into_iter()
            .flatten()
            .any(|at| at > file.observed_at + super::MAX_CLOCK_LEAD)
        {
            return Err("raw_news_source_snapshot_invalid");
        }
    }
    Ok(())
}

pub(super) fn write_at(
    path: &Path,
    boot_id: &str,
    registry: &GlobalNewsSourceRegistry,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let snapshots = registry.snapshot().map_err(|error| error.to_string())?;
    let registry_started_at = snapshots
        .first()
        .ok_or("raw GlobalNews registry has no sources")?
        .registry_started_at;
    let file = SourceRecoveryFile {
        version: VERSION,
        boot_id: boot_id.to_owned(),
        observed_at: now,
        coverage: GLOBAL_NEWS_BREAKER_COVERAGE.to_owned(),
        registry_started_at,
        sources: snapshots
            .into_iter()
            .map(SourceRecoveryRow::from_snapshot)
            .collect(),
    };
    validate(&file).map_err(str::to_owned)?;
    let bytes = serde_json::to_vec(&file)
        .map_err(|error| format!("serialize raw news source snapshot: {error}"))?;
    super::atomic_replace_bytes(path, &bytes, "raw news source snapshot")
}

pub fn write_raw_news_source_snapshot(
    test_mode: bool,
    boot_id: &str,
    registry: &GlobalNewsSourceRegistry,
) -> Result<(), String> {
    let root = stock_analysis::production_root::root_for_mode(test_mode);
    write_at(&path(root, test_mode), boot_id, registry, Utc::now())
}

fn read_at(path: &Path) -> Result<SourceRecoveryFile, &'static str> {
    let file = std::fs::File::open(path).map_err(|_| "raw_news_source_snapshot_unavailable")?;
    let mut bytes = Vec::new();
    file.take(super::MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "raw_news_source_snapshot_unavailable")?;
    if bytes.len() as u64 > super::MAX_SNAPSHOT_BYTES {
        return Err("raw_news_source_snapshot_invalid");
    }
    let decoded: SourceRecoveryFile =
        serde_json::from_slice(&bytes).map_err(|_| "raw_news_source_snapshot_invalid")?;
    validate(&decoded)?;
    Ok(decoded)
}

#[derive(Debug, Serialize)]
pub(super) struct SourceRecoveryReport {
    pub(super) status: &'static str,
    pub(super) reason_code: Option<&'static str>,
    coverage: &'static str,
    observed_at: Option<DateTime<Utc>>,
    observed_age_seconds: Option<i64>,
    sources: Vec<SourceRecoveryRow>,
}

impl SourceRecoveryReport {
    pub(super) fn unavailable() -> Self {
        Self::with_reason("raw_news_source_snapshot_unavailable")
    }

    fn with_reason(reason_code: &'static str) -> Self {
        Self {
            status: "unavailable",
            reason_code: Some(reason_code),
            coverage: GLOBAL_NEWS_BREAKER_COVERAGE,
            observed_at: None,
            observed_age_seconds: None,
            sources: Vec::new(),
        }
    }

    pub(super) fn source_summary(&self) -> String {
        if self.sources.is_empty() {
            return "none".to_owned();
        }
        let checked_at = self
            .observed_at
            .zip(self.observed_age_seconds)
            .map(|(observed_at, age)| observed_at + chrono::Duration::seconds(age));
        self.sources
            .iter()
            .map(|source| {
                let mut summary = format!("{}:{}", source.provider, source.breaker_state);
                if source.warming {
                    summary.push_str(":warming");
                }
                if let Some(outage_at) = source.outage_started_at {
                    if let Some(checked_at) = checked_at {
                        summary.push_str(&format!(
                            ":outage_age_s={}",
                            checked_at
                                .signed_duration_since(outage_at)
                                .num_seconds()
                                .max(0)
                        ));
                    }
                    if let Some(reason) = source.last_reason_code.as_deref() {
                        summary.push_str(":reason=");
                        summary.push_str(reason);
                    }
                } else if let (Some(checked_at), Some(success_at)) =
                    (checked_at, source.last_successful_pull_at)
                {
                    summary.push_str(&format!(
                        ":last_success_age_s={}",
                        checked_at
                            .signed_duration_since(success_at)
                            .num_seconds()
                            .max(0)
                    ));
                }
                summary
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn report_from(
    file: Result<SourceRecoveryFile, &'static str>,
    lease_identity: Option<&str>,
    now: DateTime<Utc>,
) -> SourceRecoveryReport {
    let file = match file {
        Ok(file) => file,
        Err(reason) => return SourceRecoveryReport::with_reason(reason),
    };
    let age = now.signed_duration_since(file.observed_at);
    if age < -super::MAX_CLOCK_LEAD || age > super::MAX_SNAPSHOT_AGE {
        return SourceRecoveryReport::with_reason("raw_news_source_snapshot_stale");
    }
    if lease_identity != Some(file.boot_id.as_str()) {
        return SourceRecoveryReport::with_reason("raw_news_source_snapshot_process_mismatch");
    }
    let (status, reason_code) = if file.sources.iter().any(SourceRecoveryRow::degraded) {
        ("degraded", Some("raw_news_source_degraded"))
    } else if file
        .sources
        .iter()
        .any(|source| source.warming || source.last_successful_pull_at.is_none())
    {
        ("warming", Some("raw_news_sources_warming"))
    } else if file.sources.iter().any(|source| {
        !super::fresh_at(
            source
                .last_successful_pull_at
                .expect("warm sources handled above"),
            now,
        )
    }) {
        ("idle", Some("raw_news_source_success_stale"))
    } else {
        ("ok", None)
    };
    SourceRecoveryReport {
        status,
        reason_code,
        coverage: GLOBAL_NEWS_BREAKER_COVERAGE,
        observed_at: Some(file.observed_at),
        observed_age_seconds: Some(age.num_seconds()),
        sources: file.sources,
    }
}

pub(super) fn report_at(
    root: &Path,
    test_mode: bool,
    lease_identity: Option<&str>,
    now: DateTime<Utc>,
) -> SourceRecoveryReport {
    report_from(read_at(&path(root, test_mode)), lease_identity, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_news_source_snapshot_is_boot_scoped_and_never_claims_warm_sources_ready() {
        let root = tempfile::tempdir().unwrap();
        let at = Utc::now();
        let registry = GlobalNewsSourceRegistry::new();
        write_at(&path(root.path(), true), "123:456:1", &registry, at).unwrap();
        let warming = report_at(root.path(), true, Some("123:456:1"), at);
        assert_eq!(warming.status, "warming");
        assert_eq!(warming.sources.len(), 4);
        assert!(warming.sources.iter().all(|source| source.warming));
        let wrong_boot = report_at(root.path(), true, Some("123:456:2"), at);
        assert_eq!(wrong_boot.status, "unavailable");
        assert!(wrong_boot.sources.is_empty());
        assert_eq!(
            wrong_boot.reason_code,
            Some("raw_news_source_snapshot_process_mismatch")
        );
        let stale = report_at(
            root.path(),
            true,
            Some("123:456:1"),
            at + super::super::MAX_SNAPSHOT_AGE + chrono::Duration::seconds(1),
        );
        assert_eq!(stale.reason_code, Some("raw_news_source_snapshot_stale"));
        assert!(stale.sources.is_empty());
    }

    #[test]
    fn raw_news_source_snapshot_rejects_missing_or_reordered_feeds_and_reports_outage() {
        let at = Utc::now();
        let registry = GlobalNewsSourceRegistry::new();
        let root = tempfile::tempdir().unwrap();
        let file_path = path(root.path(), true);
        write_at(&file_path, "123:456:1", &registry, at).unwrap();
        let mut file = read_at(&file_path).unwrap();
        file.sources[0].warming = false;
        file.sources[0].last_attempt_at = Some(at);
        file.sources[0].breaker_state = "open".to_owned();
        file.sources[0].consecutive_retryable_failures = 10;
        file.sources[0].outage_started_at = Some(at);
        file.sources[0].last_failure_at = Some(at);
        file.sources[0].opened_at = Some(at);
        file.sources[0].next_probe_at = Some(at + chrono::Duration::seconds(60));
        file.sources[0].last_reason_code = Some("provider_transport".to_owned());
        file.sources[0].last_retryable = Some(true);
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();
        let degraded = report_at(root.path(), true, Some("123:456:1"), at);
        assert_eq!(degraded.status, "degraded");
        assert_eq!(degraded.sources[0].breaker_state, "open");
        file.sources.swap(0, 1);
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();
        let invalid = report_at(root.path(), true, Some("123:456:1"), at);
        assert_eq!(
            invalid.reason_code,
            Some("raw_news_source_snapshot_invalid")
        );
        assert!(invalid.sources.is_empty());
    }

    #[test]
    fn raw_news_source_ready_requires_recent_verified_pull_for_every_feed() {
        let at = Utc::now();
        let registry = GlobalNewsSourceRegistry::new();
        let root = tempfile::tempdir().unwrap();
        let file_path = path(root.path(), true);
        write_at(&file_path, "123:456:1", &registry, at).unwrap();
        let mut file = read_at(&file_path).unwrap();
        for source in &mut file.sources {
            source.warming = false;
            source.last_attempt_at = Some(at);
            source.last_successful_pull_at = Some(at);
        }
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();
        assert_eq!(
            report_at(root.path(), true, Some("123:456:1"), at).status,
            "ok"
        );

        file.sources[0].last_successful_pull_at = None;
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();
        assert_eq!(
            report_at(root.path(), true, Some("123:456:1"), at).status,
            "warming"
        );

        file.sources[0].last_successful_pull_at = Some(at - chrono::Duration::minutes(11));
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();
        let idle = report_at(root.path(), true, Some("123:456:1"), at);
        assert_eq!(idle.status, "idle");
        assert_eq!(idle.reason_code, Some("raw_news_source_success_stale"));
    }

    #[test]
    fn raw_news_source_rejects_contradictory_recovery_fields_before_claiming_ok() {
        let at = Utc::now();
        let registry = GlobalNewsSourceRegistry::new();
        let root = tempfile::tempdir().unwrap();
        let file_path = path(root.path(), true);
        write_at(&file_path, "123:456:1", &registry, at).unwrap();
        let mut healthy = read_at(&file_path).unwrap();
        for source in &mut healthy.sources {
            source.warming = false;
            source.last_attempt_at = Some(at);
            source.last_successful_pull_at = Some(at);
        }
        let assert_invalid = |file: &SourceRecoveryFile| {
            super::super::atomic_replace_bytes(
                &file_path,
                &serde_json::to_vec(file).unwrap(),
                "test source snapshot",
            )
            .unwrap();
            let report = report_at(root.path(), true, Some("123:456:1"), at);
            assert_eq!(report.status, "unavailable");
            assert_eq!(report.reason_code, Some("raw_news_source_snapshot_invalid"));
        };

        let mut missing_retryable = healthy.clone();
        missing_retryable.sources[0].last_failure_at = Some(at);
        missing_retryable.sources[0].last_reason_code = Some("provider_transport".into());
        assert_invalid(&missing_retryable);

        let mut open_without_probe = healthy;
        open_without_probe.sources[0].breaker_state = "open".into();
        open_without_probe.sources[0].outage_started_at = Some(at);
        open_without_probe.sources[0].opened_at = Some(at);
        assert_invalid(&open_without_probe);
    }

    #[test]
    fn text_summary_exposes_closed_breaker_outage_and_recovery_age() {
        let at = Utc::now();
        let registry = GlobalNewsSourceRegistry::new();
        let root = tempfile::tempdir().unwrap();
        let file_path = path(root.path(), true);
        write_at(&file_path, "123:456:1", &registry, at).unwrap();
        let mut file = read_at(&file_path).unwrap();
        let source = &mut file.sources[0];
        source.warming = false;
        source.last_attempt_at = Some(at);
        source.outage_started_at = Some(at);
        source.last_failure_at = Some(at);
        source.last_reason_code = Some("provider_transport".to_owned());
        source.last_retryable = Some(true);
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();

        let degraded = report_at(
            root.path(),
            true,
            Some("123:456:1"),
            at + chrono::Duration::seconds(17),
        );
        assert_eq!(degraded.status, "degraded");
        assert!(degraded
            .source_summary()
            .contains("Eastmoney:closed:outage_age_s=17:reason=provider_transport"));

        let source = &mut file.sources[0];
        source.outage_started_at = None;
        source.last_successful_pull_at = Some(at + chrono::Duration::seconds(5));
        super::super::atomic_replace_bytes(
            &file_path,
            &serde_json::to_vec(&file).unwrap(),
            "test source snapshot",
        )
        .unwrap();
        let recovered = report_at(
            root.path(),
            true,
            Some("123:456:1"),
            at + chrono::Duration::seconds(17),
        );
        assert!(recovered
            .source_summary()
            .contains("Eastmoney:closed:last_success_age_s=12"));
        assert!(!recovered
            .source_summary()
            .contains("reason=provider_transport"));
    }
}
