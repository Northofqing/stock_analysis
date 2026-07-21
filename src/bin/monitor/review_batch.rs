//! BR-140 typed post-session review outcomes and per-task scheduling.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewTask {
    R02,
    R03,
    R04,
    R05,
    R06,
    R08,
    A10,
    A01,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewTaskOutcome {
    Delivered {
        count: usize,
    },
    NoData {
        reason: String,
    },
    ExpectedWait {
        retry_at: chrono::NaiveTime,
        reason: String,
    },
    Disabled {
        capability: String,
        reason: String,
    },
    Failed {
        retryable: bool,
        reason: String,
    },
}

impl ReviewTaskOutcome {
    pub fn delivered(count: usize) -> Self {
        Self::Delivered { count }
    }

    pub fn expected_wait(retry_at: chrono::NaiveTime, reason: impl Into<String>) -> Self {
        Self::ExpectedWait {
            retry_at,
            reason: reason.into(),
        }
    }

    pub fn disabled(capability: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Disabled {
            capability: capability.into(),
            reason: reason.into(),
        }
    }

    pub fn failed(retryable: bool, reason: impl Into<String>) -> Self {
        Self::Failed {
            retryable,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBatchOutcome {
    pub tasks: Vec<(ReviewTask, ReviewTaskOutcome)>,
}

impl ReviewBatchOutcome {
    pub fn new(tasks: Vec<(ReviewTask, ReviewTaskOutcome)>) -> Self {
        Self { tasks }
    }

    pub fn delivered_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|(_, outcome)| matches!(outcome, ReviewTaskOutcome::Delivered { .. }))
            .count()
    }

    pub fn has_confirmed_delivery(&self) -> bool {
        self.delivered_count() > 0
    }

    pub fn waiting_tasks(&self) -> Vec<ReviewTask> {
        self.tasks_by(|outcome| matches!(outcome, ReviewTaskOutcome::ExpectedWait { .. }))
    }

    pub fn disabled_tasks(&self) -> Vec<ReviewTask> {
        self.tasks_by(|outcome| matches!(outcome, ReviewTaskOutcome::Disabled { .. }))
    }

    pub fn failed_tasks(&self) -> Vec<ReviewTask> {
        self.tasks_by(|outcome| matches!(outcome, ReviewTaskOutcome::Failed { .. }))
    }

    fn tasks_by(&self, predicate: impl Fn(&ReviewTaskOutcome) -> bool) -> Vec<ReviewTask> {
        self.tasks
            .iter()
            .filter_map(|(task, outcome)| predicate(outcome).then_some(*task))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn br140_batch_classifies_every_outcome_without_calling_wait_disabled_failed_success() {
        let retry_at = chrono::NaiveTime::from_hms_opt(21, 0, 0).expect("valid test time");
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::A01, ReviewTaskOutcome::delivered(1)),
            (
                ReviewTask::R04,
                ReviewTaskOutcome::expected_wait(retry_at, "source not published"),
            ),
            (
                ReviewTask::R05,
                ReviewTaskOutcome::disabled("signal_outcome", "source absent"),
            ),
            (
                ReviewTask::R08,
                ReviewTaskOutcome::failed(true, "transport"),
            ),
        ]);

        assert_eq!(batch.delivered_count(), 1);
        assert_eq!(batch.waiting_tasks(), vec![ReviewTask::R04]);
        assert_eq!(batch.disabled_tasks(), vec![ReviewTask::R05]);
        assert_eq!(batch.failed_tasks(), vec![ReviewTask::R08]);
    }

    #[test]
    fn br140_batch_zero_delivery_is_not_cli_success() {
        let batch = ReviewBatchOutcome::new(vec![(
            ReviewTask::R05,
            ReviewTaskOutcome::disabled("signal_outcome", "source absent"),
        )]);

        assert!(!batch.has_confirmed_delivery());
    }
}
