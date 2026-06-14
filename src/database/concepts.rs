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
}
