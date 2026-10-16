//! 建议结果回填 (2026-10-16).
//!
//! 读 `advice_outcome_tracking` 的未回填行 → 以推送日 (`business_date`) 收盘为
//! 基准, 读 `stock_daily` 的 T+1/T+3/T+5 收盘 + 观察窗 high/low → 算
//! `change_t1/t3/t5` + `mfe/mae` → 写回表.
//!
//! 口径: "假设执行"的建议质量 (推送日收盘 → 后续 N 个交易日), **不是用户真实
//!       收益**. 停牌日不参与 (`stock_daily.is_suspended = 0`).
//!
//! 数据缺失的处理 (BR-222: 不补 0):
//!   - 观察窗 (T+5) 还没走完           → 本轮跳过, 下轮再算 (滚动回填)
//!   - 观察窗已走完但 high/low 无数据   → 同上; 若基准日距今超过 `VOID_AFTER_DAYS`
//!     自然日仍无数据, 标记作废 (只写说明, 数值列保持 NULL), 不再重扫
//!   - 基准日当日本身无收盘 (停牌/未采集) → 同上超期规则
//!   - 单个期数 (如 T+3) 缺收盘         → 该期记 NULL, 不阻碍整行写回
//!
//! 注意: `qualified_daily_trading_status` 只覆盖数百只票 / 每日十几行 (窄集合),
//!       因此本模块只依赖 `stock_daily.is_suspended` —— 那是行情源的真实字段,
//!       且覆盖全量已采集票.

use crate::database::advice_outcome_tracking::{
    apply_outcome, load_pending_outcomes, mark_outcome_void, FilledOutcome,
};
use crate::database::DatabaseManager;
use chrono::NaiveDate;
use diesel::{OptionalExtension, RunQueryDsl};
use stock_analysis::calendar::verified_next_a_share_trading_day;

/// 单轮扫描上限 (防止一次回填拖太久).
const BACKFILL_BATCH_LIMIT: i64 = 500;
/// 观察窗止于 T+5.
const HORIZON_DAYS: u32 = 5;
/// 基准日距今超过这么多自然日仍取不到数据 → 标记作废, 不再重扫.
const VOID_AFTER_DAYS: i64 = 30;

#[derive(Debug, Clone, Default)]
pub struct BackfillReport {
    /// 本轮取出的待回填行数
    pub scanned: usize,
    /// 成功写回的行数
    pub filled: usize,
    /// 观察窗未走完或数据未到 (滚动下轮再算)
    pub not_mature: usize,
    /// 超期无数据, 已标记作废 (数值列仍为 NULL)
    pub voided: usize,
    pub errors: Vec<String>,
}

impl BackfillReport {
    pub fn log(&self) {
        if self.scanned == 0 {
            log::info!("[建议回填] 无待回填行");
            return;
        }
        log::info!(
            "[建议回填] scanned={} filled={} not_mature={} voided={}",
            self.scanned,
            self.filled,
            self.not_mature,
            self.voided
        );
        for error in &self.errors {
            log::warn!("[建议回填] {error}");
        }
    }
}

