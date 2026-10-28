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

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};
use serde_json::json;

use crate::database::DatabaseManager;

/// 拉多少天：ma20 需要 20 条，多拉 10 天给停牌/缺口留余量。
const LOOKBACK_DAYS: usize = 30;

/// 单只标的的拉取硬超时（2026-10-22）。
///
/// 本地桥在 provider 全部失败时会一只票耗 30~60 秒（`router_sources_exhausted`），
/// 全市场一轮因此能拖到 90 分钟以上；超时即记失败并跳过，不再等。
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 每多少只落一次库（2026-10-22）。
///
/// 原来是「全部拉完再一次性事务」：一轮 90 分钟，中途被杀就一条都不写。
/// 分批提交后已拉到的部分先落盘，重跑仍是幂等 upsert。
const PERSIST_CHUNK: usize = 200;

/// 单轮拉取的时间预算（2026-10-22）。
///
/// 本机桥被别的高频任务（分析师一致预期失败重试等）挤住时，HistoricalBars 会整体变慢；
/// 没有预算的话一轮能跑通宵，把第二天的盘前链路一起拖住。到点即停止取数、把已拿到的
/// 批次提交后返回，未覆盖的部分留给下一轮或手工 CLI 补。
const FETCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(45 * 60);
const MA_SHORT: usize = 5;
const MA_MID: usize = 10;
const MA_LONG: usize = 20;

#[derive(Debug, Default, Clone)]
pub struct DailyBarsReport {
    pub targets: usize,
    pub fetched: usize,
    pub persisted_bars: usize,
    pub with_ma20: usize,
    /// 失败/跳过的标的数（`errors` 只保留前 [`MAX_REPORTED_ERRORS`] 条明细）。
    pub failed: usize,
    pub errors: Vec<String>,
}

/// 失败明细最多保留多少条（全市场一轮失败上百只是常态，逐条打日志会淹没可读性）。
const MAX_REPORTED_ERRORS: usize = 20;

impl DailyBarsReport {
    fn note_failure(&mut self, message: String) {
        self.failed += 1;
        if self.errors.len() < MAX_REPORTED_ERRORS {
            self.errors.push(message);
        }
    }

