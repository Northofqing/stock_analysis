//! Registered business rules: BR-043.
//! History query and success-rate aggregation — v17.3 Task 3
//!
//! Reads persisted JSONL event files and provides:
//! - `HistoryQuery::query()` — filtered, sorted, limited history scan
//! - `HistoryQuery::push_success_rate()` — delivery success rate stats

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs::{self, File};
use tokio::io::{AsyncBufReadExt, BufReader};

// ========================================================================
// HistoryError
// ========================================================================

/// Errors that can occur during history operations.
#[derive(Error, Debug)]
pub enum HistoryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("record extraction error: {0}")]
    Record(String),
}

// ========================================================================
// HistoryOrder
// ========================================================================

/// Sort order for history results.
#[derive(Debug, Clone, Copy, Default)]
pub enum HistoryOrder {
    #[default]
    Desc,
    Asc,
}

// ========================================================================
// HistoryFilter
// ========================================================================

/// Filter parameters for a history query.
#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    /// Restrict to this specific date (YYYY-MM-DD).
    pub date: Option<NaiveDate>,
    /// Restrict to this stock code (exact match on envelope.entity_key).
    pub code: Option<String>,
    /// Restrict to this event kind (exact match on payload.kind).
    pub kind: Option<String>,
    /// Maximum number of entries to return.
    pub limit: usize,
    /// Sort order for results.
    pub order: HistoryOrder,
}

// ========================================================================
// Window
// ========================================================================

/// A time window for rate aggregation.
#[derive(Debug, Clone)]
pub enum Window {
    Hours(u32),
    Days(u32),
}

impl Window {
    fn to_chrono(&self) -> chrono::Duration {
        match self {
            Window::Hours(h) => chrono::Duration::hours(*h as i64),
            Window::Days(d) => chrono::Duration::days(*d as i64),
        }
    }
}

// ========================================================================
// HistoryEntry
// ========================================================================

/// A single history record returned by `HistoryQuery::query()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub kind: String,
    pub code: Option<String>,
    pub ts: DateTime<Local>,
    pub summary: serde_json::Value,
}

/// Format every queried history entry for terminal output.
///
/// Keeping formatting here makes the `limit=0` (unbounded) contract testable
/// across query and presentation instead of adding a second hidden CLI cap.
pub fn format_history_lines(entries: &[HistoryEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| {
            let summary = serde_json::to_string_pretty(&entry.summary)
                .unwrap_or_else(|error| format!("<summary serialization failed: {error}>"));
            format!(
                "  {} {} {:?} {}",
                entry.ts.format("%Y-%m-%d %H:%M:%S"),
                entry.kind,
                entry.code,
                summary
            )
        })
        .collect()
}

// ========================================================================
// RateStats
// ========================================================================

/// Delivery success-rate statistics over a time window.
#[derive(Debug, Clone)]
pub struct RateStats {
    /// Total number of push.delivery envelopes seen.
    pub total: u64,
    /// Envelopes with outcome = Pushed.
    pub pushed: u64,
    /// Envelopes with outcome = Deduped.
    pub deduped: u64,
    /// Envelopes with outcome = Denied.
    pub denied: u64,
    /// Envelopes with outcome = Failed / SinkError.
    pub failed: u64,
    /// Pushed / (Pushed + Failed); NaN if denominator is 0.
    pub success_rate: f64,
    /// Per-channel success rates.
    pub per_sink_rate: BTreeMap<String, f64>,
    /// Per-kind success rates.
    pub per_kind_rate: BTreeMap<String, f64>,
    /// Average latency_ms across all parsed records.
    pub avg_latency_ms: f64,
    /// Window start (inclusive).
    pub window_start: DateTime<Local>,
    /// Window end (inclusive).
    pub window_end: DateTime<Local>,
}

// ========================================================================
// HistoryQuery
// ========================================================================

/// Queries persisted event history and aggregates delivery statistics.
pub struct HistoryQuery {
    base_dir: PathBuf,
}

