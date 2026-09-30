//! Create-once local observations for CLI Custom webhook sends.
//!
//! This is a post-send audit of weak Custom channel results. A missing file does
//! not prove no request was sent; a present file is not a remote delivery receipt
//! or permission to retry. Target and entity hashes describe only the initial
//! built request; the production notification client rejects redirects.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CliProducer, CliReportSnapshot, CliSubject};
use crate::monitor::push_job::WeakOutcomeKind;
use crate::notification::{NotificationChannel, NotificationSendReport, NotificationService};

const DOMAIN: &[u8] = b"stock_analysis.cli_target_weak_receipt.v2\0";
static NEXT_PENDING: AtomicU64 = AtomicU64::new(0);

fn digest_fields(fields: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    format!("{:x}", digest.finalize())
}

pub(super) fn producer_name(producer: CliProducer, subject: &CliSubject) -> &'static str {
    match (producer, subject) {
        (CliProducer::Default, CliSubject::Stock { .. }) => "cli-single-default",
        (CliProducer::Schedule, CliSubject::Stock { .. }) => "cli-single-schedule",
        (CliProducer::Lhb, CliSubject::Stock { .. }) => "cli-single-lhb",
        (CliProducer::Direct, CliSubject::Stock { .. }) => "cli-single-direct",
        (CliProducer::Default, CliSubject::Summary) => "cli-summary-default",
        (CliProducer::Schedule, CliSubject::Summary) => "cli-summary-schedule",
        (CliProducer::Lhb, CliSubject::Summary) => "cli-summary-lhb",
        (CliProducer::Direct, CliSubject::Summary) => "cli-summary-direct",
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CliTargetSubject {
    Stock { code: String, input_ordinal: usize },
    Summary,
}

impl From<&CliSubject> for CliTargetSubject {
    fn from(subject: &CliSubject) -> Self {
        match subject {
            CliSubject::Stock {
                code,
                input_ordinal,
            } => Self::Stock {
                code: code.clone(),
                input_ordinal: *input_ordinal,
            },
            CliSubject::Summary => Self::Summary,
        }
    }
}

/// Keep the pre-send intent and post-send Custom target receipts on the same
/// notification identity without changing existing v2 receipt bytes.
pub(super) fn notification_id_for(snapshot: &CliReportSnapshot) -> Result<String> {
    let identity = snapshot.identity();
    let producer = producer_name(identity.invocation().producer(), identity.subject());
    let subject_json = serde_json::to_vec(&CliTargetSubject::from(identity.subject()))?;
    Ok(digest_fields(&[
        b"notification",
        identity.invocation().id().as_bytes(),
        producer.as_bytes(),
        &subject_json,
    ]))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CliTargetWeakReceipt {
    pub schema: String,
    pub receipt_id: String,
    pub notification_id: String,
    pub invocation_id: String,
    pub producer: String,
    pub subject: CliTargetSubject,
    pub send_id: String,
    pub target_index: usize,
    pub initial_target_sha256: String,
    pub report_sha256: String,
    pub initial_request_body_sha256: String,
    pub initial_request_body_len: usize,
    pub outcome: String,
}

#[derive(Debug)]
pub struct CliTargetReceiptBatch {
    pub path: PathBuf,
    pub receipts: Vec<CliTargetWeakReceipt>,
}

/// A CLI snapshot and the result of sending those exact bytes in one call.
/// Private fields prevent a caller from attaching a send to another identity.
pub struct CliBoundSend {
    snapshot: CliReportSnapshot,
    report: NotificationSendReport,
}

impl CliBoundSend {
    pub fn snapshot(&self) -> &CliReportSnapshot {
        &self.snapshot
    }

    pub fn report(&self) -> &NotificationSendReport {
        &self.report
    }

    pub fn has_custom_attempts(&self) -> bool {
        self.report
            .attempts()
            .iter()
            .any(|attempt| attempt.channel() == NotificationChannel::Custom)
    }

    pub fn into_report(self) -> NotificationSendReport {
        self.report
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CliAuditSendError {
    #[error("CLI send intent was not durable; no notification send started: {0:#}")]
    BeforeSend(anyhow::Error),
    #[error("CLI notification outcome is unknown; weak observation was not durable: {0:#}")]
    AfterSend(anyhow::Error),
}

/// A durable intent must exist before any legacy channel can be called. The
/// observation is only the local weak result; it never authorizes a retry.
pub async fn send_cli_report_audited(
    notifier: &NotificationService,
    snapshot: CliReportSnapshot,
    directory: &Path,
) -> std::result::Result<CliBoundSend, CliAuditSendError> {
    let pending = super::cli_send_audit::begin(&snapshot, directory)
        .map_err(CliAuditSendError::BeforeSend)?;
    let report = notifier.send_report(snapshot.report()).await;
    pending
        .observe(&report)
        .map_err(CliAuditSendError::AfterSend)?;
    Ok(CliBoundSend { snapshot, report })
}

#[cfg(test)]
pub async fn send_cli_report(
    notifier: &NotificationService,
    snapshot: CliReportSnapshot,
) -> CliBoundSend {
    let report = notifier.send_report(snapshot.report()).await;
    CliBoundSend { snapshot, report }
}

/// Persist the Custom attempts from one send as a create-once JSON batch.
/// The caller must inspect errors: persistence happens after the physical send.
pub fn persist_custom_target_receipts(
    sent: &CliBoundSend,
    directory: &Path,
) -> Result<CliTargetReceiptBatch> {
    let snapshot = sent.snapshot();
    let report = sent.report();
    let custom_attempts = report
        .attempts()
        .iter()
        .filter(|attempt| attempt.channel() == NotificationChannel::Custom)
        .collect::<Vec<_>>();
    if custom_attempts.is_empty() {
        bail!("cannot create target receipts for a send with no target attempts");
    }
    if custom_attempts
        .iter()
        .any(|attempt| attempt.request_entity().is_none())
    {
        bail!("target receipt scope requires a built Custom HTTP entity for every attempt");
    }
    if custom_attempts.iter().any(|attempt| {
        !attempt
            .request_entity()
            .expect("checked all entities")
            .matches_custom_content(snapshot.report())
    }) {
        bail!("Custom request entity does not match the supplied report snapshot");
    }

    let identity = snapshot.identity();
    let invocation_id = identity.invocation().id();
    let producer = producer_name(identity.invocation().producer(), identity.subject());
    let subject = CliTargetSubject::from(identity.subject());
    let notification_id = notification_id_for(snapshot)?;
    let report_sha256 = snapshot.report_bytes().sha256().as_str();

    let mut receipts = Vec::with_capacity(custom_attempts.len());
    for attempt in custom_attempts {
        if report.attempts().get(attempt.target_index()) != Some(attempt) {
            bail!("target attempts are not in their original send order");
        }
        let entity = attempt.request_entity().expect("checked all entities");
        let index = attempt.target_index().to_string();
        let body_len = entity.body_len().to_string();
        let outcome = outcome_name(attempt.outcome());
        let receipt_id = digest_fields(&[
            b"receipt",
            notification_id.as_bytes(),
            report.send_id().as_bytes(),
            index.as_bytes(),
            entity.target_sha256().as_bytes(),
            entity.body_sha256().as_bytes(),
            body_len.as_bytes(),
            report_sha256.as_bytes(),
            outcome.as_bytes(),
        ]);
        receipts.push(CliTargetWeakReceipt {
            schema: "cli-target-weak-receipt-v2".into(),
            receipt_id,
            notification_id: notification_id.clone(),
            invocation_id: invocation_id.to_owned(),
            producer: producer.into(),
            subject: subject.clone(),
            send_id: report.send_id().to_owned(),
            target_index: attempt.target_index(),
            initial_target_sha256: entity.target_sha256().into(),
            report_sha256: report_sha256.into(),
            initial_request_body_sha256: entity.body_sha256().into(),
            initial_request_body_len: entity.body_len(),
            outcome: outcome.into(),
        });
    }

    fs::create_dir_all(directory).with_context(|| format!("create {}", directory.display()))?;
    let batch_id = digest_fields(&[
        b"batch",
        notification_id.as_bytes(),
        report.send_id().as_bytes(),
    ]);
    let path = directory.join(format!("{batch_id}.json"));
    let bytes = serde_json::to_vec(&receipts)?;
    let (temporary, mut file) = loop {
        let sequence = NEXT_PENDING.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(
            "{batch_id}-{}-{sequence:x}.pending",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("create {}", temporary.display()))
            }
        }
    };
    let written = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::hard_link(&temporary, &path)?;
        fs::remove_file(&temporary)?;
        OpenOptions::new().read(true).open(directory)?.sync_all()?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.with_context(|| format!("persist local weak receipts to {}", path.display()))?;
    Ok(CliTargetReceiptBatch { path, receipts })
}

#[cfg(test)]
mod tests {
    use super::{persist_custom_target_receipts, send_cli_report, CliTargetWeakReceipt};
    use crate::notification::send_report_tests::{
        spawn_webhook_fixture, test_service, ScriptedResponse,
    };
    use crate::notification::{NotificationChannel, NotificationConfig};
    use crate::pipeline::{CliInvocationIdentity, CliProducer, CliReportSnapshot};
    use sha2::{Digest, Sha256};

    fn received_body(request: &[u8]) -> &[u8] {
        let start = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("HTTP headers")
            + 4;
        &request[start..]
    }

    #[tokio::test]
    async fn repeated_custom_target_has_distinct_persisted_weak_receipts_and_exact_body_hash() {
        let fixture = spawn_webhook_fixture(vec![
            ScriptedResponse::Http(r#"{"ok":true}"#),
            ScriptedResponse::Http(r#"{"ok":false}"#),
        ]);
        let target_url = fixture.url();
        let notifier = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![target_url.clone(), target_url.clone()],
                custom_webhook_bearer_token: Some("TEST_CODE_TOKEN_SECRET".into()),
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let invocation = CliInvocationIdentity::new(CliProducer::Lhb);
        let snapshot = CliReportSnapshot::new(
            invocation
                .stock_business("TEST_CODE", 3)
                .matching_notification(),
            "TEST_CODE_REPORT_SECRET".into(),
        );
        let sent = send_cli_report(&notifier, snapshot).await;
        let requests = fixture.finish();
        let directory = tempfile::tempdir().expect("temporary receipt store");
        let stored = persist_custom_target_receipts(&sent, directory.path())
            .expect("persist complete batch");
        let bytes = std::fs::read(&stored.path).expect("read persisted batch");
        let decoded: Vec<CliTargetWeakReceipt> =
            serde_json::from_slice(&bytes).expect("decode receipt batch");
        assert_eq!(decoded, stored.receipts);
        assert_eq!(decoded.len(), 2);
        assert_eq!(
            decoded[0].initial_target_sha256,
            decoded[1].initial_target_sha256
        );
        assert_ne!(decoded[0].receipt_id, decoded[1].receipt_id);
        assert_eq!(decoded[0].outcome, "weak_accepted");
        assert_eq!(decoded[1].outcome, "unknown");
        assert_eq!(decoded[0].producer, "cli-single-lhb");
        assert_eq!(
            decoded[0].initial_request_body_sha256,
            decoded[1].initial_request_body_sha256
        );
        for (receipt, request) in decoded.iter().zip(requests.iter()) {
            let body = received_body(request);
            let mut digest = Sha256::new();
            digest.update(b"stock_analysis.notification_http_entity.v1\0");
            digest.update(body);
            assert_eq!(
                receipt.initial_request_body_sha256,
                format!("{:x}", digest.finalize())
            );
            assert_eq!(receipt.initial_request_body_len, body.len());
        }
        let text = String::from_utf8(bytes).expect("UTF-8 JSON");
        for secret in [
            &target_url,
            "TEST_CODE_TOKEN_SECRET",
            "TEST_CODE_REPORT_SECRET",
        ] {
            assert!(!text.contains(secret));
        }
        assert!(persist_custom_target_receipts(&sent, directory.path()).is_err());
        assert_eq!(std::fs::read(&stored.path).unwrap(), text.as_bytes());
    }

    #[tokio::test]
    async fn same_body_from_another_cli_invocation_cannot_borrow_a_send() {
        let fixture = spawn_webhook_fixture(vec![
            ScriptedResponse::Http(r#"{"ok":true}"#),
            ScriptedResponse::Http(r#"{"ok":true}"#),
        ]);
        let notifier = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![fixture.url()],
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let sent_invocation = CliInvocationIdentity::new(CliProducer::Default);
        let sent =
            CliReportSnapshot::new(sent_invocation.summary_notification(), "same report".into());
        let other_invocation = CliInvocationIdentity::new(CliProducer::Schedule);
        let other = CliReportSnapshot::new(
            other_invocation.summary_notification(),
            "same report".into(),
        );
        let first = send_cli_report(&notifier, sent).await;
        let second = send_cli_report(&notifier, other).await;
        fixture.finish();
        let directory = tempfile::tempdir().unwrap();
        let first_batch = persist_custom_target_receipts(&first, directory.path()).unwrap();
        let second_batch = persist_custom_target_receipts(&second, directory.path()).unwrap();
        assert_eq!(first_batch.receipts[0].invocation_id, sent_invocation.id());
        assert_eq!(
            second_batch.receipts[0].invocation_id,
            other_invocation.id()
        );
        assert_ne!(
            first_batch.receipts[0].receipt_id,
            second_batch.receipts[0].receipt_id
        );
        assert_ne!(
            first_batch.receipts[0].send_id,
            second_batch.receipts[0].send_id
        );
    }

    #[tokio::test]
    async fn stale_pending_file_does_not_block_persisting_a_sent_batch() {
        let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":true}"#)]);
        let notifier = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![fixture.url()],
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let invocation = CliInvocationIdentity::new(CliProducer::Default);
        let snapshot = CliReportSnapshot::new(invocation.summary_notification(), "report".into());
        let sent = send_cli_report(&notifier, snapshot).await;
        fixture.finish();
        let directory = tempfile::tempdir().unwrap();
        let first = persist_custom_target_receipts(&sent, directory.path()).unwrap();
        std::fs::remove_file(&first.path).unwrap();
        let old_pending = first.path.with_extension("pending");
        std::fs::write(&old_pending, b"incomplete after crash").unwrap();
        let recovered = persist_custom_target_receipts(&sent, directory.path()).unwrap();
        assert_eq!(recovered.path, first.path);
        assert_eq!(recovered.receipts, first.receipts);
        assert_eq!(
            std::fs::read(&old_pending).unwrap(),
            b"incomplete after crash"
        );
    }

    #[test]
    fn no_target_report_cannot_masquerade_as_a_persisted_send() {
        let invocation = CliInvocationIdentity::new(CliProducer::Default);
        let snapshot = CliReportSnapshot::new(invocation.summary_notification(), "report".into());
        let directory = tempfile::tempdir().expect("temporary receipt store");
        let notifier = test_service(NotificationConfig::default(), vec![]);
        let sent = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(send_cli_report(&notifier, snapshot));
        assert!(persist_custom_target_receipts(&sent, directory.path()).is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