    pub fn log(&self) {
        log::info!(
            "[日线均线] targets={} fetched={} persisted_bars={} with_ma20={} failed={}",
            self.targets,
            self.fetched,
            self.persisted_bars,
            self.with_ma20,
            self.failed
        );
        for error in &self.errors {
            log::warn!("[日线均线] {error}");
        }
        if self.failed > self.errors.len() {
            log::warn!(
                "[日线均线] 其余 {} 条失败明细省略",
                self.failed - self.errors.len()
            );
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

/// 全市场口径（2026-10-22）：概念票 ∪ 板块成分 ∪ 活跃 ticket ∪ 持仓。
///
/// 供 21:30 全量刷新使用；`collect_target_codes` 保持「活跃池」口径不变
/// （15:55 轻量轮只服务 P2 的逐票均线）。板块轮动的「上一交易日涨幅排序」和
/// 归因基准都依赖全市场日线，只用活跃池会把候选面压到几十只。
pub fn collect_full_universe_codes(
    conn: &mut SqliteConnection,
) -> Result<Vec<String>, String> {
    #[derive(QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        code: String,
    }
    let rows: Vec<CodeRow> = diesel::sql_query(
        "SELECT code FROM ( \
           SELECT code FROM stock_concepts \
           UNION SELECT instrument_code AS code FROM board_member \
           UNION SELECT code FROM ticket_lifecycle WHERE close_date IS NULL \
           UNION SELECT code FROM stock_position WHERE status = 'open') \
         WHERE code IS NOT NULL AND code != '' \
         ORDER BY code",
    )
    .load(conn)
    .map_err(|e| format!("全市场标的查询失败: {e}"))?;
    Ok(rows.into_iter().map(|row| row.code).collect())
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
    codes_override: Option<&[String]>,
) -> Result<DailyBarsReport, String> {
    let mut report = DailyBarsReport::default();

    let codes = match codes_override {
        // 显式指定标的：补齐新闻票、板块成分等非 ticket 标的的历史日线。
        Some(list) => list.to_vec(),
        None => {
            let mut conn = db.get_conn().map_err(|e| e.to_string())?;
            collect_target_codes(&mut conn)?
        }
    };
    report.targets = codes.len();
    if codes.is_empty() {
        return Ok(report);
    }

    let bridge = crate::data_gateway::grpc_source::bridge_for("HistoricalBars")
        .map_err(|e| e.to_string())?;

    // 2026-10-22：单只硬超时 + 分批落库。
    // 全市场 2965 只时，失败码每只 30~60 秒会把一轮拖到 90 分钟以上，
    // 而「全部拉完再写」意味着中途被杀就一条都不落盘。
    let mut pending: Vec<(String, Vec<crate::data_provider::KlineData>, String)> =
        Vec::with_capacity(PERSIST_CHUNK);
    let fetch_started = std::time::Instant::now();
    for (index, code) in codes.iter().enumerate() {
        if fetch_started.elapsed() >= FETCH_BUDGET {
            log::warn!(
                "[日线均线] 拉取预算 {} 分钟用尽，已处理 {}/{}，剩余交给下一轮或 CLI 补",
                FETCH_BUDGET.as_secs() / 60,
                index,
                codes.len()
            );
            break;
        }
        match tokio::time::timeout(FETCH_TIMEOUT, bridge.daily_bars_async(code, LOOKBACK_DAYS)).await
        {
            Ok(Ok(crate::data_gateway::GatewayBatch::Available { records, evidence })) => {
                report.fetched += 1;
                pending.push((code.clone(), records, format!("{:?}", evidence.provider)));
            }
            Ok(Ok(crate::data_gateway::GatewayBatch::VerifiedEmpty(_))) => {
                report.note_failure(format!("{code}: 返回空批次"));
            }
            Ok(Err(error)) => report.note_failure(format!("{code}: {error}")),
            Err(_) => report.note_failure(format!(
                "{code}: 拉取超时（>{}s），跳过",
                FETCH_TIMEOUT.as_secs()
            )),
        }
        if pending.len() >= PERSIST_CHUNK {
            persist_bars_batch(db, &mut pending, &mut report)?;
            log::info!(
                "[日线均线] 进度 {}/{} fetched={} failed={}",
                index + 1,
                codes.len(),
                report.fetched,
                report.failed
            );
        }
    }
    persist_bars_batch(db, &mut pending, &mut report)?;

    let _ = json!({"as_of": as_of});
    Ok(report)
}

/// 把一批已拉到的日线写进 `stock_daily`（单事务），成功后清空该批。
///
/// 2026-10-22：写入必须在一个事务里——逐条 INSERT 各自提交时实测只有
/// ≈1500 行/分钟（8.9 万行要 1 小时），单事务后是秒级；批之间仍可增量落盘。
fn persist_bars_batch(
    db: &DatabaseManager,
    pending: &mut Vec<(String, Vec<crate::data_provider::KlineData>, String)>,
    report: &mut DailyBarsReport,
) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    conn.batch_execute("BEGIN IMMEDIATE")
        .map_err(|e| format!("日线写入事务开启失败: {e}"))?;
    let mut write_error: Option<String> = None;
    'batch: for entry in pending.iter_mut() {
        let (code, bars, source) = entry;
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
            .bind::<Text, _>(code.as_str())
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
            .execute(&mut *conn);
            match written {
                Ok(written) => report.persisted_bars += written,
                Err(error) => {
                    write_error = Some(format!("{code} {} 写入失败: {error}", bar.date));
                    break 'batch;
                }
            }
        }
    }
    match write_error {
        Some(error) => {
            let _ = conn.batch_execute("ROLLBACK");
            return Err(error);
        }
        None => conn
            .batch_execute("COMMIT")
            .map_err(|e| format!("日线写入提交失败: {e}"))?,
    }
    pending.clear();
    Ok(())
}

