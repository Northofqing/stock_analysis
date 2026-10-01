//! Durable, redacted bounds around one legacy CLI notification invocation.
//!
//! An intent is synced before `send_report` can run. A missing observation after
//! that intent is an unknown physical outcome, never permission to resend. The
//! post-send observation records only local weak results, not channel authority.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::cli_target_receipt::{notification_id_for, producer_name, CliTargetSubject};
use super::CliReportSnapshot;
use crate::monitor::push_job::WeakOutcomeKind;
use crate::notification::{NotificationChannel, NotificationSendReport};

fn channel_name(channel: NotificationChannel) -> &'static str {
    match channel {
        NotificationChannel::Wechat => "wechat",
        NotificationChannel::Feishu => "feishu",
        NotificationChannel::Email => "email",
        NotificationChannel::ServerChan => "server_chan",
        NotificationChannel::DingTalk => "ding_talk",
        NotificationChannel::Telegram => "telegram",
        NotificationChannel::Slack => "slack",
        NotificationChannel::Discord => "discord",
        NotificationChannel::Pushover => "pushover",
        NotificationChannel::Custom => "custom",
    }
}

fn outcome_name(outcome: WeakOutcomeKind) -> &'static str {
    match outcome {
        WeakOutcomeKind::Accepted => "weak_accepted",
        WeakOutcomeKind::Rejected => "weak_rejected",
        WeakOutcomeKind::Unknown => "unknown",
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SendIntent {
    schema: String,
    notification_id: String,
    invocation_id: String,
    producer: String,
    subject: CliTargetSubject,
    report_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SendAttempt {
    channel: String,
    target_index: usize,
    outcome: String,
    // Optional hashes describe the initial built HTTP entity, not a remote
    // acceptance receipt or the bytes of a redirected request.
    initial_target_sha256: Option<String>,
    initial_request_body_sha256: Option<String>,
    initial_request_body_len: Option<usize>,
    response_target_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SendObservation {
    schema: String,
    notification_id: String,
    report_sha256: String,
    send_id: String,
    attempts: Vec<SendAttempt>,
}

pub(super) struct PendingSend {
    directory: PathBuf,
    notification_id: String,
    report_sha256: String,
}

fn create_once_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("CLI audit file has no directory")?;
    fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create-once {}", path.display()))?;
    // A partial file after a crash remains a blocking intent. Removing it
    // could authorize another send after an uncertain first attempt.
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

pub(super) fn begin(snapshot: &CliReportSnapshot, directory: &Path) -> Result<PendingSend> {
    let identity = snapshot.identity();
    let subject = CliTargetSubject::from(identity.subject());
    let producer = producer_name(identity.invocation().producer(), identity.subject());
    let notification_id = notification_id_for(snapshot)?;
    let report_sha256 = snapshot.report_bytes().sha256().as_str().to_owned();
    let intent = SendIntent {
        schema: "cli-send-intent-v1".into(),
        notification_id: notification_id.clone(),
        invocation_id: identity.invocation().id().to_owned(),
        producer: producer.into(),
        subject,
        report_sha256: report_sha256.clone(),
    };
    create_once_synced(
        &directory.join(format!("{notification_id}.intent.json")),
        &serde_json::to_vec(&intent)?,
    )?;
    Ok(PendingSend {
        directory: directory.to_owned(),
        notification_id,
        report_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::{SendIntent, SendObservation};
    use crate::notification::send_report_tests::{
        spawn_webhook_fixture, test_service, ScriptedResponse,
    };
    use crate::notification::{NotificationChannel, NotificationConfig};
    use crate::pipeline::cli_target_receipt::{
        notification_id_for, persist_custom_target_receipts, send_cli_report_audited,
        CliAuditSendError,
    };
    use crate::pipeline::{CliInvocationIdentity, CliProducer, CliReportSnapshot};
    use sha2::{Digest, Sha256};

    #[tokio::test]
    async fn audited_send_persists_redacted_intent_and_weak_observation() {
        let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":true}"#)]);
        let url = fixture.url();
        let notifier = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![url.clone()],
                custom_webhook_bearer_token: Some("TEST_CODE_TOKEN_SECRET".into()),
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let invocation = CliInvocationIdentity::new(CliProducer::Schedule);
        let snapshot = CliReportSnapshot::new(
            invocation.summary_notification(),
            "TEST_CODE_REPORT_SECRET".into(),
        );
        let directory = tempfile::tempdir().unwrap();
        let key = notification_id_for(&snapshot).unwrap();
        let sent = send_cli_report_audited(&notifier, snapshot.clone(), directory.path())
            .await
            .unwrap();
        let intent_bytes =
            std::fs::read(directory.path().join(format!("{key}.intent.json"))).unwrap();
        let observation_bytes =
            std::fs::read(directory.path().join(format!("{key}.observation.json"))).unwrap();
        let intent: SendIntent = serde_json::from_slice(&intent_bytes).unwrap();
        let observation: SendObservation = serde_json::from_slice(&observation_bytes).unwrap();
        assert_eq!(observation.schema, "cli-send-weak-observation-v2");
        assert_eq!(intent.invocation_id, invocation.id());
        assert_eq!(
            intent.report_sha256,
            snapshot.report_bytes().sha256().as_str()
        );
        assert_eq!(observation.notification_id, intent.notification_id);
        assert_eq!(observation.send_id, sent.report().send_id());
        assert_eq!(observation.attempts.len(), 1);
        assert_eq!(observation.attempts[0].channel, "custom");
        assert_eq!(observation.attempts[0].outcome, "weak_accepted");
        let requests = fixture.finish();
        assert_eq!(requests.len(), 1);
        let body_start = requests[0]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let request_body = &requests[0][body_start..];
        let mut digest = Sha256::new();
        digest.update(b"stock_analysis.notification_http_entity.v1\0");
        digest.update(request_body);
        let expected_body_hash = format!("{:x}", digest.finalize());
        assert_eq!(
            observation.attempts[0]
                .initial_request_body_sha256
                .as_deref(),
            Some(expected_body_hash.as_str())
        );
        assert_eq!(
            observation.attempts[0].initial_request_body_len,
            Some(request_body.len())
        );
        assert!(observation.attempts[0].initial_target_sha256.is_some());
        assert_eq!(
            observation.attempts[0].response_target_sha256.as_deref(),
            observation.attempts[0].initial_target_sha256.as_deref()
        );
        let targets =
            persist_custom_target_receipts(&sent, &directory.path().join("targets")).unwrap();
        assert_eq!(targets.receipts[0].notification_id, intent.notification_id);
        assert_eq!(targets.receipts[0].send_id, observation.send_id);
        for bytes in [&intent_bytes, &observation_bytes] {
            let text = std::str::from_utf8(bytes).unwrap();
            for secret in [&url, "TEST_CODE_TOKEN_SECRET", "TEST_CODE_REPORT_SECRET"] {
                assert!(!text.contains(secret));
            }
        }
        assert!(matches!(
            send_cli_report_audited(&notifier, snapshot, directory.path()).await,
            Err(CliAuditSendError::BeforeSend(_))
        ));
    }

    #[tokio::test]
    async fn failed_intent_prevents_any_channel_call() {
        let fixture = spawn_webhook_fixture(vec![]);
        let notifier = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![fixture.url()],
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("file");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let invocation = CliInvocationIdentity::new(CliProducer::Direct);
        let snapshot = CliReportSnapshot::new(invocation.summary_notification(), "report".into());
        assert!(matches!(
            send_cli_report_audited(&notifier, snapshot, &blocked.join("child")).await,
            Err(CliAuditSendError::BeforeSend(_))
        ));
        assert!(fixture.finish().is_empty());
    }

    #[tokio::test]
    async fn failed_observation_keeps_sent_invocation_uncertain() {
        let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":true}"#)]);
        let notifier = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![fixture.url()],
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let invocation = CliInvocationIdentity::new(CliProducer::Direct);
        let snapshot = CliReportSnapshot::new(invocation.summary_notification(), "report".into());
        let directory = tempfile::tempdir().unwrap();
        let key = notification_id_for(&snapshot).unwrap();
        let observation_path = directory.path().join(format!("{key}.observation.json"));
        std::fs::write(&observation_path, b"TEST_CODE_collision").unwrap();
        assert!(matches!(
            send_cli_report_audited(&notifier, snapshot, directory.path()).await,
            Err(CliAuditSendError::AfterSend(_))
        ));
        assert_eq!(fixture.finish().len(), 1);
        assert!(directory.path().join(format!("{key}.intent.json")).exists());
        assert_eq!(
            std::fs::read(observation_path).unwrap(),
            b"TEST_CODE_collision"
        );
    }
}

impl PendingSend {
    pub(super) fn observe(self, report: &NotificationSendReport) -> Result<()> {
        let observation = SendObservation {
            schema: "cli-send-weak-observation-v2".into(),
            notification_id: self.notification_id.clone(),
            report_sha256: self.report_sha256,
            send_id: report.send_id().to_owned(),
            attempts: report
                .attempts()
                .iter()
                .map(|attempt| SendAttempt {
                    channel: channel_name(attempt.channel()).into(),
                    target_index: attempt.target_index(),
                    outcome: outcome_name(attempt.outcome()).into(),
                    initial_target_sha256: attempt
                        .request_entity()
                        .map(|entity| entity.target_sha256().to_owned()),
                    initial_request_body_sha256: attempt
                        .request_entity()
                        .map(|entity| entity.body_sha256().to_owned()),
                    initial_request_body_len: attempt
                        .request_entity()
                        .map(|entity| entity.body_len()),
                    response_target_sha256: attempt
                        .request_entity()
                        .and_then(|entity| entity.response_url_sha256().map(str::to_owned)),
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
