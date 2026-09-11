use crate::monitor::push_job::WeakOutcomeKind;

use super::NotificationChannel;

/// One actual notification target observed during a single send_report call.
///
/// target_index is only its zero-based position in this invocation. It is not
/// a durable target identity and must not be used for retry or recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationAttempt {
    channel: NotificationChannel,
    target_index: usize,
    outcome: WeakOutcomeKind,
}

impl NotificationAttempt {
    pub(super) fn new(
        channel: NotificationChannel,
        target_index: usize,
        outcome: WeakOutcomeKind,
    ) -> Self {
        Self {
            channel,
            target_index,
            outcome,
        }
    }

    pub fn channel(&self) -> NotificationChannel {
        self.channel
    }

    pub fn target_index(&self) -> usize {
        self.target_index
    }

    pub fn outcome(&self) -> WeakOutcomeKind {
        self.outcome
    }
}

/// Read-only weak observations from one real notification send invocation.
///
/// Accepted means only that an existing channel method returned its weak
/// success value. It does not prove the full original content was sent, that a
/// user read it, that every required target completed, or that delivery is
/// idempotent across restarts. This report is not durable completion evidence.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotificationSendReport {
    attempts: Vec<NotificationAttempt>,
}

impl NotificationSendReport {
    pub(super) fn from_attempts(attempts: Vec<NotificationAttempt>) -> Self {
        Self { attempts }
    }

    pub fn attempts(&self) -> &[NotificationAttempt] {
        &self.attempts
    }

    pub fn has_success(&self) -> bool {
        self.attempts
            .iter()
            .any(|attempt| attempt.outcome == WeakOutcomeKind::Accepted)
    }
}