/// 用 stock_daily 自身的历史收盘重算某交易日的 ma5/ma10/ma20 与量比。
///
/// TDX `.day` 导入只写 OHLCV，不写均线；15:40 的 buy_zone 依赖 ma5/ma10，
/// 不补这一步会让所有 newly synced 的票在日线后仍被判 `no_data`。
pub fn recompute_ma_for_date(
    db: &DatabaseManager,
    date_text: &str,
) -> Result<usize, String> {
    #[derive(diesel::QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        code: String,
    }
    #[derive(diesel::QueryableByName)]
    struct BarRow {
        #[diesel(sql_type = Double)]
        close: f64,
        #[diesel(sql_type = Nullable<Double>)]
        volume: Option<f64>,
    }
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let codes: Vec<CodeRow> = diesel::sql_query(
        "SELECT DISTINCT code FROM stock_daily WHERE date = ?1 AND close IS NOT NULL",
    )
    .bind::<Text, _>(date_text)
    .load(&mut conn)
    .map_err(|error| format!("均线重算标的查询失败: {error}"))?;
    conn.batch_execute("BEGIN IMMEDIATE")
        .map_err(|error| format!("均线重算事务开启失败: {error}"))?;
    let mut updated = 0usize;
    for code in codes {
        let bars: Vec<BarRow> = match diesel::sql_query(
            "SELECT close, volume FROM stock_daily \
             WHERE code = ?1 AND date <= ?2 AND close IS NOT NULL \
             ORDER BY date DESC LIMIT 20",
        )
        .bind::<Text, _>(code.code.as_str())
        .bind::<Text, _>(date_text)
        .load(&mut conn)
        {
            Ok(bars) => bars,
            Err(error) => {
                let _ = conn.batch_execute("ROLLBACK");
                return Err(format!("{} 均线历史查询失败: {error}", code.code));
            }
        };
        if bars.len() < MA_SHORT {
            continue;
        }
        let mut closes: Vec<f64> = bars.iter().map(|bar| bar.close).collect();
        closes.reverse();
        let current = closes.len() - 1;
        let ma5 = window_mean(&closes, current, MA_SHORT);
        let ma10 = window_mean(&closes, current, MA_MID);
        let ma20 = window_mean(&closes, current, MA_LONG);
        let volume_ratio = bars
            .first()
            .and_then(|bar| bar.volume)
            .and_then(|volume| {
                let volumes: Vec<f64> = bars
                    .iter()
                    .take(MA_SHORT)
                    .filter_map(|bar| bar.volume)
                    .collect();
                if volumes.len() == MA_SHORT {
                    let mean = volumes.iter().sum::<f64>() / MA_SHORT as f64;
                    (mean > 0.0).then_some(volume / mean)
                } else {
                    None
                }
            });
        let result = diesel::sql_query(
            "UPDATE stock_daily SET ma5 = ?1, ma10 = ?2, ma20 = ?3, volume_ratio = ?4 \
             WHERE code = ?5 AND date = ?6",
        )
        .bind::<Nullable<Double>, _>(ma5)
        .bind::<Nullable<Double>, _>(ma10)
        .bind::<Nullable<Double>, _>(ma20)
        .bind::<Nullable<Double>, _>(volume_ratio)
        .bind::<Text, _>(code.code.as_str())
        .bind::<Text, _>(date_text)
        .execute(&mut conn);
        match result {
            Ok(rows) => updated += rows,
            Err(error) => {
                let _ = conn.batch_execute("ROLLBACK");
                return Err(format!("{} 均线写入失败: {error}", code.code));
            }
        }
    }
    conn.batch_execute("COMMIT")
        .map_err(|error| format!("均线重算提交失败: {error}"))?;
    Ok(updated)
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
