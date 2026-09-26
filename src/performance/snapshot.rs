//! PaperLedger 追加式每日绩效 revision；旧日期表仅作历史只读兼容。
//! 同一有效成交 receipt 提供策略净指标和独立 opening-inventory exclusions。

use crate::database::DatabaseManager;
use chrono::{Local, NaiveDate};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, QueryableByName)]
pub struct PerformanceSnapshot {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub date: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub total_trades: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub winning_trades: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub losing_trades: i32,
    #[diesel(sql_type = diesel::sql_types::Double)]
    pub total_pnl: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub sharpe_ratio: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub sortino_ratio: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub win_rate: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub max_drawdown: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub info_ratio: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub created_at: String,
}

pub fn compute_snapshot(date: NaiveDate) -> Result<PerformanceSnapshot, String> {
    use crate::trading::paper_ledger::{
        EffectiveFillRequest, EffectiveFillScope, EffectiveHistory, PaperLedger,
    };
    let binding = crate::trading::paper_ledger_runtime::active_binding()?;
    let db = DatabaseManager::try_get().ok_or("paper snapshot DB unavailable")?;
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding),
        history: EffectiveHistory::RestatedLatest,
        as_of: date,
    };
    PaperLedger::open(db, &chrono::Utc::now)
        .settle_snapshot(&request)
        .map(|revision| revision.metrics)
        .map_err(|e| e.to_string())
}

pub fn has_current_snapshot(date: NaiveDate) -> Result<bool, String> {
    use crate::trading::paper_ledger::{
        EffectiveFillRequest, EffectiveFillScope, EffectiveHistory, PaperLedger,
    };
    let binding = crate::trading::paper_ledger_runtime::active_binding()?;
    let db = DatabaseManager::try_get().ok_or("paper snapshot DB unavailable")?;
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding),
        history: EffectiveHistory::RestatedLatest,
        as_of: date,
    };
    PaperLedger::open(db, &chrono::Utc::now)
        .current_snapshot(&request)
        .map(|revision| revision.is_some())
        .map_err(|e| e.to_string())
}

pub(crate) fn metrics_from_pnls(
    date: NaiveDate,
    pnls: &[f64],
    id: i32,
    created_at: String,
) -> Result<PerformanceSnapshot, String> {
    let total_trades = i32::try_from(pnls.len())
        .map_err(|_| format!("paper performance trade count overflow: {}", pnls.len()))?;
    let winning = i32::try_from(pnls.iter().filter(|pnl| **pnl > 0.0).count())
        .map_err(|_| "paper performance winning count overflow".to_string())?;
    let losing = total_trades - winning;
    let total_pnl = pnls.iter().sum::<f64>();
    let win_rate = (total_trades > 0).then_some(winning as f64 / total_trades as f64);
    let sharpe = compute_sharpe(pnls);
    let sortino = compute_sortino(pnls);
    let max_dd = compute_max_drawdown(pnls);
    let info_ratio: Option<f64> = None;

    Ok(PerformanceSnapshot {
        id,
        date: date.to_string(),
        total_trades,
        winning_trades: winning,
        losing_trades: losing,
        total_pnl,
        sharpe_ratio: sharpe,
        sortino_ratio: sortino,
        win_rate,
        max_drawdown: max_dd,
        info_ratio,
        created_at,
    })
}

#[cfg(test)]
#[derive(diesel::QueryableByName, Debug)]
struct PaperFillRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    local_ts: String,
}

#[cfg(test)]
#[derive(Debug)]
struct OpenLot {
    remaining: u32,
    price: f64,
}

#[cfg(test)]
fn realized_pnls_for_date(
    rows: &[PaperFillRow],
    target_date: NaiveDate,
) -> Result<Vec<f64>, String> {
    use std::collections::{HashMap, VecDeque};

    let mut lots: HashMap<String, VecDeque<OpenLot>> = HashMap::new();
    let mut realized = Vec::new();
    let mut previous_order: Option<(chrono::NaiveDateTime, i64)> = None;

    for row in rows {
        if row.id <= 0 || row.code.trim().is_empty() {
            return Err(format!(
                "paper fill identity invalid: id={} code={:?}",
                row.id, row.code
            ));
        }
        let timestamp =
            chrono::NaiveDateTime::parse_from_str(&row.local_ts, "%Y-%m-%d %H:%M:%S")
                .map_err(|error| format!("paper fill id={} timestamp invalid: {error}", row.id))?;
        if timestamp.date() > target_date {
            return Err(format!(
                "paper fill id={} is later than settlement date {}",
                row.id, target_date
            ));
        }
        if previous_order.is_some_and(|previous| previous > (timestamp, row.id)) {
            return Err(format!("paper fills are not ordered at id={}", row.id));
        }
        previous_order = Some((timestamp, row.id));
        let price = row
            .fill_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| format!("paper fill id={} fill_price missing/invalid", row.id))?;
        let quantity = u32::try_from(row.quantity)
            .ok()
            .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "paper fill id={} quantity invalid: {}",
                    row.id, row.quantity
                )
            })?;

        match row.direction.as_str() {
            "buy" => lots
                .entry(row.code.clone())
                .or_default()
                .push_back(OpenLot {
                    remaining: quantity,
                    price,
                }),
            "sell" => {
                let queue = lots
                    .get_mut(&row.code)
                    .ok_or_else(|| format!("paper sell id={} has no matched buy lots", row.id))?;
                let mut remaining = quantity;
                let mut pnl = 0.0;
                while remaining > 0 {
                    let lot = queue.front_mut().ok_or_else(|| {
                        format!(
                            "paper sell id={} quantity {} exceeds matched buys",
                            row.id, quantity
                        )
                    })?;
                    let matched = remaining.min(lot.remaining);
                    pnl += (price - lot.price) * f64::from(matched);
                    remaining -= matched;
                    lot.remaining -= matched;
                    if lot.remaining == 0 {
                        queue.pop_front();
                    }
                }
                if timestamp.date() == target_date {
                    if !pnl.is_finite() {
                        return Err(format!("paper sell id={} PnL is non-finite", row.id));
                    }
                    realized.push(pnl);
                }
            }
            other => {
                return Err(format!(
                    "paper fill id={} direction invalid: {other}",
                    row.id
                ));
            }
        }
    }
    Ok(realized)
}

