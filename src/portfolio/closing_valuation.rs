//! BR-147: unadjusted settled close validation and partial-coverage valuation.
use super::user_position_snapshot::UserPositionItemInput;
use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub struct ClosingPriceEvidence {
    pub code: String,
    pub price_date: NaiveDate,
    pub close: f64,
    pub provider: String,
    pub evidence_hash: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ClosingValuationItem {
    pub code: String,
    pub name: String,
    pub quantity: u64,
    pub cost_price: f64,
    pub close: Option<f64>,
    pub market_value: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub unrealized_return_pct: Option<f64>,
    pub daily_price_pnl: Option<f64>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ClosingValuationView {
    pub price_date: NaiveDate,
    pub provider: String,
    pub covered: usize,
    pub total: usize,
    pub items: Vec<ClosingValuationItem>,
    pub total_market_value: Option<f64>,
    pub total_unrealized_pnl: Option<f64>,
}

pub fn calculate_closing_valuation(
    items: &[UserPositionItemInput],
    prices: &[ClosingPriceEvidence],
    previous_closes: &[(String, f64)],
    price_date: NaiveDate,
    provider: &str,
) -> Result<ClosingValuationView, String> {
    let mut out = Vec::with_capacity(items.len());
    let mut covered = 0;
    let mut total_mv = 0.0;
    let mut total_pnl = 0.0;
    for i in items {
        let p = prices
            .iter()
            .find(|p| p.code == i.code && p.price_date == price_date);
        let prev = previous_closes
            .iter()
            .find(|(c, _)| c == &i.code)
            .map(|(_, v)| *v);
        let (close, mv, pnl, ret, dp) = if let Some(p) = p {
            if !p.close.is_finite() || p.close <= 0.0 {
                return Err(format!("invalid close for {}", i.code));
            }
            let mv = p.close * i.quantity as f64;
            let pnl = (p.close - i.cost_price) * i.quantity as f64;
            covered += 1;
            total_mv += mv;
            total_pnl += pnl;
            (
                Some(p.close),
                Some(mv),
                Some(pnl),
                Some((p.close / i.cost_price - 1.0) * 100.0),
                prev.map(|v| (p.close - v) * i.quantity as f64),
            )
        } else {
            (None, None, None, None, None)
        };
        out.push(ClosingValuationItem {
            code: i.code.clone(),
            name: i.name.clone(),
            quantity: i.quantity,
            cost_price: i.cost_price,
            close,
            market_value: mv,
            unrealized_pnl: pnl,
            unrealized_return_pct: ret,
            daily_price_pnl: dp,
        });
    }
    let totals = if covered == items.len() {
        (Some(total_mv), Some(total_pnl))
    } else {
        (None, None)
    };
    Ok(ClosingValuationView {
        price_date,
        provider: provider.into(),
        covered,
        total: items.len(),
        items: out,
        total_market_value: totals.0,
        total_unrealized_pnl: totals.1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_coverage_does_not_fake_totals() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        let items = vec![
            UserPositionItemInput {
                code: "TEST_CODE_000001".into(),
                name: "甲".into(),
                quantity: 100,
                cost_price: 10.0,
            },
            UserPositionItemInput {
                code: "TEST_CODE_600000".into(),
                name: "乙".into(),
                quantity: 100,
                cost_price: 20.0,
            },
        ];
        let p = vec![ClosingPriceEvidence {
            code: "TEST_CODE_000001".into(),
            price_date: d,
            close: 11.0,
            provider: "magic_tdx".into(),
            evidence_hash: "a".into(),
        }];
        let v = calculate_closing_valuation(&items, &p, &[], d, "magic_tdx").unwrap();
        assert_eq!(v.covered, 1);
        assert_eq!(v.total_market_value, None);
        assert_eq!(v.items[0].unrealized_pnl, Some(100.0));
    }
}
