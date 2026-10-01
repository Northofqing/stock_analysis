//! G5b 深链归因 — AI 深链版异动归因 (盘后批量, 2026-08-22 落地)。
//!
//! Registered business rules: BR-045, BR-181.
//! 设计: monitor/attribution.rs (G5a) 注释「AI 深链归 G5b (盘后/手动)」。
//!
//! G5a = 盘中规则快归因 (P95 ≤ 2s, 不用 LLM); G5b = 盘后 LLM 深链归因:
//! - 输入: 当日 alert_log JSONL 的 AlertRecord (与 G5a 同源事件)
//! - LLM: 复用 LlmRegistry 模型通道 (DeepSeek 优先), 45s 超时 + receipt 保真
//! - 输出: strict JSON {main_reason, catalyst_chain, capital_logic, confidence, risk_note}
//! - 落库: data/g5b/{date}.jsonl (含 receipt), 失败出声不静默
//! - 消费: 15:05 归因闭环追加深链段 + PushKind::G5bAttribution 独立推送
//!
//! 与 G5a 的关系: G5a 的 attribution_decision 作为输入上下文喂给 LLM,
//! 深链验证/深化规则结论, 不是取代。

use crate::durable_delivery::{
    DurableDeliveryCoordinator, DurableDeliveryError, G5bCountedTerminalV1,
};
use crate::llm::{LlmError, LlmProvider, ModelCallReceipt, ReceiptBearingJson};
use crate::monitor::alert_log::AlertRecord;
use crate::risk::env_guard::{current_env, runtime_is_test_process, TradingEnv};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 深链归因模型调用超时 (与 news_ai 同档, 45s)。
const MODEL_CALL_TIMEOUT_SECONDS: u64 = 45;
/// 单日深链事件上限 (模型调用成本护栏, 优先级取前 N)。
pub const DEEP_ATTRIBUTION_MAX_EVENTS: usize = 3;

/// G5b 系统 prompt v1: 深链归因角色 + strict JSON schema。
pub const G5B_SYSTEM_PROMPT_V1: &str = r#"你是 A 股异动深链归因分析师 (虚拟盘研究, 非投资建议)。
输入: 当日异动告警记录 (含 G5a 规则快归因结论)。
任务: 用你的市场知识深链分析异动根因, 输出 strict JSON (无 markdown 围栏, 无多余文字):

{
  "main_reason": "一句话主因, ≤40 字",
  "catalyst_chain": ["链上证据 1", "链上证据 2", "链上证据 3"],  // 1-3 条, 每条 ≤30 字
  "capital_logic": "资金逻辑, ≤60 字",
  "confidence": "high 或 medium 或 low",
  "risk_note": "风险提示, ≤40 字"
}

约束: 证据不足时 confidence=low 并明示; 不得编造新闻/公告/数据。"#;

/// G5b 分析请求 (输入 = 当日告警记录 + 观测时刻)。
#[derive(Debug, Clone)]
pub struct DeepAttributionRequest {
    pub record: AlertRecord,
    pub as_of: DateTime<Utc>,
}

/// strict JSON 解析产物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeepAttributionResult {
    pub main_reason: String,
    pub catalyst_chain: Vec<String>,
    pub capital_logic: String,
    pub confidence: String,
    pub risk_note: String,
}

/// assess() 产物: 解析结果 + 模型回执 (落库保真)。
#[derive(Debug, Clone)]
pub struct DeepAttributionOutcome {
    pub result: DeepAttributionResult,
    pub receipt: ModelCallReceipt,
    pub elapsed_ms: u64,
}

/// 落库行: 请求 + 结果 + 模型 receipt (与 news_ai 审计同档保真)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepAttributionRow {
    pub record: AlertRecord,
    pub result: DeepAttributionResult,
    pub analyzed_at: String,
    pub provider: String,
    pub model: String,
    pub upstream_request_id: Option<String>,
    pub upstream_response_id: Option<String>,
    pub elapsed_ms: u64,
}

/// 冻结当日选集并在每次非确定模型调用前记录尝试。标记存在只表示调用已开始，
/// 不能当作结果、counted 决策或物理发送的完成证据。
pub struct DeepAttributionJournal {
    dir: PathBuf,
    production: bool,
}

#[derive(Serialize, Deserialize)]
struct DeepAttributionSelection {
    schema_version: u8,
    business_date: NaiveDate,
    events: Vec<AlertRecord>,
}

/// 精确的归档行 JSON 与推送摘要字节。存在只证明 LLM 产物已持久化，
/// 不证明 JSONL 归档、counted prepare 或物理发送的状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeepAttributionFrozenResult {
    schema_version: u8,
    business_date: NaiveDate,
    selection_index: usize,
    selected_record_sha256: String,
    row_json: String,
    row_sha256: String,
    summary: String,
    summary_sha256: String,
}

impl DeepAttributionFrozenResult {
    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn row_sha256(&self) -> &str {
        &self.row_sha256
    }

    pub fn summary_sha256(&self) -> &str {
        &self.summary_sha256
    }
}

/// 已有 attempt 的 LLM 完成状态；两种旧状态均不授权自动归档或发送。
#[derive(Debug)]
pub enum DeepAttributionClaim {
    Fresh,
    CompletionUnproven,
    Frozen(DeepAttributionFrozenResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepAttributionArchiveOutcome {
    Appended,
    AlreadyPresent,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeepAttributionArchiveRecovery {
    pub appended: usize,
    pub already_present: usize,
    pub completion_unproven: usize,
    /// A damaged date does not prevent later dates from being inspected.
    pub failures: Vec<(NaiveDate, String)>,
}

/// One selected event as observed from existing files. A frozen result and an
/// exact JSONL row are analysis/archive facts, not counted delivery evidence.
#[derive(Debug)]
pub struct DeepAttributionEventInspection {
    pub index: usize,
    pub record: AlertRecord,
    pub progress: DeepAttributionEventProgress,
}

#[derive(Debug)]
pub enum DeepAttributionEventProgress {
    NotStarted,
    CompletionUnproven,
    Frozen {
        result: DeepAttributionFrozenResult,
        archived: bool,
    },
}

/// The same canonical G5b source bytes used by the counted producer and the
/// read-only day inspection. The rendered hash binds the exact frozen summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct G5bCountedSourceFacts {
    occurrence_identity: String,
    canonical_source: Vec<u8>,
    source_sha256: String,
    rendered_sha256: String,
}

impl G5bCountedSourceFacts {
    pub fn occurrence_identity(&self) -> &str {
        &self.occurrence_identity
    }

    pub fn canonical_source(&self) -> &[u8] {
        &self.canonical_source
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn rendered_sha256(&self) -> &str {
        &self.rendered_sha256
    }
}

pub fn g5b_counted_source_facts(
    business_date: NaiveDate,
    record: &AlertRecord,
    summary: &str,
) -> G5bCountedSourceFacts {
    let rendered_sha256 = sha256_hex(summary.as_bytes());
    let canonical = serde_json::json!({
        "schema": "g5b-attribution-v1",
        "business_date": business_date.format("%Y-%m-%d").to_string(),
        "code": record.code,
        "triggered_at": record.triggered_at,
        "category": record.category,
        "level": record.level,
        "message": record.message,
        "rendered_sha256": rendered_sha256,
    });
    let canonical_source = canonical.to_string().into_bytes();
    G5bCountedSourceFacts {
        occurrence_identity: g5b_event_occurrence_identity(business_date, record),
        source_sha256: sha256_hex(&canonical_source),
        canonical_source,
        rendered_sha256,
    }
}

fn g5b_event_occurrence_identity(business_date: NaiveDate, record: &AlertRecord) -> String {
    let event_facts = format!(
        "{}|{}|{}|{}",
        record.triggered_at, record.code, record.category, record.message
    );
    format!(
        "g5b-attribution:{business_date}:{}:{}",
        record.code,
        sha256_hex(event_facts.as_bytes())
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G5bSelectedEventState {
    NotStarted,
    CompletionUnproven,
    NoCountedDecision,
    Pending,
    /// Validated physical Accepted receipt.
    Accepted,
    /// Manual resolution; distinct from a physical Accepted receipt.
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
}

fn selected_state_for_counted_terminal(terminal: G5bCountedTerminalV1) -> G5bSelectedEventState {
    match terminal {
        G5bCountedTerminalV1::Pending => G5bSelectedEventState::Pending,
        G5bCountedTerminalV1::Accepted => G5bSelectedEventState::Accepted,
        G5bCountedTerminalV1::ManualAccepted => G5bSelectedEventState::ManualAccepted,
        G5bCountedTerminalV1::Rejected => G5bSelectedEventState::Rejected,
        G5bCountedTerminalV1::Uncertain => G5bSelectedEventState::Uncertain,
        G5bCountedTerminalV1::ManualNotDelivered => G5bSelectedEventState::ManualNotDelivered,
    }
}

#[derive(Debug)]
pub struct G5bSelectedEventObservation {
    pub index: usize,
    pub record: AlertRecord,
    /// `None` means no validated frozen result; JSONL is never delivery proof.
    pub archived: Option<bool>,
    pub completion: G5bSelectedEventState,
    pub decision_identity: Option<String>,
}

/// Counts and rows cover only the saved G5b selection. An unmatched durable
/// G5b decision fails reconciliation instead of being omitted from counts.
#[derive(Debug)]
pub struct G5bSelectedEventsObservation {
    pub business_date: NaiveDate,
    pub events: Vec<G5bSelectedEventObservation>,
}

impl G5bSelectedEventsObservation {
    /// Number of selected rows in this state, not a whole-day completion count.
    pub fn count(&self, completion: G5bSelectedEventState) -> usize {
        self.events
            .iter()
            .filter(|event| event.completion == completion)
            .count()
    }
}

/// Counts for one reconciled selected-event observation. Outcome counts are
/// disjoint; `without_exact_archive` is an additional filesystem dimension.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct G5bCompletionCounts {
    pub selected: usize,
    pub not_started: usize,
    pub completion_unproven: usize,
    pub no_counted_decision: usize,
    pub pending: usize,
    /// Validated physical Accepted receipts only.
    pub accepted: usize,
    /// Manual resolution, never a physical Accepted receipt.
    pub manual_accepted: usize,
    pub rejected: usize,
    pub uncertain: usize,
    pub manual_not_delivered: usize,
    /// Includes selected events that have no frozen result yet.
    pub without_exact_archive: usize,
}

/// Point-in-time classification of reconciled G5b evidence. Even
/// `TerminalOutcomesObserved` cannot authorize a day seal: journal files and
/// SQLite do not share an atomic snapshot, and a selection is not an input
/// cutoff. Reconciliation errors are returned by the inspector, not mapped to
/// a verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum G5bCompletionVerdict {
    NoSelection,
    Incomplete(G5bCompletionCounts),
    TerminalOutcomesObserved(G5bCompletionCounts),
}

fn classify_g5b_completion(
    observation: Option<&G5bSelectedEventsObservation>,
) -> G5bCompletionVerdict {
    let Some(observation) = observation else {
        return G5bCompletionVerdict::NoSelection;
    };
    let mut counts = G5bCompletionCounts {
        selected: observation.events.len(),
        ..G5bCompletionCounts::default()
    };
    for event in &observation.events {
        match event.completion {
            G5bSelectedEventState::NotStarted => counts.not_started += 1,
            G5bSelectedEventState::CompletionUnproven => counts.completion_unproven += 1,
            G5bSelectedEventState::NoCountedDecision => counts.no_counted_decision += 1,
            G5bSelectedEventState::Pending => counts.pending += 1,
            G5bSelectedEventState::Accepted => counts.accepted += 1,
            G5bSelectedEventState::ManualAccepted => counts.manual_accepted += 1,
            G5bSelectedEventState::Rejected => counts.rejected += 1,
            G5bSelectedEventState::Uncertain => counts.uncertain += 1,
            G5bSelectedEventState::ManualNotDelivered => counts.manual_not_delivered += 1,
        }
        if event.archived != Some(true) {
            counts.without_exact_archive += 1;
        }
    }
    if counts.selected == 0
        || counts.not_started != 0
        || counts.completion_unproven != 0
        || counts.no_counted_decision != 0
        || counts.pending != 0
        || counts.uncertain != 0
        || counts.without_exact_archive != 0
    {
        G5bCompletionVerdict::Incomplete(counts)
    } else {
        G5bCompletionVerdict::TerminalOutcomesObserved(counts)
    }
}

#[derive(Debug)]
pub enum G5bSelectedEventsObservationError {
    Journal(DeepAttributionError),
    Counted(DurableDeliveryError),
    Reconciliation(String),
}

impl std::fmt::Display for G5bSelectedEventsObservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Journal(error) => write!(f, "G5b selected journal inspection: {error}"),
            Self::Counted(error) => write!(f, "G5b selected counted observation: {error}"),
            Self::Reconciliation(error) => write!(f, "G5b selected reconciliation: {error}"),
        }
    }
}

