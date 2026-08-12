//! Registered business rules: BR-140, BR-192, BR-194, BR-198, BR-199, BR-209, BR-212.
//! BR-140 typed post-session review outcomes and per-task scheduling.

use sha2::{Digest, Sha256};

/// BR-140 keeps the business as-of date separate from the real observation
/// clock. Providers, task identities and audit partitions use `business_date`;
/// diagnostics retain the unmodified `observed_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewRunContext {
    review_date: chrono::NaiveDate,
    observed_at: chrono::NaiveDateTime,
    /// 2026-08-06: 手动 --review 时跳过 21:00 龙虎榜发布门 (R-04/R-07 立即
    /// 尝试, 未发布数据由 dispatcher 内部降级 + 出声)。自动调度保持等待。
    manual_override: bool,
}

impl ReviewRunContext {
    pub fn at(observed_at: chrono::NaiveDateTime) -> Self {
        Self {
            review_date: stock_analysis::calendar::latest_completed_trading_day_at(observed_at),
            observed_at,
            manual_override: false,
        }
    }

    /// 手动触发 (--review CLI): 不等待 21:00 龙虎榜发布门。
    pub fn at_manual(observed_at: chrono::NaiveDateTime) -> Self {
        Self {
            review_date: stock_analysis::calendar::latest_completed_trading_day_at(observed_at),
            observed_at,
            manual_override: true,
        }
    }

    pub fn manual_override(self) -> bool {
        self.manual_override
    }

    pub fn review_date(self) -> chrono::NaiveDate {
        self.review_date
    }

    /// Frozen evidence business date for every provider and report in this run.
    ///
    /// `review_date` remains as a compatibility accessor; new code uses this
    /// domain name so it cannot be confused with the observation wall clock.
    pub fn business_date(self) -> chrono::NaiveDate {
        self.review_date
    }

    pub fn observed_at(self) -> chrono::NaiveDateTime {
        self.observed_at
    }

    pub fn observed_at_fixed(self) -> chrono::DateTime<chrono::FixedOffset> {
        fixed_shanghai_datetime(self.observed_at)
    }

    pub fn eligibility_time(self) -> chrono::NaiveTime {
        if self.observed_at.date() > self.review_date {
            chrono::NaiveTime::from_hms_opt(23, 59, 59)
                .expect("BR-140 end-of-business-day time is valid")
        } else {
            self.observed_at.time()
        }
    }
}

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

