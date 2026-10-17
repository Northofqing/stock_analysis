//! 日线 + 均线补齐（P2 数据基础, 2026-10-17）。
//!
//! ## 为什么需要这条
//!
//! `stock_daily` 的 ma5/ma10/ma20/volume_ratio 从建表起就没人写过（全库 0 行有值），
//! 而 `outcome-data` 的日线入库又被 trading-authority 门挡住了（authority_deferred）。
//!
//! 这条路径**与 authority 门解耦**：直接用 HistoricalBars 拉最近 N 个交易日，
//! 在本模块内算均线并写回。代价是均线是"我们自算的第二套口径"，将来 authority
//! 通了如果数值不一致，要对齐。
//!
//! ## 口径
//!
//! - maN = 最近 N 个交易日收盘价的算术平均（**含当日**）
//! - volume_ratio = 当日成交量 / 最近 5 个交易日成交量均值（含当日）
//! - 数据不足 N 条的日期：对应均线写 NULL（BR-222，不补值）
//!
//! ## 标的范围
//!
//! 当日 active tickets ∪ 当前实际持仓（与 P1.3 价格带落库同一口径）。

use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};
use serde_json::json;

use crate::database::DatabaseManager;

/// 拉多少天：ma20 需要 20 条，多拉 10 天给停牌/缺口留余量。
const LOOKBACK_DAYS: usize = 30;
const MA_SHORT: usize = 5;
const MA_MID: usize = 10;
const MA_LONG: usize = 20;

#[derive(Debug, Default, Clone)]
pub struct DailyBarsReport {
    pub targets: usize,
    pub fetched: usize,
    pub persisted_bars: usize,
    pub with_ma20: usize,
    pub errors: Vec<String>,
}

impl DailyBarsReport {
    pub fn log(&self) {
        log::info!(
            "[日线均线] targets={} fetched={} persisted_bars={} with_ma20={}",
            self.targets,
            self.fetched,
            self.persisted_bars,
            self.with_ma20
        );
        for error in &self.errors {
            log::warn!("[日线均线] {error}");
        }
    }
}

fn collect_target_codes(conn: &mut SqliteConnection) -> Result<Vec<String>, String> {
    #[derive(QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        code: String,
    }

    let mut codes: BTreeSet<String> = BTreeSet::new();
    let rows: Vec<CodeRow> =
        diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL")
            .load(conn)
            .unwrap_or_default();
    for row in rows {
        codes.insert(row.code);
    }
    for position in crate::portfolio::load_positions().unwrap_or_default() {
        codes.insert(position.code);
    }
    Ok(codes.into_iter().collect())
}

/// 均值（含当日）。不足窗口返回 None。
fn window_mean(values: &[f64], end_inclusive: usize, window: usize) -> Option<f64> {
    if end_inclusive + 1 < window {
        return None;
    }
    let slice = &values[end_inclusive + 1 - window..=end_inclusive];
    let sum: f64 = slice.iter().sum();
    Some(sum / window as f64)
}

pub async fn refresh_daily_bars_and_ma(
    db: &DatabaseManager,
    as_of: &str,
) -> Result<DailyBarsReport, String> {
    let mut report = DailyBarsReport::default();

    let codes = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        collect_target_codes(&mut conn)?
    };
    report.targets = codes.len();
    if codes.is_empty() {
        return Ok(report);
    }

    let bridge =
        crate::data_gateway::grpc_source::bridge_for("HistoricalBars").map_err(|e| e.to_string())?;

    // 先全部拉取（async），再统一写库，避免长时间持有数据库连接
    let mut fetched: Vec<(String, Vec<crate::data_provider::KlineData>, String)> = Vec::new();
    for code in &codes {
        match bridge.daily_bars_async(code, LOOKBACK_DAYS).await {
            Ok(crate::data_gateway::GatewayBatch::Available { records, evidence }) => {
                report.fetched += 1;
                fetched.push((code.clone(), records, format!("{:?}", evidence.provider)));
            }
            Ok(crate::data_gateway::GatewayBatch::VerifiedEmpty(_)) => {
                report.errors.push(format!("{code}: 返回空批次"));
            }
            Err(error) => report.errors.push(format!("{code}: {error}")),
        }
    }
    if fetched.is_empty() {
        return Ok(report);
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    for (code, mut bars, source) in fetched {
        bars.sort_by_key(|bar| bar.date);
        let closes: Vec<f64> = bars.iter().map(|bar| bar.close).collect();
        let volumes: Vec<f64> = bars.iter().map(|bar| bar.volume).collect();

        for (index, bar) in bars.iter().enumerate() {
            // 只写 as_of 当天及更早的行（历史行补 ma 也一并写，便于后续回填）
            let ma5 = window_mean(&closes, index, MA_SHORT);
            let ma10 = window_mean(&closes, index, MA_MID);
            let ma20 = window_mean(&closes, index, MA_LONG);
            let volume_ratio = window_mean(&volumes, index, MA_SHORT).and_then(|average| {
                if average > 0.0 {
                    Some(bar.volume / average)
                } else {
                    None
                }
            });
            if ma20.is_some() {
                report.with_ma20 += 1;
            }

            let written = diesel::sql_query(
                "INSERT INTO stock_daily (\
                    code, date, open, high, low, close, volume, amount, pct_chg,\
                    ma5, ma10, ma20, volume_ratio, data_source\
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)\
                 ON CONFLICT(code, date) DO UPDATE SET \
                    open=excluded.open,\
                    high=excluded.high,\
                    low=excluded.low,\
                    close=excluded.close,\
                    volume=excluded.volume,\
                    amount=excluded.amount,\
                    pct_chg=excluded.pct_chg,\
                    ma5=excluded.ma5,\
                    ma10=excluded.ma10,\
                    ma20=excluded.ma20,\
                    volume_ratio=excluded.volume_ratio,\
                    data_source=excluded.data_source,\
                    updated_at=CURRENT_TIMESTAMP",
            )
            .bind::<Text, _>(&code)
            .bind::<Text, _>(bar.date.to_string())
            .bind::<Nullable<Double>, _>(Some(bar.open))
            .bind::<Nullable<Double>, _>(Some(bar.high))
            .bind::<Nullable<Double>, _>(Some(bar.low))
            .bind::<Nullable<Double>, _>(Some(bar.close))
            .bind::<Nullable<Double>, _>(Some(bar.volume))
            .bind::<Nullable<Double>, _>(Some(bar.amount))
            .bind::<Nullable<Double>, _>(Some(bar.pct_chg))
            .bind::<Nullable<Double>, _>(ma5)
            .bind::<Nullable<Double>, _>(ma10)
            .bind::<Nullable<Double>, _>(ma20)
            .bind::<Nullable<Double>, _>(volume_ratio)
            .bind::<Nullable<Text>, _>(Some(source.as_str()))
            .execute(&mut *conn)
            .map_err(|e| format!("{code} {} 写入失败: {e}", bar.date))?;
            report.persisted_bars += written;
        }
    }

    let _ = json!({"as_of": as_of});
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_mean_requires_full_window() {
        let values = [10.0, 11.0, 12.0];
        assert_eq!(window_mean(&values, 2, 3), Some(11.0));
        assert_eq!(window_mean(&values, 1, 3), None);
    }

    #[test]
    fn window_mean_includes_current_day() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        // 含当日：(1+2+3+4+5)/5 = 3
        assert_eq!(window_mean(&values, 4, 5), Some(3.0));
    }
}
