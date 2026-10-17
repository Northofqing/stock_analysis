//! op 67 价格带每日落库（P1.3, 2026-10-17）。
//!
//! ## 为什么必须每日落库
//!
//! 对端明确：`price_band` 只在**观测快照日**有值（行情快照本身不携带历史价格带）。
//! 也就是说「今天不查，昨天的价格带就永久拿不到」——不能等用的时候再查。
//!
//! ## 存什么、不存什么
//!
//! 存：code / trade_date / 来源 / listing_state / listed_on / limit_up / limit_down /
//!     price_band_source / evidence_batch / observed_at。
//! 不存：任何推导值。非快照日 `limit_up/limit_down` 保持 NULL（BR-222）。
//!
//! ## 与权威模型的边界
//!
//! 这里落的是 **provider-observed 观察值**（`quality` 恒 `provider_observed`），
//! 不是 `trading_facts_authority` 的权威证明。消费方必须看 `quality` 字段区分。

use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::Text;
use serde_json::{json, Value};

use crate::data_gateway::provider_observed_trading_facts::{self, ProviderObservedTradingFact};
use crate::data_gateway::grpc_source;
use crate::database::DatabaseManager;

/// op 67 的 instruments 上限（对端清单 §1.2）。
const MAX_INSTRUMENTS_PER_QUERY: usize = 50;

#[derive(Debug, Default, Clone)]
pub struct SnapshotReport {
    pub targets: usize,
    pub queried: usize,
    pub persisted: usize,
    pub without_price_band: usize,
    pub errors: Vec<String>,
}

impl SnapshotReport {
    pub fn log(&self) {
        log::info!(
            "[价格带落库] targets={} queried={} persisted={} without_price_band={}",
            self.targets,
            self.queried,
            self.persisted,
            self.without_price_band
        );
        for error in &self.errors {
            log::warn!("[价格带落库] {error}");
        }
    }
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS trading_fact_snapshot (\
            code TEXT NOT NULL,\
            trade_date TEXT NOT NULL,\
            quality TEXT NOT NULL,\
            source TEXT NOT NULL,\
            listing_state TEXT,\
            listed_on TEXT,\
            limit_up REAL,\
            limit_down REAL,\
            price_band_source TEXT,\
            evidence_batch TEXT,\
            observed_at TEXT NOT NULL,\
            recorded_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
            PRIMARY KEY (code, trade_date)\
        )",
    )
    .execute(conn)
    .map_err(|e| format!("create trading_fact_snapshot: {e}"))?;
    Ok(())
}

/// 标的集合 = 当日 active tickets + 当前实际持仓。
fn collect_target_codes(conn: &mut SqliteConnection) -> Result<Vec<String>, String> {
    #[derive(QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        code: String,
    }

    let mut codes: BTreeSet<String> = BTreeSet::new();

    let ticket_rows: Vec<CodeRow> = diesel::sql_query(
        "SELECT code FROM ticket_lifecycle WHERE close_date IS NULL",
    )
    .load(conn)
    .unwrap_or_default();
    for row in ticket_rows {
        codes.insert(row.code);
    }

    for position in crate::portfolio::load_positions().unwrap_or_default() {
        codes.insert(position.code);
    }

    Ok(codes.into_iter().collect())
}

/// 6 → Shanghai；0/3 → Shenzhen（与项目其它 InstrumentId 推断一致）。
fn exchange_for(code: &str) -> &'static str {
    match code.chars().next() {
        Some('6') => "Shanghai",
        Some('0') | Some('3') => "Shenzhen",
        _ => "Shanghai",
    }
}

