//! 修复 Top10#3+#4: chain_analysis.rs (1839 行) 拆 3 子模块
//!
//! 这个文件: `chain_analysis/fetchers.rs` — 数据获取 helpers
//!
//! 包含产业链附加证据获取 helpers。
//! 拆分后 mod.rs 从 1839 → 1469 行 (-20%)

//! 子模块互见: 在 mod.rs 把 fetchers 声明为 super 模块, 这里用 super::xxx 调入 fetchers.

use futures::stream::{self, StreamExt};
use log::info;
#[cfg(test)]
use log::warn;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;

use crate::agent::tool::Tool;
use crate::agent::tools_sector::FetchSectorTool;
use crate::data_gateway::{
    BatchEvidence, BoardDataGateway, BoardKind, DragonTigerGateway, DragonTigerStockReview,
    GatewayBatch,
};
use crate::database::concepts::{LocalConceptCacheRead, LocalConceptCacheRow};
use crate::database::DatabaseManager;
use crate::market_data::TopStock;
use crate::market_domain::EvidenceTimestamp;

use super::ChainCluster;
// is_generic_board 在 mod.rs 是 pub(super) — 让 fetchers 可见
use super::is_generic_board;

#[cfg(test)]
type SearchFuture = futures::future::BoxFuture<'static, Vec<crate::search_service::SearchResult>>;

/// 获取指定代码集的概念标签：优先 7 天内缓存，缺失的并发拉取并落库。
pub(super) async fn fetch_concepts_cached(
    codes: &[String],
) -> Result<HashMap<String, Vec<String>>, String> {
    fetch_concepts_cached_observed(codes)
        .await
        .map(|result| result.legacy_map)
        .map_err(ObservedConceptFetchError::into_legacy_error)
}

#[derive(Debug)]
pub(super) struct ObservedConceptFetch {
    pub(super) legacy_map: HashMap<String, Vec<String>>,
    pub(super) observation: ObservedConceptProjection,
}

#[derive(Debug)]
pub(super) enum ObservedConceptFetchError {
    BeforeCache {
        legacy_error: String,
    },
    AfterCache {
        legacy_error: String,
        observation: ObservedConceptProjection,
    },
}

impl ObservedConceptFetchError {
    fn into_legacy_error(self) -> String {
        match self {
            Self::BeforeCache { legacy_error } | Self::AfterCache { legacy_error, .. } => {
                legacy_error
            }
        }
    }
}

/// Retain evidence from the same cache read and provider calls as the legacy
/// fetch. The old public projection still comes from `legacy_map`.
pub(super) async fn fetch_concepts_cached_observed(
    codes: &[String],
) -> Result<ObservedConceptFetch, ObservedConceptFetchError> {
    if codes.is_empty() || codes.iter().any(|code| code.trim().is_empty()) {
        return Err(ObservedConceptFetchError::BeforeCache {
            legacy_error: "产业链概念批次代码为空".to_string(),
        });
    }
    let db = DatabaseManager::try_get().ok_or_else(|| ObservedConceptFetchError::BeforeCache {
        legacy_error: "产业链概念缓存数据库未初始化".to_string(),
    })?;
    let tool = FetchSectorTool::new();
    fetch_concepts_cached_observed_in(db, codes, |code| {
        let tool = &tool;
        async move { fetch_boards_raw(tool, &code).await }
    })
    .await
}

pub(super) async fn fetch_concepts_cached_observed_in<F, Fut>(
    db: &DatabaseManager,
    codes: &[String],
    fetch_raw: F,
) -> Result<ObservedConceptFetch, ObservedConceptFetchError>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let cache_read = db
        .get_cached_concepts_observed(7)
        .map_err(|legacy_error| ObservedConceptFetchError::BeforeCache { legacy_error })?;
    let map = cache_read.clone().into_map();

    let missing: Vec<String> = codes
        .iter()
        .filter(|c| !map.contains_key(*c))
        .cloned()
        .collect();

    if !missing.is_empty() {
        info!(
            "[产业链] 概念缓存命中 {}/{}，在线拉取 {} 只...",
            codes.len() - missing.len(),
            codes.len(),
            missing.len()
        );
        let fetched: Vec<(String, Result<(String, Vec<String>), String>)> = stream::iter(missing)
            .map(|code| {
                let fetch_raw = &fetch_raw;
                async move {
                    let result = fetch_raw(code.clone()).await.and_then(|raw| {
                        let boards = parse_tool_boards(&raw, &code)?;
                        Ok((raw, boards))
                    });
                    (code, result)
                }
            })
            .buffer_unordered(6)
            .collect()
            .await;
        return apply_fetched_concepts(db, codes, cache_read, map, fetched);
    }
    apply_fetched_concepts(db, codes, cache_read, map, Vec::new())
}

fn apply_fetched_concepts(
    db: &DatabaseManager,
    codes: &[String],
    cache_read: LocalConceptCacheRead,
    mut map: HashMap<String, Vec<String>>,
    fetched: Vec<(String, Result<(String, Vec<String>), String>)>,
) -> Result<ObservedConceptFetch, ObservedConceptFetchError> {
    let mut written = Vec::<(String, Vec<String>, String)>::new();
    for (code, result) in fetched {
        let (raw, boards) = match result {
            Ok(value) => value,
            Err(legacy_error) => {
                return Err(ObservedConceptFetchError::AfterCache {
                    legacy_error,
                    observation: observe_written_concepts(
                        codes,
                        cache_read,
                        written,
                        ConceptFetchTerminal::Failed,
                    ),
                });
            }
        };
        if let Err(legacy_error) = db.save_stock_concepts(&code, &boards) {
            return Err(ObservedConceptFetchError::AfterCache {
                legacy_error,
                observation: observe_written_concepts(
                    codes,
                    cache_read,
                    written,
                    ConceptFetchTerminal::Failed,
                ),
            });
        }
        map.insert(code.clone(), boards.clone());
        written.push((code, boards, raw));
    }
    if codes.iter().any(|code| !map.contains_key(code)) {
        return Err(ObservedConceptFetchError::AfterCache {
            legacy_error: "产业链概念批次未覆盖全部股票代码".to_string(),
            observation: observe_written_concepts(
                codes,
                cache_read,
                written,
                ConceptFetchTerminal::Failed,
            ),
        });
    }
    Ok(ObservedConceptFetch {
        legacy_map: map,
        observation: observe_written_concepts(
            codes,
            cache_read,
            written,
            ConceptFetchTerminal::Completed,
        ),
    })
}

fn observe_written_concepts(
    codes: &[String],
    cache_read: LocalConceptCacheRead,
    written: Vec<(String, Vec<String>, String)>,
    terminal: ConceptFetchTerminal,
) -> ObservedConceptProjection {
    let writes = written
        .into_iter()
        .map(
            |(code, boards, raw)| match parse_tool_boards_observed(&raw, &code) {
                Ok(observation) if observation.boards == boards => {
                    ObservedConceptCacheWrite::ToolObservation(observation)
                }
                _ => ObservedConceptCacheWrite::LegacyProjection {
                    code,
                    boards,
                    raw_response_sha256: format!("{:x}", Sha256::digest(raw.as_bytes())),
                },
            },
        )
        .collect();
    compose_observed_concepts(codes, cache_read, writes, terminal)
        .expect("concept codes validated before cache query")
}

