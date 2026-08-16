//! A-10 题材催化复盘 — 名单快照装载 (与生产 dispatcher 同一函数)。
//!
//! 从 `src/bin/monitor/push_templates.rs` 搬移 (2026-08-12): 让 T+1 跟踪
//! backfill 工具与生产 A-10 推送共用同一条 snapshot 装载路径 (BR-160:
//! 只消费 unified Gateway 的 exact visible batch, 无 legacy 回退源)。
//! `push_templates.rs` 通过 `pub use` re-export 保持现有引用不变。

use chrono::NaiveDate;

use crate::database::chain_intelligence::{VisibleChain, VisibleChainBatch, VisibleChainMember};

/// v13 §14.3 A-10 题材催化复盘 — 持续性
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PersistentLevel {
    High,
    Med,
    #[default]
    Low,
}

#[derive(Debug, Clone, Default)]
pub struct CatalystReviewSnapshot {
    pub date: String,
    pub source_batch_id: String,
    pub source_content_hash: String,
    pub source_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub theme: String,
    pub score: Option<f32>,
    pub persistent: PersistentLevel,
    pub member_count: usize,
    pub continuous_count: usize,
    pub leading_members: Vec<String>,
    pub other_members: Vec<String>,
    /// T+1 跟踪: 名单成员 (code/name/streak) 落库快照, 来自
    /// chain_intelligence_members.instrument_id + streak (与 leading_members 同序)
    pub leading_entries: Vec<crate::database::catalyst_watchlist::WatchEntry>,
    pub other_entries: Vec<crate::database::catalyst_watchlist::WatchEntry>,
    pub watch_point: Option<String>,
}

fn parse_a10_source_observed_at(
    value: &str,
) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp);
    }
    let utc = if let Some(milliseconds) = value.strip_prefix("unix-ms:") {
        let milliseconds = milliseconds
            .parse::<i64>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(milliseconds)
    } else if let Some((seconds, nanos)) = value.split_once('.') {
        if seconds.is_empty()
            || nanos.is_empty()
            || nanos.len() > 9
            || !nanos.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(format!("A-10 source observed_at is invalid: {value:?}"));
        }
        let seconds = seconds
            .parse::<i64>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        let padded_nanos = format!("{nanos:0<9}")
            .parse::<u32>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, padded_nanos)
    } else {
        let raw = value
            .parse::<i64>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        if raw.unsigned_abs() >= 100_000_000_000_000_000 {
            let seconds = raw.div_euclid(1_000_000_000);
            let nanos = u32::try_from(raw.rem_euclid(1_000_000_000))
                .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
            chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, nanos)
        } else if raw.unsigned_abs() >= 100_000_000_000 {
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(raw)
        } else {
            chrono::DateTime::<chrono::Utc>::from_timestamp(raw, 0)
        }
    }
    .ok_or_else(|| format!("A-10 source observed_at is out of range: {value:?}"))?;
    Ok(utc.fixed_offset())
}

