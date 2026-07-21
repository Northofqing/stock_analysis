//! JSONL writer for event envelopes — v17.3 Task 2
//!
//! Persists `EventEnvelope`s to daily JSONL files under a base directory.
//! Files are named `YYYY-MM-DD.jsonl` and rotate daily. Old files are
//! removed by `cleanup_expired` based on `retention_days`.

use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::envelope::EventEnvelope;

// ========================================================================
// JsonlError
// ========================================================================

/// Errors that can occur during JSONL persistence operations.
#[derive(Error, Debug)]
pub enum JsonlError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("receiver error: {0}")]
    Receive(#[from] tokio::sync::broadcast::error::RecvError),
}

// ========================================================================
// JsonlWriter
// ========================================================================

/// Writes `EventEnvelope`s to daily JSONL files.
///
/// File layout: `{base_dir}/YYYY-MM-DD.jsonl` derived from `env.ts.date_naive()`.
/// The current day's file is never removed by cleanup.
pub struct JsonlWriter {
    base_dir: PathBuf,
    retention_days: u32,
}

impl JsonlWriter {
    /// Spawn a writer task that consumes envelopes from `receiver`.
    ///
    /// Directory creation and retention cleanup complete before this returns.
    /// The consumer task then runs until the receiver's sender is dropped or a
    /// fatal error occurs. The returned `JoinHandle` can be aborted to stop it.
    pub async fn spawn(
        receiver: broadcast::Receiver<EventEnvelope>,
        base_dir: PathBuf,
        retention_days: u32,
    ) -> Result<JoinHandle<()>, JsonlError> {
        let writer = Self {
            base_dir,
            retention_days,
        };
        fs::create_dir_all(&writer.base_dir).await?;
        Self::cleanup_expired(&writer.base_dir, writer.retention_days).await?;
        Ok(tokio::spawn(async move {
            if let Err(error) = writer.consume(receiver).await {
                log::error!("[jsonl_writer] fatal error: {error}");
            }
        }))
    }

