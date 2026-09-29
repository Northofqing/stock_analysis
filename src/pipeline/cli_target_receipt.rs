//! Create-once local observations for CLI Custom webhook sends.
//!
//! This is a post-send audit of weak channel results. A missing file does not
//! prove no request was sent; a present file is not a remote delivery receipt
//! or permission to retry. Other channel types are deliberately unsupported.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CliProducer, CliReportSnapshot, CliSubject};
use crate::monitor::push_job::WeakOutcomeKind;
use crate::notification::{NotificationChannel, NotificationSendReport};

const DOMAIN: &[u8] = b"stock_analysis.cli_target_weak_receipt.v1\0";

fn digest_fields(fields: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    format!("{:x}", digest.finalize())
}

fn producer_name(producer: CliProducer, subject: &CliSubject) -> &'static str {
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
    pub target_sha256: String,
    pub report_sha256: String,
    pub request_body_sha256: String,
    pub request_body_len: usize,
    pub outcome: String,
}

#[derive(Debug)]
pub struct CliTargetReceiptBatch {
    pub path: PathBuf,
    pub receipts: Vec<CliTargetWeakReceipt>,
}

/// Persist one complete Custom-only send report as a create-once JSON batch.
/// The caller must inspect errors: persistence happens after the physical send.
pub fn persist_custom_target_receipts(
    snapshot: &CliReportSnapshot,
    report: &NotificationSendReport,
    directory: &Path,
) -> Result<CliTargetReceiptBatch> {
    if report.attempts().is_empty() {
        bail!("cannot create target receipts for a send with no target attempts");
    }
    if report.attempts().iter().any(|attempt| {
        attempt.channel() != NotificationChannel::Custom || attempt.request_entity().is_none()
    }) {
        bail!("target receipt scope requires a built Custom HTTP entity for every attempt");
    }
    if report.attempts().iter().any(|attempt| {
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
    let subject_json = serde_json::to_vec(&subject)?;
    let notification_id = digest_fields(&[
        b"notification",
        invocation_id.as_bytes(),
        producer.as_bytes(),
        &subject_json,
    ]);
    let report_sha256 = snapshot.report_bytes().sha256().as_str();

    let mut receipts = Vec::with_capacity(report.attempts().len());
    for (ordinal, attempt) in report.attempts().iter().enumerate() {
        if attempt.target_index() != ordinal {
            bail!("target attempts are not in their original send order");
        }
        let entity = attempt.request_entity().expect("checked all entities");
        let index = ordinal.to_string();
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
            schema: "cli-target-weak-receipt-v1".into(),
            receipt_id,
            notification_id: notification_id.clone(),
            invocation_id: invocation_id.to_owned(),
            producer: producer.into(),
            subject: subject.clone(),
            send_id: report.send_id().to_owned(),
            target_index: ordinal,
            target_sha256: entity.target_sha256().into(),
            report_sha256: report_sha256.into(),
            request_body_sha256: entity.body_sha256().into(),
            request_body_len: entity.body_len(),
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
    let temporary = directory.join(format!("{batch_id}.pending"));
    let bytes = serde_json::to_vec(&receipts)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
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
    use super::{persist_custom_target_receipts, CliTargetWeakReceipt};
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
        let report = notifier.send_report(snapshot.report()).await;
        let requests = fixture.finish();
        let directory = tempfile::tempdir().expect("temporary receipt store");
        let wrong_snapshot = CliReportSnapshot::new(
            snapshot.identity().clone(),
            "TEST_CODE_DIFFERENT_REPORT_SECRET".into(),
        );
        assert!(
            persist_custom_target_receipts(&wrong_snapshot, &report, directory.path()).is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        let stored = persist_custom_target_receipts(&snapshot, &report, directory.path())
            .expect("persist complete batch");
        let bytes = std::fs::read(&stored.path).expect("read persisted batch");
        let decoded: Vec<CliTargetWeakReceipt> =
            serde_json::from_slice(&bytes).expect("decode receipt batch");
        assert_eq!(decoded, stored.receipts);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].target_sha256, decoded[1].target_sha256);
        assert_ne!(decoded[0].receipt_id, decoded[1].receipt_id);
        assert_eq!(decoded[0].outcome, "weak_accepted");
        assert_eq!(decoded[1].outcome, "unknown");
        assert_eq!(decoded[0].producer, "cli-single-lhb");
        assert_eq!(
            decoded[0].request_body_sha256,
            decoded[1].request_body_sha256
        );
        for (receipt, request) in decoded.iter().zip(requests.iter()) {
            let body = received_body(request);
            let mut digest = Sha256::new();
            digest.update(b"stock_analysis.notification_http_entity.v1\0");
            digest.update(body);
            assert_eq!(
                receipt.request_body_sha256,
                format!("{:x}", digest.finalize())
            );
            assert_eq!(receipt.request_body_len, body.len());
        }
        let text = String::from_utf8(bytes).expect("UTF-8 JSON");
        for secret in [
            &target_url,
            "TEST_CODE_TOKEN_SECRET",
            "TEST_CODE_REPORT_SECRET",
        ] {
            assert!(!text.contains(secret));
        }
        assert!(persist_custom_target_receipts(&snapshot, &report, directory.path()).is_err());
        assert_eq!(std::fs::read(&stored.path).unwrap(), text.as_bytes());
    }

    #[test]
    fn no_target_report_cannot_masquerade_as_a_persisted_send() {
        let invocation = CliInvocationIdentity::new(CliProducer::Default);
        let snapshot = CliReportSnapshot::new(invocation.summary_notification(), "report".into());
        let directory = tempfile::tempdir().expect("temporary receipt store");
        assert!(persist_custom_target_receipts(
            &snapshot,
            &crate::notification::NotificationSendReport::default(),
            directory.path()
        )
        .is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
