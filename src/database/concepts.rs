//! Registered business rules: BR-068, BR-101, BR-102.
//! 股票概念板块标签缓存。
//!
//! 概念标签（东财 F10 核心题材）变化缓慢，落库缓存避免每日重复请求。
//! 供 `pipeline::chain_analysis` 产业链聚类使用。

use chrono::{DateTime, Duration, Local, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Binary, Integer, Text};
use diesel::sqlite::SqliteConnection;
use log::warn;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::database::DatabaseManager;

#[derive(QueryableByName)]
struct ConceptRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    concepts: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

/// One exact row returned by the local `stock_concepts` cache query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConceptCacheRow {
    code: String,
    concepts_json: String,
    concepts: Vec<String>,
    updated_at: String,
}

impl LocalConceptCacheRow {
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn concepts_json(&self) -> &str {
        &self.concepts_json
    }

    pub fn concepts(&self) -> &[String] {
        &self.concepts
    }

    /// The raw local-naive SQLite timestamp; this is not provider source time.
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }
}

/// Local cache evidence from one read, without any upstream qualification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConceptCacheRead {
    cutoff_local: String,
    observed_at_utc: DateTime<Utc>,
    rows: Vec<LocalConceptCacheRow>,
}

impl LocalConceptCacheRead {
    /// The exact local-naive cutoff bound to the cache SELECT.
    pub fn cutoff_local(&self) -> &str {
        &self.cutoff_local
    }

    pub fn observed_at_utc(&self) -> DateTime<Utc> {
        self.observed_at_utc
    }

    pub fn rows(&self) -> &[LocalConceptCacheRow] {
        &self.rows
    }

    pub fn into_map(self) -> HashMap<String, Vec<String>> {
        self.rows
            .into_iter()
            .map(|row| (row.code, row.concepts))
            .collect()
    }
}

fn parse_cached_concept_values(code: &str, concepts: &str) -> Result<Vec<String>, String> {
    if code.trim().is_empty() {
        return Err("概念缓存存在空 code".to_string());
    }
    let list = serde_json::from_str::<Vec<String>>(concepts)
        .map_err(|error| format!("概念缓存 {code} JSON 非法: {error}"))?;
    if list.is_empty() || list.iter().any(|concept| concept.trim().is_empty()) {
        return Err(format!("概念缓存 {code} 含空概念列表/字段"));
    }
    Ok(list)
}

pub(crate) fn parse_cached_concept_rows<I>(rows: I) -> Result<HashMap<String, Vec<String>>, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut map = HashMap::new();
    for (code, concepts) in rows {
        let list = parse_cached_concept_values(&code, &concepts)?;
        map.insert(code, list);
    }
    Ok(map)
}

