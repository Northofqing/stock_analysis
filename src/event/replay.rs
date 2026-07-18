//! Registered business rules: BR-043.
//! Replay infrastructure — v17.3 Task 4.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::bus::{EventBus, PublishOutcome};
use super::envelope::EventEnvelope;

static REPLAY_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fresh_replay_id(original_id: &str) -> String {
    let sequence = REPLAY_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "replay-{}-{:x}-{:x}",
        original_id,
        std::process::id(),
        sequence
    )
}

#[derive(Error, Debug)]
pub enum ReplayError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no such date file: {0}")]
    NoSuchDate(String),
}

/// Explicit counters for one replay-file scan.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReplaySummary {
    pub attempted: usize,
    pub replayable: usize,
    pub published: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl ReplaySummary {
    pub fn has_failures(&self) -> bool {
        self.failed > 0
    }
}

/// Structured failures from the awaited replay publication boundary.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ReplayPublishError {
    #[error("event bus has no subscribers")]
    NoSubscribers,
    #[error("event bus rejected replay: {0}")]
    BusRejected(String),
    #[error("replay sink failed: {0}")]
    Sink(String),
    #[error("replay audit failed: {0}")]
    Audit(String),
    #[error("invalid replay envelope: {0}")]
    InvalidEnvelope(String),
    #[error("force replay is disabled by environment: {0}")]
    Environment(String),
}

#[async_trait]
pub trait ReplayPublisher: Send + Sync {
    async fn publish(&self, envelope: EventEnvelope) -> Result<(), ReplayPublishError>;
}

#[async_trait]
impl ReplayPublisher for EventBus {
    async fn publish(&self, envelope: EventEnvelope) -> Result<(), ReplayPublishError> {
        match EventBus::publish(self, envelope) {
            PublishOutcome::Published(_) => Ok(()),
            PublishOutcome::NoSubscribers => Err(ReplayPublishError::NoSubscribers),
            PublishOutcome::Rejected(reason) => {
                Err(ReplayPublishError::BusRejected(format!("{reason:?}")))
            }
        }
    }
}

pub struct ReplayRunner {
    base_dir: PathBuf,
    publisher: Arc<dyn ReplayPublisher>,
}

impl ReplayRunner {
    pub fn new<P>(base_dir: PathBuf, publisher: P) -> Self
    where
        P: ReplayPublisher + 'static,
    {
        Self {
            base_dir,
            publisher: Arc::new(publisher),
        }
    }

