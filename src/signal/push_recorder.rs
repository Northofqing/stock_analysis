//! v16.3 Commit 2 — 推送票池记录器.
//! Registered business rule: BR-126.
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
    for (field, value) in [
        ("code", meta.code.as_str()),
        ("name", meta.name.as_str()),
        ("push_kind", meta.push_kind.as_str()),
        ("source", meta.source.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("pushed_stocks {field} 不能为空"));
        }
    }
    if !meta.push_price.is_finite() || meta.push_price <= 0.0 {
        return Err(format!(
            "pushed_stocks {} push_price 非法: {}",
            meta.code, meta.push_price
        ));
    }
    let metrics: serde_json::Value = serde_json::from_str(&meta.metric_json)
        .map_err(|error| format!("pushed_stocks {} metric_json 非法: {error}", meta.code))?;
    if !metrics.is_object() {
        return Err(format!(
            "pushed_stocks {} metric_json 必须是对象",
            meta.code
        ));
    }

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
        meta.name,
        meta.code,
        meta.push_kind,
        meta.source,
        row.id
    );
    Ok(row.id)
}

// ============ Unit tests ============

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_meta() -> PushRecordMeta {
        PushRecordMeta {
            code: format!(
                "TEST_CODE_PUSH_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time")
                    .as_nanos()
            ),
            name: "测试推送".to_string(),
            push_kind: "P-02".to_string(),
            push_price: 10.5,
            metric_json: serde_json::json!({
                "vol_ratio": 6.0,
                "price_chg_pct": 1.0,
                "push_subkind": "AuctionVolume"
            })
            .to_string(),
            source: "preopen".to_string(),
        }
    }

    #[test]
    fn push_record_meta_clone() {
        let m = PushRecordMeta {
            code: "TEST_CODE_000001".to_string(),
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
            code: "TEST_CODE_000001".to_string(),
            name: "测试".to_string(),
            push_kind: "D-01".to_string(),
            push_price: 10.5,
            metric_json: "{}".to_string(),
            source: "preopen".to_string(),
        };
        let s = format!("{:?}", m);
        assert!(s.contains("D-01"));
        assert!(s.contains("TEST_CODE_000001"));
    }

    #[test]
    #[serial_test::serial]
    fn br126_record_rejects_bad_rows_and_persists_complete_audit_row() {
        DatabaseManager::init(None).expect("test database init");
        let base = valid_meta();
        let mut cases = Vec::new();
        for field in ["code", "name", "push_kind", "source"] {
            let mut invalid = base.clone();
            match field {
                "code" => invalid.code = " ".to_string(),
                "name" => invalid.name.clear(),
                "push_kind" => invalid.push_kind.clear(),
                "source" => invalid.source = "\t".to_string(),
                _ => unreachable!(),
            }
            cases.push((field, invalid));
        }
        for (label, price) in [("zero", 0.0), ("negative", -1.0), ("nan", f64::NAN)] {
            let mut invalid = base.clone();
            invalid.push_price = price;
            cases.push((label, invalid));
        }
        for (label, json) in [("bad-json", "not-json"), ("array-json", "[]")] {
            let mut invalid = base.clone();
            invalid.metric_json = json.to_string();
            cases.push((label, invalid));
        }
        for (label, invalid) in cases {
            assert!(record(&invalid).is_err(), "case={label}");
        }

        #[derive(QueryableByName)]
        struct StoredPush {
            #[diesel(sql_type = diesel::sql_types::Text)]
            code: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            metric_json: String,
            #[diesel(sql_type = diesel::sql_types::Text)]
            source: String,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
            consumed_at: Option<String>,
        }
        let id = record(&base).expect("valid push audit row");
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test database connection");
        let stored = diesel::sql_query(
            "SELECT code, metric_json, source, consumed_at FROM pushed_stocks WHERE id = ?",
        )
        .bind::<diesel::sql_types::BigInt, _>(id)
        .get_result::<StoredPush>(&mut conn)
        .expect("stored push row");
        assert_eq!(stored.code, base.code);
        assert_eq!(stored.metric_json, base.metric_json);
        assert_eq!(stored.source, "preopen");
        assert!(stored.consumed_at.is_none());
        diesel::sql_query("DELETE FROM pushed_stocks WHERE id = ?")
            .bind::<diesel::sql_types::BigInt, _>(id)
            .execute(&mut conn)
            .expect("clean exact test push");
    }
}