impl std::error::Error for G5bSelectedEventsObservationError {}

impl From<DeepAttributionError> for G5bSelectedEventsObservationError {
    fn from(error: DeepAttributionError) -> Self {
        Self::Journal(error)
    }
}

impl From<DurableDeliveryError> for G5bSelectedEventsObservationError {
    fn from(error: DurableDeliveryError) -> Self {
        Self::Counted(error)
    }
}

/// 与 G5b counted occurrence 的事件事实字段保持一致；业务日由归档路径约束。
#[derive(PartialEq, Eq)]
struct DeepAttributionEventKey {
    code: String,
    event_hash: String,
}

impl From<&AlertRecord> for DeepAttributionEventKey {
    fn from(record: &AlertRecord) -> Self {
        let event_facts = format!(
            "{}|{}|{}|{}",
            record.triggered_at, record.code, record.category, record.message
        );
        Self {
            code: record.code.clone(),
            event_hash: sha256_hex(event_facts.as_bytes()),
        }
    }
}

impl DeepAttributionJournal {
    /// Classify the existing selected events and one validated counted-day DB
    /// snapshot without changing journal, delivery, or day-gate state. This
    /// verdict is observational and cannot seal `G5B_LAST_RUN`.
    pub fn inspect_completion_verdict(
        &self,
        date: NaiveDate,
        coordinator: &DurableDeliveryCoordinator,
    ) -> Result<G5bCompletionVerdict, G5bSelectedEventsObservationError> {
        let observation = self.inspect_selected_events_delivery(date, coordinator)?;
        Ok(classify_g5b_completion(observation.as_ref()))
    }

    pub fn production() -> Self {
        Self {
            dir: PathBuf::from("data/g5b/attempts"),
            production: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn isolated_for_test(dir: PathBuf) -> Self {
        Self {
            dir,
            production: false,
        }
    }

    fn ensure_allowed(&self) -> Result<(), DeepAttributionError> {
        if self.production && (runtime_is_test_process() || current_env() == TradingEnv::Test) {
            return Err(DeepAttributionError::Io(
                "test runtime cannot write the production G5b attempt journal".to_string(),
            ));
        }
        Ok(())
    }

    fn selection_path(&self, date: NaiveDate) -> PathBuf {
        self.dir.join(format!("{date}.selection.json"))
    }

    fn attempt_path(&self, date: NaiveDate, index: usize) -> PathBuf {
        self.dir.join(format!("{date}.{index}.attempt"))
    }

    fn result_path(&self, date: NaiveDate, index: usize) -> PathBuf {
        self.dir.join(format!("{date}.{index}.result.json"))
    }

    fn archive_path(&self, date: NaiveDate) -> PathBuf {
        self.dir
            .parent()
            .expect("G5b journal directory has parent")
            .join(format!("{date}.jsonl"))
    }

    fn archive_lock_path(&self, date: NaiveDate) -> PathBuf {
        self.dir.join(format!("{date}.archive.lock"))
    }

    /// 已保存的选集优先于当天不断增长的告警文件。首次无可选事件时不冻结选集。
    pub fn load_or_select(
        &self,
        date: NaiveDate,
        records: Vec<AlertRecord>,
    ) -> Result<Vec<AlertRecord>, DeepAttributionError> {
        self.ensure_allowed()?;
        let path = self.selection_path(date);
        let selection = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<DeepAttributionSelection>(&bytes)
                .map_err(|e| DeepAttributionError::Io(format!("读取 {path:?}: {e}")))?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                // 选集丢失但 attempt 留存时，ordinal 已无权威事件映射。
                for index in 0..DEEP_ATTRIBUTION_MAX_EVENTS {
                    for artifact in [
                        self.attempt_path(date, index),
                        self.result_path(date, index),
                    ] {
                        match fs::symlink_metadata(&artifact) {
                            Ok(_) => {
                                return Err(DeepAttributionError::Io(format!(
                                    "G5b 选集缺失但分析状态留存, 需人工裁定: {artifact:?}"
                                )))
                            }
                            Err(error) if error.kind() == ErrorKind::NotFound => {}
                            Err(error) => {
                                return Err(DeepAttributionError::Io(format!(
                                    "检查 {artifact:?}: {error}"
                                )))
                            }
                        }
                    }
                }
                let archive_lock = self.archive_lock_path(date);
                match fs::symlink_metadata(&archive_lock) {
                    Ok(_) => {
                        return Err(DeepAttributionError::Io(format!(
                            "G5b 选集缺失但归档锁留存, 需人工裁定: {archive_lock:?}"
                        )))
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(DeepAttributionError::Io(format!(
                            "检查 {archive_lock:?}: {error}"
                        )))
                    }
                }
                // 升级前的结果归档没有 pre-call 标记，不能据此证明尚未开始分析。
                let legacy_archive = self
                    .dir
                    .parent()
                    .expect("journal dir has parent")
                    .join(format!("{date}.jsonl"));
                match fs::symlink_metadata(&legacy_archive) {
                    Ok(_) => {
                        return Err(DeepAttributionError::Io(format!(
                            "已有无 attempt 标记的 G5b 归档, 需人工裁定: {legacy_archive:?}"
                        )))
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(DeepAttributionError::Io(format!(
                            "检查 {legacy_archive:?}: {error}"
                        )))
                    }
                }
                let events = top_events_for_deep(records, DEEP_ATTRIBUTION_MAX_EVENTS);
                if events.is_empty() {
                    return Ok(events);
                }
                fs::create_dir_all(&self.dir)
                    .map_err(|e| DeepAttributionError::Io(format!("创建 {:?}: {e}", self.dir)))?;
                let selection = DeepAttributionSelection {
                    schema_version: 1,
                    business_date: date,
                    events,
                };
                let bytes = serde_json::to_vec(&selection)
                    .map_err(|e| DeepAttributionError::Io(e.to_string()))?;
                match write_new_synced(&path, &bytes) {
                    Ok(true) => selection,
                    Ok(false) => {
                        let bytes = fs::read(&path)
                            .map_err(|e| DeepAttributionError::Io(format!("读取 {path:?}: {e}")))?;
                        serde_json::from_slice(&bytes)
                            .map_err(|e| DeepAttributionError::Io(format!("读取 {path:?}: {e}")))?
                    }
                    Err(error) => {
                        return Err(DeepAttributionError::Io(format!("保存 {path:?}: {error}")))
                    }
                }
            }
            Err(error) => return Err(DeepAttributionError::Io(format!("读取 {path:?}: {error}"))),
        };
        validate_selection(&selection, date, &path)?;
        fs::File::open(&path)
            .and_then(|file| file.sync_all())
            .and_then(|()| sync_parent(&path))
            .map_err(|error| DeepAttributionError::Io(format!("同步 {path:?}: {error}")))?;
        Ok(selection.events)
    }

