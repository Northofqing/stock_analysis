//! Durable bounds for one direct CLI chain notification invocation.
//!
//! An intent is synced before the legacy sender starts. If the process stops
//! before its weak observation is synced, the physical outcome stays unknown;
//! neither record is a channel receipt or authority to retry.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stock_analysis::monitor::push_job::WeakOutcomeKind;
use stock_analysis::notification::NotificationSendReport;
use stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis;

static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize, Deserialize)]
struct SendIntent {
    schema: String,
    notification_id: String,
    invocation_id: String,
    producer: String,
    business_date: String,
    report_sha256: String,
    report_bytes: usize,
    prepared_artifact_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WeakAttempt {
    channel: String,
    target_index: usize,
    outcome: String,
    initial_target_sha256: Option<String>,
    initial_body_sha256: Option<String>,
    initial_body_bytes: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SendObservation {
    schema: String,
    notification_id: String,
    invocation_id: String,
    report_sha256: String,
    prepared_artifact_sha256: String,
    send_id: String,
    attempts: Vec<WeakAttempt>,
}

pub(super) struct PendingSend {
    directory: PathBuf,
    notification_id: String,
    invocation_id: String,
    report_sha256: String,
    prepared_artifact_sha256: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn notification_id(invocation_id: &str, business_date: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"stock_analysis.cli_chain_notification.v1\0");
    for field in [invocation_id, "cli-chain", business_date] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn create_once_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .context("chain CLI audit file has no directory")?;
    fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create-once {}", path.display()))?;
    // A partial file is still a blocking intent after a crash.
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    OpenOptions::new()
        .read(true)
        .open(directory)?
        .sync_all()
        .with_context(|| format!("sync {}", directory.display()))
}

pub(super) fn begin(
    prepared: &PreparedChainAnalysis,
    report_input: &[u8],
    directory: &Path,
) -> Result<PendingSend> {
    let invocation_id = format!(
        "cli-chain-{}-{:x}-{:x}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id(),
        NEXT_INVOCATION.fetch_add(1, Ordering::Relaxed)
    );
    begin_with_invocation(prepared, report_input, directory, invocation_id)
}

fn begin_with_invocation(
    prepared: &PreparedChainAnalysis,
    report_input: &[u8],
    directory: &Path,
    invocation_id: String,
) -> Result<PendingSend> {
    anyhow::ensure!(
        prepared.report().as_bytes() == report_input,
        "chain CLI intent report differs from the prepared report"
    );
    let business_date = prepared.business_date().to_string();
    let notification_id = notification_id(&invocation_id, &business_date);
    let report_sha256 = digest(report_input);
    let prepared_artifact_sha256 = digest(&prepared.to_artifact_bytes()?);
    let intent = SendIntent {
        schema: "chain-cli-send-intent-v1".into(),
        notification_id: notification_id.clone(),
        invocation_id: invocation_id.clone(),
        producer: "cli-chain".into(),
        business_date,
        report_sha256: report_sha256.clone(),
        report_bytes: report_input.len(),
        prepared_artifact_sha256: prepared_artifact_sha256.clone(),
    };
    create_once_synced(
        &directory.join(format!("{notification_id}.intent.json")),
        &serde_json::to_vec(&intent)?,
    )?;
    Ok(PendingSend {
        directory: directory.to_owned(),
        notification_id,
        invocation_id,
        report_sha256,
        prepared_artifact_sha256,
    })
}

impl PendingSend {
    pub(super) fn observe(self, report_input: &[u8], sent: &NotificationSendReport) -> Result<()> {
        anyhow::ensure!(
            digest(report_input) == self.report_sha256,
            "chain CLI observed report differs from its durable intent"
        );
        let observation = SendObservation {
            schema: "chain-cli-send-weak-observation-v1".into(),
            notification_id: self.notification_id.clone(),
            invocation_id: self.invocation_id,
            report_sha256: self.report_sha256,
            prepared_artifact_sha256: self.prepared_artifact_sha256,
            send_id: sent.send_id().to_owned(),
            attempts: sent
                .attempts()
                .iter()
                .map(|attempt| WeakAttempt {
                    channel: attempt.channel().name().to_owned(),
                    target_index: attempt.target_index(),
                    outcome: match attempt.outcome() {
                        WeakOutcomeKind::Accepted => "weak_accepted",
                        WeakOutcomeKind::Rejected => "weak_rejected",
                        WeakOutcomeKind::Unknown => "unknown",
                    }
                    .into(),
                    initial_target_sha256: attempt
                        .request_entity()
                        .map(|entity| entity.target_sha256().to_owned()),
                    initial_body_sha256: attempt
                        .request_entity()
                        .map(|entity| entity.body_sha256().to_owned()),
                    initial_body_bytes: attempt.request_entity().map(|entity| entity.body_len()),
                })
                .collect(),
        };
        create_once_synced(
            &self
                .directory
                .join(format!("{}.observation.json", self.notification_id)),
            &serde_json::to_vec(&observation)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use chrono::NaiveDate;
    use stock_analysis::notification::NotificationSendReport;

    use super::{begin_with_invocation, SendIntent, SendObservation};
    use crate::app::chain_shadow_input::test_prepared;

    #[tokio::test]
    async fn create_once_intent_binds_prepared_bytes_and_weak_observation() {
        let prepared = test_prepared(
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap(),
            Rc::new(Cell::new(0)),
        )
        .await;
        let directory = tempfile::tempdir().unwrap();
        let bytes = prepared.report().as_bytes();
        let pending = begin_with_invocation(
            &prepared,
            bytes,
            directory.path(),
            "cli-chain-test-invocation".into(),
        )
        .unwrap();
        let key = pending.notification_id.clone();
        assert!(begin_with_invocation(
            &prepared,
            bytes,
            directory.path(),
            "cli-chain-test-invocation".into(),
        )
        .is_err());
        let report = NotificationSendReport::default();
        pending.observe(bytes, &report).unwrap();
        let intent_bytes =
            std::fs::read(directory.path().join(format!("{key}.intent.json"))).unwrap();
        let observation_bytes =
            std::fs::read(directory.path().join(format!("{key}.observation.json"))).unwrap();
        let intent: SendIntent = serde_json::from_slice(&intent_bytes).unwrap();
        let observation: SendObservation = serde_json::from_slice(&observation_bytes).unwrap();
        assert_eq!(intent.notification_id, observation.notification_id);
        assert_eq!(intent.invocation_id, observation.invocation_id);
        assert_eq!(intent.report_sha256, observation.report_sha256);
        assert_eq!(
            intent.prepared_artifact_sha256,
            observation.prepared_artifact_sha256
        );
        assert_eq!(observation.send_id, report.send_id());
        assert_eq!(intent.report_bytes, bytes.len());
        for persisted in [&intent_bytes, &observation_bytes] {
            assert!(!persisted.windows(bytes.len()).any(|window| window == bytes));
        }
    }

    #[tokio::test]
    async fn mismatched_report_cannot_create_intent_or_observation() {
        let prepared = test_prepared(
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap(),
            Rc::new(Cell::new(0)),
        )
        .await;
        let directory = tempfile::tempdir().unwrap();
        assert!(begin_with_invocation(
            &prepared,
            b"changed",
            directory.path(),
            "cli-chain-different".into(),
        )
        .is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        let pending = begin_with_invocation(
            &prepared,
            prepared.report().as_bytes(),
            directory.path(),
            "cli-chain-test".into(),
        )
        .unwrap();
        let key = pending.notification_id.clone();
        assert!(pending
            .observe(b"changed", &NotificationSendReport::default())
            .is_err());
        assert!(!directory
            .path()
            .join(format!("{key}.observation.json"))
            .exists());
    }
}
