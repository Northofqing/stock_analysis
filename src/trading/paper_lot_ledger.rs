//! BR-134 纸面交易批次库存重建。

use std::collections::{BTreeMap, HashSet, VecDeque};

use super::paper_replay_financial_work_v1::{
    self as financial, FinancialFailure, FinancialSink, FinancialWork, HistoryChrono, HistoryText,
    HistoryTreeSlot,
};
use crate::performance::fee_evidence::{fill_adverse_cost, FillSide};

/// 解析持久化纸面成交的规范时间。禁止 SQLite/调用方把 `now`、仅日期或仅时间
/// 补造成事实；执行账本与策略研究共用同一严格边界。
pub(crate) fn parse_paper_fill_timestamp(
    fill_id: i64,
    raw: &str,
) -> Result<chrono::NaiveDateTime, String> {
    financial::historical_text(parse_paper_fill_timestamp_body(
        fill_id,
        raw,
        &mut FinancialWork::Historical,
    ))
}
pub(crate) fn parse_paper_fill_timestamp_body(
    fill_id: i64,
    raw: &str,
    work: &mut FinancialWork<'_, '_>,
) -> financial::Result<chrono::NaiveDateTime> {
    work.history_begin()?;
    let parsed = match chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f") {
        Ok(value) => value,
        Err(error) => return Err(fifo_error(work, FifoText::Timestamp { fill_id, error })?),
    };
    let whole_seconds = work.history_time(HistoryChrono::Whole(parsed))?;
    let canonical = raw == whole_seconds
        || raw
            .strip_prefix(&work.history_time(HistoryChrono::Dotted(&whole_seconds))?)
            .is_some_and(|fraction| {
                !fraction.is_empty()
                    && fraction.len() <= 9
                    && fraction.bytes().all(|byte| byte.is_ascii_digit())
            });
    if !canonical {
        return Err(fifo_error(work, FifoText::Canonical { fill_id, raw })?);
    }
    Ok(parsed)
}