/// 调 FetchSectorTool 拉单只股票的完整板块列表。
pub(super) async fn fetch_boards_via_tool(
    tool: &FetchSectorTool,
    code: &str,
) -> Result<Vec<String>, String> {
    let raw = fetch_boards_raw(tool, code).await?;
    parse_tool_boards(&raw, code)
}

/// Fetch one provider response without parsing it so durable callers can first
/// preserve the exact returned bytes. The legacy path immediately parses it.
pub(super) async fn fetch_boards_raw(tool: &FetchSectorTool, code: &str) -> Result<String, String> {
    tool.call(json!({ "code": code }))
        .await
        .map_err(|error| format_membership_fetch_failure(code, &error))
}

pub(crate) fn format_membership_fetch_failure(code: &str, error: &dyn std::fmt::Display) -> String {
    format!("产业链 {code} 板块拉取失败: {error}")
}

/// BR-114: validate a complete sector-tool response before it enters the cache.
pub(super) fn parse_tool_boards(raw: &str, code: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("产业链 {code} 板块 JSON 非法: {error}"))?;
    parse_tool_board_values(&value, code)
}

/// Syntactic evidence from one tool response. The provider label is opaque;
/// this value does not qualify an upstream provider batch as Available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolBoardsObservation {
    pub(super) requested_code: String,
    pub(super) all_boards: Vec<String>,
    pub(super) boards: Vec<String>,
    pub(super) board_count: usize,
    pub(super) evidence: OpaqueToolBatchEvidence,
    pub(super) raw_response_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpaqueToolBatchEvidence {
    pub(super) provider_label: String,
    pub(super) source: String,
    pub(super) source_at: Option<String>,
    pub(super) observed_at: String,
    pub(super) batch_id: String,
}

/// Parse the same raw bytes returned by the existing tool call. No additional
/// gateway call is made, and the SHA-256 covers the unmodified response bytes.
pub(super) fn parse_tool_boards_observed(
    raw: &str,
    code: &str,
) -> Result<ToolBoardsObservation, String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("产业链 {code} 板块 JSON 非法: {error}"))?;
    if value.get("fetched") != Some(&serde_json::Value::Bool(true)) {
        return Err(format!("产业链 {code} 工具结果未确认已拉取"));
    }
    if required_tool_text(&value, "secucode", code)? != code {
        return Err(format!("产业链 {code} 工具结果代码不匹配"));
    }
    let rows = value
        .get("all_boards")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("产业链 {code} 缺少 all_boards 数组"))?;
    let board_count = value
        .get("board_count")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("产业链 {code} board_count 非法"))?;
    let actual_board_count =
        u64::try_from(rows.len()).map_err(|_| format!("产业链 {code} all_boards 过大"))?;
    if board_count != actual_board_count {
        return Err(format!("产业链 {code} board_count 与 all_boards 不一致"));
    }
    let boards = parse_tool_board_values(&value, code)?;
    let all_boards = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            row.as_str()
                .filter(|board| !board.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("产业链 {code} all_boards[{index}] 非法"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let evidence = value
        .get("evidence")
        .filter(|evidence| evidence.is_object())
        .ok_or_else(|| format!("产业链 {code} 缺少 evidence 对象"))?;
    let provider_label = required_tool_text(evidence, "provider", code)?.to_owned();
    let source = required_tool_text(evidence, "source", code)?.to_owned();
    let observed_at = required_tool_text(evidence, "observed_at", code)?.to_owned();
    EvidenceTimestamp::parse_instant(&observed_at)
        .map_err(|_| format!("产业链 {code} observed_at 非明确时刻"))?;
    let batch_id = required_tool_text(evidence, "batch_id", code)?.to_owned();
    let source_at = match evidence.get("source_at") {
        Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            EvidenceTimestamp::parse(value)
                .map_err(|_| format!("产业链 {code} source_at 非有效时间"))?;
            Some(value.clone())
        }
        _ => return Err(format!("产业链 {code} source_at 字段非法或缺失")),
    };
    Ok(ToolBoardsObservation {
        requested_code: code.to_owned(),
        all_boards,
        boards,
        board_count: rows.len(),
        evidence: OpaqueToolBatchEvidence {
            provider_label,
            source,
            source_at,
            observed_at,
            batch_id,
        },
        raw_response_sha256: format!("{:x}", Sha256::digest(raw.as_bytes())),
    })
}

fn required_tool_text<'a>(
    value: &'a serde_json::Value,
    field: &str,
    code: &str,
) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("产业链 {code} 工具结果 {field} 字段非法或缺失"))
}

fn parse_tool_board_values(value: &serde_json::Value, code: &str) -> Result<Vec<String>, String> {
    let rows = value
        .get("all_boards")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("产业链 {code} 缺少 all_boards 数组"))?;
    if rows.is_empty() {
        return Err(format!("产业链 {code} all_boards 为空"));
    }
    let mut boards = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let board = row
            .as_str()
            .filter(|board| !board.trim().is_empty())
            .ok_or_else(|| format!("产业链 {code} all_boards[{index}] 非法"))?;
        if !boards.iter().any(|existing| existing == board) {
            boards.push(board.to_string());
        }
    }
    Ok(boards)
}

/// A provider result recorded only after the corresponding legacy cache write
/// succeeds. The vector of these records preserves actual write order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ObservedConceptCacheWrite {
    ToolObservation(ToolBoardsObservation),
    /// Legacy boards were valid and written, but strict evidence was not.
    LegacyProjection {
        code: String,
        boards: Vec<String>,
        raw_response_sha256: String,
    },
}

impl ObservedConceptCacheWrite {
    fn code(&self) -> &str {
        match self {
            Self::ToolObservation(observation) => &observation.requested_code,
            Self::LegacyProjection { code, .. } => code,
        }
    }

