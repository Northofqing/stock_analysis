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

impl ReviewTask {
    pub const ALL: [Self; 8] = [
        Self::R02,
        Self::R03,
        Self::R04,
        Self::R05,
        Self::R06,
        Self::R08,
        Self::A10,
        Self::A01,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::R02 => "R-02",
            Self::R03 => "R-03",
            Self::R04 => "R-04",
            Self::R05 => "R-05",
            Self::R06 => "R-06",
            Self::R08 => "R-08",
            Self::A10 => "A-10",
            Self::A01 => "A-01",
        }
    }
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

    pub fn no_data(reason: impl Into<String>) -> Self {
        Self::NoData {
            reason: reason.into(),
        }
    }

    pub fn status_label(&self) -> &'static str {
        match self {
            Self::Delivered { .. } => "delivered",
            Self::NoData { .. } => "no_data",
            Self::ExpectedWait { .. } => "expected_wait",
            Self::Disabled { .. } => "disabled",
            Self::Failed { .. } => "failed",
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskScheduleState {
    Pending,
    Terminal,
    Waiting(chrono::NaiveTime),
    Retry {
        at: chrono::NaiveDateTime,
        failures: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewScheduleState {
    date: chrono::NaiveDate,
    tasks: std::collections::BTreeMap<ReviewTask, TaskScheduleState>,
}

impl ReviewScheduleState {
    pub fn for_date(date: chrono::NaiveDate) -> Self {
        let tasks = ReviewTask::ALL
            .into_iter()
            .map(|task| (task, TaskScheduleState::Pending))
            .collect();
        Self { date, tasks }
    }

    pub fn date(&self) -> chrono::NaiveDate {
        self.date
    }

    pub fn apply(&mut self, batch: &ReviewBatchOutcome, now: chrono::NaiveDateTime) {
        if now.date() != self.date {
            return;
        }
        for (task, outcome) in &batch.tasks {
            let next = match outcome {
                ReviewTaskOutcome::Delivered { .. }
                | ReviewTaskOutcome::NoData { .. }
                | ReviewTaskOutcome::Disabled { .. } => TaskScheduleState::Terminal,
                ReviewTaskOutcome::ExpectedWait { retry_at, .. } => {
                    TaskScheduleState::Waiting(*retry_at)
                }
                ReviewTaskOutcome::Failed {
                    retryable: false, ..
                } => TaskScheduleState::Terminal,
                ReviewTaskOutcome::Failed {
                    retryable: true, ..
                } => {
                    let failures = match self.tasks.get(task) {
                        Some(TaskScheduleState::Retry { failures, .. }) => failures + 1,
                        _ => 1,
                    };
                    let delay_minutes = match failures {
                        1 => 1,
                        2 => 5,
                        _ => 15,
                    };
                    TaskScheduleState::Retry {
                        at: now + chrono::Duration::minutes(delay_minutes),
                        failures,
                    }
                }
            };
            self.tasks.insert(*task, next);
        }
    }

    pub fn is_due(&self, task: ReviewTask, now: chrono::NaiveDateTime) -> bool {
        if now.date() != self.date {
            return false;
        }
        match self.tasks.get(&task) {
            Some(TaskScheduleState::Pending) => true,
            Some(TaskScheduleState::Waiting(retry_at)) => now.time() >= *retry_at,
            Some(TaskScheduleState::Retry { at, .. }) => now >= *at,
            Some(TaskScheduleState::Terminal) | None => false,
        }
    }

    pub fn due_tasks(&self, now: chrono::NaiveDateTime) -> std::collections::BTreeSet<ReviewTask> {
        self.tasks
            .keys()
            .copied()
            .filter(|task| self.is_due(*task, now))
            .collect()
    }

    pub fn has_unfinished_tasks(&self) -> bool {
        self.tasks
            .values()
            .any(|state| !matches!(state, TaskScheduleState::Terminal))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPreflight {
    pub outcomes: Vec<(ReviewTask, ReviewTaskOutcome)>,
    pub runnable: std::collections::BTreeSet<ReviewTask>,
}

impl ReviewPreflight {
    #[cfg(test)]
    pub fn outcome_for(&self, task: ReviewTask) -> Option<&ReviewTaskOutcome> {
        self.outcomes
            .iter()
            .find_map(|(candidate, outcome)| (*candidate == task).then_some(outcome))
    }
}

pub fn review_preflight(
    now: chrono::NaiveTime,
    due: &std::collections::BTreeSet<ReviewTask>,
) -> ReviewPreflight {
    let mut runnable = due.clone();
    let mut outcomes = Vec::new();

    let disabled = [
        (
            ReviewTask::R02,
            "market_review_contract",
            "required main_flow/money_effect/position_limit evidence unavailable",
        ),
        (
            ReviewTask::R05,
            "signal_outcome",
            "no complete signal outcome source",
        ),
        (
            ReviewTask::R06,
            "classified_failure_outcome",
            "no classified failure outcome source",
        ),
    ];
    for (task, capability, reason) in disabled {
        if runnable.remove(&task) {
            outcomes.push((task, ReviewTaskOutcome::disabled(capability, reason)));
        }
    }

    let lhb_ready = chrono::NaiveTime::from_hms_opt(21, 0, 0)
        .expect("BR-140 LHB publication time must be valid");
    if now < lhb_ready && runnable.remove(&ReviewTask::R04) {
        outcomes.push((
            ReviewTask::R04,
            ReviewTaskOutcome::expected_wait(lhb_ready, "LHB source not published before 21:00"),
        ));
    }

    ReviewPreflight { outcomes, runnable }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).expect("valid test date")
    }

    fn at_datetime(hour: u32, minute: u32) -> chrono::NaiveDateTime {
        day().and_hms_opt(hour, minute, 0).expect("valid test time")
    }

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

    #[test]
    fn br140_one_delivery_does_not_complete_waiting_or_retryable_tasks() {
        let mut state = ReviewScheduleState::for_date(day());
        state.apply(
            &ReviewBatchOutcome::new(vec![
                (ReviewTask::A01, ReviewTaskOutcome::delivered(1)),
                (
                    ReviewTask::R04,
                    ReviewTaskOutcome::expected_wait(
                        chrono::NaiveTime::from_hms_opt(21, 0, 0).expect("valid wait time"),
                        "not ready",
                    ),
                ),
                (
                    ReviewTask::R08,
                    ReviewTaskOutcome::failed(true, "transport"),
                ),
            ]),
            at_datetime(19, 0),
        );

        assert!(!state.is_due(ReviewTask::A01, at_datetime(19, 1)));
        assert!(!state.is_due(ReviewTask::R04, at_datetime(20, 59)));
        assert!(state.is_due(ReviewTask::R04, at_datetime(21, 0)));
        assert!(state.is_due(ReviewTask::R08, at_datetime(19, 1)));
        assert!(state.has_unfinished_tasks());
    }

    #[test]
    fn br140_disabled_task_is_terminal_and_absent_from_due_set() {
        let mut state = ReviewScheduleState::for_date(day());
        state.apply(
            &ReviewBatchOutcome::new(vec![(
                ReviewTask::R05,
                ReviewTaskOutcome::disabled("signal_outcome", "source absent"),
            )]),
            at_datetime(19, 0),
        );

        let due = state.due_tasks(at_datetime(23, 0));
        assert!(!due.contains(&ReviewTask::R05));
        assert!(due.contains(&ReviewTask::A01));
    }

    #[test]
    fn br140_review_preflight_disables_missing_capabilities_and_waits_for_lhb() {
        let due = ReviewScheduleState::for_date(day()).due_tasks(at_datetime(19, 0));
        let preflight = review_preflight(
            chrono::NaiveTime::from_hms_opt(19, 0, 0).expect("valid test time"),
            &due,
        );

        assert_eq!(
            preflight.outcome_for(ReviewTask::R02),
            Some(&ReviewTaskOutcome::disabled(
                "market_review_contract",
                "required main_flow/money_effect/position_limit evidence unavailable",
            ))
        );
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R04),
            Some(ReviewTaskOutcome::ExpectedWait { retry_at, .. })
                if *retry_at == chrono::NaiveTime::from_hms_opt(21, 0, 0).expect("valid wait time")
        ));
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R05),
            Some(ReviewTaskOutcome::Disabled { .. })
        ));
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R06),
            Some(ReviewTaskOutcome::Disabled { .. })
        ));
        assert!(!preflight.runnable.contains(&ReviewTask::R02));
        assert!(!preflight.runnable.contains(&ReviewTask::R04));
        assert!(preflight.runnable.contains(&ReviewTask::A01));
    }
}