fn compute_sharpe(pnls: &[f64]) -> Option<f64> {
    if pnls.is_empty() {
        return None;
    }
    let mean = pnls.iter().sum::<f64>() / pnls.len() as f64;
    let var = pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / pnls.len() as f64;
    if var > 0.0 {
        Some(mean / var.sqrt())
    } else {
        None
    }
}

/// Fix review #4 (HIGH) + #S1 (MEDIUM): Sortino = mean / downside_dev (只算负收益 stddev)
fn compute_sortino(pnls: &[f64]) -> Option<f64> {
    if pnls.is_empty() {
        return None;
    }
    let mean = pnls.iter().sum::<f64>() / pnls.len() as f64;
    let neg_pnls: Vec<f64> = pnls.iter().filter(|&&p| p < 0.0).copied().collect();
    if neg_pnls.is_empty() {
        return None;
    }
    let neg_var = neg_pnls.iter().map(|p| p.powi(2)).sum::<f64>() / neg_pnls.len() as f64;
    if neg_var > 0.0 {
        Some(mean / neg_var.sqrt())
    } else {
        None
    }
}

fn compute_max_drawdown(pnls: &[f64]) -> Option<f64> {
    if pnls.is_empty() {
        return None;
    }
    let mut cum = 0.0;
    let mut peak = 0.0;
    let mut max_dd: f64 = 0.0;
    for pnl in pnls {
        cum += pnl;
        if cum > peak {
            peak = cum;
        }
        let dd = peak - cum;
        if dd > max_dd {
            max_dd = dd;
        }
    }
    Some(max_dd)
}

pub struct PerformanceEngine;

impl PerformanceEngine {
    pub fn daily_settlement() -> Result<PerformanceSnapshot, String> {
        let today = Local::now().date_naive();
        compute_snapshot(today)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        local_ts: &str,
    ) -> PaperFillRow {
        PaperFillRow {
            id,
            code: code.to_string(),
            direction: direction.to_string(),
            fill_price: Some(price),
            quantity,
            local_ts: local_ts.to_string(),
        }
    }

    #[test]
    fn fifo_realized_pnl_uses_historical_buy_cost() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-17 10:00:00",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "buy",
                12.0,
                200,
                "2026-07-18 09:31:00",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                15.0,
                200,
                "2026-07-18 14:00:00",
            ),
        ];

        let pnls = realized_pnls_for_date(&rows, target).expect("valid FIFO fills");

        assert_eq!(pnls, vec![800.0]);
    }

    #[test]
    fn fifo_rejects_oversell() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                100,
                "2026-07-18 10:00:00",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                200,
                "2026-07-18 14:00:00",
            ),
        ];

        let error = realized_pnls_for_date(&rows, target).expect_err("oversell must fail");

        assert!(error.contains("exceeds matched buys"));
    }

    #[test]
    fn fifo_only_emits_target_date_sells() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600000",
                "buy",
                10.0,
                200,
                "2026-07-16 10:00:00",
            ),
            fill(
                2,
                "TEST_CODE_600000",
                "sell",
                11.0,
                100,
                "2026-07-17 14:00:00",
            ),
            fill(
                3,
                "TEST_CODE_600000",
                "sell",
                12.0,
                100,
                "2026-07-18 14:00:00",
            ),
        ];

        let pnls = realized_pnls_for_date(&rows, target).expect("valid FIFO fills");

        assert_eq!(pnls, vec![200.0]);
    }

    #[test]
    fn rejects_missing_fill_price_and_invalid_lot_size() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let mut missing_price = fill(
            1,
            "TEST_CODE_600000",
            "buy",
            10.0,
            100,
            "2026-07-18 10:00:00",
        );
        missing_price.fill_price = None;
        let error = realized_pnls_for_date(&[missing_price], target)
            .expect_err("missing fill price must fail");
        assert!(error.contains("fill_price missing/invalid"));

        let invalid_lot = fill(
            2,
            "TEST_CODE_600000",
            "buy",
            10.0,
            101,
            "2026-07-18 10:00:00",
        );
        let error =
            realized_pnls_for_date(&[invalid_lot], target).expect_err("invalid lot must fail");
        assert!(error.contains("quantity invalid"));
    }

    #[test]
    fn sortino_differs_from_sharpe() {
        let pnls = vec![500.0, -300.0];
        let sharpe = compute_sharpe(&pnls).expect("variable returns have Sharpe");
        let sortino = compute_sortino(&pnls).expect("negative return provides downside risk");
        assert_ne!(sharpe, sortino, "Sortino 应 != Sharpe (downside dev 不同)");
    }

    #[test]
    fn ratios_are_unavailable_without_required_evidence() {
        assert_eq!(compute_sharpe(&[]), None);
        assert_eq!(compute_sharpe(&[1.0, 1.0]), None);
        assert_eq!(compute_sortino(&[1.0, 2.0]), None);
        assert_eq!(compute_max_drawdown(&[]), None);
    }

    #[test]
    fn max_drawdown_uses_cumulative_realized_pnl() {
        assert_eq!(compute_max_drawdown(&[100.0, -50.0, -100.0]), Some(150.0));
    }
}