    /// Dry-run validates and counts. Force mode marks, paces, awaits, and
    /// counts a publication only after the configured publisher succeeds.
    pub async fn run(
        &self,
        date: NaiveDate,
        force: bool,
        rate_ms: u32,
    ) -> Result<ReplaySummary, ReplayError> {
        let date_str = date.format("%Y-%m-%d").to_string();
        let file_path = self.base_dir.join(format!("{date_str}.jsonl"));
        if !file_path.exists() {
            return Err(ReplayError::NoSuchDate(date_str));
        }

        let file = File::open(&file_path).await?;
        let mut reader = BufReader::new(file).lines();
        let mut summary = ReplaySummary::default();
        let mut attempted_publish = false;

        while let Some(line) = reader.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            summary.attempted += 1;

            let env: EventEnvelope = match serde_json::from_str(line) {
                Ok(envelope) => envelope,
                Err(error) => {
                    summary.failed += 1;
                    log::warn!("[replay] malformed row date={date_str}: {error}");
                    continue;
                }
            };
            if env.event_type != "push.source" {
                summary.skipped += 1;
                continue;
            }

            let Some(source_text) = env.payload.get("text").and_then(serde_json::Value::as_str)
            else {
                summary.failed += 1;
                log::warn!(
                    "[replay] invalid id={}: payload.text must be a string",
                    env.id
                );
                continue;
            };
            if source_text.trim().is_empty() {
                summary.failed += 1;
                log::warn!("[replay] invalid id={}: payload.text is blank", env.id);
                continue;
            }
            let source_text = source_text.to_owned();
            summary.replayable += 1;

            if !force {
                log::info!("[replay] mode=DRY-RUN id={}", env.id);
                continue;
            }

            let original_id = env.id.clone();
            let fresh_id = fresh_replay_id(&original_id);
            let mut cloned = env;
            cloned.id = fresh_id.clone();
            cloned.replay_of = Some(original_id.clone());
            cloned.ts = Local::now();
            cloned.payload["text"] =
                serde_json::Value::String(format!("[REPLAY {date_str}] {source_text}"));

            if attempted_publish && rate_ms > 0 {
                tokio::time::sleep(Duration::from_millis(u64::from(rate_ms))).await;
            }
            attempted_publish = true;

            match self.publisher.publish(cloned).await {
                Ok(()) => {
                    summary.published += 1;
                    log::info!("[replay] published id={fresh_id}");
                }
                Err(reason) => {
                    summary.failed += 1;
                    log::error!("[replay] failed replay_of={original_id}: {reason}");
                }
            }
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::push_record::ReplayablePushEvent;
    use std::sync::Mutex;

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct RecordingReplayPublisher {
        envelopes: Mutex<Vec<EventEnvelope>>,
        reject: bool,
    }

    #[async_trait]
    impl ReplayPublisher for RecordingReplayPublisher {
        async fn publish(&self, envelope: EventEnvelope) -> Result<(), ReplayPublishError> {
            if self.reject {
                return Err(ReplayPublishError::Sink("sink rejected replay".into()));
            }
            self.envelopes.lock().unwrap().push(envelope);
            Ok(())
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "replay-test-{name}-{}-{sequence}",
            std::process::id()
        ))
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    fn source_envelope(id: &str, text: serde_json::Value) -> EventEnvelope {
        EventEnvelope {
            id: id.into(),
            ts: Local::now(),
            trace_id: format!("trace-{id}"),
            source: "monitor".into(),
            event_type: "push.source".into(),
            entity_key: Some("TEST_CODE_600519".into()),
            payload: serde_json::json!({"kind": "Announcement", "text": text}),
            version: 1,
            replay_of: None,
        }
    }

    async fn write_envelopes(name: &str, envelopes: &[EventEnvelope]) -> PathBuf {
        let dir = test_dir(name);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let body = envelopes
            .iter()
            .map(|envelope| serde_json::to_string(envelope).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(
            dir.join(format!("{}.jsonl", today().format("%Y-%m-%d"))),
            body,
        )
        .await
        .unwrap();
        dir
    }

    async fn source_dir() -> PathBuf {
        write_envelopes(
            "source",
            &[source_envelope(
                "original",
                serde_json::json!("Test message"),
            )],
        )
        .await
    }

    #[tokio::test]
    async fn replay_defaults_to_dry_run_and_does_not_publish() {
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let summary = ReplayRunner::new(source_dir().await, bus)
            .run(today(), false, 0)
            .await
            .unwrap();
        assert_eq!(summary.replayable, 1);
        assert_eq!(summary.published, 0);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn force_replay_publishes_new_id_with_original_marker() {
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let summary = ReplayRunner::new(source_dir().await, bus)
            .run(today(), true, 0)
            .await
            .unwrap();
        let env = rx.recv().await.unwrap();
        assert_eq!(summary.published, 1);
        assert_ne!(env.id, "original");
        assert_eq!(env.replay_of.as_deref(), Some("original"));
        assert!(env.payload["text"]
            .as_str()
            .unwrap()
            .starts_with("[REPLAY "));
    }

    #[tokio::test]
    async fn force_replay_skips_delivery_events() {
        let event = ReplayablePushEvent::new(
            "Announcement".into(),
            Some("TEST_CODE_600519".into()),
            "Test".into(),
            "monitor".into(),
        );
        let source =
            EventEnvelope::from_event(&event, "source".into(), "trace-source".into(), Local::now())
                .unwrap();
        let mut delivery = source.clone();
        delivery.id = "delivery".into();
        delivery.event_type = "push.delivery".into();
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let summary =
            ReplayRunner::new(write_envelopes("delivery", &[source, delivery]).await, bus)
                .run(today(), true, 0)
                .await
                .unwrap();
        assert_eq!(summary.published, 1);
        assert_eq!(summary.skipped, 1);
        rx.recv().await.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn replay_returns_error_for_missing_date_file() {
        let dir = test_dir("missing");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let result = ReplayRunner::new(dir, EventBus::new_for_test(8))
            .run(NaiveDate::from_ymd_opt(2099, 12, 31).unwrap(), false, 0)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn force_replay_rejects_source_without_string_text() {
        for text in [serde_json::json!(42), serde_json::Value::Null] {
            let dir = write_envelopes("invalid", &[source_envelope("bad", text)]).await;
            let bus = EventBus::new_for_test(8);
            let mut rx = bus.subscribe();
            let summary = ReplayRunner::new(dir, bus)
                .run(today(), true, 0)
                .await
                .unwrap();
            assert_eq!(summary.failed, 1);
            assert_eq!(summary.published, 0);
            assert!(rx.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn force_replay_rejects_blank_source_text() {
        let dir = write_envelopes(
            "blank",
            &[source_envelope("blank", serde_json::json!("  "))],
        )
        .await;
        let summary = ReplayRunner::new(dir, RecordingReplayPublisher::default())
            .run(today(), true, 0)
            .await
            .unwrap();
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.replayable, 0);
    }

    #[tokio::test]
    async fn force_replay_applies_rate_between_publish_attempts() {
        let envelope = source_envelope("source", serde_json::json!("body"));
        let dir = write_envelopes("rate", &[envelope.clone(), envelope]).await;
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let replay = tokio::spawn(async move {
            ReplayRunner::new(dir, bus)
                .run(today(), true, 200)
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("first publication must be immediate")
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .is_err());
        tokio::time::timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("second publication must follow configured delay")
            .unwrap();
        assert_eq!(replay.await.unwrap().published, 2);
    }

    #[tokio::test]
    async fn repeated_force_replays_generate_distinct_envelope_ids() {
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        let runner = ReplayRunner::new(source_dir().await, bus);
        runner.run(today(), true, 0).await.unwrap();
        let first = rx.recv().await.unwrap();
        runner.run(today(), true, 0).await.unwrap();
        let second = rx.recv().await.unwrap();
        assert_ne!(first.id, second.id);
    }

    #[tokio::test]
    async fn force_replay_counts_no_subscribers_as_failure() {
        let summary = ReplayRunner::new(source_dir().await, EventBus::new_for_test(8))
            .run(today(), true, 0)
            .await
            .unwrap();
        assert_eq!(summary.published, 0);
        assert_eq!(summary.failed, 1);
    }

    #[tokio::test]
    async fn force_replay_counts_rejected_publish_as_failure() {
        let bus = EventBus::new_for_test(8);
        bus.shutdown();
        let summary = ReplayRunner::new(source_dir().await, bus)
            .run(today(), true, 0)
            .await
            .unwrap();
        assert_eq!(summary.published, 0);
        assert_eq!(summary.failed, 1);
    }

    #[tokio::test]
    async fn force_replay_awaits_custom_publisher_result() {
        let summary = ReplayRunner::new(source_dir().await, RecordingReplayPublisher::default())
            .run(today(), true, 0)
            .await
            .unwrap();
        assert_eq!(summary.published, 1);
        let publisher = RecordingReplayPublisher {
            reject: true,
            ..Default::default()
        };
        let summary = ReplayRunner::new(source_dir().await, publisher)
            .run(today(), true, 0)
            .await
            .unwrap();
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.published, 0);
    }
}