impl HistoryQuery {
    /// Create a new HistoryQuery that reads from `base_dir`.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Query persisted envelopes with optional filters.
    ///
    /// Reads one or more JSONL files from `base_dir`, applies filters,
    /// sorts by `ts` descending (default), and returns up to `limit` entries.
    pub async fn query(&self, filter: HistoryFilter) -> Result<Vec<HistoryEntry>, HistoryError> {
        let mut entries = Vec::new();

        // Collect files to read based on date filter
        let files_to_read = if let Some(date) = filter.date {
            let date_str = date.format("%Y-%m-%d").to_string();
            let path = self.base_dir.join(format!("{}.jsonl", date_str));
            vec![path]
        } else {
            // Read all jsonl files
            let mut files = Vec::new();
            let mut dir = fs::read_dir(&self.base_dir).await?;
            while let Some(entry) = dir.next_entry().await? {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    files.push(path);
                }
            }
            files
        };

        for file_path in files_to_read {
            let file = match File::open(&file_path).await {
                Ok(f) => f,
                Err(e) => {
                    log::warn!(
                        "[history] could not open file {}: {}",
                        file_path.display(),
                        e
                    );
                    continue;
                }
            };
            let mut reader = BufReader::new(file).lines();

            while let Some(line) = reader.next_line().await? {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let env: super::envelope::EventEnvelope = match serde_json::from_str(line) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!(
                            "[history] skipping malformed JSON line in {}: {}",
                            file_path.display(),
                            e
                        );
                        continue;
                    }
                };

                // Filter by code (entity_key)
                if let Some(ref code) = filter.code {
                    if env.entity_key.as_deref() != Some(code.as_str()) {
                        continue;
                    }
                }

                // Filter by kind (payload.kind)
                if let Some(ref kind) = filter.kind {
                    if env.payload.get("kind").and_then(|v| v.as_str()) != Some(kind.as_str()) {
                        continue;
                    }
                }

                // Filter by date if specified
                if let Some(date) = filter.date {
                    if env.ts.date_naive() != date {
                        continue;
                    }
                }