/// chain_daily 行：某日某主线簇。
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct ChainDailyRow {
    #[diesel(sql_type = Text)]
    pub date: String,
    #[diesel(sql_type = Text)]
    pub concept: String,
    /// JSON 数组：["code1","code2",...]
    #[diesel(sql_type = Text)]
    pub stocks: String,
    #[diesel(sql_type = Integer)]
    pub continuation_count: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum ChainDailyReplaceError {
    #[error("P-01 exact-date 主线输入非法: {0}")]
    InvalidInput(String),
    #[error("P-01 exact-date 主线事务内读回与写入不一致")]
    ReadbackMismatch,
    #[error("P-01 exact-date 主线代次事务内读回与写入不一致")]
    GenerationReadbackMismatch,
    #[error("P-01 exact-date 主线存储失败: {0}")]
    Storage(String),
}

impl From<diesel::result::Error> for ChainDailyReplaceError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

const P01_GENERATION_SCHEMA: &str = "P01_CHAIN_PRODUCER_GENERATION_V1";
const P01_GENERATION_DOMAIN: &[u8] = b"P01_CHAIN_PRODUCER_GENERATION_V1\0";
const P01_CHAIN_ROW_DOMAIN: &[u8] = b"P01_CHAIN_ROW_V1\0";
const P01_STORED_ROWS_SCHEMA: &str = "P01_CHAIN_DAILY_STORED_ROWS_V1";

/// The producer supplies this identity before the chain rows are written.
/// The DAO only publishes it after the same transaction verifies both objects.
pub(crate) struct P01ChainGenerationInput<'a> {
    pub canonical_bytes: &'a [u8],
    pub generation_sha256: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedP01ChainGeneration {
    generation_sha256: String,
    canonical_bytes: Vec<u8>,
    persistence_receipt_sha256: String,
}

impl PersistedP01ChainGeneration {
    pub fn generation_sha256(&self) -> &str {
        &self.generation_sha256
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub fn persistence_receipt_sha256(&self) -> &str {
        &self.persistence_receipt_sha256
    }
}

/// A persisted P-01 generation is still not a qualified P-05 origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P05ChainGenerationStatus {
    NoRows,
    UnboundLegacyRows,
    P01BoundUnqualified(PersistedP01ChainGeneration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P05ChainSnapshotRead {
    rows: Vec<ChainDailyRow>,
    generation: P05ChainGenerationStatus,
}

impl P05ChainSnapshotRead {
    pub fn rows(&self) -> &[ChainDailyRow] {
        &self.rows
    }

    pub fn generation(&self) -> &P05ChainGenerationStatus {
        &self.generation
    }

    /// Consume the two facts read under one SQLite snapshot together.
    pub fn into_parts(self) -> (Vec<ChainDailyRow>, P05ChainGenerationStatus) {
        (self.rows, self.generation)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum P05ChainSnapshotReadError {
    #[error("P-05 chain_daily snapshot storage failed: {0}")]
    Storage(String),
    #[error("P-05 chain_daily P-01 generation binding invalid: {0}")]
    BindingMismatch(&'static str),
}

impl From<diesel::result::Error> for P05ChainSnapshotReadError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(QueryableByName)]
struct StoredP01GenerationRow {
    #[diesel(sql_type = Text)]
    generation_sha256: String,
    #[diesel(sql_type = Binary)]
    canonical_bytes: Vec<u8>,
    #[diesel(sql_type = Text)]
    stored_rows_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct P01GenerationCanonical {
    schema: String,
    evidence_date: String,
    provider: crate::market_domain::ProviderId,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    persistence_receipt_sha256: String,
    ordered_chain_row_hashes: Vec<String>,
}

#[derive(Serialize)]
struct P01ChainRowCanonical<'a> {
    date: &'a str,
    concept: &'a str,
    stocks: &'a [String],
    continuation_count: i32,
}

#[derive(Serialize)]
struct P01StoredRowCanonical<'a> {
    date: &'a str,
    concept: &'a str,
    stocks: &'a str,
    continuation_count: i32,
}

fn exact_stored_rows_sha256(rows: &[ChainDailyRow]) -> Result<String, serde_json::Error> {
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        right
            .continuation_count
            .cmp(&left.continuation_count)
            .then_with(|| left.concept.cmp(&right.concept))
    });
    let canonical = ordered
        .into_iter()
        .map(|row| P01StoredRowCanonical {
            date: &row.date,
            concept: &row.concept,
            stocks: &row.stocks,
            continuation_count: row.continuation_count,
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&(P01_STORED_ROWS_SCHEMA, canonical))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn verify_p01_generation(
    date: &str,
    rows: &[ChainDailyRow],
    stored: &StoredP01GenerationRow,
) -> Result<PersistedP01ChainGeneration, &'static str> {
    let mut generation_digest = Sha256::new();
    generation_digest.update(P01_GENERATION_DOMAIN);
    generation_digest.update(&stored.canonical_bytes);
    if hex::encode(generation_digest.finalize()) != stored.generation_sha256 {
        return Err("generation_sha256_mismatch");
    }
    let canonical: P01GenerationCanonical =
        serde_json::from_slice(&stored.canonical_bytes).map_err(|_| "canonical_invalid")?;
    if serde_json::to_vec(&canonical).map_err(|_| "canonical_invalid")? != stored.canonical_bytes {
        return Err("canonical_bytes_invalid");
    }
    if canonical.schema != P01_GENERATION_SCHEMA
        || canonical.evidence_date != date
        || !matches!(
            canonical.provider,
            crate::market_domain::ProviderId::Eastmoney
                | crate::market_domain::ProviderId::Tonghuashun
        )
        || canonical.source_at.as_deref() != Some(date)
        || canonical.source.trim().is_empty()
        || canonical.observed_at.trim().is_empty()
        || canonical.batch_id.trim().is_empty()
        || canonical.persistence_receipt_sha256.len() != 64
        || rows.is_empty()
        || rows.len() != canonical.ordered_chain_row_hashes.len()
    {
        return Err("generation_identity_invalid");
    }
    if exact_stored_rows_sha256(rows).map_err(|_| "stored_rows_encoding_failed")?
        != stored.stored_rows_sha256
    {
        return Err("stored_rows_sha256_mismatch");
    }
    let mut actual_hashes = Vec::with_capacity(rows.len());
    for row in rows {
        if row.date != date {
            return Err("row_date_mismatch");
        }
        let stocks: Vec<String> =
            serde_json::from_str(&row.stocks).map_err(|_| "row_stocks_invalid")?;
        let bytes = serde_json::to_vec(&P01ChainRowCanonical {
            date: &row.date,
            concept: &row.concept,
            stocks: &stocks,
            continuation_count: row.continuation_count,
        })
        .map_err(|_| "row_encoding_failed")?;
        let mut digest = Sha256::new();
        digest.update(P01_CHAIN_ROW_DOMAIN);
        digest.update(bytes);
        actual_hashes.push(hex::encode(digest.finalize()));
    }
    actual_hashes.sort();
    let mut expected_hashes = canonical.ordered_chain_row_hashes;
    expected_hashes.sort();
    if actual_hashes != expected_hashes {
        return Err("row_hashes_mismatch");
    }
    Ok(PersistedP01ChainGeneration {
        generation_sha256: stored.generation_sha256.clone(),
        canonical_bytes: stored.canonical_bytes.clone(),
        persistence_receipt_sha256: canonical.persistence_receipt_sha256,
    })
}

/// P-05 uses the same ordering as the exact-date P-01 reader. The primary key
/// (date, concept) makes this a total order within the latest date.
fn query_p05_latest_chain_clusters(
    conn: &mut SqliteConnection,
) -> Result<Vec<ChainDailyRow>, diesel::result::Error> {
    diesel::sql_query(
        "SELECT date, concept, stocks, continuation_count FROM chain_daily \
         WHERE date = (SELECT MAX(date) FROM chain_daily) \
         ORDER BY continuation_count DESC, concept ASC",
    )
    .load(conn)
}

/// B-002 板块联动归因 (Board hit) 行: 某日某板块的"板块拉升新闻+异动股列表"。
#[derive(Debug, Clone)]
pub struct BoardRotationRow {
    pub date: String,
    pub board_code: String,
    pub board_name: String,
    pub news_title: String,
    pub board_change_pct: f64,
    pub board_main_net_pct: f64,
    /// JSON 数组: [{"code":"002208","name":"合肥城建","change_pct":10.0},...]
    pub stocks_json: String,
}

/// board_rotation_daily 入库条目 (B-002 调用方构造).
#[derive(Debug, Clone)]
pub struct BoardRotationEntry {
    pub board_code: String,
    pub board_name: String,
    pub news_title: String,
    pub board_change_pct: f64,
    pub board_main_net_pct: f64,
    pub stocks_json: String,
}

#[derive(QueryableByName)]
pub struct BoardRotationQueryRow {
    #[diesel(sql_type = Text)]
    pub date: String,
    #[diesel(sql_type = Text)]
    pub board_code: String,
    #[diesel(sql_type = Text)]
    pub board_name: String,
    #[diesel(sql_type = Text)]
    pub news_title: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub board_change_pct: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub board_main_net_pct: f64,
    #[diesel(sql_type = Text)]
    pub stocks: String,
}

/// B-003 事件去重条目: (simhash, title) — simhash 用于精确/汉明距去重, title 用于 LCS 去重.
#[derive(Debug, Clone)]
pub struct EventSeenEntry {
    pub simhash: u64,
    pub title: String,
}

#[derive(QueryableByName)]
pub struct EventSeenRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub simhash: i64,
    #[diesel(sql_type = Text)]
    pub title: String,
}

impl DatabaseManager {
    /// 读取未过期的概念标签缓存（concepts 为 JSON 数组字符串）。
    ///
    /// 返回 `code -> 概念列表` 映射；数据库或坏缓存行使整批失败。
    pub fn get_cached_concepts(
        &self,
        max_age_days: i64,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        Ok(self.get_cached_concepts_observed(max_age_days)?.into_map())
    }

    /// Read the same cache rows once, retaining the exact cutoff, read time,
    /// raw JSON and each row's local `updated_at` for later provenance work.
    pub fn get_cached_concepts_observed(
        &self,
        max_age_days: i64,
    ) -> Result<LocalConceptCacheRead, String> {
        if max_age_days <= 0 {
            return Err(format!("概念缓存 max_age_days 非法: {max_age_days}"));
        }
        let cutoff = (Local::now() - Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let mut conn = self
            .get_conn()
            .map_err(|error| format!("概念缓存获取数据库连接失败: {error}"))?;

        let rows: Vec<ConceptRow> = diesel::sql_query(
            "SELECT code, concepts, updated_at FROM stock_concepts WHERE updated_at >= ?",
        )
        .bind::<Text, _>(&cutoff)
        .load(&mut conn)
        .map_err(|error| format!("概念缓存查询失败: {error}"))?;
        let observed_at_utc = Utc::now();
        let mut rows = rows
            .into_iter()
            .map(|row| {
                let concepts = parse_cached_concept_values(&row.code, &row.concepts)?;
                Ok(LocalConceptCacheRow {
                    code: row.code,
                    concepts_json: row.concepts,
                    concepts,
                    updated_at: row.updated_at,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        rows.sort_by(|left, right| left.code.cmp(&right.code));
        Ok(LocalConceptCacheRead {
            cutoff_local: cutoff,
            observed_at_utc,
            rows,
        })
    }

    /// 写入/覆盖某只股票的概念标签缓存。
    pub fn save_stock_concepts(&self, code: &str, concepts: &[String]) -> Result<(), String> {
        if code.trim().is_empty()
            || concepts.is_empty()
            || concepts.iter().any(|concept| concept.trim().is_empty())
        {
            return Err(format!("概念缓存写入参数非法: code={code:?}"));
        }
        let json = serde_json::to_string(concepts)
            .map_err(|error| format!("序列化 {code} 概念失败: {error}"))?;
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut conn = self
            .get_conn()
            .map_err(|error| format!("概念缓存写入获取数据库连接失败: {error}"))?;

        diesel::sql_query(
            "INSERT OR REPLACE INTO stock_concepts (code, concepts, updated_at) VALUES (?, ?, ?)",
        )
        .bind::<Text, _>(code)
        .bind::<Text, _>(&json)
        .bind::<Text, _>(&now)
        .execute(&mut conn)
        .map_err(|error| format!("概念缓存写入 {code} 失败: {error}"))?;
        Ok(())
    }

    /// 保存某日的主线簇结果（覆盖同日同概念）。
    pub fn save_chain_clusters(
        &self,
        date: &str,
        clusters: &[(String, Vec<String>, i32)],
    ) -> Result<(), String> {
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|error| format!("主线落库日期非法: {error}"))?;
        let mut encoded = Vec::with_capacity(clusters.len());
        for (concept, codes, cont) in clusters {
            if concept.trim().is_empty()
                || codes.is_empty()
                || codes.iter().any(|code| code.trim().is_empty())
                || *cont < 0
            {
                return Err(format!("主线落库行非法: concept={concept:?} cont={cont}"));
            }
            let json = serde_json::to_string(codes)
                .map_err(|error| format!("序列化主线 {concept} 失败: {error}"))?;
            encoded.push((concept, json, *cont));
        }
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("主线落库获取连接失败: {error}"))?;
        conn.transaction::<_, diesel::result::Error, _>(|tx| {
            if !encoded.is_empty() {
                diesel::sql_query("DELETE FROM chain_daily_p01_generation WHERE date = ?")
                    .bind::<Text, _>(date)
                    .execute(tx)?;
            }
            for (concept, json, cont) in &encoded {
                diesel::sql_query(
                "INSERT OR REPLACE INTO chain_daily (date, concept, stocks, continuation_count) VALUES (?, ?, ?, ?)",
            )
            .bind::<Text, _>(date)
            .bind::<Text, _>(concept)
            .bind::<Text, _>(json)
            .bind::<Integer, _>(*cont)
            .execute(tx)?;
            }
            Ok(())
        })
        .map_err(|error| format!("主线批量落库失败: {error}"))
    }

    /// BR-241: 原子替换指定证据日的完整 P-01 主线投影。
    ///
    /// 删除和插入位于同一事务；任一行失败时不会留下部分投影。同日旧概念不会
    /// 混入本次由 exact LimitPools 批次派生的结果。
    pub fn replace_chain_clusters_for_date_strict(
        &self,
        date: chrono::NaiveDate,
        clusters: &[(String, Vec<String>, i32)],
    ) -> Result<(), String> {
        self.replace_and_read_chain_clusters_for_date_strict(date, clusters)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Replace and verify the exact-date projection inside one SQLite write
    /// transaction. The returned rows are the committed generation's own
    /// read-back, not a later query that another producer could overtake.
    pub fn replace_and_read_chain_clusters_for_date_strict(
        &self,
        date: chrono::NaiveDate,
        clusters: &[(String, Vec<String>, i32)],
    ) -> Result<Vec<ChainDailyRow>, ChainDailyReplaceError> {
        self.replace_and_read_chain_clusters_for_date_inner(date, clusters, None)
    }

    pub(crate) fn replace_and_read_chain_clusters_with_p01_generation_strict(
        &self,
        date: chrono::NaiveDate,
        clusters: &[(String, Vec<String>, i32)],
        generation: P01ChainGenerationInput<'_>,
    ) -> Result<Vec<ChainDailyRow>, ChainDailyReplaceError> {
        self.replace_and_read_chain_clusters_for_date_inner(date, clusters, Some(generation))
    }

    fn replace_and_read_chain_clusters_for_date_inner(
        &self,
        date: chrono::NaiveDate,
        clusters: &[(String, Vec<String>, i32)],
        generation: Option<P01ChainGenerationInput<'_>>,
    ) -> Result<Vec<ChainDailyRow>, ChainDailyReplaceError> {
        let date = date.format("%Y-%m-%d").to_string();
        let mut concepts = std::collections::HashSet::with_capacity(clusters.len());
        let mut encoded = Vec::with_capacity(clusters.len());
        for (concept, codes, continuation_count) in clusters {
            if concept.trim().is_empty()
                || codes.is_empty()
                || codes.iter().any(|code| code.trim().is_empty())
                || *continuation_count < 0
            {
                return Err(ChainDailyReplaceError::InvalidInput(format!(
                    "P-01 exact-date 主线行非法: concept={concept:?} continuation_count={continuation_count}"
                )));
            }
            if !concepts.insert(concept.as_str()) {
                return Err(ChainDailyReplaceError::InvalidInput(format!(
                    "P-01 exact-date 主线概念重复: {concept}"
                )));
            }
            let stocks = serde_json::to_string(codes).map_err(|error| {
                ChainDailyReplaceError::InvalidInput(format!(
                    "序列化 P-01 主线 {concept} 失败: {error}"
                ))
            })?;
            encoded.push((concept, stocks, *continuation_count));
        }

        let expected_generation = generation
            .map(|generation| {
                let expected_rows = encoded
                    .iter()
                    .map(|(concept, stocks, continuation_count)| ChainDailyRow {
                        date: date.clone(),
                        concept: (*concept).clone(),
                        stocks: stocks.clone(),
                        continuation_count: *continuation_count,
                    })
                    .collect::<Vec<_>>();
                let stored = StoredP01GenerationRow {
                    generation_sha256: generation.generation_sha256.to_owned(),
                    canonical_bytes: generation.canonical_bytes.to_vec(),
                    stored_rows_sha256: exact_stored_rows_sha256(&expected_rows).map_err(
                        |error| {
                            ChainDailyReplaceError::InvalidInput(format!(
                                "P-01 stored rows encoding failed: {error}"
                            ))
                        },
                    )?,
                };
                verify_p01_generation(&date, &expected_rows, &stored).map_err(|reason| {
                    ChainDailyReplaceError::InvalidInput(format!(
                        "P-01 generation does not bind projected rows: {reason}"
                    ))
                })?;
                Ok::<_, ChainDailyReplaceError>(stored)
            })
            .transpose()?;

        let mut conn = self
            .get_conn()
            .map_err(|error| ChainDailyReplaceError::Storage(error.to_string()))?;
        conn.transaction::<_, ChainDailyReplaceError, _>(|tx| {
            diesel::sql_query("DELETE FROM chain_daily_p01_generation WHERE date = ?")
                .bind::<Text, _>(&date)
                .execute(tx)?;
            diesel::sql_query("DELETE FROM chain_daily WHERE date = ?")
                .bind::<Text, _>(&date)
                .execute(tx)?;
            for (concept, stocks, continuation_count) in &encoded {
                diesel::sql_query(
                    "INSERT INTO chain_daily (date, concept, stocks, continuation_count) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind::<Text, _>(&date)
                .bind::<Text, _>(*concept)
                .bind::<Text, _>(stocks)
                .bind::<Integer, _>(*continuation_count)
                .execute(tx)?;
            }
            let read_back: Vec<ChainDailyRow> = diesel::sql_query(
                "SELECT date, concept, stocks, continuation_count FROM chain_daily \
                 WHERE date = ? ORDER BY continuation_count DESC, concept ASC",
            )
            .bind::<Text, _>(&date)
            .load(tx)?;
            if read_back.len() != encoded.len()
                || read_back.iter().any(|row| {
                    row.date != date
                        || !encoded.iter().any(|(concept, stocks, continuation_count)| {
                            row.concept == **concept
                                && row.stocks == *stocks
                                && row.continuation_count == *continuation_count
                        })
                })
            {
                return Err(ChainDailyReplaceError::ReadbackMismatch);
            }
            if let Some(expected) = expected_generation.as_ref() {
                diesel::sql_query(
                    "INSERT INTO chain_daily_p01_generation(\
                     date, generation_sha256, canonical_bytes, stored_rows_sha256) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind::<Text, _>(&date)
                .bind::<Text, _>(&expected.generation_sha256)
                .bind::<Binary, _>(&expected.canonical_bytes)
                .bind::<Text, _>(&expected.stored_rows_sha256)
                .execute(tx)?;
                let read_generation = diesel::sql_query(
                    "SELECT generation_sha256, canonical_bytes, stored_rows_sha256 \
                     FROM chain_daily_p01_generation \
                     WHERE date = ?",
                )
                .bind::<Text, _>(&date)
                .get_result::<StoredP01GenerationRow>(tx)
                .optional()?
                .ok_or(ChainDailyReplaceError::GenerationReadbackMismatch)?;
                let final_rows: Vec<ChainDailyRow> = diesel::sql_query(
                    "SELECT date, concept, stocks, continuation_count FROM chain_daily \
                     WHERE date = ? ORDER BY continuation_count DESC, concept ASC",
                )
                .bind::<Text, _>(&date)
                .load(tx)?;
                if read_generation.generation_sha256 != expected.generation_sha256
                    || read_generation.canonical_bytes != expected.canonical_bytes
                    || read_generation.stored_rows_sha256 != expected.stored_rows_sha256
                    || final_rows != read_back
                    || verify_p01_generation(&date, &final_rows, &read_generation).is_err()
                {
                    return Err(ChainDailyReplaceError::GenerationReadbackMismatch);
                }
            }
            Ok(read_back)
        })
    }

    /// 读取最近一个有记录日期的主线簇（含当天）。
    pub fn get_latest_chain_clusters(&self) -> Vec<ChainDailyRow> {
        match self.get_latest_chain_clusters_strict() {
            Ok(rows) => rows,
            Err(error) => {
                warn!("[主线读取] {}", error);
                Vec::new()
            }
        }
    }

    /// 读取最近一个有记录日期的主线簇，并向严格数据链路传递失败。
    ///
    /// 新的推送/决策路径必须调用此方法，避免把数据库失败伪装成“没有主线”。
    pub fn get_latest_chain_clusters_strict(&self) -> Result<Vec<ChainDailyRow>, String> {
        let mut conn = match self.get_conn() {
            Ok(connection) => connection,
            Err(error) => return Err(format!("获取 chain_daily 数据库连接失败: {error}")),
        };
        diesel::sql_query(
            "SELECT date, concept, stocks, continuation_count FROM chain_daily \
             WHERE date = (SELECT MAX(date) FROM chain_daily)",
        )
        .load(&mut conn)
        .map_err(|error| format!("查询 chain_daily 失败: {error}"))
    }

    /// P-05 same-query latest-date rows with a deterministic top-five order.
    /// Other latest-date callers retain their existing query behavior.
    pub fn get_p05_latest_chain_clusters_strict(&self) -> Result<Vec<ChainDailyRow>, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("获取 P-05 chain_daily 数据库连接失败: {error}"))?;
        query_p05_latest_chain_clusters(&mut conn)
            .map_err(|error| format!("查询 P-05 chain_daily 失败: {error}"))
    }

    /// Read the same latest-date rows and optional P-01 generation in one
    /// SQLite snapshot. Old `chain_daily` rows remain explicitly unbound.
    pub fn get_p05_latest_chain_snapshot_strict(
        &self,
    ) -> Result<P05ChainSnapshotRead, P05ChainSnapshotReadError> {
        let mut conn = self
            .get_conn()
            .map_err(|error| P05ChainSnapshotReadError::Storage(error.to_string()))?;
        conn.transaction::<_, P05ChainSnapshotReadError, _>(|tx| {
            let rows = query_p05_latest_chain_clusters(tx)?;
            let Some(date) = rows.first().map(|row| row.date.as_str()) else {
                return Ok(P05ChainSnapshotRead {
                    rows,
                    generation: P05ChainGenerationStatus::NoRows,
                });
            };
            let generation = diesel::sql_query(
                "SELECT generation_sha256, canonical_bytes, stored_rows_sha256 \
                 FROM chain_daily_p01_generation \
                 WHERE date = ?",
            )
            .bind::<Text, _>(date)
            .get_result::<StoredP01GenerationRow>(tx)
            .optional()?;
            let generation = match generation {
                Some(stored) => P05ChainGenerationStatus::P01BoundUnqualified(
                    verify_p01_generation(date, &rows, &stored)
                        .map_err(P05ChainSnapshotReadError::BindingMismatch)?,
                ),
                None => P05ChainGenerationStatus::UnboundLegacyRows,
            };
            Ok(P05ChainSnapshotRead { rows, generation })
        })
    }

    /// BR-241: 严格读取指定证据日的 P-01 主线投影。
    ///
    /// 空集表示该日期没有已持久化投影；连接或查询失败保持显式错误。调用方不得
    /// 把空集或其他日期的最新行改标为指定日期证据。
    pub fn get_chain_clusters_for_date_strict(
        &self,
        date: chrono::NaiveDate,
    ) -> Result<Vec<ChainDailyRow>, String> {
        let date = date.format("%Y-%m-%d").to_string();
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("获取 exact-date chain_daily 数据库连接失败: {error}"))?;
        diesel::sql_query(
            "SELECT date, concept, stocks, continuation_count FROM chain_daily \
             WHERE date = ? ORDER BY continuation_count DESC, concept ASC",
        )
        .bind::<Text, _>(&date)
        .load(&mut conn)
        .map_err(|error| format!("查询 exact-date chain_daily {date} 失败: {error}"))
    }

    /// BR-195: 严格查询某概念主线在截至 `as_of` 的最近 N 个自然日内出现的天数。
    pub fn get_chain_appearance_days_as_of_strict(
        &self,
        concept: &str,
        days: i64,
        as_of: chrono::NaiveDate,
    ) -> Result<i64, String> {
        if concept.trim().is_empty() || days <= 0 {
            return Err(format!(
                "主线近窗出现天数参数非法: concept={concept:?} days={days} as_of={as_of}"
            ));
        }
        let offset_days = days
            .checked_sub(1)
            .ok_or_else(|| format!("主线近窗天数减一溢出: days={days}"))?;
        let offset = chrono::TimeDelta::try_days(offset_days)
            .ok_or_else(|| format!("主线近窗天数不可表示: days={days}"))?;
        let cutoff = as_of
            .checked_sub_signed(offset)
            .ok_or_else(|| format!("主线近窗日期下界溢出: days={days} as_of={as_of}"))?;
        let mut conn = match self.get_conn() {
            Ok(connection) => connection,
            Err(error) => return Err(format!("获取 chain_daily 连接失败: {error}")),
        };
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            n: i64,
        }
        let cutoff = cutoff.format("%Y-%m-%d").to_string();
        let as_of = as_of.format("%Y-%m-%d").to_string();
        let rows: Vec<CountRow> = diesel::sql_query(
            "SELECT COUNT(DISTINCT date) AS n FROM chain_daily \
             WHERE concept = ? AND date >= ? AND date <= ?",
        )
        .bind::<Text, _>(concept)
        .bind::<Text, _>(&cutoff)
        .bind::<Text, _>(&as_of)
        .load(&mut conn)
        .map_err(|error| format!("查询 chain_daily 主线近窗出现天数失败: {error}"))?;
        rows.first()
            .map(|row| row.n)
            .ok_or_else(|| "chain_daily 主线近窗出现天数聚合结果缺失".to_string())
    }

    // ===== B-003 事件抽取去重 (simhash) DAO =====

    /// 保存一批事件 (simhash, title), 用于下次去重.
    /// CR-7 (review): 用 conn.transaction 包裹循环, N 条事件 1 次 fsync 而非 N 次.
    ///                之前: 5min 一次 run_opportunity_scan, 100 条事件 → 100 次 INSERT + 100 次 lock.
    ///                现在: 1 个事务批量提交, 减少 100x fsync.
    pub fn save_event_seen(&self, entries: &[EventSeenEntry]) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("EventSeen 获取连接失败: {error}"))?;
        let result: Result<(), diesel::result::Error> = conn.transaction(|tx| {
            for entry in entries {
                let simhash_i64 = i64::try_from(entry.simhash)
                    .map_err(|_| diesel::result::Error::RollbackTransaction)?;
                if entry.title.trim().is_empty() {
                    return Err(diesel::result::Error::RollbackTransaction);
                }
                diesel::sql_query(
                    "INSERT OR REPLACE INTO event_seen_simhash (simhash, title) VALUES (?, ?)",
                )
                .bind::<diesel::sql_types::BigInt, _>(simhash_i64)
                .bind::<Text, _>(&entry.title)
                .execute(tx)?;
            }
            Ok(())
        });
        result.map_err(|error| format!("EventSeen 批量写入 {} 条失败: {error}", entries.len()))
    }

    /// 读取所有近 N 天内的事件去重条目 (供 extract_batch_rules_only_with_seen 跨日去重).
    /// B-003 默认 N=2 (与 max_age 对齐, 不留太久).
    pub fn get_recent_event_seen(&self, max_age_days: i64) -> Result<Vec<EventSeenEntry>, String> {
        if max_age_days <= 0 {
            return Err(format!("EventSeen max_age_days 非法: {max_age_days}"));
        }
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("EventSeen 获取连接失败: {error}"))?;
        // CR-21 (review): 改用 Utc::now() 与 seen_at CURRENT_TIMESTAMP (UTC) 一致.
        // 之前用 Local::now() 在 TZ 边界 (e.g. Asia/Shanghai UTC+8) 错配, 字符串 lexical 比较
        // 表面上 OK 但语义错位 — Asia/Shanghai 09:00 拉的 cutoff 与 DB UTC 时间错开最多 8h,
        // 导致跨日 dedup 漏判或过判.
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let rows: Vec<EventSeenRow> =
            diesel::sql_query("SELECT simhash, title FROM event_seen_simhash WHERE seen_at >= ?")
                .bind::<Text, _>(&cutoff)
                .load(&mut conn)
                .map_err(|error| format!("EventSeen 查询失败: {error}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let simhash = u64::try_from(row.simhash)
                .map_err(|_| format!("EventSeen simhash 非法: {}", row.simhash))?;
            if row.title.trim().is_empty() {
                return Err("EventSeen 存在空 title".to_string());
            }
            out.push(EventSeenEntry {
                simhash,
                title: row.title,
            });
        }
        Ok(out)
    }

    /// 清理过期事件去重条目 (cron 入口, 默认保留 7 天).
    pub fn cleanup_old_event_seen(&self, max_age_days: i64) -> Result<usize, String> {
        if max_age_days <= 0 {
            return Err(format!(
                "EventSeen cleanup max_age_days 非法: {max_age_days}"
            ));
        }
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("EventSeen cleanup 获取连接失败: {error}"))?;
        let cutoff = (chrono::Utc::now() - Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        diesel::sql_query("DELETE FROM event_seen_simhash WHERE seen_at < ?")
            .bind::<Text, _>(&cutoff)
            .execute(&mut conn)
            .map_err(|error| format!("EventSeen cleanup 失败: {error}"))
    }

    // ===== B-002 板块联动归因 (Board hit) DAO =====

    /// 保存某日的板块联动归因条目 (覆盖同日同 board_code).
    /// CR-7 (review): 用 conn.transaction 批量提交, N 条 1 次 fsync.
    pub fn save_board_rotations(
        &self,
        date: &str,
        entries: &[BoardRotationEntry],
    ) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|error| format!("BoardRotation 日期非法: {error}"))?;
        for entry in entries {
            if entry.board_code.trim().is_empty()
                || entry.board_name.trim().is_empty()
                || entry.news_title.trim().is_empty()
                || !entry.board_change_pct.is_finite()
                || !entry.board_main_net_pct.is_finite()
            {
                return Err(format!("BoardRotation 行非法: {}", entry.board_code));
            }
            let stocks: serde_json::Value =
                serde_json::from_str(&entry.stocks_json).map_err(|error| {
                    format!(
                        "BoardRotation {} stocks JSON 非法: {error}",
                        entry.board_code
                    )
                })?;
            if !stocks.is_array() {
                return Err(format!(
                    "BoardRotation {} stocks 不是数组",
                    entry.board_code
                ));
            }
        }
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BoardRotation 获取连接失败: {error}"))?;
        let result: Result<(), diesel::result::Error> = conn.transaction(|tx| {
            for entry in entries {
                diesel::sql_query(
                    "INSERT OR REPLACE INTO board_rotation_daily \
                     (date, board_code, board_name, news_title, board_change_pct, board_main_net_pct, stocks) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind::<Text, _>(date)
                .bind::<Text, _>(&entry.board_code)
                .bind::<Text, _>(&entry.board_name)
                .bind::<Text, _>(&entry.news_title)
                .bind::<diesel::sql_types::Double, _>(entry.board_change_pct)
                .bind::<diesel::sql_types::Double, _>(entry.board_main_net_pct)
                .bind::<Text, _>(&entry.stocks_json)
                .execute(tx)?;
            }
            Ok(())
        });
        result.map_err(|error| format!("BoardRotation 批量写入 {} 条失败: {error}", entries.len()))
    }

    /// 读取最近一天的所有板块联动归因条目 (含今天).
    /// 按 board_change_pct 降序排列 (最强板块在前), 供 NewsCatalyst 选 top cluster.
    pub fn get_latest_board_rotations(&self) -> Vec<BoardRotationRow> {
        match self.get_latest_board_rotations_strict() {
            Ok(rows) => rows,
            Err(error) => {
                warn!("[BoardRotation] {}", error);
                Vec::new()
            }
        }
    }

    /// 严格读取最近一天的板块联动归因条目。
    pub fn get_latest_board_rotations_strict(&self) -> Result<Vec<BoardRotationRow>, String> {
        let mut conn = match self.get_conn() {
            Ok(connection) => connection,
            Err(error) => {
                return Err(format!("获取 board_rotation_daily 数据库连接失败: {error}"));
            }
        };
        let rows: Vec<BoardRotationQueryRow> = diesel::sql_query(
            "SELECT date, board_code, board_name, news_title, board_change_pct, board_main_net_pct, stocks \
             FROM board_rotation_daily \
             WHERE date = (SELECT MAX(date) FROM board_rotation_daily) \
             ORDER BY board_change_pct DESC, board_main_net_pct DESC",
        )
        .load(&mut conn)
        .map_err(|error| format!("查询 board_rotation_daily 失败: {error}"))?;
        Ok(rows
            .into_iter()
            .map(|r| BoardRotationRow {
                date: r.date,
                board_code: r.board_code,
                board_name: r.board_name,
                news_title: r.news_title,
                board_change_pct: r.board_change_pct,
                board_main_net_pct: r.board_main_net_pct,
                stocks_json: r.stocks,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, path::Path};

    #[test]
    fn local_concept_cache_read_retains_exact_query_rows_and_old_map() {
        let isolated = tempfile::tempdir().expect("isolated concept cache database");
        let db = DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_concept_cache_read.db"),
        )
        .expect("open isolated concept cache database");
        let recent = (Local::now() - Duration::days(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let stale = (Local::now() - Duration::days(8))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let concepts = vec!["TEST_CODE_算力".to_string(), "TEST_CODE_液冷".to_string()];
        for (code, updated_at) in [
            ("TEST_CODE_CACHE_Z", &recent),
            ("TEST_CODE_CACHE_A", &recent),
            ("TEST_CODE_CACHE_STALE", &stale),
        ] {
            db.save_stock_concepts(code, &concepts)
                .expect("seed concept cache row");
            diesel::sql_query("UPDATE stock_concepts SET updated_at = ? WHERE code = ?")
                .bind::<Text, _>(updated_at)
                .bind::<Text, _>(code)
                .execute(&mut db.get_conn().unwrap())
                .expect("set exact row timestamp");
        }

        let before = Utc::now();
        let read = db
            .get_cached_concepts_observed(7)
            .expect("one observed cache query");
        let after = Utc::now();
        assert!(read.observed_at_utc() >= before);
        assert!(read.observed_at_utc() <= after);
        assert!(stale < read.cutoff_local().to_string());
        assert!(recent >= read.cutoff_local().to_string());
        assert_eq!(
            read.rows()
                .iter()
                .map(LocalConceptCacheRow::code)
                .collect::<Vec<_>>(),
            ["TEST_CODE_CACHE_A", "TEST_CODE_CACHE_Z"]
        );
        for row in read.rows() {
            assert_eq!(
                row.concepts_json(),
                serde_json::to_string(&concepts).unwrap()
            );
            assert_eq!(row.concepts(), concepts);
            assert_eq!(row.updated_at(), recent);
        }
        let old_map = db.get_cached_concepts(7).expect("compatible cache map");
        assert_eq!(read.into_map(), old_map);
    }

    #[test]
    fn p05_latest_chain_query_uses_latest_date_and_exact_date_tie_order() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        diesel::sql_query(
            "CREATE TABLE chain_daily (date TEXT NOT NULL, concept TEXT NOT NULL, \
             stocks TEXT NOT NULL, continuation_count INTEGER NOT NULL, \
             PRIMARY KEY (date, concept))",
        )
        .execute(&mut conn)
        .unwrap();
        assert!(query_p05_latest_chain_clusters(&mut conn)
            .unwrap()
            .is_empty());

        for (date, concept, count) in [
            ("2026-09-23", "older", 99),
            ("2026-09-24", "己", 1),
            ("2026-09-24", "乙", 3),
            ("2026-09-24", "戊", 1),
            ("2026-09-24", "甲", 3),
            ("2026-09-24", "丙", 2),
            ("2026-09-24", "丁", 2),
        ] {
            diesel::sql_query(
                "INSERT INTO chain_daily (date, concept, stocks, continuation_count) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind::<Text, _>(date)
            .bind::<Text, _>(concept)
            .bind::<Text, _>("[\"TEST_CODE_000001\"]")
            .bind::<Integer, _>(count)
            .execute(&mut conn)
            .unwrap();
        }
        let rows = query_p05_latest_chain_clusters(&mut conn).unwrap();
        assert_eq!(rows.len(), 6);
        assert!(rows.iter().all(|row| row.date == "2026-09-24"));
        assert_eq!(
            rows.iter()
                .map(|row| row.concept.as_str())
                .collect::<Vec<_>>(),
            // SQLite's default BINARY collation compares the UTF-8 bytes.
            vec!["乙", "甲", "丁", "丙", "己", "戊"]
        );
    }

    #[test]
    fn p01_replace_readback_mismatch_rolls_back_the_whole_generation() {
        let isolated = tempfile::tempdir().expect("isolated P-01 generation database");
        let db = DatabaseManager::open_isolated_for_test(
            isolated.path().join("TEST_CODE_p01_generation.db"),
        )
        .expect("open isolated P-01 generation database");
        let date = chrono::NaiveDate::from_ymd_opt(2198, 11, 17).unwrap();
        db.save_chain_clusters(
            "2198-11-17",
            &[("TEST_CODE_OLD".into(), vec!["TEST_CODE_600099".into()], 1)],
        )
        .expect("seed old same-date row");
        {
            let mut conn = db.get_conn().unwrap();
            diesel::sql_query(
                "CREATE TRIGGER TEST_CODE_mutate_chain_generation AFTER INSERT ON chain_daily \
                 BEGIN UPDATE chain_daily SET stocks='[\"TEST_CODE_600098\"]' \
                 WHERE date=NEW.date AND concept=NEW.concept; END",
            )
            .execute(&mut conn)
            .expect("install isolated read-back mutation");
        }
        let error = db
            .replace_and_read_chain_clusters_for_date_strict(
                date,
                &[("TEST_CODE_NEW".into(), vec!["TEST_CODE_600001".into()], 2)],
            )
            .expect_err("changed projection cannot commit as a producer generation");
        assert!(matches!(error, ChainDailyReplaceError::ReadbackMismatch));
        let rows = db.get_chain_clusters_for_date_strict(date).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].concept, "TEST_CODE_OLD");
        assert_eq!(rows[0].stocks, "[\"TEST_CODE_600099\"]");
    }

    struct ConceptsGuard {
        code: String,
        chain_date: String,
        simhashes: Vec<i64>,
    }

    impl Drop for ConceptsGuard {
        fn drop(&mut self) {
            if let Ok(mut conn) = DatabaseManager::get().get_conn() {
                let _ = diesel::sql_query("DELETE FROM stock_concepts WHERE code = ?")
                    .bind::<Text, _>(&self.code)
                    .execute(&mut conn);
                let _ = diesel::sql_query("DELETE FROM chain_daily WHERE date = ?")
                    .bind::<Text, _>(&self.chain_date)
                    .execute(&mut conn);
                for simhash in &self.simhashes {
                    let _ = diesel::sql_query("DELETE FROM event_seen_simhash WHERE simhash = ?")
                        .bind::<diesel::sql_types::BigInt, _>(*simhash)
                        .execute(&mut conn);
                }
            }
        }
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    }

    fn remove_test_file_if_present(path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    // The board DAO lifecycle uses its own file so another test's global
    // singleton cannot change the assertions or lose its database to cleanup.
    const TEST_DATE: &str = "2099-01-01"; // 远期日期, 不与生产 / 其他测试冲突

    /// B-002 综合测试: 独立数据库验证 round-trip + 排序 + INSERT OR REPLACE.
    #[test]
    fn test_board_rotations_dao_lifecycle() {
        let test_data_dir = Path::new("./test_data");
        let isolated = tempfile::tempdir().expect("isolated board rotations database root");
        let test_db = isolated.path().join("TEST_CODE_board_rotations.db");
        std::fs::create_dir_all(test_data_dir)
            .expect("create TEST_CODE board rotations test data directory");
        let cleanup_probe = test_data_dir.join(format!(
            "TEST_CODE_concepts_cleanup_probe_{}.tmp",
            unique_suffix()
        ));
        std::fs::write(&cleanup_probe, b"TEST_CODE cleanup probe")
            .expect("create TEST_CODE concept cleanup probe");
        remove_test_file_if_present(&cleanup_probe)
            .expect("remove existing TEST_CODE concept cleanup probe");
        remove_test_file_if_present(&cleanup_probe)
            .expect("accept missing TEST_CODE concept cleanup probe");
        let db = DatabaseManager::open_isolated_for_test(test_db)
            .expect("initialize exact TEST_CODE board rotations database");

        // === 场景 1: round-trip, 2 个 board 写入, 验证字段 + 排序 ===
        db.save_board_rotations(
            TEST_DATE,
            &[
                BoardRotationEntry {
                    board_code: "B002_DEV".to_string(),
                    board_name: "[板块联动] 房地产开发".to_string(),
                    news_title: "房地产板块短线拉升，合肥城建涨停".to_string(),
                    board_change_pct: 2.5,
                    board_main_net_pct: 1.5,
                    stocks_json:
                        r#"[{"code":"TEST_CODE_002208","name":"合肥城建","change_pct":10.0}]"#
                            .to_string(),
                },
                BoardRotationEntry {
                    board_code: "B002_BANK".to_string(),
                    board_name: "[板块联动] 银行".to_string(),
                    news_title: "银行板块异动拉升".to_string(),
                    board_change_pct: 1.8,
                    board_main_net_pct: 0.8,
                    stocks_json:
                        r#"[{"code":"TEST_CODE_600036","name":"招商银行","change_pct":5.5}]"#
                            .to_string(),
                },
            ],
        )
        .unwrap();

        let got = db.get_latest_board_rotations();
        let our_rows: Vec<_> = got.iter().filter(|r| r.date == TEST_DATE).collect();
        assert_eq!(our_rows.len(), 2, "应读回 2 条本测试写入的 row");

        // 按 board_change_pct DESC: 房地产开发 (2.5) > 银行 (1.8)
        assert_eq!(our_rows[0].board_code, "B002_DEV");
        assert_eq!(our_rows[0].board_name, "[板块联动] 房地产开发");
        assert_eq!(our_rows[0].board_change_pct, 2.5);
        assert_eq!(our_rows[0].board_main_net_pct, 1.5);
        assert!(our_rows[0].stocks_json.contains("TEST_CODE_002208"));
        assert!(our_rows[0].news_title.contains("房地产板块短线拉升"));

        assert_eq!(our_rows[1].board_code, "B002_BANK");

        // === 场景 2: INSERT OR REPLACE 幂等性 (同 (date, board_code)) ===
        db.save_board_rotations(
            TEST_DATE,
            &[BoardRotationEntry {
                board_code: "B002_DEV".to_string(),
                board_name: "[板块联动] 房地产开发".to_string(),
                news_title: "new title v2".to_string(),
                board_change_pct: 5.0,
                board_main_net_pct: 3.0,
                stocks_json: "[]".to_string(),
            }],
        )
        .unwrap();
        let got = db.get_latest_board_rotations();
        let our_rows: Vec<_> = got.iter().filter(|r| r.date == TEST_DATE).collect();
        assert_eq!(our_rows.len(), 2, "覆盖后应仍 2 条");
        let dev_row = our_rows
            .iter()
            .find(|r| r.board_code == "B002_DEV")
            .unwrap();
        assert_eq!(dev_row.board_change_pct, 5.0, "应保留最新的 change_pct");
        assert!(
            dev_row.news_title.contains("new"),
            "应保留最新的 news_title"
        );

        // === 场景 3: get_latest 只返回 MAX(date) 的数据, 旧 date 应被排除 ===
        db.save_board_rotations(
            "2020-01-01",
            &[BoardRotationEntry {
                board_code: "B002_OLD".to_string(),
                board_name: "old".to_string(),
                news_title: "stale".to_string(),
                board_change_pct: 99.0,
                board_main_net_pct: 99.0,
                stocks_json: "[]".to_string(),
            }],
        )
        .unwrap();
        let got = db.get_latest_board_rotations();
        // MAX(date) 应是 TEST_DATE (2099-01-01 > 2020-01-01)
        assert!(
            !got.iter().any(|r| r.board_code == "B002_OLD"),
            "get_latest 应只返回最新 date 的 row, 不返 2020 的 stale 数据"
        );
    }

    #[test]
    #[serial_test::serial]
    fn p01_chain_read_uses_requested_date_not_max_date() {
        DatabaseManager::init(None).expect("test database init");
        let requested = "2198-12-30".to_string();
        let later = "2198-12-31".to_string();
        let _requested_guard = ConceptsGuard {
            code: "TEST_CODE_P01_REQUESTED".to_string(),
            chain_date: requested.clone(),
            simhashes: Vec::new(),
        };
        let _later_guard = ConceptsGuard {
            code: "TEST_CODE_P01_LATER".to_string(),
            chain_date: later.clone(),
            simhashes: Vec::new(),
        };
        let db = DatabaseManager::get();
        db.save_chain_clusters(
            &requested,
            &[
                (
                    "TEST_CODE_P01_SECOND".to_string(),
                    vec!["TEST_CODE_000002".to_string()],
                    1,
                ),
                (
                    "TEST_CODE_P01_FIRST".to_string(),
                    vec!["TEST_CODE_000001".to_string()],
                    3,
                ),
            ],
        )
        .expect("save requested-date P-01 chain rows");
        db.save_chain_clusters(
            &later,
            &[(
                "TEST_CODE_P01_STALE_MAX".to_string(),
                vec!["TEST_CODE_000003".to_string()],
                9,
            )],
        )
        .expect("save later chain row");

        let rows = db
            .get_chain_clusters_for_date_strict(
                chrono::NaiveDate::parse_from_str(&requested, "%Y-%m-%d")
                    .expect("valid requested date"),
            )
            .expect("exact-date P-01 chain read");

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.date == requested));
        assert_eq!(rows[0].concept, "TEST_CODE_P01_FIRST");
        assert_eq!(rows[1].concept, "TEST_CODE_P01_SECOND");
    }

    #[test]
    #[serial_test::serial]
    fn br101_concept_and_chain_repository_lifecycle_is_strict() {
        DatabaseManager::init(None).expect("test database init");
        let suffix = unique_suffix();
        let code = format!("TEST_CODE_CONCEPT_{suffix}");
        let chain_date = "2199-01-01".to_string();
        let _guard = ConceptsGuard {
            code: code.clone(),
            chain_date: chain_date.clone(),
            simhashes: Vec::new(),
        };
        let db = DatabaseManager::get();

        assert!(db.get_cached_concepts(0).is_err());
        for (bad_code, concepts) in [
            ("", vec!["算力".to_string()]),
            ("TEST_CODE_BAD", Vec::new()),
            ("TEST_CODE_BAD", vec![" ".to_string()]),
        ] {
            assert!(db.save_stock_concepts(bad_code, &concepts).is_err());
        }
        let concepts = vec!["算力".to_string(), "液冷".to_string()];
        db.save_stock_concepts(&code, &concepts)
            .expect("save complete concept evidence");
        let cached = db.get_cached_concepts(1).expect("fresh concept cache");
        assert_eq!(cached.get(&code), Some(&concepts));

        assert!(db
            .save_chain_clusters("bad-date", &[("算力".to_string(), vec![code.clone()], 1)])
            .is_err());
        for bad in [
            ("".to_string(), vec![code.clone()], 1),
            ("算力".to_string(), Vec::new(), 1),
            ("算力".to_string(), vec![" ".to_string()], 1),
            ("算力".to_string(), vec![code.clone()], -1),
        ] {
            assert!(db.save_chain_clusters(&chain_date, &[bad]).is_err());
        }
        db.save_chain_clusters(
            &chain_date,
            &[
                ("算力".to_string(), vec![code.clone()], 2),
                ("液冷".to_string(), vec![code.clone()], 1),
            ],
        )
        .expect("save complete chain batch");
        let latest = db
            .get_latest_chain_clusters_strict()
            .expect("latest chain batch");
        assert_eq!(latest.len(), 2);
        assert!(latest.iter().all(|row| row.date == chain_date));
        assert!(latest.iter().any(|row| {
            row.concept == "算力"
                && row.continuation_count == 2
                && serde_json::from_str::<Vec<String>>(&row.stocks).unwrap() == vec![code.clone()]
        }));
        assert_eq!(db.get_latest_chain_clusters().len(), 2);
        let row_date = chrono::NaiveDate::parse_from_str(&chain_date, "%Y-%m-%d")
            .expect("valid chain business date");
        assert_eq!(
            db.get_chain_appearance_days_as_of_strict("算力", 1, row_date)
                .expect("row at as-of is included"),
            1
        );
        assert_eq!(
            db.get_chain_appearance_days_as_of_strict(
                "算力",
                10,
                row_date
                    .checked_add_signed(chrono::Duration::days(9))
                    .expect("lower-bound as-of"),
            )
            .expect("row at lower bound is included"),
            1
        );
        assert_eq!(
            db.get_chain_appearance_days_as_of_strict(
                "算力",
                10,
                row_date
                    .checked_add_signed(chrono::Duration::days(10))
                    .expect("outside-window as-of"),
            )
            .expect("row below lower bound is excluded"),
            0
        );
        assert_eq!(
            db.get_chain_appearance_days_as_of_strict(
                "算力",
                1,
                row_date.pred_opt().expect("previous date"),
            )
            .expect("row after as-of is excluded"),
            0
        );
        assert!(db
            .get_chain_appearance_days_as_of_strict("", 1, row_date)
            .is_err());
        assert!(db
            .get_chain_appearance_days_as_of_strict("算力", 0, row_date)
            .is_err());
        assert!(db
            .get_chain_appearance_days_as_of_strict("算力", i64::MAX, row_date)
            .is_err());
    }

    #[test]
    #[serial_test::serial]
    fn br068_event_seen_repository_validates_transactions_and_retention() {
        DatabaseManager::init(None).expect("test database init");
        let base = (unique_suffix() % 1_000_000_000) as i64 + 1_000_000_000;
        let first = base;
        let second = base + 1;
        let _guard = ConceptsGuard {
            code: format!("TEST_CODE_UNUSED_{base}"),
            chain_date: "2199-12-31".to_string(),
            simhashes: vec![first, second],
        };
        let db = DatabaseManager::get();
        assert!(db.save_event_seen(&[]).is_ok());
        assert!(db.get_recent_event_seen(0).is_err());
        assert!(db.cleanup_old_event_seen(0).is_err());
        assert!(db
            .save_event_seen(&[EventSeenEntry {
                simhash: u64::MAX,
                title: "越界".to_string(),
            }])
            .is_err());
        assert!(db
            .save_event_seen(&[
                EventSeenEntry {
                    simhash: first as u64,
                    title: "有效但应回滚".to_string(),
                },
                EventSeenEntry {
                    simhash: second as u64,
                    title: " ".to_string(),
                },
            ])
            .is_err());
        let recent = db.get_recent_event_seen(2).expect("recent event evidence");
        assert!(!recent.iter().any(|entry| entry.simhash == first as u64));

        db.save_event_seen(&[
            EventSeenEntry {
                simhash: first as u64,
                title: "算力服务器订单增长".to_string(),
            },
            EventSeenEntry {
                simhash: second as u64,
                title: "液冷产业链扩产".to_string(),
            },
        ])
        .expect("save event evidence batch");
        let recent = db.get_recent_event_seen(2).expect("recent event evidence");
        assert!(recent.iter().any(|entry| {
            entry.simhash == first as u64 && entry.title == "算力服务器订单增长"
        }));
        let mut conn = db.get_conn().expect("test database connection");
        diesel::sql_query(
            "UPDATE event_seen_simhash SET seen_at = '2000-01-01 00:00:00' WHERE simhash = ?",
        )
        .bind::<diesel::sql_types::BigInt, _>(first)
        .execute(&mut conn)
        .expect("age exact test event");
        assert!(
            db.cleanup_old_event_seen(7)
                .expect("event retention cleanup")
                >= 1
        );
        let recent = db
            .get_recent_event_seen(7)
            .expect("retained event evidence");
        assert!(!recent.iter().any(|entry| entry.simhash == first as u64));
        assert!(recent.iter().any(|entry| entry.simhash == second as u64));
    }

    #[test]
    fn br101_board_rotation_rejects_bad_batches_before_writing() {
        DatabaseManager::init(None).expect("test database init");
        let db = DatabaseManager::get();
        let valid = BoardRotationEntry {
            board_code: "TEST_BOARD".to_string(),
            board_name: "测试板块".to_string(),
            news_title: "测试催化".to_string(),
            board_change_pct: 1.0,
            board_main_net_pct: 0.5,
            stocks_json: "[]".to_string(),
        };
        assert!(db
            .save_board_rotations("bad-date", std::slice::from_ref(&valid))
            .is_err());
        let mut empty_code = valid.clone();
        empty_code.board_code.clear();
        let mut empty_name = valid.clone();
        empty_name.board_name.clear();
        let mut empty_title = valid.clone();
        empty_title.news_title.clear();
        let mut bad_net = valid.clone();
        bad_net.board_main_net_pct = f64::INFINITY;
        for bad in [empty_code, empty_name, empty_title, bad_net] {
            assert!(db.save_board_rotations("2199-01-02", &[bad]).is_err());
        }
        let mut bad = valid.clone();
        bad.board_change_pct = f64::NAN;
        assert!(db.save_board_rotations("2199-01-02", &[bad]).is_err());
        let mut bad = valid.clone();
        bad.stocks_json = "not-json".to_string();
        assert!(db.save_board_rotations("2199-01-02", &[bad]).is_err());
        let mut bad = valid;
        bad.stocks_json = "{}".to_string();
        assert!(db.save_board_rotations("2199-01-02", &[bad]).is_err());
    }
}