    fn boards(&self) -> &[String] {
        match self {
            Self::ToolObservation(observation) => &observation.boards,
            Self::LegacyProjection { boards, .. } => boards,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConceptFetchTerminal {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConceptProjectionCompletion {
    Complete { content_sha256: String },
    Incomplete { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConceptCodeEvidence {
    LocalCache(LocalConceptCacheRow),
    ToolResponse(ToolBoardsObservation),
    LegacyToolProjection { raw_response_sha256: String },
}

/// Requested-code content and per-code origin from one cache read plus the
/// provider writes that actually succeeded. Complete here describes only the
/// local projection, not upstream provider admission or M1 source availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedConceptProjection {
    pub(super) cache_read: LocalConceptCacheRead,
    pub(super) requested_codes: Vec<String>,
    pub(super) concepts: BTreeMap<String, Vec<String>>,
    pub(super) sources: BTreeMap<String, ConceptCodeEvidence>,
    pub(super) successful_writes: Vec<ObservedConceptCacheWrite>,
    pub(super) completion: ConceptProjectionCompletion,
}

/// Pure composition seam for the existing single-read, multi-provider flow.
/// `successful_writes` must be recorded in the legacy write loop immediately
/// after each successful `save_stock_concepts`; a failed terminal always stays
/// incomplete even when an earlier duplicate write covered every code.
pub(super) fn compose_observed_concepts(
    requested_codes: &[String],
    cache_read: LocalConceptCacheRead,
    successful_writes: Vec<ObservedConceptCacheWrite>,
    terminal: ConceptFetchTerminal,
) -> Result<ObservedConceptProjection, String> {
    if requested_codes.is_empty() || requested_codes.iter().any(|code| code.trim().is_empty()) {
        return Err("产业链概念批次代码为空".to_string());
    }
    let requested = requested_codes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut concepts = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for row in cache_read.rows() {
        if requested.contains(row.code()) {
            concepts.insert(row.code().to_owned(), row.concepts().to_vec());
            sources.insert(
                row.code().to_owned(),
                ConceptCodeEvidence::LocalCache(row.clone()),
            );
        }
    }
    let mut expected_provider_writes = BTreeMap::<String, usize>::new();
    for code in requested_codes {
        if !sources.contains_key(code) {
            *expected_provider_writes.entry(code.clone()).or_default() += 1;
        }
    }
    let mut observed_provider_writes = BTreeMap::<String, usize>::new();
    let mut unexpected_write = false;
    let mut incomplete_tool_evidence = false;
    for write in &successful_writes {
        let code = write.code();
        *observed_provider_writes.entry(code.to_owned()).or_default() += 1;
        if !requested.contains(code)
            || matches!(sources.get(code), Some(ConceptCodeEvidence::LocalCache(_)))
        {
            unexpected_write = true;
        }
        if requested.contains(code) {
            concepts.insert(code.to_owned(), write.boards().to_vec());
            let evidence = match write {
                ObservedConceptCacheWrite::ToolObservation(observation) => {
                    ConceptCodeEvidence::ToolResponse(observation.clone())
                }
                ObservedConceptCacheWrite::LegacyProjection {
                    raw_response_sha256,
                    ..
                } => {
                    incomplete_tool_evidence = true;
                    ConceptCodeEvidence::LegacyToolProjection {
                        raw_response_sha256: raw_response_sha256.clone(),
                    }
                }
            };
            sources.insert(code.to_owned(), evidence);
        }
    }
    let completion = if terminal == ConceptFetchTerminal::Failed {
        ConceptProjectionCompletion::Incomplete {
            reason: if successful_writes.is_empty() {
                "fetch_failed_before_any_write"
            } else {
                "fetch_failed_after_partial_writes"
            },
        }
    } else if unexpected_write {
        ConceptProjectionCompletion::Incomplete {
            reason: "provider_write_did_not_match_cache_miss",
        }
    } else if requested.iter().any(|code| !concepts.contains_key(*code)) {
        ConceptProjectionCompletion::Incomplete {
            reason: "requested_code_uncovered",
        }
    } else if expected_provider_writes != observed_provider_writes {
        ConceptProjectionCompletion::Incomplete {
            reason: "provider_write_cardinality_mismatch",
        }
    } else if incomplete_tool_evidence {
        ConceptProjectionCompletion::Incomplete {
            reason: "tool_evidence_incomplete",
        }
    } else {
        ConceptProjectionCompletion::Complete {
            content_sha256: requested_concept_projection_sha256(requested_codes, &concepts),
        }
    };
    Ok(ObservedConceptProjection {
        cache_read,
        requested_codes: requested_codes.to_vec(),
        concepts,
        sources,
        successful_writes,
        completion,
    })
}

pub(super) fn requested_concept_projection_sha256(
    requested_codes: &[String],
    concepts: &BTreeMap<String, Vec<String>>,
) -> String {
    fn text_field(hash: &mut Sha256, value: &str) {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    let mut hash = Sha256::new();
    hash.update(b"stock_analysis.chain.requested_concept_projection.v1\0");
    hash.update((requested_codes.len() as u64).to_be_bytes());
    for code in requested_codes {
        text_field(&mut hash, code);
    }
    hash.update((concepts.len() as u64).to_be_bytes());
    for (code, boards) in concepts {
        text_field(&mut hash, code);
        hash.update((boards.len() as u64).to_be_bytes());
        for board in boards {
            text_field(&mut hash, board);
        }
    }
    format!("{:x}", hash.finalize())
}

pub(super) struct BoardCodeMapBatch {
    pub(super) codes: HashMap<String, String>,
    pub(super) evidence: Vec<BatchEvidence>,
}

/// 通过统一 Magic TDX Gateway 拉取行业和概念目录，并保留每个完整批次证据。
pub(super) async fn fetch_board_code_map() -> Result<BoardCodeMapBatch, String> {
    let mut map = HashMap::new();
    let mut evidence = Vec::new();
    for kind in [BoardKind::Industry, BoardKind::Concept] {
        let batch = BoardDataGateway::new()
            .directory(kind, 10_000)
            .await
            .map_err(|error| format!("产业链板块目录不可用 ({kind:?}): {error}"))?;
        fold_board_directory_kind(&mut map, &mut evidence, kind, batch)?;
    }
    if map.is_empty() {
        Err("Magic TDX 板块目录没有可用记录".to_string())
    } else {
        Ok(BoardCodeMapBatch {
            codes: map,
            evidence,
        })
    }
}

pub(crate) fn fold_board_directory_kind(
    map: &mut HashMap<String, String>,
    evidence: &mut Vec<BatchEvidence>,
    kind: BoardKind,
    batch: GatewayBatch<crate::data_gateway::BoardDirectoryFact>,
) -> Result<(), String> {
    let records = match batch {
        GatewayBatch::Available {
            records,
            evidence: batch_evidence,
        } => {
            evidence.push(batch_evidence);
            records
        }
        GatewayBatch::VerifiedEmpty(evidence) => {
            return Err(format!(
                "产业链板块目录已验证为空 ({kind:?}): provider={:?} source={} \
                 observed_at={} batch_id={}",
                evidence.provider, evidence.source, evidence.observed_at, evidence.batch_id
            ));
        }
    };
    for record in records {
        match map.insert(record.name.clone(), record.code.clone()) {
            Some(previous) if previous != record.code => {
                return Err(format!(
                    "产业链板块名称跨类别冲突: {} => {previous}/{}",
                    record.name, record.code
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// 当前发布的 Magic TDX 成分合同没有同批次价格、涨幅和证券名称。
/// 补涨筛选必须等待上游发布完整合同，不能再拼接旧行情源。
pub(super) async fn fetch_laggard_candidates(
    board_code: &str,
    _limit_codes: &HashSet<String>,
) -> Result<GatewayBatch<TopStock>, String> {
    Err(format!(
        "产业链补涨候选 unsupported: board={board_code}; \
         Magic TDX 当前只提供成分身份，不提供同批次名称/价格/涨幅，禁止跨源拼接"
    ))
}

/// 今日龙虎榜净买入映射 code -> 净买额(万元)。
/// 2026-08-06 实证: disclosure_limit 原传 5_000, R-04 gateway 上限 100
/// (invalid request: market dragon-tiger limit must be at most 100) —
/// 断点 A 接线后首次暴露, 改为上限内值。
/// Preserve Gateway status before the legacy optional-background projection loses it.
pub(super) async fn fetch_lhb_observed(
) -> Result<(HashMap<String, f64>, super::preparation::SourceObservation), String> {
    use super::preparation::{SourceObservation, SourceStatus};
    let requested_at = chrono::Local::now();
    let request_date = requested_at.date_naive();
    let batch = match DragonTigerGateway::new()
        .market_review(request_date, 100, 5_000)
        .await
    {
        Ok(batch) => batch,
        // 2026-08-07: 龙虎榜是 LLM 分析的增强背景 (净买入), 缺失不阻断核心
        // 聚类/落库/报告 — Gateway Err 降级为 warn + 空背景 (与 VerifiedEmpty
        // 同语义, 出声不静默)。盘前时段 Eastmoney 接口常返回 no usable
        // records (09:07 实证), 原 map_err 硬失败导致整条链分析推送失败。
        Err(error) => {
            log::warn!("[产业链][BR-164] 龙虎榜 Gateway 不可用，降级为空背景");
            return Ok((
                HashMap::new(),
                SourceObservation::unavailable(error.to_string())
                    .requested(request_date, requested_at.to_rfc3339()),
            ));
        }
    };
    match batch {
        GatewayBatch::Available { records, evidence } => Ok((
            map_lhb_reviews(records)?,
            SourceObservation::batch(SourceStatus::Available, &evidence)
                .requested(request_date, requested_at.to_rfc3339()),
        )),
        GatewayBatch::VerifiedEmpty(evidence) => {
            log::info!("[产业链][BR-164] 龙虎榜已验证为空");
            Ok((
                HashMap::new(),
                SourceObservation::batch(SourceStatus::VerifiedEmpty, &evidence)
                    .requested(request_date, requested_at.to_rfc3339()),
            ))
        }
    }
}

pub(crate) fn map_lhb_reviews(
    records: Vec<DragonTigerStockReview>,
) -> Result<HashMap<String, f64>, String> {
    let mut out = HashMap::new();
    for record in records {
        if record.code.trim().is_empty() || !record.ranking_net_amount_yuan.is_finite() {
            return Err(format!("产业链龙虎榜行非法: code={:?}", record.code));
        }
        let net_amount_wan = record.ranking_net_amount_yuan / 10_000.0;
        if out.insert(record.code.clone(), net_amount_wan).is_some() {
            return Err(format!("产业链龙虎榜 code 重复: {}", record.code));
        }
    }
    Ok(out)
}

pub(super) fn append_after_market_items(
    items: &mut Vec<String>,
    theme: &str,
    results: Vec<crate::search_service::SearchResult>,
) {
    for result in results {
        let date = result.published_date.as_deref().unwrap_or("");
        let snippet: String = result.snippet.chars().take(100).collect();
        let item = format!(
            "- 🔥 **{}** [{}] {}\n  {}",
            result.title, theme, date, snippet
        );
        if !items
            .iter()
            .any(|existing| existing.contains(&result.title))
        {
            items.push(item);
        }
    }
}

pub(super) fn render_after_market_section(
    today: &str,
    time_label: &str,
    items: &[String],
) -> String {
    if items.is_empty() {
        return String::new();
    }
    format!(
        "## 🚨 盘后催化追踪（{} {} 最新动态，{} 条）\n\n{}\n",
        today,
        time_label,
        items.len(),
        items.join("\n")
    )
}

#[cfg(test)]
async fn resolve_after_market_catalysts<F>(
    top_themes: &[&str],
    today: &str,
    time_label: &str,
    timeout: std::time::Duration,
    mut search: F,
) -> String
where
    F: FnMut(String, usize) -> SearchFuture,
{
    let mut items = Vec::new();
    for theme in top_themes.iter().take(5) {
        if items.len() >= 10 {
            break;
        }
        let query = format!("{today} {theme} 最新 突发 催化");
        let results = tokio::time::timeout(timeout, search(query, 2))
            .await
            .unwrap_or_default();
        append_after_market_items(&mut items, theme, results);
    }
    render_after_market_section(today, time_label, &items)
}

pub(super) fn build_cluster_query_context(
    cluster: &ChainCluster,
    concepts: &HashMap<String, Vec<String>>,
) -> (Vec<String>, String) {
    let leaders: Vec<&str> = cluster
        .stocks
        .iter()
        .take(2)
        .map(|stock| stock.name.as_str())
        .collect();
    let queries = vec![format!(
        "{} 板块 集体涨停 原因 {}",
        cluster.concept,
        leaders.join(" ")
    )];

    let mut stock_lines = String::new();
    for stock in cluster.stocks.iter().take(10) {
        let tags: Vec<&str> = concepts
            .get(&stock.code)
            .map(|boards| {
                boards
                    .iter()
                    .filter(|board| !is_generic_board(board))
                    .map(|board| board.as_str())
                    .take(6)
                    .collect()
            })
            .unwrap_or_default();
        stock_lines.push_str(&format!("- {}：{}\n", stock.name, tags.join("、")));
    }
    let prompt = format!(
        r#"今日 A 股「{}」概念 {} 只股票集体涨停（股票及其概念标签）：
{}
请推测最可能驱动这次集体涨停的催化事件方向，输出 2-3 条具体的中文新闻搜索词，每行一条，不要编号、不要解释。
要求：
- 搜索词必须指向具体事件/商品价格/供给变化/政策/赛事（例："钨 出口管制 价格上涨"、"世界杯 转播权 广告 概念股"、"六氟化钨 停产"）
- 禁止使用"板块 涨停 原因"这类泛词
- 从股票组合的共性倒推：这些公司共同的上游、下游或终端场景最近可能发生了什么"#,
        cluster.concept,
        cluster.stocks.len(),
        stock_lines
    );
    (queries, prompt)
}

pub(super) fn append_generated_cluster_queries(queries: &mut Vec<String>, text: &str) {
    for line in text.lines() {
        let query = line
            .trim()
            .trim_start_matches(|character: char| {
                character.is_ascii_digit()
                    || character == '.'
                    || character == '-'
                    || character == '、'
                    || character == '*'
            })
            .trim()
            .trim_matches('"');
        let len = query.chars().count();
        let looks_like_sentence = query.contains('。')
            || query.contains('，')
            || query.contains('；')
            || query.contains('？');
        if (4..=40).contains(&len) && !looks_like_sentence && queries.len() < 4 {
            queries.push(query.to_string());
        }
    }
}

pub(super) fn append_cluster_news_items(
    seen: &mut HashSet<String>,
    items: &mut Vec<String>,
    results: Vec<crate::search_service::SearchResult>,
) {
    for result in results {
        let key: String = result.title.chars().take(20).collect();
        if !seen.insert(key) {
            continue;
        }
        let published = result.published_date.as_deref().unwrap_or("");
        let snippet: String = result.snippet.chars().take(150).collect();
        items.push(format!(
            "- **{}** {}\n  {}",
            result.title, published, snippet
        ));
        if items.len() >= 10 {
            break;
        }
    }
}

#[cfg(test)]
async fn resolve_cluster_news<F>(
    cluster: &ChainCluster,
    mut queries: Vec<String>,
    generated_queries: Result<String, String>,
    timeout: std::time::Duration,
    mut search: F,
) -> String
where
    F: FnMut(String, usize) -> SearchFuture,
{
    match generated_queries {
        Ok(text) => append_generated_cluster_queries(&mut queries, &text),
        Err(error) => warn!(
            "[产业链] 主线「{}」催化搜索词生成失败: {}",
            cluster.concept, error
        ),
    }
    log::debug!("[产业链] 主线「{}」检索词: {:?}", cluster.concept, queries);

    // 执行检索，按标题去重合并
    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<String> = Vec::new();
    for q in &queries {
        let results = match tokio::time::timeout(timeout, search(q.clone(), 4)).await {
            Ok(r) => r,
            Err(_) => {
                warn!("[产业链] 主线「{}」检索词 '{}' 超时", cluster.concept, q);
                continue;
            }
        };
        append_cluster_news_items(&mut seen, &mut items, results);
        if items.len() >= 10 {
            break;
        }
    }
    items.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        append_after_market_items, append_cluster_news_items, append_generated_cluster_queries,
        apply_fetched_concepts, build_cluster_query_context, compose_observed_concepts,
        fetch_concepts_cached, fetch_concepts_cached_observed_in, fetch_laggard_candidates,
        map_lhb_reviews, parse_tool_boards, parse_tool_boards_observed,
        render_after_market_section, resolve_after_market_catalysts, resolve_cluster_news,
        ConceptCodeEvidence, ConceptFetchTerminal, ConceptProjectionCompletion,
        ObservedConceptCacheWrite, ObservedConceptFetchError,
    };
    use crate::data_gateway::{
        BatchEvidence, BoardKind, BoardMembershipRecord, DragonTigerStockReview, GatewayBatch,
    };
    use crate::market_domain::{Exchange, ProviderId};
    use std::collections::{HashMap, HashSet};
    use std::{cell::RefCell, rc::Rc};

    fn search_result(
        title: impl Into<String>,
        snippet: impl Into<String>,
        published_date: Option<&str>,
    ) -> crate::search_service::SearchResult {
        crate::search_service::SearchResult {
            title: title.into(),
            snippet: snippet.into(),
            url: "https://example.invalid/test".to_string(),
            source: "TEST_CODE_SOURCE".to_string(),
            published_date: published_date.map(str::to_string),
            news_type: crate::search_service::NewsType::Industry,
            sentiment: crate::search_service::Sentiment::Neutral,
            importance: 5,
            relevance: 1.0,
            keywords: Vec::new(),
            evidence: crate::search_service::SearchEvidence::Unverified,
        }
    }

    #[test]
    fn resolved_catalyst_results_deduplicate_truncate_and_render() {
        let mut items = Vec::new();
        append_after_market_items(
            &mut items,
            "测试主线",
            vec![
                search_result("真实催化A", "甲".repeat(120), Some("2026-07-18")),
                search_result("真实催化A", "重复", None),
                search_result("真实催化B", "乙", None),
            ],
        );
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("测试主线"));
        assert!(!items[0].contains(&"甲".repeat(101)));
        assert!(render_after_market_section("07月18日", "盘后", &[]).is_empty());
        let section = render_after_market_section("07月18日", "盘后", &items);
        assert!(section.contains("2 条"));
        assert!(section.contains("真实催化A"));
    }

    #[tokio::test]
    async fn resolved_after_market_search_enforces_theme_and_item_limits() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let search_calls = std::sync::Arc::clone(&calls);
        let section = resolve_after_market_catalysts(
            &["主题甲", "主题乙", "主题丙", "主题丁", "主题戊", "主题己"],
            "07月18日",
            "盘后",
            std::time::Duration::from_secs(1),
            move |query, limit| {
                search_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Box::pin(async move {
                    assert_eq!(limit, 2);
                    vec![
                        search_result(format!("{query}-A"), "真实摘要A", Some("2026-07-18")),
                        search_result(format!("{query}-B"), "真实摘要B", None),
                    ]
                })
            },
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 5);
        assert!(section.contains("10 条"));
        assert!(!section.contains("主题己"));

        let timed_out = resolve_after_market_catalysts(
            &["超时主题"],
            "07月18日",
            "盘中",
            std::time::Duration::ZERO,
            |_query, _limit| Box::pin(futures::future::pending()),
        )
        .await;
        assert!(timed_out.is_empty());
        assert!(resolve_after_market_catalysts(
            &[],
            "07月18日",
            "盘后",
            std::time::Duration::from_secs(1),
            |_query, _limit| Box::pin(async { Vec::new() }),
        )
        .await
        .is_empty());
    }

    #[tokio::test]
    async fn resolved_cluster_search_merges_generated_queries_and_explicit_failures() {
        let cluster = super::super::ChainCluster {
            concept: "TEST_CODE_固态电池".to_string(),
            aliases: Vec::new(),
            stocks: Vec::new(),
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let search_calls = std::sync::Arc::clone(&calls);
        let news = resolve_cluster_news(
            &cluster,
            vec!["默认 主线查询".to_string()],
            Ok("1. 电解质 扩产\n- 原材料 涨价".to_string()),
            std::time::Duration::from_secs(1),
            move |query, limit| {
                search_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Box::pin(async move {
                    assert_eq!(limit, 4);
                    vec![
                        search_result("跨查询重复标题", format!("{query} 摘要"), None),
                        search_result(format!("{query} 独有"), "真实摘要", Some("2026-07-18")),
                    ]
                })
            },
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 3);
        assert_eq!(news.matches("跨查询重复标题").count(), 1);
        assert!(news.contains("电解质 扩产 独有"));

        let unavailable = resolve_cluster_news(
            &cluster,
            vec!["超时 查询".to_string()],
            Err("TEST_CODE_模型不可用".to_string()),
            std::time::Duration::ZERO,
            |_query, _limit| Box::pin(futures::future::pending()),
        )
        .await;
        assert!(unavailable.is_empty());
    }

    #[test]
    fn cluster_query_protocol_and_result_dedup_keep_registered_limits() {
        let cluster = super::super::ChainCluster {
            concept: "TEST_CODE_固态电池".to_string(),
            aliases: Vec::new(),
            stocks: vec![
                crate::market_data::TopStock {
                    code: "TEST_CODE_000001".to_string(),
                    name: "测试甲".to_string(),
                    ..Default::default()
                },
                crate::market_data::TopStock {
                    code: "TEST_CODE_000002".to_string(),
                    name: "测试乙".to_string(),
                    ..Default::default()
                },
            ],
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        let concepts = HashMap::from([
            (
                "TEST_CODE_000001".to_string(),
                vec!["融资融券".to_string(), "固态电池设备".to_string()],
            ),
            ("TEST_CODE_000002".to_string(), vec!["电解质".to_string()]),
        ]);
        let (mut queries, prompt) = build_cluster_query_context(&cluster, &concepts);
        assert_eq!(queries.len(), 1);
        assert!(queries[0].contains("测试甲 测试乙"));
        assert!(prompt.contains("固态电池设备"));
        assert!(!prompt.contains("融资融券、固态电池设备"));
        append_generated_cluster_queries(
            &mut queries,
            "1. 电解质 扩产\n- 固态电池 政策\n这是完整句子，应该被拒绝。\nx\n* 原材料 涨价",
        );
        assert_eq!(queries.len(), 4);
        assert!(queries.iter().any(|query| query == "电解质 扩产"));
        assert!(!queries.iter().any(|query| query.contains("应该被拒绝")));

        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut results: Vec<_> = (0..12)
            .map(|index| search_result(format!("真实产业新闻{index}"), "摘要".repeat(100), None))
            .collect();
        results.insert(
            1,
            search_result("真实产业新闻0", "重复", Some("2026-07-18")),
        );
        append_cluster_news_items(&mut seen, &mut items, results);
        assert_eq!(items.len(), 10);
        assert_eq!(seen.len(), 10);
        assert!(!items[0].contains(&"摘要".repeat(76)));
    }

    #[test]
    fn tool_board_batch_deduplicates_only_complete_nonempty_strings() {
        let boards = parse_tool_boards(
            r#"{"all_boards":["TEST_CODE_机器人","TEST_CODE_算力","TEST_CODE_机器人"]}"#,
            "TEST_CODE_000001",
        )
        .expect("complete tool response");
        assert_eq!(boards, ["TEST_CODE_机器人", "TEST_CODE_算力"]);

        for raw in [
            "not-json",
            r#"{}"#,
            r#"{"all_boards":[]}"#,
            r#"{"all_boards":[""]}"#,
            r#"{"all_boards":[1]}"#,
        ] {
            assert!(parse_tool_boards(raw, "TEST_CODE_000001").is_err(), "{raw}");
        }
    }

    fn rendered_membership_raw() -> String {
        let code = "TEST_CODE_000001";
        let records = [
            ("TEST_CODE_BOARD_A", "TEST_CODE_算力"),
            ("TEST_CODE_BOARD_B", "TEST_CODE_液冷"),
            ("TEST_CODE_BOARD_C", "TEST_CODE_算力"),
        ]
        .into_iter()
        .map(|(board_code, board_name)| BoardMembershipRecord {
            instrument_code: code.into(),
            board_code: board_code.into(),
            board_name: board_name.into(),
            kind: BoardKind::Concept,
        })
        .collect();
        crate::agent::tools_sector::render_membership_batch(
            code,
            GatewayBatch::Available {
                records,
                evidence: BatchEvidence {
                    provider: ProviderId::Tdx,
                    source: "TEST_CODE_tdx_membership_v1".into(),
                    source_at: None,
                    observed_at: "2026-10-01T09:31:00+08:00".into(),
                    batch_id: "TEST_CODE_batch_1".into(),
                },
            },
        )
        .expect("render the real sector tool schema")
    }

    #[test]
    fn observed_tool_boards_preserve_raw_response_and_opaque_evidence() {
        let raw = rendered_membership_raw();
        let observed = parse_tool_boards_observed(&raw, "TEST_CODE_000001")
            .expect("real tool response has complete syntactic evidence");
        assert_eq!(observed.requested_code, "TEST_CODE_000001");
        assert_eq!(observed.board_count, 3);
        assert_eq!(
            observed.all_boards,
            ["TEST_CODE_算力", "TEST_CODE_液冷", "TEST_CODE_算力"]
        );
        assert_eq!(observed.boards, ["TEST_CODE_算力", "TEST_CODE_液冷"]);
        assert_eq!(observed.evidence.provider_label, "Tdx");
        assert_eq!(observed.evidence.source, "TEST_CODE_tdx_membership_v1");
        assert_eq!(observed.evidence.source_at, None);
        assert_eq!(observed.evidence.observed_at, "2026-10-01T09:31:00+08:00");
        assert_eq!(observed.evidence.batch_id, "TEST_CODE_batch_1");
        assert_eq!(observed.raw_response_sha256.len(), 64);
        assert!(observed
            .raw_response_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));

        let same_json_with_extra_whitespace = format!("{raw} ");
        let changed =
            parse_tool_boards_observed(&same_json_with_extra_whitespace, "TEST_CODE_000001")
                .expect("JSON whitespace does not change board projection");
        assert_eq!(changed.boards, observed.boards);
        assert_ne!(changed.raw_response_sha256, observed.raw_response_sha256);

        let mut opaque_provider: serde_json::Value = serde_json::from_str(&raw).unwrap();
        opaque_provider["evidence"]["provider"] = serde_json::json!("TEST_CODE_NewProvider");
        opaque_provider["evidence"]["source_at"] = serde_json::json!("2026-10-01");
        let opaque = parse_tool_boards_observed(&opaque_provider.to_string(), "TEST_CODE_000001")
            .expect("provider labels remain opaque and source date is retained");
        assert_eq!(opaque.evidence.provider_label, "TEST_CODE_NewProvider");
        assert_eq!(opaque.evidence.source_at.as_deref(), Some("2026-10-01"));
    }

    #[test]
    fn observed_tool_boards_reject_mismatched_shape_and_incomplete_evidence() {
        let raw = rendered_membership_raw();
        let base: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parse_tool_boards_observed(&raw, "TEST_CODE_OTHER").is_err());
        for (field, value) in [
            ("board_count", serde_json::json!(2)),
            ("secucode", serde_json::json!("TEST_CODE_OTHER")),
            ("fetched", serde_json::json!(false)),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            assert!(
                parse_tool_boards_observed(&changed.to_string(), "TEST_CODE_000001").is_err(),
                "field={field}"
            );
        }
        for (field, value) in [
            ("provider", serde_json::json!(" ")),
            ("source", serde_json::json!(null)),
            ("source_at", serde_json::json!("not-a-date")),
            ("observed_at", serde_json::json!("2026-10-01")),
            ("batch_id", serde_json::json!("")),
        ] {
            let mut changed = base.clone();
            changed["evidence"][field] = value;
            assert!(
                parse_tool_boards_observed(&changed.to_string(), "TEST_CODE_000001").is_err(),
                "evidence field={field}"
            );
        }
        let mut missing_source_at = base;
        missing_source_at["evidence"]
            .as_object_mut()
            .unwrap()
            .remove("source_at");
        assert!(
            parse_tool_boards_observed(&missing_source_at.to_string(), "TEST_CODE_000001").is_err()
        );

        // The legacy parser keeps accepting its original projection-only shape.
        assert_eq!(
            parse_tool_boards(r#"{"all_boards":["TEST_CODE_算力"]}"#, "TEST_CODE_000001").unwrap(),
            ["TEST_CODE_算力"]
        );
    }

    #[test]
    fn observed_concept_projection_binds_content_and_each_code_origin() {
        let isolated = tempfile::tempdir().expect("isolated concept projection database");
        let db = crate::database::DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_concept_projection.db"),
        )
        .expect("open isolated concept projection database");
        db.save_stock_concepts("TEST_CODE_CACHE_A", &["TEST_CODE_缓存概念".into()])
            .unwrap();
        db.save_stock_concepts("TEST_CODE_UNRELATED", &["TEST_CODE_无关".into()])
            .unwrap();
        let cache_read = db.get_cached_concepts_observed(7).unwrap();
        let provider =
            parse_tool_boards_observed(&rendered_membership_raw(), "TEST_CODE_000001").unwrap();
        let writes = vec![ObservedConceptCacheWrite::ToolObservation(provider)];
        let requested = vec!["TEST_CODE_CACHE_A".into(), "TEST_CODE_000001".into()];
        let first = compose_observed_concepts(
            &requested,
            cache_read.clone(),
            writes.clone(),
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        let second = compose_observed_concepts(
            &requested,
            cache_read.clone(),
            writes.clone(),
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.requested_codes, requested);
        assert_eq!(first.cache_read, cache_read);
        assert!(!first.concepts.contains_key("TEST_CODE_UNRELATED"));
        assert!(matches!(
            first.sources.get("TEST_CODE_CACHE_A"),
            Some(ConceptCodeEvidence::LocalCache(row))
                if row.code() == "TEST_CODE_CACHE_A" && !row.updated_at().is_empty()
        ));
        assert!(matches!(
            first.sources.get("TEST_CODE_000001"),
            Some(ConceptCodeEvidence::ToolResponse(observed))
                if observed.evidence.provider_label == "Tdx"
                    && observed.raw_response_sha256.len() == 64
        ));
        let ConceptProjectionCompletion::Complete { content_sha256 } = &first.completion else {
            panic!("fully covered requested projection must be complete");
        };
        assert_eq!(content_sha256.len(), 64);
        let same_boards_new_raw = parse_tool_boards_observed(
            &format!("{} ", rendered_membership_raw()),
            "TEST_CODE_000001",
        )
        .unwrap();
        let same_content = compose_observed_concepts(
            &requested,
            cache_read.clone(),
            vec![ObservedConceptCacheWrite::ToolObservation(
                same_boards_new_raw,
            )],
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        assert_eq!(first.completion, same_content.completion);
        assert_ne!(first.sources, same_content.sources);
        let reordered = compose_observed_concepts(
            &[requested[1].clone(), requested[0].clone()],
            cache_read,
            writes,
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        assert_ne!(first.completion, reordered.completion);
    }

    #[test]
    fn observed_concept_projection_retains_partial_writes_without_complete_digest() {
        let isolated = tempfile::tempdir().expect("isolated partial concept database");
        let db = crate::database::DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_partial_concept.db"),
        )
        .expect("open isolated partial concept database");
        let cache_read = db.get_cached_concepts_observed(7).unwrap();
        let first =
            parse_tool_boards_observed(&rendered_membership_raw(), "TEST_CODE_000001").unwrap();
        let writes = vec![ObservedConceptCacheWrite::ToolObservation(first.clone())];
        let failed_before_write = compose_observed_concepts(
            &["TEST_CODE_000001".into()],
            cache_read.clone(),
            Vec::new(),
            ConceptFetchTerminal::Failed,
        )
        .unwrap();
        assert_eq!(
            failed_before_write.completion,
            ConceptProjectionCompletion::Incomplete {
                reason: "fetch_failed_before_any_write"
            }
        );
        let failed = compose_observed_concepts(
            &["TEST_CODE_000001".into()],
            cache_read.clone(),
            writes.clone(),
            ConceptFetchTerminal::Failed,
        )
        .unwrap();
        assert_eq!(failed.successful_writes, writes);
        assert_eq!(failed.concepts["TEST_CODE_000001"], first.boards);
        assert_eq!(
            failed.completion,
            ConceptProjectionCompletion::Incomplete {
                reason: "fetch_failed_after_partial_writes"
            }
        );

        let uncovered = compose_observed_concepts(
            &["TEST_CODE_000001".into(), "TEST_CODE_MISSING".into()],
            cache_read.clone(),
            writes.clone(),
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        assert_eq!(
            uncovered.completion,
            ConceptProjectionCompletion::Incomplete {
                reason: "requested_code_uncovered"
            }
        );

        let duplicate_request = vec!["TEST_CODE_000001".into(), "TEST_CODE_000001".into()];
        let missing_duplicate_write = compose_observed_concepts(
            &duplicate_request,
            cache_read.clone(),
            writes.clone(),
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        assert_eq!(
            missing_duplicate_write.completion,
            ConceptProjectionCompletion::Incomplete {
                reason: "provider_write_cardinality_mismatch"
            }
        );

        let mut changed: serde_json::Value =
            serde_json::from_str(&rendered_membership_raw()).unwrap();
        changed["all_boards"][0] = serde_json::json!("TEST_CODE_新概念");
        let second = parse_tool_boards_observed(&changed.to_string(), "TEST_CODE_000001").unwrap();
        let duplicate_writes = vec![
            ObservedConceptCacheWrite::ToolObservation(first),
            ObservedConceptCacheWrite::ToolObservation(second.clone()),
        ];
        let unchanged = compose_observed_concepts(
            &duplicate_request,
            cache_read.clone(),
            vec![writes[0].clone(), writes[0].clone()],
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        let duplicated = compose_observed_concepts(
            &duplicate_request,
            cache_read,
            duplicate_writes.clone(),
            ConceptFetchTerminal::Completed,
        )
        .unwrap();
        assert_eq!(duplicated.successful_writes, duplicate_writes);
        assert_eq!(duplicated.concepts["TEST_CODE_000001"], second.boards);
        assert!(matches!(
            duplicated.completion,
            ConceptProjectionCompletion::Complete { .. }
        ));
        assert_ne!(unchanged.completion, duplicated.completion);
    }

    #[tokio::test]
    async fn observed_concept_fetch_keeps_legacy_map_and_one_raw_call_per_miss() {
        let isolated = tempfile::tempdir().expect("isolated observed fetch database");
        let db = crate::database::DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_observed_fetch.db"),
        )
        .expect("open isolated observed fetch database");
        db.save_stock_concepts("TEST_CODE_CACHE_A", &["TEST_CODE_缓存概念".into()])
            .unwrap();
        db.save_stock_concepts("TEST_CODE_UNRELATED", &["TEST_CODE_无关".into()])
            .unwrap();
        let requested = vec!["TEST_CODE_CACHE_A".into(), "TEST_CODE_000001".into()];
        let raw = rendered_membership_raw();
        let calls = Rc::new(RefCell::new(Vec::<String>::new()));
        let result = fetch_concepts_cached_observed_in(&db, &requested, {
            let calls = Rc::clone(&calls);
            move |code| {
                calls.borrow_mut().push(code);
                let raw = raw.clone();
                async move { Ok(raw) }
            }
        })
        .await
        .expect("strict tool response and cache hit complete the fetch");
        assert_eq!(*calls.borrow(), ["TEST_CODE_000001"]);
        assert_eq!(
            result.legacy_map["TEST_CODE_CACHE_A"],
            ["TEST_CODE_缓存概念"]
        );
        assert_eq!(
            result.legacy_map["TEST_CODE_000001"],
            ["TEST_CODE_算力", "TEST_CODE_液冷"]
        );
        assert_eq!(result.legacy_map["TEST_CODE_UNRELATED"], ["TEST_CODE_无关"]);
        assert!(!result
            .observation
            .concepts
            .contains_key("TEST_CODE_UNRELATED"));
        assert!(matches!(
            result.observation.completion,
            ConceptProjectionCompletion::Complete { .. }
        ));
        assert!(matches!(
            result.observation.sources.get("TEST_CODE_CACHE_A"),
            Some(ConceptCodeEvidence::LocalCache(_))
        ));
        assert!(matches!(
            result.observation.sources.get("TEST_CODE_000001"),
            Some(ConceptCodeEvidence::ToolResponse(_))
        ));
    }

    #[tokio::test]
    async fn observed_concept_fetch_preserves_projection_only_duplicate_writes() {
        let isolated = tempfile::tempdir().expect("isolated legacy projection database");
        let db = crate::database::DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_projection_only.db"),
        )
        .expect("open isolated legacy projection database");
        let requested = vec!["TEST_CODE_000001".into(), "TEST_CODE_000001".into()];
        let raw = r#"{"all_boards":["TEST_CODE_旧响应"]}"#.to_string();
        let calls = Rc::new(RefCell::new(Vec::<String>::new()));
        let result = fetch_concepts_cached_observed_in(&db, &requested, {
            let calls = Rc::clone(&calls);
            move |code| {
                calls.borrow_mut().push(code);
                let raw = raw.clone();
                async move { Ok(raw) }
            }
        })
        .await
        .expect("old projection-only response still succeeds");
        assert_eq!(*calls.borrow(), ["TEST_CODE_000001", "TEST_CODE_000001"]);
        assert_eq!(result.legacy_map["TEST_CODE_000001"], ["TEST_CODE_旧响应"]);
        assert_eq!(result.observation.successful_writes.len(), 2);
        assert!(result
            .observation
            .successful_writes
            .iter()
            .all(|write| matches!(write, ObservedConceptCacheWrite::LegacyProjection { .. })));
        assert!(matches!(
            result.observation.sources.get("TEST_CODE_000001"),
            Some(ConceptCodeEvidence::LegacyToolProjection { .. })
        ));
        assert_eq!(
            result.observation.completion,
            ConceptProjectionCompletion::Incomplete {
                reason: "tool_evidence_incomplete"
            }
        );
        assert_eq!(
            db.get_cached_concepts(7).unwrap()["TEST_CODE_000001"],
            ["TEST_CODE_旧响应"]
        );
    }

    #[test]
    fn observed_concept_fetch_failure_retains_written_prefix_and_original_error() {
        let isolated = tempfile::tempdir().expect("isolated observed failure database");
        let db = crate::database::DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_observed_failure.db"),
        )
        .expect("open isolated observed failure database");
        let requested = vec!["TEST_CODE_000001".into(), "TEST_CODE_000002".into()];
        let cache_read = db.get_cached_concepts_observed(7).unwrap();
        let map = cache_read.clone().into_map();
        let raw = rendered_membership_raw();
        let boards = parse_tool_boards(&raw, "TEST_CODE_000001").unwrap();
        let error = apply_fetched_concepts(
            &db,
            &requested,
            cache_read,
            map,
            vec![
                ("TEST_CODE_000001".into(), Ok((raw, boards.clone()))),
                (
                    "TEST_CODE_000002".into(),
                    Err("TEST_CODE_original_error".into()),
                ),
            ],
        )
        .expect_err("second completion error stops writes");
        let ObservedConceptFetchError::AfterCache {
            legacy_error,
            observation,
        } = error
        else {
            panic!("cache query already succeeded");
        };
        assert_eq!(legacy_error, "TEST_CODE_original_error");
        assert_eq!(observation.successful_writes.len(), 1);
        assert_eq!(observation.concepts["TEST_CODE_000001"], boards);
        assert!(!observation.concepts.contains_key("TEST_CODE_000002"));
        assert_eq!(
            observation.completion,
            ConceptProjectionCompletion::Incomplete {
                reason: "fetch_failed_after_partial_writes"
            }
        );
        let cached = db.get_cached_concepts(7).unwrap();
        assert_eq!(cached["TEST_CODE_000001"], boards);
        assert!(!cached.contains_key("TEST_CODE_000002"));
    }

    #[test]
    fn dragon_tiger_mapping_preserves_yuan_units_and_rejects_ambiguous_rows() {
        let mapped = map_lhb_reviews(vec![
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: "TEST_CODE_600001".to_string(),
                ranking_net_amount_yuan: 123_450_000.0,
                disclosures: Vec::new(),
            },
            DragonTigerStockReview {
                exchange: Exchange::Shenzhen,
                code: "TEST_CODE_000002".to_string(),
                ranking_net_amount_yuan: -50_000.0,
                disclosures: Vec::new(),
            },
        ])
        .expect("complete gateway records");
        assert_eq!(mapped.get("TEST_CODE_600001"), Some(&12_345.0));
        assert_eq!(mapped.get("TEST_CODE_000002"), Some(&-5.0));

        for record in [
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: String::new(),
                ranking_net_amount_yuan: 1.0,
                disclosures: Vec::new(),
            },
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: "TEST_CODE_NAN".to_string(),
                ranking_net_amount_yuan: f64::NAN,
                disclosures: Vec::new(),
            },
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: "TEST_CODE_INFINITY".to_string(),
                ranking_net_amount_yuan: f64::INFINITY,
                disclosures: Vec::new(),
            },
        ] {
            assert!(map_lhb_reviews(vec![record]).is_err());
        }

        let duplicate = DragonTigerStockReview {
            exchange: Exchange::Shanghai,
            code: "TEST_CODE_DUPLICATE".to_string(),
            ranking_net_amount_yuan: 10_000.0,
            disclosures: Vec::new(),
        };
        assert!(map_lhb_reviews(vec![duplicate.clone(), duplicate]).is_err());
    }

    #[tokio::test]
    async fn cached_concepts_and_parsed_protocols_cover_success_boundaries() {
        assert!(fetch_concepts_cached(&[]).await.is_err());
        assert!(fetch_concepts_cached(&[" ".to_string()]).await.is_err());
        crate::database::DatabaseManager::init(None).expect("test database initialization");
        let db = crate::database::DatabaseManager::try_get().expect("test database");
        let cached_code = "TEST_CODE_CHAIN_CACHE_000001";
        let cached = vec!["TEST_CODE_固态电池".to_string()];
        db.save_stock_concepts(cached_code, &cached)
            .expect("cache isolated concepts");
        let concepts = fetch_concepts_cached(&[cached_code.to_string()])
            .await
            .expect("complete cache hit must avoid external transport");
        assert_eq!(concepts.get(cached_code), Some(&cached));
        assert!(
            fetch_laggard_candidates("tdx:concept:TEST_CODE_板块", &HashSet::new())
                .await
                .expect_err("released contract has no same-batch prices")
                .contains("unsupported")
        );
    }

    #[tokio::test]
    async fn empty_resolved_search_batches_are_stable_regardless_of_environment_keys() {
        let cluster = super::super::ChainCluster {
            concept: "TEST_CODE_主题".to_string(),
            aliases: Vec::new(),
            stocks: Vec::new(),
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        assert!(resolve_after_market_catalysts(
            &["TEST_CODE_主题"],
            "07月19日",
            "盘后",
            std::time::Duration::from_secs(1),
            |_query, _limit| Box::pin(async { Vec::new() }),
        )
        .await
        .is_empty());
        assert!(resolve_cluster_news(
            &cluster,
            vec!["TEST_CODE_查询".into()],
            Ok(String::new()),
            std::time::Duration::from_secs(1),
            |_query, _limit| Box::pin(async { Vec::new() }),
        )
        .await
        .is_empty());
    }
}
