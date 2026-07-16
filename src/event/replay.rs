//! Replay infrastructure — v17.3 Task 4
//!
//! Reads persisted JSONL event files for a given date and replays replayable
//! `push.source` envelopes through the `EventBus`. Supports dry-run and
//! force modes.

use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use thiserror::Error;
use tokio::fs::{self, File};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::bus::{EventBus, PublishOutcome};
use super::envelope::EventEnvelope;

// ========================================================================
// ReplayError
// ========================================================================

/// Errors that can occur during replay.
#[derive(Error, Debug)]
pub enum ReplayError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("no such date file: {0}")]
    NoSuchDate(String),
}

// ========================================================================
// ReplayRunner
// ========================================================================

/// Replays persisted `push.source` envelopes from a date-indexed JSONL file.
pub struct ReplayRunner {
    base_dir: PathBuf,
    bus: EventBus,
}

impl ReplayRunner {
    /// Create a new ReplayRunner.
    pub fn new(base_dir: PathBuf, bus: EventBus) -> Self {
        Self { base_dir, bus }
    }

    /// Replay all replayable `push.source` envelopes for the given date.
    ///
    /// In dry-run mode (`force = false`), parses and validates envelopes,
    /// logs `mode=DRY-RUN`, and does NOT publish anything.
    ///
    /// In force mode (`force = true`), parses valid envelopes, clones each
    /// with a fresh deterministic-in-process ID, sets `replay_of` to the
    /// original ID, prefixes the text with `[REPLAY YYYY-MM-DD] `, and
    /// publishes only `push.source` envelopes through the bus.
    /// `push.delivery` and `push.delivery.audit` envelopes are logged as
    /// `not_replayable` and are never re-sent.
    ///
    /// Returns the number of replayable envelopes processed (published in
    /// force mode, counted in dry-run mode).
    pub async fn run(
        &self,
        date: NaiveDate,
        force: bool,
        _rate_ms: u32,
    ) -> Result<usize, ReplayError> {
        let date_str = date.format("%Y-%m-%d").to_string();
        let file_path = self.base_dir.join(format!("{}.jsonl", date_str));

        if !file_path.exists() {
            return Err(ReplayError::NoSuchDate(date_str));
        }

        let file = File::open(&file_path).await?;
        let mut reader = BufReader::new(file).lines();

        let mut count = 0usize;

        while let Some(line) = reader.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let env: EventEnvelope = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("[replay] skipping malformed JSON line in {}: {}", date_str, e);
                    continue;
                }
            };

            // Only replayable events are push.source
            if env.event_type != "push.source" {
                log::debug!("[replay] not_replayable event_type={}", env.event_type);
                continue;
            }

            count += 1;

            if !force {
                log::info!("[replay] mode=DRY-RUN id={} event_type={}", env.id, env.event_type);
                continue;
            }

            // Force mode: clone with fresh ID and replay_of marker
            let original_id = env.id.clone();
            let fresh_id = format!("replay-{}-{}", original_id, count);

            let mut cloned = env.clone();
            cloned.id = fresh_id;
            cloned.replay_of = Some(original_id.clone());
            cloned.ts = Local::now();

            // Prefix text with [REPLAY YYYY-MM-DD]
            if let Some(text) = cloned.payload.get_mut("text") {
                if let Some(s) = text.as_str() {
                    let prefixed = format!("[REPLAY {}] {}", date_str, s);
                    *text = serde_json::Value::String(prefixed);
                }
            }

            let outcome = self.bus.publish(cloned);
            match outcome {
                PublishOutcome::Published(n) => {
                    log::info!(
                        "[replay] published id=replay-{}-{} via {} subscribers",
                        original_id, count, n
                    );
                }
                PublishOutcome::NoSubscribers => {
                    log::warn!("[replay] no subscribers for replay of {}", original_id);
                }
                PublishOutcome::Rejected(r) => {
                    log::error!("[replay] rejected replay of {}: {:?}", original_id, r);
                }
            }
        }

        Ok(count)
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::bus::EventBus;
    use crate::event::push_record::ReplayablePushEvent;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "replay-test-{}-{}",
            name,
            std::process::id()
        ))
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    async fn test_replay_dir_with_source_event() -> PathBuf {
        let dir = test_dir("source");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let event = ReplayablePushEvent::new(
            "Announcement".into(),
            Some("600519".into()),
            "Test message".into(),
            "monitor".into(),
        );
        let env = EventEnvelope::from_event(
            &event,
            "original".into(),
            "trace-1".into(),
            Local::now(),
        )
        .unwrap();

        let date_str = today().format("%Y-%m-%d").to_string();
        let path = dir.join(format!("{}.jsonl", date_str));
        let json = serde_json::to_string(&env).unwrap();
        tokio::fs::write(&path, json).await.unwrap();

        dir
    }

    #[tokio::test]
    async fn replay_defaults_to_dry_run_and_does_not_publish() {
        let dir = test_replay_dir_with_source_event().await;
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let count =
            ReplayRunner::new(dir, bus).run(today(), false, 0).await.unwrap();
        assert_eq!(count, 1);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn force_replay_publishes_new_id_with_original_marker() {
        let dir = test_replay_dir_with_source_event().await;
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        ReplayRunner::new(dir, bus)
            .run(today(), true, 0)
            .await
            .unwrap();
        let env = rx.recv().await.unwrap();
        assert_ne!(env.id, "original");
        assert_eq!(env.replay_of.as_deref(), Some("original"));
    }

    #[tokio::test]
    async fn force_replay_skips_delivery_events() {
        let dir = test_dir("delivery");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let today = today();
        let date_str = today.format("%Y-%m-%d").to_string();

        // Write a push.source and a push.delivery to the same file
        let source_event = ReplayablePushEvent::new(
            "Announcement".into(),
            Some("600519".into()),
            "Test".into(),
            "monitor".into(),
        );
        let source_env = EventEnvelope::from_event(
            &source_event,
            "source-id".into(),
            "trace-source".into(),
            Local::now(),
        )
        .unwrap();

        let delivery_env = crate::event::envelope::EventEnvelope {
            id: "delivery-id".into(),
            ts: Local::now(),
            trace_id: "trace-delivery".into(),
            source: "push_l4".into(),
            event_type: "push.delivery".into(),
            entity_key: Some("600519".into()),
            payload: serde_json::json!({
                "kind": "Announcement",
                "code": "600519",
                "outcome": "Pushed",
                "channel": "dry_run",
                "rendered_len": 12,
                "latency_ms": 37,
            }),
            version: 1,
            replay_of: None,
        };

        let path = dir.join(format!("{}.jsonl", date_str));
        tokio::fs::write(
            &path,
            serde_json::to_string(&source_env).unwrap() + "\n" + &serde_json::to_string(&delivery_env).unwrap(),
        )
        .await
        .unwrap();

        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let count = ReplayRunner::new(dir, bus)
            .run(today, true, 0)
            .await
            .unwrap();

        // Only the push.source should be counted
        assert_eq!(count, 1);
        // And only one message should be received (the source, not the delivery)
        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, "replay-source-id-1");
    }

    #[tokio::test]
    async fn replay_returns_error_for_missing_date_file() {
        let dir = test_dir("missing");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let bus = EventBus::new_for_test(8);
        let result = ReplayRunner::new(dir, bus)
            .run(NaiveDate::from_ymd_opt(2099, 12, 31).unwrap(), false, 0)
            .await;
        assert!(result.is_err());
    }
}
