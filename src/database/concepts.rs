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
    /// 返回 `code -> 概念列表` 映射；任何错误均降级为空映射（缓存失效不阻断主流程）。
    pub fn get_cached_concepts(&self, max_age_days: i64) -> HashMap<String, Vec<String>> {
        let mut map = HashMap::new();
        let cutoff = (Local::now() - Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(e) => {
                warn!("[概念缓存] 获取数据库连接失败: {}", e);
                return map;
            }
        };

        let rows: Vec<ConceptRow> = match diesel::sql_query(
            "SELECT code, concepts FROM stock_concepts WHERE updated_at >= ?",
        )
        .bind::<Text, _>(&cutoff)
        .load(&mut conn)
        {
            Ok(r) => r,
            Err(e) => {
                warn!("[概念缓存] 查询失败: {}", e);
                return map;
            }
        };

        for row in rows {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(&row.concepts) {
                map.insert(row.code, list);
            }
        }
        map
    }

    /// 写入/覆盖某只股票的概念标签缓存。
    pub fn save_stock_concepts(&self, code: &str, concepts: &[String]) {
        let json = match serde_json::to_string(concepts) {
            Ok(j) => j,
            Err(_) => return,
        };
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(e) => {
                warn!("[概念缓存] 获取数据库连接失败: {}", e);
                return;
            }
        };

        if let Err(e) = diesel::sql_query(
            "INSERT OR REPLACE INTO stock_concepts (code, concepts, updated_at) VALUES (?, ?, ?)",
        )
        .bind::<Text, _>(code)
        .bind::<Text, _>(&json)
        .bind::<Text, _>(&now)
        .execute(&mut conn)
        {
            warn!("[概念缓存] 写入 {} 失败: {}", code, e);
        }
    }

    /// 保存某日的主线簇结果（覆盖同日同概念）。
    pub fn save_chain_clusters(&self, date: &str, clusters: &[(String, Vec<String>, i32)]) {
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(e) => {
                warn!("[主线落库] 获取连接失败: {}", e);
                return;
            }
        };
        for (concept, codes, cont) in clusters {
            let json = match serde_json::to_string(codes) {
                Ok(j) => j,
                Err(_) => continue,
            };
            if let Err(e) = diesel::sql_query(
                "INSERT OR REPLACE INTO chain_daily (date, concept, stocks, continuation_count) VALUES (?, ?, ?, ?)",
            )
            .bind::<Text, _>(date)
            .bind::<Text, _>(concept)
            .bind::<Text, _>(&json)
            .bind::<Integer, _>(*cont)
            .execute(&mut conn)
            {
                warn!("[主线落库] 写入 {} 失败: {}", concept, e);
            }
        }
    }

    /// 读取最近一个有记录日期的主线簇（含当天）。
    pub fn get_latest_chain_clusters(&self) -> Vec<ChainDailyRow> {
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        diesel::sql_query(
            "SELECT date, concept, stocks, continuation_count FROM chain_daily \
             WHERE date = (SELECT MAX(date) FROM chain_daily)",
        )
        .load(&mut conn)
        .unwrap_or_default()
    }

    /// 查某概念主线在最近 N 天内出现的天数（生命周期参考）。
    pub fn get_chain_streak_days(&self, concept: &str, days: i64) -> i64 {
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(_) => return 0,
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
        .unwrap_or_default();
        rows.first().map(|r| r.n).unwrap_or(0)
    }

    // ===== B-003 事件抽取去重 (simhash) DAO =====

    /// 保存一批事件 (simhash, title), 用于下次去重.
    /// CR-7 (review): 用 conn.transaction 包裹循环, N 条事件 1 次 fsync 而非 N 次.
    ///                之前: 5min 一次 run_opportunity_scan, 100 条事件 → 100 次 INSERT + 100 次 lock.
    ///                现在: 1 个事务批量提交, 减少 100x fsync.
    pub fn save_event_seen(&self, entries: &[EventSeenEntry]) {
        if entries.is_empty() {
            return;
        }
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(e) => {
                warn!("[EventSeen] 获取连接失败: {}", e);
                return;
            }
        };
        let result: Result<(), diesel::result::Error> = conn.transaction(|tx| {
            for entry in entries {
                // CR-26 (review): u64 → i64 裸 as cast 会在 >i64::MAX 时变负, 改用 try_from
                // 显式截断为 i64::MAX (防御性, 实际 simhash 64-bit 不应到这么大量级).
                let simhash_i64 = i64::try_from(entry.simhash).unwrap_or(i64::MAX);
                diesel::sql_query(
                    "INSERT OR REPLACE INTO event_seen_simhash (simhash, title) VALUES (?, ?)",
                )
                .bind::<diesel::sql_types::BigInt, _>(simhash_i64)
                .bind::<Text, _>(&entry.title)
                .execute(tx)?;
            }
            Ok(())
        });
        if let Err(e) = result {
            warn!("[EventSeen] 批量写入 {} 条失败: {}", entries.len(), e);
        }
    }

    /// 读取所有近 N 天内的事件去重条目 (供 extract_batch_rules_only_with_seen 跨日去重).
    /// B-003 默认 N=2 (与 max_age 对齐, 不留太久).
    pub fn get_recent_event_seen(&self, max_age_days: i64) -> Vec<EventSeenEntry> {
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        // CR-21 (review): 改用 Utc::now() 与 seen_at CURRENT_TIMESTAMP (UTC) 一致.
        // 之前用 Local::now() 在 TZ 边界 (e.g. Asia/Shanghai UTC+8) 错配, 字符串 lexical 比较
        // 表面上 OK 但语义错位 — Asia/Shanghai 09:00 拉的 cutoff 与 DB UTC 时间错开最多 8h,
        // 导致跨日 dedup 漏判或过判.
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let rows: Vec<EventSeenRow> = match diesel::sql_query(
            "SELECT simhash, title FROM event_seen_simhash WHERE seen_at >= ?",
        )
        .bind::<Text, _>(&cutoff)
        .load(&mut conn)
        {
            Ok(r) => r,
            Err(e) => {
                warn!("[EventSeen] 查询失败: {}", e);
                return Vec::new();
            }
        };
        rows.into_iter()
            .map(|r| EventSeenEntry {
                simhash: r.simhash as u64,
                title: r.title,
            })
            .collect()
    }

    /// 清理过期事件去重条目 (cron 入口, 默认保留 7 天).
    pub fn cleanup_old_event_seen(&self, max_age_days: i64) {
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(_) => return,
        };
        let cutoff = (Local::now() - Duration::days(max_age_days))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let _ = diesel::sql_query("DELETE FROM event_seen_simhash WHERE seen_at < ?")
            .bind::<Text, _>(&cutoff)
            .execute(&mut conn);
    }

    // ===== B-002 板块联动归因 (Board hit) DAO =====

    /// 保存某日的板块联动归因条目 (覆盖同日同 board_code).
    /// CR-7 (review): 用 conn.transaction 批量提交, N 条 1 次 fsync.
    pub fn save_board_rotations(&self, date: &str, entries: &[BoardRotationEntry]) {
        if entries.is_empty() {
            return;
        }
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(e) => {
                warn!("[BoardRotation] 获取连接失败: {}", e);
                return;
            }
        };
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
        if let Err(e) = result {
            warn!("[BoardRotation] 批量写入 {} 条失败: {}", entries.len(), e);
        }
    }

    /// 读取最近一天的所有板块联动归因条目 (含今天).
    /// 按 board_change_pct 降序排列 (最强板块在前), 供 NewsCatalyst 选 top cluster.
    pub fn get_latest_board_rotations(&self) -> Vec<BoardRotationRow> {
        let mut conn = match self.get_conn() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let rows: Vec<BoardRotationQueryRow> = diesel::sql_query(
            "SELECT date, board_code, board_name, news_title, board_change_pct, board_main_net_pct, stocks \
             FROM board_rotation_daily \
             WHERE date = (SELECT MAX(date) FROM board_rotation_daily) \
             ORDER BY board_change_pct DESC, board_main_net_pct DESC",
        )
        .load(&mut conn)
        .unwrap_or_default();
        rows.into_iter()
            .map(|r| BoardRotationRow {
                date: r.date,
                board_code: r.board_code,
                board_name: r.board_name,
                news_title: r.news_title,
                board_change_pct: r.board_change_pct,
                board_main_net_pct: r.board_main_net_pct,
                stocks_json: r.stocks,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    stocks_json: r#"[{"code":"002208","name":"合肥城建","change_pct":10.0}]"#
                        .to_string(),
                },
                BoardRotationEntry {
                    board_code: "B002_BANK".to_string(),
                    board_name: "[板块联动] 银行".to_string(),
                    news_title: "银行板块异动拉升".to_string(),
                    board_change_pct: 1.8,
                    board_main_net_pct: 0.8,
                    stocks_json: r#"[{"code":"600036","name":"招商银行","change_pct":5.5}]"#
                        .to_string(),
                },
            ],
        );

        let got = db.get_latest_board_rotations();
        let our_rows: Vec<_> = got.iter().filter(|r| r.date == TEST_DATE).collect();
        assert_eq!(our_rows.len(), 2, "应读回 2 条本测试写入的 row");

        // 按 board_change_pct DESC: 房地产开发 (2.5) > 银行 (1.8)
        assert_eq!(our_rows[0].board_code, "B002_DEV");
        assert_eq!(our_rows[0].board_name, "[板块联动] 房地产开发");
        assert_eq!(our_rows[0].board_change_pct, 2.5);
        assert_eq!(our_rows[0].board_main_net_pct, 1.5);
        assert!(our_rows[0].stocks_json.contains("002208"));
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
        );
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
        );
        let got = db.get_latest_board_rotations();
        // MAX(date) 应是 TEST_DATE (2099-01-01 > 2020-01-01)
        assert!(
            !got.iter().any(|r| r.board_code == "B002_OLD"),
            "get_latest 应只返回最新 date 的 row, 不返 2020 的 stale 数据"
        );
    }
}