    /// Fresh 才允许进入 LLM；已有标记或任何 IO 不确定都阻止再次调用。
    pub fn begin_assessment(
        &self,
        date: NaiveDate,
        index: usize,
    ) -> Result<DeepAttributionClaim, DeepAttributionError> {
        self.ensure_allowed()?;
        let events = self.load_or_select(date, Vec::new())?;
        let selected = events.get(index).ok_or_else(|| {
            DeepAttributionError::Io(format!(
                "G5b attempt 不在已保存选集内: {date} index={index}"
            ))
        })?;
        let path = self.attempt_path(date, index);
        let result_path = self.result_path(date, index);
        match fs::symlink_metadata(&result_path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 冻结结果不是普通文件: {result_path:?}"
                )))
            }
            Ok(_) => match fs::symlink_metadata(&path) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    return Err(DeepAttributionError::Io(format!(
                        "G5b 冻结结果缺少前置 attempt: {result_path:?}"
                    )))
                }
                Err(error) => {
                    return Err(DeepAttributionError::Io(format!("检查 {path:?}: {error}")))
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(DeepAttributionError::Io(format!(
                    "检查 {result_path:?}: {error}"
                )))
            }
        }
        match write_new_synced(&path, b"") {
            Ok(true) => Ok(DeepAttributionClaim::Fresh),
            Ok(false) => {
                self.validate_attempt_marker(date, index)?;
                match self.read_frozen_result(date, index, selected)? {
                    Some(frozen) => Ok(DeepAttributionClaim::Frozen(frozen)),
                    None => Ok(DeepAttributionClaim::CompletionUnproven),
                }
            }
            Err(error) => Err(DeepAttributionError::Io(format!("记录 {path:?}: {error}"))),
        }
    }

    fn validate_attempt_marker(
        &self,
        date: NaiveDate,
        index: usize,
    ) -> Result<(), DeepAttributionError> {
        self.inspect_attempt_marker(date, index, true)
    }

    fn inspect_attempt_marker(
        &self,
        date: NaiveDate,
        index: usize,
        sync: bool,
    ) -> Result<(), DeepAttributionError> {
        let path = self.attempt_path(date, index);
        if !sync {
            let bytes = read_regular_bytes_for_inspection(&path)?.ok_or_else(|| {
                DeepAttributionError::Io(format!("G5b attempt 标记读取中消失: {path:?}"))
            })?;
            if !bytes.is_empty() {
                return Err(DeepAttributionError::Io(format!(
                    "G5b attempt 标记无效: {path:?}"
                )));
            }
            return Ok(());
        }
        let marker = fs::symlink_metadata(&path)
            .map_err(|error| DeepAttributionError::Io(format!("检查 {path:?}: {error}")))?;
        if !marker.file_type().is_file() || marker.len() != 0 {
            return Err(DeepAttributionError::Io(format!(
                "G5b attempt 标记无效: {path:?}"
            )));
        }
        if sync {
            fs::File::open(&path)
                .and_then(|file| file.sync_all())
                .and_then(|()| sync_parent(&path))
                .map_err(|error| DeepAttributionError::Io(format!("同步 {path:?}: {error}")))?;
        }
        Ok(())
    }

    /// 必须在首次 JSONL 归档或 counted 调用前执行；已存在结果绝不覆盖。
    pub fn freeze_result(
        &self,
        date: NaiveDate,
        index: usize,
        row: &DeepAttributionRow,
        summary: &str,
    ) -> Result<DeepAttributionFrozenResult, DeepAttributionError> {
        self.ensure_allowed()?;
        let events = self.load_or_select(date, Vec::new())?;
        let selected = events.get(index).ok_or_else(|| {
            DeepAttributionError::Io(format!("G5b 结果不在已保存选集内: {date} index={index}"))
        })?;
        let selected_bytes = serde_json::to_vec(selected)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        let row_record_bytes = serde_json::to_vec(&row.record)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        if selected_bytes != row_record_bytes || summary != render_deep_attribution_summary(row) {
            return Err(DeepAttributionError::Io(
                "G5b 冻结结果与选集或摘要渲染不一致".to_string(),
            ));
        }
        self.validate_attempt_marker(date, index)?;

        let row_json = serde_json::to_string(row)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        let frozen = DeepAttributionFrozenResult {
            schema_version: 1,
            business_date: date,
            selection_index: index,
            selected_record_sha256: sha256_hex(&selected_bytes),
            row_sha256: sha256_hex(row_json.as_bytes()),
            summary_sha256: sha256_hex(summary.as_bytes()),
            row_json,
            summary: summary.to_string(),
        };
        let path = self.result_path(date, index);
        let bytes = serde_json::to_vec(&frozen)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        match write_new_synced(&path, &bytes) {
            Ok(true) => Ok(frozen),
            Ok(false) => Err(DeepAttributionError::Io(format!(
                "G5b 冻结结果已存在, 不覆盖: {path:?}"
            ))),
            Err(error) => Err(DeepAttributionError::Io(format!(
                "保存 G5b 冻结结果 {path:?}: {error}"
            ))),
        }
    }

    fn read_frozen_result(
        &self,
        date: NaiveDate,
        index: usize,
        selected: &AlertRecord,
    ) -> Result<Option<DeepAttributionFrozenResult>, DeepAttributionError> {
        self.inspect_frozen_result(date, index, selected, true)
    }

    fn inspect_frozen_result(
        &self,
        date: NaiveDate,
        index: usize,
        selected: &AlertRecord,
        sync: bool,
    ) -> Result<Option<DeepAttributionFrozenResult>, DeepAttributionError> {
        let path = self.result_path(date, index);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 冻结结果不是普通文件: {path:?}"
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(DeepAttributionError::Io(format!("检查 {path:?}: {error}"))),
        }
        let bytes = if sync {
            match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(DeepAttributionError::Io(format!("读取 {path:?}: {error}")))
                }
            }
        } else {
            let Some(bytes) = read_regular_bytes_for_inspection(&path)? else {
                return Ok(None);
            };
            bytes
        };
        let frozen: DeepAttributionFrozenResult = serde_json::from_slice(&bytes)
            .map_err(|error| DeepAttributionError::Io(format!("解析 {path:?}: {error}")))?;
        let row: DeepAttributionRow = serde_json::from_str(&frozen.row_json)
            .map_err(|error| DeepAttributionError::Io(format!("解析 {path:?} row: {error}")))?;
        let selected_bytes = serde_json::to_vec(selected)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        let row_record_bytes = serde_json::to_vec(&row.record)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        if frozen.schema_version != 1
            || frozen.business_date != date
            || frozen.selection_index != index
            || selected_bytes != row_record_bytes
            || frozen.selected_record_sha256 != sha256_hex(&selected_bytes)
            || frozen.row_sha256 != sha256_hex(frozen.row_json.as_bytes())
            || frozen.summary_sha256 != sha256_hex(frozen.summary.as_bytes())
            || serde_json::to_string(&row).ok().as_deref() != Some(frozen.row_json.as_str())
            || frozen.summary != render_deep_attribution_summary(&row)
        {
            return Err(DeepAttributionError::Io(format!(
                "G5b 冻结结果完整性检查失败: {path:?}"
            )));
        }
        if sync {
            fs::File::open(&path)
                .and_then(|file| file.sync_all())
                .and_then(|()| sync_parent(&path))
                .map_err(|error| DeepAttributionError::Io(format!("同步 {path:?}: {error}")))?;
        }
        Ok(Some(frozen))
    }

    /// Inspect an existing selection and its event files without creating,
    /// syncing, locking, archiving, or starting an attempt. A missing selection
    /// is `None` only when this date has no known journal/archive artifacts;
    /// this does not inspect orphan counted decisions in SQLite.
    pub fn inspect_existing_day(
        &self,
        date: NaiveDate,
    ) -> Result<Option<Vec<DeepAttributionEventInspection>>, DeepAttributionError> {
        let selection_path = self.selection_path(date);
        match fs::symlink_metadata(&selection_path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 选集不是普通文件: {selection_path:?}"
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let mut artifacts = vec![self.archive_lock_path(date), self.archive_path(date)];
                for index in 0..DEEP_ATTRIBUTION_MAX_EVENTS {
                    artifacts.push(self.attempt_path(date, index));
                    artifacts.push(self.result_path(date, index));
                }
                for artifact in artifacts {
                    match fs::symlink_metadata(&artifact) {
                        Ok(_) => {
                            return Err(DeepAttributionError::Io(format!(
                                "G5b 选集缺失但状态留存: {artifact:?}"
                            )))
                        }
                        Err(error) if error.kind() == ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(DeepAttributionError::Io(format!(
                                "检查 {artifact:?}: {error}"
                            )))
                        }
                    }
                }
                return Ok(None);
            }
            Err(error) => {
                return Err(DeepAttributionError::Io(format!(
                    "检查 {selection_path:?}: {error}"
                )))
            }
        }
        let selection_bytes =
            read_regular_bytes_for_inspection(&selection_path)?.ok_or_else(|| {
                DeepAttributionError::Io(format!("选集读取中消失: {selection_path:?}"))
            })?;
        let selection: DeepAttributionSelection = serde_json::from_slice(&selection_bytes)
            .map_err(|error| {
                DeepAttributionError::Io(format!("解析 {selection_path:?}: {error}"))
            })?;
        validate_selection(&selection, date, &selection_path)?;
        let mut selected_keys = Vec::with_capacity(selection.events.len());
        for record in &selection.events {
            let key = DeepAttributionEventKey::from(record);
            if selected_keys.contains(&key) {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 选集事件身份重复: {selection_path:?}"
                )));
            }
            selected_keys.push(key);
        }

        let archive_path = self.archive_path(date);
        let archive = inspect_archive_bytes(&archive_path)?;
        let mut archived_rows: Vec<(DeepAttributionEventKey, Vec<u8>)> = Vec::new();
        if !archive.is_empty() {
            if archive.last() != Some(&b'\n') {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 历史归档末行不完整: {archive_path:?}"
                )));
            }
            for (line_number, line) in archive[..archive.len() - 1]
                .split(|byte| *byte == b'\n')
                .enumerate()
            {
                let row: DeepAttributionRow = serde_json::from_slice(line).map_err(|error| {
                    DeepAttributionError::Io(format!(
                        "G5b 历史归档第 {} 行无效 {archive_path:?}: {error}",
                        line_number + 1
                    ))
                })?;
                let key = DeepAttributionEventKey::from(&row.record);
                if !selected_keys.contains(&key) {
                    return Err(DeepAttributionError::Io(format!(
                        "G5b 历史归档含选集外事件: {archive_path:?} line={}",
                        line_number + 1
                    )));
                }
                if archived_rows.iter().any(|(seen, _)| *seen == key) {
                    return Err(DeepAttributionError::Io(format!(
                        "G5b 历史归档事件身份重复: {archive_path:?} line={}",
                        line_number + 1
                    )));
                }
                archived_rows.push((key, line.to_vec()));
            }
        }

        let mut inspections = Vec::with_capacity(selection.events.len());
        for (index, record) in selection.events.into_iter().enumerate() {
            let attempt_path = self.attempt_path(date, index);
            let has_attempt = match fs::symlink_metadata(&attempt_path) {
                Ok(_) => {
                    self.inspect_attempt_marker(date, index, false)?;
                    true
                }
                Err(error) if error.kind() == ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(DeepAttributionError::Io(format!(
                        "检查 {attempt_path:?}: {error}"
                    )))
                }
            };
            let frozen = self.inspect_frozen_result(date, index, &record, false)?;
            if frozen.is_some() && !has_attempt {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 冻结结果缺少前置 attempt: {:?}",
                    self.result_path(date, index)
                )));
            }
            let archived = archived_rows
                .iter()
                .find(|(key, _)| *key == DeepAttributionEventKey::from(&record));
            let progress = match (has_attempt, frozen, archived) {
                (false, None, None) => DeepAttributionEventProgress::NotStarted,
                (true, None, None) => DeepAttributionEventProgress::CompletionUnproven,
                (true, Some(result), archived) => {
                    if let Some((_, bytes)) = archived {
                        if bytes.as_slice() != result.row_json.as_bytes() {
                            return Err(DeepAttributionError::Io(format!(
                                "G5b 同事件归档字节冲突: {archive_path:?} index={index}"
                            )));
                        }
                    }
                    DeepAttributionEventProgress::Frozen {
                        result,
                        archived: archived.is_some(),
                    }
                }
                (_, None, Some(_)) => {
                    return Err(DeepAttributionError::Io(format!(
                        "G5b 归档事件缺少冻结结果: {archive_path:?} index={index}"
                    )))
                }
                (false, Some(_), _) => unreachable!("checked frozen attempt above"),
            };
            inspections.push(DeepAttributionEventInspection {
                index,
                record,
                progress,
            });
        }
        Ok(Some(inspections))
    }

    /// Observe only events in the saved selection. `None` means no selection
    /// artifact. All G5b decisions for the date are validated in one DB read
    /// snapshot, including decisions outside the selection; those fail closed.
    /// Files and SQLite are not one atomic snapshot, so this cannot directly
    /// seal G5B_LAST_RUN. This never prepares or retries a decision. Run it
    /// off the async monitor tick.
    pub fn inspect_selected_events_delivery(
        &self,
        date: NaiveDate,
        coordinator: &DurableDeliveryCoordinator,
    ) -> Result<Option<G5bSelectedEventsObservation>, G5bSelectedEventsObservationError> {
        let inspections = self.inspect_existing_day(date)?;
        let counted = coordinator.g5b_counted_day_snapshot(&date.to_string())?;
        let Some(inspections) = inspections else {
            if !counted.facts().is_empty() {
                return Err(G5bSelectedEventsObservationError::Reconciliation(
                    "counted decision exists without a saved selection".to_owned(),
                ));
            }
            return Ok(None);
        };
        let selected_occurrences = inspections
            .iter()
            .map(|event| g5b_event_occurrence_identity(date, &event.record))
            .collect::<BTreeSet<_>>();
        for fact in counted.facts() {
            if !selected_occurrences.contains(fact.occurrence_identity()) {
                return Err(G5bSelectedEventsObservationError::Reconciliation(format!(
                    "counted occurrence is outside saved selection: {}",
                    fact.occurrence_identity()
                )));
            }
        }
        let mut events = Vec::with_capacity(inspections.len());
        for inspection in inspections {
            let occurrence = g5b_event_occurrence_identity(date, &inspection.record);
            let fact = counted
                .facts()
                .iter()
                .find(|fact| fact.occurrence_identity() == occurrence);
            let (completion, archived, decision_identity) = match inspection.progress {
                DeepAttributionEventProgress::NotStarted => {
                    if fact.is_some() {
                        return Err(G5bSelectedEventsObservationError::Reconciliation(format!(
                            "counted decision has no analysis attempt: {occurrence}"
                        )));
                    }
                    (G5bSelectedEventState::NotStarted, None, None)
                }
                DeepAttributionEventProgress::CompletionUnproven => {
                    if fact.is_some() {
                        return Err(G5bSelectedEventsObservationError::Reconciliation(format!(
                            "counted decision has no frozen result: {occurrence}"
                        )));
                    }
                    (G5bSelectedEventState::CompletionUnproven, None, None)
                }
                DeepAttributionEventProgress::Frozen { result, archived } => {
                    let expected =
                        g5b_counted_source_facts(date, &inspection.record, result.summary());
                    if let Some(fact) = fact {
                        if fact.source_binding_sha256() != expected.source_sha256()
                            || fact.rendered_content_sha256() != result.summary_sha256()
                        {
                            return Err(G5bSelectedEventsObservationError::Reconciliation(
                                format!("frozen source or summary mismatch: {occurrence}"),
                            ));
                        }
                    }
                    let completion = fact
                        .map(|fact| {
                            selected_state_for_counted_terminal(fact.observation().terminal())
                        })
                        .unwrap_or(G5bSelectedEventState::NoCountedDecision);
                    (
                        completion,
                        Some(archived),
                        fact.map(|fact| fact.observation().decision_identity().to_string()),
                    )
                }
            };
            events.push(G5bSelectedEventObservation {
                index: inspection.index,
                record: inspection.record,
                archived,
                completion,
                decision_identity,
            });
        }
        Ok(Some(G5bSelectedEventsObservation {
            business_date: date,
            events,
        }))
    }

    /// 将已冻结的精确行字节投影到旧 JSONL。只证明归档完成，不授权 counted 重发。
    /// 同日锁保护完整扫描与原子替换；坏行、重复事件或不同结果均拒绝写入。
    pub fn archive_frozen_result(
        &self,
        date: NaiveDate,
        index: usize,
    ) -> Result<DeepAttributionArchiveOutcome, DeepAttributionError> {
        self.ensure_allowed()?;
        let events = self.load_or_select(date, Vec::new())?;
        let selected = events.get(index).ok_or_else(|| {
            DeepAttributionError::Io(format!("G5b 归档不在已保存选集内: {date} index={index}"))
        })?;
        self.validate_attempt_marker(date, index)?;
        let frozen = self
            .read_frozen_result(date, index, selected)?
            .ok_or_else(|| {
                DeepAttributionError::Io(format!("G5b 归档缺少冻结结果: {date} index={index}"))
            })?;
        let row: DeepAttributionRow = serde_json::from_str(&frozen.row_json)
            .map_err(|error| DeepAttributionError::Io(error.to_string()))?;
        if !row.record.is_production_eligible() {
            return Err(DeepAttributionError::IneligibleRecord(format!(
                "code={} origin={:?}",
                row.record.code, row.record.origin
            )));
        }
        let key = DeepAttributionEventKey::from(&row.record);
        let path = self.archive_path(date);
        let lock_path = self.archive_lock_path(date);
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)
            .map_err(|error| DeepAttributionError::Io(format!("打开 {lock_path:?}: {error}")))?;
        let lock_metadata = fs::symlink_metadata(&lock_path)
            .map_err(|error| DeepAttributionError::Io(format!("检查 {lock_path:?}: {error}")))?;
        if !lock_metadata.file_type().is_file()
            || !lock
                .metadata()
                .map_err(|error| DeepAttributionError::Io(error.to_string()))?
                .is_file()
        {
            return Err(DeepAttributionError::Io(format!(
                "G5b 归档锁不是普通文件: {lock_path:?}"
            )));
        }
        lock.lock_exclusive()
            .map_err(|error| DeepAttributionError::Io(format!("锁定 {lock_path:?}: {error}")))?;

        let original = read_archive_bytes(&path)?;
        let mut seen: Vec<DeepAttributionEventKey> = Vec::new();
        if !original.is_empty() {
            if original.last() != Some(&b'\n') {
                return Err(DeepAttributionError::Io(format!(
                    "G5b 历史归档末行不完整: {path:?}"
                )));
            }
            for (line_number, line) in original[..original.len() - 1]
                .split(|byte| *byte == b'\n')
                .enumerate()
            {
                let historical: DeepAttributionRow = serde_json::from_slice(line).map_err(|e| {
                    DeepAttributionError::Io(format!(
                        "G5b 历史归档第 {} 行无效 {path:?}: {e}",
                        line_number + 1
                    ))
                })?;
                let historical_key = DeepAttributionEventKey::from(&historical.record);
                if seen.contains(&historical_key) {
                    return Err(DeepAttributionError::Io(format!(
                        "G5b 历史归档事件身份重复: {path:?} line={}",
                        line_number + 1
                    )));
                }
                if historical_key == key {
                    if line != frozen.row_json.as_bytes() {
                        return Err(DeepAttributionError::Io(format!(
                            "G5b 同事件归档字节冲突: {path:?} line={}",
                            line_number + 1
                        )));
                    }
                    // 仍扫描余下各行，不能将后续坏行或重复行误判为有效归档。
                }
                seen.push(historical_key);
            }
        }
        if seen.contains(&key) {
            return Ok(DeepAttributionArchiveOutcome::AlreadyPresent);
        }
        let mut projected = original;
        projected.extend_from_slice(frozen.row_json.as_bytes());
        projected.push(b'\n');
        write_atomic_archive(&path, &projected)?;
        Ok(DeepAttributionArchiveOutcome::Appended)
    }

    /// 只扫描已有 journal；没有选集时不会冻结新选集或建立 Fresh attempt。
    /// 归档可以从冻结结果补齐，counted/物理投递必须由独立权威裁定。
    pub fn recover_existing_frozen_archives(
        &self,
        as_of: NaiveDateTime,
    ) -> Result<DeepAttributionArchiveRecovery, DeepAttributionError> {
        self.recover_frozen_archives_for_dates(self.existing_recovery_dates(as_of)?)
    }

    /// Discover dates once in a blocking worker. Callers can process the
    /// returned dates in bounded batches and retry only failed dates.
    pub fn existing_recovery_dates(
        &self,
        as_of: NaiveDateTime,
    ) -> Result<Vec<NaiveDate>, DeepAttributionError> {
        self.ensure_allowed()?;
        let dir_metadata = match fs::symlink_metadata(&self.dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(DeepAttributionError::Io(format!(
                    "检查 {:?}: {error}",
                    self.dir
                )))
            }
        };
        if !dir_metadata.file_type().is_dir() {
            return Err(DeepAttributionError::Io(format!(
                "G5b journal 不是目录: {:?}",
                self.dir
            )));
        }
        let mut dates = std::collections::BTreeSet::new();
        for entry in fs::read_dir(&self.dir)
            .map_err(|error| DeepAttributionError::Io(format!("读取 {:?}: {error}", self.dir)))?
        {
            let entry = entry.map_err(|error| DeepAttributionError::Io(error.to_string()))?;
            let name = entry.file_name().into_string().map_err(|_| {
                DeepAttributionError::Io("G5b journal 存在非 UTF-8 文件名".to_string())
            })?;
            let date_text = if let Some(date) = name.strip_suffix(".selection.json") {
                Some(date)
            } else if let Some(date) = name.strip_suffix(".archive.lock") {
                Some(date)
            } else if name.ends_with(".attempt") || name.ends_with(".result.json") {
                name.get(..10)
            } else {
                None
            };
            let Some(date_text) = date_text else {
                continue;
            };
            let date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|error| {
                DeepAttributionError::Io(format!("G5b journal 日期文件名无效 {name:?}: {error}"))
            })?;
            if date.to_string() != date_text {
                return Err(DeepAttributionError::Io(format!(
                    "G5b journal 日期文件名非标准格式: {name:?}"
                )));
            }
            if date <= as_of.date() {
                dates.insert(date);
            }
        }

        Ok(dates.into_iter().collect())
    }

    /// Retry only dates that failed the initial scan. All filesystem work is
    /// performed by the caller's blocking worker, never on the async tick.
    pub fn recover_frozen_archives_for_dates(
        &self,
        dates: impl IntoIterator<Item = NaiveDate>,
    ) -> Result<DeepAttributionArchiveRecovery, DeepAttributionError> {
        self.ensure_allowed()?;
        let mut recovery = DeepAttributionArchiveRecovery::default();
        for date in dates.into_iter().collect::<std::collections::BTreeSet<_>>() {
            match self.recover_frozen_archives_for_date(date) {
                Ok(date_recovery) => {
                    recovery.appended += date_recovery.appended;
                    recovery.already_present += date_recovery.already_present;
                    recovery.completion_unproven += date_recovery.completion_unproven;
                }
                Err(error) => recovery.failures.push((date, error.to_string())),
            }
        }
        Ok(recovery)
    }

    fn recover_frozen_archives_for_date(
        &self,
        date: NaiveDate,
    ) -> Result<DeepAttributionArchiveRecovery, DeepAttributionError> {
        // 空输入只加载已存在选集；孤儿 attempt/result/lock 会 fail closed。
        let events = self.load_or_select(date, Vec::new())?;
        let mut recovery = DeepAttributionArchiveRecovery::default();
        for index in 0..events.len() {
            let result_path = self.result_path(date, index);
            match fs::symlink_metadata(&result_path) {
                Ok(metadata) if !metadata.file_type().is_file() => {
                    return Err(DeepAttributionError::Io(format!(
                        "G5b 冻结结果不是普通文件: {result_path:?}"
                    )))
                }
                Ok(_) => match self.archive_frozen_result(date, index)? {
                    DeepAttributionArchiveOutcome::Appended => recovery.appended += 1,
                    DeepAttributionArchiveOutcome::AlreadyPresent => recovery.already_present += 1,
                },
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    let attempt = self.attempt_path(date, index);
                    match fs::symlink_metadata(&attempt) {
                        Ok(_) => {
                            self.validate_attempt_marker(date, index)?;
                            recovery.completion_unproven += 1;
                        }
                        Err(error) if error.kind() == ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(DeepAttributionError::Io(format!(
                                "检查 {attempt:?}: {error}"
                            )))
                        }
                    }
                }
                Err(error) => {
                    return Err(DeepAttributionError::Io(format!(
                        "检查 {result_path:?}: {error}"
                    )))
                }
            }
        }
        Ok(recovery)
    }
}

