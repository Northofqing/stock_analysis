//! BR-134 纸面交易批次库存重建。

use std::collections::{BTreeMap, VecDeque};

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
}

#[derive(Debug)]
struct OpenPaperLot {
    bought_at: chrono::NaiveDateTime,
    remaining_quantity: u32,
    price: f64,
}

#[derive(Debug)]
struct PositionState {
    name: String,
    lots: VecDeque<OpenPaperLot>,
}

pub(crate) fn rebuild_paper_positions(
    fills: &[PaperFill],
    as_of_date: chrono::NaiveDate,
) -> Result<Vec<PaperPositionInventory>, String> {
    let mut states = BTreeMap::<String, PositionState>::new();
    for fill in fills {
        if fill.direction != "buy" {
            return Err(format!(
                "paper fill id={} direction invalid: {:?}",
                fill.id, fill.direction
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
            });
        state.name.clone_from(&fill.name);
        state.lots.push_back(OpenPaperLot {
            bought_at: fill.occurred_at,
            remaining_quantity: quantity,
            price,
        });
    }

    states
        .into_iter()
        .map(|(code, state)| inventory_from_state(code, state, as_of_date))
        .collect()
}

fn inventory_from_state(
    code: String,
    state: PositionState,
    as_of_date: chrono::NaiveDate,
) -> Result<PaperPositionInventory, String> {
    let mut total_quantity = 0_u32;
    let mut sellable_quantity = 0_u32;
    let mut locked_quantity = 0_u32;
    let mut sellable_cost = 0.0_f64;
    let mut earliest_sellable_date = None;

    for lot in state.lots {
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
        } else if bought_date == as_of_date {
            locked_quantity = locked_quantity
                .checked_add(lot.remaining_quantity)
                .ok_or_else(|| format!("paper position {code} locked quantity overflow"))?;
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
        name: state.name,
        total_quantity,
        sellable_quantity,
        locked_quantity,
        sellable_avg_price,
        earliest_sellable_date,
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
}
