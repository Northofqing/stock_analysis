//! Registered business rules: BR-068, BR-101, BR-102.
//! 股票概念板块标签缓存。
//!
//! 概念标签（东财 F10 核心题材）变化缓慢，落库缓存避免每日重复请求。
//! 供 `pipeline::chain_analysis` 产业链聚类使用。

use chrono::{Duration, Local};
use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};
use log::warn;
use std::collections::HashMap;

use crate::database::DatabaseManager;

#[derive(QueryableByName)]
struct ConceptRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    concepts: String,
}

/// chain_daily 行：某日某主线簇。
#[derive(QueryableByName)]
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
        if max_age_days <= 0 {
            return Err(format!("概念缓存 max_age_days 非法: {max_age_days}"));
        }
        let mut map = HashMap::new();
        let cutoff = (Local::now() - Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let mut conn = self
            .get_conn()
            .map_err(|error| format!("概念缓存获取数据库连接失败: {error}"))?;

        let rows: Vec<ConceptRow> =
            diesel::sql_query("SELECT code, concepts FROM stock_concepts WHERE updated_at >= ?")
                .bind::<Text, _>(&cutoff)
                .load(&mut conn)
                .map_err(|error| format!("概念缓存查询失败: {error}"))?;

        for row in rows {
            if row.code.trim().is_empty() {
                return Err("概念缓存存在空 code".to_string());
            }
            let list = serde_json::from_str::<Vec<String>>(&row.concepts)
                .map_err(|error| format!("概念缓存 {} JSON 非法: {error}", row.code))?;
            if list.is_empty() || list.iter().any(|concept| concept.trim().is_empty()) {
                return Err(format!("概念缓存 {} 含空概念列表/字段", row.code));
            }
            map.insert(row.code, list);
        }
        Ok(map)
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

    /// 查某概念主线在最近 N 天内出现的天数（生命周期参考）。
    pub fn get_chain_streak_days(&self, concept: &str, days: i64) -> i64 {
        match self.get_chain_streak_days_strict(concept, days) {
            Ok(days) => days,
            Err(error) => {
                warn!("[主线生命周期] {}", error);
                0
            }
        }
    }

    /// 严格查询某概念主线在最近 N 天内出现的天数。
    pub fn get_chain_streak_days_strict(&self, concept: &str, days: i64) -> Result<i64, String> {
        if concept.trim().is_empty() || days <= 0 {
            return Err(format!(
                "主线生命周期参数非法: concept={concept:?} days={days}"
            ));
        }
        let mut conn = match self.get_conn() {
            Ok(connection) => connection,
            Err(error) => return Err(format!("获取 chain_daily 连接失败: {error}")),
        };
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            n: i64,
        }
        let cutoff = (Local::now() - Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        let rows: Vec<CountRow> = diesel::sql_query(
            "SELECT COUNT(DISTINCT date) AS n FROM chain_daily WHERE concept = ? AND date >= ?",
        )
        .bind::<Text, _>(concept)
        .bind::<Text, _>(&cutoff)
        .load(&mut conn)
        .map_err(|error| format!("查询 chain_daily 主线生命周期失败: {error}"))?;
        rows.first()
            .map(|row| row.n)
            .ok_or_else(|| "chain_daily 主线生命周期聚合结果缺失".to_string())
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

    // B-002 DAO 测试约束: DatabaseManager 是 OnceCell 单例, init() 仅生效一次.
    // 全部 DAO 测试共用同一份 ./test_data/test.db, 用唯一 date 隔离, 不假定空表.
    const TEST_DATE: &str = "2099-01-01"; // 远期日期, 不与生产 / 其他测试冲突

    /// B-002 综合测试: 一次 init 验证 round-trip + 排序 + INSERT OR REPLACE.
    /// 拆 3 个独立测试因 OnceCell 限制都共享同一 DB, 写在一起避免干扰.
    #[test]
    fn test_board_rotations_dao_lifecycle() {
        let test_db = std::path::PathBuf::from("./test_data/test.db");
        std::fs::create_dir_all("./test_data").ok();
        if test_db.exists() {
            let _ = std::fs::remove_file(&test_db);
        }
        let _ = DatabaseManager::init(Some(test_db));
        let db = DatabaseManager::get();

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
        assert_eq!(
            db.get_chain_streak_days_strict("算力", 1)
                .expect("chain streak"),
            1
        );
        assert_eq!(db.get_chain_streak_days("算力", 1), 1);
        assert_eq!(db.get_chain_streak_days("", 0), 0);
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