pub fn catalyst_review_from_chain_batch(
    batch: &VisibleChainBatch,
) -> Result<CatalystReviewSnapshot, String> {
    let Some(top) = batch.chains.first() else {
        return Ok(CatalystReviewSnapshot {
            date: batch.trading_date.format("%Y-%m-%d").to_string(),
            ..CatalystReviewSnapshot::default()
        });
    };
    if top.board_name.trim().is_empty() {
        return Err("A-10 visible chain has an empty source-backed board name".to_string());
    }
    if top.members.len() < 3 || top.upper_limit_count != top.members.len() as i32 {
        return Err(format!(
            "A-10 visible chain {} violates member-count contract",
            top.chain_id
        ));
    }
    let entries = top
        .members
        .iter()
        .map(|member| {
            let name = member.security_name.trim();
            let code = member.instrument_id.trim();
            if name.is_empty() {
                return Err("A-10 visible chain contains an empty security name".to_string());
            }
            if code.is_empty() {
                return Err("A-10 visible chain contains an empty instrument id".to_string());
            }
            Ok(crate::database::catalyst_watchlist::WatchEntry {
                code: code.to_string(),
                name: name.to_string(),
                streak: i64::from(member.streak.max(0)),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    let continuous_count = usize::try_from(top.continuous_count).map_err(|_| {
        format!(
            "A-10 visible chain {} has negative continuous count",
            top.chain_id
        )
    })?;
    if continuous_count > names.len() {
        return Err(format!(
            "A-10 visible chain {} has continuous count above member count",
            top.chain_id
        ));
    }
    let persistent = if continuous_count >= 3 {
        PersistentLevel::High
    } else if continuous_count >= 1 {
        PersistentLevel::Med
    } else {
        PersistentLevel::Low
    };
    let source_observed_at = batch
        .inputs
        .iter()
        .map(|input| parse_a10_source_observed_at(&input.observed_at))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| "A-10 visible chain batch has no input observation time".to_string())?;
    Ok(CatalystReviewSnapshot {
        date: batch.trading_date.format("%Y-%m-%d").to_string(),
        source_batch_id: batch.batch_id.clone(),
        source_content_hash: batch.content_hash.clone(),
        source_observed_at: Some(source_observed_at),
        theme: top.board_name.clone(),
        score: None,
        persistent,
        member_count: names.len(),
        continuous_count,
        leading_members: entries.iter().take(3).map(|e| e.name.clone()).collect(),
        other_members: entries
            .iter()
            .skip(3)
            .take(3)
            .map(|e| e.name.clone())
            .collect(),
        leading_entries: entries.iter().take(3).cloned().collect(),
        other_entries: entries.iter().skip(3).take(3).cloned().collect(),
        // The admitted chain batch has no independent next-day volume/trend
        // evidence. Keep the field absent instead of fabricating advice from
        // the board name.
        watch_point: None,
    })
}

/// BR-160: A-10 only consumes the exact visible batch published by the
/// unified Gateway. Stale `chain_daily`, local rotation caches, and direct
/// name lookups are not fallback sources.
///
/// M4c 双路 (用户决策「2 op 服务端化 → 全 gRPC → 移除」):
/// - gRPC 模式 (DATA_GATEWAY_GRPC=1): 经 op 61 (market.chain_batch) 从服务端
///   取完整 VisibleChainBatch — A-10 计算+stage+publish 副作用在服务端进程
///   执行 (单写方), 本地只做 catalyst_review_from_chain_batch 纯转换。
/// - library 模式 (默认): 本地 build_for_date 重算路径不变 (v15.x 出声)。
///
/// 桥失败 → Err (fail-closed, 绝不静默回退 library 重算 — 双算会双写 chain_daily)。
pub async fn load_catalyst_review_snapshot_real(
    date: &str,
) -> Result<CatalystReviewSnapshot, String> {
    if let Some(batch) = crate::data_gateway::grpc_source::fetch_chain_batch_grpc(date)
        .await
        .map_err(|error| format!("A-10 gRPC chain_batch 获取失败: {error}"))?
    {
        return catalyst_review_from_chain_batch(&batch);
    }
    let review_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("A-10 非法复盘日期 {date}: {error}"))?;
    // no-feature (monitor 零 magic): library 重算 transport 不存在。
    // 无 bridge 时显式失败 (fail-closed), 绝不静默回退/双写旧缓存产物。
    #[cfg(not(feature = "magic-gateway"))]
    {
        return Err(format!(
            "A-10 library transport disabled: DATA_GATEWAY_GRPC=1 required"
        ));
    }
    #[cfg(feature = "magic-gateway")]
    {
        let batch = crate::data_gateway::ChainIntelligenceGateway::new()
            .build_for_date(review_date)
            .await
            .map_err(|error| error.to_string())?;
        if batch.trading_date != review_date {
            return Err(format!(
                "A-10 visible batch as_of={} differs from requested {}",
                batch.trading_date, review_date
            ));
        }
        catalyst_review_from_chain_batch(&batch)
    }
    #[cfg(not(feature = "magic-gateway"))]
    {
        unreachable!("A-10 library transport disabled guard returned above")
    }
}

/// BR-160 历史回放 (backfill 专用): 读取指定交易日「最早落库」的 visible batch
/// — 即当日首次盘后复盘推送消费的那一版 — 不重新计算。
///
/// 与 `load_catalyst_review_snapshot_real` 的差异: 后者每次重新 build, 次日
/// 重算会得到与推送时不同的名单 (2026-08-12 实测 8/11 前排变 3/6: 秦安/京投/
/// 第一医药 → 豪尔赛/恒银/百合花); 回放则忠实还原用户当晚看到的名单。
pub fn load_catalyst_review_snapshot_stored(date: &str) -> Result<CatalystReviewSnapshot, String> {
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let batch = replay_visible_batch_with_conn(&mut conn, date)?;
    catalyst_review_from_chain_batch(&batch)
}

fn replay_visible_batch_with_conn(
    conn: &mut diesel::SqliteConnection,
    date: &str,
) -> Result<VisibleChainBatch, String> {
    use diesel::prelude::*;
    use diesel::sql_types::{Integer, Text};

    #[derive(QueryableByName)]
    struct StoredBatchRow {
        #[diesel(sql_type = Text)]
        batch_id: String,
        #[diesel(sql_type = Text)]
        content_hash: String,
        #[diesel(sql_type = Text)]
        trading_date: String,
        #[diesel(sql_type = Text)]
        calculation_version: String,
        #[diesel(sql_type = Text)]
        taxonomy_version: String,
    }
    #[derive(QueryableByName)]
    struct StoredChainRow {
        #[diesel(sql_type = Text)]
        chain_row_id: String,
        #[diesel(sql_type = Text)]
        chain_id: String,
        #[diesel(sql_type = Text)]
        canonical_board_id: String,
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = Integer)]
        upper_limit_count: i32,
        #[diesel(sql_type = Integer)]
        continuous_count: i32,
    }
    #[derive(QueryableByName)]
    struct StoredMemberRow {
        #[diesel(sql_type = Text)]
        instrument_id: String,
        #[diesel(sql_type = Text)]
        security_name: String,
        #[diesel(sql_type = Text)]
        source_event_id: String,
        #[diesel(sql_type = Integer)]
        streak: i32,
    }
    #[derive(QueryableByName)]
    struct StoredInputRow {
        #[diesel(sql_type = Text)]
        input_id: String,
        #[diesel(sql_type = Integer)]
        ordinal: i32,
        #[diesel(sql_type = Text)]
        capability: String,
        #[diesel(sql_type = Text)]
        provider: String,
        #[diesel(sql_type = Text)]
        source: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
        source_at: Option<String>,
        #[diesel(sql_type = Text)]
        observed_at: String,
        #[diesel(sql_type = Text)]
        source_batch_id: String,
        #[diesel(sql_type = Text)]
        source_batch_hash: String,
        #[diesel(sql_type = Text)]
        content_hash: String,
    }
    #[derive(QueryableByName)]
    struct StoredRejectionRow {
        #[diesel(sql_type = Text)]
        rejection_id: String,
        #[diesel(sql_type = Integer)]
        ordinal: i32,
        #[diesel(sql_type = Text)]
        identity_hash: String,
        #[diesel(sql_type = Text)]
        reason_code: String,
        #[diesel(sql_type = Integer)]
        retryable: i32,
        #[diesel(sql_type = Text)]
        content_hash: String,
    }

    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("A-10 非法复盘日期 {date}: {error}"))?;
    let batch_row: Option<StoredBatchRow> = diesel::sql_query(
        "SELECT batch_id, content_hash, trading_date, calculation_version, taxonomy_version
         FROM chain_intelligence_batches WHERE trading_date = ?
         ORDER BY recorded_at ASC, batch_id ASC LIMIT 1",
    )
    .bind::<Text, _>(date)
    .get_result(conn)
    .optional()
    .map_err(|error| format!("A-10 stored batch query failed: {error}"))?;
    let Some(batch_row) = batch_row else {
        return Err(format!("A-10 无 {date} 已落库 visible batch, 无法回放"));
    };
    let chain_rows: Vec<StoredChainRow> = diesel::sql_query(
        "SELECT chain_row_id, chain_id, canonical_board_id, board_name,
                upper_limit_count, continuous_count
         FROM chain_intelligence_chains WHERE batch_id = ? ORDER BY ordinal ASC",
    )
    .bind::<Text, _>(&batch_row.batch_id)
    .load(conn)
    .map_err(|error| format!("A-10 stored chains query failed: {error}"))?;
    let mut chains = Vec::with_capacity(chain_rows.len());
    for chain_row in chain_rows {
        let members: Vec<StoredMemberRow> = diesel::sql_query(
            "SELECT instrument_id, security_name, source_event_id, streak
             FROM chain_intelligence_members WHERE chain_row_id = ? ORDER BY ordinal ASC",
        )
        .bind::<Text, _>(&chain_row.chain_row_id)
        .load(conn)
        .map_err(|error| format!("A-10 stored members query failed: {error}"))?;
        chains.push(VisibleChain {
            chain_id: chain_row.chain_id,
            canonical_board_id: chain_row.canonical_board_id,
            board_name: chain_row.board_name,
            upper_limit_count: chain_row.upper_limit_count,
            continuous_count: chain_row.continuous_count,
            members: members
                .into_iter()
                .map(|member| VisibleChainMember {
                    instrument_id: member.instrument_id,
                    security_name: member.security_name,
                    source_event_id: member.source_event_id,
                    streak: member.streak,
                })
                .collect(),
        });
    }
    let inputs: Vec<StoredInputRow> = diesel::sql_query(
        "SELECT input_id, ordinal, capability, provider, source, source_at, observed_at,
                source_batch_id, source_batch_hash, content_hash
         FROM chain_intelligence_input_evidence WHERE batch_id = ? ORDER BY ordinal ASC",
    )
    .bind::<Text, _>(&batch_row.batch_id)
    .load(conn)
    .map_err(|error| format!("A-10 stored inputs query failed: {error}"))?;
    let rejections: Vec<StoredRejectionRow> = diesel::sql_query(
        "SELECT rejection_id, ordinal, identity_hash, reason_code, retryable, content_hash
         FROM chain_intelligence_rejections WHERE batch_id = ? ORDER BY ordinal ASC",
    )
    .bind::<Text, _>(&batch_row.batch_id)
    .load(conn)
    .map_err(|error| format!("A-10 stored rejections query failed: {error}"))?;
    Ok(VisibleChainBatch {
        batch_id: batch_row.batch_id,
        content_hash: batch_row.content_hash,
        trading_date: NaiveDate::parse_from_str(&batch_row.trading_date, "%Y-%m-%d")
            .map_err(|error| format!("A-10 stored batch 非法日期: {error}"))?,
        calculation_version: batch_row.calculation_version,
        taxonomy_version: batch_row.taxonomy_version,
        inputs: inputs
            .into_iter()
            .map(
                |input| crate::database::chain_intelligence::ChainInputEvidenceInput {
                    input_id: input.input_id,
                    ordinal: input.ordinal,
                    capability: input.capability,
                    provider: input.provider,
                    source: input.source,
                    source_at: input.source_at,
                    observed_at: input.observed_at,
                    source_batch_id: input.source_batch_id,
                    source_batch_hash: input.source_batch_hash,
                    content_hash: input.content_hash,
                },
            )
            .collect(),
        chains,
        rejections: rejections
            .into_iter()
            .map(
                |rejection| crate::database::chain_intelligence::ChainRejectionInput {
                    rejection_id: rejection.rejection_id,
                    ordinal: rejection.ordinal,
                    identity_hash: rejection.identity_hash,
                    reason_code: rejection.reason_code,
                    retryable: rejection.retryable != 0,
                    content_hash: rejection.content_hash,
                },
            )
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::chain_intelligence::{
        ChainInputEvidenceInput, VisibleChain, VisibleChainBatch, VisibleChainMember,
    };

    fn visible_chain_batch(names: &[&str]) -> VisibleChainBatch {
        VisibleChainBatch {
            batch_id: "TEST_CODE_chain_batch".to_string(),
            content_hash: "a".repeat(64),
            trading_date: chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            calculation_version: "TEST_CODE_v1".to_string(),
            taxonomy_version: "TEST_CODE_taxonomy_v1".to_string(),
            inputs: vec![ChainInputEvidenceInput {
                input_id: "TEST_CODE_chain_input".to_string(),
                ordinal: 0,
                capability: "TEST_CODE_limit_pool".to_string(),
                provider: "TEST_CODE_provider".to_string(),
                source: "TEST_CODE_source".to_string(),
                source_at: Some("2099-01-02".to_string()),
                observed_at: "2099-01-02T15:00:00+08:00".to_string(),
                source_batch_id: "TEST_CODE_input_batch".to_string(),
                source_batch_hash: "b".repeat(64),
                content_hash: "c".repeat(64),
            }],
            chains: vec![VisibleChain {
                chain_id: "TEST_CODE_chain".to_string(),
                canonical_board_id: "TEST_CODE_board".to_string(),
                board_name: "测试主线".to_string(),
                upper_limit_count: i32::try_from(names.len()).unwrap(),
                continuous_count: 3,
                members: names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| VisibleChainMember {
                        instrument_id: format!("TEST_CODE_{index:06}"),
                        security_name: (*name).to_string(),
                        source_event_id: format!("TEST_CODE_event_{index}"),
                        streak: i32::try_from(names.len() - index).unwrap(),
                    })
                    .collect(),
            }],
            rejections: vec![],
        }
    }

    #[test]
    fn br160_a10_maps_only_the_visible_gateway_batch() {
        let batch = visible_chain_batch(&["测试一", "测试二", "测试三", "测试四"]);
        let snapshot =
            catalyst_review_from_chain_batch(&batch).expect("visible batch maps to A-10");
        assert_eq!(snapshot.date, "2099-01-02");
        assert_eq!(snapshot.source_batch_id, batch.batch_id);
        assert_eq!(snapshot.source_content_hash, batch.content_hash);
        assert_eq!(
            snapshot.source_observed_at,
            Some(chrono::DateTime::parse_from_rfc3339("2099-01-02T15:00:00+08:00").unwrap())
        );
        assert_eq!(snapshot.theme, "测试主线");
        assert_eq!(snapshot.persistent, PersistentLevel::High);
        assert_eq!(snapshot.member_count, 4);
        assert_eq!(snapshot.continuous_count, 3);
        assert_eq!(snapshot.leading_members, ["测试一", "测试二", "测试三"]);
        assert_eq!(snapshot.other_members, ["测试四"]);
        // T+1: 代码/连板与名字同步提取 (同序)
        assert_eq!(snapshot.leading_entries.len(), 3);
        assert_eq!(snapshot.leading_entries[0].code, "TEST_CODE_000000");
        assert_eq!(snapshot.leading_entries[0].streak, 4);
        assert_eq!(snapshot.other_entries.len(), 1);
        assert_eq!(snapshot.other_entries[0].code, "TEST_CODE_000003");
        assert_eq!(snapshot.other_entries[0].streak, 1);
        assert_eq!(snapshot.score, None);
        assert_eq!(snapshot.watch_point, None);
    }

    #[test]
    fn br160_a10_rejects_visible_batch_count_contradiction() {
        let mut batch = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        batch.chains[0].upper_limit_count = 4;
        let error = catalyst_review_from_chain_batch(&batch)
            .expect_err("contradictory visible batch must fail");
        assert!(error.contains("member-count contract"));
    }

    #[test]
    fn br160_a10_rejects_missing_or_invalid_input_observation_time() {
        let mut batch = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        batch.inputs[0].observed_at = "TEST_CODE_invalid_observation".to_string();
        let error = catalyst_review_from_chain_batch(&batch)
            .expect_err("invalid source observation must fail before delivery");
        assert!(error.contains("source observed_at is invalid"), "{error}");

        batch.inputs.clear();
        let error = catalyst_review_from_chain_batch(&batch)
            .expect_err("missing source observation must fail before delivery");
        assert!(error.contains("no input observation time"), "{error}");
    }

    #[test]
    fn br160_a10_rejects_invalid_continuous_counts() {
        let mut above_member_count = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        above_member_count.chains[0].continuous_count = 4;
        let error = catalyst_review_from_chain_batch(&above_member_count)
            .expect_err("continuous count above member count must fail");
        assert!(error.contains("continuous count above member count"));

        let mut negative = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        negative.chains[0].continuous_count = -1;
        let error = catalyst_review_from_chain_batch(&negative)
            .expect_err("negative continuous count must fail");
        assert!(error.contains("negative continuous count"));
    }

    #[test]
    fn br160_a10_stored_replay_returns_earliest_batch_for_date() {
        use diesel::connection::SimpleConnection;
        use diesel::prelude::*;
        use diesel::sql_types::{Integer, Text};

        let mut conn = diesel::SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute("PRAGMA foreign_keys = ON;").unwrap();
        crate::database::chain_intelligence::create_schema(&mut conn)
            .expect("chain_intelligence schema");

        // 同一交易日两份已落库 batch: 回放必须取最早 (推送消费的那一版)。
        for (batch_id, recorded_at, board_name, name) in [
            (
                "TEST_CODE_batch_early",
                "2026-08-11T11:01:00.000Z",
                "早盘主线",
                "TEST_CODE_早盘龙头",
            ),
            (
                "TEST_CODE_batch_late",
                "2026-08-12T04:08:41.000Z",
                "重算主线",
                "TEST_CODE_重算龙头",
            ),
        ] {
            diesel::sql_query(
                "INSERT INTO chain_intelligence_batches
                    (batch_id, content_hash, trading_date, calculation_version,
                     taxonomy_version, created_at, recorded_at)
                 VALUES (?, ?, '2026-08-11', 'TEST_CODE_v1', 'TEST_CODE_taxonomy_v1',
                         '2026-08-11T00:00:00Z', ?)",
            )
            .bind::<Text, _>(batch_id)
            .bind::<Text, _>("a".repeat(64))
            .bind::<Text, _>(recorded_at)
            .execute(&mut conn)
            .unwrap();
            diesel::sql_query(
                "INSERT INTO chain_intelligence_chains
                    (chain_row_id, batch_id, chain_id, canonical_board_id, board_name,
                     ordinal, upper_limit_count, continuous_count, content_hash)
                 VALUES (?, ?, ?, ?, ?, 0, 3, 3, ?)",
            )
            .bind::<Text, _>(format!("{batch_id}_chain_row"))
            .bind::<Text, _>(batch_id)
            .bind::<Text, _>(format!("{batch_id}_chain"))
            .bind::<Text, _>(format!("{batch_id}_board"))
            .bind::<Text, _>(board_name)
            .bind::<Text, _>("b".repeat(64))
            .execute(&mut conn)
            .unwrap();
            diesel::sql_query(
                "INSERT INTO chain_intelligence_input_evidence
                    (input_id, batch_id, ordinal, capability, provider, source,
                     source_at, observed_at, source_batch_id, source_batch_hash, content_hash)
                 VALUES (?, ?, 0, 'TEST_CODE_limit_pool', 'TEST_CODE_provider',
                         'TEST_CODE_source', '2026-08-11', '2026-08-11T11:01:00+08:00',
                         ?, ?, ?)",
            )
            .bind::<Text, _>(format!("{batch_id}_input"))
            .bind::<Text, _>(batch_id)
            .bind::<Text, _>(format!("{batch_id}_source_batch"))
            .bind::<Text, _>("d".repeat(64))
            .bind::<Text, _>("e".repeat(64))
            .execute(&mut conn)
            .unwrap();
            for ordinal in 0..3 {
                diesel::sql_query(
                    "INSERT INTO chain_intelligence_members
                        (member_id, chain_row_id, ordinal, instrument_id, security_name,
                         source_event_id, streak, content_hash)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind::<Text, _>(format!("{batch_id}_member_{ordinal}"))
                .bind::<Text, _>(format!("{batch_id}_chain_row"))
                .bind::<Integer, _>(ordinal)
                .bind::<Text, _>(format!("TEST_CODE_60000{ordinal}"))
                .bind::<Text, _>(if ordinal == 0 {
                    name
                } else {
                    "TEST_CODE_同板成员"
                })
                .bind::<Text, _>(format!("{batch_id}_event_{ordinal}"))
                .bind::<Integer, _>(1)
                .bind::<Text, _>("c".repeat(64))
                .execute(&mut conn)
                .unwrap();
            }
        }

        let replay =
            replay_visible_batch_with_conn(&mut conn, "2026-08-11").expect("stored batch replays");
        let snapshot = catalyst_review_from_chain_batch(&replay).expect("maps to A-10");
        // 最早落库版: 用户当晚在推送里看到的名单
        assert_eq!(snapshot.theme, "早盘主线");
        assert_eq!(snapshot.leading_entries[0].name, "TEST_CODE_早盘龙头");
        assert_eq!(snapshot.leading_entries[0].code, "TEST_CODE_600000");
        assert!(replay.batch_id.starts_with("TEST_CODE_batch_early"));

        let missing = replay_visible_batch_with_conn(&mut conn, "2026-08-10");
        assert!(
            missing.unwrap_err().contains("无 2026-08-10"),
            "missing date must fail loudly"
        );
    }

    #[test]
    fn br160_a10_loader_has_no_legacy_source_fallback() {
        let source = include_str!("catalyst_review.rs");
        let start = source
            .find("pub async fn load_catalyst_review_snapshot_real")
            .expect("A-10 loader");
        let end = source[start..]
            .find("// ============================================================================")
            .map(|offset| start + offset)
            .unwrap_or(source.len());
        let loader = &source[start..end];
        for forbidden in [
            "chain_daily",
            "board_rotation_daily",
            "DataFetcherManager",
            "get_latest_chain_clusters",
            "get_latest_board_rotations",
        ] {
            assert!(
                !loader.contains(forbidden),
                "legacy A-10 source: {forbidden}"
            );
        }
        assert!(loader.contains("ChainIntelligenceGateway"));
    }
}
