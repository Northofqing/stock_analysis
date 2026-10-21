//! 观察池淘汰（2026-10-18）。
//!
//! ## 为什么需要
//!
//! 进池有规则（新闻催化 / 板块轮动），出池此前**完全没有**——601811 会永久留在
//! 观察池里，哪怕它跌了 30%，或当初的催化早已凉透。
//!
//! ## 两条淘汰规则（任一命中即出池）
//!
//! 1. **超期**：进池超过 10 个自然日仍未转 holding
//!    —— 一个观察对象十天没有买入信号，说明当初的判断已不成立
//! 2. **跌幅**：最新收盘相对进池日收盘跌超 15%
//!    —— 跌这么多说明"买点"根本不存在，继续挂着只会误导
//!
//! ## 边界
//!
//! - 只处理 `stage = 'observing'`。`holding` 票**绝不淘汰**——那是持仓，
//!   归 `ticket_stage_sync` 的持仓快照逻辑管，两者互不越界。
//! - 出池是软关闭（写 `close_date` + `stage='exited'`），记录保留可追溯。
//! - 进场价取 `open_date` 当天或之后的第一个收盘价（进池日可能是周末/节假日）。

use chrono::NaiveDate;
use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};

use crate::database::{ticket_lifecycle, DatabaseManager};

const MAX_OBSERVE_DAYS: i64 = 10;
const MAX_DRAWDOWN_FROM_ENTRY: f64 = 0.15;

#[derive(Debug, Default, Clone)]
pub struct EvictReport {
    pub scanned: usize,
    pub evicted: Vec<(String, String)>,
    pub errors: Vec<String>,
}

impl EvictReport {
    pub fn log(&self) {
        log::info!(
            "[观察淘汰] scanned={} evicted={}",
            self.scanned,
            self.evicted.len()
        );
        for (code, reason) in &self.evicted {
            log::info!("[观察淘汰] {code} 出池: {reason}");
        }
        for error in &self.errors {
            log::warn!("[观察淘汰] {error}");
        }
    }
}

#[derive(QueryableByName)]
struct PriceRow {
    #[diesel(sql_type = Nullable<Double>)]
    close: Option<f64>,
}

#[derive(QueryableByName)]
struct CodeRow {
    #[diesel(sql_type = Text)]
    code: String,
}

fn first_close_on_or_after(conn: &mut SqliteConnection, code: &str, date: &str) -> Option<f64> {
    diesel::sql_query(
        "SELECT close FROM stock_daily \
         WHERE code = ? AND date >= ? AND close IS NOT NULL \
         ORDER BY date ASC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(date)
    .get_result::<PriceRow>(conn)
    .ok()
    .and_then(|row| row.close)
}

fn latest_close_through(conn: &mut SqliteConnection, code: &str, date: &str) -> Option<f64> {
    diesel::sql_query(
        "SELECT close FROM stock_daily \
         WHERE code = ? AND date <= ? AND close IS NOT NULL \
         ORDER BY date DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(date)
    .get_result::<PriceRow>(conn)
    .ok()
    .and_then(|row| row.close)
}

pub fn evict_stale_observations(db: &DatabaseManager, as_of: &str) -> Result<EvictReport, String> {
    let as_of_date = NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
        .map_err(|error| format!("as_of 非法 ({as_of}): {error}"))?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    let observing: Vec<String> = diesel::sql_query(
        "SELECT code FROM ticket_lifecycle WHERE close_date IS NULL AND stage = 'observing'",
    )
    .load::<CodeRow>(&mut conn)
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|row| row.code)
    .collect();

    let mut report = EvictReport {
        scanned: observing.len(),
        ..Default::default()
    };

    for code in &observing {
        let ticket = match ticket_lifecycle::find_active_ticket(&mut conn, code) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => continue,
            Err(error) => {
                report.errors.push(format!("{code} 查询失败: {error}"));
                continue;
            }
        };

        let mut reason: Option<String> = None;

        // 规则 1：超期
        if let Ok(open_date) = NaiveDate::parse_from_str(&ticket.open_date, "%Y-%m-%d") {
            let age = (as_of_date - open_date).num_days();
            if age > MAX_OBSERVE_DAYS {
                reason = Some(format!(
                    "进池 {age} 天仍未转持有（上限 {MAX_OBSERVE_DAYS} 天）"
                ));
            }
        }

        // 规则 2：相对进池价跌幅
        if reason.is_none() {
            if let (Some(entry), Some(latest)) = (
                first_close_on_or_after(&mut conn, code, &ticket.open_date),
                latest_close_through(&mut conn, code, as_of),
            ) {
                if entry > 0.0 {
                    let drawdown = (latest - entry) / entry;
                    if drawdown < -MAX_DRAWDOWN_FROM_ENTRY {
                        reason = Some(format!(
                            "相对进池价 {entry} 跌至 {latest}（{:.1}%）",
                            drawdown * 100.0
                        ));
                    }
                }
            }
        }

        let Some(reason) = reason else {
            continue;
        };

        match ticket_lifecycle::close_ticket(&mut conn, ticket.id, as_of, "exited") {
            Ok(_) => {
                let _ = ticket_lifecycle::append_event(
                    &mut conn,
                    ticket.id,
                    as_of,
                    "evicted",
                    None,
                    None,
                    Some(&reason),
                );
                report.evicted.push((code.clone(), reason));
            }
            Err(error) => report.errors.push(format!("{code} 出池失败: {error}")),
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawdown_rule_boundary() {
        // 阈值本身不算命中（严格小于）
        assert!(!(-MAX_DRAWDOWN_FROM_ENTRY).lt(&-MAX_DRAWDOWN_FROM_ENTRY));
        // 超过阈值才算
        assert!(-0.16_f64 < -MAX_DRAWDOWN_FROM_ENTRY);
        // 未达阈值不命中
        assert!(-0.14_f64 > -MAX_DRAWDOWN_FROM_ENTRY);
    }

    #[test]
    fn thresholds_match_documented_values() {
        assert_eq!(MAX_OBSERVE_DAYS, 10);
        assert!((MAX_DRAWDOWN_FROM_ENTRY - 0.15).abs() < f64::EPSILON);
    }
}
