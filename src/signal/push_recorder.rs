//! v16.3 Commit 2 — 推送票池记录器.
//!
//! 业务 (R3): 推送只入池, 不直接买; intraday_monitor (Commit 3) 消费 pushed_stocks
//!
//! 写入: pushed_stocks 表 (12 字段, 3 索引)
//!   - push_time / push_kind / code / name / push_price (推送快照)
//!   - metric_json: { vol_ratio, price_chg_pct, sector, push_subkind }
//!   - source: "preopen" / "intraday" / "postclose"
//!   - consumed_at / consumed_by / outcome: 留给 Commit 3 intraday_monitor 填
//!
//! 失败策略: log::warn + 返回 Err, 不抛 (调用方决定是否降级)

use crate::database::DatabaseManager;
use chrono::Local;
use diesel::prelude::*;

/// Push record 输入元数据
#[derive(Debug, Clone)]
pub struct PushRecordMeta {
    pub code: String,
    pub name: String,
    /// "D-01" / "盘后资金" / "I-01" / "I-03" / "P-02" / "AuctionAnomaly"
    pub push_kind: String,
    pub push_price: f64,
    /// serialize!({vol_ratio, price_chg_pct, sector, push_subkind})
    pub metric_json: String,
    /// "preopen" / "intraday" / "postclose"
    pub source: String,
}

#[derive(diesel::QueryableByName)]
struct LastRowId {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
}

/// 记录 1 条推送 → pushed_stocks 表
///
/// 返回 inserted id (自增主键). 失败返回 Err.
pub fn record(meta: &PushRecordMeta) -> Result<i64, String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let now = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

    // v16.3 review fix (Issue #4): 参数化绑定替代 format! 拼接 (name 来自网络行情数据)
    diesel::sql_query(
        "INSERT INTO pushed_stocks (push_time, push_kind, code, name, push_price, metric_json, source) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<diesel::sql_types::Text, _>(&now)
    .bind::<diesel::sql_types::Text, _>(&meta.push_kind)
    .bind::<diesel::sql_types::Text, _>(&meta.code)
    .bind::<diesel::sql_types::Text, _>(&meta.name)
    .bind::<diesel::sql_types::Double, _>(meta.push_price)
    .bind::<diesel::sql_types::Text, _>(&meta.metric_json)
    .bind::<diesel::sql_types::Text, _>(&meta.source)
        .execute(&mut conn)
        .map_err(|e| {
            log::warn!(
                "[push_recorder] 插入失败 {}({}): {}",
                meta.name, meta.code, e
            );
            format!("insert pushed_stocks: {}", e)
        })?;

    let row = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result::<LastRowId>(&mut conn)
        .map_err(|e| format!("get last_insert_rowid: {}", e))?;

    log::info!(
        "[push_recorder] 入池 {}({}) kind={} source={} → pushed_stocks#{}",
        meta.name, meta.code, meta.push_kind, meta.source, row.id
    );
    Ok(row.id)
}

// ============ Unit tests ============

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_record_meta_clone() {
        let m = PushRecordMeta {
            code: "000001".to_string(),
            name: "测试".to_string(),
            push_kind: "D-01".to_string(),
            push_price: 10.5,
            metric_json: r#"{"vol_ratio": 3.0, "push_subkind": "AuctionVolume"}"#.to_string(),
            source: "intraday".to_string(),
        };
        let m2 = m.clone();
        assert_eq!(m.code, m2.code);
        assert_eq!(m.metric_json, m2.metric_json);
    }

    #[test]
    fn push_record_meta_debug() {
        let m = PushRecordMeta {
            code: "000001".to_string(),
            name: "测试".to_string(),
            push_kind: "D-01".to_string(),
            push_price: 10.5,
            metric_json: "{}".to_string(),
            source: "preopen".to_string(),
        };
        let s = format!("{:?}", m);
        assert!(s.contains("D-01"));
        assert!(s.contains("000001"));
    }
}