pub async fn refresh_daily_snapshots(
    db: &DatabaseManager,
    trade_date: &str,
) -> Result<SnapshotReport, String> {
    let mut report = SnapshotReport::default();

    let codes = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        // 懒建表：本模块不属于 database::init 的建表清单（避免 monitor→database 反向依赖）
        create_schema(&mut *conn)?;
        collect_target_codes(&mut conn)?
    };
    report.targets = codes.len();
    if codes.is_empty() {
        return Ok(report);
    }

    let bridge = grpc_source::bridge_for("QualifiedTradingFacts")
        .map_err(|e| format!("grpc bridge unavailable: {e}"))?;

    for chunk in codes.chunks(MAX_INSTRUMENTS_PER_QUERY) {
        let instruments: Vec<Value> = chunk
            .iter()
            .map(|code| {
                json!({
                    "exchange": exchange_for(code),
                    "code": code,
                    "asset_class": "Equity",
                })
            })
            .collect();
        let params = json!({
            "instruments": instruments,
            "dates": [trade_date],
            "include": ["lifecycle", "price_band"],
        });

        let result = match bridge.qualified_trading_facts_async(params).await {
            Ok(result) => result,
            Err(error) => {
                report
                    .errors
                    .push(format!("查询失败 ({}..): {error}", &chunk[0]));
                continue;
            }
        };
        if !result.diagnostic_blocker.is_empty() {
            report.errors.push(format!(
                "诊断读取被拒 ({}..): {}",
                &chunk[0], result.diagnostic_blocker
            ));
            continue;
        }

        for record in &result.records {
            let value: Value = match serde_json::from_slice(&record.data) {
                Ok(value) => value,
                Err(error) => {
                    report.errors.push(format!("record 非 JSON: {error}"));
                    continue;
                }
            };
            let fact = match provider_observed_trading_facts::parse_fact(&value) {
                Ok(fact) => fact,
                Err(error) => {
                    report.errors.push(format!("record 解析失败: {error}"));
                    continue;
                }
            };
            report.queried += 1;
            match persist(db, &fact) {
                Ok(()) => {
                    report.persisted += 1;
                    if !fact.price_band_available() {
                        report.without_price_band += 1;
                    }
                }
                Err(error) => report.errors.push(format!("{} 落库失败: {error}", fact.code)),
            }
        }
    }

    Ok(report)
}

fn persist(db: &DatabaseManager, fact: &ProviderObservedTradingFact) -> Result<(), String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let quality = match fact.quality {
        provider_observed_trading_facts::TradingFactQuality::ProviderObserved => {
            "provider_observed"
        }
    };
    let listing_state = fact.listing_state.map(|state| match state {
        provider_observed_trading_facts::ListingState::Listed => "listed",
        provider_observed_trading_facts::ListingState::NotYetListed => "not_yet_listed",
        provider_observed_trading_facts::ListingState::Unknown => "unknown",
    });
    let evidence_batch = fact
        .price_band_evidence
        .as_ref()
        .map(|evidence| evidence.batch_id.as_str());

    diesel::sql_query(
        "INSERT INTO trading_fact_snapshot (\
            code, trade_date, quality, source, listing_state, listed_on,\
            limit_up, limit_down, price_band_source, evidence_batch, observed_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)\
         ON CONFLICT(code, trade_date) DO UPDATE SET \
            quality=excluded.quality,\
            source=excluded.source,\
            listing_state=excluded.listing_state,\
            listed_on=excluded.listed_on,\
            limit_up=excluded.limit_up,\
            limit_down=excluded.limit_down,\
            price_band_source=excluded.price_band_source,\
            evidence_batch=excluded.evidence_batch,\
            observed_at=excluded.observed_at,\
            recorded_at=CURRENT_TIMESTAMP",
    )
    .bind::<Text, _>(&fact.code)
    .bind::<Text, _>(&fact.date)
    .bind::<Text, _>(quality)
    .bind::<Text, _>(fact.source.as_str())
    .bind::<diesel::sql_types::Nullable<Text>, _>(listing_state)
    .bind::<diesel::sql_types::Nullable<Text>, _>(fact.listed_on.as_deref())
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(fact.limit_up)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(fact.limit_down)
    .bind::<diesel::sql_types::Nullable<Text>, _>(
        fact.price_band_source.map(|source| source.as_str()),
    )
    .bind::<diesel::sql_types::Nullable<Text>, _>(evidence_batch)
    .bind::<Text, _>(&fact.observed_at)
    .execute(&mut *conn)
    .map_err(|e| format!("upsert snapshot: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_infers_shanghai_for_6_and_shenzhen_for_0_or_3() {
        assert_eq!(exchange_for("600396"), "Shanghai");
        assert_eq!(exchange_for("000813"), "Shenzhen");
        assert_eq!(exchange_for("300750"), "Shenzhen");
    }
}