#[derive(Debug, Clone)]
pub(crate) struct PaperFill {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub direction: String,
    pub fill_price: Option<f64>,
    pub quantity: i64,
    pub occurred_at: chrono::NaiveDateTime,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaperPositionInventory {
    pub code: String,
    pub name: String,
    pub total_quantity: u32,
    pub sellable_quantity: u32,
    pub locked_quantity: u32,
    pub sellable_avg_price: Option<f64>,
    /// Scenario buy-side fees allocated from each original fill to its open sellable shares.
    pub sellable_buy_fee: f64,
    pub earliest_sellable_date: Option<chrono::NaiveDate>,
    as_of_date: chrono::NaiveDate,
    source_fill_ids: Vec<i64>,
    open_lots: Vec<PaperLotAuditEvidence>,
}

impl PaperPositionInventory {
    pub(crate) fn audit_evidence(&self) -> String {
        let source_fill_ids = self
            .source_fill_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let open_lots = self
            .open_lots
            .iter()
            .map(|lot| {
                format!(
                    "{}@{}@{}@{}@{:016x}@{}",
                    lot.buy_fill_id,
                    lot.bought_at.format("%Y-%m-%dT%H:%M:%S%.9f"),
                    lot.original_quantity,
                    lot.remaining_quantity,
                    lot.price.to_bits(),
                    if lot.sellable { "sellable" } else { "locked" }
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        let sellable_avg_price_bits = self.sellable_avg_price.map_or_else(
            || "none".to_string(),
            |price| format!("{:016x}", price.to_bits()),
        );
        format!(
            "BR134_FIFO_V2;as_of={};source_fill_ids={source_fill_ids};open_lots={open_lots};sellable_quantity={};locked_quantity={};sellable_avg_price_bits={sellable_avg_price_bits};fee_basis=lot-rates-v1;sellable_buy_fee_bits={:016x}",
            self.as_of_date, self.sellable_quantity, self.locked_quantity, self.sellable_buy_fee.to_bits()
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaperLotAuditEvidence {
    buy_fill_id: i64,
    bought_at: chrono::NaiveDateTime,
    original_quantity: u32,
    remaining_quantity: u32,
    price: f64,
    sellable: bool,
}

#[derive(Debug)]
pub(crate) struct OpenPaperLot {
    buy_fill_id: i64,
    bought_at: chrono::NaiveDateTime,
    original_quantity: u32,
    remaining_quantity: u32,
    price: f64,
}

#[derive(Debug)]
pub(crate) struct PositionState {
    name: String,
    lots: VecDeque<OpenPaperLot>,
    source_fill_ids: Vec<i64>,
}

pub(crate) fn rebuild_paper_positions(
    fills: &[PaperFill],
    as_of_date: chrono::NaiveDate,
) -> Result<Vec<PaperPositionInventory>, String> {
    financial::historical_text(rebuild_paper_positions_body(
        fills,
        as_of_date,
        &mut FinancialWork::Historical,
    ))
}
pub(crate) fn rebuild_paper_positions_body(
    fills: &[PaperFill],
    as_of_date: chrono::NaiveDate,
    work: &mut FinancialWork<'_, '_>,
) -> financial::Result<Vec<PaperPositionInventory>> {
    work.history_begin()?;
    let mut states = BTreeMap::<String, PositionState>::new();
    let mut seen_ids = HashSet::new();
    let mut previous_order = None;
    for fill in fills {
        if fill.id <= 0 || fill.code.trim().is_empty() || fill.name.trim().is_empty() {
            return Err(fifo_error(work, FifoText::Identity(fill))?);
        }
        if !work.history_seen(&mut seen_ids, fill.id)? {
            return Err(fifo_error(work, FifoText::Duplicate(fill.id))?);
        }
        let current_order = (fill.occurred_at, fill.id);
        if previous_order.is_some_and(|previous| previous >= current_order) {
            return Err(fifo_error(work, FifoText::Order(fill))?);
        }
        previous_order = Some(current_order);
        if fill.occurred_at.date() > as_of_date {
            return Err(fifo_error(work, FifoText::Future { fill, as_of_date })?);
        }
        let price = match fill
            .fill_price
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            Some(value) => value,
            None => return Err(fifo_error(work, FifoText::Price(fill.id))?),
        };
        let quantity = match u32::try_from(fill.quantity)
            .ok()
            .filter(|value| *value > 0 && value.is_multiple_of(100))
        {
            Some(value) => value,
            None => return Err(fifo_error(work, FifoText::Quantity(fill))?),
        };
        let key = work.copy(&fill.code)?;
        let state = match work.history_entry(&mut states, key)? {
            HistoryTreeSlot::Occupied(value) => value,
            HistoryTreeSlot::Vacant(entry) => {
                let value = PositionState {
                    name: work.copy(&fill.name)?,
                    lots: VecDeque::new(),
                    source_fill_ids: Vec::new(),
                };
                work.history_insert(entry, value)?
            }
        };
        work.history_name(&mut state.name, &fill.name)?;
        work.history_push(&mut state.source_fill_ids, fill.id)?;
        match fill.direction.as_str() {
            "buy" => work.history_lot(
                &mut state.lots,
                OpenPaperLot {
                    buy_fill_id: fill.id,
                    bought_at: fill.occurred_at,
                    original_quantity: quantity,
                    remaining_quantity: quantity,
                    price,
                },
            )?,
            "sell" => {
                let mut remaining = quantity;
                while remaining > 0 {
                    let lot = match state.lots.front_mut() {
                        Some(lot) => lot,
                        None => {
                            return Err(fifo_error(work, FifoText::Oversell { fill, remaining })?)
                        }
                    };
                    if lot.bought_at.date() >= fill.occurred_at.date() {
                        return Err(fifo_error(
                            work,
                            FifoText::TPlusOne {
                                fill,
                                bought: lot.bought_at.date(),
                            },
                        )?);
                    }
                    let consumed = remaining.min(lot.remaining_quantity);
                    lot.remaining_quantity -= consumed;
                    remaining -= consumed;
                    if lot.remaining_quantity == 0 {
                        state.lots.pop_front();
                    }
                }
            }
            other => {
                return Err(fifo_error(
                    work,
                    FifoText::Direction { id: fill.id, other },
                )?)
            }
        }
    }
    let mut positions = Vec::new();
    for (code, state) in states {
        let inventory = inventory_from_state_body(code, state, as_of_date, work)?;
        if inventory.total_quantity > 0 {
            work.history_push(&mut positions, inventory)?;
        }
    }
    Ok(positions)
}
fn inventory_from_state_body(
    code: String,
    state: PositionState,
    as_of_date: chrono::NaiveDate,
    work: &mut FinancialWork<'_, '_>,
) -> financial::Result<PaperPositionInventory> {
    let PositionState {
        name,
        lots,
        source_fill_ids,
    } = state;
    let mut total_quantity = 0_u32;
    let mut sellable_quantity = 0_u32;
    let mut locked_quantity = 0_u32;
    let mut sellable_cost = 0.0_f64;
    let mut sellable_buy_fee = 0.0_f64;
    let mut earliest_sellable_date = None;
    let mut open_lots = work.history_vector(lots.len())?;
    for lot in lots {
        total_quantity = match total_quantity.checked_add(lot.remaining_quantity) {
            Some(value) => value,
            None => {
                return Err(fifo_error(
                    work,
                    FifoText::Position {
                        code: &code,
                        reason: PositionReason::Quantity,
                    },
                )?)
            }
        };
        let bought_date = lot.bought_at.date();
        if bought_date < as_of_date {
            sellable_quantity = match sellable_quantity.checked_add(lot.remaining_quantity) {
                Some(value) => value,
                None => {
                    return Err(fifo_error(
                        work,
                        FifoText::Position {
                            code: &code,
                            reason: PositionReason::SellableQuantity,
                        },
                    )?)
                }
            };
            sellable_cost += lot.price * f64::from(lot.remaining_quantity);
            if !sellable_cost.is_finite() {
                return Err(fifo_error(
                    work,
                    FifoText::Position {
                        code: &code,
                        reason: PositionReason::SellableCost,
                    },
                )?);
            }
            let original_notional = lot.price * f64::from(lot.original_quantity);
            if !original_notional.is_finite() {
                return Err(fifo_error(
                    work,
                    FifoText::Position {
                        code: &code,
                        reason: PositionReason::Notional,
                    },
                )?);
            }
            sellable_buy_fee += fill_adverse_cost(FillSide::Buy, original_notional)
                * f64::from(lot.remaining_quantity)
                / f64::from(lot.original_quantity);
            if !sellable_buy_fee.is_finite() {
                return Err(fifo_error(
                    work,
                    FifoText::Position {
                        code: &code,
                        reason: PositionReason::Fee,
                    },
                )?);
            }
            earliest_sellable_date = Some(
                earliest_sellable_date.map_or(bought_date, |current: chrono::NaiveDate| {
                    current.min(bought_date)
                }),
            );
            work.history_push(
                &mut open_lots,
                PaperLotAuditEvidence {
                    buy_fill_id: lot.buy_fill_id,
                    bought_at: lot.bought_at,
                    original_quantity: lot.original_quantity,
                    remaining_quantity: lot.remaining_quantity,
                    price: lot.price,
                    sellable: true,
                },
            )?;
        } else if bought_date == as_of_date {
            locked_quantity = match locked_quantity.checked_add(lot.remaining_quantity) {
                Some(value) => value,
                None => {
                    return Err(fifo_error(
                        work,
                        FifoText::Position {
                            code: &code,
                            reason: PositionReason::LockedQuantity,
                        },
                    )?)
                }
            };
            work.history_push(
                &mut open_lots,
                PaperLotAuditEvidence {
                    buy_fill_id: lot.buy_fill_id,
                    bought_at: lot.bought_at,
                    original_quantity: lot.original_quantity,
                    remaining_quantity: lot.remaining_quantity,
                    price: lot.price,
                    sellable: false,
                },
            )?;
        } else {
            return Err(fifo_error(
                work,
                FifoText::FutureLot {
                    code: &code,
                    bought_date,
                    as_of_date,
                },
            )?);
        }
    }
    let sellable_avg_price = if sellable_quantity == 0 {
        None
    } else {
        let average = sellable_cost / f64::from(sellable_quantity);
        if !average.is_finite() || average <= 0.0 {
            return Err(fifo_error(
                work,
                FifoText::Average {
                    code: &code,
                    average,
                },
            )?);
        }
        Some(average)
    };
    Ok(PaperPositionInventory {
        code,
        name,
        total_quantity,
        sellable_quantity,
        locked_quantity,
        sellable_avg_price,
        sellable_buy_fee,
        earliest_sellable_date,
        as_of_date,
        source_fill_ids,
        open_lots,
    })
}
#[derive(Clone, Copy)]
pub(crate) enum PositionReason {
    Quantity,
    SellableQuantity,
    SellableCost,
    Notional,
    Fee,
    LockedQuantity,
}
pub(crate) enum FifoText<'a> {
    Timestamp {
        fill_id: i64,
        error: chrono::ParseError,
    },
    Canonical {
        fill_id: i64,
        raw: &'a str,
    },
    Identity(&'a PaperFill),
    Duplicate(i64),
    Order(&'a PaperFill),
    Future {
        fill: &'a PaperFill,
        as_of_date: chrono::NaiveDate,
    },
    Price(i64),
    Quantity(&'a PaperFill),
    Oversell {
        fill: &'a PaperFill,
        remaining: u32,
    },
    TPlusOne {
        fill: &'a PaperFill,
        bought: chrono::NaiveDate,
    },
    Direction {
        id: i64,
        other: &'a str,
    },
    Position {
        code: &'a str,
        reason: PositionReason,
    },
    FutureLot {
        code: &'a str,
        bought_date: chrono::NaiveDate,
        as_of_date: chrono::NaiveDate,
    },
    Average {
        code: &'a str,
        average: f64,
    },
}
impl FifoText<'_> {
    pub(crate) fn write(&self, out: &mut FinancialSink<'_>) -> Result<(), ()> {
        use std::fmt::Write;
        let result = match self {
            Self::Timestamp {
                fill_id,
                error
            } => write!(out, "paper fill id={fill_id} timestamp invalid: {error}"),
            Self::Canonical {
                fill_id,
                raw
            } => write!(out, "paper fill id={fill_id} timestamp invalid: expected YYYY-MM-DD HH:MM:SS[.fraction], got {raw:?}"),
            Self::Identity(fill) => write!(out, "paper fill identity invalid: id={} code={:?} name={:?}", fill.id, fill.code, fill.name),
            Self::Duplicate(id) => write!(out, "paper fill duplicate identity: id={id}"),
            Self::Order(fill) => write!(out, "paper fills out of order at id={} occurred_at={}", fill.id, fill.occurred_at),
            Self::Future {
                fill,
                as_of_date
            } => write!(out, "paper fill id={} has future fill date {} after {}", fill.id, fill.occurred_at.date(), as_of_date),
            Self::Price(id) => write!(out, "paper fill id={id} fill_price missing/invalid"),
            Self::Quantity(fill) => write!(out, "paper fill id={} quantity invalid: {}", fill.id, fill.quantity),
            Self::Oversell {
                fill,
                remaining
            } => write!(out, "paper sell id={} oversells {} by {} shares", fill.id, fill.code, remaining),
            Self::TPlusOne {
                fill,
                bought
            } => write!(out, "paper sell id={} violates A-share T+1 for {}: buy_date={} sell_date={}", fill.id, fill.code, bought, fill.occurred_at.date()),
            Self::Direction {
                id,
                other
            } => write!(out, "paper fill id={id} direction invalid: {other:?}"),
            Self::Position {
                code,
                reason
            } => write!(out, "paper position {code} {}", match reason {
                PositionReason::Quantity => "quantity overflow", PositionReason::SellableQuantity => "sellable quantity overflow", PositionReason::SellableCost => "sellable cost invalid", PositionReason::Notional => "original buy notional invalid", PositionReason::Fee => "allocated buy fee invalid", PositionReason::LockedQuantity => "locked quantity overflow",
            }),
            Self::FutureLot {
                code,
                bought_date,
                as_of_date
            } => write!(out, "paper position {code} contains future fill date {bought_date} after {as_of_date}"),
            Self::Average {
                code,
                average
            } => write!(out, "paper position {code} sellable average price invalid: {average}"),
        };
        result.map_err(|_| ())
    }
}
fn fifo_error(
    work: &mut FinancialWork<'_, '_>,
    text: FifoText<'_>,
) -> financial::Result<FinancialFailure> {
    work.history_error(HistoryText::Fifo(text))
}
impl financial::history_sealed::Element for PaperFill {}
impl financial::HistoryElement for PaperFill {}
impl financial::history_sealed::Element for PaperPositionInventory {}
impl financial::HistoryElement for PaperPositionInventory {}
impl financial::history_sealed::Element for PaperLotAuditEvidence {}
impl financial::HistoryElement for PaperLotAuditEvidence {}
impl financial::history_sealed::TreeEntry for (String, PositionState) {}
impl financial::HistoryTreeEntry for (String, PositionState) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn fill(id: i64, direction: &str, price: f64, quantity: i64, occurred_at: &str) -> PaperFill {
        PaperFill {
            id,
            code: "TEST_CODE_600001".to_string(),
            name: "测试股票".to_string(),
            direction: direction.to_string(),
            fill_price: Some(price),
            quantity,
            occurred_at: chrono::NaiveDateTime::parse_from_str(occurred_at, "%Y-%m-%d %H:%M:%S")
                .unwrap(),
        }
    }

    #[test]
    fn mixed_overnight_and_same_day_lots_only_expose_overnight_quantity() {
        let fills = vec![
            fill(1, "buy", 10.0, 200, "2026-08-03 10:00:00"),
            fill(2, "buy", 12.0, 100, "2026-08-05 10:00:00"),
        ];

        let positions = rebuild_paper_positions(&fills, date(2026, 8, 5)).unwrap();

        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].total_quantity, 300);
        assert_eq!(positions[0].sellable_quantity, 200);
        assert_eq!(positions[0].locked_quantity, 100);
        assert_eq!(positions[0].sellable_avg_price, Some(10.0));
        assert!((positions[0].sellable_buy_fee - 5.0).abs() < 1e-9);
        assert_eq!(positions[0].earliest_sellable_date, Some(date(2026, 8, 3)));
    }

    #[test]
    fn inventory_audit_evidence_binds_source_fills_and_open_lots() {
        let fills = vec![
            fill(1, "buy", 10.0, 200, "2026-08-03 10:00:00"),
            fill(2, "buy", 12.0, 100, "2026-08-05 10:00:00"),
        ];

        let positions = rebuild_paper_positions(&fills, date(2026, 8, 5)).unwrap();

        assert_eq!(
            positions[0].audit_evidence(),
            "BR134_FIFO_V2;as_of=2026-08-05;source_fill_ids=1,2;open_lots=1@2026-08-03T10:00:00.000000000@200@200@4024000000000000@sellable|2@2026-08-05T10:00:00.000000000@100@100@4028000000000000@locked;sellable_quantity=200;locked_quantity=100;sellable_avg_price_bits=4024000000000000;fee_basis=lot-rates-v1;sellable_buy_fee_bits=4014000000000000"
        );
    }

    #[test]
    fn prior_partial_sell_consumes_the_oldest_lot() {
        let fills = vec![
            fill(1, "buy", 10.0, 200, "2026-08-03 10:00:00"),
            fill(2, "buy", 12.0, 100, "2026-08-04 10:00:00"),
            fill(3, "sell", 11.0, 100, "2026-08-05 10:00:00"),
        ];

        let positions = rebuild_paper_positions(&fills, date(2026, 8, 6)).unwrap();

        assert_eq!(positions[0].total_quantity, 200);
        assert_eq!(positions[0].sellable_quantity, 200);
        assert_eq!(positions[0].locked_quantity, 0);
        assert_eq!(positions[0].sellable_avg_price, Some(11.0));
        assert!((positions[0].sellable_buy_fee - 7.5).abs() < 1e-9);
        assert_eq!(positions[0].earliest_sellable_date, Some(date(2026, 8, 3)));
    }

    #[test]
    fn three_buy_lots_keep_three_commission_floors_for_one_sell() {
        let fills = vec![
            fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(2, "buy", 10.0, 100, "2026-08-03 10:01:00"),
            fill(3, "buy", 10.0, 100, "2026-08-03 10:02:00"),
        ];
        let position = rebuild_paper_positions(&fills, date(2026, 8, 4))
            .expect("three valid buy fills")
            .remove(0);
        assert_eq!(position.sellable_quantity, 300);
        assert!((position.sellable_buy_fee - 15.0).abs() < 1e-9);
        let net = crate::performance::fee_evidence::net_return_pct_with_allocated_buy_fee(
            position.sellable_avg_price.unwrap() * f64::from(position.sellable_quantity),
            3000.0,
            position.sellable_buy_fee,
        )
        .expect("valid FIFO net return");
        assert!((net + 23.0 / 3000.0 * 100.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_sell_that_would_consume_a_same_day_buy_lot() {
        let fills = vec![
            fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(2, "sell", 11.0, 100, "2026-08-03 14:00:00"),
        ];

        let error = rebuild_paper_positions(&fills, date(2026, 8, 4))
            .expect_err("A-share T+1 must reject a historical same-day sell");

        assert!(error.contains("T+1"), "{error}");
        assert!(error.contains("id=2"), "{error}");
    }

    #[test]
    fn rejects_future_sell_and_future_buy_even_if_the_position_would_be_cleared() {
        let future_sell = vec![
            fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(2, "sell", 11.0, 100, "2026-08-07 10:00:00"),
        ];
        let future_round_trip = vec![
            fill(1, "buy", 10.0, 100, "2026-08-07 10:00:00"),
            fill(2, "sell", 11.0, 100, "2026-08-08 10:00:00"),
        ];

        for (label, fills) in [
            ("future sell", future_sell),
            ("future round trip", future_round_trip),
        ] {
            let error = rebuild_paper_positions(&fills, date(2026, 8, 6)).expect_err(label);
            assert!(error.contains("future fill"), "{label}: {error}");
        }
    }

    #[test]
    fn rejects_sell_quantity_that_exceeds_overnight_inventory() {
        let fills = vec![
            fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(2, "buy", 12.0, 100, "2026-08-05 10:00:00"),
            fill(3, "sell", 11.0, 200, "2026-08-05 14:00:00"),
        ];

        let error = rebuild_paper_positions(&fills, date(2026, 8, 6))
            .expect_err("sell cannot consume the same-day remainder");

        assert!(error.contains("T+1"), "{error}");
        assert!(error.contains("id=3"), "{error}");
    }

    #[test]
    fn rejects_invalid_identity_and_order_before_returning_positions() {
        let mut non_positive_id = fill(0, "buy", 10.0, 100, "2026-08-03 10:00:00");
        let mut blank_code = fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00");
        blank_code.code = "  ".to_string();
        let mut blank_name = fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00");
        blank_name.name = String::new();
        let duplicate_ids = vec![
            fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(1, "buy", 11.0, 100, "2026-08-03 10:01:00"),
        ];
        let out_of_order = vec![
            fill(2, "buy", 10.0, 100, "2026-08-03 10:01:00"),
            fill(1, "buy", 11.0, 100, "2026-08-03 10:00:00"),
        ];

        let cases = vec![
            (
                "non-positive id",
                vec![non_positive_id.clone()],
                "identity invalid",
            ),
            ("blank code", vec![blank_code], "identity invalid"),
            ("blank name", vec![blank_name], "identity invalid"),
            ("duplicate id", duplicate_ids, "duplicate identity"),
            ("out of order", out_of_order, "out of order"),
        ];

        for (label, fills, expected) in cases {
            let error = rebuild_paper_positions(&fills, date(2026, 8, 6)).expect_err(label);
            assert!(error.contains(expected), "{label}: {error}");
        }

        non_positive_id.id = -1;
        let error =
            rebuild_paper_positions(&[non_positive_id], date(2026, 8, 6)).expect_err("negative id");
        assert!(error.contains("identity invalid"), "{error}");
    }

    #[test]
    fn fully_sold_symbol_is_not_an_open_position() {
        let fills = vec![
            fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00"),
            fill(2, "sell", 11.0, 100, "2026-08-04 10:00:00"),
        ];

        let positions = rebuild_paper_positions(&fills, date(2026, 8, 5)).unwrap();

        assert!(positions.is_empty());
    }

    #[test]
    fn rejects_invalid_trade_facts_for_the_whole_batch() {
        let mut missing_price = fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00");
        missing_price.fill_price = None;
        let mut non_finite_price = fill(1, "buy", 10.0, 100, "2026-08-03 10:00:00");
        non_finite_price.fill_price = Some(f64::NAN);
        let invalid_quantity = fill(1, "buy", 10.0, 99, "2026-08-03 10:00:00");
        let invalid_direction = fill(1, "hold", 10.0, 100, "2026-08-03 10:00:00");
        let oversell = fill(1, "sell", 10.0, 100, "2026-08-03 10:00:00");
        let future_fill = fill(1, "buy", 10.0, 100, "2026-08-07 10:00:00");
        let overflow = vec![
            fill(1, "buy", 10.0, 4_294_967_200, "2026-08-03 10:00:00"),
            fill(2, "buy", 10.0, 4_294_967_200, "2026-08-03 10:01:00"),
        ];

        let cases = vec![
            ("missing price", vec![missing_price], "fill_price"),
            ("non-finite price", vec![non_finite_price], "fill_price"),
            (
                "invalid quantity",
                vec![invalid_quantity],
                "quantity invalid",
            ),
            (
                "invalid direction",
                vec![invalid_direction],
                "direction invalid",
            ),
            ("oversell", vec![oversell], "oversells"),
            ("future fill", vec![future_fill], "future fill"),
            ("quantity overflow", overflow, "quantity overflow"),
        ];

        for (label, fills, expected) in cases {
            let error = rebuild_paper_positions(&fills, date(2026, 8, 6)).expect_err(label);
            assert!(error.contains(expected), "{label}: {error}");
        }
    }

    #[test]
    fn same_day_only_inventory_has_no_sellable_cost_or_date() {
        let fills = vec![fill(1, "buy", 12.0, 100, "2026-08-05 10:00:00")];

        let positions = rebuild_paper_positions(&fills, date(2026, 8, 5)).unwrap();

        assert_eq!(positions[0].total_quantity, 100);
        assert_eq!(positions[0].sellable_quantity, 0);
        assert_eq!(positions[0].locked_quantity, 100);
        assert_eq!(positions[0].sellable_avg_price, None);
        assert_eq!(positions[0].earliest_sellable_date, None);
    }

    #[test]
    fn interleaved_symbols_keep_fifo_state_isolated_and_sorted_by_code() {
        let mut code_b = fill(1, "buy", 20.0, 100, "2026-08-03 10:00:00");
        code_b.code = "TEST_CODE_600002".to_string();
        code_b.name = "测试乙".to_string();
        let mut code_a_old = fill(2, "buy", 10.0, 200, "2026-08-03 10:01:00");
        code_a_old.code = "TEST_CODE_600001".to_string();
        code_a_old.name = "测试甲".to_string();
        let mut code_b_new = fill(3, "buy", 30.0, 100, "2026-08-04 10:00:00");
        code_b_new.code = "TEST_CODE_600002".to_string();
        code_b_new.name = "测试乙".to_string();
        let mut code_a_new = fill(4, "buy", 12.0, 100, "2026-08-04 10:01:00");
        code_a_new.code = "TEST_CODE_600001".to_string();
        code_a_new.name = "测试甲".to_string();
        let mut sell_b = fill(5, "sell", 25.0, 100, "2026-08-05 10:00:00");
        sell_b.code = "TEST_CODE_600002".to_string();
        sell_b.name = "测试乙".to_string();
        let mut sell_a = fill(6, "sell", 11.0, 100, "2026-08-05 10:01:00");
        sell_a.code = "TEST_CODE_600001".to_string();
        sell_a.name = "测试甲".to_string();

        let positions = rebuild_paper_positions(
            &[code_b, code_a_old, code_b_new, code_a_new, sell_b, sell_a],
            date(2026, 8, 6),
        )
        .unwrap();

        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].code, "TEST_CODE_600001");
        assert_eq!(positions[0].name, "测试甲");
        assert_eq!(positions[0].total_quantity, 200);
        assert_eq!(positions[0].sellable_avg_price, Some(11.0));
        assert_eq!(positions[0].earliest_sellable_date, Some(date(2026, 8, 3)));
        assert_eq!(positions[1].code, "TEST_CODE_600002");
        assert_eq!(positions[1].name, "测试乙");
        assert_eq!(positions[1].total_quantity, 100);
        assert_eq!(positions[1].sellable_avg_price, Some(30.0));
        assert_eq!(positions[1].earliest_sellable_date, Some(date(2026, 8, 4)));
    }
}

#[cfg(test)]
pub(crate) fn history_boundary_open_lot() -> OpenPaperLot {
    OpenPaperLot {
        buy_fill_id: 1,
        bought_at: chrono::NaiveDate::from_ymd_opt(2026, 9, 23)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap(),
        original_quantity: 100,
        remaining_quantity: 100,
        price: 10.0,
    }
}

#[cfg(test)]
pub(crate) fn history_boundary_open_lots() -> [OpenPaperLot; 6] {
    [1, 2, 3, 4, 5, 6].map(|id| {
        let mut lot = history_boundary_open_lot();
        lot.buy_fill_id = id;
        lot
    })
}