                let kind = env
                    .payload
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                entries.push(HistoryEntry {
                    id: env.id,
                    kind,
                    code: env.entity_key,
                    ts: env.ts,
                    summary: env.payload,
                });
            }
        }

        // Sort by ts, then by id for stable ordering
        match filter.order {
            HistoryOrder::Desc => {
                entries.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| b.id.cmp(&a.id)))
            }
            HistoryOrder::Asc => {
                entries.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)))
            }
        }

        // Apply limit after sort
        if filter.limit > 0 {
            entries.truncate(filter.limit);
        }
        Ok(entries)
    }

    /// Compute push delivery success rates over a time window.
    ///
    /// Parses only `push.delivery` envelopes via `PushRecord::try_from`.
    /// Denominator = Pushed + Failed; Deduped and Denied are counted but
    /// do not affect the success rate.
    pub async fn push_success_rate(
        &self,
        kind: Option<&str>,
        window: Window,
        sink: Option<&str>,
    ) -> Result<RateStats, HistoryError> {
        let window_duration = window.to_chrono();
        let window_end = Local::now();
        let window_start = window_end - window_duration;

        let mut all_records: Vec<super::push_record::PushRecord> = Vec::new();

        // Collect files to read based on window dates
        let mut date = window_start.date_naive();
        let end_date = window_end.date_naive();
        let mut files_to_read = Vec::new();

        while date <= end_date {
            let date_str = date.format("%Y-%m-%d").to_string();
            let path = self.base_dir.join(format!("{}.jsonl", date_str));
            files_to_read.push(path);
            date = date.succ_opt().unwrap_or(date);
        }

        for file_path in files_to_read {
            let file = match File::open(&file_path).await {
                Ok(f) => f,
                Err(_) => continue, // File doesn't exist, skip
            };
            let mut reader = BufReader::new(file).lines();

            while let Some(line) = reader.next_line().await? {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let env: super::envelope::EventEnvelope = match serde_json::from_str(line) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!(
                            "[history] skipping malformed JSON line in {}: {}",
                            file_path.display(),
                            e
                        );
                        continue;
                    }
                };

                // Try to extract PushRecord (only for push.delivery event type)
                match super::push_record::PushRecord::try_from(&env) {
                    Ok(record) => {
                        // Filter by window
                        if record.ts < window_start || record.ts > window_end {
                            continue;
                        }
                        // Filter by kind if specified
                        if let Some(k) = kind {
                            if record.kind != k {
                                continue;
                            }
                        }
                        // Filter by sink (channel) if specified
                        if let Some(s) = sink {
                            if record.channel != s {
                                continue;
                            }
                        }
                        all_records.push(record);
                    }
                    Err(_) => {
                        // Not a push.delivery envelope, skip silently
                    }
                }
            }
        }

        // Compute statistics
        let mut total = 0u64;
        let mut pushed = 0u64;
        let mut deduped = 0u64;
        let mut denied = 0u64;
        let mut failed = 0u64;
        let mut total_latency_ms: u64 = 0;
        let mut per_sink_stats: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new(); // channel -> (pushed, failed, count)
        let mut per_kind_stats: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new(); // kind -> (pushed, failed, count)

        for record in &all_records {
            total += 1;
            total_latency_ms += record.latency_ms;

            match record.outcome {
                super::push_record::PushOutcomeLabel::Pushed => pushed += 1,
                super::push_record::PushOutcomeLabel::Deduped => deduped += 1,
                super::push_record::PushOutcomeLabel::Denied => denied += 1,
                super::push_record::PushOutcomeLabel::Failed => failed += 1,
            }

            // Per-sink stats
            let sink_entry = per_sink_stats
                .entry(record.channel.clone())
                .or_insert((0, 0, 0));
            match record.outcome {
                super::push_record::PushOutcomeLabel::Pushed => sink_entry.0 += 1,
                super::push_record::PushOutcomeLabel::Failed => sink_entry.1 += 1,
                _ => {}
            }
            sink_entry.2 += 1;

            // Per-kind stats
            let kind_entry = per_kind_stats
                .entry(record.kind.clone())
                .or_insert((0, 0, 0));
            match record.outcome {
                super::push_record::PushOutcomeLabel::Pushed => kind_entry.0 += 1,
                super::push_record::PushOutcomeLabel::Failed => kind_entry.1 += 1,
                _ => {}
            }
            kind_entry.2 += 1;
        }

        // Compute success rate: pushed / (pushed + failed)
        let denominator = pushed.saturating_add(failed);
        let success_rate = if denominator > 0 {
            pushed as f64 / denominator as f64
        } else {
            f64::NAN
        };

        // Compute per-sink rates
        let per_sink_rate: BTreeMap<String, f64> = per_sink_stats
            .into_iter()
            .map(|(channel, (p, f, _))| {
                let denom = p.saturating_add(f);
                let rate = if denom > 0 {
                    p as f64 / denom as f64
                } else {
                    f64::NAN
                };
                (channel, rate)
            })
            .collect();

        // Compute per-kind rates
        let per_kind_rate: BTreeMap<String, f64> = per_kind_stats
            .into_iter()
            .map(|(kind, (p, f, _))| {
                let denom = p.saturating_add(f);
                let rate = if denom > 0 {
                    p as f64 / denom as f64
                } else {
                    f64::NAN
                };
                (kind, rate)
            })
            .collect();

        // Average latency
        let avg_latency_ms = if total > 0 {
            total_latency_ms as f64 / total as f64
        } else {
            f64::NAN
        };

        Ok(RateStats {
            total,
            pushed,
            deduped,
            denied,
            failed,
            success_rate,
            per_sink_rate,
            per_kind_rate,
            avg_latency_ms,
            window_start,
            window_end,
        })
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::envelope::{EventEnvelope, PushDeliveryEvent};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    fn test_dir(name: &str) -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "history-test-{}-{}-{}",
            name,
            std::process::id(),
            sequence
        ))
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    /// Write a synthetic JSONL envelope line to a file.
    async fn write_envelope_line(dir: &Path, date: &str, line: &str) {
        let path = dir.join(format!("{}.jsonl", date));
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .unwrap();
        use tokio::io::AsyncWriteExt;
        file.write_all(line.as_bytes()).await.unwrap();
        file.write_all(b"\n").await.unwrap();
    }

    /// Create a delivery envelope with the given parameters.
    fn make_delivery_envelope(
        id: &str,
        kind: &str,
        code: Option<&str>,
        outcome: &str,
        channel: &str,
        latency_ms: u64,
        ts: DateTime<Local>,
    ) -> EventEnvelope {
        let event = PushDeliveryEvent::new(
            kind.into(),
            code.map(String::from),
            outcome.into(),
            channel.into(),
            0,
            latency_ms,
        );
        EventEnvelope::from_event(&event, id.into(), format!("trace-{}", id), ts).unwrap()
    }

    /// Create a test directory with given delivery lines.
    async fn test_history_dir_with_records(
        records: Vec<(String, String, Option<String>, String)>,
    ) -> PathBuf {
        let dir = test_dir("query");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let today = today();
        let date_str = today.format("%Y-%m-%d").to_string();

        for (id, kind, code, outcome) in records {
            let env = make_delivery_envelope(
                &id,
                &kind,
                code.as_deref(),
                &outcome,
                "dry_run",
                37,
                today
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_local_timezone(Local)
                    .unwrap(),
            );
            let json = serde_json::to_string(&env).unwrap();
            write_envelope_line(&dir, &date_str, &json).await;
        }

        dir
    }

    // ---------------------------------------------------------------------------
    // Unit tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn history_filters_code_kind_and_desc_limit() {
        let dir = test_history_dir_with_records(vec![
            (
                "a".into(),
                "Announcement".into(),
                Some("TEST_CODE_600519".into()),
                "Pushed".into(),
            ),
            (
                "b".into(),
                "PolicyHit".into(),
                Some("TEST_CODE_000001".into()),
                "Failed".into(),
            ),
            (
                "c".into(),
                "Announcement".into(),
                Some("TEST_CODE_600519".into()),
                "Denied".into(),
            ),
        ])
        .await;

        let result = HistoryQuery::new(dir.clone())
            .query(HistoryFilter {
                date: Some(today()),
                code: Some("TEST_CODE_600519".into()),
                kind: Some("Announcement".into()),
                limit: 1,
                order: HistoryOrder::Desc,
            })
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "c"); // Most recent first (desc)

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn zero_limit_returns_and_formats_all_matching_history_entries() {
        let records = (0..101)
            .map(|index| {
                (
                    format!("id-{index:03}"),
                    "Announcement".into(),
                    Some("TEST_CODE_600519".into()),
                    "Pushed".into(),
                )
            })
            .collect();
        let dir = test_history_dir_with_records(records).await;

        let result = HistoryQuery::new(dir.clone())
            .query(HistoryFilter {
                date: Some(today()),
                code: None,
                kind: None,
                limit: 0,
                order: HistoryOrder::Desc,
            })
            .await
            .unwrap();

        assert_eq!(result.len(), 101);
        assert_eq!(format_history_lines(&result).len(), 101);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn success_rate_excludes_denied_and_deduped_from_denominator() {
        let dir = test_dir("rate");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let today = today();
        let date_str = today.format("%Y-%m-%d").to_string();

        // Create 4 records: 1 Pushed, 1 Failed, 1 Denied, 1 Deduped
        // Use push.delivery event_type for PushRecord compatibility
        let outcomes = vec!["Pushed", "Failed", "Denied", "Deduped"];
        let ids = vec!["a", "b", "c", "d"];

        for (id, outcome) in ids.into_iter().zip(outcomes.into_iter()) {
            let env = super::super::envelope::EventEnvelope {
                id: id.to_string(),
                ts: Local::now(),
                trace_id: format!("trace-{}", id),
                source: "push_l4".to_string(),
                event_type: "push.delivery".to_string(),
                entity_key: Some("TEST_CODE_600519".to_string()),
                payload: serde_json::json!({
                    "kind": "Announcement",
                    "code": "TEST_CODE_600519",
                    "outcome": outcome,
                    "channel": "dry_run",
                    "rendered_len": 12,
                    "latency_ms": 37,
                }),
                version: 1,
                replay_of: None,
            };
            let json = serde_json::to_string(&env).unwrap();
            write_envelope_line(&dir, &date_str, &json).await;
        }

        let stats = HistoryQuery::new(dir.clone())
            .push_success_rate(Some("Announcement"), Window::Hours(24), None)
            .await
            .unwrap();

        assert_eq!(stats.total, 4);
        assert_eq!(stats.pushed, 1);
        assert_eq!(stats.failed, 1);
        assert!((stats.success_rate - 0.5).abs() < f64::EPSILON);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
