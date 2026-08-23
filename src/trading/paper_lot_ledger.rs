//! BR-134 纸面交易批次库存重建。

use std::collections::{BTreeMap, HashSet, VecDeque};

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
                    "{}@{}@{}@{:016x}@{}",
                    lot.buy_fill_id,
                    lot.bought_at.format("%Y-%m-%dT%H:%M:%S%.9f"),
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
            "BR134_FIFO_V1;as_of={};source_fill_ids={source_fill_ids};open_lots={open_lots};sellable_quantity={};locked_quantity={};sellable_avg_price_bits={sellable_avg_price_bits}",
            self.as_of_date, self.sellable_quantity, self.locked_quantity
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PaperLotAuditEvidence {
    buy_fill_id: i64,
    bought_at: chrono::NaiveDateTime,
    remaining_quantity: u32,
    price: f64,
    sellable: bool,
}

#[derive(Debug)]
struct OpenPaperLot {
    buy_fill_id: i64,
    bought_at: chrono::NaiveDateTime,
    remaining_quantity: u32,
    price: f64,
}

#[derive(Debug)]
struct PositionState {
    name: String,
    lots: VecDeque<OpenPaperLot>,
    source_fill_ids: Vec<i64>,
}

pub(crate) fn rebuild_paper_positions(
    fills: &[PaperFill],
    as_of_date: chrono::NaiveDate,
) -> Result<Vec<PaperPositionInventory>, String> {
    let mut states = BTreeMap::<String, PositionState>::new();
    let mut seen_ids = HashSet::new();
    let mut previous_order = None;
    for fill in fills {
        if fill.id <= 0 || fill.code.trim().is_empty() || fill.name.trim().is_empty() {
            return Err(format!(
                "paper fill identity invalid: id={} code={:?} name={:?}",
                fill.id, fill.code, fill.name
            ));
        }
        if !seen_ids.insert(fill.id) {
            return Err(format!("paper fill duplicate identity: id={}", fill.id));
        }
        let current_order = (fill.occurred_at, fill.id);
        if previous_order.is_some_and(|previous| previous >= current_order) {
            return Err(format!(
                "paper fills out of order at id={} occurred_at={}",
                fill.id, fill.occurred_at
            ));
        }
        previous_order = Some(current_order);
        if fill.occurred_at.date() > as_of_date {
            return Err(format!(
                "paper fill id={} has future fill date {} after {}",
                fill.id,
                fill.occurred_at.date(),
                as_of_date
            ));
        }
        let price = fill
            .fill_price
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("paper fill id={} fill_price missing/invalid", fill.id))?;
        let quantity = u32::try_from(fill.quantity)
            .ok()
            .filter(|value| *value > 0 && value.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "paper fill id={} quantity invalid: {}",
                    fill.id, fill.quantity
                )
            })?;
        let state = states
            .entry(fill.code.clone())
            .or_insert_with(|| PositionState {
                name: fill.name.clone(),
                lots: VecDeque::new(),
                source_fill_ids: Vec::new(),
            });
        state.name.clone_from(&fill.name);
        state.source_fill_ids.push(fill.id);
        match fill.direction.as_str() {
            "buy" => state.lots.push_back(OpenPaperLot {
                buy_fill_id: fill.id,
                bought_at: fill.occurred_at,
                remaining_quantity: quantity,
                price,
            }),
            "sell" => {
                let mut remaining = quantity;
                while remaining > 0 {
                    let lot = state.lots.front_mut().ok_or_else(|| {
                        format!(
                            "paper sell id={} oversells {} by {} shares",
                            fill.id, fill.code, remaining
                        )
                    })?;
                    if lot.bought_at.date() >= fill.occurred_at.date() {
                        return Err(format!(
                            "paper sell id={} violates A-share T+1 for {}: buy_date={} sell_date={}",
                            fill.id,
                            fill.code,
                            lot.bought_at.date(),
                            fill.occurred_at.date()
                        ));
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
                return Err(format!(
                    "paper fill id={} direction invalid: {other:?}",
                    fill.id
                ));
            }
        }
    }

    let mut positions = Vec::new();
    for (code, state) in states {
        let inventory = inventory_from_state(code, state, as_of_date)?;
        if inventory.total_quantity > 0 {
            positions.push(inventory);
        }
    }
    Ok(positions)
}

fn inventory_from_state(
    code: String,
    state: PositionState,
    as_of_date: chrono::NaiveDate,
) -> Result<PaperPositionInventory, String> {
    let PositionState {
        name,
        lots,
        source_fill_ids,
    } = state;
    let mut total_quantity = 0_u32;
    let mut sellable_quantity = 0_u32;
    let mut locked_quantity = 0_u32;
    let mut sellable_cost = 0.0_f64;
    let mut earliest_sellable_date = None;
    let mut open_lots = Vec::with_capacity(lots.len());

    for lot in lots {
        total_quantity = total_quantity
            .checked_add(lot.remaining_quantity)
            .ok_or_else(|| format!("paper position {code} quantity overflow"))?;
        let bought_date = lot.bought_at.date();
        if bought_date < as_of_date {
            sellable_quantity = sellable_quantity
                .checked_add(lot.remaining_quantity)
                .ok_or_else(|| format!("paper position {code} sellable quantity overflow"))?;
            sellable_cost += lot.price * f64::from(lot.remaining_quantity);
            if !sellable_cost.is_finite() {
                return Err(format!("paper position {code} sellable cost invalid"));
            }
            earliest_sellable_date = Some(
                earliest_sellable_date.map_or(bought_date, |current: chrono::NaiveDate| {
                    current.min(bought_date)
                }),
            );
            open_lots.push(PaperLotAuditEvidence {
                buy_fill_id: lot.buy_fill_id,
                bought_at: lot.bought_at,
                remaining_quantity: lot.remaining_quantity,
                price: lot.price,
                sellable: true,
            });
        } else if bought_date == as_of_date {
            locked_quantity = locked_quantity
                .checked_add(lot.remaining_quantity)
                .ok_or_else(|| format!("paper position {code} locked quantity overflow"))?;
            open_lots.push(PaperLotAuditEvidence {
                buy_fill_id: lot.buy_fill_id,
                bought_at: lot.bought_at,
                remaining_quantity: lot.remaining_quantity,
                price: lot.price,
                sellable: false,
            });
        } else {
            return Err(format!(
                "paper position {code} contains future fill date {bought_date} after {as_of_date}"
            ));
        }
    }

    let sellable_avg_price = if sellable_quantity == 0 {
        None
    } else {
        let average = sellable_cost / f64::from(sellable_quantity);
        if !average.is_finite() || average <= 0.0 {
            return Err(format!(
                "paper position {code} sellable average price invalid: {average}"
            ));
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
        earliest_sellable_date,
        as_of_date,
        source_fill_ids,
        open_lots,
    })
}

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
            "BR134_FIFO_V1;as_of=2026-08-05;source_fill_ids=1,2;open_lots=1@2026-08-03T10:00:00.000000000@200@4024000000000000@sellable|2@2026-08-05T10:00:00.000000000@100@4028000000000000@locked;sellable_quantity=200;locked_quantity=100;sellable_avg_price_bits=4024000000000000"
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
        assert_eq!(positions[0].earliest_sellable_date, Some(date(2026, 8, 3)));
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
