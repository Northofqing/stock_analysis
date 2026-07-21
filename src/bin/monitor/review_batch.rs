//! BR-140 typed post-session review outcomes and per-task scheduling.

use sha2::{Digest, Sha256};

pub fn audit_identity_hash(domain: &str, identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis/review/v1\0");
    hasher.update(domain.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sanitize_reason_code(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .take(64)
        .collect::<String>();
    if sanitized.is_empty() {
        "unspecified".to_string()
    } else {
        sanitized
    }
}

fn review_reason_category(task: ReviewTask, outcome: &ReviewTaskOutcome) -> String {
    let classify_failure = |reason: &str| {
        let normalized = reason.to_ascii_lowercase();
        if normalized.contains("deduplicat") {
            "push_governance_deduplicated"
        } else if normalized.contains("denied") || reason.contains("治理拒绝") {
            "push_governance_denied"
        } else if normalized.contains("sink") || reason.contains("投递") {
            "push_sink_delivery_failed"
        } else if normalized.contains("audit") || reason.contains("审计") {
            "audit_persistence_failed"
        } else if normalized.contains("kline") || reason.contains("日 K") || reason.contains("K 线")
        {
            "daily_kline_unavailable"
        } else if normalized.contains("announcement") || reason.contains("公告") {
            "announcement_source_unavailable"
        } else if normalized.contains("position") || reason.contains("持仓") {
            "position_source_unavailable"
        } else if normalized.contains("industry") || reason.contains("产业链") {
            "industry_evidence_unavailable"
        } else if normalized.contains("lhb") || reason.contains("龙虎榜") {
            "lhb_source_unavailable"
        } else if normalized.contains("transport")
            || normalized.contains("http")
            || normalized.contains("request")
            || reason.contains("请求")
        {
            "source_transport_failed"
        } else if normalized.contains("join")
            || normalized.contains("panic")
            || reason.contains("任务失败")
        {
            "source_task_execution_failed"
        } else if normalized.contains("date") || reason.contains("日期") {
            "invalid_source_date"
        } else {
            match task {
                ReviewTask::R02 => "market_review_contract_failed",
                ReviewTask::R03 => "industry_chain_review_failed",
                ReviewTask::R04 => "lhb_review_failed",
                ReviewTask::R05 => "signal_outcome_review_failed",
                ReviewTask::R06 => "failure_outcome_review_failed",
                ReviewTask::R08 => "event_calendar_review_failed",
                ReviewTask::A10 => "catalyst_review_failed",
                ReviewTask::A01 => "virtual_observation_review_failed",
            }
        }
    };

    match outcome {
        ReviewTaskOutcome::Delivered { .. } => "sink_confirmed".to_string(),
        ReviewTaskOutcome::NoData { reason } if reason.contains("T+1") => {
            "complete_source_no_t1_record".to_string()
        }
        ReviewTaskOutcome::NoData { .. } => "complete_source_no_data".to_string(),
        ReviewTaskOutcome::ExpectedWait { .. } => "source_not_published".to_string(),
        ReviewTaskOutcome::Disabled { capability, .. } => {
            format!("capability_disabled_{}", sanitize_reason_code(capability))
        }
        ReviewTaskOutcome::Failed { reason, .. } => classify_failure(reason).to_string(),
    }
}

fn review_audit_hash(prev_hash: &str, payload: &ReviewAuditPayload) -> Result<String, String> {
    let bytes = serde_json::to_vec(payload)
        .map_err(|error| format!("serialize review audit payload: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(b"\n");
    hasher.update(bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn resolve_review_audit_dir(
    override_base: Option<std::path::PathBuf>,
    is_test: bool,
) -> std::path::PathBuf {
    match override_base {
        Some(base) => base.join(if is_test { "test" } else { "prod" }),
        None if is_test => std::path::PathBuf::from("data/test/review_audit"),
        None => std::path::PathBuf::from("data/review_audit"),
    }
}

pub fn review_audit_dir() -> std::path::PathBuf {
    let is_test = stock_analysis::risk::env_guard::runtime_is_test_process()
        || stock_analysis::risk::env_guard::current_env()
            == stock_analysis::risk::env_guard::TradingEnv::Test;
    resolve_review_audit_dir(
        std::env::var("REVIEW_AUDIT_DIR")
            .ok()
            .map(std::path::PathBuf::from),
        is_test,
    )
}

pub fn append_review_audit(
    dir: &std::path::Path,
    date: chrono::NaiveDate,
    payloads: &[ReviewAuditPayload],
) -> Result<std::path::PathBuf, String> {
    use fs2::FileExt;
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};

    static REVIEW_AUDIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = REVIEW_AUDIT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "review audit writer lock poisoned".to_string())?;

    std::fs::create_dir_all(dir)
        .map_err(|error| format!("create review audit dir {}: {error}", dir.display()))?;
    let path = dir.join(format!("{}.jsonl", date.format("%Y-%m-%d")));
    let lock_path = dir.join(format!("{}.lock", date.format("%Y-%m-%d")));
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("open review audit lock {}: {error}", lock_path.display()))?;
    FileExt::lock_exclusive(&lock_file)
        .map_err(|error| format!("lock review audit {}: {error}", lock_path.display()))?;

    // The OS lock spans validation, append and fsync. Unlike the process-local
    // mutex above, it also serializes the resident monitor and a manual
    // `monitor --review` process. A crashed writer releases the kernel lock;
    // a partial tail remains fail-closed during the next full-chain validation.
    let mut prev_hash = "0".repeat(64);
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("read review audit {}: {error}", path.display()))?;
        if !raw.is_empty() && !raw.ends_with('\n') {
            return Err(format!(
                "review audit {} has an incomplete trailing record",
                path.display()
            ));
        }
        for (line_index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                return Err(format!(
                    "review audit {} contains blank line {}",
                    path.display(),
                    line_index + 1
                ));
            }
            let record: ReviewAuditRecord = serde_json::from_str(line).map_err(|error| {
                format!(
                    "parse review audit {} line {}: {error}",
                    path.display(),
                    line_index + 1
                )
            })?;
            if record.prev_hash != prev_hash {
                return Err(format!(
                    "review audit {} chain mismatch at line {}",
                    path.display(),
                    line_index + 1
                ));
            }
            let expected = review_audit_hash(&record.prev_hash, &record.payload)?;
            if record.record_hash != expected {
                return Err(format!(
                    "review audit {} record hash mismatch at line {}",
                    path.display(),
                    line_index + 1
                ));
            }
            prev_hash = record.record_hash;
        }
    }

    let mut encoded = Vec::new();
    for payload in payloads {
        let record_hash = review_audit_hash(&prev_hash, payload)?;
        let record = ReviewAuditRecord {
            payload: payload.clone(),
            prev_hash,
            record_hash: record_hash.clone(),
        };
        serde_json::to_writer(&mut encoded, &record)
            .map_err(|error| format!("write review audit {}: {error}", path.display()))?;
        encoded
            .write_all(b"\n")
            .map_err(|error| format!("write review audit newline {}: {error}", path.display()))?;
        prev_hash = record_hash;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open review audit {}: {error}", path.display()))?;
    file.write_all(&encoded)
        .map_err(|error| format!("append review audit {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("flush review audit {}: {error}", path.display()))?;
    file.sync_data()
        .map_err(|error| format!("sync review audit {}: {error}", path.display()))?;
    FileExt::unlock(&lock_file)
        .map_err(|error| format!("unlock review audit {}: {error}", lock_path.display()))?;
    Ok(path)
}

pub fn append_task_transition_audit(
    transitions: Vec<ReviewTaskTransition>,
    date: chrono::NaiveDate,
) -> Result<std::path::PathBuf, String> {
    let payloads = transitions
        .into_iter()
        .map(ReviewAuditPayload::TaskTransition)
        .collect::<Vec<_>>();
    append_review_audit(&review_audit_dir(), date, &payloads)
}

pub fn append_candidate_rejection_audit(
    rejections: Vec<ReviewCandidateRejection>,
    date: chrono::NaiveDate,
) -> Result<std::path::PathBuf, String> {
    let payloads = rejections
        .into_iter()
        .map(ReviewAuditPayload::CandidateRejection)
        .collect::<Vec<_>>();
    append_review_audit(&review_audit_dir(), date, &payloads)
}

pub fn append_source_protocol_audit(
    decision: ReviewSourceProtocolDecision,
    date: chrono::NaiveDate,
) -> Result<std::path::PathBuf, String> {
    append_review_audit(
        &review_audit_dir(),
        date,
        &[ReviewAuditPayload::SourceProtocolDecision(decision)],
    )
}

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

    fn source_label(self) -> &'static str {
        match self {
            Self::R02 => "market_review_contract",
            Self::R03 => "portfolio_industry_kline",
            Self::R04 => "lhb_producer",
            Self::R05 => "signal_outcome",
            Self::R06 => "classified_failure_outcome",
            Self::R08 => "announcement_positions_virtual_overnight",
            Self::A10 => "chain_rotation_security_master",
            Self::A01 => "virtual_observation_kline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReviewTaskTransition {
    pub observed_at: String,
    pub task: String,
    pub source: String,
    pub source_time: Option<String>,
    pub rule_ids: Vec<String>,
    pub status: String,
    pub success: bool,
    pub snapshot_size: usize,
    pub retryable: bool,
    pub next_attempt: Option<String>,
    pub reason_code: String,
    pub identity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReviewCandidateRejection {
    pub observed_at: String,
    pub task: String,
    pub source: String,
    pub source_time: Option<String>,
    pub rule_ids: Vec<String>,
    pub retryable: bool,
    pub identity_hash: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReviewSourceProtocolDecision {
    pub observed_at: String,
    pub task: String,
    pub source: String,
    pub source_time: Option<String>,
    pub query_date: String,
    pub selected_protocol: String,
    pub fallback_used: bool,
    pub reason_code: Option<String>,
    pub identity_hash: String,
    pub rule_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum ReviewAuditPayload {
    TaskTransition(ReviewTaskTransition),
    CandidateRejection(ReviewCandidateRejection),
    SourceProtocolDecision(ReviewSourceProtocolDecision),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ReviewAuditRecord {
    payload: ReviewAuditPayload,
    prev_hash: String,
    record_hash: String,
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

    /// Convert the authoritative push governor result without collapsing
    /// deduplication, governance denial, and sink failure into one boolean.
    pub fn from_push_outcome(outcome: crate::notify::PushOutcome, delivered_count: usize) -> Self {
        match outcome {
            crate::notify::PushOutcome::Pushed => Self::delivered(delivered_count),
            crate::notify::PushOutcome::Deduped => {
                Self::failed(false, "delivery deduplicated by push governance")
            }
            crate::notify::PushOutcome::Denied(reason) => Self::failed(
                false,
                format!("delivery denied by push governance: {reason}"),
            ),
            crate::notify::PushOutcome::SinkError(reason) => {
                Self::failed(true, format!("delivery sink failed: {reason}"))
            }
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

    pub fn apply(
        &mut self,
        batch: &ReviewBatchOutcome,
        now: chrono::NaiveDateTime,
    ) -> Vec<ReviewTaskTransition> {
        if now.date() != self.date {
            return Vec::new();
        }
        let mut transitions = Vec::with_capacity(batch.tasks.len());
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
            self.tasks.insert(*task, next.clone());
            let (retryable, next_attempt) = match &next {
                TaskScheduleState::Waiting(retry_at) => (
                    true,
                    Some(
                        self.date
                            .and_time(*retry_at)
                            .format("%Y-%m-%dT%H:%M:%S")
                            .to_string(),
                    ),
                ),
                TaskScheduleState::Retry { at, .. } => {
                    (true, Some(at.format("%Y-%m-%dT%H:%M:%S").to_string()))
                }
                TaskScheduleState::Pending | TaskScheduleState::Terminal => (false, None),
            };
            let snapshot_size = match outcome {
                ReviewTaskOutcome::Delivered { count } => *count,
                _ => 0,
            };
            let reason_code = review_reason_category(*task, outcome);
            let reason_detail = match outcome {
                ReviewTaskOutcome::Delivered { .. } => "sink_confirmed",
                ReviewTaskOutcome::NoData { reason }
                | ReviewTaskOutcome::ExpectedWait { reason, .. }
                | ReviewTaskOutcome::Disabled { reason, .. }
                | ReviewTaskOutcome::Failed { reason, .. } => reason.as_str(),
            };
            let reason_fingerprint = audit_identity_hash("review-reason", reason_detail);
            let reason_code = format!("{reason_code}_{}", &reason_fingerprint[..16]);
            let observed_at = now.format("%Y-%m-%dT%H:%M:%S").to_string();
            transitions.push(ReviewTaskTransition {
                observed_at: observed_at.clone(),
                task: task.label().to_string(),
                source: task.source_label().to_string(),
                // ReviewTaskOutcome currently carries report status, not a
                // provider publication timestamp. Query/as-of date is encoded
                // in the task identity; missing provider time stays absent.
                source_time: None,
                rule_ids: vec!["BR-110".to_string(), "BR-140".to_string()],
                status: outcome.status_label().to_string(),
                success: matches!(outcome, ReviewTaskOutcome::Delivered { .. }),
                snapshot_size,
                retryable,
                next_attempt,
                reason_code,
                identity_hash: audit_identity_hash(
                    "review-task",
                    &format!("{}:{}", self.date, task.label()),
                ),
            });
        }
        transitions
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

    #[test]
    fn br140_audit_identity_hash_is_stable_domain_separated_and_non_reversible() {
        let identity = "TEST_CODE_SECRET_IDENTITY";
        let first = audit_identity_hash("A-01", identity);
        let second = audit_identity_hash("A-01", identity);
        let other_domain = audit_identity_hash("R-03", identity);

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        assert_ne!(first, other_domain);
        assert!(!first.contains(identity));
    }

    #[test]
    fn br140_review_audit_override_keeps_test_and_production_physically_separate() {
        let base = std::path::PathBuf::from("/tmp/stock_analysis_review_audit_override");
        let test = resolve_review_audit_dir(Some(base.clone()), true);
        let prod = resolve_review_audit_dir(Some(base), false);

        assert_ne!(test, prod);
        assert!(test.ends_with("test"));
        assert!(prod.ends_with("prod"));
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
    fn br140_push_outcomes_preserve_terminal_and_retryable_semantics() {
        assert_eq!(
            ReviewTaskOutcome::from_push_outcome(crate::notify::PushOutcome::Pushed, 2),
            ReviewTaskOutcome::delivered(2)
        );
        assert!(matches!(
            ReviewTaskOutcome::from_push_outcome(crate::notify::PushOutcome::Deduped, 2),
            ReviewTaskOutcome::Failed {
                retryable: false,
                ..
            }
        ));
        assert!(matches!(
            ReviewTaskOutcome::from_push_outcome(
                crate::notify::PushOutcome::Denied("policy".to_string()),
                2
            ),
            ReviewTaskOutcome::Failed {
                retryable: false,
                ..
            }
        ));
        assert!(matches!(
            ReviewTaskOutcome::from_push_outcome(
                crate::notify::PushOutcome::SinkError("transport".to_string()),
                2
            ),
            ReviewTaskOutcome::Failed {
                retryable: true,
                ..
            }
        ));
    }

    #[test]
    fn br140_one_delivery_does_not_complete_waiting_or_retryable_tasks() {
        let mut state = ReviewScheduleState::for_date(day());
        let transitions = state.apply(
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
        let r08 = transitions
            .iter()
            .find(|transition| transition.task == "R-08")
            .unwrap();
        assert!(r08.retryable);
        assert_eq!(r08.next_attempt.as_deref(), Some("2026-07-21T19:01:00"));
        assert!(!r08.success);
        assert_eq!(r08.source_time, None);
        assert!(r08.reason_code.starts_with("source_transport_failed_"));
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
    fn br140_review_reason_codes_preserve_decision_category_without_raw_identity() {
        let cases = [
            (
                ReviewTask::R03,
                ReviewTaskOutcome::failed(true, "603031 日 K 批次失败"),
                "daily_kline_unavailable",
            ),
            (
                ReviewTask::R08,
                ReviewTaskOutcome::failed(true, "公告 provenance 审计失败"),
                "audit_persistence_failed",
            ),
            (
                ReviewTask::A01,
                ReviewTaskOutcome::failed(true, "delivery sink failed"),
                "push_sink_delivery_failed",
            ),
        ];

        for (task, outcome, expected) in cases {
            let category = review_reason_category(task, &outcome);
            assert_eq!(category, expected);
            assert!(!category.contains("603031"));
        }
    }

    #[test]
    fn br140_review_audit_is_valid_json_hash_chained_and_detects_tamper() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_review_audit_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let identity = "TEST_CODE_PRIVATE_IDENTITY";
        let payload = ReviewAuditPayload::CandidateRejection(ReviewCandidateRejection {
            observed_at: "2026-07-21T19:00:00".to_string(),
            task: "A-01".to_string(),
            source: "virtual_observation".to_string(),
            source_time: Some("2026-07-21T18:59:59".to_string()),
            rule_ids: vec!["BR-104".to_string(), "BR-140".to_string()],
            retryable: false,
            identity_hash: audit_identity_hash("A-01", identity),
            reason_code: "invalid_json".to_string(),
        });
        let path = append_review_audit(&dir, day(), std::slice::from_ref(&payload)).unwrap();
        append_review_audit(&dir, day(), &[payload]).unwrap();
        assert!(dir.join("2026-07-21.lock").exists());

        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2);
        assert!(!raw.contains(identity));
        for line in raw.lines() {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }

        let tampered = raw.replacen("invalid_json", "changed", 1);
        std::fs::write(&path, tampered).unwrap();
        let error = append_review_audit(&dir, day(), &[]).unwrap_err();
        assert!(error.contains("record hash mismatch"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn br140_review_audit_rejects_a_valid_record_without_trailing_newline() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_review_audit_tail_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let payload = ReviewAuditPayload::CandidateRejection(ReviewCandidateRejection {
            observed_at: "2026-07-21T19:00:00".to_string(),
            task: "A-01".to_string(),
            source: "virtual_observation".to_string(),
            source_time: None,
            rule_ids: vec!["BR-140".to_string()],
            retryable: false,
            identity_hash: audit_identity_hash("A-01", "TEST_CODE_TAIL"),
            reason_code: "invalid_json".to_string(),
        });
        let path = append_review_audit(&dir, day(), std::slice::from_ref(&payload)).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, raw.strip_suffix('\n').unwrap()).unwrap();

        let error = append_review_audit(&dir, day(), &[payload]).unwrap_err();

        assert!(error.contains("incomplete trailing record"));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("}{"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "invoked as a child by the cross-process locking test"]
    fn br140_review_audit_process_writer_helper() {
        let Ok(dir) = std::env::var("BR140_REVIEW_AUDIT_HELPER_DIR") else {
            return;
        };
        let identity = std::env::var("BR140_REVIEW_AUDIT_HELPER_ID").unwrap();
        let payload = ReviewAuditPayload::CandidateRejection(ReviewCandidateRejection {
            observed_at: "2026-07-21T19:00:00".to_string(),
            task: "A-01".to_string(),
            source: "cross_process_test".to_string(),
            source_time: None,
            rule_ids: vec!["BR-140".to_string()],
            retryable: false,
            identity_hash: audit_identity_hash("A-01", &identity),
            reason_code: "cross_process_test".to_string(),
        });
        append_review_audit(std::path::Path::new(&dir), day(), &[payload]).unwrap();
    }

    #[test]
    fn br140_review_audit_serializes_independent_process_writers() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_review_audit_process_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let executable = std::env::current_exe().unwrap();
        let mut children = (0..4)
            .map(|index| {
                std::process::Command::new(&executable)
                    .args([
                        "--exact",
                        "review_batch::tests::br140_review_audit_process_writer_helper",
                        "--ignored",
                    ])
                    .env("BR140_REVIEW_AUDIT_HELPER_DIR", &dir)
                    .env("BR140_REVIEW_AUDIT_HELPER_ID", format!("writer-{index}"))
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }

        let path = append_review_audit(&dir, day(), &[]).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
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