/// 读指定日的收盘价; 停牌或无记录 → None.
fn read_close(db: &DatabaseManager, code: &str, date: &str) -> Result<Option<f64>, String> {
    #[derive(diesel::QueryableByName)]
    struct Close {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        close: Option<f64>,
    }
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let row = diesel::sql_query(
        "SELECT close FROM stock_daily \
         WHERE code = ?1 AND date = ?2 AND is_suspended = 0 LIMIT 1",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(date)
    .get_result::<Close>(&mut *conn)
    .optional()
    .map_err(|e| format!("close read failed ({code} {date}): {e}"))?;
    Ok(row
        .and_then(|r| r.close)
        .filter(|c| c.is_finite() && *c > 0.))
}

/// 读区间 [from, to] 的最高/最低价 (仅非停牌行); 无数据 → None.
fn read_high_low(
    db: &DatabaseManager,
    code: &str,
    from: &str,
    to: &str,
) -> Result<Option<(f64, f64, i64)>, String> {
    #[derive(diesel::QueryableByName)]
    struct Hl {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        hi: Option<f64>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        lo: Option<f64>,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let row = diesel::sql_query(
        "SELECT MAX(high) AS hi, MIN(low) AS lo, COUNT(*) AS n FROM stock_daily \
         WHERE code = ?1 AND date >= ?2 AND date <= ?3 AND is_suspended = 0",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(from)
    .bind::<diesel::sql_types::Text, _>(to)
    .get_result::<Hl>(&mut *conn)
    .optional()
    .map_err(|e| format!("high/low read failed ({code} {from}..{to}): {e}"))?;
    match row {
        Some(Hl {
            hi: Some(hi),
            lo: Some(lo),
            n,
        }) if hi.is_finite() && lo.is_finite() && lo > 0. && n > 0 => Ok(Some((hi, lo, n))),
        _ => Ok(None),
    }
}

/// 从 `from` 起推进 `steps` 个交易日; 日历数据不足 → None.
fn advance_trading_days(from: NaiveDate, steps: u32) -> Result<Option<NaiveDate>, String> {
    let mut cursor = from;
    for _ in 0..steps {
        match verified_next_a_share_trading_day(cursor) {
            Ok(next) => cursor = next,
            Err(_) => return Ok(None),
        }
    }
    Ok(Some(cursor))
}

/// 相对基准的百分比涨跌.
fn pct_from(base: f64, value: f64) -> f64 {
    (value - base) / base * 100.
}

/// 回填一轮: 取未回填行, 能算的算完写回.
pub fn backfill_advice_outcomes(
    db: &DatabaseManager,
    as_of: NaiveDate,
) -> Result<BackfillReport, String> {
    let mut report = BackfillReport::default();
    let pending = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        load_pending_outcomes(&mut *conn, BACKFILL_BATCH_LIMIT)?
    };
    report.scanned = pending.len();
    if pending.is_empty() {
        return Ok(report);
    }

    for row in pending {
        let base_date = match NaiveDate::parse_from_str(&row.business_date, "%Y-%m-%d") {
            Ok(date) => date,
            Err(error) => {
                let mut conn = db.get_conn().map_err(|e| e.to_string())?;
                let _ = mark_outcome_void(&mut *conn, row.id, "business_date_invalid");
                report.voided += 1;
                report
                    .errors
                    .push(format!("id={} business_date 非法: {error}", row.id));
                continue;
            }
        };
        let overdue = (as_of - base_date).num_days() > VOID_AFTER_DAYS;

        // 基准日收盘 —— 回填的锚点, 没有它整行无从算起.
        let Some(base_close) = read_close(db, &row.code, &row.business_date)? else {
            if overdue {
                let mut conn = db.get_conn().map_err(|e| e.to_string())?;
                let _ = mark_outcome_void(&mut *conn, row.id, "base_close_missing");
                report.voided += 1;
                report.errors.push(format!(
                    "id={} {} 基准日 {} 无收盘, 已作废",
                    row.id, row.code, row.business_date
                ));
            } else {
                report.not_mature += 1;
            }
            continue;
        };

        // 观察窗终点: T+5. 日历数据不足时先推迟, 超期则作废.
        let Some(window_end) = advance_trading_days(base_date, HORIZON_DAYS)? else {
            if overdue {
                let mut conn = db.get_conn().map_err(|e| e.to_string())?;
                let _ = mark_outcome_void(&mut *conn, row.id, "trading_calendar_unavailable");
                report.voided += 1;
            } else {
                report.not_mature += 1;
            }
            continue;
        };
        if window_end > as_of {
            report.not_mature += 1;
            continue;
        }

        // 观察窗内的极值 —— 没有它就不写回 (不补 0).
        let Some(window_start) = advance_trading_days(base_date, 1)? else {
            report.not_mature += 1;
            continue;
        };
        let from = window_start.format("%Y-%m-%d").to_string();
        let to = window_end.format("%Y-%m-%d").to_string();
        let Some((hi, lo, traded_days)) = read_high_low(db, &row.code, &from, &to)? else {
            if overdue {
                let mut conn = db.get_conn().map_err(|e| e.to_string())?;
                let _ = mark_outcome_void(&mut *conn, row.id, "window_no_data");
                report.voided += 1;
                report.errors.push(format!(
                    "id={} {} 观察窗 {}..{} 无行情, 已作废",
                    row.id, row.code, from, to
                ));
            } else {
                report.not_mature += 1;
            }
            continue;
        };

        // 各期收盘: 缺哪期就记 NULL, 不阻碍整行写回.
        let mut read_change = |horizon: u32| -> Result<Option<f64>, String> {
            match advance_trading_days(base_date, horizon)? {
                Some(target) if target <= as_of => {
                    let day = target.format("%Y-%m-%d").to_string();
                    Ok(read_close(db, &row.code, &day)?.map(|close| pct_from(base_close, close)))
                }
                _ => Ok(None),
            }
        };
        let change_t1 = read_change(1)?;
        let change_t3 = read_change(3)?;
        let change_t5 = read_change(5)?;

        let expected_days = HORIZON_DAYS as i64;
        let note = if traded_days < expected_days {
            Some(format!(
                "window_partial: {traded_days}/{expected_days} trading days with data"
            ))
        } else {
            None
        };

        let filled = FilledOutcome {
            expected_price: base_close,
            change_t1,
            change_t3,
            change_t5,
            mfe: pct_from(base_close, hi),
            mae: pct_from(base_close, lo),
        };
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        match apply_outcome(&mut *conn, row.id, &filled, note.as_deref()) {
            Ok(()) => report.filled += 1,
            Err(error) => {
                report.errors.push(format!("id={} 写回失败: {error}", row.id));
            }
        }
    }

    Ok(report)
}