fn validate_selection(
    selection: &DeepAttributionSelection,
    date: NaiveDate,
    path: &Path,
) -> Result<(), DeepAttributionError> {
    if selection.schema_version != 1
        || selection.business_date != date
        || selection.events.is_empty()
        || selection.events.len() > DEEP_ATTRIBUTION_MAX_EVENTS
        || selection
            .events
            .iter()
            .any(|event| !event.is_production_eligible())
    {
        return Err(DeepAttributionError::Io(format!(
            "G5b 选集完整性检查失败: {path:?}"
        )));
    }
    Ok(())
}

fn read_archive_bytes(path: &Path) -> Result<Vec<u8>, DeepAttributionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(DeepAttributionError::Io(format!(
                "G5b 历史归档不是普通文件: {path:?}"
            )))
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(DeepAttributionError::Io(format!("检查 {path:?}: {error}"))),
    }
    let mut file = fs::File::open(path)
        .map_err(|error| DeepAttributionError::Io(format!("打开 {path:?}: {error}")))?;
    if !file
        .metadata()
        .map_err(|error| DeepAttributionError::Io(error.to_string()))?
        .is_file()
    {
        return Err(DeepAttributionError::Io(format!(
            "G5b 历史归档不是普通文件: {path:?}"
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| DeepAttributionError::Io(format!("读取 {path:?}: {error}")))?;
    file.sync_all()
        .and_then(|()| sync_parent(path))
        .map_err(|error| DeepAttributionError::Io(format!("同步 {path:?}: {error}")))?;
    Ok(bytes)
}

fn inspect_archive_bytes(path: &Path) -> Result<Vec<u8>, DeepAttributionError> {
    Ok(read_regular_bytes_for_inspection(path)?.unwrap_or_default())
}

/// Pin a regular file before reading so a changed pathname cannot redirect
/// inspection to a different object between symlink_metadata and open.
fn read_regular_bytes_for_inspection(path: &Path) -> Result<Option<Vec<u8>>, DeepAttributionError> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => {
            return Err(DeepAttributionError::Io(format!(
                "G5b inspection 文件不是普通文件: {path:?}"
            )))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(DeepAttributionError::Io(format!("检查 {path:?}: {error}"))),
    };
    let mut file = fs::File::open(path)
        .map_err(|error| DeepAttributionError::Io(format!("打开 {path:?}: {error}")))?;
    let opened = file
        .metadata()
        .map_err(|error| DeepAttributionError::Io(format!("检查已打开 {path:?}: {error}")))?;
    if !opened.is_file() || !same_file_identity(&before, &opened) {
        return Err(DeepAttributionError::Io(format!(
            "G5b inspection 文件在打开时被替换: {path:?}"
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| DeepAttributionError::Io(format!("读取 {path:?}: {error}")))?;
    let after = fs::symlink_metadata(path)
        .map_err(|error| DeepAttributionError::Io(format!("复查 {path:?}: {error}")))?;
    if !after.file_type().is_file() || !same_file_identity(&opened, &after) {
        return Err(DeepAttributionError::Io(format!(
            "G5b inspection 文件在读取时被替换: {path:?}"
        )));
    }
    Ok(Some(bytes))
}

#[cfg(unix)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

fn write_atomic_archive(path: &Path, bytes: &[u8]) -> Result<(), DeepAttributionError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().expect("G5b archive path has parent");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| DeepAttributionError::Io(error.to_string()))?
        .as_nanos();
    for _ in 0..16 {
        let candidate = parent.join(format!(
            ".{}.tmp.{}.{}.{}",
            path.file_name()
                .expect("archive has filename")
                .to_string_lossy(),
            std::process::id(),
            nonce,
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(DeepAttributionError::Io(format!(
                    "创建 {candidate:?}: {error}"
                )))
            }
        };
        let result = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&candidate, path))
            .and_then(|()| sync_parent(path));
        if let Err(error) = result {
            let _ = fs::remove_file(&candidate);
            return Err(DeepAttributionError::Io(format!(
                "原子归档 {path:?}: {error}"
            )));
        }
        return Ok(());
    }
    Err(DeepAttributionError::Io(format!(
        "G5b 归档临时文件名冲突: {path:?}"
    )))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    file.write_all(bytes).and_then(|()| file.sync_all())?;
    sync_parent(path)?;
    Ok(true)
}