    /// Remove JSONL files older than `retention_days` from `base_dir`.
    ///
    /// Never removes the current day's file. Files are identified by the
    /// `YYYY-MM-DD` prefix in the filename.
    pub async fn cleanup_expired(base_dir: &Path, retention_days: u32) -> Result<(), JsonlError> {
        let cutoff =
            chrono::Local::now().date_naive() - chrono::Duration::days(retention_days as i64);
        let mut entries = fs::read_dir(base_dir).await?;
        let mut to_delete = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Parse YYYY-MM-DD from filename stem
            if let Ok(file_date) = chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d") {
                if file_date < cutoff {
                    to_delete.push(path);
                }
            }
        }

        for path in to_delete {
            fs::remove_file(&path).await?;
            log::info!("[jsonl_writer] removed expired file: {}", path.display());
        }

        Ok(())
    }

    async fn consume(&self, mut rx: broadcast::Receiver<EventEnvelope>) -> Result<(), JsonlError> {
        loop {
            match rx.recv().await {
                Ok(env) => {
                    if env.replay_of.is_some() {
                        // Skip replay envelopes; real events must not be lost.
                        continue;
                    }
                    if let Err(e) = self.write_envelope(&env).await {
                        log::error!("[jsonl_writer] write error for {}: {}", env.id, e);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("[jsonl_writer] receiver lagged, skipped {} events", skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    log::info!("[jsonl_writer] receiver closed, shutting down normally");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn write_envelope(&self, env: &EventEnvelope) -> Result<(), JsonlError> {
        let date_str = env.ts.format("%Y-%m-%d").to_string();
        let file_path = self.base_dir.join(format!("{}.jsonl", date_str));

        tokio::fs::create_dir_all(&self.base_dir).await?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;

        let json = serde_json::to_string(env)?;
        file.write_all(json.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;

        Ok(())
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::bus::EventBus;
    use crate::event::envelope::{EventEnvelope, PushDeliveryEvent};
    use serde_json::Value;
    use std::sync::Arc;

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("jsonl-writer-test-{}-{}", name, std::process::id()))
    }

    fn today_string() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    fn test_bus(capacity: usize) -> (Arc<EventBus>, broadcast::Receiver<EventEnvelope>) {
        let bus = EventBus::new_for_test(capacity);
        let rx = bus.subscribe();
        (Arc::new(bus), rx)
    }

    fn test_live_envelope(id: &str) -> EventEnvelope {
        EventEnvelope::from_event(
            &PushDeliveryEvent::new(
                "announcement_v1".into(),
                Some("TEST_CODE_600519".into()),
                "Pushed".into(),
                "dry_run".into(),
                12,
                37,
            ),
            id.into(),
            "trace-1".into(),
            chrono::Local::now(),
        )
        .unwrap()
    }

    fn test_replay_envelope(id: &str, original_id: &str) -> EventEnvelope {
        let mut env = EventEnvelope::from_event(
            &PushDeliveryEvent::new(
                "announcement_v1".into(),
                None,
                "Pushed".into(),
                "dry_run".into(),
                0,
                0,
            ),
            id.into(),
            "trace-replay".into(),
            chrono::Local::now(),
        )
        .unwrap();
        env.replay_of = Some(original_id.to_string());
        env
    }

    async fn read_today_lines(dir: &Path) -> Vec<Value> {
        let path = dir.join(format!("{}.jsonl", today_string()));
        let text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        text.lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    async fn read_today_text(dir: &Path) -> String {
        let path = dir.join(format!("{}.jsonl", today_string()));
        tokio::fs::read_to_string(&path).await.unwrap_or_default()
    }

    async fn wait_until_file_contains(dir: &Path, substring: &str) {
        let path = dir.join(format!("{}.jsonl", today_string()));
        let max_attempts = 50;
        for _ in 0..max_attempts {
            if let Ok(text) = tokio::fs::read_to_string(&path).await {
                if text.contains(substring) {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!(
            "Timed out waiting for file {} to contain '{}'",
            path.display(),
            substring
        );
    }

    async fn create_dated_file(dir: &Path, date: &str) {
        let path = dir.join(format!("{}.jsonl", date));
        tokio::fs::create_dir_all(dir).await.unwrap();
        tokio::fs::write(&path, "").await.unwrap();
    }

    // ---------------------------------------------------------------------------
    // Unit tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn writer_appends_one_json_envelope_per_line() {
        let dir = test_dir("append");
        let (bus, rx) = test_bus(8);
        let handle = JsonlWriter::spawn(rx, dir.clone(), 7)
            .await
            .expect("initialize test JSONL writer");

        bus.publish(test_live_envelope("evt-1"));
        wait_until_file_contains(&dir, "evt-1").await;
        bus.shutdown();
        let _ = handle.await;

        let lines = read_today_lines(&dir).await;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["id"], "evt-1");

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn writer_skips_only_replay_envelopes() {
        let dir = test_dir("replay-filter");
        let (bus, rx) = test_bus(8);
        let handle = JsonlWriter::spawn(rx, dir.clone(), 7)
            .await
            .expect("initialize test JSONL writer");

        bus.publish(test_live_envelope("live"));
        bus.publish(test_replay_envelope("replay", "original"));
        wait_until_file_contains(&dir, "live").await;
        bus.shutdown();
        let _ = handle.await;

        let text = read_today_text(&dir).await;
        assert!(
            text.contains("\"id\":\"live\""),
            "file should contain 'live' id: {:?}",
            text
        );
        assert!(
            !text.contains("\"id\":\"replay\""),
            "file should NOT contain 'replay' id: {:?}",
            text
        );

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn cleanup_removes_only_files_older_than_retention() {
        let dir = test_dir("cleanup");
        create_dated_file(&dir, "2000-01-01").await;
        create_dated_file(&dir, &today_string()).await;

        JsonlWriter::cleanup_expired(&dir, 7).await.unwrap();

        assert!(
            !dir.join("2000-01-01.jsonl").exists(),
            "old file should be deleted"
        );
        assert!(
            dir.join(format!("{}.jsonl", today_string())).exists(),
            "today's file should exist"
        );

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
