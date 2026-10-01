//! Durable bounds for one direct CLI chain notification invocation.
//!
//! An intent is synced before the legacy sender starts. If the process stops
//! before its weak observation is synced, the physical outcome stays unknown;
//! neither record is a channel receipt or authority to retry.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stock_analysis::monitor::push_job::WeakOutcomeKind;
use stock_analysis::notification::{NotificationChannel, NotificationSendReport};
use stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis;

static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WeakAttempt {
    channel: String,
    target_index: usize,
    outcome: String,
    initial_target_sha256: Option<String>,
    initial_body_sha256: Option<String>,
    initial_body_bytes: Option<usize>,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Local disk evidence only; neither variant is a remote receipt or a retry permit.
#[derive(Debug, Eq, PartialEq)]
enum AuditReadback {
    IntentOnly(SendIntent),
    WeakObserved(SendIntent, SendObservation),
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_channel(channel: &str) -> bool {
    [
        NotificationChannel::Wechat,
        NotificationChannel::Feishu,
        NotificationChannel::Telegram,
        NotificationChannel::Email,
        NotificationChannel::Pushover,
        NotificationChannel::Custom,
        NotificationChannel::ServerChan,
        NotificationChannel::DingTalk,
        NotificationChannel::Slack,
        NotificationChannel::Discord,
    ]
    .iter()
    .any(|known| known.name() == channel)
}

/// Read one persisted invocation after restart. A missing observation means
/// unknown physical outcome, including a crash before the sender returned.
fn readback(directory: &Path, key: &str) -> Result<AuditReadback> {
    ensure!(is_sha256(key), "invalid chain CLI notification id");
    let intent_path = directory.join(format!("{key}.intent.json"));
    let intent: SendIntent = serde_json::from_slice(
        &fs::read(&intent_path).with_context(|| format!("read {}", intent_path.display()))?,
    )
    .with_context(|| format!("decode {}", intent_path.display()))?;
    ensure!(
        intent.schema == "chain-cli-send-intent-v1",
        "unknown chain CLI intent schema"
    );
    ensure!(
        intent.notification_id == key,
        "chain CLI intent file identity mismatch"
    );
    ensure!(
        intent.producer == "cli-chain",
        "unexpected chain CLI producer"
    );
    ensure!(
        intent.invocation_id.starts_with("cli-chain-")
            && intent.invocation_id.len() > "cli-chain-".len(),
        "invalid chain CLI invocation id"
    );
    ensure!(
        chrono::NaiveDate::parse_from_str(&intent.business_date, "%Y-%m-%d")?.to_string()
            == intent.business_date,
        "invalid chain CLI business date"
    );
    ensure!(
        notification_id(&intent.invocation_id, &intent.business_date) == key,
        "chain CLI notification id does not bind invocation and business date"
    );
    ensure!(
        is_sha256(&intent.report_sha256) && is_sha256(&intent.prepared_artifact_sha256),
        "invalid chain CLI report or artifact digest"
    );
    ensure!(intent.report_bytes > 0, "empty chain CLI report");

    let observation_path = directory.join(format!("{key}.observation.json"));
    let bytes = match fs::read(&observation_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AuditReadback::IntentOnly(intent));
        }
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", observation_path.display()));
        }
    };
    let observation: SendObservation = serde_json::from_slice(&bytes)
        .with_context(|| format!("decode {}", observation_path.display()))?;
    ensure!(
        observation.schema == "chain-cli-send-weak-observation-v1",
        "unknown chain CLI observation schema"
    );
    ensure!(
        observation.notification_id == intent.notification_id
            && observation.invocation_id == intent.invocation_id
            && observation.report_sha256 == intent.report_sha256
            && observation.prepared_artifact_sha256 == intent.prepared_artifact_sha256,
        "chain CLI observation does not match its intent"
    );
    ensure!(
        observation.send_id.starts_with("notification-send-")
            && observation.send_id.len() > "notification-send-".len(),
        "invalid chain CLI send id"
    );
    for (index, attempt) in observation.attempts.iter().enumerate() {
        ensure!(
            attempt.target_index == index,
            "chain CLI target order mismatch"
        );
        ensure!(
            valid_channel(&attempt.channel),
            "unknown chain CLI target channel"
        );
        ensure!(
            matches!(
                attempt.outcome.as_str(),
                "weak_accepted" | "weak_rejected" | "unknown"
            ),
            "unknown chain CLI weak outcome"
        );
        match (
            &attempt.initial_target_sha256,
            &attempt.initial_body_sha256,
            attempt.initial_body_bytes,
        ) {
            (None, None, None) => {}
            (Some(target), Some(body), Some(_)) if is_sha256(target) && is_sha256(body) => {}
            _ => bail!("incomplete chain CLI initial request evidence"),
        }
    }
    Ok(AuditReadback::WeakObserved(intent, observation))
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
    ensure!(
        readback(directory, &notification_id)? == AuditReadback::IntentOnly(intent),
        "chain CLI intent readback differs from the prepared send"
    );
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
        let AuditReadback::IntentOnly(intent) = readback(&self.directory, &self.notification_id)?
        else {
            bail!("chain CLI observation already exists; physical outcome requires review");
        };
        ensure!(
            intent.invocation_id == self.invocation_id
                && intent.report_sha256 == self.report_sha256
                && intent.prepared_artifact_sha256 == self.prepared_artifact_sha256,
            "chain CLI intent changed after physical send"
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
        )?;
        ensure!(
            readback(&self.directory, &self.notification_id)?
                == AuditReadback::WeakObserved(intent, observation),
            "chain CLI weak observation readback differs from the sent report"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use chrono::NaiveDate;
    use stock_analysis::notification::NotificationSendReport;

    use super::{begin_with_invocation, readback, AuditReadback, SendIntent, SendObservation};
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
        assert!(matches!(
            readback(directory.path(), &key).unwrap(),
            AuditReadback::IntentOnly(_)
        ));
        assert!(begin_with_invocation(
            &prepared,
            bytes,
            directory.path(),
            "cli-chain-test-invocation".into(),
        )
        .is_err());
        let report = NotificationSendReport::default();
        pending.observe(bytes, &report).unwrap();
        assert!(matches!(
            readback(directory.path(), &key).unwrap(),
            AuditReadback::WeakObserved(_, _)
        ));
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

    #[tokio::test]
    async fn readback_rejects_changed_identity_and_invalid_target_evidence() {
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
            "cli-chain-reopen-test".into(),
        )
        .unwrap();
        let key = pending.notification_id.clone();
        let intent_path = directory.path().join(format!("{key}.intent.json"));
        let observation_path = directory.path().join(format!("{key}.observation.json"));
        let original_intent = std::fs::read(&intent_path).unwrap();
        pending
            .observe(bytes, &NotificationSendReport::default())
            .unwrap();
        let original_observation = std::fs::read(&observation_path).unwrap();

        let mut changed: serde_json::Value = serde_json::from_slice(&original_observation).unwrap();
        changed["report_sha256"] = serde_json::json!("0".repeat(64));
        std::fs::write(&observation_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(readback(directory.path(), &key).is_err());

        changed = serde_json::from_slice(&original_observation).unwrap();
        changed["attempts"] = serde_json::json!([{
            "channel": "自定义Webhook",
            "target_index": 0,
            "outcome": "unknown",
            "initial_target_sha256": "0".repeat(64),
            "initial_body_sha256": "1".repeat(64),
            "initial_body_bytes": 1
        }]);
        std::fs::write(&observation_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(matches!(
            readback(directory.path(), &key).unwrap(),
            AuditReadback::WeakObserved(_, _)
        ));

        changed["attempts"][0]["target_index"] = serde_json::json!(1);
        std::fs::write(&observation_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(readback(directory.path(), &key).is_err());

        changed["attempts"][0]["target_index"] = serde_json::json!(0);
        changed["attempts"][0]["initial_body_sha256"] = serde_json::Value::Null;
        std::fs::write(&observation_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(readback(directory.path(), &key).is_err());

        std::fs::write(&observation_path, &original_observation).unwrap();
        let mut changed: serde_json::Value = serde_json::from_slice(&original_intent).unwrap();
        changed["business_date"] = serde_json::json!("2026-09-30");
        std::fs::write(&intent_path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(readback(directory.path(), &key).is_err());
    }
}