fn sync_parent(path: &Path) -> std::io::Result<()> {
    fs::File::open(path.parent().expect("journal path has parent")).and_then(|dir| dir.sync_all())
}

/// G5b 分析器 (side-effect-free, 仿 NewsAIAnalyzer)。
#[derive(Clone)]
pub struct DeepAttributionAnalyzer {
    provider: Arc<dyn LlmProvider>,
}

/// G5b 错误 (出声语义, 不静默折叠)。
#[derive(Debug)]
pub enum DeepAttributionError {
    IneligibleRecord(String),
    ModelUnavailable(String),
    InvalidModelSchema(String),
    Io(String),
}

impl std::fmt::Display for DeepAttributionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IneligibleRecord(m) => write!(f, "ineligible record: {m}"),
            Self::ModelUnavailable(m) => write!(f, "model unavailable: {m}"),
            Self::InvalidModelSchema(m) => write!(f, "invalid model schema: {m}"),
            Self::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for DeepAttributionError {}

impl DeepAttributionAnalyzer {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// 深链归因: 45s 超时 + receipt 保真。
    /// provider 无回执能力 (ReceiptUnavailable) → 显式失败 (fail-closed, 与 news_ai 同策)。
    pub async fn assess(
        &self,
        request: &DeepAttributionRequest,
    ) -> Result<DeepAttributionOutcome, DeepAttributionError> {
        if !request.record.is_production_eligible() {
            return Err(DeepAttributionError::IneligibleRecord(format!(
                "code={} origin={:?}",
                request.record.code, request.record.origin
            )));
        }
        let user_prompt = deep_attribution_prompt(request);
        let started = std::time::Instant::now();
        let completed: ReceiptBearingJson = tokio::time::timeout(
            std::time::Duration::from_secs(MODEL_CALL_TIMEOUT_SECONDS),
            self.provider
                .chat_json_with_receipt(G5B_SYSTEM_PROMPT_V1, &user_prompt),
        )
        .await
        .map_err(|_| {
            DeepAttributionError::ModelUnavailable(format!(
                "model call exceeded {MODEL_CALL_TIMEOUT_SECONDS}s"
            ))
        })?
        .map_err(deep_model_call_error)?;
        let (_, raw_response, receipt) = completed.into_parts();
        let result = parse_deep_attribution_output(&raw_response).map_err(|e| {
            DeepAttributionError::InvalidModelSchema(format!(
                "G5b 深链归因输出无法按 v1 schema 解析: {e}"
            ))
        })?;
        Ok(DeepAttributionOutcome {
            result,
            receipt,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// user prompt 构建: 告警记录全字段 → 模型输入 (缺字段明示 absent, 不编造)。
pub fn deep_attribution_prompt(request: &DeepAttributionRequest) -> String {
    let r = &request.record;
    format!(
        "告警时间: {triggered}\n\
         代码: {code} {name}\n\
         级别: {level} | 类别: {category}\n\
         消息: {message}\n\
         价格: {price}\n\
         涨跌幅: {change_pct}%\n\
         主力净流入(亿): {main_flow}\n\
         关联新闻: {news_title}\n\
         新闻重要度: {news_importance}\n\
         G5a 规则快归因: {attribution_decision}\n\
         T1 锁定: {t1_locked}",
        triggered = r.triggered_at,
        code = r.code,
        name = r.name,
        level = r.level,
        category = r.category,
        message = r.message,
        price = r
            .price
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        change_pct = r
            .change_pct
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        main_flow = r
            .main_flow_yi
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        news_title = r.news_title.as_deref().unwrap_or("absent"),
        news_importance = r
            .news_importance
            .map(|v| v.to_string())
            .unwrap_or_else(|| "absent".to_string()),
        attribution_decision = r.attribution_decision.as_deref().unwrap_or("absent"),
        t1_locked = r.t1_locked,
    )
}

/// strict JSON 解析: 只接受完整 5 字段; 缺失/类型错 → 显式错误。
pub fn parse_deep_attribution_output(response: &str) -> Result<DeepAttributionResult, String> {
    let trimmed = response.trim();
    // 兼容模型可能输出的 markdown 围栏 (实际要求无, 但剥离后仍按 strict 校验)。
    let json_body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.strip_suffix("```").unwrap_or(s))
        .unwrap_or(trimmed);
    let value: serde_json::Value = serde_json::from_str(json_body).map_err(|e| {
        format!(
            "JSON 解析失败: {e} (原文: {})",
            truncated_for_error(trimmed)
        )
    })?;
    let obj = value.as_object().ok_or("顶层必须是 JSON 对象")?;
    let main_reason = required_string(obj, "main_reason")?;
    let catalyst_chain = match obj.get("catalyst_chain") {
        Some(serde_json::Value::Array(items)) => {
            let chains: Result<Vec<String>, String> = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "catalyst_chain 元素必须是字符串".to_string())
                })
                .collect();
            chains?
        }
        Some(_) => return Err("catalyst_chain 必须是数组".to_string()),
        None => return Err("缺少 catalyst_chain 字段".to_string()),
    };
    let capital_logic = required_string(obj, "capital_logic")?;
    let confidence = required_string(obj, "confidence")?;
    if !matches!(confidence.as_str(), "high" | "medium" | "low") {
        return Err(format!(
            "confidence 必须是 high/medium/low, 收到: {confidence}"
        ));
    }
    let risk_note = required_string(obj, "risk_note")?;
    Ok(DeepAttributionResult {
        main_reason,
        catalyst_chain,
        capital_logic,
        confidence,
        risk_note,
    })
}

fn required_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("缺少或非字符串字段: {key}"))
}

fn truncated_for_error(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{head}…(截断)")
    } else {
        head
    }
}

fn deep_model_call_error(error: LlmError) -> DeepAttributionError {
    DeepAttributionError::ModelUnavailable(error.to_string())
}

/// 当日事件优先级筛选: Emergency > Important > Info, 最多 max 个。
/// 同级别按告警时间先后 (sort_by_key 稳定), 不足 max 时全取。
pub fn top_events_for_deep(records: Vec<AlertRecord>, max: usize) -> Vec<AlertRecord> {
    let mut events: Vec<_> = records
        .into_iter()
        .filter(|record| {
            let eligible = record.is_production_eligible();
            if !eligible {
                log::warn!(
                    "[g5b] top_events_for_deep 跳过非生产告警: code={} origin={:?}",
                    record.code,
                    record.origin
                );
            }
            eligible
        })
        .collect();
    let priority = |level: &str| match level {
        "紧急" => 0usize,
        "重要" => 1,
        _ => 2,
    };
    events.sort_by_key(|r| priority(&r.level));
    events.truncate(max);
    events
}