/// Raw, untruncated cause for a review outcome. Never returns an empty string:
/// an outcome without a cause is itself a defect worth surfacing.
fn review_outcome_detail(outcome: &ReviewTaskOutcome) -> String {
    let detail = match outcome {
        ReviewTaskOutcome::Delivered { count } => format!("delivered count={count}"),
        ReviewTaskOutcome::NoData { reason } => reason.clone(),
        ReviewTaskOutcome::ExpectedWait { retry_at, reason } => {
            format!("{reason} | retry_at={retry_at}")
        }
        ReviewTaskOutcome::DeferredUntil { at, reason } => {
            format!("{reason:?} | deferred_until={}", at.to_rfc3339())
        }
        ReviewTaskOutcome::Disabled { capability, reason } => {
            format!("capability={capability} | {reason}")
        }
        ReviewTaskOutcome::Failed {
            failure: ReviewTaskFailure::ExistingSourceFailure { reason, .. },
        } => reason.clone(),
        ReviewTaskOutcome::Failed {
            failure: ReviewTaskFailure::AccountDependency(failure),
        } => format!(
            "stage={:?} reason_code={:?} source_provider={} source_time={} observed_at={}",
            failure.stage,
            failure.reason_code,
            failure.source_provider.as_deref().unwrap_or("none"),
            failure
                .source_time
                .map(|value| value.to_rfc3339())
                .unwrap_or_else(|| "none".to_string()),
            failure.observed_at.to_rfc3339(),
        ),
    };
    if detail.trim().is_empty() {
        "unspecified".to_string()
    } else {
        detail
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
                ReviewTask::R07 => "tomorrow_watch_failed",
                ReviewTask::R08 => "event_calendar_review_failed",
                ReviewTask::R09 => "provider_top_n_review_failed",
                ReviewTask::R11 => "position_review_failed",
                ReviewTask::R12 => "paper_backtest_failed",
                ReviewTask::R13 => "watchlist_tracking_failed",
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
        ReviewTaskOutcome::DeferredUntil { .. } => "push_governance_deferred".to_string(),
        ReviewTaskOutcome::Disabled { capability, .. } => {
            format!("capability_disabled_{}", sanitize_reason_code(capability))
        }
        ReviewTaskOutcome::Failed {
            failure: ReviewTaskFailure::ExistingSourceFailure { reason, .. },
        } => classify_failure(reason).to_string(),
        ReviewTaskOutcome::Failed {
            failure: ReviewTaskFailure::AccountDependency(_),
        } => "account_metrics_incomplete".to_string(),
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

    for payload in payloads {
        validate_review_audit_payload_for_append(payload)?;
    }

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

fn validate_review_audit_payload_for_append(payload: &ReviewAuditPayload) -> Result<(), String> {
    if let ReviewAuditPayload::TaskTransition(transition) = payload {
        let value = serde_json::to_value(transition)
            .map_err(|error| format!("serialize review task transition for validation: {error}"))?;
        serde_json::from_value::<ReviewTaskTransition>(value)
            .map_err(|error| format!("validate review task transition before append: {error}"))?;
    }
    Ok(())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewTask {
    R02,
    R03,
    R04,
    R05,
    R06,
    R07,
    R08,
    R09,
    R11,
    R12,
    R13,
    A10,
    A01,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTaskDependency {
    SourceOnly,
    LegacyAccountGate,
    UnclassifiedConservative,
}

impl ReviewTask {
    pub const ALL: [Self; 13] = [
        Self::R02,
        Self::R03,
        Self::R04,
        Self::R05,
        Self::R06,
        Self::R07,
        Self::R08,
        Self::R09,
        Self::R11,
        Self::R12,
        Self::R13,
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
            Self::R07 => "R-07",
            Self::R08 => "R-08",
            Self::R09 => "R-09",
            Self::R11 => "R-11",
            Self::R12 => "R-12",
            Self::R13 => "R-13",
            Self::A10 => "A-10",
            Self::A01 => "A-01",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|task| task.label() == label)
    }

    fn source_label(self) -> &'static str {
        match self {
            Self::R02 => "market_review_contract",
            Self::R03 => "portfolio_industry_kline",
            Self::R04 => "lhb_producer",
            Self::R05 => "signal_outcome",
            Self::R06 => "classified_failure_outcome",
            Self::R07 => "tomorrow_watch_sources",
            Self::R08 => "event_calendar_public_component_batches",
            Self::R09 => "eastmoney_provider_top_n",
            Self::R11 => "user_confirmed_position_summary",
            Self::R12 => "paper_trades_15min_backtest",
            Self::R13 => "catalyst_watchlist_t1_tracking",
            Self::A10 => "chain_rotation_security_master",
            Self::A01 => "virtual_observation_kline",
        }
    }

    pub fn dependency(self) -> ReviewTaskDependency {
        match self {
            Self::R04
            | Self::R07
            | Self::R08
            | Self::R09
            | Self::R11
            | Self::R12
            | Self::R13
            | Self::A10
            | Self::A01 => ReviewTaskDependency::SourceOnly,
            // BR-194 §4.2: R-03 读的是 portfolio projection，不是 verified broker
            // batch + 同批 trade-sync watermark，因此必须留在账户依赖闸门内。
            Self::R03 => ReviewTaskDependency::LegacyAccountGate,
            Self::R02 | Self::R05 | Self::R06 => ReviewTaskDependency::UnclassifiedConservative,
        }
    }
}

/// One shared identity derivation for BR-140 task scheduling and BR-192's
/// canonical R-09 binding. Callers must not duplicate this algorithm.
pub fn review_task_identity(date: chrono::NaiveDate, task: ReviewTask) -> String {
    audit_identity_hash("review-task", &format!("{}:{}", date, task.label()))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<ReviewTransitionFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defer: Option<ReviewTransitionDefer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDeferReasonCode {
    QuietHour,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTransitionDefer {
    pub reason_code: ReviewDeferReasonCode,
    pub deferred_until: chrono::DateTime<chrono::FixedOffset>,
    pub provider_calls: usize,
    pub renderer_calls: usize,
    pub sink_calls: usize,
    pub automatic_retry: bool,
    pub manual_reinvoke_required: bool,
}

impl<'de> serde::Deserialize<'de> for ReviewTaskTransition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            observed_at: String,
            task: String,
            source: String,
            source_time: Option<String>,
            rule_ids: Vec<String>,
            status: String,
            success: bool,
            snapshot_size: usize,
            retryable: bool,
            next_attempt: Option<String>,
            reason_code: String,
            identity_hash: String,
            #[serde(default)]
            failure: Option<ReviewTransitionFailure>,
            #[serde(default)]
            defer: Option<ReviewTransitionDefer>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value.as_object().ok_or_else(|| {
            serde::de::Error::custom("review task transition must be a JSON object")
        })?;
        if let Some(failure) = object.get("failure") {
            if !failure.is_object() {
                return Err(serde::de::Error::custom(
                    "review task transition failure must be an object when present",
                ));
            }
        }
        if let Some(defer) = object.get("defer") {
            if !defer.is_object() {
                return Err(serde::de::Error::custom(
                    "review task transition defer must be an object when present",
                ));
            }
        }
        let wire: Wire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        if wire.status != "failed" && wire.failure.is_some() {
            return Err(serde::de::Error::custom(
                "non-failed review task transition must omit failure",
            ));
        }
        match (wire.status.as_str(), wire.defer.as_ref()) {
            ("deferred", Some(defer)) => {
                let expected_attempt = defer.deferred_until.to_rfc3339();
                let observed_at =
                    chrono::NaiveDateTime::parse_from_str(&wire.observed_at, "%Y-%m-%dT%H:%M:%S")
                        .map_err(|_| {
                        serde::de::Error::custom(
                            "BR-209 deferred observation must be a canonical local timestamp",
                        )
                    })?;
                let expected_release =
                    a10_quiet_hour_release_at(fixed_shanghai_datetime(observed_at));
                let shanghai_offset_seconds = 8 * 60 * 60;
                if wire.task != ReviewTask::A10.label()
                    || wire.source != "review_preflight_quiet_hour_policy"
                    || !wire.rule_ids.iter().any(|rule| rule == "BR-209")
                    || wire.success
                    || wire.snapshot_size != 0
                    || !wire.retryable
                    || wire.next_attempt.as_deref() != Some(expected_attempt.as_str())
                    || defer.reason_code != ReviewDeferReasonCode::QuietHour
                    || defer.deferred_until.offset().local_minus_utc() != shanghai_offset_seconds
                    || defer.deferred_until != expected_release
                    || defer.provider_calls != 0
                    || defer.renderer_calls != 0
                    || defer.sink_calls != 0
                    || defer.automatic_retry
                    || !defer.manual_reinvoke_required
                {
                    return Err(serde::de::Error::custom(
                        "deferred review task transition violates BR-209",
                    ));
                }
            }
            ("deferred", None) => {
                return Err(serde::de::Error::custom(
                    "deferred review task transition requires defer evidence",
                ));
            }
            (_, Some(_)) => {
                return Err(serde::de::Error::custom(
                    "non-deferred review task transition must omit defer evidence",
                ));
            }
            (_, None) => {}
        }
        Ok(Self {
            observed_at: wire.observed_at,
            task: wire.task,
            source: wire.source,
            source_time: wire.source_time,
            rule_ids: wire.rule_ids,
            status: wire.status,
            success: wire.success,
            snapshot_size: wire.snapshot_size,
            retryable: wire.retryable,
            next_attempt: wire.next_attempt,
            reason_code: wire.reason_code,
            identity_hash: wire.identity_hash,
            failure: wire.failure,
            defer: wire.defer,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "failure_class", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewTransitionFailure {
    ExistingSourceFailure {
        retryable: bool,
        reason: String,
    },
    AccountDependency {
        stage: ReviewAccountDependencyStage,
        reason_code: ReviewAccountFailureReasonCode,
        retryable: bool,
        source_provider: Option<String>,
        source_time: Option<chrono::DateTime<chrono::FixedOffset>>,
        observed_at: chrono::DateTime<chrono::FixedOffset>,
        evidence_identity_hash: Option<String>,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct ReviewAuditRecord {
    payload: ReviewAuditPayload,
    prev_hash: String,
    record_hash: String,
}

// This exact immutable production record was emitted by the first BR-209 live
// probe before typed defer evidence was added. It cannot be deleted or edited
// under Rule 2.7. Admission is deliberately byte-exact; every other deferred
// record must pass ReviewTaskTransition's typed BR-209 validation.
const BR209_LEGACY_UNTYPED_DEFER_RECORD: &[u8] = br#"{"payload":{"event_type":"task_transition","observed_at":"2026-08-04T06:25:51","task":"A-10","source":"review_preflight_quiet_hour_policy","source_time":null,"rule_ids":["BR-110","BR-140","BR-209"],"status":"deferred","success":false,"snapshot_size":0,"retryable":true,"next_attempt":"2026-08-05T06:00:00+08:00","reason_code":"push_governance_deferred_b024b797abaa07c7","identity_hash":"685d44098f02f84c10b88fa19fb379f8da1248ff24ed2f4893b842ddec4689ba"},"prev_hash":"efac0c34cb56827c1ceb717106ac3bf3271797bd46922765931a3bfaa1de39e4","record_hash":"983462d427ef5f174436cb2276a8a66ec37f7ae50e3a62c183a28f09c85c4d69"}"#;

impl<'de> serde::Deserialize<'de> for ReviewAuditRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            payload: ReviewAuditPayload,
            prev_hash: String,
            record_hash: String,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let legacy_value: serde_json::Value =
            serde_json::from_slice(BR209_LEGACY_UNTYPED_DEFER_RECORD)
                .expect("fixed BR-209 legacy audit fixture must be valid JSON");
        if value == legacy_value {
            return Ok(Self {
                payload: ReviewAuditPayload::TaskTransition(ReviewTaskTransition {
                    observed_at: "2026-08-04T06:25:51".to_string(),
                    task: "A-10".to_string(),
                    source: "review_preflight_quiet_hour_policy".to_string(),
                    source_time: None,
                    rule_ids: vec![
                        "BR-110".to_string(),
                        "BR-140".to_string(),
                        "BR-209".to_string(),
                    ],
                    status: "deferred".to_string(),
                    success: false,
                    snapshot_size: 0,
                    retryable: true,
                    next_attempt: Some("2026-08-05T06:00:00+08:00".to_string()),
                    reason_code: "push_governance_deferred_b024b797abaa07c7".to_string(),
                    identity_hash:
                        "685d44098f02f84c10b88fa19fb379f8da1248ff24ed2f4893b842ddec4689ba"
                            .to_string(),
                    failure: None,
                    defer: None,
                }),
                prev_hash: "efac0c34cb56827c1ceb717106ac3bf3271797bd46922765931a3bfaa1de39e4"
                    .to_string(),
                record_hash: "983462d427ef5f174436cb2276a8a66ec37f7ae50e3a62c183a28f09c85c4d69"
                    .to_string(),
            });
        }

        let wire: Wire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            payload: wire.payload,
            prev_hash: wire.prev_hash,
            record_hash: wire.record_hash,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAccountFailureReasonCode {
    AccountMetricsIncomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAccountDependencyStage {
    AcquireBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAccountDependencyFailure {
    pub stage: ReviewAccountDependencyStage,
    pub reason_code: ReviewAccountFailureReasonCode,
    pub retryable: bool,
    pub source_provider: Option<String>,
    pub source_time: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub evidence_identity_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "failure_class", rename_all = "snake_case")]
pub enum ReviewTaskFailure {
    ExistingSourceFailure { retryable: bool, reason: String },
    AccountDependency(ReviewAccountDependencyFailure),
}

impl ReviewTaskFailure {
    fn retryable(&self) -> bool {
        match self {
            Self::ExistingSourceFailure { retryable, .. } => *retryable,
            Self::AccountDependency(failure) => failure.retryable,
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
    DeferredUntil {
        at: chrono::DateTime<chrono::FixedOffset>,
        reason: ReviewDeferReasonCode,
    },
    Disabled {
        capability: String,
        reason: String,
    },
    Failed {
        failure: ReviewTaskFailure,
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

    pub fn deferred_until(
        at: chrono::DateTime<chrono::FixedOffset>,
        reason: ReviewDeferReasonCode,
    ) -> Self {
        Self::DeferredUntil { at, reason }
    }

    pub fn disabled(capability: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Disabled {
            capability: capability.into(),
            reason: reason.into(),
        }
    }

    pub fn failed(retryable: bool, reason: impl Into<String>) -> Self {
        Self::Failed {
            failure: ReviewTaskFailure::ExistingSourceFailure {
                retryable,
                reason: reason.into(),
            },
        }
    }

    pub fn account_metrics_incomplete(observed_at: chrono::DateTime<chrono::FixedOffset>) -> Self {
        Self::Failed {
            failure: ReviewTaskFailure::AccountDependency(ReviewAccountDependencyFailure {
                stage: ReviewAccountDependencyStage::AcquireBatch,
                reason_code: ReviewAccountFailureReasonCode::AccountMetricsIncomplete,
                retryable: true,
                source_provider: None,
                source_time: None,
                observed_at,
                evidence_identity_hash: None,
            }),
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
            // BR-207: the L5 denial remains authoritative, but a transient
            // wall-clock gate must not terminally consume the review task.
            crate::notify::PushOutcome::Denied(reason) if reason == "quiet_hour" => {
                Self::failed(true, "delivery deferred by push governance: quiet_hour")
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
            Self::DeferredUntil { .. } => "deferred",
            Self::Disabled { .. } => "disabled",
            Self::Failed { .. } => "failed",
        }
    }
}

/// v15.x rule 4 (silent paths must be visible): every non-delivered review task
/// gets one operator-readable log line carrying its raw reason.
///
/// The raw reason is already persisted in the BR-140 audit JSONL under
/// `failure.reason`, but the aggregated `[B-005-C]` completion line only carries
/// `review_reason_category` + a 16-hex fingerprint, so an operator reading stdout
/// sees `lhb_review_failed_59fa3c07...` and has no way to learn that the actual
/// cause was a durable-delivery terminal state. Emitting the detail keeps the
/// failure diagnosable without grepping the audit trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTaskDiagnostic {
    pub task: ReviewTask,
    pub status: &'static str,
    pub reason_code: String,
    pub retryable: Option<bool>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBatchOutcome {
    pub tasks: Vec<(ReviewTask, ReviewTaskOutcome)>,
}

/// BR-212 keeps a review run's CLI completion distinct from individual task
/// outcomes, so one successful delivery cannot hide incomplete review work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewCompletion {
    Complete,
    Partial,
    NoDelivery,
}

impl ReviewBatchOutcome {
    pub fn new(tasks: Vec<(ReviewTask, ReviewTaskOutcome)>) -> Self {
        Self { tasks }
    }

    /// One diagnostic per non-delivered task, in task order. Delivered tasks are
    /// omitted: they already log their own confirmation.
    pub fn non_delivered_diagnostics(&self) -> Vec<ReviewTaskDiagnostic> {
        self.tasks
            .iter()
            .filter(|(_, outcome)| !matches!(outcome, ReviewTaskOutcome::Delivered { .. }))
            .map(|(task, outcome)| ReviewTaskDiagnostic {
                task: *task,
                status: outcome.status_label(),
                reason_code: review_reason_category(*task, outcome),
                retryable: match outcome {
                    ReviewTaskOutcome::Failed { failure } => Some(failure.retryable()),
                    _ => None,
                },
                detail: review_outcome_detail(outcome),
            })
            .collect()
    }

    pub fn delivered_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|(_, outcome)| matches!(outcome, ReviewTaskOutcome::Delivered { .. }))
            .count()
    }

    #[cfg(test)]
    pub fn has_confirmed_delivery(&self) -> bool {
        self.delivered_count() > 0
    }

    pub fn completion(&self) -> ReviewCompletion {
        if self.delivered_count() == 0 {
            ReviewCompletion::NoDelivery
        } else if self.tasks.iter().any(|(_, outcome)| {
            matches!(
                outcome,
                ReviewTaskOutcome::Failed { .. }
                    | ReviewTaskOutcome::ExpectedWait { .. }
                    | ReviewTaskOutcome::DeferredUntil { .. }
            )
        }) {
            ReviewCompletion::Partial
        } else {
            ReviewCompletion::Complete
        }
    }

    pub fn waiting_tasks(&self) -> Vec<ReviewTask> {
        self.tasks_by(|outcome| matches!(outcome, ReviewTaskOutcome::ExpectedWait { .. }))
    }

    pub fn deferred_tasks(&self) -> Vec<ReviewTask> {
        self.tasks_by(|outcome| matches!(outcome, ReviewTaskOutcome::DeferredUntil { .. }))
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

    pub fn without_tasks(&self, excluded: &std::collections::BTreeSet<ReviewTask>) -> Self {
        Self::new(
            self.tasks
                .iter()
                .filter(|(task, _)| !excluded.contains(task))
                .cloned()
                .collect(),
        )
    }
}

pub fn merge_review_task_outcomes(
    preflight: Vec<(ReviewTask, ReviewTaskOutcome)>,
    source_only: Vec<(ReviewTask, ReviewTaskOutcome)>,
    account_required: Vec<(ReviewTask, ReviewTaskOutcome)>,
) -> Result<ReviewBatchOutcome, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut tasks =
        Vec::with_capacity(preflight.len() + source_only.len() + account_required.len());
    for (task, outcome) in preflight
        .into_iter()
        .chain(source_only)
        .chain(account_required)
    {
        if !seen.insert(task) {
            return Err(format!("duplicate review task outcome: {}", task.label()));
        }
        tasks.push((task, outcome));
    }
    tasks.sort_by_key(|(task, _)| *task);
    Ok(ReviewBatchOutcome::new(tasks))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTaskPhases {
    pub source_only: std::collections::BTreeSet<ReviewTask>,
    pub account_required: std::collections::BTreeSet<ReviewTask>,
}

pub fn partition_review_tasks(
    runnable: &std::collections::BTreeSet<ReviewTask>,
) -> ReviewTaskPhases {
    let mut source_only = std::collections::BTreeSet::new();
    let mut account_required = std::collections::BTreeSet::new();
    for task in runnable {
        match task.dependency() {
            ReviewTaskDependency::SourceOnly => {
                source_only.insert(*task);
            }
            ReviewTaskDependency::LegacyAccountGate
            | ReviewTaskDependency::UnclassifiedConservative => {
                account_required.insert(*task);
            }
        }
    }
    ReviewTaskPhases {
        source_only,
        account_required,
    }
}

pub fn account_dependency_outcomes(
    tasks: &std::collections::BTreeSet<ReviewTask>,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> Vec<(ReviewTask, ReviewTaskOutcome)> {
    tasks
        .iter()
        .copied()
        .map(|task| {
            (
                task,
                ReviewTaskOutcome::account_metrics_incomplete(observed_at),
            )
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskScheduleState {
    Pending,
    Terminal,
    Waiting(chrono::NaiveTime),
    DeferredUntil(chrono::DateTime<chrono::FixedOffset>),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableHydrationApplication {
    pub tasks: std::collections::BTreeSet<ReviewTask>,
    pub transition_identities: std::collections::BTreeSet<String>,
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
        self.apply_for_run(
            batch,
            ReviewRunContext {
                review_date: now.date(),
                observed_at: now,
                manual_override: false,
            },
        )
    }

    pub fn apply_for_run(
        &mut self,
        batch: &ReviewBatchOutcome,
        context: ReviewRunContext,
    ) -> Vec<ReviewTaskTransition> {
        if context.review_date() != self.date {
            return Vec::new();
        }
        let now = context.observed_at();
        let mut transitions = Vec::with_capacity(batch.tasks.len());
        for (task, outcome) in &batch.tasks {
            let next = match outcome {
                ReviewTaskOutcome::Delivered { .. }
                | ReviewTaskOutcome::NoData { .. }
                | ReviewTaskOutcome::Disabled { .. } => TaskScheduleState::Terminal,
                ReviewTaskOutcome::ExpectedWait { retry_at, .. } => {
                    TaskScheduleState::Waiting(*retry_at)
                }
                ReviewTaskOutcome::DeferredUntil { at, .. } => {
                    TaskScheduleState::DeferredUntil(*at)
                }
                ReviewTaskOutcome::Failed { failure } if !failure.retryable() => {
                    TaskScheduleState::Terminal
                }
                ReviewTaskOutcome::Failed { .. } => {
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
                TaskScheduleState::DeferredUntil(at) => (true, Some(at.to_rfc3339())),
                TaskScheduleState::Pending | TaskScheduleState::Terminal => (false, None),
            };
            let snapshot_size = match outcome {
                ReviewTaskOutcome::Delivered { count } => *count,
                _ => 0,
            };
            let (reason_code, source, source_time, transition_failure) = match outcome {
                ReviewTaskOutcome::Failed {
                    failure:
                        ReviewTaskFailure::AccountDependency(ReviewAccountDependencyFailure {
                            stage,
                            reason_code,
                            retryable,
                            source_provider,
                            source_time,
                            observed_at,
                            evidence_identity_hash,
                        }),
                } => (
                    "account_metrics_incomplete".to_string(),
                    "account_dependency_unavailable".to_string(),
                    source_time.as_ref().map(chrono::DateTime::to_rfc3339),
                    Some(ReviewTransitionFailure::AccountDependency {
                        stage: *stage,
                        reason_code: *reason_code,
                        retryable: *retryable,
                        source_provider: source_provider.clone(),
                        source_time: *source_time,
                        observed_at: *observed_at,
                        evidence_identity_hash: evidence_identity_hash.clone(),
                    }),
                ),
                _ => {
                    let reason_category = review_reason_category(*task, outcome);
                    let reason_detail = match outcome {
                        ReviewTaskOutcome::Delivered { .. } => "sink_confirmed",
                        ReviewTaskOutcome::NoData { reason }
                        | ReviewTaskOutcome::ExpectedWait { reason, .. }
                        | ReviewTaskOutcome::Disabled { reason, .. } => reason.as_str(),
                        ReviewTaskOutcome::DeferredUntil { reason, .. } => match reason {
                            ReviewDeferReasonCode::QuietHour => "quiet_hour",
                        },
                        ReviewTaskOutcome::Failed {
                            failure: ReviewTaskFailure::ExistingSourceFailure { reason, .. },
                        } => reason.as_str(),
                        ReviewTaskOutcome::Failed {
                            failure: ReviewTaskFailure::AccountDependency(_),
                        } => unreachable!("account dependency handled above"),
                    };
                    let fingerprint = audit_identity_hash("review-reason", reason_detail);
                    let failure = match outcome {
                        ReviewTaskOutcome::Failed {
                            failure: ReviewTaskFailure::ExistingSourceFailure { retryable, reason },
                        } => Some(ReviewTransitionFailure::ExistingSourceFailure {
                            retryable: *retryable,
                            reason: reason.clone(),
                        }),
                        _ => None,
                    };
                    let source = if matches!(outcome, ReviewTaskOutcome::DeferredUntil { .. }) {
                        "review_preflight_quiet_hour_policy".to_string()
                    } else {
                        task.source_label().to_string()
                    };
                    (
                        format!("{reason_category}_{}", &fingerprint[..16]),
                        source,
                        None,
                        failure,
                    )
                }
            };
            let observed_at = now.format("%Y-%m-%dT%H:%M:%S").to_string();
            let mut rule_ids = vec!["BR-110".to_string(), "BR-140".to_string()];
            if matches!(*task, ReviewTask::R04 | ReviewTask::R08 | ReviewTask::R09) {
                rule_ids.push("BR-192".to_string());
                rule_ids.push("BR-200".to_string());
            }
            if *task == ReviewTask::R08 {
                rule_ids.push("BR-199".to_string());
            }
            if matches!(
                outcome,
                ReviewTaskOutcome::Failed {
                    failure: ReviewTaskFailure::AccountDependency(_)
                }
            ) {
                rule_ids.push("BR-194".to_string());
            }
            if matches!(outcome, ReviewTaskOutcome::DeferredUntil { .. }) {
                rule_ids.push("BR-209".to_string());
            }
            let transition_defer = match outcome {
                ReviewTaskOutcome::DeferredUntil { at, reason } => Some(ReviewTransitionDefer {
                    reason_code: *reason,
                    deferred_until: *at,
                    provider_calls: 0,
                    renderer_calls: 0,
                    sink_calls: 0,
                    automatic_retry: false,
                    manual_reinvoke_required: true,
                }),
                _ => None,
            };
            transitions.push(ReviewTaskTransition {
                observed_at: observed_at.clone(),
                task: task.label().to_string(),
                source,
                // ReviewTaskOutcome currently carries report status, not a
                // provider publication timestamp. Query/as-of date is encoded
                // in the task identity; missing provider time stays absent.
                source_time,
                rule_ids,
                status: outcome.status_label().to_string(),
                success: matches!(outcome, ReviewTaskOutcome::Delivered { .. }),
                snapshot_size,
                retryable,
                next_attempt,
                reason_code,
                identity_hash: review_task_identity(self.date, *task),
                failure: transition_failure,
                defer: transition_defer,
            });
        }
        transitions
    }

    /// BR-192 applies coordinator-owned, already appended BR-140 transitions
    /// without creating a second transition. The evidence-returning variant
    /// identifies the exact current-business-date transitions the caller may
    /// acknowledge; foreign dates remain pending.
    #[cfg(test)]
    pub fn apply_durable_hydrations(
        &mut self,
        hydrations: &[stock_analysis::durable_delivery::ScheduleHydration],
    ) -> Result<std::collections::BTreeSet<ReviewTask>, String> {
        Ok(self
            .apply_durable_hydrations_with_evidence(hydrations)?
            .tasks)
    }

    pub fn apply_durable_hydrations_with_evidence(
        &mut self,
        hydrations: &[stock_analysis::durable_delivery::ScheduleHydration],
    ) -> Result<DurableHydrationApplication, String> {
        #[derive(serde::Deserialize)]
        struct DurableTaskTransition {
            schema_version: u32,
            transition_identity: String,
            task_identity: String,
            decision_identity: String,
            task_disposition: String,
            task_binding_sha256: String,
        }

        #[derive(serde::Deserialize)]
        struct DurableTaskBasis {
            task_identity: String,
            business_date: String,
            task: String,
        }

        let mut applicable = Vec::new();
        for hydration in hydrations {
            let transition: DurableTaskTransition =
                serde_json::from_slice(&hydration.transition_canonical).map_err(|error| {
                    format!(
                        "parse durable task transition {}: {error}",
                        hydration.transition_identity
                    )
                })?;
            if transition.schema_version != 1
                || transition.transition_identity != hydration.transition_identity
                || transition.decision_identity != hydration.decision_identity
                || sha256_bytes(&hydration.transition_canonical) != hydration.transition_sha256
            {
                return Err(format!(
                    "durable task transition {} identity/hash mismatch",
                    hydration.transition_identity
                ));
            }
            if transition.task_identity != hydration.task_identity
                || sha256_bytes(&hydration.transition_basis_canonical)
                    != hydration.transition_basis_sha256
                || hydration.transition_basis_sha256 != transition.task_binding_sha256
            {
                return Err(format!(
                    "durable task transition {} binding mismatch",
                    hydration.transition_identity
                ));
            }

            let basis: DurableTaskBasis =
                serde_json::from_slice(&hydration.transition_basis_canonical).map_err(|error| {
                    format!(
                        "parse durable task basis {}: {error}",
                        hydration.transition_identity
                    )
                })?;
            if basis.task_identity != transition.task_identity {
                return Err(format!(
                    "durable task transition {} task identity mismatch",
                    hydration.transition_identity
                ));
            }
            let business_date = chrono::NaiveDate::parse_from_str(&basis.business_date, "%Y-%m-%d")
                .map_err(|error| {
                    format!(
                        "parse durable task transition {} business date: {error}",
                        hydration.transition_identity
                    )
                })?;
            if business_date != self.date {
                continue;
            }
            let task = ReviewTask::from_label(&basis.task).ok_or_else(|| {
                format!(
                    "durable task transition {} has unsupported task {}",
                    hydration.transition_identity, basis.task
                )
            })?;
            if transition.task_identity != review_task_identity(self.date, task) {
                return Err(format!(
                    "durable task transition {} does not match the canonical {} identity",
                    hydration.transition_identity,
                    task.label()
                ));
            }
            if !matches!(
                transition.task_disposition.as_str(),
                "Accepted" | "Rejected" | "Uncertain" | "ManualRejected"
            ) {
                return Err(format!(
                    "durable task transition {} has unsupported disposition {}",
                    hydration.transition_identity, transition.task_disposition
                ));
            }

            applicable.push((task, hydration.transition_identity.clone()));
        }
        let mut tasks = std::collections::BTreeSet::new();
        let mut transition_identities = std::collections::BTreeSet::new();
        for (task, transition_identity) in applicable {
            self.tasks.insert(task, TaskScheduleState::Terminal);
            tasks.insert(task);
            transition_identities.insert(transition_identity);
        }
        Ok(DurableHydrationApplication {
            tasks,
            transition_identities,
        })
    }

    pub fn is_due(&self, task: ReviewTask, now: chrono::NaiveDateTime) -> bool {
        if let Some(TaskScheduleState::DeferredUntil(at)) = self.tasks.get(&task) {
            return fixed_shanghai_datetime(now) >= *at;
        }
        if now.date() != self.date {
            return false;
        }
        match self.tasks.get(&task) {
            Some(TaskScheduleState::Pending) => true,
            Some(TaskScheduleState::Waiting(retry_at)) => now.time() >= *retry_at,
            Some(TaskScheduleState::Retry { at, .. }) => now >= *at,
            Some(TaskScheduleState::DeferredUntil(_)) => unreachable!("handled above"),
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

fn fixed_shanghai_datetime(value: chrono::NaiveDateTime) -> chrono::DateTime<chrono::FixedOffset> {
    let offset = chrono::FixedOffset::east_opt(8 * 60 * 60)
        .expect("Asia/Shanghai fixed offset must be valid");
    value
        .and_local_timezone(offset)
        .single()
        .expect("fixed offsets never produce ambiguous local instants")
}

fn a10_quiet_hour_release_at(
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> chrono::DateTime<chrono::FixedOffset> {
    use chrono::Timelike;

    let date = if (2..6).contains(&observed_at.hour()) {
        observed_at.date_naive()
    } else {
        observed_at
            .date_naive()
            .succ_opt()
            .expect("review observation date must have a following day")
    };
    date.and_hms_opt(6, 0, 0)
        .expect("06:00 must be a valid wall time")
        .and_local_timezone(*observed_at.offset())
        .single()
        .expect("fixed offsets never produce ambiguous local instants")
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
    context: ReviewRunContext,
    due: &std::collections::BTreeSet<ReviewTask>,
    is_test: bool,
) -> ReviewPreflight {
    let mut runnable = due.clone();
    let mut outcomes = Vec::new();

    if is_test {
        for task in ReviewTask::ALL
            .into_iter()
            .filter(|task| task.dependency() == ReviewTaskDependency::SourceOnly)
        {
            if runnable.remove(&task) {
                outcomes.push((
                    task,
                    ReviewTaskOutcome::disabled(
                        "test_environment_external_provider_blocked",
                        "test_environment_external_provider_blocked; provider_calls=0; sink_calls=0",
                    ),
                ));
            }
        }
    }

    if runnable.contains(&ReviewTask::A10)
        && crate::v14_adapter::quiet_hour_active_at(context.observed_at().time())
    {
        runnable.remove(&ReviewTask::A10);
        let deferred_until = a10_quiet_hour_release_at(context.observed_at_fixed());
        log::info!(
            "[A-10][BR-209] status=deferred reason=quiet_hour business_date={} observed_at={} deferred_until={} provider_calls=0 renderer_calls=0 sink_calls=0 automatic_retry=false manual_reinvoke_required=true",
            context.business_date(),
            context.observed_at_fixed().to_rfc3339(),
            deferred_until.to_rfc3339()
        );
        outcomes.push((
            ReviewTask::A10,
            ReviewTaskOutcome::deferred_until(deferred_until, ReviewDeferReasonCode::QuietHour),
        ));
    }

    let disabled = [
        (
            ReviewTask::R02,
            "market_review_contract",
            "no complete review-date market overview batch (indices, turnover, and full-market breadth)",
        ),
        (
            ReviewTask::R05,
            "signal_outcome",
            "no append-only signal-delivery-execution-settlement outcome source",
        ),
        (
            ReviewTask::R06,
            "classified_failure_outcome",
            "no evidence-bound classified failure outcome source",
        ),
    ];
    for (task, capability, reason) in disabled {
        if runnable.remove(&task) {
            outcomes.push((task, ReviewTaskOutcome::disabled(capability, reason)));
        }
    }

    let lhb_ready = chrono::NaiveTime::from_hms_opt(21, 0, 0)
        .expect("BR-140 LHB publication time must be valid");
    // 2026-08-06: 手动 --review 跳过 21:00 门 (用户要求手动复盘即出明日关注/
    // 龙虎榜; 未发布数据由 dispatcher 降级 + 出声)。自动调度保持等待。
    let lhb_gate_open = context.manual_override() || context.eligibility_time() >= lhb_ready;
    if !lhb_gate_open && runnable.remove(&ReviewTask::R04) {
        outcomes.push((
            ReviewTask::R04,
            ReviewTaskOutcome::expected_wait(lhb_ready, "LHB source not published before 21:00"),
        ));
    }
    // BR-222: R-07 明日观察的龙虎榜来源同样依赖 21:00 发布, 与 R-04 同门等待。
    // 2026-08-06 回滚手动放行: R-07 是 counted ceremony (BR-140/BR-192),
    // 手动立即跑会因 LHB 未发布产生 RejectedDurable 残留 (不可变审计, 无法
    // 清理), 卡死当日 21:00 自动重试。R-07 保持 21:00 自动 (正确行为);
    // 手动 --review 只放行 R-04 (非 counted 依赖路径)。
    if context.eligibility_time() < lhb_ready && runnable.remove(&ReviewTask::R07) {
        outcomes.push((
            ReviewTask::R07,
            ReviewTaskOutcome::expected_wait(
                lhb_ready,
                "LHB source not published before 21:00 (R-07 watchlist needs it)",
            ),
        ));
    }

    if runnable.contains(&ReviewTask::R09) {
        let current_date = context.observed_at().date();
        let provider_ready = chrono::NaiveTime::from_hms_opt(15, 35, 0)
            .expect("BR-192 provider publication time must be valid");
        let outcome = if context.review_date() > current_date {
            Some(ReviewTaskOutcome::failed(
                false,
                "provider_top_n_future_date",
            ))
        } else if context.review_date() == current_date
            && context.eligibility_time() < provider_ready
        {
            Some(ReviewTaskOutcome::expected_wait(
                provider_ready,
                "Eastmoney provider Top-N is not eligible before 15:35",
            ))
        } else {
            None
        };
        if let Some(outcome) = outcome {
            runnable.remove(&ReviewTask::R09);
            outcomes.push((ReviewTask::R09, outcome));
        }
    }

    ReviewPreflight { outcomes, runnable }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 21).expect("valid test date")
    }

    fn task_hydration_for_date(
        task: ReviewTask,
        state: stock_analysis::durable_delivery::ScheduleHydrationState,
        business_date: chrono::NaiveDate,
    ) -> stock_analysis::durable_delivery::ScheduleHydration {
        let task_label = task.label().replace('-', "");
        let task_identity = review_task_identity(business_date, task);
        let decision_identity = format!("TEST_CODE_{task_label}_DECISION");
        let transition_identity = format!("TEST_CODE_{task_label}_TRANSITION");
        let transition_basis_canonical = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity.clone(),
            "business_date": business_date.format("%Y-%m-%d").to_string(),
            "task": task.label(),
            "source": task.source_label(),
            "rule_ids": ["BR-110", "BR-140", "BR-192"],
            "source_time": null,
            "snapshot_size": 40,
            "request_hashes": ["a".repeat(64), "b".repeat(64)],
            "batch_ids": ["TEST_CODE_BATCH_A", "TEST_CODE_BATCH_B"],
        }))
        .unwrap();
        let transition_basis_sha256 = sha256_bytes(&transition_basis_canonical);
        let transition_canonical = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "transition_identity": transition_identity.clone(),
            "task_identity": task_identity.clone(),
            "decision_identity": decision_identity.clone(),
            "source_identity": "TEST_CODE_SOURCE",
            "task_disposition": "Accepted",
            "task_binding_sha256": transition_basis_sha256,
            "generic_disposition_identity": "TEST_CODE_DISPOSITION",
            "generic_disposition_sha256": "c".repeat(64),
        }))
        .unwrap();
        let transition_sha256 = sha256_bytes(&transition_canonical);
        stock_analysis::durable_delivery::ScheduleHydration {
            decision_identity,
            task_identity,
            transition_identity,
            transition_canonical,
            transition_sha256,
            transition_basis_canonical,
            transition_basis_sha256,
            immutable_audit_ref: "TEST_CODE_AUDIT_REF".to_string(),
            hydration_state: state,
        }
    }

    fn task_hydration(
        task: ReviewTask,
        state: stock_analysis::durable_delivery::ScheduleHydrationState,
    ) -> stock_analysis::durable_delivery::ScheduleHydration {
        task_hydration_for_date(task, state, day())
    }

    fn r09_hydration(
        state: stock_analysis::durable_delivery::ScheduleHydrationState,
    ) -> stock_analysis::durable_delivery::ScheduleHydration {
        task_hydration(ReviewTask::R09, state)
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
    fn br212_review_completion_is_complete_for_delivered_only_batch() {
        let batch =
            ReviewBatchOutcome::new(vec![(ReviewTask::R04, ReviewTaskOutcome::delivered(5))]);

        assert_eq!(batch.completion(), ReviewCompletion::Complete);
    }

    #[test]
    fn br212_review_completion_is_partial_for_delivered_and_failed_batch() {
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::R04, ReviewTaskOutcome::delivered(5)),
            (
                ReviewTask::R08,
                ReviewTaskOutcome::failed(true, "TEST_CODE transport"),
            ),
        ]);

        assert_eq!(batch.completion(), ReviewCompletion::Partial);
    }

    #[test]
    fn br212_review_completion_is_partial_for_delivered_and_waiting_batch() {
        let retry_at = chrono::NaiveTime::from_hms_opt(21, 0, 0).unwrap();
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::R04, ReviewTaskOutcome::delivered(5)),
            (
                ReviewTask::A01,
                ReviewTaskOutcome::expected_wait(retry_at, "TEST_CODE source not published"),
            ),
        ]);

        assert_eq!(batch.completion(), ReviewCompletion::Partial);
    }

    #[test]
    fn br212_review_completion_is_partial_for_delivered_and_deferred_batch() {
        let deferred_at =
            chrono::DateTime::parse_from_rfc3339("2099-01-02T21:00:00+08:00").unwrap();
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::R04, ReviewTaskOutcome::delivered(5)),
            (
                ReviewTask::A10,
                ReviewTaskOutcome::deferred_until(deferred_at, ReviewDeferReasonCode::QuietHour),
            ),
        ]);

        assert_eq!(batch.completion(), ReviewCompletion::Partial);
    }

    #[test]
    fn br212_review_completion_keeps_no_data_and_disabled_terminal_complete() {
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::R04, ReviewTaskOutcome::delivered(5)),
            (
                ReviewTask::A01,
                ReviewTaskOutcome::no_data("TEST_CODE verified empty"),
            ),
            (
                ReviewTask::R05,
                ReviewTaskOutcome::disabled("TEST_CODE capability", "TEST_CODE unsupported"),
            ),
        ]);

        assert_eq!(batch.completion(), ReviewCompletion::Complete);
    }

    #[test]
    fn br212_review_completion_is_no_delivery_without_confirmed_delivery() {
        let batch = ReviewBatchOutcome::new(vec![(
            ReviewTask::R05,
            ReviewTaskOutcome::disabled("TEST_CODE capability", "TEST_CODE unsupported"),
        )]);

        assert_eq!(batch.completion(), ReviewCompletion::NoDelivery);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br209_a10_quiet_hour_preflight_defers_to_observation_date_0600() {
        let _guard = crate::TestEnvGuard::capture(&["STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE"]);
        std::env::remove_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE");
        let observed_date = day().succ_opt().unwrap();
        let context = ReviewRunContext {
            review_date: day(),
            observed_at: observed_date.and_hms_opt(3, 0, 0).unwrap(),
            manual_override: false,
        };
        let preflight = review_preflight(
            context,
            &std::collections::BTreeSet::from([ReviewTask::A10]),
            false,
        );

        assert!(preflight.runnable.is_empty());
        assert!(matches!(
            preflight.outcome_for(ReviewTask::A10),
            Some(ReviewTaskOutcome::DeferredUntil { at, reason })
                if at.to_rfc3339() == "2026-07-22T06:00:00+08:00"
                    && *reason == ReviewDeferReasonCode::QuietHour
        ));
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br209_a10_quiet_boundaries_and_test_isolation_are_exact() {
        let _guard = crate::TestEnvGuard::capture(&["STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE"]);
        std::env::remove_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE");
        let a10 = std::collections::BTreeSet::from([ReviewTask::A10]);
        for (hour, minute, second, deferred) in [
            (1, 59, 59, false),
            (2, 0, 0, true),
            (5, 59, 59, true),
            (6, 0, 0, false),
        ] {
            let context = ReviewRunContext {
                review_date: day(),
                observed_at: day().and_hms_opt(hour, minute, second).unwrap(),
                manual_override: false,
            };
            let preflight = review_preflight(context, &a10, false);
            assert_eq!(
                matches!(
                    preflight.outcome_for(ReviewTask::A10),
                    Some(ReviewTaskOutcome::DeferredUntil { .. })
                ),
                deferred,
                "unexpected A-10 state at {hour:02}:{minute:02}:{second:02}"
            );
        }

        std::env::set_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "1");
        let test_preflight = review_preflight(
            ReviewRunContext {
                review_date: day(),
                observed_at: at_datetime(3, 0),
                manual_override: false,
            },
            &a10,
            true,
        );
        assert!(matches!(
            test_preflight.outcome_for(ReviewTask::A10),
            Some(ReviewTaskOutcome::Disabled { .. })
        ));
        assert_eq!(test_preflight.outcomes.len(), 1);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br209_quiet_override_false_releases_and_true_uses_next_wall_date() {
        let _guard = crate::TestEnvGuard::capture(&["STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE"]);
        let a10 = std::collections::BTreeSet::from([ReviewTask::A10]);
        let context = ReviewRunContext {
            review_date: day(),
            observed_at: day().and_hms_opt(7, 0, 0).unwrap(),
            manual_override: false,
        };

        std::env::set_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "0");
        let released = review_preflight(context, &a10, false);
        assert_eq!(released.runnable, a10);
        assert!(released.outcomes.is_empty());

        std::env::set_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "1");
        let forced = review_preflight(context, &a10, false);
        assert!(matches!(
            forced.outcome_for(ReviewTask::A10),
            Some(ReviewTaskOutcome::DeferredUntil { at, .. })
                if at.to_rfc3339() == "2026-07-22T06:00:00+08:00"
        ));
    }

    #[test]
    fn br209_deferred_transition_uses_absolute_wall_clock_and_rule_evidence() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-07-22T06:00:00+08:00").unwrap();
        let batch = ReviewBatchOutcome::new(vec![(
            ReviewTask::A10,
            ReviewTaskOutcome::deferred_until(at, ReviewDeferReasonCode::QuietHour),
        )]);
        let mut state = ReviewScheduleState::for_date(day());
        let transition = state
            .apply_for_run(
                &batch,
                ReviewRunContext {
                    review_date: day(),
                    observed_at: day().succ_opt().unwrap().and_hms_opt(3, 0, 0).unwrap(),
                    manual_override: false,
                },
            )
            .pop()
            .unwrap();

        assert_eq!(batch.waiting_tasks(), Vec::<ReviewTask>::new());
        assert_eq!(batch.deferred_tasks(), vec![ReviewTask::A10]);
        assert_eq!(transition.status, "deferred");
        assert_eq!(transition.source, "review_preflight_quiet_hour_policy");
        assert_eq!(
            transition.next_attempt.as_deref(),
            Some("2026-07-22T06:00:00+08:00")
        );
        assert!(transition.retryable);
        assert!(!transition.success);
        assert_eq!(transition.failure, None);
        assert_eq!(
            transition.defer,
            Some(ReviewTransitionDefer {
                reason_code: ReviewDeferReasonCode::QuietHour,
                deferred_until: at,
                provider_calls: 0,
                renderer_calls: 0,
                sink_calls: 0,
                automatic_retry: false,
                manual_reinvoke_required: true,
            })
        );
        assert!(transition.rule_ids.contains(&"BR-209".to_string()));
        assert!(!state.is_due(
            ReviewTask::A10,
            day().succ_opt().unwrap().and_hms_opt(5, 59, 59).unwrap()
        ));
        assert!(state.is_due(
            ReviewTask::A10,
            day().succ_opt().unwrap().and_hms_opt(6, 0, 0).unwrap()
        ));
    }

    #[test]
    fn br209_deferred_transition_wire_rejects_missing_or_contradictory_evidence() {
        let valid = serde_json::json!({
            "observed_at": "2026-07-22T03:00:00",
            "task": "A-10",
            "source": "review_preflight_quiet_hour_policy",
            "source_time": null,
            "rule_ids": ["BR-110", "BR-140", "BR-209"],
            "status": "deferred",
            "success": false,
            "snapshot_size": 0,
            "retryable": true,
            "next_attempt": "2026-07-22T06:00:00+08:00",
            "reason_code": "push_governance_deferred_TEST_CODE",
            "identity_hash": "TEST_CODE_IDENTITY",
            "defer": {
                "reason_code": "quiet_hour",
                "deferred_until": "2026-07-22T06:00:00+08:00",
                "provider_calls": 0,
                "renderer_calls": 0,
                "sink_calls": 0,
                "automatic_retry": false,
                "manual_reinvoke_required": true
            }
        });
        assert!(serde_json::from_value::<ReviewTaskTransition>(valid.clone()).is_ok());

        let mut missing_defer = valid.clone();
        missing_defer.as_object_mut().unwrap().remove("defer");
        assert!(serde_json::from_value::<ReviewTaskTransition>(missing_defer).is_err());

        let mut bad_success = valid.clone();
        bad_success["success"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReviewTaskTransition>(bad_success).is_err());
        let mut bad_calls = valid.clone();
        bad_calls["defer"]["provider_calls"] = serde_json::json!(1);
        assert!(serde_json::from_value::<ReviewTaskTransition>(bad_calls).is_err());
        let mut bad_attempt = valid;
        bad_attempt["next_attempt"] = serde_json::json!("2026-07-22T06:01:00+08:00");
        assert!(serde_json::from_value::<ReviewTaskTransition>(bad_attempt).is_err());
    }

    #[test]
    fn br209_deferred_transition_rejects_noncanonical_release_instants() {
        let valid = serde_json::json!({
            "observed_at": "2026-07-22T03:00:00",
            "task": "A-10",
            "source": "review_preflight_quiet_hour_policy",
            "source_time": null,
            "rule_ids": ["BR-110", "BR-140", "BR-209"],
            "status": "deferred",
            "success": false,
            "snapshot_size": 0,
            "retryable": true,
            "next_attempt": "2026-07-22T06:00:00+08:00",
            "reason_code": "push_governance_deferred_TEST_CODE",
            "identity_hash": "TEST_CODE_IDENTITY",
            "defer": {
                "reason_code": "quiet_hour",
                "deferred_until": "2026-07-22T06:00:00+08:00",
                "provider_calls": 0,
                "renderer_calls": 0,
                "sink_calls": 0,
                "automatic_retry": false,
                "manual_reinvoke_required": true
            }
        });

        for release in [
            "2026-07-22T07:30:00+09:00",
            "2026-07-22T06:00:00+09:00",
            "2026-07-22T06:01:00+08:00",
            "2026-07-21T06:00:00+08:00",
            "2026-07-23T06:00:00+08:00",
        ] {
            let mut invalid = valid.clone();
            invalid["defer"]["deferred_until"] = serde_json::json!(release);
            invalid["next_attempt"] = serde_json::json!(release);
            assert!(
                serde_json::from_value::<ReviewTaskTransition>(invalid).is_err(),
                "BR-209 admitted noncanonical release instant {release}"
            );
        }

        let mut malformed_observation = valid;
        malformed_observation["observed_at"] = serde_json::json!("2026-07-22T03:00:00+08:00");
        assert!(serde_json::from_value::<ReviewTaskTransition>(malformed_observation).is_err());
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
                failure: ReviewTaskFailure::ExistingSourceFailure {
                    retryable: false,
                    ..
                },
            }
        ));
        assert!(matches!(
            ReviewTaskOutcome::from_push_outcome(
                crate::notify::PushOutcome::Denied("policy".to_string()),
                2
            ),
            ReviewTaskOutcome::Failed {
                failure: ReviewTaskFailure::ExistingSourceFailure {
                    retryable: false,
                    ..
                },
            }
        ));
        assert!(matches!(
            ReviewTaskOutcome::from_push_outcome(
                crate::notify::PushOutcome::Denied("quiet_hour".to_string()),
                2
            ),
            ReviewTaskOutcome::Failed {
                failure: ReviewTaskFailure::ExistingSourceFailure {
                    retryable: true,
                    ref reason,
                },
            } if reason.contains("quiet_hour")
        ));
        assert!(matches!(
            ReviewTaskOutcome::from_push_outcome(
                crate::notify::PushOutcome::SinkError("transport".to_string()),
                2
            ),
            ReviewTaskOutcome::Failed {
                failure: ReviewTaskFailure::ExistingSourceFailure {
                    retryable: true,
                    ..
                },
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
    fn br140_manual_review_audit_keeps_business_date_and_real_observation_time_separate() {
        let observed_at = chrono::NaiveDate::from_ymd_opt(2026, 7, 25)
            .expect("valid Saturday")
            .and_hms_opt(2, 42, 50)
            .expect("valid observation time");
        let context = ReviewRunContext::at(observed_at);
        assert_eq!(
            context.business_date(),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 24).unwrap()
        );
        let mut state = ReviewScheduleState::for_date(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 24).expect("known completed Friday"),
        );

        let transitions = state.apply_for_run(
            &ReviewBatchOutcome::new(vec![(
                ReviewTask::A01,
                ReviewTaskOutcome::no_data("complete source empty"),
            )]),
            context,
        );

        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].observed_at, "2026-07-25T02:42:50");
        assert_eq!(
            transitions[0].identity_hash,
            audit_identity_hash("review-task", "2026-07-24:A-01")
        );
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
            ReviewRunContext {
                review_date: day(),
                observed_at: at_datetime(19, 0),
                manual_override: false,
            },
            &due,
            false,
        );

        assert_eq!(
            preflight.outcome_for(ReviewTask::R02),
            Some(&ReviewTaskOutcome::disabled(
                "market_review_contract",
                "no complete review-date market overview batch (indices, turnover, and full-market breadth)",
            ))
        );
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R04),
            Some(ReviewTaskOutcome::ExpectedWait { retry_at, .. })
                if *retry_at == chrono::NaiveTime::from_hms_opt(21, 0, 0).expect("valid wait time")
        ));
        assert_eq!(
            preflight.outcome_for(ReviewTask::R05),
            Some(&ReviewTaskOutcome::disabled(
                "signal_outcome",
                "no append-only signal-delivery-execution-settlement outcome source",
            ))
        );
        assert_eq!(
            preflight.outcome_for(ReviewTask::R06),
            Some(&ReviewTaskOutcome::disabled(
                "classified_failure_outcome",
                "no evidence-bound classified failure outcome source",
            ))
        );
        assert!(!preflight.runnable.contains(&ReviewTask::R02));
        assert!(!preflight.runnable.contains(&ReviewTask::R04));
        assert!(preflight.runnable.contains(&ReviewTask::R09));
        assert!(preflight.runnable.contains(&ReviewTask::A01));
    }

    #[test]
    fn br194_review_task_dependency_mapping() {
        assert_eq!(
            ReviewTask::R04.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(
            ReviewTask::R07.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(
            ReviewTask::R09.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(
            ReviewTask::R11.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(
            ReviewTask::R03.dependency(),
            ReviewTaskDependency::LegacyAccountGate
        );
        assert_eq!(
            ReviewTask::R08.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(
            ReviewTask::A10.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(
            ReviewTask::A01.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        for task in [ReviewTask::R02, ReviewTask::R05, ReviewTask::R06] {
            assert_eq!(
                task.dependency(),
                ReviewTaskDependency::UnclassifiedConservative
            );
        }
    }

    #[test]
    fn br194_account_tasks_are_frozen_without_real_batch_watermark() {
        let observed_at =
            chrono::DateTime::parse_from_rfc3339("2026-07-21T19:00:00+08:00").unwrap();
        let outcome = ReviewTaskOutcome::account_metrics_incomplete(observed_at);

        assert_eq!(
            outcome,
            ReviewTaskOutcome::Failed {
                failure: ReviewTaskFailure::AccountDependency(ReviewAccountDependencyFailure {
                    stage: ReviewAccountDependencyStage::AcquireBatch,
                    reason_code: ReviewAccountFailureReasonCode::AccountMetricsIncomplete,
                    retryable: true,
                    source_provider: None,
                    source_time: None,
                    observed_at,
                    evidence_identity_hash: None,
                })
            }
        );
    }

    #[test]
    fn br194_account_failure_serializes_exact_transition_audit() {
        let observed_at =
            chrono::DateTime::parse_from_rfc3339("2026-07-21T19:00:00+08:00").unwrap();
        let mut state = ReviewScheduleState::for_date(day());
        let transition = state
            .apply(
                &ReviewBatchOutcome::new(vec![(
                    ReviewTask::R03,
                    ReviewTaskOutcome::account_metrics_incomplete(observed_at),
                )]),
                at_datetime(19, 0),
            )
            .pop()
            .unwrap();
        let value = serde_json::to_value(&transition).unwrap();

        assert_eq!(transition.source, "account_dependency_unavailable");
        assert!(transition.rule_ids.contains(&"BR-194".to_string()));
        assert_eq!(transition.reason_code, "account_metrics_incomplete");
        assert!(transition.retryable);
        assert_eq!(transition.source_time, None);
        assert_eq!(
            value.get("failure"),
            Some(&serde_json::json!({
                "failure_class": "account_dependency",
                "stage": "acquire_batch",
                "reason_code": "account_metrics_incomplete",
                "retryable": true,
                "source_provider": null,
                "source_time": null,
                "observed_at": "2026-07-21T19:00:00+08:00",
                "evidence_identity_hash": null
            }))
        );
    }

    #[test]
    fn br194_legacy_transition_fixture_remains_byte_identical_and_hash_valid() {
        let legacy = br#"{"payload":{"event_type":"task_transition","observed_at":"2026-07-21T19:00:00","task":"R-03","source":"portfolio_industry_kline","source_time":null,"rule_ids":["BR-110","BR-140"],"status":"failed","success":false,"snapshot_size":0,"retryable":true,"next_attempt":"2026-07-21T19:01:00","reason_code":"industry_evidence_unavailable_0123456789abcdef","identity_hash":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"prev_hash":"0000000000000000000000000000000000000000000000000000000000000000","record_hash":"a7fb48219edf03c54020b216f4508e28dedded154ee0453a0121f3cd7b12c4a8"}"#;
        let parsed: ReviewAuditRecord = serde_json::from_slice(legacy).unwrap();

        let ReviewAuditPayload::TaskTransition(transition) = &parsed.payload else {
            panic!("legacy fixture must remain a task transition");
        };
        assert_eq!(transition.failure, None);
        assert_eq!(
            review_audit_hash(&parsed.prev_hash, &parsed.payload).unwrap(),
            parsed.record_hash
        );
        assert_eq!(serde_json::to_vec(&parsed).unwrap(), legacy);
    }

    #[test]
    fn br209_exact_first_live_probe_record_remains_byte_and_hash_valid_only() {
        let parsed: ReviewAuditRecord =
            serde_json::from_slice(BR209_LEGACY_UNTYPED_DEFER_RECORD).unwrap();
        assert_eq!(
            review_audit_hash(&parsed.prev_hash, &parsed.payload).unwrap(),
            parsed.record_hash
        );
        assert_eq!(
            serde_json::to_vec(&parsed).unwrap(),
            BR209_LEGACY_UNTYPED_DEFER_RECORD
        );

        let mut altered: serde_json::Value =
            serde_json::from_slice(BR209_LEGACY_UNTYPED_DEFER_RECORD).unwrap();
        altered["record_hash"] = serde_json::json!("TEST_CODE_CHANGED_HASH");
        assert!(serde_json::from_value::<ReviewAuditRecord>(altered).is_err());
    }

    #[test]
    fn br209_append_validation_rejects_directly_constructed_invalid_defer() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-07-22T06:00:00+08:00").unwrap();
        let mut state = ReviewScheduleState::for_date(day());
        let mut transition = state
            .apply_for_run(
                &ReviewBatchOutcome::new(vec![(
                    ReviewTask::A10,
                    ReviewTaskOutcome::deferred_until(at, ReviewDeferReasonCode::QuietHour),
                )]),
                ReviewRunContext {
                    review_date: day(),
                    observed_at: day().succ_opt().unwrap().and_hms_opt(3, 0, 0).unwrap(),
                    manual_override: false,
                },
            )
            .pop()
            .unwrap();
        transition.success = true;

        assert!(
            validate_review_audit_payload_for_append(&ReviewAuditPayload::TaskTransition(
                transition
            ))
            .is_err()
        );
    }

    #[test]
    fn br194_account_failure_full_record_fixture_is_fixed_and_hash_valid() {
        let fixture = br#"{"payload":{"event_type":"task_transition","observed_at":"2026-07-21T19:00:00","task":"R-03","source":"account_dependency_unavailable","source_time":null,"rule_ids":["BR-110","BR-140","BR-194"],"status":"failed","success":false,"snapshot_size":0,"retryable":true,"next_attempt":"2026-07-21T19:01:00","reason_code":"account_metrics_incomplete","identity_hash":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","failure":{"failure_class":"account_dependency","stage":"acquire_batch","reason_code":"account_metrics_incomplete","retryable":true,"source_provider":null,"source_time":null,"observed_at":"2026-07-21T19:00:00+08:00","evidence_identity_hash":null}},"prev_hash":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","record_hash":"d2a8c7cda75c86c85fe3745bd14c8479ba3b8a621d494b8c9d17b69ab76c138b"}"#;
        let parsed: ReviewAuditRecord = serde_json::from_slice(fixture).unwrap();

        assert_eq!(
            review_audit_hash(&parsed.prev_hash, &parsed.payload).unwrap(),
            parsed.record_hash
        );
        assert_eq!(serde_json::to_vec(&parsed).unwrap(), fixture);
    }

    #[test]
    fn br194_transition_failure_wire_rejects_null_array_unknown_and_nonfailed_payloads() {
        let prefix = r#"{"observed_at":"2026-07-21T19:00:00","task":"R-03","source":"account_dependency_unavailable","source_time":null,"rule_ids":["BR-194"],"status":"failed","success":false,"snapshot_size":0,"retryable":true,"next_attempt":"2026-07-21T19:01:00","reason_code":"account_metrics_incomplete","identity_hash":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789""#;
        for suffix in [
            r#","failure":null}"#,
            r#","failure":[]}"#,
            r#","failure":{"failure_class":"TEST_CODE_unknown"}}"#,
            r#","TEST_CODE_unknown_field":true}"#,
        ] {
            let wire = format!("{prefix}{suffix}");
            assert!(
                serde_json::from_str::<ReviewTaskTransition>(&wire).is_err(),
                "invalid fixed wire must fail closed: {suffix}"
            );
        }

        let nonfailed = format!(
            "{}{}",
            prefix.replace(r#""status":"failed""#, r#""status":"no_data""#),
            r#","failure":{"failure_class":"existing_source_failure","retryable":false,"reason":"TEST_CODE_REASON"}}"#
        );
        assert!(serde_json::from_str::<ReviewTaskTransition>(&nonfailed).is_err());
    }

    #[test]
    fn br194_preflight_precedes_dependency_acquisition() {
        let due = ReviewTask::ALL.into_iter().collect();
        let context = ReviewRunContext {
            review_date: day(),
            observed_at: at_datetime(15, 0),
            manual_override: false,
        };
        let preflight = review_preflight(context, &due, true);

        assert_eq!(
            preflight
                .outcomes
                .iter()
                .map(|(task, _)| *task)
                .collect::<Vec<_>>(),
            vec![
                ReviewTask::R04,
                ReviewTask::R07,
                ReviewTask::R08,
                ReviewTask::R09,
                ReviewTask::R11,
                ReviewTask::R12,
                ReviewTask::R13,
                ReviewTask::A10,
                ReviewTask::A01,
                ReviewTask::R02,
                ReviewTask::R05,
                ReviewTask::R06,
            ],
            "test/live isolation must run before static capability disabling"
        );
        for task in [
            ReviewTask::R04,
            ReviewTask::R07,
            ReviewTask::R08,
            ReviewTask::R09,
            ReviewTask::R11,
            ReviewTask::R12,
            ReviewTask::R13,
            ReviewTask::A10,
            ReviewTask::A01,
        ] {
            assert_eq!(
                preflight.outcome_for(task),
                Some(&ReviewTaskOutcome::disabled(
                    "test_environment_external_provider_blocked",
                    "test_environment_external_provider_blocked; provider_calls=0; sink_calls=0",
                ))
            );
            assert!(!preflight.runnable.contains(&task));
        }
        for task in [ReviewTask::R02, ReviewTask::R05, ReviewTask::R06] {
            assert!(matches!(
                preflight.outcome_for(task),
                Some(ReviewTaskOutcome::Disabled { .. })
            ));
        }
    }

    #[test]
    fn br194_time_boundaries_1535_and_2100() {
        let r09 = std::collections::BTreeSet::from([ReviewTask::R09]);
        let r04 = std::collections::BTreeSet::from([ReviewTask::R04]);
        let context = |hour, minute, second| ReviewRunContext {
            review_date: day(),
            observed_at: day()
                .and_hms_opt(hour, minute, second)
                .expect("valid TEST_CODE review time"),
            manual_override: false,
        };

        assert!(matches!(
            review_preflight(context(15, 34, 59), &r09, false).outcome_for(ReviewTask::R09),
            Some(ReviewTaskOutcome::ExpectedWait { .. })
        ));
        assert!(review_preflight(context(15, 35, 0), &r09, false)
            .runnable
            .contains(&ReviewTask::R09));
        assert!(matches!(
            review_preflight(context(20, 59, 59), &r04, false).outcome_for(ReviewTask::R04),
            Some(ReviewTaskOutcome::ExpectedWait { .. })
        ));
        assert!(review_preflight(context(21, 0, 0), &r04, false)
            .runnable
            .contains(&ReviewTask::R04));
    }

    #[test]
    fn br194_review_batch_merge_rejects_duplicate_task() {
        let duplicate = merge_review_task_outcomes(
            vec![(
                ReviewTask::R04,
                ReviewTaskOutcome::no_data("complete provider empty"),
            )],
            vec![(ReviewTask::R04, ReviewTaskOutcome::delivered(5))],
            Vec::new(),
        )
        .unwrap_err();
        assert_eq!(duplicate, "duplicate review task outcome: R-04");

        let merged = merge_review_task_outcomes(
            vec![(
                ReviewTask::R02,
                ReviewTaskOutcome::disabled("missing", "missing"),
            )],
            vec![(ReviewTask::R09, ReviewTaskOutcome::delivered(40))],
            vec![(
                ReviewTask::R03,
                ReviewTaskOutcome::account_metrics_incomplete(
                    chrono::DateTime::parse_from_rfc3339("2026-07-21T19:00:00+08:00").unwrap(),
                ),
            )],
        )
        .unwrap();
        assert_eq!(
            merged
                .tasks
                .iter()
                .map(|(task, _)| *task)
                .collect::<Vec<_>>(),
            vec![ReviewTask::R02, ReviewTask::R03, ReviewTask::R09]
        );
    }

    #[test]
    fn br194_source_only_runs_before_frozen_account_tasks() {
        let phases = partition_review_tasks(&std::collections::BTreeSet::from([
            ReviewTask::R03,
            ReviewTask::R04,
            ReviewTask::R07,
            ReviewTask::R08,
            ReviewTask::R09,
            ReviewTask::R11,
            ReviewTask::A10,
            ReviewTask::A01,
        ]));
        assert_eq!(
            phases.source_only,
            std::collections::BTreeSet::from([
                ReviewTask::R04,
                ReviewTask::R07,
                ReviewTask::R08,
                ReviewTask::R09,
                ReviewTask::R11,
                ReviewTask::A10,
                ReviewTask::A01,
            ])
        );
        assert_eq!(
            phases.account_required,
            std::collections::BTreeSet::from([ReviewTask::R03])
        );

        let delivered = (ReviewTask::R09, ReviewTaskOutcome::delivered(40));
        let observed_at =
            chrono::DateTime::parse_from_rfc3339("2026-07-21T19:00:00+08:00").unwrap();
        let merged = merge_review_task_outcomes(
            Vec::new(),
            vec![delivered],
            account_dependency_outcomes(&phases.account_required, observed_at),
        )
        .unwrap();
        assert!(matches!(
            merged
                .tasks
                .iter()
                .find(|(task, _)| *task == ReviewTask::R09),
            Some((_, ReviewTaskOutcome::Delivered { count: 40 }))
        ));
    }

    #[test]
    fn br192_r09_catalog_identity_and_audit_source_are_stable() {
        assert!(ReviewTask::ALL.contains(&ReviewTask::R09));
        assert_eq!(ReviewTask::ALL.len(), 13);
        assert!(ReviewTask::ALL.contains(&ReviewTask::R07));
        assert!(ReviewTask::ALL.contains(&ReviewTask::R11));
        assert!(ReviewTask::ALL.contains(&ReviewTask::R12));
        assert!(ReviewTask::ALL.contains(&ReviewTask::R13));
        assert_eq!(ReviewTask::R12.label(), "R-12");
        assert_eq!(ReviewTask::R12.source_label(), "paper_trades_15min_backtest");
        assert_eq!(
            ReviewTask::R12.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(ReviewTask::R13.label(), "R-13");
        assert_eq!(
            ReviewTask::R13.source_label(),
            "catalyst_watchlist_t1_tracking"
        );
        assert_eq!(
            ReviewTask::R13.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        assert_eq!(ReviewTask::R09.label(), "R-09");
        assert_eq!(ReviewTask::R09.source_label(), "eastmoney_provider_top_n");
        assert_eq!(
            review_task_identity(day(), ReviewTask::R09),
            audit_identity_hash("review-task", "2026-07-21:R-09")
        );

        let mut state = ReviewScheduleState::for_date(day());
        let transitions = state.apply(
            &ReviewBatchOutcome::new(vec![(
                ReviewTask::R09,
                ReviewTaskOutcome::disabled("durable_delivery_producer", "disabled=no_producer"),
            )]),
            at_datetime(19, 0),
        );
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].task, "R-09");
        assert_eq!(transitions[0].source, "eastmoney_provider_top_n");
        assert!(transitions[0].rule_ids.contains(&"BR-192".to_string()));
        assert_eq!(
            transitions[0].identity_hash,
            review_task_identity(day(), ReviewTask::R09)
        );
    }

    #[test]
    fn br199_r08_transition_uses_only_public_component_source_and_rules() {
        assert_eq!(
            ReviewTask::R08.source_label(),
            "event_calendar_public_component_batches"
        );
        let mut state = ReviewScheduleState::for_date(day());
        let transitions = state.apply(
            &ReviewBatchOutcome::new(vec![(
                ReviewTask::R08,
                ReviewTaskOutcome::failed(true, "TEST_CODE_CFFEX_UNAVAILABLE"),
            )]),
            at_datetime(19, 0),
        );

        assert_eq!(transitions.len(), 1);
        let transition = &transitions[0];
        assert_eq!(transition.source, "event_calendar_public_component_batches");
        for rule in ["BR-192", "BR-199", "BR-200"] {
            assert!(transition.rule_ids.contains(&rule.to_string()));
        }
        assert!(!transition.source.contains("position"));
        assert!(!transition.source.contains("virtual"));
        assert!(!transition.source.contains("account"));
    }

    #[test]
    fn br192_durable_r09_hydration_is_terminal_without_a_second_transition() {
        let mut state = ReviewScheduleState::for_date(day());
        let applied = state
            .apply_durable_hydrations(&[r09_hydration(
                stock_analysis::durable_delivery::ScheduleHydrationState::Pending,
            )])
            .unwrap();
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::R09, ReviewTaskOutcome::delivered(40)),
            (
                ReviewTask::A01,
                ReviewTaskOutcome::no_data("complete empty"),
            ),
        ]);
        let legacy = batch.without_tasks(&applied);
        let transitions = state.apply(&legacy, at_datetime(19, 0));

        assert_eq!(applied, std::collections::BTreeSet::from([ReviewTask::R09]));
        assert!(!state.is_due(ReviewTask::R09, at_datetime(19, 1)));
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].task, "A-01");
    }

    #[test]
    fn br192_applied_hydration_reconstructs_terminal_r09_after_restart() {
        let mut restarted = ReviewScheduleState::for_date(day());
        let applied = restarted
            .apply_durable_hydrations(&[r09_hydration(
                stock_analysis::durable_delivery::ScheduleHydrationState::Applied,
            )])
            .unwrap();

        assert!(applied.contains(&ReviewTask::R09));
        assert!(!restarted.is_due(ReviewTask::R09, at_datetime(23, 0)));
    }

    #[test]
    fn br192_durable_hydration_maps_all_registered_review_task_labels() {
        let mut state = ReviewScheduleState::for_date(day());
        let hydrations = ReviewTask::ALL
            .into_iter()
            .map(|task| {
                task_hydration(
                    task,
                    stock_analysis::durable_delivery::ScheduleHydrationState::Pending,
                )
            })
            .collect::<Vec<_>>();

        let applied = state.apply_durable_hydrations(&hydrations).unwrap();

        assert_eq!(
            applied,
            ReviewTask::ALL
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
        for task in ReviewTask::ALL {
            assert!(!state.is_due(task, at_datetime(23, 0)));
        }
    }

    #[test]
    fn br192_hydration_application_returns_only_current_date_transition_identities() {
        let current =
            r09_hydration(stock_analysis::durable_delivery::ScheduleHydrationState::Pending);
        let foreign = task_hydration_for_date(
            ReviewTask::A01,
            stock_analysis::durable_delivery::ScheduleHydrationState::Pending,
            day().succ_opt().unwrap(),
        );
        let mut state = ReviewScheduleState::for_date(day());

        let application = state
            .apply_durable_hydrations_with_evidence(&[current.clone(), foreign.clone()])
            .expect("apply current business date only");

        assert_eq!(
            application.tasks,
            std::collections::BTreeSet::from([ReviewTask::R09])
        );
        assert_eq!(
            application.transition_identities,
            std::collections::BTreeSet::from([current.transition_identity])
        );
        assert!(!application
            .transition_identities
            .contains(&foreign.transition_identity));
    }

    #[test]
    fn br192_r09_preflight_waits_until_1535_without_making_it_runnable() {
        let due = std::collections::BTreeSet::from([ReviewTask::R09]);
        let context = ReviewRunContext {
            review_date: day(),
            observed_at: at_datetime(15, 34),
            manual_override: false,
        };

        let preflight = review_preflight(context, &due, false);

        assert!(!preflight.runnable.contains(&ReviewTask::R09));
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R09),
            Some(ReviewTaskOutcome::ExpectedWait { retry_at, .. })
                if *retry_at == chrono::NaiveTime::from_hms_opt(15, 35, 0).unwrap()
        ));
    }

    #[test]
    fn br198_r09_closed_day_uses_prior_review_trading_date() {
        let due = std::collections::BTreeSet::from([ReviewTask::R09]);
        let saturday = chrono::NaiveDate::from_ymd_opt(2026, 8, 1)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap();
        let context = ReviewRunContext {
            review_date: chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            observed_at: saturday,
            manual_override: false,
        };

        let preflight = review_preflight(context, &due, false);

        assert_eq!(preflight.runnable, due);
        assert!(preflight.outcome_for(ReviewTask::R09).is_none());
    }

    #[test]
    fn br198_r09_future_review_date_fails_nonretryable_before_provider() {
        let due = std::collections::BTreeSet::from([ReviewTask::R09]);
        let context = ReviewRunContext {
            review_date: day().succ_opt().unwrap(),
            observed_at: at_datetime(19, 0),
            manual_override: false,
        };

        let preflight = review_preflight(context, &due, false);

        assert!(!preflight.runnable.contains(&ReviewTask::R09));
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R09),
            Some(ReviewTaskOutcome::Failed {
                failure: ReviewTaskFailure::ExistingSourceFailure {
                    retryable: false,
                    reason,
                },
            }) if reason == "provider_top_n_future_date"
        ));
    }

    #[test]
    fn br192_r09_test_mode_is_disabled_before_provider_eligibility() {
        let due = std::collections::BTreeSet::from([ReviewTask::R09]);
        let context = ReviewRunContext {
            review_date: day(),
            observed_at: at_datetime(19, 0),
            manual_override: false,
        };

        let preflight = review_preflight(context, &due, true);

        assert!(!preflight.runnable.contains(&ReviewTask::R09));
        assert!(matches!(
            preflight.outcome_for(ReviewTask::R09),
            Some(ReviewTaskOutcome::Disabled { capability, reason })
                if capability == "test_environment_external_provider_blocked"
                    && reason.contains("provider_calls=0")
        ));
    }

    #[test]
    fn every_non_delivered_task_reports_its_raw_cause() {
        let observed_at = chrono::Local::now().fixed_offset();
        let batch = ReviewBatchOutcome::new(vec![
            (ReviewTask::R09, ReviewTaskOutcome::delivered(40)),
            (
                ReviewTask::R04,
                ReviewTaskOutcome::failed(
                    false,
                    "durable R-04 delivery 4d38409a already rejected state=RejectedDurable",
                ),
            ),
            (
                ReviewTask::R03,
                ReviewTaskOutcome::Failed {
                    failure: ReviewTaskFailure::AccountDependency(ReviewAccountDependencyFailure {
                        stage: ReviewAccountDependencyStage::AcquireBatch,
                        reason_code: ReviewAccountFailureReasonCode::AccountMetricsIncomplete,
                        retryable: true,
                        source_provider: None,
                        source_time: None,
                        observed_at,
                        evidence_identity_hash: None,
                    }),
                },
            ),
            (
                ReviewTask::R05,
                ReviewTaskOutcome::disabled(
                    "signal_outcome_contract",
                    "selection_v2_activation_not_released",
                ),
            ),
            (
                ReviewTask::A01,
                ReviewTaskOutcome::NoData {
                    reason: "virtual_observation 无 T+1 记录".to_string(),
                },
            ),
        ]);

        let diagnostics = batch.non_delivered_diagnostics();
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.task)
                .collect::<Vec<_>>(),
            vec![
                ReviewTask::R04,
                ReviewTask::R03,
                ReviewTask::R05,
                ReviewTask::A01
            ],
            "delivered tasks are omitted, every other task is reported"
        );
        for diagnostic in &diagnostics {
            assert!(
                !diagnostic.detail.trim().is_empty(),
                "{:?} must carry a raw cause, not just a hashed category",
                diagnostic.task
            );
            assert_ne!(diagnostic.detail, "unspecified");
        }

        let r04 = &diagnostics[0];
        assert_eq!(r04.status, "failed");
        assert_eq!(r04.retryable, Some(false));
        assert!(
            r04.detail.contains("RejectedDurable"),
            "R-04 detail must expose the durable terminal state, got {}",
            r04.detail
        );

        let r03 = &diagnostics[1];
        assert_eq!(r03.reason_code, "account_metrics_incomplete");
        assert_eq!(r03.retryable, Some(true));
        assert!(r03.detail.contains("AcquireBatch"));

        let r05 = &diagnostics[2];
        assert_eq!(r05.status, "disabled");
        assert_eq!(r05.retryable, None);
        assert!(r05.detail.contains("selection_v2_activation_not_released"));

        assert_eq!(diagnostics[3].status, "no_data");
    }

    #[test]
    fn br199_r08_is_source_only_and_partitions_before_account_gate() {
        assert_eq!(
            ReviewTask::R08.dependency(),
            ReviewTaskDependency::SourceOnly
        );
        let runnable = std::collections::BTreeSet::from([ReviewTask::R08, ReviewTask::R03]);

        let phases = partition_review_tasks(&runnable);

        assert_eq!(
            phases.source_only,
            std::collections::BTreeSet::from([ReviewTask::R08])
        );
        // BR-194: R-03 stays behind the account gate; BR-199 only moved R-08.
        assert_eq!(
            phases.account_required,
            std::collections::BTreeSet::from([ReviewTask::R03])
        );
    }
}
