//! Invocation-local CLI identities and the report handed to the existing sender.
//! These process-local values prepare a future durable binding; they are not
//! cross-restart occurrence keys or delivery receipts.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::monitor::push_job::ExactBytes;

static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CliProducer {
    Default,
    Schedule,
    Lhb,
    Direct,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliInvocationIdentity {
    id: String,
    producer: CliProducer,
}

impl CliInvocationIdentity {
    pub(super) fn new(producer: CliProducer) -> Self {
        let sequence = NEXT_INVOCATION.fetch_add(1, Ordering::Relaxed);
        Self {
            id: format!(
                "cli-{}-{:x}-{:x}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
                std::process::id(),
                sequence
            ),
            producer,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn producer(&self) -> CliProducer {
        self.producer
    }

    pub(super) fn stock_business(&self, code: &str) -> CliBusinessIdentity {
        CliBusinessIdentity {
            invocation: self.clone(),
            subject: CliSubject::Stock(code.to_owned()),
        }
    }

    pub(super) fn stock_notification(&self, code: &str) -> CliNotificationIdentity {
        CliNotificationIdentity {
            invocation: self.clone(),
            subject: CliSubject::Stock(code.to_owned()),
        }
    }

    pub(super) fn summary_business(&self) -> CliBusinessIdentity {
        CliBusinessIdentity {
            invocation: self.clone(),
            subject: CliSubject::Summary,
        }
    }

    pub(super) fn summary_notification(&self) -> CliNotificationIdentity {
        CliNotificationIdentity {
            invocation: self.clone(),
            subject: CliSubject::Summary,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliSubject {
    Stock(String),
    Summary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliBusinessIdentity {
    invocation: CliInvocationIdentity,
    subject: CliSubject,
}

impl CliBusinessIdentity {
    pub fn invocation(&self) -> &CliInvocationIdentity {
        &self.invocation
    }

    pub fn subject(&self) -> &CliSubject {
        &self.subject
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliNotificationIdentity {
    invocation: CliInvocationIdentity,
    subject: CliSubject,
}

impl CliNotificationIdentity {
    pub fn invocation(&self) -> &CliInvocationIdentity {
        &self.invocation
    }

    pub fn subject(&self) -> &CliSubject {
        &self.subject
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliReportSnapshot {
    identity: CliNotificationIdentity,
    report_bytes: ExactBytes,
}

impl CliReportSnapshot {
    pub(super) fn new(identity: CliNotificationIdentity, report: String) -> Self {
        Self {
            identity,
            report_bytes: ExactBytes::new(report.into_bytes()),
        }
    }

    pub fn identity(&self) -> &CliNotificationIdentity {
        &self.identity
    }

    /// UTF-8 report bytes, before channel-specific encoding, chunking or fallback.
    pub fn report_bytes(&self) -> &ExactBytes {
        &self.report_bytes
    }

    pub(super) fn report(&self) -> &str {
        std::str::from_utf8(self.report_bytes.as_bytes())
            .expect("snapshot was constructed from a UTF-8 String")
    }
}

#[cfg(test)]
mod tests {
    use super::{CliInvocationIdentity, CliProducer, CliReportSnapshot, CliSubject};

    #[test]
    fn repeated_invocations_and_single_summary_have_distinct_scope() {
        let first = CliInvocationIdentity::new(CliProducer::Default);
        let second = CliInvocationIdentity::new(CliProducer::Default);
        assert_ne!(first, second);
        assert_eq!(first.producer(), CliProducer::Default);
        assert_eq!(first.stock_business("TEST_CODE").invocation(), &first);
        assert_eq!(first.stock_notification("TEST_CODE").invocation(), &first);
        assert_eq!(first.summary_business().invocation(), &first);
        assert_eq!(first.summary_notification().invocation(), &first);
        assert_eq!(
            first.stock_business("TEST_CODE").subject(),
            &CliSubject::Stock("TEST_CODE".into())
        );
        assert_eq!(first.summary_notification().subject(), &CliSubject::Summary);
        assert_ne!(
            first.stock_notification("TEST_CODE"),
            first.summary_notification()
        );
        assert_ne!(
            first.stock_notification("TEST_CODE"),
            first.stock_notification("OTHER_CODE")
        );
        assert_ne!(
            first.stock_notification("TEST_CODE"),
            second.stock_notification("TEST_CODE")
        );
        let lhb = CliInvocationIdentity::new(CliProducer::Lhb);
        assert_eq!(lhb.producer(), CliProducer::Lhb);
        assert_ne!(
            lhb.stock_business("TEST_CODE"),
            first.stock_business("TEST_CODE")
        );
    }

    #[test]
    fn snapshot_binds_original_report_bytes_without_claiming_channel_entity() {
        let invocation = CliInvocationIdentity::new(CliProducer::Schedule);
        let identity = invocation.stock_notification("TEST_CODE");
        let first = CliReportSnapshot::new(identity.clone(), "测试  \n".into());
        let changed = CliReportSnapshot::new(identity, "测试 \n".into());
        assert_eq!(first.report_bytes().as_bytes(), "测试  \n".as_bytes());
        assert_eq!(first.report(), "测试  \n");
        assert_eq!(first.identity(), changed.identity());
        assert_ne!(
            first.report_bytes().sha256(),
            changed.report_bytes().sha256()
        );
        assert!(!format!("{first:?}").contains("测试"));
    }
}