/// 渲染单条深链归因 markdown 段 (15:05 报告追加 + 推送文本复用)。
pub fn render_deep_attribution(row: &DeepAttributionRow) -> String {
    let r = &row.record;
    let chains = if row.result.catalyst_chain.is_empty() {
        "  - （无链上证据）".to_string()
    } else {
        row.result
            .catalyst_chain
            .iter()
            .map(|c| format!("  - {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## G5b 深链归因: {code} {name} ({category} / {level})\n\
         主因: {main_reason}\n\
         催化剂链:\n{chains}\n\
         资金逻辑: {capital_logic}\n\
         置信度: {confidence} | 风险: {risk_note}\n\
         (模型 {provider}/{model}, {elapsed_ms}ms, 告警 {triggered})",
        code = r.code,
        name = r.name,
        category = r.category,
        level = r.level,
        main_reason = row.result.main_reason,
        capital_logic = row.result.capital_logic,
        confidence = row.result.confidence,
        risk_note = row.result.risk_note,
        provider = row.provider,
        model = row.model,
        elapsed_ms = row.elapsed_ms,
        triggered = r.triggered_at,
    )
}

/// 推送摘要 (单条, 供 PushKind::G5bAttribution 独立推送)。
pub fn render_deep_attribution_summary(row: &DeepAttributionRow) -> String {
    let r = &row.record;
    format!(
        "🔍 {code} {name} ({category}) 深链归因\n\
         主因: {main_reason}\n\
         逻辑: {capital_logic}\n\
         置信度: {confidence} | 风险: {risk_note}",
        code = r.code,
        name = r.name,
        category = r.category,
        main_reason = row.result.main_reason,
        capital_logic = row.result.capital_logic,
        confidence = row.result.confidence,
        risk_note = row.result.risk_note,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmError;
    use serde_json::Value;
    use std::sync::Arc;

    fn sample_record() -> AlertRecord {
        AlertRecord {
            origin: crate::monitor::alert_log::AlertRecordOrigin::LegacyUnknown,
            triggered_at: "2026-08-22T14:00:00+08:00".to_string(),
            code: "600396".to_string(),
            name: "金山股份".to_string(),
            level: "重要".to_string(),
            category: "主力突袭".to_string(),
            message: "盘中 14:00 主力资金突袭".to_string(),
            price: Some(14.28),
            change_pct: Some(5.2),
            main_flow_yi: Some(0.8),
            news_title: Some("公司中标新能源项目".to_string()),
            news_importance: Some(5),
            attribution_decision: Some("NewsCatalyst".to_string()),
            routed_external_id: None,
            t1_locked: false,
        }
    }

    fn sample_request() -> DeepAttributionRequest {
        DeepAttributionRequest {
            record: sample_record(),
            as_of: Utc::now(),
        }
    }

    fn sample_row() -> DeepAttributionRow {
        DeepAttributionRow {
            record: sample_record(),
            result: DeepAttributionResult {
                main_reason: "中标催化".into(),
                catalyst_chain: vec!["公告".into()],
                capital_logic: "吸筹".into(),
                confidence: "medium".into(),
                risk_note: "回落".into(),
            },
            analyzed_at: "2026-09-20T15:06:00Z".into(),
            provider: "fixture".into(),
            model: "fixture-model".into(),
            upstream_request_id: Some("req-1".into()),
            upstream_response_id: Some("resp-1".into()),
            elapsed_ms: 123,
        }
    }

    fn prepared_archive_journal(
        root: &Path,
    ) -> (
        DeepAttributionJournal,
        NaiveDate,
        DeepAttributionFrozenResult,
    ) {
        let journal = DeepAttributionJournal {
            dir: root.join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let summary = render_deep_attribution_summary(&row);
        let frozen = journal.freeze_result(date, 0, &row, &summary).unwrap();
        (journal, date, frozen)
    }

    #[test]
    fn prompt_contains_all_record_fields() {
        let prompt = deep_attribution_prompt(&sample_request());
        for needle in [
            "600396",
            "金山股份",
            "重要",
            "主力突袭",
            "14.28",
            "5.2",
            "0.8",
            "公司中标新能源项目",
            "5",
            "NewsCatalyst",
            "false",
        ] {
            assert!(
                prompt.contains(needle),
                "prompt 缺少字段: {needle}\n{prompt}"
            );
        }
    }

    #[test]
    fn prompt_absent_fields_are_explicit() {
        let mut record = sample_record();
        record.price = None;
        record.news_title = None;
        record.attribution_decision = None;
        let request = DeepAttributionRequest {
            record,
            as_of: Utc::now(),
        };
        let prompt = deep_attribution_prompt(&request);
        assert!(
            prompt.contains("absent"),
            "缺失字段必须明示 absent:\n{prompt}"
        );
    }

    #[test]
    fn parse_valid_output() {
        let out = parse_deep_attribution_output(
            r#"{"main_reason":"中标新能源项目催化","catalyst_chain":["公告催化","板块共振"],"capital_logic":"主力借题材吸筹","confidence":"medium","risk_note":"谨防冲高回落"}"#,
        )
        .expect("合法输出应解析");
        assert_eq!(out.main_reason, "中标新能源项目催化");
        assert_eq!(out.catalyst_chain.len(), 2);
        assert_eq!(out.confidence, "medium");
    }

    #[test]
    fn parse_accepts_markdown_fence() {
        let out = parse_deep_attribution_output(
            "```json\n{\"main_reason\":\"a\",\"catalyst_chain\":[\"b\"],\"capital_logic\":\"c\",\"confidence\":\"high\",\"risk_note\":\"d\"}\n```",
        )
        .expect("围栏包裹应剥离后解析");
        assert_eq!(out.main_reason, "a");
    }

    #[test]
    fn parse_rejects_missing_fields() {
        let err =
            parse_deep_attribution_output(r#"{"main_reason":"a"}"#).expect_err("缺字段必须报错");
        assert!(err.contains("catalyst_chain"), "错误应点名缺字段: {err}");
    }

    #[test]
    fn parse_rejects_invalid_confidence() {
        let err = parse_deep_attribution_output(
            r#"{"main_reason":"a","catalyst_chain":[],"capital_logic":"c","confidence":"extreme","risk_note":"d"}"#,
        )
        .expect_err("confidence 非法必须报错");
        assert!(err.contains("confidence"), "{err}");
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_deep_attribution_output("这不是 JSON").is_err());
    }

    #[test]
    fn top_events_priority_then_cap() {
        let mk = |level: &str| AlertRecord {
            level: level.to_string(),
            ..sample_record()
        };
        let mut records = vec![mk("参考"), mk("紧急"), mk("重要"), mk("参考")];
        records[0].triggered_at = "t1".to_string();
        records[1].triggered_at = "t2".to_string();
        records[2].triggered_at = "t3".to_string();
        records[3].triggered_at = "t4".to_string();
        let picked = top_events_for_deep(records, 3);
        assert_eq!(picked.len(), 3);
        assert_eq!(picked[0].level, "紧急");
        assert_eq!(picked[1].level, "重要");
        assert_eq!(picked[2].level, "参考");
    }

    #[test]
    fn top_events_empty_input() {
        assert!(top_events_for_deep(vec![], 3).is_empty());
    }

    #[test]
    fn top_events_excludes_legacy_test_code() {
        let mut record = sample_record();
        record.code = "TEST_CODE_000001".into();
        assert!(top_events_for_deep(vec![record], 3).is_empty());
    }

    #[test]
    fn top_events_excludes_test_origin_with_normal_code_and_honors_zero_cap() {
        let mut record = sample_record();
        record.origin = crate::monitor::alert_log::AlertRecordOrigin::Test;
        assert!(top_events_for_deep(vec![record], 3).is_empty());
        assert!(top_events_for_deep(vec![sample_record()], 0).is_empty());
    }

    #[test]
    fn journal_replays_selected_events_without_restarting_an_attempt() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("isolated-g5b-attempts");
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let journal = DeepAttributionJournal {
            dir: dir.clone(),
            production: false,
        };
        let mut second = sample_record();
        second.code = "600397".into();
        second.level = "参考".into();
        let selected = journal
            .load_or_select(date, vec![sample_record(), second])
            .expect("首次选集必须持久化");
        assert_eq!(selected.len(), 2);
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));

        // 模拟重启后告警文件新增了更高优先级的记录。已开始的分析不可重选/重算。
        let restarted = DeepAttributionJournal {
            dir,
            production: false,
        };
        let mut newer = sample_record();
        newer.code = "600001".into();
        newer.level = "紧急".into();
        let replay = restarted
            .load_or_select(date, vec![newer])
            .expect("重启后应使用原选集");
        assert_eq!(replay[0].code, selected[0].code);
        assert!(matches!(
            restarted.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::CompletionUnproven
        ));
        assert!(matches!(
            restarted.begin_assessment(date, 1).unwrap(),
            DeepAttributionClaim::Fresh
        ));
    }

    #[test]
    fn g5b_selected_events_read_existing_facts_without_creating_or_delivering() {
        use crate::durable_delivery::CoordinatorConfig;

        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        assert!(journal.inspect_existing_day(date).unwrap().is_none());
        assert!(!journal.dir.exists());

        let mut attempt_only = sample_record();
        attempt_only.code = "600397".into();
        let mut not_started = sample_record();
        not_started.code = "600398".into();
        let selected = journal
            .load_or_select(date, vec![sample_record(), attempt_only, not_started])
            .unwrap();
        let frozen_index = selected
            .iter()
            .position(|row| row.code == "600396")
            .unwrap();
        let attempt_index = selected
            .iter()
            .position(|row| row.code == "600397")
            .unwrap();
        assert!(matches!(
            journal.begin_assessment(date, frozen_index).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        assert!(matches!(
            journal.begin_assessment(date, attempt_index).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let summary = render_deep_attribution_summary(&row);
        let frozen = journal
            .freeze_result(date, frozen_index, &row, &summary)
            .unwrap();
        let selection_bytes = fs::read(journal.selection_path(date)).unwrap();
        let result_bytes = fs::read(journal.result_path(date, frozen_index)).unwrap();

        fs::create_dir_all("data/test").unwrap();
        let database_dir = tempfile::Builder::new()
            .prefix("TEST_CODE_G5B_DAY_")
            .tempdir_in("data/test")
            .unwrap();
        let test_code = database_dir.path().file_name().unwrap().to_str().unwrap();
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            database_dir.path().join("durable_delivery.sqlite3"),
            test_code,
            "owner-g5b-day-read-0123456789abcdef",
        ))
        .unwrap();

        let day = journal
            .inspect_selected_events_delivery(date, &coordinator)
            .unwrap()
            .unwrap();
        assert_eq!(day.events.len(), 3);
        assert_eq!(day.count(G5bSelectedEventState::NoCountedDecision), 1);
        assert_eq!(day.count(G5bSelectedEventState::CompletionUnproven), 1);
        assert_eq!(day.count(G5bSelectedEventState::NotStarted), 1);
        assert_eq!(day.events[frozen_index].archived, Some(false));
        assert!(day.events[frozen_index].decision_identity.is_none());
        assert!(!journal.archive_path(date).exists());
        assert_eq!(
            fs::read(journal.selection_path(date)).unwrap(),
            selection_bytes
        );
        assert_eq!(
            fs::read(journal.result_path(date, frozen_index)).unwrap(),
            result_bytes
        );

        journal.archive_frozen_result(date, frozen_index).unwrap();
        let archived = journal
            .inspect_selected_events_delivery(date, &coordinator)
            .unwrap()
            .unwrap();
        assert_eq!(archived.events[frozen_index].archived, Some(true));
        assert_eq!(
            archived.events[frozen_index].completion,
            G5bSelectedEventState::NoCountedDecision
        );
        assert_eq!(frozen.summary_sha256(), sha256_hex(summary.as_bytes()));
    }

    #[test]
    fn g5b_selected_events_preserve_counted_terminal_distinctions() {
        use G5bCountedTerminalV1 as Terminal;
        use G5bSelectedEventState as Selected;

        for (terminal, expected) in [
            (Terminal::Pending, Selected::Pending),
            (Terminal::Accepted, Selected::Accepted),
            (Terminal::ManualAccepted, Selected::ManualAccepted),
            (Terminal::Rejected, Selected::Rejected),
            (Terminal::Uncertain, Selected::Uncertain),
            (Terminal::ManualNotDelivered, Selected::ManualNotDelivered),
        ] {
            assert_eq!(selected_state_for_counted_terminal(terminal), expected);
        }
        assert_ne!(
            selected_state_for_counted_terminal(Terminal::Accepted),
            selected_state_for_counted_terminal(Terminal::ManualAccepted)
        );
    }

    #[test]
    fn g5b_completion_verdict_keeps_manual_and_negative_outcomes_distinct() {
        use G5bCompletionVerdict as Verdict;
        use G5bSelectedEventState as State;

        assert_eq!(classify_g5b_completion(None), Verdict::NoSelection);
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let observation = |states: &[(State, Option<bool>)]| G5bSelectedEventsObservation {
            business_date: date,
            events: states
                .iter()
                .enumerate()
                .map(
                    |(index, (completion, archived))| G5bSelectedEventObservation {
                        index,
                        record: sample_record(),
                        archived: *archived,
                        completion: *completion,
                        decision_identity: None,
                    },
                )
                .collect(),
        };

        let terminal = observation(&[
            (State::Accepted, Some(true)),
            (State::ManualAccepted, Some(true)),
            (State::Rejected, Some(true)),
            (State::ManualNotDelivered, Some(true)),
        ]);
        assert_eq!(
            classify_g5b_completion(Some(&terminal)),
            Verdict::TerminalOutcomesObserved(G5bCompletionCounts {
                selected: 4,
                accepted: 1,
                manual_accepted: 1,
                rejected: 1,
                manual_not_delivered: 1,
                ..G5bCompletionCounts::default()
            })
        );

        for incomplete_state in [
            State::NotStarted,
            State::CompletionUnproven,
            State::NoCountedDecision,
            State::Pending,
            State::Uncertain,
        ] {
            let day = observation(&[
                (State::Accepted, Some(true)),
                (incomplete_state, Some(true)),
            ]);
            assert!(matches!(
                classify_g5b_completion(Some(&day)),
                Verdict::Incomplete(_)
            ));
        }
        let missing_archive = observation(&[(State::Accepted, Some(false))]);
        assert_eq!(
            classify_g5b_completion(Some(&missing_archive)),
            Verdict::Incomplete(G5bCompletionCounts {
                selected: 1,
                accepted: 1,
                without_exact_archive: 1,
                ..G5bCompletionCounts::default()
            })
        );
        assert!(matches!(
            classify_g5b_completion(Some(&observation(&[]))),
            Verdict::Incomplete(_)
        ));
    }

    #[test]
    fn g5b_selected_events_reject_archive_without_matching_frozen_bytes() {
        let root = tempfile::tempdir().unwrap();
        let (journal, date, frozen) = prepared_archive_journal(root.path());
        let archive = journal.archive_path(date);
        let mut conflicting = sample_row();
        conflicting.result.main_reason = "另一份模型输出".into();
        fs::write(
            &archive,
            format!("{}\n", serde_json::to_string(&conflicting).unwrap()),
        )
        .unwrap();
        assert!(journal.inspect_existing_day(date).is_err());
        assert_eq!(
            fs::read(&archive).unwrap(),
            format!("{}\n", serde_json::to_string(&conflicting).unwrap()).as_bytes()
        );

        fs::write(&archive, format!("{}\n", frozen.row_json)).unwrap();
        let inspected = journal.inspect_existing_day(date).unwrap().unwrap();
        assert!(matches!(
            inspected[0].progress,
            DeepAttributionEventProgress::Frozen { archived: true, .. }
        ));
        fs::write(&archive, b"partial").unwrap();
        assert!(journal.inspect_existing_day(date).is_err());

        let mut orphan = sample_row();
        orphan.record.code = "600999".into();
        fs::write(
            &archive,
            format!("{}\n", serde_json::to_string(&orphan).unwrap()),
        )
        .unwrap();
        assert!(journal.inspect_existing_day(date).is_err());
    }

    #[test]
    fn frozen_result_survives_restart_with_exact_row_and_summary_bytes() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("isolated-g5b-attempts");
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let journal = DeepAttributionJournal {
            dir: dir.clone(),
            production: false,
        };
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let summary = render_deep_attribution_summary(&row);
        let frozen = journal.freeze_result(date, 0, &row, &summary).unwrap();
        assert_eq!(frozen.summary(), summary);
        assert_eq!(frozen.row_json, serde_json::to_string(&row).unwrap());
        assert_eq!(frozen.row_sha256(), sha256_hex(frozen.row_json.as_bytes()));
        assert_eq!(frozen.summary_sha256(), sha256_hex(summary.as_bytes()));

        let restarted = DeepAttributionJournal {
            dir,
            production: false,
        };
        match restarted.begin_assessment(date, 0).unwrap() {
            DeepAttributionClaim::Frozen(replayed) => {
                assert_eq!(replayed.row_json, frozen.row_json);
                assert_eq!(replayed.summary(), frozen.summary());
            }
            other => panic!("重启后应识别已冻结结果, 收到 {other:?}"),
        }
    }

    #[test]
    fn frozen_summary_tampering_with_matching_hash_blocks_restart() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let summary = render_deep_attribution_summary(&row);
        let mut frozen = journal.freeze_result(date, 0, &row, &summary).unwrap();
        frozen.summary.push_str("\n伪造后缀");
        frozen.summary_sha256 = sha256_hex(frozen.summary.as_bytes());
        fs::write(
            journal.result_path(date, 0),
            serde_json::to_vec(&frozen).unwrap(),
        )
        .unwrap();

        assert!(journal.begin_assessment(date, 0).is_err());
    }

    #[test]
    fn non_regular_attempt_marker_blocks_analysis() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        fs::create_dir(journal.attempt_path(date, 0)).unwrap();
        assert!(journal.begin_assessment(date, 0).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_frozen_result_blocks_restart() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let summary = render_deep_attribution_summary(&row);
        journal.freeze_result(date, 0, &row, &summary).unwrap();
        let result = journal.result_path(date, 0);
        let target = root.path().join("frozen-result.json");
        fs::rename(&result, &target).unwrap();
        symlink(&target, &result).unwrap();
        assert!(journal.begin_assessment(date, 0).is_err());
    }

    #[test]
    fn frozen_event_archive_is_idempotent_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("isolated-g5b-attempts");
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let journal = DeepAttributionJournal {
            dir: dir.clone(),
            production: false,
        };
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let summary = render_deep_attribution_summary(&row);
        let frozen = journal.freeze_result(date, 0, &row, &summary).unwrap();

        assert_eq!(
            journal.archive_frozen_result(date, 0).unwrap(),
            DeepAttributionArchiveOutcome::Appended
        );
        let archive = root.path().join(format!("{date}.jsonl"));
        let expected = format!("{}\n", frozen.row_json);
        assert_eq!(fs::read(&archive).unwrap(), expected.as_bytes());

        let restarted = DeepAttributionJournal {
            dir,
            production: false,
        };
        assert_eq!(
            restarted.archive_frozen_result(date, 0).unwrap(),
            DeepAttributionArchiveOutcome::AlreadyPresent
        );
        assert_eq!(fs::read(&archive).unwrap(), expected.as_bytes());
    }

    #[test]
    fn archive_rejects_partial_or_malformed_historical_rows_without_rewrite() {
        let root = tempfile::tempdir().unwrap();
        let (journal, date, _) = prepared_archive_journal(root.path());
        let archive = journal.archive_path(date);
        let no_newline = serde_json::to_string(&sample_row()).unwrap();
        for broken in [
            b"{\"record\":".as_slice(),
            no_newline.as_bytes(),
            b"not-json\n".as_slice(),
        ] {
            fs::write(&archive, broken).unwrap();
            assert!(journal.archive_frozen_result(date, 0).is_err());
            assert_eq!(fs::read(&archive).unwrap(), broken);
        }
    }

    #[test]
    fn archive_rejects_conflicting_or_duplicate_counted_occurrence() {
        let root = tempfile::tempdir().unwrap();
        let (journal, date, frozen) = prepared_archive_journal(root.path());
        let archive = journal.archive_path(date);
        let mut conflicting = sample_row();
        conflicting.result.main_reason = "另一份模型输出".into();
        let historical = format!("{}\n", serde_json::to_string(&conflicting).unwrap());
        fs::write(&archive, &historical).unwrap();
        assert!(journal.archive_frozen_result(date, 0).is_err());
        assert_eq!(fs::read_to_string(&archive).unwrap(), historical);

        let duplicate = format!("{0}\n{0}\n", frozen.row_json);
        fs::write(&archive, &duplicate).unwrap();
        assert!(journal.archive_frozen_result(date, 0).is_err());
        assert_eq!(fs::read_to_string(&archive).unwrap(), duplicate);
    }

    #[test]
    fn archive_preserves_unrelated_historical_bytes_after_orphan_temp() {
        let root = tempfile::tempdir().unwrap();
        let (journal, date, frozen) = prepared_archive_journal(root.path());
        let archive = journal.archive_path(date);
        let mut other = sample_row();
        other.record.code = "600001".into();
        let historical = format!("{}\r\n", serde_json::to_string(&other).unwrap());
        fs::write(&archive, &historical).unwrap();
        let orphan = root.path().join(format!(".{date}.jsonl.tmp.crashed"));
        fs::write(&orphan, b"{broken").unwrap();

        assert_eq!(
            journal.archive_frozen_result(date, 0).unwrap(),
            DeepAttributionArchiveOutcome::Appended
        );
        assert_eq!(
            fs::read(&archive).unwrap(),
            format!("{historical}{}\n", frozen.row_json).as_bytes()
        );
        assert_eq!(fs::read(&orphan).unwrap(), b"{broken");
    }

    #[test]
    fn archive_recovery_after_window_needs_no_provider_or_new_attempt() {
        let root = tempfile::tempdir().unwrap();
        let (journal, date, frozen) = prepared_archive_journal(root.path());
        let after_window = date.and_hms_opt(15, 21, 0).unwrap();

        let first = journal
            .recover_existing_frozen_archives(after_window)
            .unwrap();
        assert_eq!(first.appended, 1);
        assert_eq!(first.already_present, 0);
        assert_eq!(first.completion_unproven, 0);
        let archive = journal.archive_path(date);
        let expected = format!("{}\n", frozen.row_json);
        assert_eq!(fs::read(&archive).unwrap(), expected.as_bytes());

        let second = journal
            .recover_existing_frozen_archives(after_window)
            .unwrap();
        assert_eq!(second.appended, 0);
        assert_eq!(second.already_present, 1);
        assert_eq!(fs::read(&archive).unwrap(), expected.as_bytes());
        assert!(!journal.attempt_path(date, 1).exists());
    }

    #[test]
    fn bad_older_archive_does_not_block_newer_date_and_retry_is_targeted() {
        let root = tempfile::tempdir().unwrap();
        let (journal, older, _) = prepared_archive_journal(root.path());
        let newer = older.succ_opt().unwrap();
        journal
            .load_or_select(newer, vec![sample_record()])
            .unwrap();
        assert!(matches!(
            journal.begin_assessment(newer, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let row = sample_row();
        let newer_frozen = journal
            .freeze_result(newer, 0, &row, &render_deep_attribution_summary(&row))
            .unwrap();
        let broken = b"{partial";
        fs::write(journal.archive_path(older), broken).unwrap();

        let report = journal
            .recover_existing_frozen_archives(newer.and_hms_opt(15, 21, 0).unwrap())
            .unwrap();
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].0, older);
        assert_eq!(report.appended, 1);
        assert_eq!(fs::read(journal.archive_path(older)).unwrap(), broken);
        let newer_bytes = format!("{}\n", newer_frozen.row_json).into_bytes();
        assert_eq!(fs::read(journal.archive_path(newer)).unwrap(), newer_bytes);

        fs::write(journal.archive_path(older), b"").unwrap();
        let retried = journal
            .recover_frozen_archives_for_dates(report.failures.iter().map(|(date, _)| *date))
            .unwrap();
        assert!(retried.failures.is_empty());
        assert_eq!(retried.appended, 1);
        assert_eq!(retried.already_present, 0);
        assert_eq!(fs::read(journal.archive_path(newer)).unwrap(), newer_bytes);
    }

    #[test]
    fn archive_recovery_without_existing_selection_creates_nothing() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("never-created-attempts"),
            production: false,
        };
        let after_window = NaiveDate::from_ymd_opt(2026, 9, 20)
            .unwrap()
            .and_hms_opt(15, 21, 0)
            .unwrap();

        let report = journal
            .recover_existing_frozen_archives(after_window)
            .unwrap();
        assert_eq!(report.appended, 0);
        assert_eq!(report.already_present, 0);
        assert_eq!(report.completion_unproven, 0);
        assert!(!journal.dir.exists());
        assert!(!journal.archive_path(after_window.date()).exists());
    }

    #[test]
    fn archive_recovery_leaves_attempt_only_unresolved() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));

        let report = journal
            .recover_existing_frozen_archives(date.and_hms_opt(15, 21, 0).unwrap())
            .unwrap();
        assert_eq!(report.completion_unproven, 1);
        assert!(!journal.result_path(date, 0).exists());
        assert!(!journal.archive_path(date).exists());
    }

    #[test]
    fn partial_frozen_result_blocks_reanalysis() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        fs::write(journal.result_path(date, 0), b"{\"schema_version\":1").unwrap();
        assert!(journal.begin_assessment(date, 0).is_err());
    }

    #[test]
    fn frozen_result_without_attempt_blocks_new_call() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        fs::write(journal.result_path(date, 0), b"{}").unwrap();
        assert!(journal.begin_assessment(date, 0).is_err());
        assert!(!journal.attempt_path(date, 0).exists());
    }

    #[test]
    fn incomplete_selection_fails_closed_without_reselection() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        fs::create_dir_all(&journal.dir).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        fs::write(journal.selection_path(date), b"{\"schema_version\":1").unwrap();
        assert!(journal.load_or_select(date, vec![sample_record()]).is_err());
        assert!(!journal.attempt_path(date, 0).exists());
    }

    #[test]
    fn legacy_result_without_attempt_journal_blocks_new_analysis() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        fs::write(root.path().join(format!("{date}.jsonl")), b"old result\n").unwrap();
        assert!(journal.load_or_select(date, vec![sample_record()]).is_err());
        assert!(!journal.selection_path(date).exists());
    }

    #[test]
    fn orphan_attempt_without_selection_blocks_reselection() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        fs::create_dir_all(&journal.dir).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        fs::write(journal.attempt_path(date, 0), b"").unwrap();
        assert!(journal.load_or_select(date, vec![sample_record()]).is_err());
        assert!(!journal.selection_path(date).exists());
    }

    #[test]
    fn orphan_archive_lock_without_selection_blocks_reselection() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        fs::create_dir_all(&journal.dir).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        fs::write(journal.dir.join(format!("{date}.archive.lock")), b"").unwrap();

        assert!(journal.load_or_select(date, vec![sample_record()]).is_err());
        assert!(!journal.selection_path(date).exists());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_legacy_archive_blocks_new_selection() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        symlink(
            root.path().join("missing-target"),
            journal.archive_path(date),
        )
        .unwrap();

        assert!(journal.load_or_select(date, vec![sample_record()]).is_err());
        assert!(!journal.selection_path(date).exists());
    }

    #[test]
    fn production_journal_rejects_test_process_before_io() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        assert!(DeepAttributionJournal::production()
            .load_or_select(date, vec![sample_record()])
            .is_err());
    }

    #[test]
    fn frozen_result_rejects_ineligible_record_before_archive_io() {
        let root = tempfile::tempdir().unwrap();
        let journal = DeepAttributionJournal {
            dir: root.path().join("isolated-g5b-attempts"),
            production: false,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        journal.load_or_select(date, vec![sample_record()]).unwrap();
        assert!(matches!(
            journal.begin_assessment(date, 0).unwrap(),
            DeepAttributionClaim::Fresh
        ));
        let mut row = sample_row();
        row.record.code = "TEST_CODE_000001".into();
        let summary = render_deep_attribution_summary(&row);
        assert!(journal.freeze_result(date, 0, &row, &summary).is_err());
        assert!(!journal.result_path(date, 0).exists());
        assert!(!journal.archive_path(date).exists());
    }

    #[test]
    fn render_includes_core_fields() {
        let row = DeepAttributionRow {
            record: sample_record(),
            result: parse_deep_attribution_output(
                r#"{"main_reason":"中标催化","catalyst_chain":["公告"],"capital_logic":"吸筹","confidence":"medium","risk_note":"回落"}"#,
            )
            .unwrap(),
            analyzed_at: "2026-08-22T15:06:00Z".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            upstream_request_id: None,
            upstream_response_id: Some("resp-1".to_string()),
            elapsed_ms: 12_345,
        };
        let md = render_deep_attribution(&row);
        for needle in [
            "600396",
            "金山股份",
            "中标催化",
            "公告",
            "吸筹",
            "medium",
            "回落",
            "deepseek",
        ] {
            assert!(md.contains(needle), "渲染缺少: {needle}\n{md}");
        }
        let summary = render_deep_attribution_summary(&row);
        assert!(summary.contains("600396") && summary.contains("中标催化"));
    }

    /// mock provider: 合法回执 + 固定响应文本 (LlmError 不可 Clone, 用开关构造)。
    struct MockDeepProvider {
        raw_response: String,
        fail_with_api: bool,
    }

    struct PanicIfCalledProvider;

    #[async_trait::async_trait]
    impl LlmProvider for PanicIfCalledProvider {
        fn name(&self) -> &'static str {
            "panic-if-called"
        }
        fn model(&self) -> &str {
            "none"
        }
        async fn chat_json(&self, _system: &str, _user: &str) -> Result<Value, LlmError> {
            panic!("ineligible record reached provider")
        }
        async fn chat_json_with_receipt(
            &self,
            _system: &str,
            _user: &str,
        ) -> Result<ReceiptBearingJson, LlmError> {
            panic!("ineligible record reached provider")
        }
    }

    #[tokio::test]
    async fn assess_rejects_test_origin_before_provider_call() {
        let mut request = sample_request();
        request.record.origin = crate::monitor::alert_log::AlertRecordOrigin::Test;
        let analyzer = DeepAttributionAnalyzer::new(Arc::new(PanicIfCalledProvider));
        let error = analyzer
            .assess(&request)
            .await
            .expect_err("测试来源必须拒绝");
        assert!(matches!(error, DeepAttributionError::IneligibleRecord(_)));
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockDeepProvider {
        fn name(&self) -> &'static str {
            "mock-deep"
        }
        fn model(&self) -> &str {
            "mock-model"
        }
        async fn chat_json(&self, _system: &str, _user: &str) -> Result<Value, LlmError> {
            Ok(serde_json::from_str(&self.raw_response).unwrap_or(Value::Null))
        }
        async fn chat_json_with_receipt(
            &self,
            system: &str,
            user: &str,
        ) -> Result<ReceiptBearingJson, LlmError> {
            if self.fail_with_api {
                return Err(LlmError::Api {
                    status: 500,
                    body: "mock server boom".to_string(),
                });
            }
            Ok(ReceiptBearingJson::test_fixture(
                "mock-deep",
                "mock-model",
                None,
                "mock-response-id",
                system,
                user,
                &self.raw_response,
                Utc::now() - chrono::Duration::seconds(1),
                Utc::now(),
            ))
        }
    }

    #[tokio::test]
    async fn assess_with_mock_provider_succeeds() {
        let provider = Arc::new(MockDeepProvider {
            raw_response: r#"{"main_reason":"中标催化","catalyst_chain":["公告"],"capital_logic":"吸筹","confidence":"high","risk_note":"回落"}"#.to_string(),
            fail_with_api: false,
        });
        let analyzer = DeepAttributionAnalyzer::new(provider);
        let outcome = analyzer
            .assess(&sample_request())
            .await
            .expect("mock 应成功");
        assert_eq!(outcome.result.main_reason, "中标催化");
        assert_eq!(outcome.result.confidence, "high");
        assert_eq!(outcome.receipt.provider(), "mock-deep");
        assert_eq!(
            outcome.receipt.upstream_response_id(),
            Some("mock-response-id")
        );
    }

    #[tokio::test]
    async fn assess_mock_bad_schema_fails_loudly() {
        let provider = Arc::new(MockDeepProvider {
            raw_response: r#"{"main_reason":"a"}"#.to_string(),
            fail_with_api: false,
        });
        let analyzer = DeepAttributionAnalyzer::new(provider);
        let err = analyzer
            .assess(&sample_request())
            .await
            .expect_err("schema 缺失必须失败");
        assert!(
            matches!(err, DeepAttributionError::InvalidModelSchema(_)),
            "非法输出必须是 InvalidModelSchema: {err:?}"
        );
    }

    #[tokio::test]
    async fn assess_mock_call_error_fails_loudly() {
        let provider = Arc::new(MockDeepProvider {
            raw_response: String::new(),
            fail_with_api: true,
        });
        let analyzer = DeepAttributionAnalyzer::new(provider);
        let err = analyzer
            .assess(&sample_request())
            .await
            .expect_err("API 错误必须失败");
        assert!(
            matches!(err, DeepAttributionError::ModelUnavailable(_)),
            "调用失败必须是 ModelUnavailable: {err:?}"
        );
    }
}
